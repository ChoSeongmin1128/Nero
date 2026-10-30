//! One process-lifetime owner for quitting or replacing the application.
//! Admission checks and ownership changes share a mutex; an IPC admitted before
//! either action must still acquire a lease before it can close the app.
use std::sync::{Arc, Mutex};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ExitKind {
    Quit,
    Update,
}

#[derive(Default)]
struct State {
    owner: Option<(u64, ExitKind)>,
    next: u64,
    transitions: usize,
}

#[derive(Clone, Default)]
pub struct Lifecycle(Arc<Mutex<State>>);

impl Lifecycle {
    pub fn is_closing(&self) -> bool {
        self.0.lock().unwrap().owner.is_some()
    }

    pub fn begin_exit(&self, kind: ExitKind) -> Result<ExitLease, String> {
        let mut state = self.0.lock().unwrap();
        if state.owner.is_some() {
            return Err("앱을 종료하거나 업데이트하고 있습니다. 잠시 기다려 주세요.".into());
        }
        if state.transitions != 0 {
            return Err(
                "녹음 상태와 후속 작업을 저장하고 있습니다. 잠시 후 다시 시도해 주세요.".into(),
            );
        }
        let token = state.next.checked_add(1).ok_or("앱을 다시 열어 주세요.")?;
        state.next = token;
        state.owner = Some((token, kind));
        Ok(ExitLease {
            lifecycle: self.clone(),
            token,
            kind,
            armed: true,
        })
    }

    /// Keeps stop -> completed snapshot -> automatic transcription admission
    /// indivisible with respect to both update installation and normal quit.
    pub fn begin_transition(&self) -> Result<TransitionLease, String> {
        let mut state = self.0.lock().unwrap();
        if state.owner.is_some() {
            return Err("앱을 종료하거나 업데이트하고 있습니다. 잠시 기다려 주세요.".into());
        }
        state.transitions = state
            .transitions
            .checked_add(1)
            .ok_or("작업 상태를 확인하지 못했습니다.")?;
        Ok(TransitionLease {
            lifecycle: self.clone(),
        })
    }
}

pub struct ExitLease {
    lifecycle: Lifecycle,
    token: u64,
    kind: ExitKind,
    armed: bool,
}
impl ExitLease {
    pub fn owns(&self, kind: ExitKind) -> bool {
        self.kind == kind && self.lifecycle.0.lock().unwrap().owner == Some((self.token, kind))
    }
    /// Successful exit/restart keeps ownership after this stack frame returns.
    /// Only process termination releases it; no competing path can take over.
    pub fn commit(mut self) {
        self.armed = false;
    }
}
impl Drop for ExitLease {
    fn drop(&mut self) {
        if self.armed {
            let mut state = self.lifecycle.0.lock().unwrap();
            if state.owner == Some((self.token, self.kind)) {
                state.owner = None;
            }
        }
    }
}

pub struct TransitionLease {
    lifecycle: Lifecycle,
}
impl Drop for TransitionLease {
    fn drop(&mut self) {
        self.lifecycle.0.lock().unwrap().transitions -= 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{sync::mpsc, thread};

    #[test]
    fn simultaneous_quit_and_update_have_exactly_one_owner() {
        let lifecycle = Lifecycle::default();
        let begin = Arc::new(std::sync::Barrier::new(3));
        let release = Arc::new(std::sync::Barrier::new(3));
        let (sender, results) = mpsc::channel();
        let threads: Vec<_> = [ExitKind::Quit, ExitKind::Update]
            .into_iter()
            .map(|kind| {
                let lifecycle = lifecycle.clone();
                let begin = begin.clone();
                let release = release.clone();
                let sender = sender.clone();
                thread::spawn(move || {
                    begin.wait();
                    let lease = lifecycle.begin_exit(kind);
                    sender.send(lease.is_ok()).unwrap();
                    release.wait();
                    drop(lease);
                })
            })
            .collect();
        begin.wait();
        assert_eq!(
            [results.recv().unwrap(), results.recv().unwrap()]
                .into_iter()
                .filter(|owned| *owned)
                .count(),
            1
        );
        assert!(lifecycle.is_closing());
        release.wait();
        for thread in threads {
            thread.join().unwrap();
        }
        assert!(!lifecycle.is_closing());
    }

    #[test]
    fn already_admitted_quit_cannot_take_over_an_update_owner() {
        let lifecycle = Lifecycle::default();
        // The quit handler has passed its UI check but has not acquired a lease.
        let (release, allowed) = mpsc::channel();
        let delayed = lifecycle.clone();
        let quit = thread::spawn(move || {
            allowed.recv().unwrap();
            delayed.begin_exit(ExitKind::Quit).is_err()
        });
        let update = lifecycle.begin_exit(ExitKind::Update).unwrap();
        release.send(()).unwrap();
        assert!(quit.join().unwrap());
        assert!(update.owns(ExitKind::Update));
        drop(update);
        lifecycle.begin_exit(ExitKind::Quit).unwrap().commit();
        assert!(lifecycle.is_closing());
        assert!(lifecycle.begin_exit(ExitKind::Update).is_err());
        assert!(lifecycle.begin_exit(ExitKind::Quit).is_err());
    }

    #[test]
    fn quit_ownership_survives_successful_handler_return_until_process_exit() {
        let lifecycle = Lifecycle::default();
        lifecycle.begin_exit(ExitKind::Quit).unwrap().commit();
        assert!(lifecycle.begin_exit(ExitKind::Update).is_err());
        assert!(lifecycle.begin_transition().is_err());
    }

    #[tokio::test]
    async fn dropped_ipc_waiter_cannot_release_its_running_shutdown_task() {
        let lifecycle = Lifecycle::default();
        let lease = lifecycle.begin_exit(ExitKind::Quit).unwrap();
        let (started, ready) = tokio::sync::oneshot::channel();
        let (release, allowed) = mpsc::channel();
        let (finished, done) = tokio::sync::oneshot::channel();
        let work = tokio::task::spawn_blocking(move || {
            started.send(()).unwrap();
            allowed.recv().unwrap();
            lease.commit();
            finished.send(()).unwrap();
        });
        ready.await.unwrap();
        let waiter = tokio::spawn(async move {
            work.await.unwrap();
        });
        waiter.abort();
        let _ = waiter.await;
        assert!(lifecycle.begin_exit(ExitKind::Update).is_err());
        release.send(()).unwrap();
        done.await.unwrap();
        assert!(lifecycle.is_closing());
        assert!(lifecycle.begin_transition().is_err());
    }

    #[test]
    fn failure_releases_only_its_own_token() {
        let lifecycle = Lifecycle::default();
        let old = lifecycle.begin_exit(ExitKind::Update).unwrap();
        // Simulate a stale cleanup callback without providing such an operation
        // in the production API. Its old token cannot release a newer owner.
        lifecycle.0.lock().unwrap().owner = None;
        let quit = lifecycle.begin_exit(ExitKind::Quit).unwrap();
        drop(old);
        assert!(quit.owns(ExitKind::Quit));
        drop(quit);
        assert!(!lifecycle.is_closing());
    }

    #[test]
    fn recording_handoff_blocks_exit_until_registration_even_after_capture_finishes() {
        let lifecycle = Lifecycle::default();
        let transition = lifecycle.begin_transition().unwrap();
        let (finished_capture, capture_finished) = mpsc::channel();
        let (register, registration) = mpsc::channel();
        let background = thread::spawn(move || {
            let _transition = transition;
            finished_capture.send(()).unwrap();
            registration.recv().unwrap();
            // The real blocking task retains its lease until transcribe_recording
            // has either registered the job or emitted its failure.
        });
        capture_finished.recv().unwrap();
        assert!(lifecycle.begin_exit(ExitKind::Update).is_err());
        assert!(lifecycle.begin_exit(ExitKind::Quit).is_err());
        register.send(()).unwrap();
        background.join().unwrap();
        lifecycle.begin_exit(ExitKind::Update).unwrap().commit();
        assert!(lifecycle.begin_transition().is_err());
    }
}
