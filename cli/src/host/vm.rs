use super::{read, resolver, rt, stage, unstage, wt, Events, Exports, Host, MemoryCap, Project, Sink, State};
use crate::builtins;
use anyhow::{anyhow, bail, Result};
use compiler::abi::WireValue;
use compiler::vm::Limits;
use std::cell::{RefCell, RefMut};
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver};
use std::sync::Arc;
use std::time::Duration;
use wasmtime::{ResourceLimiter, Store};

const KIND_SHIFT: u32 = 29;

/* What a run step left the interpreter waiting on, the packed status of the compiler exports. */
pub enum Status {
    Done,
    PendingTimer(u64),
    PendingFrame,
    PendingEvent,
    PendingHostCall,
    Error(String),
    Exit(u8),
    Preempted,
}

/* A host call the compiler parked on, answered later by id. */
pub struct Deferred {
    pub id: u32,
    pub module: &'static str,
    pub name: String,
    pub args: Vec<WireValue>,
}

pub enum Completion {
    Value { id: u32, value: WireValue },
    Error { id: u32, msg: String },
    // A stream capability's event, the script reads it through receive().
    Event(String),
}

/* One compiler instance, its store and the slots its interpreters run in. */
pub struct Instance {
    pub(super) host: Rc<Host>,
    pub(super) store: Store<State>,
    pub(super) ex: Exports,
    pub project: Project,
    // The slot the per-run exports act on right now.
    selected: u32,
    // Entry dir and source whose modules are registered, slots booting them skip the walk.
    prepared: Option<(String, String)>,
    // A trap leaves the instance memory untrustworthy, nothing runs in it again.
    poisoned: bool,
    slots: usize,
}

impl Host {
    /* A fresh compiler instance, `deadline` in epoch ticks and `memory` in bytes bound an untrusted run. */
    pub fn instance(self: &Rc<Self>, sink: Sink, project: Project, deadline: Option<u64>, memory: Option<usize>) -> Result<Rc<RefCell<Instance>>> {
        let state = State {
            exports: None,
            print: sink,
            natives: Vec::new(),
            fetched: HashMap::new(),
            registered: HashMap::new(),
            deferred: Vec::new(),
            outbox: Vec::new(),
            limiter: MemoryCap { max: memory.unwrap_or(usize::MAX) },
            events: None,
        };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|s: &mut State| &mut s.limiter as &mut dyn ResourceLimiter);
        store.set_epoch_deadline(deadline.unwrap_or(1 << 40));
        store.epoch_deadline_trap();
        let instance = wt(self.compiler.instantiate(&mut store))?;
        let ex = Exports::bind(&mut store, &instance)?;
        store.data_mut().exports = Some(ex.clone());
        wt(ex.reset_modules.call(&mut store, ()))?;
        Ok(Rc::new(RefCell::new(Instance { host: self.clone(), store, ex, project, selected: 0, prepared: None, poisoned: false, slots: 0 })))
    }

    /* One interpreter on an instance of its own, the shape of every one-shot run. */
    pub fn vm(self: &Rc<Self>, sink: Sink, project: Project, deadline: Option<u64>, memory: Option<usize>) -> Result<Vm> {
        Ok(Vm::on(self.instance(sink, project, deadline, memory)?, 0))
    }
}

impl Instance {
    /* Opens another interpreter slot in `inst`, the slot frees when the Vm drops. */
    pub fn slot(inst: &Rc<RefCell<Instance>>) -> Result<Vm> {
        let id = {
            let mut i = inst.borrow_mut();
            let raw = {
                let Instance { store, ex, .. } = &mut *i;
                ex.vm_create.call(&mut *store, ())
            };
            let id = i.checked(raw)?;
            i.slots += 1;
            id as u32
        };
        Ok(Vm::on(inst.clone(), id))
    }

    pub fn slots(&self) -> usize {
        self.slots
    }

    pub fn poisoned(&self) -> bool {
        self.poisoned
    }

    /* Any failed call is a trap, and a trapped instance never runs again. */
    fn checked<R>(&mut self, r: wasmtime::Result<R>) -> Result<R> {
        r.map_err(|e| {
            self.poisoned = true;
            anyhow!("{e}")
        })
    }

    fn select(&mut self, slot: u32, events: &Events) -> Result<()> {
        if self.poisoned {
            bail!("the interpreter instance trapped");
        }
        if self.selected != slot {
            let raw = self.ex.vm_select.call(&mut self.store, slot as i32);
            if self.checked(raw)? != 0 {
                bail!("interpreter slot {slot} is gone");
            }
            self.selected = slot;
            self.store.data_mut().events = None;
        }
        if self.store.data().events.is_none() {
            self.store.data_mut().events = Some(events.clone());
        }
        Ok(())
    }

    /* Stages `bytes` for one `(ptr, len)` call and frees them once it returns. */
    fn with_bytes<R>(&mut self, bytes: &[u8], call: impl FnOnce(&mut Store<State>, &Exports, i32, i32) -> wasmtime::Result<R>) -> Result<wasmtime::Result<R>> {
        let staged = stage(&mut self.store, &self.ex, bytes);
        let ptr = self.checked(staged)?;
        let ex = self.ex.clone();
        let result = call(&mut self.store, &ex, ptr, bytes.len() as i32);
        unstage(&mut self.store, &self.ex, ptr, bytes.len());
        Ok(result)
    }

    /* The compiler's out buffer, what the last call left there. */
    fn read_out(&mut self) -> Vec<u8> {
        let ptr = self.ex.out_ptr.call(&mut self.store, ()).unwrap_or(0);
        let len = self.ex.out_len.call(&mut self.store, ()).unwrap_or(0);
        read(&mut self.store, self.ex.memory, ptr, len)
    }

    // Registers the modules a source reaches once per instance, then roots the entry dir and feeds stdin.
    fn prepare(&mut self, src: &str, input: Option<&str>) -> Result<Option<Status>> {
        let key = (self.project.entry_dir.clone(), src.to_string());
        if self.prepared.as_ref() != Some(&key) {
            if let Err(e) = resolver::prefetch(self, src) {
                return Ok(Some(Status::Error(e)));
            }
            self.prepared = Some(key);
        }
        let dir = self.project.entry_dir.clone();
        let raw = self.with_bytes(dir.as_bytes(), |store, ex, ptr, len| ex.set_entry_dir.call(store, (ptr, len)))?;
        self.checked(raw)?;
        if let Some(text) = input {
            let raw = self.with_bytes(text.as_bytes(), |store, ex, ptr, len| ex.set_input.call(store, (ptr, len)))?;
            self.checked(raw)?;
        }
        Ok(None)
    }

    fn status(&mut self, raw: wasmtime::Result<i32>) -> Result<Status> {
        let status = match raw {
            Ok(raw) => raw as u32,
            Err(e) => {
                self.poisoned = true;
                return Ok(Status::Error(trap_text(&e)));
            }
        };
        Ok(match status >> KIND_SHIFT {
            0 => Status::Done,
            1 => {
                let raw = self.ex.last_yield_deadline_ns.call(&mut self.store, ());
                Status::PendingTimer(self.checked(raw)? as u64)
            }
            2 => Status::PendingFrame,
            3 => Status::PendingEvent,
            4 => Status::Error(String::from_utf8_lossy(&self.read_out()).into_owned()),
            5 => Status::PendingHostCall,
            6 => Status::Exit((status & 0xFF) as u8),
            7 => Status::Preempted,
            kind => bail!("unknown run status kind {kind}"),
        })
    }

    pub(super) fn register_code(&mut self, spec: &str, src: &[u8]) -> Result<(), String> {
        self.register_pair(spec, src, |store, ex, (s, sl), (p, pl)| ex.register_code_module.call(store, (s, sl, p, pl)))
    }

    pub(super) fn register_error(&mut self, spec: &str, msg: &str) -> Result<(), String> {
        self.register_pair(spec, msg.as_bytes(), |store, ex, (s, sl), (p, pl)| ex.register_module_error.call(store, (s, sl, p, pl)))
    }

    pub(super) fn register_native(&mut self, spec: &str, names: &[String], base: usize) -> Result<(), String> {
        let joined = names.join("\n");
        self.register_pair(spec, joined.as_bytes(), |store, ex, (s, sl), (p, pl)| ex.register_native_module.call(store, (s, sl, p, pl, base as i32)))
    }

    // Stages a spec and a payload for one registration call, both freed once it returns.
    fn register_pair(&mut self, spec: &str, payload: &[u8], call: impl FnOnce(&mut Store<State>, &Exports, (i32, i32), (i32, i32)) -> wasmtime::Result<()>) -> Result<(), String> {
        let s = stage(&mut self.store, &self.ex, spec.as_bytes()).map_err(|e| e.to_string())?;
        let p = stage(&mut self.store, &self.ex, payload).map_err(|e| e.to_string())?;
        let ex = self.ex.clone();
        let r = call(&mut self.store, &ex, (s, spec.len() as i32), (p, payload.len() as i32));
        unstage(&mut self.store, &self.ex, s, spec.len());
        unstage(&mut self.store, &self.ex, p, payload.len());
        r.map_err(|e| e.to_string())
    }
}

/* One interpreter, a slot of an instance plus the completions its deferred calls report to. */
pub struct Vm {
    inst: Rc<RefCell<Instance>>,
    slot: u32,
    events: Events,
    // One strong ref here plus one per open stream.
    streams: Arc<()>,
    rx: Receiver<Completion>,
    // Calls this interpreter parked on, moved out of the shared store after every step.
    parked: Vec<Deferred>,
    inflight: usize,
    buffered: VecDeque<String>,
}

impl Drop for Vm {
    fn drop(&mut self) {
        if self.slot == 0 {
            return;
        }
        if let Ok(mut inst) = self.inst.try_borrow_mut() {
            inst.slots -= 1;
            if !inst.poisoned {
                let Instance { store, ex, selected, .. } = &mut *inst;
                let _ = ex.vm_drop.call(&mut *store, self.slot as i32);
                // Dropping the selected slot moves the compiler back to slot 0.
                if *selected == self.slot {
                    *selected = 0;
                    store.data_mut().events = None;
                }
            }
        }
    }
}

impl Vm {
    fn on(inst: Rc<RefCell<Instance>>, slot: u32) -> Vm {
        let (tx, rx) = channel();
        let streams = Arc::new(());
        let events = Events { tx, streams: Arc::downgrade(&streams) };
        Vm { inst, slot, events, streams, rx, parked: Vec::new(), inflight: 0, buffered: VecDeque::new() }
    }

    /* Keeps what the step just parked on, before another slot of the instance runs. */
    fn stepped(&mut self, status: Result<Status>) -> Result<Status> {
        let calls = std::mem::take(&mut self.inst.borrow_mut().store.data_mut().deferred);
        self.parked.extend(calls);
        status
    }

    /* The instance with this interpreter's slot selected. */
    fn enter(&self) -> Result<RefMut<'_, Instance>> {
        let mut inst = self.inst.borrow_mut();
        inst.select(self.slot, &self.events)?;
        Ok(inst)
    }

    pub fn instance(&self) -> &Rc<RefCell<Instance>> {
        &self.inst
    }

    /* Resolves imports and starts a fresh run, a resolution failure reads like a compile error. */
    pub fn start(&mut self, src: &str, input: Option<&str>) -> Result<Status> {
        let status = {
            let mut inst = self.enter()?;
            if let Some(status) = inst.prepare(src, input)? {
                return Ok(status);
            }
            let raw = inst.with_bytes(src.as_bytes(), |store, ex, ptr, len| ex.run_start.call(store, (ptr, len)))?;
            inst.status(raw)
        };
        self.stepped(status)
    }

    /* Runs one more input on the persistent interpreter, history never re-executes. */
    pub fn repl_eval(&mut self, src: &str, input: Option<&str>) -> Result<Status> {
        let status = {
            let mut inst = self.enter()?;
            if let Some(status) = inst.prepare(src, input)? {
                return Ok(status);
            }
            let raw = inst.with_bytes(src.as_bytes(), |store, ex, ptr, len| ex.repl_eval.call(store, (ptr, len)))?;
            inst.status(raw)
        };
        self.stepped(status)
    }

    pub fn resume(&mut self) -> Result<Status> {
        let status = {
            let mut inst = self.enter()?;
            let raw = {
                let Instance { store, ex, .. } = &mut *inst;
                ex.run_resume.call(&mut *store, ())
            };
            inst.status(raw)
        };
        self.stepped(status)
    }

    /* Roots the next input's relative imports at `dir`. */
    pub fn set_base(&mut self, dir: &str) {
        self.inst.borrow_mut().project.entry_dir = dir.to_string();
    }

    /* Wakes a parked receive(), or queues the message until the run parks on one. */
    pub fn push_event(&mut self, message: &str) -> bool {
        if self.inject(message) {
            return true;
        }
        self.buffered.push_back(message.to_string());
        false
    }

    fn inject(&mut self, message: &str) -> bool {
        let Ok(mut inst) = self.enter() else { return false };
        let Ok(raw) = inst.with_bytes(message.as_bytes(), |store, ex, ptr, len| ex.run_push_event.call(store, (ptr, len))) else { return false };
        matches!(inst.checked(raw), Ok(0))
    }

    /* Hands queued messages to a run parked on an event, the count delivered. */
    pub fn drain_buffered(&mut self) -> usize {
        let mut delivered = 0;
        while let Some(message) = self.buffered.front().cloned() {
            if !self.inject(&message) {
                break;
            }
            self.buffered.pop_front();
            delivered += 1;
        }
        delivered
    }

    pub fn save_state(&mut self) -> Result<Option<Vec<u8>>> {
        let mut inst = self.enter()?;
        let raw = {
            let Instance { store, ex, .. } = &mut *inst;
            ex.save_state.call(&mut *store, ())
        };
        if inst.checked(raw)? < 0 {
            return Ok(None);
        }
        Ok(Some(inst.read_out()))
    }

    /* Boots from the blob's embedded source, its imports must resolve again. */
    pub fn restore_state(&mut self, blob: &[u8]) -> Result<Status> {
        let source = snapshot_source(blob)?;
        let status = {
            let mut inst = self.enter()?;
            if let Some(status) = inst.prepare(&source, None)? {
                return Ok(status);
            }
            let raw = inst.with_bytes(blob, |store, ex, ptr, len| ex.restore_state.call(store, (ptr, len)))?;
            inst.status(raw)
        };
        self.stepped(status)
    }

    pub fn set_preempt_interval(&mut self, n: usize) -> Result<()> {
        let mut inst = self.enter()?;
        let raw = {
            let Instance { store, ex, .. } = &mut *inst;
            ex.set_preempt_interval.call(&mut *store, n as i32)
        };
        inst.checked(raw)
    }

    /* Caps the next boot, a group's limits or the sandbox profile. */
    pub fn set_limits(&mut self, limits: &Limits) -> Result<()> {
        let mut inst = self.enter()?;
        let raw = {
            let Instance { store, ex, .. } = &mut *inst;
            ex.set_limits.call(&mut *store, (limits.heap as i64, limits.ops as i64, limits.calls as i64))
        };
        inst.checked(raw)
    }

    /* Names the entry frame in tracebacks, the compiler renders `<input>` otherwise. */
    pub fn set_source_name(&mut self, name: &str) -> Result<()> {
        let mut inst = self.enter()?;
        let raw = inst.with_bytes(name.as_bytes(), |store, ex, ptr, len| ex.set_source_name.call(store, (ptr, len)))?;
        inst.checked(raw)
    }

    /* Drops every module registration, the next input starts in a fresh namespace. */
    pub fn reset(&mut self) -> Result<()> {
        let mut inst = self.enter()?;
        let raw = {
            let Instance { store, ex, .. } = &mut *inst;
            ex.reset_modules.call(&mut *store, ())
        };
        inst.checked(raw)?;
        inst.prepared = None;
        let state = inst.store.data_mut();
        state.natives.clear();
        state.registered.clear();
        state.deferred.clear();
        drop(inst);
        self.parked.clear();
        Ok(())
    }

    /* Hands the calls the last step parked on to their worker threads. */
    pub fn dispatch(&mut self) {
        for call in std::mem::take(&mut self.parked) {
            builtins::spawn(call, self.events.tx.clone());
            self.inflight += 1;
        }
    }

    pub fn inflight(&self) -> usize {
        self.inflight
    }

    /* Streams still open, each may push another event into receive(). */
    pub fn streams(&self) -> usize {
        Arc::strong_count(&self.streams) - 1
    }

    /* Blocks for the next completion, then injects every one that arrived, the count delivered. */
    pub fn wait(&mut self, timeout: Option<Duration>) -> Result<usize> {
        let first = match timeout {
            Some(t) => self.rx.recv_timeout(t).ok(),
            None => self.rx.recv().ok(),
        };
        let Some(completion) = first else { return Ok(0) };
        self.deliver(completion)?;
        Ok(1 + self.poll()?)
    }

    /* Injects the completions already in, never blocks. */
    pub fn poll(&mut self) -> Result<usize> {
        let mut delivered = 0;
        while let Ok(completion) = self.rx.try_recv() {
            self.deliver(completion)?;
            delivered += 1;
        }
        Ok(delivered)
    }

    fn deliver(&mut self, completion: Completion) -> Result<()> {
        let (id, value) = match completion {
            Completion::Event(message) => {
                self.push_event(&message);
                return Ok(());
            }
            Completion::Value { id, value } => (id, Ok(value)),
            Completion::Error { id, msg } => (id, Err(msg)),
        };
        self.inflight = self.inflight.saturating_sub(1);
        let mut inst = self.enter()?;
        let raw = {
            let Instance { store, ex, .. } = &mut *inst;
            match value {
                Ok(value) => {
                    let handle = rt::encode(&mut *store, ex, &value).map_err(|e| anyhow!(e))?;
                    ex.set_host_result_by_id.call(&mut *store, (id as i32, handle as i32))
                }
                Err(msg) => {
                    let handle = rt::encode(&mut *store, ex, &WireValue::Bytes(msg.into_bytes())).map_err(|e| anyhow!(e))?;
                    ex.set_host_error_by_id.call(&mut *store, (id as i32, super::env::ERR_RUNTIME, handle as i32))
                }
            }
        };
        let code = inst.checked(raw)?;
        if code != 0 {
            bail!("host call {id} delivery returned {code}");
        }
        Ok(())
    }

    /* Everything actor.send queued during the last step. */
    pub fn take_sends(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.inst.borrow_mut().store.data_mut().outbox)
    }
}

/* The source a snapshot blob embeds, the header layout of vm/snapshot.rs. */
pub fn snapshot_source(blob: &[u8]) -> Result<String> {
    let bad = || anyhow!("not an edge-python snapshot");
    if blob.len() < 24 || u32::from_le_bytes(blob[0..4].try_into().map_err(|_| bad())?) != 0x4E53_5045 {
        return Err(bad());
    }
    let len = u64::from_le_bytes(blob[16..24].try_into().map_err(|_| bad())?) as usize;
    let end = 24usize.checked_add(len).filter(|e| *e <= blob.len()).ok_or_else(bad)?;
    Ok(String::from_utf8_lossy(&blob[24..end]).into_owned())
}

/* A trap is a run error, an epoch deadline reads as the time limit. */
fn trap_text(e: &wasmtime::Error) -> String {
    if e.downcast_ref::<wasmtime::Trap>().is_some_and(|t| *t == wasmtime::Trap::Interrupt) {
        return "error: RuntimeError: run exceeded its time limit".to_string();
    }
    format!("error: {e}")
}
