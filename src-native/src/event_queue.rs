//! Bounded FIFO with event-driven waiting; the predicate and notification share
//! one mutex so an event arriving just before sleep cannot be lost.
use std::{
    collections::VecDeque,
    sync::{Condvar, Mutex},
    time::Instant,
};
const CAPACITY: usize = 256;

struct State<T> {
    events: VecDeque<T>,
    stopped: bool,
    #[cfg(feature = "native-core")]
    consumer: Option<std::thread::Thread>,
}

pub struct Queue<T> {
    state: Mutex<State<T>>,
    changed: Condvar,
}

impl<T> Default for Queue<T> {
    fn default() -> Self {
        Self {
            state: Mutex::new(State {
                events: VecDeque::with_capacity(CAPACITY),
                stopped: false,
                #[cfg(feature = "native-core")]
                consumer: None,
            }),
            changed: Condvar::new(),
        }
    }
}

impl<T> Queue<T> {
    pub fn push(&self, event: T) {
        self.try_push(event);
    }

    pub fn try_push(&self, event: T) -> bool {
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        if !state.stopped && state.events.len() < CAPACITY {
            state.events.push_back(event);
            #[cfg(feature = "native-core")]
            if let Some(consumer) = &state.consumer {
                consumer.unpark();
            }
            drop(state);
            self.changed.notify_one();
            true
        } else {
            false
        }
    }

    #[cfg(feature = "native-core")]
    pub fn set_consumer(&self, consumer: std::thread::Thread) {
        self.state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .consumer = Some(consumer);
    }

    #[cfg(feature = "native-core")]
    pub fn wake_consumer(&self) {
        if let Some(consumer) = &self
            .state
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .consumer
        {
            consumer.unpark();
        }
    }

    /// Swap storage in constant time; the consumer processes outside the input
    /// mutex. Both bounded allocations are retained across successive batches.
    pub fn take_batch(&self, batch: &mut VecDeque<T>) {
        assert!(
            batch.is_empty(),
            "input batch must be consumed before reuse"
        );
        batch.reserve(CAPACITY);
        let mut state = self.state.lock().unwrap_or_else(|e| e.into_inner());
        std::mem::swap(&mut state.events, batch);
    }

    #[cfg(test)]
    pub fn drain(&self) -> Vec<T> {
        let mut batch = VecDeque::new();
        self.take_batch(&mut batch);
        batch.into_iter().collect()
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
        #[cfg(feature = "native-core")]
        self.wake_consumer();
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
    #[cfg(feature = "native-core")]
    #[test]
    fn native_consumer_is_woken_for_input_and_stop_without_polling() {
        let queue = Arc::new(Queue::<u8>::default());
        let (ready, started) = mpsc::channel();
        let q = queue.clone();
        let worker = std::thread::spawn(move || {
            q.set_consumer(std::thread::current());
            ready.send(()).unwrap();
            std::thread::park();
            q.drain()
        });
        started.recv_timeout(Duration::from_secs(1)).unwrap();
        queue.push(7);
        assert_eq!(worker.join().unwrap(), vec![7]);
        queue.stop();
        assert!(!queue.try_push(9));
    }

    #[test]
    fn repeated_full_batches_reuse_two_allocations_and_preserve_fifo() {
        let queue = Queue::default();
        let mut batch = VecDeque::new();
        let mut addresses = std::collections::BTreeSet::new();
        for round in 0..128 {
            for item in 0..CAPACITY {
                queue.push(round * CAPACITY + item);
            }
            queue.take_batch(&mut batch);
            assert_eq!(batch.len(), CAPACITY);
            addresses.insert(batch.as_slices().0.as_ptr() as usize);
            for item in 0..CAPACITY {
                assert_eq!(batch.pop_front(), Some(round * CAPACITY + item));
            }
        }
        assert_eq!(addresses.len(), 2, "input storage was reallocated");
    }

    #[test]
    fn producer_can_fill_next_batch_while_consumer_owns_previous_one() {
        let queue = Arc::new(Queue::default());
        queue.push(1);
        let mut batch = VecDeque::new();
        queue.take_batch(&mut batch);
        let producer = queue.clone();
        let (tx, rx) = mpsc::channel();
        let worker = std::thread::spawn(move || {
            producer.push(2);
            producer.push(3);
            tx.send(()).unwrap();
        });
        rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert_eq!(batch.pop_front(), Some(1));
        queue.take_batch(&mut batch);
        assert_eq!(batch.into_iter().collect::<Vec<_>>(), vec![2, 3]);
        worker.join().unwrap();
    }

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
            assert_eq!(queue.try_push(i), i < 256);
        }
        assert_eq!(queue.drain(), (0..256).collect::<Vec<_>>());
        let deadline = Instant::now() + Duration::from_millis(10);
        queue.wait_until(deadline);
        assert!(Instant::now() >= deadline);
        queue.stop();
        assert!(!queue.try_push(42));
        assert!(queue.drain().is_empty());
    }
}
