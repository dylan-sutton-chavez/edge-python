use rt::{Executor, Park, WakerCell};
use std::cell::{Cell, RefCell};
use std::future::Future;
use std::pin::Pin;
use std::rc::Rc;
use std::sync::{Condvar, Mutex};
use std::task::{Context, Poll, Waker};

// Flag plus condvar Park, retains a wakeup delivered before park.
struct CondPark {
    flag: Mutex<bool>,
    cv: Condvar,
}

impl CondPark {
    fn boxed() -> Box<dyn Park> {
        Box::new(CondPark {
            flag: Mutex::new(false),
            cv: Condvar::new(),
        })
    }
}

impl Park for CondPark {
    fn park(&self) {
        let mut flag = self.flag.lock().unwrap();
        while !*flag {
            flag = self.cv.wait(flag).unwrap();
        }
        *flag = false;
    }

    fn unpark(&self) {
        let mut flag = self.flag.lock().unwrap();
        *flag = true;
        self.cv.notify_one();
    }
}

struct PendOnce {
    cell: Rc<WakerCell>,
    done: Rc<Cell<bool>>,
    polls: Rc<Cell<u32>>,
    first: bool,
}

impl Future for PendOnce {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        let this = self.get_mut();
        this.polls.set(this.polls.get() + 1);
        if this.first {
            this.first = false;
            this.cell.register(cx.waker());
            Poll::Pending
        } else {
            this.done.set(true);
            Poll::Ready(())
        }
    }
}

struct Sink {
    polls: Rc<Cell<u32>>,
    waker: Rc<RefCell<Option<Waker>>>,
}

impl Future for Sink {
    type Output = ();
    fn poll(self: Pin<&mut Self>, cx: &mut Context) -> Poll<()> {
        self.polls.set(self.polls.get() + 1);
        *self.waker.borrow_mut() = Some(cx.waker().clone());
        Poll::Pending
    }
}

#[test]
fn ready_immediately_completes() {
    let done = Rc::new(Cell::new(false));
    let ex = Executor::new(CondPark::boxed());
    let d = done.clone();
    ex.spawn(async move {
        d.set(true);
    });
    ex.poll();
    assert!(done.get());
}

#[test]
fn pending_then_woken_completes() {
    let cell = Rc::new(WakerCell::new());
    let done = Rc::new(Cell::new(false));
    let polls = Rc::new(Cell::new(0u32));
    let ex = Executor::new(CondPark::boxed());
    ex.spawn(PendOnce {
        cell: cell.clone(),
        done: done.clone(),
        polls: polls.clone(),
        first: true,
    });
    ex.poll();
    assert!(!done.get());
    assert_eq!(polls.get(), 1);
    cell.wake();
    ex.poll();
    assert!(done.get());
    assert_eq!(polls.get(), 2);
}

#[test]
fn double_wake_enqueues_once() {
    let polls = Rc::new(Cell::new(0u32));
    let waker = Rc::new(RefCell::new(None));
    let ex = Executor::new(CondPark::boxed());
    ex.spawn(Sink {
        polls: polls.clone(),
        waker: waker.clone(),
    });
    ex.poll();
    assert_eq!(polls.get(), 1);
    let w = waker.borrow().clone().unwrap();
    w.wake_by_ref();
    w.wake_by_ref();
    w.wake_by_ref();
    ex.poll();
    assert_eq!(polls.get(), 2);
}

#[test]
fn run_returns_when_all_done() {
    let done = Rc::new(Cell::new(false));
    let ex = Executor::new(CondPark::boxed());
    let d = done.clone();
    ex.spawn(async move {
        d.set(true);
    });
    ex.run();
    assert!(done.get());
}
