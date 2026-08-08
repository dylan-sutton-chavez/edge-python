use core::cell::Cell;

const SPAWNED: u8 = 1 << 0;
const RUN_QUEUED: u8 = 1 << 1;

/// Task lifecycle bits, single-threaded so a plain Cell suffices.
pub struct State(Cell<u8>);

impl State {
    pub(crate) const fn new() -> Self {
        State(Cell::new(0))
    }

    pub(crate) fn set_spawned(&self) {
        self.0.set(self.0.get() | SPAWNED);
    }

    pub(crate) fn despawn(&self) {
        self.0.set(self.0.get() & !SPAWNED);
    }

    /// Enqueue once, deduping double wakes.
    pub(crate) fn run_enqueue(&self, f: impl FnOnce()) {
        let s = self.0.get();
        if s & RUN_QUEUED == 0 {
            self.0.set(s | RUN_QUEUED);
            f();
        }
    }

    pub(crate) fn dequeue(&self) {
        self.0.set(self.0.get() & !RUN_QUEUED);
    }
}
