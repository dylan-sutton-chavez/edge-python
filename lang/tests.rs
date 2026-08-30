use lang::{Engine, Error, Limits, Value};

/* A syntax error surfaces as Error::Compile with a rendered caret. */
#[test]
fn compile_error() {
    let engine = Engine::builder().build();
    let err = engine.compile("def (:").unwrap_err();
    assert!(matches!(err, Error::Compile(_)));
    assert!(err.message().contains("error:"));
}

/* A raised exception surfaces as Error::Run with a traceback. */
#[test]
fn run_error() {
    let engine = Engine::builder().build();
    let program = engine.compile("raise ValueError('boom')").unwrap();
    let err = program.run().unwrap_err();
    assert!(matches!(err, Error::Run(_)));
    assert!(err.message().contains("boom"));
}

/* An injected native is importable from the host module, and the module value is None. */
#[test]
fn native_injection() {
    let engine = Engine::builder()
        .define("double", |args| Ok(Value::Int(args[0].as_int().unwrap_or(0) * 2)))
        .build();
    let out = engine
        .compile("from host import double\nprint(double(21))")
        .unwrap()
        .run()
        .unwrap();
    assert_eq!(out.text(), "42\n");
    assert_eq!(*out.value(), Value::None);
}

/* A native receiving and returning a string round-trips through Value. */
#[test]
fn native_string_roundtrip() {
    let engine = Engine::builder()
        .define("shout", |args| {
            let s = args[0].as_str().unwrap_or("");
            Ok(Value::Str(s.to_uppercase()))
        })
        .build();
    let program = engine
        .compile("from host import shout\nprint(shout('edge'))")
        .unwrap();
    assert_eq!(program.run().unwrap().text(), "EDGE\n");
}

/* A native raising a String error becomes a run traceback. */
#[test]
fn native_error() {
    let engine = Engine::builder()
        .define("boom", |_| Err("nope".to_string()))
        .build();
    let program = engine.compile("from host import boom\nboom()").unwrap();
    let err = program.run().unwrap_err();
    assert!(err.message().contains("nope"));
}

/* A native error naming a class ahead of the message is catchable as that class. */
#[test]
fn native_error_class() {
    let engine = Engine::builder()
        .define("boom", |_| Err("ValueError: bad input".to_string()))
        .build();
    let program = engine
        .compile("from host import boom\ntry:\n  boom()\nexcept ValueError:\n  print('caught')")
        .unwrap();
    assert_eq!(program.run().unwrap().text(), "caught\n");
}

/* A composite argument has no owned form, so the native refuses it instead of seeing a blank. */
#[test]
fn native_rejects_composite() {
    let engine = Engine::builder()
        .define("take", |args| Ok(Value::Int(args.len() as i64)))
        .build();
    let program = engine.compile("from host import take\ntake([1, 2])").unwrap();
    let err = program.run().unwrap_err();
    assert!(err.message().contains("scalar"));
}

/* Natives are positional only, a keyword argument is a TypeError rather than a silent drop. */
#[test]
fn native_rejects_kwargs() {
    let engine = Engine::builder()
        .define("double", |args| Ok(Value::Int(args[0].as_int().unwrap_or(0) * 2)))
        .build();
    let program = engine.compile("from host import double\ndouble(n=21)").unwrap();
    let err = program.run().unwrap_err();
    assert!(err.message().contains("keyword"));
}

/* An int past the NaN-box range crosses as a wide int and narrows back to Int. */
#[test]
fn native_wide_int_roundtrip() {
    let engine = Engine::builder()
        .define("big", |_| Ok(Value::Int(9_000_000_000_000_000)))
        .build();
    let program = engine
        .compile("from host import big\ndef get():\n  return big()")
        .unwrap();
    assert_eq!(*program.call("get", &[]).unwrap().value(), Value::Int(9_000_000_000_000_000));
}

/* A tightened op budget trips the sandbox instead of hanging. */
#[test]
fn limits_enforced() {
    let tight = Limits { calls: 8, ops: 500, heap: 1000 };
    let engine = Engine::builder().limits(tight).build();
    let program = engine.compile("i = 0\nwhile i < 1000000:\n  i = i + 1").unwrap();
    let err = program.run().unwrap_err();
    assert!(matches!(err, Error::Run(_)));
}

/* Scalars convert into Value through From. */
#[test]
fn value_conversions() {
    assert_eq!(Value::from(7i64), Value::Int(7));
    assert_eq!(Value::from(true), Value::Bool(true));
    assert_eq!(Value::from("x"), Value::Str("x".to_string()));

    // Scalar accessors mirror the variants.
    assert_eq!(Value::Bool(true).as_bool(), Some(true));
    assert_eq!(Value::Int(5).as_bool(), None);
    assert_eq!(Value::Int(5).as_float(), Some(5.0));
}

/* Display renders values the way the language prints them. */
#[test]
fn value_display() {
    assert_eq!(Value::Bool(true).to_string(), "True");
    assert_eq!(Value::None.to_string(), "None");
    assert_eq!(Value::Int(42).to_string(), "42");
}

/* A keyword alias renames a construct, the engine sees the mapped word. */
#[test]
fn keyword_alias() {
    let engine = Engine::builder()
        .keyword("función", "def")
        .keyword("imprime", "print")
        .build();
    let program = engine
        .compile("función saluda():\n  imprime('hola')\nsaluda()")
        .unwrap();
    assert_eq!(program.run().unwrap().text(), "hola\n");
}

/* A keyword alias is whole-word, it never rewrites a substring of a longer name. */
#[test]
fn keyword_whole_word() {
    let engine = Engine::builder().keyword("if", "XX").build();
    // `ifx` must stay intact, only a standalone `if` maps.
    let program = engine.compile("ifx = 3\nprint(ifx)").unwrap();
    assert_eq!(program.run().unwrap().text(), "3\n");
}

/* A keyword alias reads tokens, so the same word inside a literal or a comment is left alone. */
#[test]
fn keyword_skips_literals() {
    let engine = Engine::builder().keyword("imprime", "print").build();
    let program = engine.compile("imprime('imprime')  # imprime\n").unwrap();
    assert_eq!(program.run().unwrap().text(), "imprime\n");
}

/* Transforms chain, each fed the previous output. */
#[test]
fn transform_chain() {
    let engine = Engine::builder()
        .transform(|src| src.replace("SHOW", "emit"))
        .keyword("emit", "print")
        .build();
    let program = engine.compile("SHOW(7)").unwrap();
    assert_eq!(program.run().unwrap().text(), "7\n");
}

/* A named function is callable with args, its Output carries the return value. */
#[test]
fn call_function() {
    let engine = Engine::builder().build();
    let program = engine
        .compile("def check(n):\n  return n > 10")
        .unwrap();
    assert_eq!(*program.call("check", &[Value::Int(21)]).unwrap().value(), Value::Bool(true));
    assert_eq!(*program.call("check", &[Value::Int(3)]).unwrap().value(), Value::Bool(false));
}

/* Program::call runs on a fresh instance each time, so calls never leak between invocations. */
#[test]
fn call_is_isolated() {
    let engine = Engine::builder().build();
    let program = engine.compile("seen = []\ndef add(x):\n  seen.append(x)\n  return len(seen)").unwrap();
    assert_eq!(*program.call("add", &[Value::Int(1)]).unwrap().value(), Value::Int(1));
    assert_eq!(*program.call("add", &[Value::Int(2)]).unwrap().value(), Value::Int(1));
}

/* An instance keeps the module state alive, so successive calls build on each other. */
#[test]
fn instance_shares_state() {
    let engine = Engine::builder().build();
    let program = engine.compile("seen = []\ndef add(x):\n  seen.append(x)\n  return len(seen)").unwrap();
    let mut inst = program.start().unwrap();
    assert_eq!(*inst.call("add", &[Value::Int(1)]).unwrap().value(), Value::Int(1));
    assert_eq!(*inst.call("add", &[Value::Int(2)]).unwrap().value(), Value::Int(2));
}

/* The module body runs once at start, so its output is never replayed onto a later call. */
#[test]
fn instance_runs_body_once() {
    let engine = Engine::builder().build();
    let program = engine.compile("print('boot')\ndef go():\n  print('call')\n  return 1").unwrap();
    let mut inst = program.start().unwrap();
    assert_eq!(inst.call("go", &[]).unwrap().text(), "call\n");
    assert_eq!(inst.call("go", &[]).unwrap().text(), "call\n");
}

/* A call captures output printed during the call alongside the return value. */
#[test]
fn call_captures_output() {
    let engine = Engine::builder().build();
    let program = engine.compile("def go():\n  print('side')\n  return 1").unwrap();
    let out = program.call("go", &[]).unwrap();
    assert_eq!(out.text(), "side\n");
    assert_eq!(*out.value(), Value::Int(1));
}

/* Calling an unknown name is an Error::Run, not a panic. */
#[test]
fn call_unknown() {
    let engine = Engine::builder().build();
    let program = engine.compile("x = 1").unwrap();
    assert!(matches!(program.call("missing", &[]), Err(Error::Run(_))));
}


