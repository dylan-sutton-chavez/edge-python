use crate::packages::Resolved;
use crate::vm::types::{HeapObj, HeapPool, Val, VmErr};

mod network;
pub(crate) mod actor;
mod time;

/* Built-in system modules, resolved after the manifest so user entries always win. */
pub(crate) fn resolve(name: &str) -> Option<Resolved> {
    let bindings = match name {
        "time" => time::bindings(),
        "network" => network::bindings(),
        "actor" => actor::bindings(),
        _ => return None,
    };
    Some(Resolved::Native { bindings, classes: Vec::new(), consts: Vec::new(), canonical: format!("native:{name}") })
}

fn str_arg(heap: &HeapPool, args: &[Val], i: usize, who: &'static str) -> Result<String, VmErr> {
    if let Some(v) = args.get(i)
        && v.is_heap()
        && let HeapObj::Str(s) = heap.get(*v) {
        return Ok(s.clone());
    }
    Err(VmErr::TypeMsg(format!("{who} expects a str at argument {}", i + 1)))
}

fn opt_str_arg(heap: &HeapPool, args: &[Val], i: usize, who: &'static str) -> Result<Option<String>, VmErr> {
    match args.get(i) {
        None => Ok(None),
        Some(v) if v.is_none() => Ok(None),
        Some(_) => str_arg(heap, args, i, who).map(Some),
    }
}

// Reads an int argument, the shape socket handles take.
fn int_arg(args: &[Val], i: usize, who: &'static str) -> Result<i64, VmErr> {
    match args.get(i) {
        Some(v) if v.is_int() => Ok(v.as_int()),
        _ => Err(VmErr::TypeMsg(format!("{who} expects an int at argument {}", i + 1))),
    }
}

/* Reads an int or float argument as f64, the numeric shapes web clocks accept. */
fn num_arg(heap: &HeapPool, args: &[Val], i: usize, who: &'static str) -> Result<Option<f64>, VmErr> {
    let Some(v) = args.get(i) else { return Ok(None) };
    if v.is_none() { return Ok(None) }
    if v.is_float() { return Ok(Some(v.as_float())) }
    if v.is_int() { return Ok(Some(v.as_int() as f64)) }
    if v.is_heap() && let HeapObj::LongInt(n) = heap.get(*v) { return Ok(Some(*n as f64)) }
    Err(VmErr::TypeMsg(format!("{who} expects a number at argument {}", i + 1)))
}
