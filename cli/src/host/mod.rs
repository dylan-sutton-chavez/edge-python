pub mod config;
pub mod driver;
mod env;
mod plugins;
mod resolver;
mod rt;
mod vm;

pub use resolver::Project;
pub use vm::{Completion, Deferred, Status, Vm};

use anyhow::{anyhow, Result};
use std::collections::HashMap;
use std::rc::Rc;
use std::sync::Arc;
use wasmtime::{AsContextMut, Engine, Instance, InstanceAllocationStrategy, InstancePre, Linker, Memory, Module, PoolingAllocationConfig, ResourceLimiter, Store, TypedFunc};

const COMPILER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compiler.cwasm"));
const JSON: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/json.cwasm"));
const RE: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/re.cwasm"));
const MATH: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/math.cwasm"));
const STRUCT: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/struct.cwasm"));

// Wall-clock ns, the base every PendingTimer deadline is minted against.
pub fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

pub fn wt<T>(r: wasmtime::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

pub type Sink = Box<dyn FnMut(&str) + Send>;

// Registered module specs, each to the native table slice it occupies.
pub type Registered = HashMap<String, (usize, Vec<String>)>;

// The universal dispatch export, op, receiver, name, argv, argc and the out slot.
pub type EdgeOp = TypedFunc<(i32, i32, i32, i32, i32, i32, i32), i32>;

/* The engine and the precompiled modules, shared by every scheduler thread. */
pub struct Runtime {
    pub engine: Engine,
    compiler: Module,
    std: Vec<(&'static str, Module)>,
}

impl Runtime {
    /* `pool` sizes a pooling allocator for an actor fleet, None allocates each instance on demand. */
    pub fn new(pool: Option<u32>) -> Result<Arc<Runtime>> {
        let mut cfg = config::base();
        if let Some(n) = pool {
            let mut p = PoolingAllocationConfig::default();
            // A compiler instance and its four std plugins share one slot budget.
            let slots = n.saturating_mul(5);
            p.total_core_instances(slots).total_memories(slots).total_tables(slots);
            p.max_memory_size(config::MEMORY_RESERVATION as usize);
            cfg.allocation_strategy(InstanceAllocationStrategy::Pooling(p));
        }
        let engine = wt(Engine::new(&cfg))?;
        let load = |bytes: &[u8]| wt(unsafe { Module::deserialize(&engine, bytes) });
        let compiler = load(COMPILER)?;
        let std = vec![("json", load(JSON)?), ("re", load(RE)?), ("math", load(MATH)?), ("struct", load(STRUCT)?)];
        Ok(Arc::new(Runtime { engine, compiler, std }))
    }

    /* Advances the engine epoch every 100 ms so untrusted deadlines fire. */
    pub fn start_ticker(self: &Arc<Self>) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_millis(100));
                runtime.engine.increment_epoch();
            }
        });
    }
}

/* Per-thread linkers over the shared runtime, instances are minted from here. */
pub struct Host {
    pub runtime: Arc<Runtime>,
    compiler: InstancePre<State>,
    std: Vec<(&'static str, InstancePre<State>)>,
}

impl Host {
    pub fn new(runtime: Arc<Runtime>) -> Result<Rc<Host>> {
        let mut linker = Linker::new(&runtime.engine);
        env::link(&mut linker)?;
        let compiler = wt(linker.instantiate_pre(&runtime.compiler))?;
        let mut guest = Linker::new(&runtime.engine);
        plugins::link(&mut guest)?;
        let mut std = Vec::new();
        for (name, module) in &runtime.std {
            std.push((*name, wt(guest.instantiate_pre(module))?));
        }
        Ok(Rc::new(Host { runtime, compiler, std }))
    }

    fn std_pre(&self, name: &str) -> Option<&InstancePre<State>> {
        self.std.iter().find(|(n, _)| *n == name).map(|(_, p)| p)
    }
}

/* One entry of the native table the compiler dispatches `host_call_native` through. */
#[derive(Clone)]
pub enum Native {
    Plugin {
        func: TypedFunc<(i32, i32, i32), i32>,
        alloc: TypedFunc<i32, i32>,
        free: Option<TypedFunc<(i32, i32), ()>>,
        memory: Memory,
    },
    Capability {
        module: &'static str,
        name: String,
        deferred: bool,
    },
}

// Refuses linear memory growth past the cap, an untrusted run cannot outgrow its slot.
pub struct MemoryCap {
    pub max: usize,
}

impl ResourceLimiter for MemoryCap {
    fn memory_growing(&mut self, _current: usize, desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(desired <= self.max)
    }

    fn table_growing(&mut self, _current: usize, _desired: usize, _maximum: Option<usize>) -> wasmtime::Result<bool> {
        Ok(true)
    }
}

/* Everything the host functions reach through the store, one per interpreter. */
pub struct State {
    pub exports: Option<Exports>,
    pub print: Sink,
    pub natives: Vec<Native>,
    pub fetched: HashMap<String, Vec<u8>>,
    pub registered: Registered,
    pub deferred: Vec<Deferred>,
    pub outbox: Vec<(String, String)>,
    pub limiter: MemoryCap,
}

/* The compiler exports the host drives, bound once per instance. */
#[derive(Clone)]
pub struct Exports {
    pub memory: Memory,
    pub src_ptr: TypedFunc<(), i32>,
    pub out_ptr: TypedFunc<(), i32>,
    pub wasm_alloc: TypedFunc<i32, i32>,
    pub wasm_free: TypedFunc<(i32, i32), ()>,
    pub register_code_module: TypedFunc<(i32, i32, i32, i32), ()>,
    pub register_native_module: TypedFunc<(i32, i32, i32, i32, i32), ()>,
    pub reset_modules: TypedFunc<(), ()>,
    pub set_entry_dir: TypedFunc<i32, ()>,
    pub set_input: TypedFunc<(i32, i32), ()>,
    pub repl_eval: TypedFunc<i32, i32>,
    pub run_start: TypedFunc<i32, i32>,
    pub run_resume: TypedFunc<(), i32>,
    pub run_push_event: TypedFunc<(i32, i32), i32>,
    pub set_host_result_by_id: TypedFunc<(i32, i32), i32>,
    pub set_host_error_by_id: TypedFunc<(i32, i32, i32), i32>,
    pub last_yield_deadline_ns: TypedFunc<(), i64>,
    pub set_preempt_interval: TypedFunc<i32, ()>,
    pub save_state: TypedFunc<(), i64>,
    pub snapshot_ptr: TypedFunc<(), i32>,
    pub restore_state: TypedFunc<i32, i32>,
    pub host_edge_op: EdgeOp,
    pub host_edge_encode: TypedFunc<(i32, i32, i32), i32>,
    pub host_edge_decode: TypedFunc<(i32, i32, i32, i32), i32>,
    pub host_edge_release: TypedFunc<i32, ()>,
    pub host_edge_throw: TypedFunc<(i32, i32, i32), ()>,
    pub host_edge_take_error: TypedFunc<(i32, i32, i32), i32>,
    pub set_limits: TypedFunc<(i64, i64, i64), ()>,
    pub set_source_name: TypedFunc<i32, ()>,
}

impl Exports {
    fn bind(store: &mut Store<State>, instance: &Instance) -> Result<Exports> {
        let memory = instance.get_memory(&mut *store, "memory").ok_or_else(|| anyhow!("compiler.wasm exports no memory"))?;
        macro_rules! f {
            ($name:literal) => {
                wt(instance.get_typed_func(&mut *store, $name))?
            };
        }
        Ok(Exports {
            memory,
            src_ptr: f!("src_ptr"),
            out_ptr: f!("out_ptr"),
            wasm_alloc: f!("wasm_alloc"),
            wasm_free: f!("wasm_free"),
            register_code_module: f!("register_code_module"),
            register_native_module: f!("register_native_module"),
            reset_modules: f!("reset_modules"),
            set_entry_dir: f!("set_entry_dir"),
            set_input: f!("set_input"),
            repl_eval: f!("repl_eval"),
            run_start: f!("run_start"),
            run_resume: f!("run_resume"),
            run_push_event: f!("run_push_event"),
            set_host_result_by_id: f!("set_host_result_by_id"),
            set_host_error_by_id: f!("set_host_error_by_id"),
            last_yield_deadline_ns: f!("last_yield_deadline_ns"),
            set_preempt_interval: f!("set_preempt_interval"),
            save_state: f!("save_state"),
            snapshot_ptr: f!("snapshot_ptr"),
            restore_state: f!("restore_state"),
            host_edge_op: f!("host_edge_op"),
            host_edge_encode: f!("host_edge_encode"),
            host_edge_decode: f!("host_edge_decode"),
            host_edge_release: f!("host_edge_release"),
            host_edge_throw: f!("host_edge_throw"),
            host_edge_take_error: f!("host_edge_take_error"),
            set_limits: f!("set_limits"),
            set_source_name: f!("set_source_name"),
        })
    }
}

pub(crate) fn read(cx: &mut impl AsContextMut<Data = State>, mem: Memory, ptr: i32, len: i32) -> Vec<u8> {
    let mut buf = vec![0u8; len.max(0) as usize];
    if mem.read(&mut *cx, ptr as u32 as usize, &mut buf).is_err() {
        buf.clear();
    }
    buf
}

pub(crate) fn write(cx: &mut impl AsContextMut<Data = State>, mem: Memory, ptr: i32, bytes: &[u8]) {
    let _ = mem.write(&mut *cx, ptr as u32 as usize, bytes);
}

pub(crate) fn read_u32(cx: &mut impl AsContextMut<Data = State>, mem: Memory, ptr: i32) -> u32 {
    let bytes = read(cx, mem, ptr, 4);
    bytes.try_into().map(u32::from_le_bytes).unwrap_or(0)
}

pub(crate) fn write_u32(cx: &mut impl AsContextMut<Data = State>, mem: Memory, ptr: i32, value: u32) {
    write(cx, mem, ptr, &value.to_le_bytes());
}

/* Copies bytes into a fresh compiler allocation, freed with `unstage` and the same length. */
pub(crate) fn stage(cx: &mut impl AsContextMut<Data = State>, ex: &Exports, bytes: &[u8]) -> wasmtime::Result<i32> {
    let ptr = ex.wasm_alloc.call(&mut *cx, bytes.len().max(1) as i32)?;
    write(cx, ex.memory, ptr, bytes);
    Ok(ptr)
}

pub(crate) fn unstage(cx: &mut impl AsContextMut<Data = State>, ex: &Exports, ptr: i32, len: usize) {
    let _ = ex.wasm_free.call(&mut *cx, (ptr, len.max(1) as i32));
}
