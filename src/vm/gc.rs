use super::VM;
use super::types::*;

impl<'a> VM<'a> {

    /* Mark all reachable roots then sweep, non-heap Vals are no-op to mark. */
    pub(crate) fn collect(&mut self, current_slots: &[Val]) {
        for &v in &self.stack { self.heap.mark(v); }
        for &v in &self.with_stack { self.heap.mark(v); }
        for &v in &self.temp_roots { self.heap.mark(v); }
        for &v in &self.yields { self.heap.mark(v); }
        for &v in &self.event_queue { self.heap.mark(v); }
        // The handled exception and any pending finally return value outlive their stack slots.
        if let Some(v) = self.pending.exc_val { self.heap.mark(v); }
        self.heap.mark(self.yield_from_value);
        if let Some(v) = self.handling_exc { self.heap.mark(v); }
        for u in &self.unwind_stack { if let Unwind::Return(v) = u { self.heap.mark(*v); } }
        // Scheduler holds parked coroutines (and their `WaitingForChildren` task lists) across `top_loop` resumes, mark them so the saved state isn't swept under us.
        for handle in &self.scheduler {
            self.heap.mark(handle.coro);
            if let CoroState::WaitingForChildren { tasks, kind } = &handle.state {
                for &t in tasks { self.heap.mark(t); }
                match kind {
                    WaitKind::Run(t) => self.heap.mark(*t),
                    WaitKind::Timeout { target, .. } => self.heap.mark(*target),
                    WaitKind::Gather => {}
                }
            }
        }
        for &v in current_slots { self.heap.mark(v); }
        for &v in &self.live_slots { self.heap.mark(v); }
        // Closure cells live on the active call frames until the closures that capture them are built.
        for frame in &self.call_stack { for &(_, c) in &frame.cells { self.heap.mark(c); } }
        for &v in &self.template_roots { self.heap.mark(v); }
        for &v in self.globals.values() { self.heap.mark(v); }
        for &v in self.module_state.values() { self.heap.mark(v); }
        let heap = &mut self.heap; // split borrow, lets closures take &mut heap while iterating other fields
        for frame in &self.iter_stack { frame.for_each_val(&mut |v| heap.mark(v)); }
        for sf in &self.pending_sync_frames { sf.for_each_val(&mut |v| heap.mark(v)); }
        for cache in self.opcode_caches.values() {
            if let Some(consts) = cache.const_vals_opt() {
                for &v in consts { self.heap.mark(v); }
            }
            // keep the IC's cached class + method Vals alive so a promoted slot can't reference a swept-and-reused slot.
            for v in cache.inst_roots() { self.heap.mark(v); }
        }
        // SAFETY each ptr is live for its exec() frame and the Vec's alloc is move-stable.
        for i in 0..self.active_const_pools.len() {
            let consts: &[Val] = unsafe { &*self.active_const_pools[i] };
            for &v in consts { self.heap.mark(v); }
        }
        // SAFETY same invariant, roots every active frame's live (mutating) slots, not just the innermost current_slots.
        for i in 0..self.active_slots.len() {
            let frame_slots: &[Val] = unsafe { &*self.active_slots[i] };
            for &v in frame_slots { self.heap.mark(v); }
        }
        self.templates.mark_all(&mut self.heap);
        self.heap.sweep();
    }
}
