use super::{read, read_u32, rt, stage, unstage, write, write_u32, Deferred, Exports, Native, State};
use crate::builtins;
use anyhow::{anyhow, Result};
use compiler::abi::WireValue;
use wasmtime::{Caller, Linker};

// The RUNTIME error kind of the ABI.
pub const ERR_RUNTIME: i32 = 2;

/* The four `env` imports compiler.wasm declares, bound to the store state. */
pub fn link(linker: &mut Linker<State>) -> Result<()> {
    linker
        .func_wrap("env", "host_print", |mut caller: Caller<'_, State>, ptr: i32, len: i32| {
            let ex = exports(&caller);
            let text = String::from_utf8_lossy(&read(&mut caller, ex.memory, ptr, len)).into_owned();
            (caller.data_mut().print)(&text);
        })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("env", "host_now_ns", |_: Caller<'_, State>| -> i64 { super::now_ns() as i64 })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("env", "host_fetch_bytes", |mut caller: Caller<'_, State>, spec_ptr: i32, spec_len: i32, _hash_ptr: i32, out_len: i32| -> wasmtime::Result<i32> {
            let ex = exports(&caller);
            let spec = String::from_utf8_lossy(&read(&mut caller, ex.memory, spec_ptr, spec_len)).into_owned();
            let Some(bytes) = caller.data().fetched.get(&spec).cloned() else {
                write_u32(&mut caller, ex.memory, out_len, 0);
                return Ok(0);
            };
            // The compiler frees this with wasm_free and the same length.
            let ptr = ex.wasm_alloc.call(&mut caller, bytes.len() as i32)?;
            write(&mut caller, ex.memory, ptr, &bytes);
            write_u32(&mut caller, ex.memory, out_len, bytes.len() as u32);
            Ok(ptr)
        })
        .map_err(|e| anyhow!("{e}"))?;
    linker
        .func_wrap("env", "host_call_native", |mut caller: Caller<'_, State>, id: i32, call_id: i32, argv_ptr: i32, argc: i32, out_ptr: i32| -> wasmtime::Result<i32> {
            call_native(&mut caller, id, call_id, argv_ptr, argc, out_ptr)
        })
        .map_err(|e| anyhow!("{e}"))?;
    Ok(())
}

fn exports(caller: &Caller<'_, State>) -> Exports {
    caller.data().exports.clone().expect("compiler exports bound before any host call")
}

/* Dispatches one extern call, plugins get staged argv, capabilities get decoded values. */
fn call_native(caller: &mut Caller<'_, State>, id: i32, call_id: i32, argv_ptr: i32, argc: i32, out_ptr: i32) -> wasmtime::Result<i32> {
    let ex = exports(caller);
    let Some(native) = caller.data().natives.get(id as usize).cloned() else {
        throw(caller, &ex, &format!("native id {id} not registered"));
        return Ok(1);
    };
    match native {
        Native::Plugin { func, alloc, free, memory } => {
            let argv = read(caller, ex.memory, argv_ptr, argc * 4);
            let len = (argc * 4).max(4);
            let g_argv = alloc.call(&mut *caller, len)?;
            let g_out = alloc.call(&mut *caller, 4)?;
            write(caller, memory, g_argv, &argv);
            let status = match func.call(&mut *caller, (g_argv, argc, g_out)) {
                Ok(status) => status,
                Err(e) => {
                    throw(caller, &ex, &format!("native module trapped: {e}"));
                    return Ok(1);
                }
            };
            if status == 0 {
                let handle = read_u32(caller, memory, g_out);
                write_u32(caller, ex.memory, out_ptr, handle);
            }
            if let Some(free) = free {
                let _ = free.call(&mut *caller, (g_argv, len));
                let _ = free.call(&mut *caller, (g_out, 4));
            }
            Ok(status)
        }
        Native::Capability { module, name, deferred } => {
            // The trailing kwargs slot is dropped, capabilities take positional values.
            let raw = read(caller, ex.memory, argv_ptr, (argc - 1).max(0) * 4);
            let mut args = Vec::with_capacity(raw.len() / 4);
            for handle in raw.chunks_exact(4).map(|c| u32::from_le_bytes([c[0], c[1], c[2], c[3]])) {
                match rt::decode(caller, &ex, handle) {
                    Ok(value) => args.push(value),
                    Err(e) => {
                        throw(caller, &ex, &e);
                        return Ok(1);
                    }
                }
            }
            if module == "actor" {
                return send(caller, &ex, &args, out_ptr);
            }
            if deferred {
                caller.data_mut().deferred.push(Deferred { id: call_id as u32, module, name, args });
                return Ok(2);
            }
            let result = builtins::call(module, &name, &args).and_then(|value| rt::encode(caller, &ex, &value));
            match result {
                Ok(handle) => {
                    write_u32(caller, ex.memory, out_ptr, handle);
                    Ok(0)
                }
                Err(e) => {
                    throw(caller, &ex, &e);
                    Ok(1)
                }
            }
        }
    }
}

/* actor.send queues a message on the outbox the scheduler drains after the step. */
fn send(caller: &mut Caller<'_, State>, ex: &Exports, args: &[WireValue], out_ptr: i32) -> wasmtime::Result<i32> {
    let text = |i: usize| match args.get(i) {
        Some(WireValue::Bytes(b)) => Ok(String::from_utf8_lossy(b).into_owned()),
        _ => Err(format!("actor.send expects a str at argument {}", i + 1)),
    };
    match text(0).and_then(|group| text(1).map(|body| (group, body))) {
        Ok(message) => caller.data_mut().outbox.push(message),
        Err(e) => {
            throw(caller, ex, &e);
            return Ok(1);
        }
    }
    match rt::encode(caller, ex, &WireValue::None) {
        Ok(handle) => {
            write_u32(caller, ex.memory, out_ptr, handle);
            Ok(0)
        }
        Err(e) => {
            throw(caller, ex, &e);
            Ok(1)
        }
    }
}

/* Stashes the error the compiler raises once the call returns 1, a class prefix picks its kind. */
pub(super) fn throw(caller: &mut Caller<'_, State>, ex: &Exports, msg: &str) {
    let (kind, msg) = match msg.split_once(": ") {
        Some(("TypeError", rest)) => (0, rest),
        Some(("ValueError", rest)) => (1, rest),
        _ => (ERR_RUNTIME, msg),
    };
    if let Ok(ptr) = stage(caller, ex, msg.as_bytes()) {
        let _ = ex.host_edge_throw.call(&mut *caller, (kind, ptr, msg.len() as i32));
        unstage(caller, ex, ptr, msg.len());
    }
}
