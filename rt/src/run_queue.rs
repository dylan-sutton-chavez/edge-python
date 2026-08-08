use core::cell::Cell;

use crate::task::TaskRef;

/// Intrusive singly-linked stack of ready tasks, single-threaded.
pub struct RunQueue {
    head: Cell<Option<TaskRef>>,
}

impl RunQueue {
    pub const fn new() -> Self {
        RunQueue {
            head: Cell::new(None),
        }
    }

    /// Push a task, returning true when the queue was empty.
    pub fn enqueue(&self, task: TaskRef) -> bool {
        let prev = self.head.replace(Some(task));
        task.header().next.set(prev);
        prev.is_none()
    }

    // Drain one batch, a self-waking task lands in the next, never starving others.
    pub fn dequeue_all(&self, on_task: impl Fn(TaskRef)) {
        let mut next = self.head.take();
        while let Some(task) = next {
            next = task.header().next.get();
            task.header().state.dequeue();
            on_task(task);
        }
    }
}
