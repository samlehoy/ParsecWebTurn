//! Bounded FIFO with event-driven waiting; the predicate and notification share
//! one mutex so an event arriving just before sleep cannot be lost.
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::Instant,
};

struct State<T> {
    events: VecDeque<T>,
    stopped: bool,
}

pub struct Queue<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                events: VecDeque::new(),
                stopped: false,
            }),
            changed: Condvar::new(),
        }
    }
}

impl<T> Queue<T> {
    pub fn push(&self, event: T) {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.stopped && state.events.len() < 256 {
            state.events.push_back(event);
            self.changed.notify_one();
        }
    }

    pub fn drain(&self) -> Vec<T> {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .events
            .drain(..)
            .collect()
    }

    pub fn wait_until(&self, deadline: Instant) {
        let state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        drop(
            self.changed
                .wait_timeout_while(
                    state,
                    deadline.saturating_duration_since(Instant::now()),
                    |s| s.events.is_empty() && !s.stopped,
                )
                .unwrap_or_else(|e| e.into_inner()),
        );
    }

    pub fn stop(&self) {
        self.state.lock().unwrap_or_else(|e| e.into_inner()).stopped = true;
        self.changed.notify_all();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{
        sync::{mpsc, Arc},
        time::Duration,
    };

    #[test]
    fn queued_events_and_shutdown_are_not_lost_before_wait() {
        for stopped in [false, true] {
            let queue = Arc::new(Queue::default());
            if stopped {
                queue.stop();
            } else {
                queue.push(7);
            }
            let worker = queue.clone();
            let (tx, rx) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                worker.wait_until(Instant::now() + Duration::from_secs(5));
                tx.send(worker.drain()).unwrap();
            });
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                if stopped { vec![] } else { vec![7] }
            );
            thread.join().unwrap();
        }
    }

    #[test]
    fn arrival_and_shutdown_interrupt_wait_but_empty_notifications_do_not() {
        for stopped in [false, true] {
            let queue = Arc::new(Queue::<u32>::default());
            let worker = queue.clone();
            let (tx, rx) = mpsc::channel();
            let thread = std::thread::spawn(move || {
                worker.wait_until(Instant::now() + Duration::from_secs(5));
                tx.send(worker.drain()).unwrap();
            });
            queue.changed.notify_all();
            assert!(matches!(
                rx.recv_timeout(Duration::from_millis(20)),
                Err(mpsc::RecvTimeoutError::Timeout)
            ));
            if stopped {
                queue.stop();
            } else {
                queue.push(9);
            }
            assert_eq!(
                rx.recv_timeout(Duration::from_secs(1)).unwrap(),
                if stopped { vec![] } else { vec![9] }
            );
            thread.join().unwrap();
        }
    }

    #[test]
    fn capacity_order_and_idle_deadline_are_preserved() {
        let queue = Queue::default();
        for i in 0..300 {
            queue.push(i);
        }
        assert_eq!(queue.drain(), (0..256).collect::<Vec<_>>());
        let deadline = Instant::now() + Duration::from_millis(10);
        queue.wait_until(deadline);
        assert!(Instant::now() >= deadline);
        queue.stop();
        queue.push(42);
        assert!(queue.drain().is_empty());
    }
}
