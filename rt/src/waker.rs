use core::task::{RawWaker, RawWakerVTable, Waker};

use crate::task::TaskRef;

static VTABLE: RawWakerVTable = RawWakerVTable::new(clone, wake, wake, drop_fn);

unsafe fn clone(p: *const ()) -> RawWaker {
    RawWaker::new(p, &VTABLE)
}

unsafe fn wake(p: *const ()) {
    wake_task(unsafe { TaskRef::from_ptr(p) });
}

unsafe fn drop_fn(_p: *const ()) {}

/// Build a waker whose data pointer is the task, one word.
pub fn from_task(task: TaskRef) -> Waker {
    unsafe { Waker::from_raw(RawWaker::new(task.as_ptr(), &VTABLE)) }
}

/// Enqueue the task, unparking only on an empty to nonempty transition.
pub fn wake_task(task: TaskRef) {
    task.header().state.run_enqueue(|| unsafe {
        let ex = &*task.header().executor.get();
        if ex.run_queue.enqueue(task) {
            ex.park.unpark();
        }
    });
}
