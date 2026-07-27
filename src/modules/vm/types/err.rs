use alloc::string::String;

use super::{CallFrame, SchedulerStatus};

/* Runtime errors; static variants alloc-free, dynamic carry user text. `HostYield` is a control-flow signal (not catchable by Python try/except), riding the `Result` chain so `?` propagation reuses without parallel signaling. */
#[derive(Debug, Clone)]
pub enum VmErr {
    CallDepth, Heap, Budget, ZeroDiv, Overflow,
    Name(String),
    Type(&'static str),
    TypeMsg(String),
    Value(&'static str),
    Runtime(&'static str),
    Attribute(String),
    Raised(String),
    HostYield(SchedulerStatus),
    /// Native `CallExtern` deferred to host; caught locally by `call_extern`, never propagates.
    HostCallDeferred,
}

impl VmErr {
    /* Class-name lookup used by the exception unwind path and the `with` cleanup opcode; native errors, `Raised` keeps the user-supplied name. */
    pub fn class_name(&self) -> alloc::string::String {
        match self {
            Self::ZeroDiv => "ZeroDivisionError".into(),
            Self::Overflow => "OverflowError".into(),
            Self::Type(_) | Self::TypeMsg(_) => "TypeError".into(),
            Self::Value(_) => "ValueError".into(),
            Self::Attribute(_) => "AttributeError".into(),
            Self::Name(_) => "NameError".into(),
            Self::CallDepth => "RecursionError".into(),
            Self::Heap => "MemoryError".into(),
            Self::Budget | Self::Runtime(_) => "RuntimeError".into(),
            // `Raised` carries "Class" or "Class: message"; the bare class name drives except-matching.
            Self::Raised(s) => s.split(':').next().unwrap_or(s).trim().into(),
            // Unreachable in correct hosts; embedder catches HostYield before traceback rendering.
            Self::HostYield(_) => "_HostYield".into(),
            // Caught locally in call_extern; unreachable here.
            Self::HostCallDeferred => "_HostCallDeferred".into(),
        }
    }

    pub fn as_str(&self) -> &'static str {
        match self {
            Self::CallDepth => "RecursionError: max depth",
            Self::Heap => "MemoryError: heap limit",
            Self::Budget => "RuntimeError: budget exceeded",
            Self::ZeroDiv => "ZeroDivisionError: division by zero",
            Self::Overflow => "OverflowError: integer too large for 128-bit int range",
            Self::Type(s) => s,
            Self::Value(s) => s,
            Self::Runtime(s) => s,
            Self::TypeMsg(_) => "TypeError",
            Self::Attribute(_) => "AttributeError",
            Self::Name(_) => "NameError",
            Self::Raised(_) => "Exception",
            Self::HostYield(_) => "host yield requested",
            Self::HostCallDeferred => "native call deferred to host",
        }
    }

    pub fn render(&self) -> alloc::string::String {
        use crate::s;
        match self {
            Self::Name(n) => s!("NameError: name '", str n, "' is not defined"),
            // Already formatted as "Class" or "Class: message"; render verbatim (no double prefix).
            Self::Raised(m) => m.clone(),
            Self::Type(m) => s!("TypeError: ", str m),
            Self::TypeMsg(m) => s!("TypeError: ", str m),
            Self::Value(m) => s!("ValueError: ", str m),
            Self::Runtime(m) => s!("RuntimeError: ", str m),
            Self::Attribute(m) => s!("AttributeError: ", str m),
            Self::HostYield(_) => alloc::string::String::from("RuntimeError: scheduler suspended; embedder must drive `run_start` / `run_resume`"),
            Self::HostCallDeferred => alloc::string::String::from("RuntimeError: HostCallDeferred leaked past `call_extern` (compiler bug)"),
            other => alloc::string::String::from(other.as_str()),
        }
    }

    /* Message-only form (no "Class:" prefix); feeds `e.args`. */
    pub fn message(&self) -> alloc::string::String {
        use alloc::string::String;
        match self {
            Self::Name(n) => crate::s!("name '", str n, "' is not defined"),
            Self::Type(m) | Self::Value(m) | Self::Runtime(m) => String::from(*m),
            Self::TypeMsg(m) | Self::Attribute(m) => m.clone(),
            // `Raised` carries "Class" or "Class: message"; the message is the part after the class (empty for a bare class), so re-wrapping into an ExcInstance doesn't double the class prefix.
            Self::Raised(m) => m.split_once(": ").map_or(String::new(), |(_, msg)| String::from(msg)),
            Self::ZeroDiv => String::from("division by zero"),
            Self::Overflow => String::from("integer too large for 128-bit int range"),
            Self::CallDepth => String::from("max depth"),
            Self::Heap => String::from("heap limit"),
            Self::Budget => String::from("budget exceeded"),
            Self::HostYield(_) => String::from("scheduler suspended; embedder must drive run_start / run_resume"),
            Self::HostCallDeferred => String::from("native call deferred to host"),
        }
    }

    /* `render()` anchored at a byte offset for rustc-style caret preview; falls back when absent. */
    pub fn render_at(&self, src: &str, byte_pos: Option<usize>, path: Option<&str>) -> alloc::string::String {
        let Some(pos) = byte_pos else { return self.render(); };
        crate::modules::parser::Diagnostic { start: pos, end: pos, msg: self.render() }.render(src, path)
    }

    /* Multi-frame traceback: `error:` at the site, then `note: called from ...` outward. */
    pub fn render_traceback(&self, error_src: &str, error_byte_pos: Option<usize>, error_path: Option<&str>, frames: &[CallFrame], function_names: &[alloc::string::String]) -> alloc::string::String {
        let mut out = self.render_at(error_src, error_byte_pos, error_path);
        for f in frames.iter().rev() {
            let fname = function_names.get(f.fi).map(|s| s.as_str()).unwrap_or("<anonymous>");
            let pos = f.call_byte_pos as usize;
            let path: Option<&str> = if f.caller_path.is_empty() { None } else { Some(f.caller_path.as_str()) };
            let note = crate::modules::parser::Diagnostic {
                start: pos, end: pos,
                msg: alloc::format!("called from {}()", fname),
            }.render(f.caller_source.as_str(), path);
            // Demote chained-frame prefix so only the top line reads as `error:`.
            let note = note.replacen("error:", "note:", 1);
            out.push('\n');
            out.push_str(&note);
        }
        out
    }
}

#[cfg(not(target_arch = "wasm32"))]
impl core::fmt::Display for VmErr {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str(&self.render())
    }
}

/* Cold, out-of-line constructors keep the hot dispatch loop icache-friendly. */
#[cold] #[inline(never)] pub fn cold_heap() -> VmErr { VmErr::Heap }
#[cold] #[inline(never)] pub fn cold_budget() -> VmErr { VmErr::Budget }
#[cold] #[inline(never)] pub fn cold_depth() -> VmErr { VmErr::CallDepth }
#[cold] #[inline(never)] pub fn cold_type(m: &'static str) -> VmErr { VmErr::Type(m) }
#[cold] #[inline(never)] pub fn cold_value(m: &'static str) -> VmErr { VmErr::Value(m) }
#[cold] #[inline(never)] pub fn cold_index(m: &'static str) -> VmErr { VmErr::Raised(crate::s!("IndexError: ", str m)) }
#[cold] #[inline(never)] pub fn cold_key(m: &'static str) -> VmErr { VmErr::Raised(crate::s!("KeyError: ", str m)) }
#[cold] #[inline(never)] pub fn cold_runtime(m: &'static str) -> VmErr { VmErr::Runtime(m) }
#[cold] #[inline(never)] pub fn cold_overflow() -> VmErr { VmErr::Overflow }
