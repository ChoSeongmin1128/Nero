//! Own the isolated worker and its CLI descendants through cancellation/shutdown.
use std::io::Write;
use std::ops::{Deref, DerefMut};
use std::process::{Child, Command};
use std::sync::atomic::{AtomicBool, Ordering};
use std::time::{Duration, Instant};

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
    /// Sends `signal` to the worker's own process group, or to the worker alone
    /// when it shares the parent's group. Not supported on Windows.
    pub(super) fn signal(&self, signal: i32) -> std::io::Result<()> {
        #[cfg(unix)]
        {
            extern "C" {
                fn kill(pid: i32, signal: i32) -> i32;
            }
            if signal == 0 {
                return Err(std::io::ErrorKind::Unsupported.into());
            }
            let target = self.group.map_or(self.child.id() as i32, |group| -group);
            if unsafe { kill(target, signal) } == 0 {
                return Ok(());
            }
            Err(std::io::Error::last_os_error())
        }
        #[cfg(not(unix))]
        {
            let _ = signal;
            Err(std::io::ErrorKind::Unsupported.into())
        }
    }

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
    #[cfg_attr(not(windows), allow(unused_mut))]
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

/// How a supervised worker ended.
#[derive(Debug)]
pub(super) enum Ended {
    /// It exited by itself.
    Exited(std::process::ExitStatus),
    /// Stopped because its job was cancelled.
    Cancelled,
    /// Stopped after running longer than its budget, paused time left out.
    OverBudget,
    /// Stopped because the caller's `stop_when` asked for it.
    Stopped,
    /// Its status could not be read; it was stopped.
    Unreadable(std::io::Error),
}

/// The worker was told to stop but its end could not be confirmed. It is recorded on the
/// job that owns the cancel flag, and Processing refuses new work after that job.
#[derive(Debug)]
pub(super) struct Unconfirmed;

pub(super) const UNCONFIRMED: &str = "음성 처리 종료를 확인하지 못했습니다. 앱을 다시 열어 주세요.";

/// What a supervised worker is held to besides its job's cancellation and pause.
#[derive(Default)]
pub(super) struct Watch<'a> {
    budget: Option<Duration>,
    stop_when: Option<&'a dyn Fn() -> bool>,
    ask_first: Option<(&'a [u8], Duration)>,
}

impl<'a> Watch<'a> {
    pub(super) fn budget(budget: Duration) -> Self {
        Self {
            budget: Some(budget),
            ..Self::default()
        }
    }
    /// Stops the worker once `stop` answers true.
    pub(super) fn stop_when(mut self, stop: &'a dyn Fn() -> bool) -> Self {
        self.stop_when = Some(stop);
        self
    }
    /// On cancellation, writes `request` to the worker's input and gives it `grace` to
    /// exit by itself before it is killed.
    pub(super) fn ask_first(mut self, request: &'a [u8], grace: Duration) -> Self {
        self.ask_first = Some((request, grace));
        self
    }
}

const POLL: Duration = Duration::from_millis(50);

/// Waits for `child` while applying its job's pause and priority, and stops it on
/// cancellation, an overrun budget or `stop_when`. Every stop kills the worker's tree
/// and waits for it; an end that cannot be confirmed is `Unconfirmed`.
pub(super) fn supervise(
    child: &mut TreeChild,
    cancel: &AtomicBool,
    watch: Watch<'_>,
) -> Result<Ended, Unconfirmed> {
    let began = Instant::now();
    let mut pauser = crate::job_control::Pauser::new(cancel);
    let ended = loop {
        let stop = if cancel.load(Ordering::Acquire) {
            Some(Ended::Cancelled)
        } else if watch
            .budget
            .is_some_and(|budget| began.elapsed().saturating_sub(pauser.paused()) > budget)
        {
            Some(Ended::OverBudget)
        } else if watch.stop_when.is_some_and(|stop| stop()) {
            Some(Ended::Stopped)
        } else {
            None
        };
        if let Some(stop) = stop {
            pauser.release(child);
            if let (Ended::Cancelled, Some((request, grace))) = (&stop, watch.ask_first) {
                if ask_to_exit(child, request, grace) {
                    break stop;
                }
            }
            break terminate(child, cancel).map(|()| stop)?;
        }
        match child.try_wait() {
            Ok(Some(status)) => break Ended::Exited(status),
            Ok(None) => {
                pauser.sync(child);
                std::thread::sleep(POLL);
            }
            Err(error) => {
                pauser.release(child);
                terminate(child, cancel)?;
                break Ended::Unreadable(error);
            }
        }
    };
    Ok(ended)
}

// Whether the worker exited by itself within `grace` after `request`.
fn ask_to_exit(child: &mut TreeChild, request: &[u8], grace: Duration) -> bool {
    if let Some(mut input) = child.stdin.take() {
        let _ = input.write_all(request);
    }
    let until = Instant::now() + grace;
    loop {
        if child.try_wait().ok().flatten().is_some() {
            return true;
        }
        if Instant::now() >= until {
            return false;
        }
        std::thread::sleep(POLL);
    }
}

fn terminate(child: &mut TreeChild, cancel: &AtomicBool) -> Result<(), Unconfirmed> {
    if child.try_wait().ok().flatten().is_some() {
        return Ok(());
    }
    child
        .kill()
        .and_then(|()| child.wait().map(|_| ()))
        .map_err(|_| {
            crate::job_control::report_unconfirmed_termination(cancel);
            Unconfirmed
        })
}

#[cfg(all(test, unix))]
mod supervise_tests {
    use super::*;
    use std::sync::Arc;

    fn sleeper(seconds: &str) -> TreeChild {
        spawn(Command::new("/bin/sleep").arg(seconds)).unwrap()
    }

    #[test]
    fn a_worker_is_waited_for_and_stopped_for_each_reason() {
        let cancel = AtomicBool::new(false);
        let mut quick = spawn(&mut Command::new("/usr/bin/true")).unwrap();
        assert!(matches!(
            supervise(&mut quick, &cancel, Watch::default()),
            Ok(Ended::Exited(status)) if status.success()
        ));

        let mut late = sleeper("30");
        let started = Instant::now();
        assert!(matches!(
            supervise(
                &mut late,
                &cancel,
                Watch::budget(Duration::from_millis(100))
            ),
            Ok(Ended::OverBudget)
        ));
        assert!(late.try_wait().unwrap().is_some());
        assert!(started.elapsed() < Duration::from_secs(5));

        let mut stopped = sleeper("30");
        let stop = || true;
        assert!(matches!(
            supervise(&mut stopped, &cancel, Watch::default().stop_when(&stop)),
            Ok(Ended::Stopped)
        ));
        assert!(stopped.try_wait().unwrap().is_some());

        let cancel = Arc::new(AtomicBool::new(false));
        let flag = cancel.clone();
        let canceller = std::thread::spawn(move || {
            std::thread::sleep(Duration::from_millis(100));
            flag.store(true, Ordering::Release);
        });
        let mut cancelled = sleeper("30");
        assert!(matches!(
            supervise(&mut cancelled, &cancel, Watch::default()),
            Ok(Ended::Cancelled)
        ));
        canceller.join().unwrap();
        assert!(cancelled.try_wait().unwrap().is_some());
    }

    #[test]
    fn a_cancelled_worker_asked_to_exit_gets_its_grace_before_it_is_killed() {
        let cancel = AtomicBool::new(true);
        let mut polite = spawn(
            Command::new("/bin/sh")
                .args(["-c", "read answer; exit 3"])
                .stdin(std::process::Stdio::piped()),
        )
        .unwrap();
        assert!(matches!(
            supervise(
                &mut polite,
                &cancel,
                Watch::default().ask_first(b"c\n", Duration::from_secs(5))
            ),
            Ok(Ended::Cancelled)
        ));
        assert_eq!(polite.try_wait().unwrap().unwrap().code(), Some(3));
    }

    #[test]
    fn an_unconfirmed_end_is_recorded_on_the_job_that_owns_the_flag() {
        let cancel = Arc::new(AtomicBool::new(false));
        let control = crate::job_control::JobControl::new(cancel.clone(), false);
        crate::job_control::report_unconfirmed_termination(&AtomicBool::new(false));
        assert!(!control.termination_unconfirmed());
        crate::job_control::report_unconfirmed_termination(&cancel);
        assert!(control.termination_unconfirmed());
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
