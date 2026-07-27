use alloc::vec::Vec;

use super::Val;
use super::HeapPool;
use super::err::VmErr;

/* Scheduler state per coroutine; stepped round-robin until the target leaves Ready/Sleeping. */
#[derive(Clone, Debug)]
pub enum CoroState {
    /// Resumable on next tick.
    Ready,
    /// Suspended until `until_ns`; the scheduler fast-forwards when all are Sleeping.
    Sleeping(u64),
    /// Parked waiting for the host's next render frame; resumed when the embedder calls back.
    WaitingFrame,
    /// Parked in `receive()` with an empty queue; resumed when the host pushes a message.
    WaitingEvent,
    /// Parked mid-`CallExtern` with its correlation id; resumed when the host calls `set_host_result_by_id(id)`.
    WaitingHostCall(u64),
    /// Parked in `run(...)` / `gather(...)` / `with_timeout(...)` until `tasks` all terminate; `kind` selects how to finalize.
    WaitingForChildren { tasks: Vec<Val>, kind: WaitKind },
    /// Next resume injects a `CancelledError` raise.
    CancelPending,
    /// Returned with this Val.
    Done(Val),
    /// Raised; stored verbatim for `gather` / `with_timeout`.
    Errored(VmErr),
    /// Cancellation already observed; yields `None` to gather() peers.
    Cancelled,
}

/* How `WaitingForChildren` finalizes when its tasks all reach terminal: `Run` returns target's value (or its error); `Gather` returns a list of all values (or the first error); `Timeout` returns the target's value, or `TimeoutError` if the deadline expired before completion. */
#[derive(Clone, Debug)]
pub enum WaitKind {
    Run(Val),
    Gather,
    Timeout { deadline_ns: u64, target: Val },
}

#[derive(Clone, Debug)]
pub struct CoroutineHandle {
    /// User-provided Coroutine HeapObj.
    pub coro: Val,
    pub state: CoroState,
}

// Suspended sync helper frame: a plain user fn called from a coroutine hit a yielding builtin mid-execution, so its state is snapshotted and parked on the enclosing Coroutine. Frames stack innermost-last; resume walks inside-out so each return value lands on the next frame's stack at the Call site. `exception_delta` carries the helper's try/except frames pushed in its exec so they survive the yield.
#[derive(Clone, Debug)]
pub struct SyncFrame {
    pub ip: usize,
    pub fi: usize,
    pub slots: Vec<Val>,
    pub stack_delta: Vec<Val>,
    pub iter_delta: Vec<IterFrame>,
    pub exception_delta: Vec<ExceptionFrame>,
}

/* Block-stack frame role: Except catches exceptions; Finally runs on every exit path. */
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum BlockKind { Except, Finally }

/* One reason a finally/with cleanup body is running; pushed on entry, popped by EndFinally. */
#[derive(Clone, Debug)]
pub enum Unwind {
    // Reached by normal fall-through; EndFinally just continues.
    Normal,
    Return(Val),
    // break/continue: run `remaining` more cleanups, then resume at `target`.
    Goto { target: usize, remaining: u16 },
    Reraise(VmErr),
}

/* Saved stack/iter/with/unwind depths for unwinding to a handler. Stored on the active Coroutine so `try`/`except` survives yields. */
#[derive(Clone, Debug)]
pub struct ExceptionFrame {
    pub kind: BlockKind,
    pub handler_ip: usize,
    pub stack_depth: usize,
    pub iter_depth: usize,
    pub with_depth: usize,
    pub unwind_depth: usize,
}

// Coroutine body: user fn (Fn) or the implicit module-body coro (Module -> self.chunk).
#[derive(Clone, Copy, Debug)]
pub enum BodyRef {
    Fn(usize),
    Module,
}

/* Call-site snapshot for traceback rendering; pushed by `exec_call`, popped on return/error. */
#[derive(Clone, Debug)]
pub struct CallFrame {
    pub fi: usize,
    pub call_byte_pos: u32,
    pub caller_source: alloc::sync::Arc<alloc::string::String>,
    pub caller_path: alloc::sync::Arc<alloc::string::String>,
    // Class where the running method was found and its implicit `self`; consumed by `super()` to walk one level up. `None` for plain function calls.
    pub current_class: Option<Val>,
    pub current_self: Option<Val>,
    // Closure cells created by MakeFunction in this frame, keyed by canonical slot. Lets sibling closures over the same enclosing variable share one cell (CPython cell semantics).
    pub cells: alloc::vec::Vec<(usize, Val)>,
}

/* ForIter state, consumed one item per `next_item`. */
#[derive(Clone, Debug)]
pub enum IterFrame {
    Seq { items: Vec<Val>, idx: usize },
    Range { cur: i64, end: i64, step: i64 },
    Coroutine(Val),
    // User-defined iterator: holds the value returned by `__iter__`; each step calls its `__next__`.
    UserDefined(Val),
}

impl IterFrame {
    /* Stateless steps only, built-in Seq/Range. User-defined iterators step in `dispatch.rs` because they need the VM to invoke `__next__`. */
    pub fn next_item(&mut self, heap: &mut HeapPool) -> Result<Option<Val>, VmErr> {
        match self {
            Self::Coroutine(_) | Self::UserDefined(_) => Ok(None),
            Self::Seq { items, idx } => {
                if *idx < items.len() { let v = items[*idx]; *idx += 1; Ok(Some(v)) } else { Ok(None) }
            }
            Self::Range { cur, end, step } => {
                let done = if *step > 0 { *cur >= *end } else { *cur <= *end };
                if done { Ok(None) } else {
                    let v = *cur;
                    // Clamp past the i64 edge so the next `done` check ends the range, never overflows.
                    *cur = cur.checked_add(*step).unwrap_or(if *step > 0 { i64::MAX } else { i64::MIN });
                    // Promote magnitudes beyond the 47-bit inline range to LongInt.
                    Ok(Some(crate::modules::vm::builtins::sequence::range_int(heap, v)?))
                }
            }
        }
    }

    /* Visit each Val in this frame; Range holds none. */
    pub(crate) fn for_each_val(&self, f: &mut impl FnMut(Val)) {
        match self {
            IterFrame::Seq { items, .. } => for &v in items { f(v); },
            Self::Coroutine(v) | Self::UserDefined(v) => f(*v),
            IterFrame::Range { .. } => {}
        }
    }
}

impl SyncFrame {
    /* Visit all Vals across slots, stack delta, and iter frames. */
    pub(crate) fn for_each_val(&self, f: &mut impl FnMut(Val)) {
        for &v in &self.slots { f(v); }
        for &v in &self.stack_delta { f(v); }
        for fr in &self.iter_delta { fr.for_each_val(f); }
    }
}
