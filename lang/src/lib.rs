#![no_std]

extern crate alloc;

use alloc::{boxed::Box, format, string::{String, ToString}, sync::Arc, vec::Vec};

use compiler::lexer::lex;
use compiler::packages::{partition_bindings, NativeBinding, Resolved, Resolver};
use compiler::parser::{Diagnostic, Parser, SSAChunk};
use compiler::value::{HeapObj, HeapPool, Val, VmErr};
use compiler::vm::VM;

pub use compiler::vm::Limits;

/* A source rewrite applied before lexing, output must be valid Edge Python. */
pub type Transform = dyn Fn(&str) -> String + Send + Sync + 'static;

/* A value crossing the Rust and script boundary. Scalars round-trip losslessly, composites surface as `Object` carrying their rendered form. */
#[derive(Clone, Debug, PartialEq)]
pub enum Value {
    None,
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    Object(String),
}

impl Value {
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            Value::Bool(b) => Some(*b as i64),
            _ => None,
        }
    }

    pub fn as_float(&self) -> Option<f64> {
        match self {
            Value::Float(f) => Some(*f),
            Value::Int(i) => Some(*i as f64),
            Value::Bool(b) => Some(*b as i64 as f64),
            _ => None,
        }
    }

    pub fn as_str(&self) -> Option<&str> {
        if let Value::Str(s) = self { Some(s) } else { None }
    }

    pub fn as_bool(&self) -> Option<bool> {
        if let Value::Bool(b) = self { Some(*b) } else { None }
    }

    /* Reads a `Val` into an owned `Value`, `render` covers composites the heap alone can't stringify. */
    fn read(v: Val, heap: &HeapPool, render: impl FnOnce(Val) -> String) -> Value {
        if v.is_none() { Value::None }
        else if v.is_bool() { Value::Bool(v.as_bool()) }
        else if v.is_int() { Value::Int(v.as_int()) }
        else if v.is_float() { Value::Float(v.as_float()) }
        else if v.is_heap() {
            match heap.get(v) {
                HeapObj::Str(s) => Value::Str(s.clone()),
                HeapObj::LongInt(i) => Value::Object(i.to_string()),
                _ => Value::Object(render(v)),
            }
        } else { Value::None }
    }

    /* Allocates this value on the engine heap for return from a native. */
    fn write(&self, heap: &mut HeapPool) -> Result<Val, VmErr> {
        Ok(match self {
            Value::None => Val::none(),
            Value::Bool(b) => Val::bool(*b),
            Value::Int(i) => match Val::int_checked(*i) {
                Some(v) => v,
                None => heap.alloc(HeapObj::LongInt(*i as i128))?,
            },
            Value::Float(f) => Val::float(*f),
            Value::Str(s) | Value::Object(s) => heap.alloc(HeapObj::Str(s.clone()))?,
        })
    }
}

impl core::fmt::Display for Value {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Value::None => f.write_str("None"),
            Value::Bool(b) => f.write_str(if *b { "True" } else { "False" }),
            Value::Int(i) => write!(f, "{i}"),
            Value::Float(x) => write!(f, "{x}"),
            Value::Str(s) | Value::Object(s) => f.write_str(s),
        }
    }
}

/* From<T> for Value and its TryFrom inverse, one line per scalar type. */
macro_rules! convert {
    ( $( $ty:ty => $var:ident, $acc:ident );* $(;)? ) => {
        $(
            impl From<$ty> for Value {
                fn from(v: $ty) -> Self { Value::$var(v) }
            }
            impl TryFrom<Value> for $ty {
                type Error = Value;
                fn try_from(v: Value) -> Result<Self, Value> {
                    v.$acc().ok_or(v)
                }
            }
        )*
    };
}

convert! {
    bool => Bool, as_bool;
    i64 => Int, as_int;
    f64 => Float, as_float;
}

impl From<&str> for Value {
    fn from(s: &str) -> Self { Value::Str(s.to_string()) }
}
impl From<String> for Value {
    fn from(s: String) -> Self { Value::Str(s) }
}
impl TryFrom<Value> for String {
    type Error = Value;
    fn try_from(v: Value) -> Result<Self, Value> {
        if let Value::Str(s) = v { Ok(s) } else { Err(v) }
    }
}

/* A compile or run failure, two levels. `Compile` carries a rustc-style diagnostic, `Run` a traceback. */
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Error {
    Compile(String),
    Run(String),
}

impl Error {
    pub fn message(&self) -> &str {
        match self { Error::Compile(m) | Error::Run(m) => m }
    }
}

impl core::fmt::Display for Error {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        match self {
            Error::Compile(m) => write!(f, "compile error: {m}"),
            Error::Run(m) => write!(f, "run error: {m}"),
        }
    }
}

impl core::error::Error for Error {}

/* A run outcome, the value the module evaluated to plus its captured output. */
#[derive(Clone, Debug)]
pub struct Output {
    value: Value,
    text: String,
}

impl Output {
    pub fn value(&self) -> &Value { &self.value }
    pub fn text(&self) -> &str { &self.text }
    pub fn into_value(self) -> Value { self.value }
}

/* A configured engine, clone-cheap and reusable across compiles. */
#[derive(Clone)]
pub struct Engine {
    limits: Limits,
    natives: Arc<Vec<NativeBinding>>,
    transforms: Arc<Vec<Box<Transform>>>,
}

impl core::fmt::Debug for Engine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Engine")
            .field("natives", &self.natives.len())
            .field("transforms", &self.transforms.len())
            .finish_non_exhaustive()
    }
}

impl Engine {
    pub fn builder() -> Builder {
        Builder { limits: Limits::sandbox(), natives: Vec::new(), transforms: Vec::new() }
    }

    /* Compiles source into a runnable `Program`. Err renders a diagnostic with a source caret. */
    pub fn compile(&self, source: &str) -> Result<Program, Error> {
        // Custom-syntax rewrites run first, each fed the previous output.
        let mut src = source.to_string();
        for t in self.transforms.iter() {
            src = t(&src);
        }
        let (tokens, lex_errs) = lex(&src);
        let resolver = HostResolver { natives: self.natives.clone() };
        let mut parser = Parser::with_resolver(&src, tokens.into_iter(), Box::new(resolver));
        for e in lex_errs {
            parser.errors.push(Diagnostic { start: e.start, end: e.end, msg: e.msg.into() });
        }
        let (mut chunk, errs) = parser.parse();
        if !errs.is_empty() {
            let mut buf = String::new();
            for (i, e) in errs.iter().enumerate() {
                if i > 0 { buf.push('\n'); }
                buf.push_str(&e.render(&src, None));
            }
            return Err(Error::Compile(buf));
        }
        compiler::optimizer::constant_fold(&mut chunk);
        Ok(Program { chunk, source: src, limits: self.limits })
    }
}

/* Configures an `Engine` before construction. */
pub struct Builder {
    limits: Limits,
    natives: Vec<NativeBinding>,
    transforms: Vec<Box<Transform>>,
}

impl Builder {
    pub fn limits(mut self, limits: Limits) -> Self {
        self.limits = limits;
        self
    }

    /* Rewrites source before lexing, the hook for custom syntax and DSLs. */
    pub fn transform<F>(mut self, f: F) -> Self
    where F: Fn(&str) -> String + Send + Sync + 'static
    {
        self.transforms.push(Box::new(f));
        self
    }

    /* Renames one keyword or name, whole-word so it never touches a longer identifier. */
    pub fn keyword(self, from: impl Into<String>, to: impl Into<String>) -> Self {
        let (from, to) = (from.into(), to.into());
        self.transform(move |src| replace_word(src, &from, &to))
    }

    /* Exposes a Rust fn to scripts under `from host import <name>`, impure so results never memoize. */
    pub fn define<F>(mut self, name: impl Into<String>, f: F) -> Self
    where F: Fn(&[Value]) -> Result<Value, String> + Send + Sync + 'static
    {
        let f = Arc::new(f);
        self.natives.push(NativeBinding {
            name: name.into(),
            func: Arc::new(move |heap, argv, _kwargs| {
                let args: Vec<Value> = argv.iter()
                    .map(|&v| Value::read(v, heap, |_| String::new()))
                    .collect();
                f(&args).map_err(VmErr::Raised).and_then(|out| out.write(heap))
            }),
            pure: false,
        });
        self
    }

    pub fn build(self) -> Engine {
        Engine {
            limits: self.limits,
            natives: Arc::new(self.natives),
            transforms: Arc::new(self.transforms),
        }
    }
}

/* A compiled unit ready to run, produced only by `Engine::compile`. */
pub struct Program {
    chunk: SSAChunk,
    source: String,
    limits: Limits,
}

impl Program {
    /* Builds a VM, binds natives, and runs the module body so top-level defs are bound. */
    fn boot(&self) -> Result<VM<'static>, Error> {
        // Storage-only 'static, the VM never outlives the caller's use of it.
        let chunk_static: &'static SSAChunk = unsafe { core::mem::transmute::<&SSAChunk, &'static SSAChunk>(&self.chunk) };
        let mut vm = VM::with_limits(chunk_static, self.limits);
        vm.bind_chunk_externs().map_err(|e| Error::Run(e.render()))?;
        Ok(vm)
    }

    /* Renders a VM error as a traceback tied to this program's source. */
    fn trace(&self, vm: &VM<'static>, e: VmErr) -> Error {
        Error::Run(e.render_traceback(
            &self.source, vm.error_pos(), None,
            vm.call_stack_frames(), vm.function_names_ref(),
        ))
    }

    /* Runs to completion under the engine limits. Err renders a traceback. */
    pub fn run(&self) -> Result<Output, Error> {
        let mut vm = self.boot()?;
        match vm.run() {
            Ok(v) => {
                let value = Value::read(v, vm.heap(), |v| vm.display(v));
                Ok(Output { value, text: vm.output_text() })
            }
            Err(e) => Err(self.trace(&vm, e)),
        }
    }

    /* Calls a top-level function by name with the given arguments and returns its result. */
    pub fn call(&self, name: &str, args: &[Value]) -> Result<Output, Error> {
        let mut vm = self.boot()?;
        // Run the module body first so its `def`s are bound before the call.
        vm.run().map_err(|e| self.trace(&vm, e))?;
        let argv: Vec<Val> = args.iter()
            .map(|a| a.write(vm.heap_mut()))
            .collect::<Result<_, _>>()
            .map_err(|e| self.trace(&vm, e))?;
        match vm.call_export(name, &argv) {
            Ok(v) => {
                let value = Value::read(v, vm.heap(), |v| vm.display(v));
                Ok(Output { value, text: vm.output_text() })
            }
            Err(e) => Err(self.trace(&vm, e)),
        }
    }

    pub fn source(&self) -> &str { &self.source }
}

impl core::fmt::Debug for Program {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Program").field("source", &self.source).finish_non_exhaustive()
    }
}

/* In-memory resolver exposing the engine natives under the `host` module. */
struct HostResolver {
    natives: Arc<Vec<NativeBinding>>,
}

impl Resolver for HostResolver {
    fn resolve(&mut self, spec: &str) -> Result<Resolved, String> {
        if spec != "host" {
            return Err(format!("module '{spec}' not found (only 'host' is available)"));
        }
        let (bindings, classes, consts) = partition_bindings((*self.natives).clone());
        Ok(Resolved::Native { bindings, classes, consts, canonical: "host".to_string() })
    }
}

/* Replaces `from` with `to` only where it stands as a whole word. */
fn replace_word(src: &str, from: &str, to: &str) -> String {
    if from.is_empty() { return src.to_string(); }
    let is_word = |c: char| c.is_alphanumeric() || c == '_';
    let mut out = String::with_capacity(src.len());
    let mut i = 0;
    while i < src.len() {
        if src[i..].starts_with(from) {
            let before_ok = i == 0 || !src[..i].chars().next_back().is_some_and(is_word);
            let after = i + from.len();
            let after_ok = after >= src.len() || !src[after..].chars().next().is_some_and(is_word);
            if before_ok && after_ok {
                out.push_str(to);
                i = after;
                continue;
            }
        }
        let ch = src[i..].chars().next().unwrap();
        out.push(ch);
        i += ch.len_utf8();
    }
    out
}
