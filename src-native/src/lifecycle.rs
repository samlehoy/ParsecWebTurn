//! One cancellation signal shared by the UI and session workers.
use std::sync::Mutex;
#[cfg(test)]
use std::{sync::Condvar, time::Duration};

#[derive(Default)]
pub struct StopSignal {
    stopped: Mutex<bool>,
    #[cfg(test)]
    changed: Condvar,
}
impl StopSignal {
    pub fn stop(&self) {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner()) = true;
        #[cfg(test)]
        self.changed.notify_all();
    }
    pub fn stopped(&self) -> bool {
        *self.stopped.lock().unwrap_or_else(|e| e.into_inner())
    }
    #[cfg(test)]
    pub fn wait_timeout(&self, timeout: Duration) -> bool {
        let guard = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        *self
            .changed
            .wait_timeout_while(guard, timeout, |stop| !*stop)
            .unwrap_or_else(|e| e.into_inner())
            .0
    }
    #[cfg(test)]
    pub fn wait(&self) {
        let guard = self.stopped.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.changed
                .wait_while(guard, |stop| !*stop)
                .unwrap_or_else(|e| e.into_inner()),
        );
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    #[test]
    fn cancellation_wakes_all_waiters_and_is_not_lost() {
        let signal = Arc::new(StopSignal::default());
        assert!(!signal.wait_timeout(Duration::from_millis(1)));
        let workers: Vec<_> = (0..4)
            .map(|_| {
                let signal = signal.clone();
                std::thread::spawn(move || {
                    signal.wait();
                    assert!(signal.stopped());
                })
            })
            .collect();
        signal.stop();
        for worker in workers {
            worker.join().unwrap();
        }
        assert!(signal.wait_timeout(Duration::from_secs(1)));
    }
}
