/* Outcome of `run_until_all_done`; surfaced via `VmErr::HostYield` for embedder resume. */
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SchedulerStatus {
    Done,
    /// Earliest wake-up deadline (ns); host arms a timer and re-enters via `run_resume`.
    PendingTimer(u64),
    /// One or more coros parked in `frame()`; host hooks `requestAnimationFrame`.
    PendingFrame,
    /// One or more coros parked in `receive()`; host waits for `run_push_event`.
    PendingEvent,
    /// Coro parked mid-`CallExtern`; host wakes via `set_host_result`.
    PendingHostCall,
    /// Interval elapsed at a back-edge; snapshottable, resumes with no host action.
    Preempted,
}
