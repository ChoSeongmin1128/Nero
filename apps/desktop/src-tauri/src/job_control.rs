//! Pause and resume for the one running processing job.
//!
//! A job's worker processes are stopped with SIGSTOP and continued with SIGCONT by
//! the loop that spawned and waits for them, so no other thread signals a process id
//! that may already have been reaped. Rust-side work pauses only at explicit points
//! between stages. Only macOS supports this; elsewhere nothing is ever paused.
use crate::process_tree::TreeChild;
use serde::Serialize;
use std::cell::Cell;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicU8, Ordering};
use std::sync::{Arc, Mutex, Weak};
use std::thread;
use std::time::{Duration, Instant};

pub(crate) const SUPPORTED: bool = cfg!(target_os = "macos");

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseReason {
    Buffer,
    Overload,
    Memory,
    Thermal,
}

impl PauseReason {
    fn code(self) -> u8 {
        match self {
            Self::Buffer => 1,
            Self::Overload => 2,
            Self::Memory => 3,
            Self::Thermal => 4,
        }
    }
    fn from_code(code: u8) -> Option<Self> {
        match code {
            1 => Some(Self::Buffer),
            2 => Some(Self::Overload),
            3 => Some(Self::Memory),
            4 => Some(Self::Thermal),
            _ => None,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseBy {
    User,
    Auto,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub enum PauseState {
    /// Requested; the job has not reached a worker or a pause point yet.
    Pausing,
    Paused,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PauseView {
    pub by: PauseBy,
    pub state: PauseState,
    pub reason: Option<PauseReason>,
}

pub(crate) struct JobControl {
    cancel: Arc<AtomicBool>,
    pausable: bool,
    user: AtomicBool,
    auto: AtomicU8,
    throttle: AtomicBool,
    held: AtomicU32,
}

static REGISTRY: Mutex<Vec<Weak<JobControl>>> = Mutex::new(Vec::new());

impl JobControl {
    /// `pausable` is whether this kind of job and engine may be paused at all.
    pub(crate) fn new(cancel: Arc<AtomicBool>, pausable: bool) -> Arc<Self> {
        let control = Arc::new(Self {
            cancel,
            pausable: pausable && SUPPORTED,
            user: AtomicBool::new(false),
            auto: AtomicU8::new(0),
            throttle: AtomicBool::new(false),
            held: AtomicU32::new(0),
        });
        let mut registry = REGISTRY.lock().unwrap();
        registry.retain(|entry| entry.strong_count() > 0);
        registry.push(Arc::downgrade(&control));
        control
    }

    /// The control of the job that owns `cancel`. Live speech and other callers pass
    /// their own flag and get none, so they are never paused.
    pub(crate) fn find(cancel: &AtomicBool) -> Option<Arc<Self>> {
        REGISTRY
            .lock()
            .unwrap()
            .iter()
            .filter_map(Weak::upgrade)
            .find(|control| std::ptr::eq(Arc::as_ptr(&control.cancel), cancel))
    }

    pub(crate) fn pausable(&self) -> bool {
        self.pausable
    }

    pub(crate) fn set_user(&self, paused: bool) {
        self.user.store(paused, Ordering::Release);
    }

    pub(crate) fn user_paused(&self) -> bool {
        self.user.load(Ordering::Acquire)
    }

    pub(crate) fn set_auto(&self, reason: Option<PauseReason>) {
        self.auto
            .store(reason.map_or(0, PauseReason::code), Ordering::Release);
    }

    pub(crate) fn auto_reason(&self) -> Option<PauseReason> {
        PauseReason::from_code(self.auto.load(Ordering::Acquire))
    }

    /// Lower the worker's scheduling and this job's disk priority while recording.
    pub(crate) fn set_throttle(&self, throttle: bool) {
        self.throttle.store(throttle, Ordering::Release);
    }

    fn throttled(&self) -> bool {
        self.throttle.load(Ordering::Acquire)
    }

    fn cancelled(&self) -> bool {
        self.cancel.load(Ordering::Acquire)
    }

    pub(crate) fn wants_pause(&self) -> bool {
        self.pausable
            && !self.cancelled()
            && (self.user_paused() || self.auto_reason().is_some())
    }

    pub(crate) fn view(&self) -> Option<PauseView> {
        if !self.wants_pause() {
            return None;
        }
        let user = self.user_paused();
        Some(PauseView {
            by: if user { PauseBy::User } else { PauseBy::Auto },
            state: if self.held.load(Ordering::Acquire) > 0 {
                PauseState::Paused
            } else {
                PauseState::Pausing
            },
            reason: if user { None } else { self.auto_reason() },
        })
    }

    /// Blocks while a pause is wanted. Call only between stages and never while
    /// holding a processing lock.
    pub(crate) fn wait_while_paused(&self) {
        apply_thread_io(self.throttled());
        if !self.wants_pause() {
            return;
        }
        self.held.fetch_add(1, Ordering::AcqRel);
        while self.wants_pause() {
            thread::sleep(Duration::from_millis(100));
        }
        self.held.fetch_sub(1, Ordering::AcqRel);
        apply_thread_io(self.throttled());
    }
}

/// Throttles or restores this thread's disk reads.
pub(crate) fn throttle_this_thread(throttle: bool) {
    apply_thread_io(throttle);
}

/// Pause point for code that only has the job's cancel flag.
pub(crate) fn pause_point(cancel: &AtomicBool) {
    if let Some(control) = JobControl::find(cancel) {
        control.wait_while_paused();
    }
}

/// Applies the job's pause and priority to one worker from the loop that waits for it.
pub(crate) struct Pauser {
    control: Option<Arc<JobControl>>,
    stopped: bool,
    since: Option<Instant>,
    total: Duration,
    background: bool,
}

impl Pauser {
    pub(crate) fn new(cancel: &AtomicBool) -> Self {
        Self {
            control: JobControl::find(cancel),
            stopped: false,
            since: None,
            total: Duration::ZERO,
            background: false,
        }
    }

    /// Call on every poll of the worker.
    pub(crate) fn sync(&mut self, child: &TreeChild) {
        let Some(control) = self.control.clone() else {
            return;
        };
        let throttle = control.throttled();
        apply_thread_io(throttle);
        if throttle != self.background && set_background(child.id(), throttle) {
            self.background = throttle;
        }
        let want = control.wants_pause();
        if want && !self.stopped {
            if child.signal(SIGSTOP).is_ok() {
                self.stopped = true;
                self.since = Some(Instant::now());
                control.held.fetch_add(1, Ordering::AcqRel);
            }
        } else if !want && self.stopped {
            self.resume(child);
        }
    }

    /// Continues a stopped worker; call before cancelling or killing it.
    pub(crate) fn release(&mut self, child: &TreeChild) {
        if self.stopped {
            self.resume(child);
        }
    }

    fn resume(&mut self, child: &TreeChild) {
        let _ = child.signal(SIGCONT);
        self.stopped = false;
        if let Some(since) = self.since.take() {
            self.total += since.elapsed();
        }
        if let Some(control) = &self.control {
            control.held.fetch_sub(1, Ordering::AcqRel);
        }
    }

    /// Time the worker has spent stopped; wall-clock limits leave it out.
    pub(crate) fn paused(&self) -> Duration {
        self.total + self.since.map_or(Duration::ZERO, |since| since.elapsed())
    }
}

impl Drop for Pauser {
    fn drop(&mut self) {
        if self.stopped {
            if let Some(control) = &self.control {
                control.held.fetch_sub(1, Ordering::AcqRel);
            }
        }
    }
}

/// One look at the machine while a recording runs alongside a paused-capable job.
pub(crate) struct LoadSample {
    /// Largest share of the recording queue waiting to be saved since the last look.
    pub fill: f64,
    /// Audio device processor-overload notices since the last look.
    pub new_overloads: u64,
    pub memory_critical: bool,
    pub thermal_serious: bool,
}

// Initial values without measurement behind them; see docs/task-management.md.
const BUFFER_FILL: f64 = 0.25;
const OVERLOADS: usize = 3;
const OVERLOAD_WINDOW: Duration = Duration::from_secs(30);
const CLEAR_FOR: Duration = Duration::from_secs(30);

/// Decides the automatic pause from successive load samples: any signal pauses at
/// once, and the pause ends after every signal stayed clear for `CLEAR_FOR`.
#[derive(Default)]
pub(crate) struct AutoPause {
    pub(crate) load: Option<nero_core::CaptureLoad>,
    pub(crate) last_overloads: Option<u64>,
    overloads: std::collections::VecDeque<Instant>,
    reason: Option<PauseReason>,
    clear_since: Option<Instant>,
}

impl AutoPause {
    pub(crate) fn observe(&mut self, now: Instant, sample: LoadSample) -> Option<PauseReason> {
        for _ in 0..sample.new_overloads.min(OVERLOADS as u64) {
            self.overloads.push_back(now);
        }
        while self
            .overloads
            .front()
            .is_some_and(|at| now.duration_since(*at) > OVERLOAD_WINDOW)
        {
            self.overloads.pop_front();
        }
        let trigger = if sample.fill >= BUFFER_FILL {
            Some(PauseReason::Buffer)
        } else if self.overloads.len() >= OVERLOADS {
            Some(PauseReason::Overload)
        } else if sample.memory_critical {
            Some(PauseReason::Memory)
        } else if sample.thermal_serious {
            Some(PauseReason::Thermal)
        } else {
            None
        };
        match trigger {
            Some(reason) => {
                self.reason = Some(reason);
                self.clear_since = None;
            }
            None if self.reason.is_some() => {
                let since = *self.clear_since.get_or_insert(now);
                if now.duration_since(since) >= CLEAR_FOR {
                    self.reason = None;
                    self.clear_since = None;
                }
            }
            None => {}
        }
        self.reason
    }

    pub(crate) fn reset(&mut self) {
        *self = Self::default();
    }
}

/// Whether macOS reports serious or critical thermal pressure.
#[cfg(target_os = "macos")]
pub(crate) fn thermal_serious() -> bool {
    use objc2::runtime::AnyObject;
    use objc2::{class, msg_send};
    unsafe {
        let info: *mut AnyObject = msg_send![class!(NSProcessInfo), processInfo];
        if info.is_null() {
            return false;
        }
        let state: isize = msg_send![info, thermalState];
        state >= 2
    }
}
#[cfg(not(target_os = "macos"))]
pub(crate) fn thermal_serious() -> bool {
    false
}

#[cfg(target_os = "macos")]
const SIGSTOP: i32 = 17;
#[cfg(target_os = "macos")]
const SIGCONT: i32 = 19;
#[cfg(not(target_os = "macos"))]
const SIGSTOP: i32 = 0;
#[cfg(not(target_os = "macos"))]
const SIGCONT: i32 = 0;

#[cfg(target_os = "macos")]
fn set_background(pid: u32, background: bool) -> bool {
    extern "C" {
        fn setpriority(which: i32, who: u32, priority: i32) -> i32;
    }
    const PRIO_DARWIN_PROCESS: i32 = 4;
    const PRIO_DARWIN_BG: i32 = 0x1000;
    unsafe {
        setpriority(
            PRIO_DARWIN_PROCESS,
            pid,
            if background { PRIO_DARWIN_BG } else { 0 },
        ) == 0
    }
}
#[cfg(not(target_os = "macos"))]
fn set_background(_pid: u32, _background: bool) -> bool {
    true
}

thread_local! {
    static THREAD_IO_THROTTLED: Cell<bool> = const { Cell::new(false) };
}

// Disk reads by this job thread (model hashes, audio preparation) compete with the
// recording writer; throttle them while recording.
#[cfg(target_os = "macos")]
fn apply_thread_io(throttle: bool) {
    if THREAD_IO_THROTTLED.with(Cell::get) == throttle {
        return;
    }
    extern "C" {
        fn setiopolicy_np(kind: i32, scope: i32, policy: i32) -> i32;
    }
    const IOPOL_TYPE_DISK: i32 = 0;
    const IOPOL_SCOPE_THREAD: i32 = 1;
    const IOPOL_DEFAULT: i32 = 0;
    const IOPOL_THROTTLE: i32 = 3;
    let policy = if throttle { IOPOL_THROTTLE } else { IOPOL_DEFAULT };
    if unsafe { setiopolicy_np(IOPOL_TYPE_DISK, IOPOL_SCOPE_THREAD, policy) } == 0 {
        THREAD_IO_THROTTLED.with(|value| value.set(throttle));
    }
}
#[cfg(not(target_os = "macos"))]
fn apply_thread_io(_throttle: bool) {}

#[cfg(test)]
mod auto_tests {
    use super::*;

    fn sample(fill: f64, new_overloads: u64) -> LoadSample {
        LoadSample { fill, new_overloads, memory_critical: false, thermal_serious: false }
    }

    #[test]
    fn each_signal_pauses_at_once_and_only_thirty_clear_seconds_resume() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut auto = AutoPause::default();
        assert_eq!(auto.observe(at(0), sample(0.2, 0)), None);
        assert_eq!(auto.observe(at(1), sample(0.25, 0)), Some(PauseReason::Buffer));
        assert_eq!(auto.observe(at(2), sample(0.0, 0)), Some(PauseReason::Buffer));
        assert_eq!(auto.observe(at(20), sample(0.3, 0)), Some(PauseReason::Buffer));
        assert_eq!(auto.observe(at(49), sample(0.0, 0)), Some(PauseReason::Buffer));
        assert_eq!(auto.observe(at(78), sample(0.0, 0)), Some(PauseReason::Buffer));
        assert_eq!(auto.observe(at(79), sample(0.0, 0)), None);
        let mut memory = sample(0.0, 0);
        memory.memory_critical = true;
        assert_eq!(auto.observe(at(80), memory), Some(PauseReason::Memory));
        let mut thermal = sample(0.0, 0);
        thermal.thermal_serious = true;
        assert_eq!(auto.observe(at(81), thermal), Some(PauseReason::Thermal));
        auto.reset();
        assert_eq!(auto.observe(at(82), sample(0.0, 0)), None);
    }

    #[test]
    fn device_overloads_pause_only_when_three_fall_within_thirty_seconds() {
        let start = Instant::now();
        let at = |seconds: u64| start + Duration::from_secs(seconds);
        let mut auto = AutoPause::default();
        assert_eq!(auto.observe(at(0), sample(0.0, 1)), None);
        assert_eq!(auto.observe(at(20), sample(0.0, 1)), None);
        assert_eq!(auto.observe(at(40), sample(0.0, 1)), None);
        assert_eq!(auto.observe(at(45), sample(0.0, 1)), Some(PauseReason::Overload));
        let mut burst = AutoPause::default();
        assert_eq!(burst.observe(at(0), sample(0.0, 40)), Some(PauseReason::Overload));
    }
}

#[cfg(all(test, target_os = "macos"))]
mod tests {
    use super::*;
    use std::process::Command;

    fn state(pid: u32) -> String {
        let output = Command::new("/bin/ps")
            .args(["-o", "stat=", "-p", &pid.to_string()])
            .output()
            .unwrap();
        String::from_utf8(output.stdout).unwrap().trim().to_owned()
    }

    #[test]
    fn a_worker_stops_and_continues_with_its_job_and_stopped_time_is_counted() {
        let cancel = Arc::new(AtomicBool::new(false));
        let control = JobControl::new(cancel.clone(), true);
        let mut child =
            crate::process_tree::spawn(Command::new("/bin/sleep").arg("30")).unwrap();
        let mut pauser = Pauser::new(&cancel);
        pauser.sync(&child);
        assert!(control.view().is_none());
        control.set_user(true);
        assert_eq!(control.view().unwrap().state, PauseState::Pausing);
        pauser.sync(&child);
        thread::sleep(Duration::from_millis(100));
        assert!(state(child.id()).starts_with('T'), "{}", state(child.id()));
        let view = control.view().unwrap();
        assert_eq!((view.by, view.state), (PauseBy::User, PauseState::Paused));
        thread::sleep(Duration::from_millis(200));
        control.set_user(false);
        pauser.sync(&child);
        thread::sleep(Duration::from_millis(100));
        assert!(!state(child.id()).starts_with('T'));
        assert!(pauser.paused() >= Duration::from_millis(250));
        assert!(control.view().is_none());
        control.set_auto(Some(PauseReason::Buffer));
        pauser.sync(&child);
        let view = control.view().unwrap();
        assert_eq!(view.by, PauseBy::Auto);
        assert_eq!(view.reason, Some(PauseReason::Buffer));
        cancel.store(true, Ordering::Release);
        assert!(control.view().is_none());
        pauser.release(&child);
        child.kill().unwrap();
        child.wait().unwrap();
    }

    #[test]
    fn only_the_jobs_own_flag_finds_its_control_and_unpausable_jobs_never_pause() {
        let cancel = Arc::new(AtomicBool::new(false));
        let control = JobControl::new(cancel.clone(), false);
        let other = AtomicBool::new(false);
        assert!(JobControl::find(&other).is_none());
        assert!(JobControl::find(&cancel).is_some());
        control.set_user(true);
        assert!(!control.wants_pause());
        control.wait_while_paused();
        drop(control);
        assert!(JobControl::find(&cancel).is_none());
    }

    #[test]
    fn a_pause_point_waits_until_the_pause_ends_or_the_job_is_cancelled() {
        let cancel = Arc::new(AtomicBool::new(false));
        let control = JobControl::new(cancel.clone(), true);
        control.set_user(true);
        let flag = cancel.clone();
        let waiting = thread::spawn(move || pause_point(&flag));
        thread::sleep(Duration::from_millis(150));
        assert_eq!(control.view().unwrap().state, PauseState::Paused);
        assert!(!waiting.is_finished());
        cancel.store(true, Ordering::Release);
        waiting.join().unwrap();
    }
}
