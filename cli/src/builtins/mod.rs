pub mod network;
pub mod time;

use crate::host::{Completion, Deferred};
use compiler::abi::WireValue;
use std::sync::mpsc::Sender;

/* The capability modules built into the CLI, each export flagged when it defers. */
pub fn exports(module: &str) -> Option<(&'static str, Vec<(&'static str, bool)>)> {
    match module {
        "time" => Some(("time", time::EXPORTS.to_vec())),
        "network" => Some(("network", network::EXPORTS.to_vec())),
        "actor" => Some(("actor", vec![("send", false)])),
        _ => None,
    }
}

/* Runs a synchronous export, the result crosses back as a transit value. */
pub fn call(module: &str, name: &str, args: &[WireValue]) -> Result<WireValue, String> {
    match module {
        "time" => time::call(name, args),
        "network" => network::call(name, args),
        _ => Err(format!("{module}.{name} is not a synchronous export")),
    }
}

/* Runs a deferred export on its own thread and reports through the channel. */
pub fn spawn(call: Deferred, tx: Sender<Completion>) {
    std::thread::spawn(move || {
        let result = match call.module {
            "time" => time::deferred(&call.name, &call.args),
            "network" => network::deferred(&call.name, &call.args),
            module => Err(format!("{module}.{} cannot defer", call.name)),
        };
        let _ = tx.send(match result {
            Ok(value) => Completion::Value { id: call.id, value },
            Err(msg) => Completion::Error { id: call.id, msg },
        });
    });
}

pub(crate) fn str_arg(args: &[WireValue], i: usize, who: &str) -> Result<String, String> {
    match args.get(i) {
        Some(WireValue::Bytes(b)) => Ok(String::from_utf8_lossy(b).into_owned()),
        _ => Err(format!("TypeError: {who} expects a str at argument {}", i + 1)),
    }
}

pub(crate) fn int_arg(args: &[WireValue], i: usize, who: &str) -> Result<i64, String> {
    match args.get(i) {
        Some(WireValue::Int(n)) => i64::try_from(*n).map_err(|_| format!("ValueError: {who} argument {} is out of range", i + 1)),
        Some(WireValue::Bool(b)) => Ok(*b as i64),
        _ => Err(format!("TypeError: {who} expects an int at argument {}", i + 1)),
    }
}

pub(crate) fn opt_str_arg(args: &[WireValue], i: usize, who: &str) -> Result<Option<String>, String> {
    match args.get(i) {
        None | Some(WireValue::None) => Ok(None),
        Some(_) => str_arg(args, i, who).map(Some),
    }
}

/* Reads an int or float argument as f64, the numeric shapes clocks accept. */
pub(crate) fn num_arg(args: &[WireValue], i: usize, who: &str) -> Result<Option<f64>, String> {
    match args.get(i) {
        None | Some(WireValue::None) => Ok(None),
        Some(WireValue::Float(f)) => Ok(Some(*f)),
        Some(WireValue::Int(n)) => Ok(Some(*n as f64)),
        Some(WireValue::Bool(b)) => Ok(Some(*b as i64 as f64)),
        Some(_) => Err(format!("TypeError: {who} expects a number at argument {}", i + 1)),
    }
}

pub(crate) fn text(s: String) -> WireValue {
    WireValue::Bytes(s.into_bytes())
}
