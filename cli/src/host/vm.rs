use super::{read, resolver, rt, stage, unstage, write, wt, Exports, Host, MemoryCap, Project, Sink, State};
use crate::builtins;
use anyhow::{anyhow, bail, Result};
use compiler::abi::WireValue;
use compiler::vm::Limits;
use std::collections::{HashMap, VecDeque};
use std::rc::Rc;
use std::sync::mpsc::{channel, Receiver, Sender};
use std::time::Duration;
use wasmtime::{ResourceLimiter, Store};

// Byte cap on any source handed to the compiler, mirrors its SRC buffer.
pub const SOURCE_LIMIT: usize = 1 << 20;
const KIND_SHIFT: u32 = 29;
const PAYLOAD_MASK: u32 = (1 << KIND_SHIFT) - 1;

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
}

/* One interpreter, its store and the completions its deferred calls report to. */
pub struct Vm {
    pub(super) host: Rc<Host>,
    pub(super) store: Store<State>,
    pub(super) ex: Exports,
    pub project: Project,
    tx: Sender<Completion>,
    rx: Receiver<Completion>,
    inflight: usize,
    buffered: VecDeque<String>,
}

impl Host {
    /* Boots one interpreter, `deadline` in epoch ticks and `memory` in bytes bound an untrusted run. */
    pub fn vm(self: &Rc<Self>, sink: Sink, project: Project, deadline: Option<u64>, memory: Option<usize>) -> Result<Vm> {
        let (tx, rx) = channel();
        let state = State {
            exports: None,
            print: sink,
            natives: Vec::new(),
            fetched: HashMap::new(),
            registered: HashMap::new(),
            deferred: Vec::new(),
            outbox: Vec::new(),
            limiter: MemoryCap { max: memory.unwrap_or(usize::MAX) },
        };
        let mut store = Store::new(&self.runtime.engine, state);
        store.limiter(|s: &mut State| &mut s.limiter as &mut dyn ResourceLimiter);
        store.set_epoch_deadline(deadline.unwrap_or(1 << 40));
        store.epoch_deadline_trap();
        let instance = wt(self.compiler.instantiate(&mut store))?;
        let ex = Exports::bind(&mut store, &instance)?;
        store.data_mut().exports = Some(ex.clone());
        wt(ex.reset_modules.call(&mut store, ()))?;
        Ok(Vm { host: self.clone(), store, ex, project, tx, rx, inflight: 0, buffered: VecDeque::new() })
    }
}

impl Vm {
    /* Resolves imports and starts a fresh run, a resolution failure reads like a compile error. */
    pub fn start(&mut self, src: &str, input: Option<&str>) -> Result<Status> {
        if let Some(status) = self.prepare(src, input)? {
            return Ok(status);
        }
        let len = self.write_src(src.as_bytes())?;
        let raw = self.ex.run_start.call(&mut self.store, len);
        self.status(raw)
    }

    /* Runs one more input on the persistent interpreter, history never re-executes. */
    pub fn repl_eval(&mut self, src: &str, input: Option<&str>) -> Result<Status> {
        if let Some(status) = self.prepare(src, input)? {
            return Ok(status);
        }
        let len = self.write_src(src.as_bytes())?;
        let raw = self.ex.repl_eval.call(&mut self.store, len);
        self.status(raw)
    }

    pub fn resume(&mut self) -> Result<Status> {
        let raw = self.ex.run_resume.call(&mut self.store, ());
        self.status(raw)
    }

    // Registers every module the source reaches, roots the entry dir, feeds stdin.
    fn prepare(&mut self, src: &str, input: Option<&str>) -> Result<Option<Status>> {
        if let Err(e) = resolver::prefetch(self, src) {
            return Ok(Some(Status::Error(e)));
        }
        let dir = self.project.entry_dir.clone();
        let len = self.write_src(dir.as_bytes())?;
        wt(self.ex.set_entry_dir.call(&mut self.store, len))?;
        if let Some(text) = input {
            let ptr = wt(stage(&mut self.store, &self.ex, text.as_bytes()))?;
            wt(self.ex.set_input.call(&mut self.store, (ptr, text.len() as i32)))?;
            unstage(&mut self.store, &self.ex, ptr, text.len());
        }
        Ok(None)
    }

    fn status(&mut self, raw: wasmtime::Result<i32>) -> Result<Status> {
        let status = match raw {
            Ok(raw) => raw as u32,
            Err(e) => return Ok(Status::Error(trap_text(&e))),
        };
        Ok(match status >> KIND_SHIFT {
            0 => Status::Done,
            1 => Status::PendingTimer(wt(self.ex.last_yield_deadline_ns.call(&mut self.store, ()))? as u64),
            2 => Status::PendingFrame,
            3 => Status::PendingEvent,
            4 => Status::Error(self.read_out((status & PAYLOAD_MASK) as i32)),
            5 => Status::PendingHostCall,
            6 => Status::Exit((status & 0xFF) as u8),
            7 => Status::Preempted,
            kind => bail!("unknown run status kind {kind}"),
        })
    }

    fn read_out(&mut self, len: i32) -> String {
        let ptr = self.ex.out_ptr.call(&mut self.store, ()).unwrap_or(0);
        String::from_utf8_lossy(&read(&mut self.store, self.ex.memory, ptr, len)).into_owned()
    }

    fn write_src(&mut self, bytes: &[u8]) -> Result<i32> {
        if bytes.len() > SOURCE_LIMIT {
            bail!("source exceeds {SOURCE_LIMIT} bytes");
        }
        let ptr = wt(self.ex.src_ptr.call(&mut self.store, ()))?;
        write(&mut self.store, self.ex.memory, ptr, bytes);
        Ok(bytes.len() as i32)
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
        let Ok(ptr) = stage(&mut self.store, &self.ex, message.as_bytes()) else { return false };
        let status = self.ex.run_push_event.call(&mut self.store, (ptr, message.len() as i32)).unwrap_or(1);
        unstage(&mut self.store, &self.ex, ptr, message.len());
        status == 0
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
        let len = wt(self.ex.save_state.call(&mut self.store, ()))?;
        if len < 0 {
            return Ok(None);
        }
        let ptr = wt(self.ex.snapshot_ptr.call(&mut self.store, ()))?;
        Ok(Some(read(&mut self.store, self.ex.memory, ptr, len as i32)))
    }

    /* Boots from the blob's embedded source, its imports must resolve again. */
    pub fn restore_state(&mut self, blob: &[u8]) -> Result<Status> {
        let source = snapshot_source(blob)?;
        if let Err(e) = resolver::prefetch(self, &source) {
            return Ok(Status::Error(e));
        }
        if blob.len() > SOURCE_LIMIT {
            bail!("snapshot exceeds {SOURCE_LIMIT} bytes");
        }
        let len = self.write_src(blob)?;
        let raw = self.ex.restore_state.call(&mut self.store, len);
        self.status(raw)
    }

    pub fn set_preempt_interval(&mut self, n: usize) -> Result<()> {
        wt(self.ex.set_preempt_interval.call(&mut self.store, n as i32))
    }

    /* Caps the next boot, a group's limits or the sandbox profile. */
    pub fn set_limits(&mut self, limits: &Limits) -> Result<()> {
        wt(self.ex.set_limits.call(&mut self.store, (limits.heap as i64, limits.ops as i64, limits.calls as i64)))
    }

    /* Names the entry frame in tracebacks, the compiler renders `<input>` otherwise. */
    pub fn set_source_name(&mut self, name: &str) -> Result<()> {
        let len = self.write_src(name.as_bytes())?;
        wt(self.ex.set_source_name.call(&mut self.store, len))
    }

    /* Drops every module registration, the next input starts in a fresh namespace. */
    pub fn reset(&mut self) -> Result<()> {
        wt(self.ex.reset_modules.call(&mut self.store, ()))?;
        let state = self.store.data_mut();
        state.natives.clear();
        state.registered.clear();
        state.deferred.clear();
        Ok(())
    }

    pub(super) fn register_code(&mut self, spec: &str, src: &[u8]) -> Result<(), String> {
        let s = stage(&mut self.store, &self.ex, spec.as_bytes()).map_err(|e| e.to_string())?;
        let b = stage(&mut self.store, &self.ex, src).map_err(|e| e.to_string())?;
        let r = self.ex.register_code_module.call(&mut self.store, (s, spec.len() as i32, b, src.len() as i32));
        unstage(&mut self.store, &self.ex, s, spec.len());
        unstage(&mut self.store, &self.ex, b, src.len());
        r.map_err(|e| e.to_string())
    }

    pub(super) fn register_native(&mut self, spec: &str, names: &[String], base: usize) -> Result<(), String> {
        let joined = names.join("\n");
        let s = stage(&mut self.store, &self.ex, spec.as_bytes()).map_err(|e| e.to_string())?;
        let n = stage(&mut self.store, &self.ex, joined.as_bytes()).map_err(|e| e.to_string())?;
        let r = self.ex.register_native_module.call(&mut self.store, (s, spec.len() as i32, n, joined.len() as i32, base as i32));
        unstage(&mut self.store, &self.ex, s, spec.len());
        unstage(&mut self.store, &self.ex, n, joined.len());
        r.map_err(|e| e.to_string())
    }

    /* Hands the calls the last step parked on to their worker threads. */
    pub fn dispatch(&mut self) {
        for call in std::mem::take(&mut self.store.data_mut().deferred) {
            builtins::spawn(call, self.tx.clone());
            self.inflight += 1;
        }
    }

    pub fn inflight(&self) -> usize {
        self.inflight
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
        self.inflight = self.inflight.saturating_sub(1);
        let (id, code) = match completion {
            Completion::Value { id, value } => {
                let handle = rt::encode(&mut self.store, &self.ex, &value).map_err(|e| anyhow!(e))?;
                (id, wt(self.ex.set_host_result_by_id.call(&mut self.store, (id as i32, handle as i32)))?)
            }
            Completion::Error { id, msg } => {
                let handle = rt::encode(&mut self.store, &self.ex, &WireValue::Bytes(msg.into_bytes())).map_err(|e| anyhow!(e))?;
                (id, wt(self.ex.set_host_error_by_id.call(&mut self.store, (id as i32, super::env::ERR_RUNTIME, handle as i32)))?)
            }
        };
        if code != 0 {
            bail!("host call {id} delivery returned {code}");
        }
        Ok(())
    }

    /* Everything actor.send queued during the last step. */
    pub fn take_sends(&mut self) -> Vec<(String, String)> {
        std::mem::take(&mut self.store.data_mut().outbox)
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
