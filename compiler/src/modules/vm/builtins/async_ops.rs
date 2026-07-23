use alloc::{vec, vec::Vec, rc::Rc};
use core::cell::RefCell;

use crate::modules::parser::SSAChunk;
use super::super::VM;
use super::super::types::*;

impl<'a> VM<'a> {

    // Resume coroutine: persist state on yield, restore caller on return. Suspended sync sub-frames run innermost-first, each pushing its result onto the next frame's stack at the Call site. The coro's `exception_frames` are restored before its body runs and saved back on yield, so `try`/`except` survives suspensions.
    pub fn resume_coroutine(&mut self, callee: Val) -> Result<Val, VmErr> {
        // Scheduler-driven resumes have nothing native above.
        let resume_safe = core::mem::take(&mut self.pending_exec_safe);
        let (outer_ip, mut outer_slots, outer_stack, outer_body, outer_iters, mut sync_frames, outer_exc) =
            if let HeapObj::Coroutine(ip, slots, stack, body, iters, sf, ef) = self.heap.get(callee) {
                (*ip, slots.clone(), stack.clone(), *body, iters.clone(), sf.clone(), ef.clone())
            } else {
                return Err(cold_type("not a coroutine"));
            };

        // Bound depth: sync frames within a coroutine, plus nested resumes from mutual awaits (native-stack recursion).
        if sync_frames.len() >= self.max_calls || self.depth >= self.max_calls {
            return Err(cold_depth());
        }
        // Charge the whole cloned state (stack/slots/iters/frames), not just frame count.
        self.charge_steps(sync_frames.len() + outer_stack.len() + outer_slots.len() + outer_iters.len())?;

        let saved_stack_len = self.stack.len();
        let saved_iter_len = self.iter_stack.len();
        let saved_exc_len = self.exception_stack.len();
        self.stack.extend_from_slice(&outer_stack);
        self.iter_stack.extend(outer_iters);
        // Denormalize: stored depths are relative to the coro's saved stack/iter; lift them to absolute positions in the live VM stacks.
        let mut restored_exc = outer_exc;
        for f in &mut restored_exc {
            f.stack_depth += saved_stack_len;
            f.iter_depth += saved_iter_len;
        }
        self.exception_stack.extend(restored_exc);
        let saved_yielded = self.yielded;
        let saved_resume_ip = self.resume_ip; // don't leak into next exec()
        self.yielded = false;
        self.depth += 1;

        // Walk frames inside-out, then the outer. `outer_ran` tracks whether `outer_ip` should be overwritten by `resume_ip` on save, a re-yield inside a sync frame leaves the outer pristine.
        let mut outer_ran = false;
        let mut pending_ret: Option<Val> = None;
        let result: Result<Val, VmErr> = 'drive: loop {
            if let Some(frame) = sync_frames.pop() {
                let SyncFrame { ip, fi, mut slots, stack_delta, iter_delta, exception_delta } = frame;
                let frame_stack_base = self.stack.len();
                let frame_iter_base = self.iter_stack.len();
                let frame_exc_base = self.exception_stack.len();
                self.stack.extend(stack_delta);
                // Inner result lands on this frame's stack.
                if let Some(v) = pending_ret.take() { self.push(v); }
                self.iter_stack.extend(iter_delta);
                let mut restored = exception_delta;
                for f in &mut restored {
                    f.stack_depth += frame_stack_base;
                    f.iter_depth += frame_iter_base;
                }
                self.exception_stack.extend(restored);
                self.pending_exec_exc_base = Some(frame_exc_base);
                self.pending_exec_safe = resume_safe;
                let (_, body, _, _) = self.functions[fi];
                match self.exec_from(body, &mut slots, ip) {
                    Err(e) => break 'drive Err(e),
                    Ok(val) if self.yielded => {
                        let new_stack = if self.stack.len() > frame_stack_base { self.stack.split_off(frame_stack_base) } else { Vec::new() };
                        let new_iter: Vec<IterFrame> = if self.iter_stack.len() > frame_iter_base { self.iter_stack.drain(frame_iter_base..).collect() } else { Vec::new() };
                        let new_exc: Vec<ExceptionFrame> = if self.exception_stack.len() > frame_exc_base {
                            self.exception_stack.drain(frame_exc_base..)
                                .map(|mut f| {
                                    f.stack_depth = f.stack_depth.saturating_sub(frame_stack_base);
                                    f.iter_depth = f.iter_depth.saturating_sub(frame_iter_base);
                                    f
                                })
                                .collect()
                        } else { Vec::new() };
                        sync_frames.push(SyncFrame {
                            ip: self.resume_ip, fi, slots,
                            stack_delta: new_stack, iter_delta: new_iter, exception_delta: new_exc,
                        });
                        // Reverse so pop re-enters innermost first.
                        let newer = core::mem::take(&mut self.pending_sync_frames);
                        sync_frames.extend(newer.into_iter().rev());
                        break 'drive Ok(val);
                    }
                    Ok(val) => { pending_ret = Some(val); }
                }
            } else {
                let body: &SSAChunk = match outer_body {
                    BodyRef::Fn(fi) => &self.functions[fi].1,
                    BodyRef::Module => self.chunk,
                };
                outer_ran = true;
                self.pending_exec_exc_base = Some(saved_exc_len);
                self.pending_exec_safe = resume_safe;
                if let Some(v) = pending_ret.take() { self.push(v); }
                match self.exec_from(body, &mut outer_slots, outer_ip) {
                    Err(e) => break 'drive Err(e),
                    Ok(val) => {
                        if self.yielded {
                            let newer = core::mem::take(&mut self.pending_sync_frames);
                            sync_frames.extend(newer.into_iter().rev());
                        }
                        break 'drive Ok(val);
                    }
                }
            }
        };

        self.depth -= 1;
        let result = result?;

        if self.yielded {
            let resume_ip = if outer_ran { self.resume_ip } else { outer_ip };
            // A coroutine that left the stack shorter must not panic split_off / drain; clamp both.
            let remaining = self.stack.split_off(saved_stack_len.min(self.stack.len()));
            let coro_iters: Vec<IterFrame> = self.iter_stack.drain(saved_iter_len.min(self.iter_stack.len())..).collect();
            // Normalize: depths captured at SetupExcept time were absolute against the live stacks; store them relative to the coro's saved stack so the next resume can denormalize against a different base.
            let coro_exc: Vec<ExceptionFrame> = self.exception_stack.drain(saved_exc_len..)
                .map(|mut f| {
                    f.stack_depth = f.stack_depth.saturating_sub(saved_stack_len);
                    f.iter_depth = f.iter_depth.saturating_sub(saved_iter_len);
                    f
                })
                .collect();
            // An inline-awaited coro isn't a scheduler root, so its body's GC may have freed it; if so skip the save (a freed coro is unreachable and won't resume).
            if let Some(HeapObj::Coroutine(sip, ss, sst, _, si, sf, ef)) = self.heap.try_get_mut(callee) {
                *sip = resume_ip;
                *ss = outer_slots;
                *sst = remaining;
                *si = coro_iters;
                *sf = sync_frames;
                *ef = coro_exc;
            }
            self.resume_ip = saved_resume_ip; // restore the caller's scratch
            Ok(result)
        } else {
            self.stack.truncate(saved_stack_len);
            self.iter_stack.truncate(saved_iter_len);
            self.exception_stack.truncate(saved_exc_len);
            self.yielded = saved_yielded;
            self.resume_ip = saved_resume_ip; // restore the caller's scratch
            Ok(result)
        }
    }

    /* Live (non-terminal) coroutine count; the concurrency that bounds scheduler work. */
    fn scheduler_active(&self) -> usize {
        self.scheduler.iter().filter(|h| !matches!(h.state, CoroState::Done(_) | CoroState::Errored(_) | CoroState::Cancelled)).count()
    }

    /* `await coro` and calling a coroutine `c()`: park the current coro on `target` (single-driver, like `run(target)`) and yield. The driving top loop resolves `target` and the wake-loop overwrites the placeholder with its value (or raises its error). Non-coroutine awaitables pass through unchanged at the call site. */
    pub(crate) fn await_coroutine(&mut self, target: Val) -> Result<(), VmErr> {
        if self.scheduler_active() >= self.max_calls {
            return Err(cold_depth());
        }
        if !self.scheduler.iter().any(|h| h.coro == target) {
            self.scheduler.push(CoroutineHandle { coro: target, state: CoroState::Ready });
        }
        self.push(Val::none()); // placeholder; wake-loop overwrites with target's result
        self.pending.waiting_for_children = Some((alloc::vec![target], WaitKind::Run(target)));
        self.yielded = true;
        Ok(())
    }

    /* `run(*coros)`, single-driver model: pushes the targets into the global scheduler, parks the outer in `WaitingForChildren` with `WaitKind::Run(target)`, and yields. The top loop drains the children and wakes the outer when all are terminal. */
    pub fn call_run(&mut self, argc: u16) -> Result<(), VmErr> {
        // Cap live concurrency like call depth: unbounded task spawning is recursion-shaped.
        if self.scheduler_active() >= self.max_calls {
            return Err(cold_depth());
        }
        let raw_tasks = self.pop_n(argc as usize)?;
        if raw_tasks.is_empty() {
            self.push(Val::none());
            return Ok(());
        }
        let target = raw_tasks[0];
        if self.time_hook.is_none() { self.virtual_clock_ns = 0; }
        let coros: Vec<Val> = raw_tasks.into_iter()
            .filter(|v| v.is_heap() && matches!(self.heap.get(*v), HeapObj::Coroutine(..)))
            .collect();
        for v in &coros {
            if !self.scheduler.iter().any(|h| h.coro == *v) {
                self.scheduler.push(CoroutineHandle { coro: *v, state: CoroState::Ready });
            }
        }
        // Placeholder on stack top; wake-loop overwrites it with the target's result.
        self.push(Val::none());
        if coros.is_empty() {
            // `run(non_coro)`, nothing to wait for; placeholder stays None.
            return Ok(());
        }
        self.pending.waiting_for_children = Some((coros, WaitKind::Run(target)));
        self.yielded = true;
        Ok(())
    }

    // Sweep `WaitingForChildren` outers: enforce timeouts, then wake any whose tracked tasks are all terminal, finalizing per `WaitKind`. Gated by `waiting_for_children_count` so the common (no-nested-run) tick is one comparison.
    fn wake_waiting_outers(&mut self) {
        if self.waiting_for_children_count == 0 { return; }

        // Timeout enforcement: mark non-terminal tasks as CancelPending when their parent's deadline expired.
        let now = self.now_ns();
        let expired: Vec<Val> = self.scheduler.iter().filter_map(|h| {
            if let CoroState::WaitingForChildren { tasks, kind: WaitKind::Timeout { deadline_ns, .. } } = &h.state
                && now >= *deadline_ns {
                Some(tasks.clone())
            } else { None }
        }).flatten().collect();
        for t in expired {
            if let Some(h) = self.scheduler.iter_mut().find(|h| h.coro == t)
                && !matches!(h.state, CoroState::Done(_) | CoroState::Errored(_) | CoroState::Cancelled | CoroState::CancelPending) {
                h.state = CoroState::CancelPending;
            }
        }

        // Wake outers whose tasks are all terminal.
        loop {
            let candidate = self.scheduler.iter().find_map(|h| {
                let CoroState::WaitingForChildren { tasks, kind } = &h.state else { return None; };
                let all_terminal = tasks.iter().all(|t| {
                    self.scheduler.iter().find(|c| c.coro == *t)
                        .is_none_or(|c| matches!(c.state, CoroState::Done(_) | CoroState::Errored(_) | CoroState::Cancelled))
                });
                if !all_terminal { return None; }
                Some((h.coro, tasks.clone(), kind.clone()))
            });
            let Some((outer, tasks, kind)) = candidate else { return; };
            let new_state = self.compute_wake_outcome(outer, &tasks, &kind);
            self.scheduler.retain(|h| h.coro == outer || !tasks.contains(&h.coro));
            let idx = self.scheduler.iter().position(|h| h.coro == outer).unwrap();
            self.waiting_for_children_count -= 1;
            self.scheduler[idx].state = new_state;
        }
    }

    // Finalize outer's state based on WaitKind and the (now-terminal) tasks. For Run / Gather / Timeout the placeholder is replaced; on error, raise it into the outer (popping a try-frame and jumping to the handler) or transition Errored if no handler is active.
    fn compute_wake_outcome(&mut self, outer: Val, tasks: &[Val], kind: &WaitKind) -> CoroState {
        match kind {
            WaitKind::Run(target) => {
                let outcome = self.scheduler.iter().find(|h| h.coro == *target).map(|h| h.state.clone());
                match outcome {
                    Some(CoroState::Errored(e)) => self.raise_into_outer(outer, e),
                    Some(CoroState::Done(v)) => {
                        self.splice_outer_placeholder(outer, v);
                        CoroState::Ready
                    }
                    _ => {
                        self.splice_outer_placeholder(outer, Val::none());
                        CoroState::Ready
                    }
                }
            }
            WaitKind::Gather => {
                let mut first_err: Option<VmErr> = None;
                let mut results = Vec::with_capacity(tasks.len());
                for t in tasks {
                    match self.scheduler.iter().find(|h| h.coro == *t).map(|h| h.state.clone()) {
                        Some(CoroState::Errored(e)) => {
                            if first_err.is_none() { first_err = Some(e); }
                            results.push(Val::none());
                        }
                        Some(CoroState::Done(v)) => results.push(v),
                        _ => results.push(Val::none()),
                    }
                }
                if let Some(e) = first_err { return self.raise_into_outer(outer, e); }
                match self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(results)))) {
                    Ok(list) => { self.splice_outer_placeholder(outer, list); CoroState::Ready }
                    Err(e) => self.raise_into_outer(outer, e),
                }
            }
            WaitKind::Timeout { deadline_ns, target } => {
                let deadline_hit = self.now_ns() >= *deadline_ns;
                let outcome = self.scheduler.iter().find(|h| h.coro == *target).map(|h| h.state.clone());
                match outcome {
                    Some(CoroState::Errored(e)) => self.raise_into_outer(outer, e),
                    Some(CoroState::Done(v)) if !deadline_hit => {
                        self.splice_outer_placeholder(outer, v);
                        CoroState::Ready
                    }
                    _ => self.raise_into_outer(outer, VmErr::Raised("TimeoutError".into())),
                }
            }
        }
    }

    fn splice_outer_placeholder(&mut self, outer: Val, value: Val) {
        if let HeapObj::Coroutine(_, _, stack, _, _, _, _) = self.heap.get_mut(outer)
            && let Some(top) = stack.last_mut() { *top = value; }
    }

    // Pop a try-frame from the outer's saved exception_frames and stage a raise: truncate saved stack/iter to the frame, push the exception instance, set IP to the handler. If no frame is active, the outer transitions to Errored and propagation continues.
    pub(crate) fn raise_into_outer(&mut self, outer: Val, e: VmErr) -> CoroState {
        let frame_opt = if let HeapObj::Coroutine(_, _, _, _, _, _, ef) = self.heap.get_mut(outer) {
            ef.pop()
        } else { None };
        let Some(frame) = frame_opt else { return CoroState::Errored(e); };
        // Build (or reuse) the exception instance.
        let exc_val = if let Some(v) = self.pending.exc_val.take() {
            v
        } else {
            let class_name = e.class_name();
            let msg_val = match self.heap.alloc(HeapObj::Str(e.message())) {
                Ok(v) => v,
                Err(alloc_e) => return CoroState::Errored(alloc_e),
            };
            match self.heap.alloc(HeapObj::ExcInstance(class_name, alloc::vec![msg_val])) {
                Ok(v) => v,
                Err(alloc_e) => return CoroState::Errored(alloc_e),
            }
        };
        // Splice into the outer's saved state, depths are relative to the saved stack/iter, matching the normalization on yield.
        if let HeapObj::Coroutine(ip, _, stack, _, iters, _, _) = self.heap.get_mut(outer) {
            stack.truncate(frame.stack_depth);
            iters.truncate(frame.iter_depth);
            stack.push(exc_val);
            *ip = frame.handler_ip;
        }
        CoroState::Ready
    }

    /* Single scheduler driver: picks a Ready coro and steps it; on no Ready, classifies the wait-state and yields to the host (PendingTimer / PendingFrame / PendingHostCall / PendingEvent) or returns Ok when nothing alive remains. */
    pub(crate) fn top_loop(&mut self) -> Result<(), VmErr> {
        loop {
            // Charge the full scheduler scan so accumulating coroutines stay bounded.
            self.charge_steps(self.scheduler.len().max(1))?;
            self.wake_waiting_outers();
            let mut next_ready: Option<usize> = None;
            let mut min_wake: Option<u64> = None;
            let mut any_frame = false;
            let mut any_event = false;
            let mut any_host_call = false;
            let mut alive = false;
            for (i, h) in self.scheduler.iter().enumerate() {
                match &h.state {
                    CoroState::Ready | CoroState::CancelPending => { next_ready = Some(i); alive = true; break; }
                    CoroState::Sleeping(w) => {
                        alive = true;
                        if min_wake.is_none_or(|m| *w < m) { min_wake = Some(*w); }
                    }
                    CoroState::WaitingForChildren { kind: WaitKind::Timeout { deadline_ns, .. }, .. } => {
                        alive = true;
                        if min_wake.is_none_or(|m| *deadline_ns < m) { min_wake = Some(*deadline_ns); }
                    }
                    CoroState::WaitingFrame => { any_frame = true; alive = true; }
                    CoroState::WaitingEvent => { any_event = true; alive = true; }
                    CoroState::WaitingHostCall(_) => { any_host_call = true; alive = true; }
                    CoroState::WaitingForChildren { .. } => { alive = true; }
                    CoroState::Done(_) | CoroState::Errored(_) | CoroState::Cancelled => {}
                }
            }
            if !alive { return Ok(()); }
            if let Some(i) = next_ready {
                self.scheduler_step(i)?;
                // Coro stays Ready; re-entering resumes it.
                if core::mem::take(&mut self.pending.preempt_request) {
                    return Err(VmErr::HostYield(SchedulerStatus::Preempted));
                }
                continue;
            }
            // Yield priority: frame tick > sleep/timeout deadline > host call > event.
            if any_frame { return Err(VmErr::HostYield(SchedulerStatus::PendingFrame)); }
            match min_wake {
                Some(w) => {
                    let now = self.now_ns();
                    if w > now {
                        if self.time_hook.is_some() {
                            return Err(VmErr::HostYield(SchedulerStatus::PendingTimer(w)));
                        }
                        self.virtual_clock_ns = w;
                    }
                    let now = self.now_ns();
                    for h in self.scheduler.iter_mut() {
                        if let CoroState::Sleeping(w) = h.state && w <= now {
                            h.state = CoroState::Ready;
                        }
                    }
                }
                None => {
                    if any_host_call { return Err(VmErr::HostYield(SchedulerStatus::PendingHostCall)); }
                    if any_event { return Err(VmErr::HostYield(SchedulerStatus::PendingEvent)); }
                    return Ok(());
                }
            }
        }
    }

    fn scheduler_step(&mut self, idx: usize) -> Result<(), VmErr> {
        let coro = self.scheduler[idx].coro;
        // CancelPending -> inject a CancelledError raise instead of resuming.
        if matches!(self.scheduler[idx].state, CoroState::CancelPending) {
            self.scheduler[idx].state = CoroState::Cancelled;
            return Ok(());
        }
        // Snapshot before resume so a yield during sleep / frame / receive / run can read it.
        self.pending.sleep_until_ns = None;
        self.pending.host_frame_request = false;
        self.pending.event_wait_request = false;
        self.pending.host_call_request = false;
        self.pending.waiting_for_children = None;
        self.pending_exec_safe = true;
        let result = self.resume_coroutine(coro);
        let yielded = self.yielded;
        self.yielded = false;
        let new_state = match result {
            Err(e) => CoroState::Errored(e),
            Ok(v) if yielded => {
                // Suspension precedence: sleep > frame > receive > host-call > children > bare yield.
                if let Some(until) = self.pending.sleep_until_ns.take() {
                    CoroState::Sleeping(until)
                } else if core::mem::replace(&mut self.pending.host_frame_request, false) {
                    CoroState::WaitingFrame
                } else if core::mem::replace(&mut self.pending.event_wait_request, false) {
                    CoroState::WaitingEvent
                } else if core::mem::replace(&mut self.pending.host_call_request, false) {
                    CoroState::WaitingHostCall(self.pending.host_call_id)
                } else if let Some((tasks, kind)) = self.pending.waiting_for_children.take() {
                    self.waiting_for_children_count += 1;
                    CoroState::WaitingForChildren { tasks, kind }
                } else {
                    let _ = v;
                    CoroState::Ready
                }
            }
            Ok(v) => CoroState::Done(v),
        };
        self.scheduler[idx].state = new_state;
        Ok(())
    }

    /* Suspend until the host's next render frame; browsers hook `requestAnimationFrame`. */
    pub fn call_frame(&mut self) -> Result<(), VmErr> {
        self.pending.host_frame_request = true;
        self.push(Val::none());
        self.yielded = true;
        Ok(())
    }

    /* Suspend until `s` real seconds elapse. */
    pub fn call_sleep(&mut self) -> Result<(), VmErr> {
        let n = self.pop()?;
        let secs: f64 = if n.is_int() { n.as_int() as f64 }
            else if n.is_float() { n.as_float() }
            else if n.is_bool() { n.as_bool() as i64 as f64 }
            else { 0.0 };
        let secs = if secs < 0.0 { 0.0 } else { secs };
        let until = self.now_ns().saturating_add((secs * 1_000_000_000.0) as u64);
        self.pending.sleep_until_ns = Some(until);
        // Push None as the yield value; the scheduler ignores it.
        self.push(Val::none());
        self.yielded = true;
        Ok(())
    }

    /* `gather(*coros)`, single-driver: pushes all targets, parks the outer in `WaitingForChildren` with `WaitKind::Gather`, and yields. Wake-loop builds the result list (or raises the first child error). */
    pub fn call_gather(&mut self, argc: u16) -> Result<(), VmErr> {
        // Cap live concurrency like call depth: unbounded task spawning is recursion-shaped.
        if self.scheduler_active() >= self.max_calls {
            return Err(cold_depth());
        }
        let tasks = self.pop_n(argc as usize)?;
        let coros: Vec<Val> = tasks.into_iter()
            .filter(|v| v.is_heap() && matches!(self.heap.get(*v), HeapObj::Coroutine(..)))
            .collect();
        for v in &coros {
            if !self.scheduler.iter().any(|h| h.coro == *v) {
                self.scheduler.push(CoroutineHandle { coro: *v, state: CoroState::Ready });
            }
        }
        self.push(Val::none()); // placeholder; replaced with the result list by the wake-loop.
        if coros.is_empty() {
            let empty = self.heap.alloc(HeapObj::List(Rc::new(RefCell::new(Vec::new()))))?;
            *self.stack.last_mut().unwrap() = empty;
            return Ok(());
        }
        self.pending.waiting_for_children = Some((coros, WaitKind::Gather));
        self.yielded = true;
        Ok(())
    }

    /* `with_timeout(seconds, coro)`, single-driver: pushes the target, parks the outer with `WaitKind::Timeout { deadline_ns, target }`, and yields. The top loop enforces the deadline by marking the target CancelPending when it expires; the wake-loop returns the target's value or `TimeoutError`. */
    pub fn call_with_timeout(&mut self) -> Result<(), VmErr> {
        let coro = self.pop()?;
        let secs_v = self.pop()?;
        if !(coro.is_heap() && matches!(self.heap.get(coro), HeapObj::Coroutine(..))) {
            return Err(cold_type("with_timeout() requires a coroutine"));
        }
        let secs: f64 = if secs_v.is_int() { secs_v.as_int() as f64 }
            else if secs_v.is_float() { secs_v.as_float() }
            else { return Err(cold_type("with_timeout() seconds must be a number")); };
        let deadline_ns = self.now_ns().saturating_add((secs.max(0.0) * 1_000_000_000.0) as u64);
        if !self.scheduler.iter().any(|h| h.coro == coro) {
            self.scheduler.push(CoroutineHandle { coro, state: CoroState::Ready });
        }
        self.push(Val::none()); // placeholder.
        self.pending.waiting_for_children = Some((vec![coro], WaitKind::Timeout { deadline_ns, target: coro }));
        self.yielded = true;
        Ok(())
    }

    /* cancel(coro), flag the coroutine for cancellation. */
    pub fn call_cancel(&mut self) -> Result<(), VmErr> {
        let coro = self.pop()?;
        if let Some(h) = self.scheduler.iter_mut().find(|h| h.coro == coro) {
            h.state = CoroState::CancelPending;
        }
        self.push(Val::none()); Ok(())
    }

    /* Pop oldest queued message; if empty, park in `WaitingEvent` until `run_push_event`. */
    pub fn call_receive(&mut self) -> Result<(), VmErr> {
        if !self.event_queue.is_empty() {
            let val = self.event_queue.remove(0);
            self.push(val);
        } else {
            self.pending.event_wait_request = true;
            self.push(Val::none());
            self.yielded = true;
        }
        Ok(())
    }
}
