//! Own the isolated worker and its CLI descendants through cancellation/shutdown.
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command};

pub(super) struct TreeChild {
    child: Child,
    #[cfg(windows)]
    job: Option<std::os::windows::io::OwnedHandle>,
    #[cfg(unix)]
    group: Option<i32>,
}

impl Deref for TreeChild {
    type Target = Child;
    fn deref(&self) -> &Child {
        &self.child
    }
}
impl DerefMut for TreeChild {
    fn deref_mut(&mut self) -> &mut Child {
        &mut self.child
    }
}

impl TreeChild {
    pub(super) fn kill(&mut self) -> std::io::Result<()> {
        #[cfg(windows)]
        if let Some(job) = &self.job {
            use std::os::windows::io::AsRawHandle;
            if unsafe {
                windows_sys::Win32::System::JobObjects::TerminateJobObject(job.as_raw_handle(), 1)
            } != 0
            {
                return Ok(());
            }
            return Err(std::io::Error::last_os_error());
        }
        #[cfg(unix)]
        if let Some(group) = self.group {
            extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            if unsafe { kill(-group, 9) } == 0 {
                return Ok(());
            }
            if self.child.try_wait()?.is_some() {
                return Ok(());
            }
            return Err(std::io::Error::last_os_error());
        }
        self.child.kill()
    }
}

pub(super) fn spawn(command: &mut Command) -> Result<TreeChild, String> {
    spawn_with_scope(command, true)
}

pub(super) fn spawn_inherited(command: &mut Command) -> Result<TreeChild, String> {
    spawn_with_scope(command, false)
}

fn spawn_with_scope(command: &mut Command, own_scope: bool) -> Result<TreeChild, String> {
    #[cfg(unix)]
    {
        use std::os::unix::process::CommandExt;
        if own_scope {
            command.process_group(0);
        }
    }
    #[cfg(windows)]
    let job = if own_scope {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::JobObjects::{
            CreateJobObjectW, JobObjectExtendedLimitInformation, SetInformationJobObject,
            JOBOBJECT_EXTENDED_LIMIT_INFORMATION, JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE,
        };
        let raw = unsafe { CreateJobObjectW(std::ptr::null(), std::ptr::null()) };
        if raw.is_null() {
            return Err("처리 작업의 종료 보호를 준비하지 못했습니다.".into());
        }
        let job = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut limits: JOBOBJECT_EXTENDED_LIMIT_INFORMATION = unsafe { std::mem::zeroed() };
        limits.BasicLimitInformation.LimitFlags = JOB_OBJECT_LIMIT_KILL_ON_JOB_CLOSE;
        if unsafe {
            SetInformationJobObject(
                job.as_raw_handle(),
                JobObjectExtendedLimitInformation,
                &limits as *const _ as *const _,
                std::mem::size_of_val(&limits) as u32,
            )
        } == 0
        {
            return Err("처리 작업의 종료 보호를 준비하지 못했습니다.".into());
        }
        Some(job)
    } else {
        None
    };
    let mut child = command
        .spawn()
        .map_err(|_| "음성 처리를 시작하지 못했습니다.")?;
    #[cfg(windows)]
    {
        use std::os::windows::io::AsRawHandle;
        use windows_sys::Win32::System::JobObjects::AssignProcessToJobObject;
        if job.as_ref().is_some_and(|job| unsafe { AssignProcessToJobObject(job.as_raw_handle(), child.as_raw_handle()) } == 0) {
            let _ = child.kill(); let _ = child.wait();
            return Err("처리 작업의 종료 보호를 연결하지 못했습니다.".into());
        }
    }
    #[cfg(unix)]
    let group = own_scope.then_some(child.id() as i32);
    Ok(TreeChild {
        child,
        #[cfg(windows)]
        job,
        #[cfg(unix)]
        group,
    })
}

impl Drop for TreeChild {
    fn drop(&mut self) {
        let _ = self.kill();
        #[cfg(windows)]
        drop(self.job.take());
        let _ = self.child.wait();
    }
}

#[cfg(all(test, windows))]
mod tests {
    use super::*;
    use std::fs;
    use std::time::{Duration, Instant};

    #[test]
    fn fixture() {
        let Ok(role) = std::env::var("NERO_TREE_FIXTURE_ROLE") else {
            return;
        };
        let root = std::path::PathBuf::from(std::env::var_os("NERO_TREE_FIXTURE_ROOT").unwrap());
        if role == "worker" {
            while !root.join("ready").exists() {
                std::thread::sleep(Duration::from_millis(5));
            }
            let mut command = Command::new(std::env::current_exe().unwrap());
            command
                .args(["--exact", "process_tree::tests::fixture", "--nocapture"])
                .env("NERO_TREE_FIXTURE_ROLE", "grandchild");
            let mut child = command.spawn().unwrap();
            fs::write(root.join("grandchild"), child.id().to_string()).unwrap();
            child.wait().unwrap();
        } else {
            loop {
                std::thread::sleep(Duration::from_secs(1));
            }
        }
    }

    #[test]
    fn cancellation_terminates_the_owned_worker_and_its_cli_descendant() {
        use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
        use windows_sys::Win32::System::Threading::{
            OpenProcess, WaitForSingleObject, PROCESS_SYNCHRONIZE,
        };
        let directory = tempfile::tempdir().unwrap();
        let mut command = Command::new(std::env::current_exe().unwrap());
        command
            .args(["--exact", "process_tree::tests::fixture", "--nocapture"])
            .env("NERO_TREE_FIXTURE_ROLE", "worker")
            .env("NERO_TREE_FIXTURE_ROOT", directory.path())
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::null());
        let mut worker = spawn(&mut command).unwrap();
        fs::write(directory.path().join("ready"), b"ready").unwrap();
        let started = Instant::now();
        let pid = loop {
            if let Ok(pid) = fs::read_to_string(directory.path().join("grandchild")) {
                if let Ok(pid) = pid.parse::<u32>() {
                    break pid;
                }
            }
            assert!(started.elapsed() < Duration::from_secs(10));
            std::thread::sleep(Duration::from_millis(10));
        };
        let raw = unsafe { OpenProcess(PROCESS_SYNCHRONIZE, 0, pid) };
        assert!(!raw.is_null());
        let handle = unsafe { OwnedHandle::from_raw_handle(raw) };
        worker.kill().unwrap();
        worker.wait().unwrap();
        // Cancellation must stop the descendant before the wrapper is dropped.
        assert_eq!(
            unsafe { WaitForSingleObject(handle.as_raw_handle(), 5000) },
            0
        );
    }
}
