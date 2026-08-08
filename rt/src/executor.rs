use alloc::boxed::Box;
use core::cell::Cell;
use core::future::Future;

use crate::run_queue::RunQueue;
use crate::task::{TaskRef, TaskStorage};

/// The entire platform frontier, blocking and wakeup.
pub trait Park {
    /// Block until unpark, must not lose a wakeup delivered before park.
    fn park(&self);
    /// Wake a parked context, safe to call re-entrantly and when not parked.
    fn unpark(&self);
}

/// Cooperative single-threaded executor, must stay at a stable address while running.
pub struct Executor {
    pub(crate) run_queue: RunQueue,
    pub(crate) park: Box<dyn Park>,
    alive: Cell<usize>,
}

impl Executor {
    pub fn new(park: Box<dyn Park>) -> Self {
        Executor {
            run_queue: RunQueue::new(),
            park,
            alive: Cell::new(0),
        }
    }

    pub fn spawn<F: Future + 'static>(&self, future: F) {
        let task = TaskStorage::alloc(future);
        task.header().executor.set(self as *const Executor);
        task.header().state.set_spawned();
        self.alive.set(self.alive.get() + 1);
        if self.run_queue.enqueue(task) {
            self.park.unpark();
        }
    }

    pub(crate) fn task_done(&self, task: TaskRef) {
        task.header().state.despawn();
        self.alive.set(self.alive.get() - 1);
    }

    // Count of spawned tasks that have not finished.
    pub fn alive(&self) -> usize {
        self.alive.get()
    }

    pub fn poll(&self) {
        self.run_queue.dequeue_all(|task| unsafe {
            (task.header().poll_fn.get().unwrap())(task);
        });
    }

    pub fn run(&self) {
        loop {
            self.poll();
            if self.alive.get() == 0 {
                break;
            }
            self.park.park();
        }
    }
}
