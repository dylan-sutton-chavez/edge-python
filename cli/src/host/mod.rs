pub mod browser;
pub mod config;
pub mod driver;
mod env;
pub mod js;
mod plugins;
mod resolver;
mod rt;
mod vm;

pub use resolver::{built_in, Project};
pub use vm::{Completion, Instance, Status, Vm};

use anyhow::{anyhow, Result};
use std::cell::RefCell;
use std::collections::HashMap;
use std::path::PathBuf;
use std::rc::Rc;
use std::sync::mpsc::Sender;
use std::sync::{Arc, Mutex, OnceLock};
use wasmtime::{AsContextMut, Engine, Instance as Wasm, InstancePre, Linker, Memory, Module, ResourceLimiter, Store, TypedFunc};
use wasmtime_wasi_http::p2::bindings::sync::ProxyPre;

const COMPILER: &[u8] = include_bytes!(concat!(env!("OUT_DIR"), "/compiler.cwasm"));
// Each std package is deserialized the first time a program imports it.
const STD: [(&str, &[u8]); 4] = [
    ("json", include_bytes!(concat!(env!("OUT_DIR"), "/json.cwasm"))),
    ("re", include_bytes!(concat!(env!("OUT_DIR"), "/re.cwasm"))),
    ("math", include_bytes!(concat!(env!("OUT_DIR"), "/math.cwasm"))),
    ("struct", include_bytes!(concat!(env!("OUT_DIR"), "/struct.cwasm"))),
];

// The official origin every package url starts with.
pub const ORIGIN: &str = "https://cdn.edgepython.com";

// The epoch ticker's period, an untrusted deadline counts these.
pub const TICK_NS: u64 = 100_000_000;

// A build pulls dozens of files, one reset among them should not end it.
const ATTEMPTS: usize = 3;

pub const SITE: &str = "https://edgepython.com";

/* The registry endpoint for `path`, which tests and staging move with EDGE_SITE_BASE. */
pub fn site(path: &str) -> String {
    let base = std::env::var("EDGE_SITE_BASE").unwrap_or_else(|_| SITE.to_string());
    format!("{}{path}", base.trim_end_matches('/'))
}

/* Tests and staging serve the official origin from EDGE_CDN_BASE, production never sets it. */
pub fn cdn(url: &str) -> String {
    match (url.strip_prefix(ORIGIN), std::env::var("EDGE_CDN_BASE")) {
        (Some(path), Ok(base)) => format!("{}{path}", base.trim_end_matches('/')),
        _ => url.to_string(),
    }
}

/* Retries a hiccup with a growing pause, anything that describes the resource comes straight back. */
pub fn get(source: &str) -> Result<ureq::http::Response<ureq::Body>, ureq::Error> {
    let mut pause = std::time::Duration::from_millis(200);

    for _ in 1..ATTEMPTS {
        match ureq::get(source).call() {
            Err(e) if again(&e) => std::thread::sleep(pause),
            done => return done,
        }
        pause *= 3;
    }

    ureq::get(source).call()
}

/* A reset or an overloaded server may answer next time, a 404 already answered. */
fn again(error: &ureq::Error) -> bool {
    match error {
        ureq::Error::StatusCode(code) => *code == 429 || *code >= 500,
        ureq::Error::Io(_) | ureq::Error::Timeout(_) | ureq::Error::ConnectionFailed | ureq::Error::Protocol(_) => true,
        _ => false,
    }
}

/* The per-user edge cache, modules and the JavaScript runtime live under it. */
pub fn cache_root() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_CACHE_HOME") {
        return Ok(PathBuf::from(x).join("edge"));
    }
    let home = std::env::var("HOME").map_err(|_| "cannot locate a cache dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home).join(".cache").join("edge"))
}

/* What edge downloaded on a user's behalf, kept apart from the cache the uninstaller wipes unasked, since a browser is not refetchable in the way a module is. */
pub fn data_root() -> Result<PathBuf, String> {
    if let Ok(x) = std::env::var("XDG_DATA_HOME") {
        return Ok(PathBuf::from(x).join("edge"));
    }
    let home = std::env::var("HOME").map_err(|_| "cannot locate a data dir (no HOME)".to_string())?;
    Ok(PathBuf::from(home).join(".local").join("share").join("edge"))
}

// Wall-clock ns, the base every PendingTimer deadline is minted against.
pub fn now_ns() -> u64 {
    use std::time::{SystemTime, UNIX_EPOCH};
    SystemTime::now().duration_since(UNIX_EPOCH).map_or(0, |d| d.as_nanos() as u64)
}

pub fn wt<T>(r: wasmtime::Result<T>) -> Result<T> {
    r.map_err(|e| anyhow!("{e}"))
}

pub type Sink = Box<dyn FnMut(&str) + Send>;

// A sink shared with the JavaScript runtime threads, whose console output lands beside print().
pub type Printer = Arc<Mutex<Sink>>;

// Registered module specs, each to the native table slice it occupies.
pub type Registered = HashMap<String, (usize, Vec<String>)>;

// The universal dispatch export, op, receiver, name, argv, argc and the out slot.
pub type EdgeOp = TypedFunc<(i32, i32, i32, i32, i32, i32, i32), i32>;

/* The engine and the precompiled modules, shared by every scheduler thread. */
pub struct Runtime {
    pub engine: Engine,
    compiler: Module,
    std: [OnceLock<Module>; 4],
    // The JavaScript runtime, loaded on the first JavaScript import and retried after a failure.
    js: Mutex<Option<ProxyPre<js::JsState>>>,
}

impl Runtime {
    pub fn new() -> Result<Arc<Runtime>> {
        let engine = wt(Engine::new(&config::base()))?;
        let compiler = wt(unsafe { Module::deserialize(&engine, COMPILER) })?;
        Ok(Arc::new(Runtime { engine, compiler, std: Default::default(), js: Mutex::new(None) }))
    }

    /* StarlingMonkey ready to instantiate, fetched from the CDN the first time any process needs it. */
    pub fn js_pre(&self) -> Result<ProxyPre<js::JsState>, String> {
        let mut slot = self.js.lock().unwrap_or_else(|e| e.into_inner());
        if let Some(pre) = slot.as_ref() {
            return Ok(pre.clone());
        }
        let pre = js::load(&self.engine)?;
        *slot = Some(pre.clone());
        Ok(pre)
    }

    /* A std package's module, None for a name that is not built in. */
    fn std_module(&self, name: &str) -> Result<Option<&Module>> {
        let Some(i) = STD.iter().position(|(n, _)| *n == name) else { return Ok(None) };
        if let Some(module) = self.std[i].get() {
            return Ok(Some(module));
        }
        let module = wt(unsafe { Module::deserialize(&self.engine, STD[i].1) })?;
        Ok(Some(self.std[i].get_or_init(|| module)))
    }

    /* Advances the engine epoch every tick so untrusted deadlines fire. */
    pub fn start_ticker(self: &Arc<Self>) {
        let runtime = self.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(std::time::Duration::from_nanos(TICK_NS));
                runtime.engine.increment_epoch();
            }
        });
    }
}

/* Per-thread linkers over the shared runtime, instances are minted from here. */
pub struct Host {
    pub runtime: Arc<Runtime>,
    compiler: InstancePre<State>,
    guest: Linker<State>,
    std: RefCell<HashMap<String, InstancePre<State>>>,
    // Third party plugins by the sha256 of their bytes, compiled once per thread.
    plugins: RefCell<HashMap<[u8; 32], InstancePre<State>>>,
}

impl Host {
    pub fn new(runtime: Arc<Runtime>) -> Result<Rc<Host>> {
        let mut linker = Linker::new(&runtime.engine);
        env::link(&mut linker)?;
        let compiler = wt(linker.instantiate_pre(&runtime.compiler))?;
        let mut guest = Linker::new(&runtime.engine);
        plugins::link(&mut guest)?;
        Ok(Rc::new(Host { runtime, compiler, guest, std: RefCell::new(HashMap::new()), plugins: RefCell::new(HashMap::new()) }))
    }

    /* A third party plugin linked for instantiation, Cranelift compiles it on the first sight. */
    pub fn third_party(&self, bytes: &[u8]) -> Result<InstancePre<State>, String> {
        let key = compiler::util::sha256::sha256(bytes);
        if let Some(pre) = self.plugins.borrow().get(&key) {
            return Ok(pre.clone());
        }
        let module = crate::wasm_cache::load(&self.runtime.engine, bytes)?;
        let pre = self.plugin_pre(&module)?;
        self.plugins.borrow_mut().insert(key, pre.clone());
        Ok(pre)
    }

    /* A plugin module linked against the six guest imports. */
    pub fn plugin_pre(&self, module: &Module) -> Result<InstancePre<State>, String> {
        wt(self.guest.instantiate_pre(module)).map_err(|e| e.to_string())
    }

    fn std_pre(&self, name: &str) -> Result<Option<InstancePre<State>>, String> {
        if let Some(pre) = self.std.borrow().get(name) {
            return Ok(Some(pre.clone()));
        }
        let Some(module) = self.runtime.std_module(name).map_err(|e| e.to_string())? else { return Ok(None) };
        let pre = self.plugin_pre(module)?;
        self.std.borrow_mut().insert(name.to_string(), pre.clone());
        Ok(Some(pre))
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
    // An export of a JavaScript module, answered by that module's runtime.
    Js {
        runtime: usize,
        name: String,
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

/* Everything the host functions reach through the store, shared by the instance's slots. */
pub struct State {
    pub exports: Option<Exports>,
    pub print: Printer,
    pub natives: Vec<Native>,
    pub fetched: HashMap<String, Vec<u8>>,
    pub registered: Registered,
    // Ids of host calls parked since the last dispatch, each answer arrives as a completion.
    pub deferred: Vec<u32>,
    // Messages send() handed over, None outside an actor pool where no scheduler drains them.
    pub outbox: Option<Vec<(String, String)>>,
    pub limiter: MemoryCap,
    // The selected slot and the channel its completions and events reach it through.
    pub slot: u32,
    pub events: Option<Sender<Completion>>,
    // Wall-clock ns an untrusted run must finish by, host-side waits honor it too.
    pub deadline: Option<u64>,
    pub js: Vec<js::JsRuntime>,
}

/* The compiler exports the host drives, bound once per instance. */
#[derive(Clone)]
pub struct Exports {
    pub memory: Memory,
    pub out_ptr: TypedFunc<(), i32>,
    pub out_len: TypedFunc<(), i32>,
    pub wasm_alloc: TypedFunc<i32, i32>,
    pub wasm_free: TypedFunc<(i32, i32), ()>,
    pub register_code_module: TypedFunc<(i32, i32, i32, i32), ()>,
    pub register_native_module: TypedFunc<(i32, i32, i32, i32, i32), ()>,
    pub register_module_error: TypedFunc<(i32, i32, i32, i32), ()>,
    pub reset_modules: TypedFunc<(), ()>,
    pub set_entry_dir: TypedFunc<(i32, i32), ()>,
    pub set_input: TypedFunc<(i32, i32), ()>,
    pub repl_eval: TypedFunc<(i32, i32), i32>,
    pub run_start: TypedFunc<(i32, i32), i32>,
    pub run_resume: TypedFunc<(), i32>,
    pub run_push_event: TypedFunc<(i32, i32), i32>,
    pub set_host_result_by_id: TypedFunc<(i32, i32), i32>,
    pub set_host_error_by_id: TypedFunc<(i32, i32, i32), i32>,
    pub last_yield_deadline_ns: TypedFunc<(), i64>,
    pub set_preempt_interval: TypedFunc<i32, ()>,
    pub save_state: TypedFunc<(), i64>,
    pub restore_state: TypedFunc<(i32, i32), i32>,
    pub host_edge_op: EdgeOp,
    pub host_edge_encode: TypedFunc<(i32, i32, i32), i32>,
    pub host_edge_decode: TypedFunc<(i32, i32, i32, i32), i32>,
    pub host_edge_release: TypedFunc<i32, ()>,
    pub host_edge_throw: TypedFunc<(i32, i32, i32), ()>,
    pub host_edge_take_error: TypedFunc<(i32, i32, i32), i32>,
    pub set_limits: TypedFunc<(i64, i64, i64), ()>,
    pub set_source_name: TypedFunc<(i32, i32), ()>,
    pub vm_create: TypedFunc<(), i32>,
    pub vm_select: TypedFunc<i32, i32>,
    pub vm_drop: TypedFunc<i32, i32>,
}

impl Exports {
    fn bind(store: &mut Store<State>, instance: &Wasm) -> Result<Exports> {
        let memory = instance.get_memory(&mut *store, "memory").ok_or_else(|| anyhow!("compiler.wasm exports no memory"))?;
        macro_rules! f {
            ($name:literal) => {
                wt(instance.get_typed_func(&mut *store, $name))?
            };
        }
        Ok(Exports {
            memory,
            out_ptr: f!("out_ptr"),
            out_len: f!("out_len"),
            wasm_alloc: f!("wasm_alloc"),
            wasm_free: f!("wasm_free"),
            register_code_module: f!("register_code_module"),
            register_native_module: f!("register_native_module"),
            register_module_error: f!("register_module_error"),
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
            restore_state: f!("restore_state"),
            host_edge_op: f!("host_edge_op"),
            host_edge_encode: f!("host_edge_encode"),
            host_edge_decode: f!("host_edge_decode"),
            host_edge_release: f!("host_edge_release"),
            host_edge_throw: f!("host_edge_throw"),
            host_edge_take_error: f!("host_edge_take_error"),
            set_limits: f!("set_limits"),
            set_source_name: f!("set_source_name"),
            vm_create: f!("vm_create"),
            vm_select: f!("vm_select"),
            vm_drop: f!("vm_drop"),
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
