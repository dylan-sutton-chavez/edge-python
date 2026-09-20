#[cfg(test)]
mod test {

    use compiler::lexer::lex;
    use compiler::parser::Parser;
    use compiler::vm::VM;
    use compiler::vm::types::Limits;
    use compiler::vm::types::{SchedulerStatus, VmErr};

    #[derive(serde::Deserialize)]
    #[serde(deny_unknown_fields)]
    struct Case {
        src: String,
        output: Vec<String>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        input: Vec<String>,
        #[serde(default)]
        events: Vec<String>,
        // Events pushed one-at-a-time after each PendingEvent yield (host-resume path).
        #[serde(default)]
        interactive_events: Vec<String>,
        // Present installs a scheduler hook, the (group, body) pairs send() handed over.
        #[serde(default)]
        sends: Option<Vec<(String, String)>>,
    }

    std::thread_local! {
        static SENT: std::cell::RefCell<Vec<(String, String)>> = const { std::cell::RefCell::new(Vec::new()) };
    }

    fn record_send(group: &str, body: &str) -> bool {
        SENT.with(|s| s.borrow_mut().push((group.to_string(), body.to_string())));
        true
    }

    // Installs the recording hook when the case expects sends, clearing what an earlier case left.
    fn hook_sends(vm: &mut VM, case: &Case) {
        SENT.with(|s| s.borrow_mut().clear());
        if case.sends.is_some() { vm.send_hook = Some(record_send); }
    }

    fn sent() -> Vec<(String, String)> {
        SENT.with(|s| s.borrow().clone())
    }

    /* Sets iterate in hash order, so canonicalize a set/frozenset line by sorting its elements. Assumes scalar elements with no nested ", ". Non-sets pass through. */
    fn normalize_set(line: &str) -> String {
        let (prefix, inner, suffix) = if let Some(i) =
            line.strip_prefix("frozenset({").and_then(|r| r.strip_suffix("})")) {
            ("frozenset({", i, "})")
        } else if line.starts_with('{') && line.ends_with('}') && line.len() > 2 && !line.contains(": ") {
            ("{", &line[1..line.len() - 1], "}")
        } else {
            return line.to_string();
        };
        let mut elems: Vec<&str> = inner.split(", ").collect();
        elems.sort_unstable();
        format!("{}{}{}", prefix, elems.join(", "), suffix)
    }

    // Apply set normalization line-by-line so both sides compare order-independent.
    fn normalize(lines: &[String]) -> Vec<String> {
        lines.iter().map(|l| normalize_set(l)).collect()
    }

    // Resume on each PendingEvent by pushing the next interactive_events entry.
    fn drive(vm: &mut VM, interactive: &[String]) -> Result<(), VmErr> {
        let mut idx = 0;
        loop {
            match vm.run() {
                Ok(_) => return Ok(()),
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    if idx >= interactive.len() { return Ok(()); }
                    vm.push_event(&interactive[idx]).expect("push_event");
                    idx += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /* Runs vm.json cases under sandbox limits testing budget heap depth guards verifying runaway allocation recursion materialization produce MemoryError RecursionError */
    #[test]
    fn test_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");

        let mut failures: Vec<String> = Vec::new();
        for case in cases {
            let (tokens, lex_errs) = lex(&case.src);
            // Skip cases whose expected error is already raised by the lexer.
            if !lex_errs.is_empty()
                && let Some(expected) = &case.error
                && lex_errs.iter().any(|e| e.msg.contains(expected.as_str()))
            {
                continue;
            }
            let (mut chunk, _errors) = Parser::new(&case.src, tokens.into_iter()).parse();
            // Run the same fold pass production does (exports.rs), so fold-path bugs surface here.
            compiler::vm::optimizer::constant_fold(&mut chunk);
            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            vm.input_buffer = case.input.clone();
            hook_sends(&mut vm, &case);
            for evt in &case.events { vm.push_event(evt).expect("push_event"); }
            let result = drive(&mut vm, &case.interactive_events);

            // Collect every mismatch (not panic-on-first) so one run lists all regressions.
            match result {
                Ok(_obj) => {
                    if normalize(&vm.output) != normalize(&case.output) {
                        failures.push(format!("OUTPUT {:?}\n   got {:?}\n   want {:?}", case.src, vm.output, case.output));
                    }
                    if let Some(want) = &case.sends && &sent() != want {
                        failures.push(format!("SENDS {:?}\n   got {:?}\n   want {:?}", case.src, sent(), want));
                    }
                }
                Err(e) => match &case.error {
                    Some(expected) => if !e.to_string().contains(expected.as_str()) {
                        failures.push(format!("ERR {:?}\n   got '{}'\n   want '{}'", case.src, e, expected));
                    },
                    None => failures.push(format!("RAISED {:?}: {}", case.src, e)),
                }
            }
        }
        if !failures.is_empty() {
            let shown = failures.len().min(80);
            panic!("{} case(s) failed:\n{}", failures.len(), failures[..shown].join("\n"));
        }
    }

    /* Reruns every vm.json case in strict_input mode (host-supplied buffer, reading past = RuntimeError) under `Limits::sandbox()` (see `test_cases` for why the bounded profile). Lex/parse errors are also asserted here. */
    #[test]
    fn strict_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");

        for case in cases {
            let (tokens, lex_errs) = lex(&case.src);
            // Lex errors are surfaced as diagnostics, match against the expected error and move on.
            if !lex_errs.is_empty() {
                if let Some(expected) = &case.error {
                    assert!(
                        lex_errs.iter().any(|e| e.msg.contains(expected.as_str())),
                        "wrong lex error on {:?}: got {:?}, expected '{}'",
                        case.src,
                        lex_errs.iter().map(|e| e.msg).collect::<Vec<_>>(),
                        expected
                    );
                    continue;
                }
                panic!("lex error on {:?}: {:?}", case.src, lex_errs.iter().map(|e| e.msg).collect::<Vec<_>>());
            }
            let (mut chunk, errs) = Parser::new(&case.src, tokens.into_iter()).parse();
            if !errs.is_empty() {
                match &case.error {
                    Some(expected) => {
                        assert!(
                            errs.iter().any(|e| e.msg.contains(expected.as_str())),
                            "wrong parse error on {:?}: got {:?}, expected '{}'",
                            case.src,
                            errs.iter().map(|e| &e.msg).collect::<Vec<_>>(),
                            expected
                        );
                        continue;
                    }
                    None => panic!("parse error on {:?}: {:?}", case.src, errs.iter().map(|e| &e.msg).collect::<Vec<_>>()),
                }
            }
            // Match production and fold before running.
            compiler::vm::optimizer::constant_fold(&mut chunk);

            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            vm.input_buffer = case.input.clone();
            hook_sends(&mut vm, &case);
            for evt in &case.events { vm.push_event(evt).expect("push_event"); }
            let expects_input_error = case.input.is_empty() && (case.src.contains("input(") || case.src.contains("input ("));

            match drive(&mut vm, &case.interactive_events) {
                Ok(_) => {
                    assert!(!expects_input_error, "expected input() to error under strict mode for: {:?}", case.src);
                    assert_eq!(normalize(&vm.output), normalize(&case.output), "output mismatch on: {:?}", case.src);
                    if let Some(want) = &case.sends { assert_eq!(&sent(), want, "sends mismatch on: {:?}", case.src); }
                }
                Err(e) => match &case.error {
                    Some(expected) => assert!(e.to_string().contains(expected.as_str()), "wrong error on {:?}: got '{}', expected '{}'", case.src, e, expected),
                    None if expects_input_error => assert!(
                        e.to_string().contains("input"),
                        "expected input RuntimeError under strict mode for: {:?}, got: {}",
                        case.src, e
                    ),
                    None => panic!("VM error on {:?}: {}", case.src, e),
                }
            }
        }
    }

    /* An error the host delivers into a parked call renders like one the call raised itself. */
    mod host_errors {
        use compiler::lexer::lex;
        use compiler::parser::{Parser, SSAChunk};
        use compiler::vm::VM;
        use compiler::vm::snapshot;
        use compiler::vm::types::{Limits, SchedulerStatus, VmErr};

        use crate::common::{test_native, TestResolver};

        // Compiles `src` against a module `m` whose `host_defer` always defers to the host.
        fn compile(src: &str) -> SSAChunk {
            let resolver = TestResolver::new().with_native("m", vec![test_native("host_defer").unwrap()]).with_alias("m", "m");
            let (tokens, _) = lex(src);
            let (mut chunk, errs) = Parser::with_resolver(src, tokens.into_iter(), Box::new(resolver)).parse();
            assert!(errs.is_empty(), "parse errors on {src:?}: {:?}", errs.iter().map(|e| &e.msg).collect::<Vec<_>>());
            compiler::vm::optimizer::constant_fold(&mut chunk);
            chunk
        }

        fn render(vm: &VM, e: &VmErr, src: &str) -> String {
            e.render_traceback(src, vm.error_pos(), Some("main.py"), vm.call_stack_frames(), vm.function_names_ref())
        }

        // Runs `src`, answering every deferred call with `error`, returns the output and any rendered traceback.
        fn run(src: &str, error: &str) -> (Vec<String>, Option<String>) {
            let chunk = compile(src);
            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            let mut next_id = 0u64;
            loop {
                match vm.run() {
                    Ok(_) => return (vm.output.clone(), None),
                    Err(VmErr::HostYield(SchedulerStatus::PendingHostCall)) => {
                        assert!(vm.push_host_error_by_id(next_id, error), "no call parked on id {next_id}");
                        next_id += 1;
                    }
                    Err(e) => return (vm.output.clone(), Some(render(&vm, &e, src))),
                }
            }
        }

        // The location lines of a traceback, the part that must not depend on how the error arrived.
        fn locations(tb: &str) -> Vec<&str> {
            tb.lines().filter(|l| l.contains("-->") || l.starts_with("note:") || l.starts_with("error:")).collect()
        }

        fn traceback(src: &str) -> String {
            let (_, tb) = run(src, "RuntimeError: boom");
            tb.unwrap_or_else(|| panic!("expected an uncaught error from {src:?}"))
        }

        #[test]
        fn uncaught_at_top_level_points_at_the_call() {
            let tb = traceback("from m import host_defer\nprint('before')\nhost_defer()\nprint('after')\n");
            assert!(tb.contains("RuntimeError: boom"), "{tb}");
            assert!(tb.contains("--> main.py:3:1"), "{tb}");
            assert!(tb.contains("3 | host_defer()"), "{tb}");
        }

        #[test]
        fn uncaught_in_a_function_keeps_the_called_from_note() {
            let tb = traceback("from m import host_defer\ndef helper():\n    x = 1\n    host_defer()\nhelper()\n");
            assert!(tb.contains("--> main.py:4:5"), "{tb}");
            assert!(tb.contains("note: called from helper()"), "{tb}");
            assert!(tb.contains(":5:7"), "{tb}");
        }

        #[test]
        fn caught_error_leaves_no_stale_position() {
            let src = "from m import host_defer\ntry:\n    host_defer()\nexcept RuntimeError as e:\n    print('caught', e)\nx = 1 / 0\n";
            let (out, tb) = run(src, "RuntimeError: boom");
            assert_eq!(out, vec!["caught boom"]);
            let tb = tb.expect("the later ZeroDivisionError escapes");
            assert!(tb.contains("ZeroDivisionError"), "{tb}");
            assert!(tb.contains("--> main.py:6:1"), "{tb}");
        }

        #[test]
        fn a_callers_handler_catches_an_error_raised_inside_a_resumed_helper() {
            let src = "from m import host_defer\ndef helper():\n    host_defer()\n    print('helper continued')\ntry:\n    helper()\nexcept RuntimeError:\n    print('caught in caller')\nprint('end')\n";
            let (out, tb) = run(src, "RuntimeError: boom");
            assert_eq!(tb, None);
            assert_eq!(out, vec!["caught in caller", "end"]);
        }

        #[test]
        fn a_gather_child_error_points_at_the_childs_call() {
            let tb = traceback("from m import host_defer\nasync def child():\n    host_defer()\ngather(child())\n");
            assert!(tb.contains("--> main.py:3:5"), "{tb}");
        }

        #[test]
        fn nested_helpers_render_like_an_error_raised_without_suspending() {
            let layout = |stmt: &str| format!("from m import host_defer\ndef inner():\n    {stmt}\ndef outer():\n    inner()\nouter()\n");
            let delivered = traceback(&layout("host_defer()"));
            let raised = traceback(&layout("raise RuntimeError('boom')"));
            assert_eq!(locations(&delivered), locations(&raised), "\n{delivered}\n{raised}");
            assert!(delivered.contains("note: called from inner()") && delivered.contains("note: called from outer()"), "{delivered}");
        }

        #[test]
        fn an_ordinary_error_after_a_helper_resumed_reaches_the_callers_handler() {
            let src = "def helper():\n    sleep(0.01)\n    raise ValueError('x')\ntry:\n    helper()\nexcept ValueError:\n    print('caught')\nprint('end')\n";
            let (out, tb) = run(src, "unused");
            assert_eq!(tb, None);
            assert_eq!(out, vec!["caught", "end"]);
        }

        #[test]
        fn an_ordinary_error_after_a_helper_resumed_keeps_its_line_and_note() {
            let resumed = traceback("def helper():\n    sleep(0.01)\n    x = 1 / 0\nhelper()\n");
            let direct = traceback("def helper():\n    pass\n    x = 1 / 0\nhelper()\n");
            assert!(resumed.contains("--> main.py:3:5"), "{resumed}");
            assert_eq!(locations(&resumed), locations(&direct), "\n{resumed}\n{direct}");
        }

        #[test]
        fn a_pending_raise_survives_a_snapshot() {
            let src = "from m import host_defer\nprint('before')\nhost_defer()\nprint('after')\n";
            let chunk = compile(src);
            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            assert!(matches!(vm.run(), Err(VmErr::HostYield(SchedulerStatus::PendingHostCall))));
            assert!(vm.push_host_error_by_id(0, "RuntimeError: boom"));
            let blob = snapshot::save(&vm, src);
            let mut restored = VM::with_limits(&chunk, Limits::sandbox());
            snapshot::restore(&mut restored, &blob).expect("restore");
            let e = restored.run().expect_err("the delivered error escapes after restore");
            let tb = render(&restored, &e, src);
            assert!(tb.contains("RuntimeError: boom") && tb.contains("--> main.py:3:1"), "{tb}");
            assert!(!restored.output.iter().any(|l| l == "after"), "{:?}", restored.output);
        }

        #[test]
        fn a_caught_gather_child_error_leaves_no_stale_position() {
            let src = "from m import host_defer\nasync def child():\n    host_defer()\ntry:\n    gather(child())\nexcept RuntimeError:\n    print('caught')\nx = 1 / 0\n";
            let (out, tb) = run(src, "RuntimeError: boom");
            assert_eq!(out, vec!["caught"]);
            let tb = tb.expect("the later ZeroDivisionError escapes");
            assert!(tb.contains("--> main.py:8:1"), "{tb}");
        }
    }
}
