use core::cell::{Cell, UnsafeCell};
use core::future::Future;
use core::mem::{self, MaybeUninit};
use core::pin::Pin;
use core::ptr::NonNull;
use core::task::Context;

use crate::executor::Executor;
use crate::state::State;
use crate::waker;

/// Lazily initialized storage for a future.
pub struct UninitCell<T>(UnsafeCell<MaybeUninit<T>>);

impl<T> UninitCell<T> {
    pub const fn uninit() -> Self {
        UninitCell(UnsafeCell::new(MaybeUninit::uninit()))
    }

    pub unsafe fn write(&self, val: T) {
        unsafe { (*self.0.get()).as_mut_ptr().write(val) };
    }

    #[allow(clippy::mut_from_ref)]
    pub unsafe fn as_mut(&self) -> &mut T {
        unsafe { &mut *(*self.0.get()).as_mut_ptr() }
    }

    pub unsafe fn drop_in_place(&self) {
        unsafe { core::ptr::drop_in_place((*self.0.get()).as_mut_ptr()) };
    }
}

/// Fixed head shared by every task, holds the intrusive run-queue link.
pub struct TaskHeader {
    pub(crate) state: State,
    pub(crate) next: Cell<Option<TaskRef>>,
    pub(crate) executor: Cell<*const Executor>,
    pub(crate) poll_fn: Cell<Option<unsafe fn(TaskRef)>>,
}

/// One-word handle to a task, the waker data pointer.
#[derive(Clone, Copy)]
pub struct TaskRef(NonNull<TaskHeader>);

impl TaskRef {
    pub(crate) fn header(self) -> &'static TaskHeader {
        unsafe { self.0.as_ref() }
    }

    pub(crate) fn as_ptr(self) -> *const () {
        self.0.as_ptr() as *const ()
    }

    pub(crate) unsafe fn from_ptr(ptr: *const ()) -> Self {
        TaskRef(unsafe { NonNull::new_unchecked(ptr as *mut TaskHeader) })
    }
}

/// Header at offset 0 so a storage pointer casts to a header pointer.
#[repr(C)]
pub struct TaskStorage<F: Future> {
    header: TaskHeader,
    future: UninitCell<F>,
}

unsafe fn poll_exited(_task: TaskRef) {}

impl<F: Future + 'static> TaskStorage<F> {
    fn new() -> Self {
        TaskStorage {
            header: TaskHeader {
                state: State::new(),
                next: Cell::new(None),
                executor: Cell::new(core::ptr::null()),
                poll_fn: Cell::new(None),
            },
            future: UninitCell::uninit(),
        }
    }

    // Alloc-based spawn fits a host that wants many tasks, no static pool bound.
    pub(crate) fn alloc(future: F) -> TaskRef {
        let storage: &'static mut TaskStorage<F> =
            alloc::boxed::Box::leak(alloc::boxed::Box::new(Self::new()));
        unsafe {
            storage.future.write(future);
        }
        storage.header.poll_fn.set(Some(Self::poll_fn));
        TaskRef(NonNull::from(&storage.header))
    }

    unsafe fn poll_fn(task: TaskRef) {
        let storage = task.as_ptr() as *const TaskStorage<F>;
        let future = unsafe { Pin::new_unchecked((*storage).future.as_mut()) };
        let waker = waker::from_task(task);
        let mut cx = Context::from_waker(&waker);
        if future.poll(&mut cx).is_ready() {
            unsafe { (*storage).future.drop_in_place() };
            task.header().poll_fn.set(Some(poll_exited));
            let ex = unsafe { &*task.header().executor.get() };
            ex.task_done(task);
        }
        // Drop is a noop so skip it.
        mem::forget(waker);
    }
}
