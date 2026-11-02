use super::{Error, Result};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

#[derive(Default)]
struct State {
    closed: bool,
    operations: usize,
}
#[derive(Default)]
pub(super) struct Lifecycle(Mutex<State>);
pub struct NotionShutdownGuard {
    lifecycle: Arc<Lifecycle>,
}
pub(super) struct OperationGuard {
    lifecycle: Arc<Lifecycle>,
}

impl Lifecycle {
    pub fn check_open(&self) -> Result<()> {
        if self.0.lock().unwrap().closed {
            Err(Error::Shutdown)
        } else {
            Ok(())
        }
    }
    pub fn enter(self: &Arc<Self>) -> Result<OperationGuard> {
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return Err(Error::Shutdown);
        }
        state.operations += 1;
        Ok(OperationGuard {
            lifecycle: self.clone(),
        })
    }
    pub fn close(self: &Arc<Self>) -> Result<NotionShutdownGuard> {
        let mut state = self.0.lock().unwrap();
        if state.closed {
            return Err(Error::Shutdown);
        }
        state.closed = true;
        Ok(NotionShutdownGuard {
            lifecycle: self.clone(),
        })
    }
    pub async fn drain(&self, timeout: Duration) -> Result<()> {
        let deadline = Instant::now() + timeout;
        loop {
            if self.0.lock().unwrap().operations == 0 {
                return Ok(());
            }
            if Instant::now() >= deadline {
                return Err(Error::Timeout);
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
}
impl Drop for NotionShutdownGuard {
    fn drop(&mut self) {
        self.lifecycle.0.lock().unwrap().closed = false;
    }
}
impl Drop for OperationGuard {
    fn drop(&mut self) {
        self.lifecycle.0.lock().unwrap().operations -= 1;
    }
}
