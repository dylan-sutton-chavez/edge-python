#[cfg(test)]
mod test {

    use compiler::modules::lexer::lex;
    use compiler::modules::parser::{Parser, SSAChunk};
    use compiler::modules::vm::VM;
    use compiler::modules::vm::snapshot;
    use compiler::modules::vm::types::{Limits, SchedulerStatus, VmErr};

    #[derive(serde::Deserialize)]
    struct Case {
        src: String,
        #[serde(default)]
        output: Vec<String>,
        #[serde(default)]
        error: Option<String>,
        #[serde(default)]
        input: Vec<String>,
        #[serde(default)]
        events: Vec<String>,
        #[serde(default)]
        interactive_events: Vec<String>,
        // Substrings that must appear in the introspection JSON at the first pause.
        #[serde(default)]
        globals_contains: Vec<String>,
        #[serde(default)]
        stack_contains: Vec<String>,
    }

    fn parse_ok(src: &str) -> SSAChunk {
        let (tokens, lex_errs) = lex(src);
        assert!(lex_errs.is_empty(), "lex error in {:?}", src);
        let (mut chunk, errs) = Parser::new(src, tokens.into_iter()).parse();
        assert!(errs.is_empty(), "parse error in {:?}: {:?}", src, errs.iter().map(|e| &e.msg).collect::<Vec<_>>());
        compiler::modules::vm::optimizer::constant_fold(&mut chunk);
        chunk
    }

    fn boot<'a>(chunk: &'a SSAChunk, case: &Case) -> VM<'a> {
        let mut vm = VM::with_limits(chunk, Limits::sandbox());
        vm.input_buffer = case.input.clone();
        for evt in &case.events { vm.push_event(evt).expect("push_event"); }
        vm
    }

    /* Uninterrupted reference run; mirrors the vm.json driver. */
    fn run_direct(case: &Case) -> Result<Vec<String>, VmErr> {
        let chunk = parse_ok(&case.src);
        let mut vm = boot(&chunk, case);
        let mut idx = 0;
        loop {
            match vm.run() {
                Ok(_) => return Ok(vm.output.clone()),
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    if idx >= case.interactive_events.len() { return Ok(vm.output.clone()); }
                    vm.push_event(&case.interactive_events[idx]).expect("push_event");
                    idx += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    /* At every PendingEvent pause: save, drop the whole VM and chunk, re-parse, restore `hops` times, then continue with the next event. Also checks save/restore idempotency when hops > 1 and runs introspection assertions at the first pause. */
    fn run_roundtrip(case: &Case, hops: u32) -> Result<Vec<String>, VmErr> {
        let mut pending: Option<(Vec<u8>, String)> = None;
        let mut idx = 0;
        loop {
            let chunk = parse_ok(&case.src);
            let mut vm;
            match pending.take() {
                None => vm = boot(&chunk, case),
                Some((mut blob, event)) => {
                    for _ in 1..hops {
                        let hop_chunk = parse_ok(&case.src);
                        let mut hop_vm = VM::with_limits(&hop_chunk, Limits::sandbox());
                        snapshot::restore(&mut hop_vm, &blob).expect("restore hop");
                        blob = snapshot::save(&hop_vm, &case.src);
                    }
                    assert_eq!(snapshot::source_of(&blob).expect("source_of"), case.src);
                    vm = VM::with_limits(&chunk, Limits::sandbox());
                    snapshot::restore(&mut vm, &blob).expect("restore");
                    vm.push_event(&event).expect("push_event");
                }
            }
            match vm.run() {
                Ok(_) => return Ok(vm.output.clone()),
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    if idx >= case.interactive_events.len() { return Ok(vm.output.clone()); }
                    if idx == 0 { check_introspection(&vm, case); }
                    let blob = snapshot::save(&vm, &case.src);
                    pending = Some((blob, case.interactive_events[idx].clone()));
                    idx += 1;
                }
                Err(e) => return Err(e),
            }
        }
    }

    fn check_introspection(vm: &VM, case: &Case) {
        if !case.globals_contains.is_empty() {
            let json = snapshot::inspect_globals(vm);
            for needle in &case.globals_contains {
                assert!(json.contains(needle.as_str()), "globals JSON missing {:?} in {:?}\n  got: {}", needle, case.src, json);
            }
        }
        if !case.stack_contains.is_empty() {
            let json = snapshot::inspect_stack(vm);
            for needle in &case.stack_contains {
                assert!(json.contains(needle.as_str()), "stack JSON missing {:?} in {:?}\n  got: {}", needle, case.src, json);
            }
        }
    }

    fn check(case: &Case, label: &str, result: Result<Vec<String>, VmErr>, failures: &mut Vec<String>) {
        match result {
            Ok(output) => {
                if case.error.is_some() {
                    failures.push(format!("[{label}] {:?}\n   expected error, got output {:?}", case.src, output));
                } else if output != case.output {
                    failures.push(format!("[{label}] {:?}\n   got {:?}\n   want {:?}", case.src, output, case.output));
                }
            }
            Err(e) => match &case.error {
                Some(expected) if e.to_string().contains(expected.as_str()) => {}
                Some(expected) => failures.push(format!("[{label}] {:?}\n   got error '{e}'\n   want '{expected}'", case.src)),
                None => failures.push(format!("[{label}] {:?}\n   unexpected error: {e}", case.src)),
            },
        }
    }

    /* Every snapshot.json case runs three ways and must behave identically: straight through, save/restore at every pause, and save/restore/save/restore (idempotency). */
    #[test]
    fn snapshot_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/snapshot.json")).expect("invalid JSON");
        let mut failures = Vec::new();
        for case in &cases {
            assert!(!case.interactive_events.is_empty(), "snapshot case needs interactive_events: {:?}", case.src);
            check(case, "direct", run_direct(case), &mut failures);
            check(case, "roundtrip", run_roundtrip(case, 1), &mut failures);
            check(case, "double", run_roundtrip(case, 2), &mut failures);
        }
        if !failures.is_empty() {
            let shown = failures.len().min(40);
            panic!("{} snapshot case(s) failed:\n{}", failures.len(), failures[..shown].join("\n"));
        }
    }

    /* Property: every interactive vm.json case must produce identical output with a save/restore cycle at each pause. */
    #[test]
    fn vm_corpus_roundtrip() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");
        let mut failures = Vec::new();
        for case in cases.iter().filter(|c| !c.interactive_events.is_empty() && c.error.is_none()) {
            let direct = run_direct(case);
            let cycled = run_roundtrip(case, 1);
            match (direct, cycled) {
                (Ok(a), Ok(b)) if a == b => {}
                (Ok(a), Ok(b)) => failures.push(format!("{:?}\n   direct {:?}\n   cycled {:?}", case.src, a, b)),
                (Err(a), Err(b)) if a.to_string() == b.to_string() => {}
                (a, b) => failures.push(format!("{:?}\n   direct {:?}\n   cycled {:?}", case.src, a.map(|_| ()), b.map(|_| ()))),
            }
        }
        if !failures.is_empty() {
            panic!("{} corpus roundtrip mismatch(es):\n{}", failures.len(), failures.join("\n"));
        }
    }

    /* A parked snapshot survives with pre-queued but unconsumed events, and restore is possible more than once from the same blob. */
    #[test]
    fn blob_reusable() {
        let src = "n = 0\nwhile True:\n    m = receive()\n    if m == 'stop':\n        break\n    n = n + 1\nprint(n)";
        let chunk = parse_ok(src);
        let mut vm = VM::with_limits(&chunk, Limits::sandbox());
        assert!(matches!(vm.run(), Err(VmErr::HostYield(SchedulerStatus::PendingEvent))));
        vm.push_event("a").unwrap();
        assert!(matches!(vm.run(), Err(VmErr::HostYield(SchedulerStatus::PendingEvent))));
        let blob = snapshot::save(&vm, src);
        drop(vm);

        for _ in 0..3 {
            let chunk2 = parse_ok(src);
            let mut vm2 = VM::with_limits(&chunk2, Limits::sandbox());
            snapshot::restore(&mut vm2, &blob).expect("restore");
            vm2.push_event("b").unwrap();
            assert!(matches!(vm2.run(), Err(VmErr::HostYield(SchedulerStatus::PendingEvent))));
            vm2.push_event("stop").unwrap();
            vm2.run().expect("finish");
            assert_eq!(vm2.output, vec!["2"]);
        }
    }

    /* Corrupt, truncated and cross-program blobs must fail with a clean error, never a panic or a silently wrong VM. */
    #[test]
    fn corrupt_blobs() {
        let src = "x = [1, 2, {'k': 3}]\nreceive()\nprint(x)";
        let chunk = parse_ok(src);
        let mut vm = VM::with_limits(&chunk, Limits::sandbox());
        assert!(matches!(vm.run(), Err(VmErr::HostYield(SchedulerStatus::PendingEvent))));
        let blob = snapshot::save(&vm, src);
        drop(vm);

        let fresh = || {
            let c = parse_ok(src);
            (c, ())
        };

        // Truncation at every 97th byte.
        for cut in (0..blob.len()).step_by(97) {
            let (chunk2, _) = fresh();
            let mut vm2 = VM::with_limits(&chunk2, Limits::sandbox());
            assert!(snapshot::restore(&mut vm2, &blob[..cut]).is_err(), "truncated blob at {cut} restored");
        }

        // Bad magic.
        let mut bad = blob.clone();
        bad[0] ^= 0xFF;
        let (chunk2, _) = fresh();
        let mut vm2 = VM::with_limits(&chunk2, Limits::sandbox());
        assert!(snapshot::restore(&mut vm2, &bad).is_err());

        // Unsupported format version.
        let mut bad = blob.clone();
        bad[4] = 0xEE;
        let mut vm2 = VM::with_limits(&chunk, Limits::sandbox());
        assert!(snapshot::restore(&mut vm2, &bad).is_err());

        // Cross-program restore: fingerprint must reject a VM booted from different source.
        let other = "y = 99\nreceive()\nprint(y)";
        let chunk3 = parse_ok(other);
        let mut vm3 = VM::with_limits(&chunk3, Limits::sandbox());
        let err = snapshot::restore(&mut vm3, &blob).unwrap_err();
        assert!(err.contains("does not match"), "unexpected error: {err}");

        // Trailing garbage.
        let mut bad = blob.clone();
        bad.push(0);
        let (chunk2, _) = fresh();
        let mut vm2 = VM::with_limits(&chunk2, Limits::sandbox());
        assert!(snapshot::restore(&mut vm2, &bad).is_err());

        // The pristine blob still restores after all of the above.
        let (chunk2, _) = fresh();
        let mut vm2 = VM::with_limits(&chunk2, Limits::sandbox());
        snapshot::restore(&mut vm2, &blob).expect("pristine blob");
        vm2.push_event("go").unwrap();
        vm2.run().expect("finish");
        assert_eq!(vm2.output, vec!["[1, 2, {'k': 3}]"]);
    }
}
