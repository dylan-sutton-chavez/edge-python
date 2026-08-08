use core::cell::Cell;
use core::task::Waker;

/// Holds a leaf future's waker so an external event can wake it later.
pub struct WakerCell {
    waker: Cell<Option<Waker>>,
}

impl Default for WakerCell {
    fn default() -> Self {
        Self::new()
    }
}

impl WakerCell {
    pub const fn new() -> Self {
        WakerCell {
            waker: Cell::new(None),
        }
    }

    /// Store the waker, waking any displaced one.
    pub fn register(&self, w: &Waker) {
        let prev = self.waker.take();
        match prev {
            Some(old) if old.will_wake(w) => self.waker.set(Some(old)),
            Some(old) => {
                self.waker.set(Some(w.clone()));
                old.wake();
            }
            None => self.waker.set(Some(w.clone())),
        }
    }

    /// Take and wake the stored waker.
    pub fn wake(&self) {
        if let Some(w) = self.waker.take() {
            w.wake();
        }
    }
}
