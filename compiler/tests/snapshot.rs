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

    /* Count preempts; `hop_at > 0` saves there and continues on a restored VM. */
    fn run_preempted(src: &str, interval: usize, hop_at: usize) -> (Vec<String>, usize, usize) {
        let chunk = parse_ok(src);
        let mut vm = VM::with_limits(&chunk, Limits::sandbox());
        vm.set_preempt_interval(interval);
        let mut preempts = 0;
        let mut blob_len = 0;
        loop {
            match vm.run() {
                Ok(_) => return (vm.output.clone(), preempts, blob_len),
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => {
                    preempts += 1;
                    if hop_at == 0 || preempts != hop_at { continue; }
                    let blob = snapshot::save(&vm, src);
                    blob_len = blob.len();
                    drop(vm);
                    let hop_chunk = parse_ok(src);
                    let mut hop_vm = VM::with_limits(&hop_chunk, Limits::sandbox());
                    hop_vm.set_preempt_interval(interval);
                    snapshot::restore(&mut hop_vm, &blob).expect("restore preempted");
                    // Rebind so the loop keeps driving the restored copy.
                    return match finish_preempted(hop_vm, &mut preempts) {
                        Ok(out) => (out, preempts, blob_len),
                        Err(e) => panic!("restored VM failed: {e}"),
                    };
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
    }

    fn finish_preempted(mut vm: VM, preempts: &mut usize) -> Result<Vec<String>, VmErr> {
        loop {
            match vm.run() {
                Ok(_) => return Ok(vm.output.clone()),
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => { *preempts += 1; }
                Err(e) => return Err(e),
            }
        }
    }

    /* A loop with no suspension point yields `Preempted` and produces identical output either way. */
    #[test]
    fn preempt_bare_loop() {
        let src = "n = 0\nwhile n < 4000:\n    n = n + 1\n    if n % 2000 == 0:\n        print(n)\nprint('done', n)";
        let (plain, none, _) = run_preempted(src, 0, 0);
        assert_eq!(none, 0, "interval 0 must not preempt");
        let (preempted, count, _) = run_preempted(src, 500, 0);
        assert!(count >= 7, "expected repeated preempts, got {count}");
        assert_eq!(plain, preempted);

        // An unmetered VM skips the budget decrement; the preempt check must not sit behind it.
        let chunk = parse_ok(src);
        let mut vm = VM::with_limits(&chunk, Limits::none());
        vm.set_preempt_interval(500);
        let mut unmetered = 0;
        loop {
            match vm.run() {
                Ok(_) => break,
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => unmetered += 1,
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(unmetered >= 7, "sandbox-off run preempted {unmetered} times");
        assert_eq!(vm.output, plain);
    }

    /* Three calls deep: two suspended sync frames plus a value-position return. */
    #[test]
    fn preempt_deep_call_roundtrip() {
        let src = "def level3(n):\n    t = 0\n    while t < 3000:\n        t = t + 1\n    return t + n\ndef level2(n):\n    return '[' + str(level3(n)) + ']'\ndef level1(n):\n    return level2(n) + '!'\nprint(level1(7))";
        let (plain, _, _) = run_preempted(src, 0, 0);
        assert_eq!(plain, vec!["[3007]!"]);
        let (hopped, count, blob_len) = run_preempted(src, 400, 2);
        assert_eq!(hopped, plain, "restored run diverged");
        assert!(count >= 5 && blob_len > 100, "count {count}, blob {blob_len}");
    }

    /* Native re-entry has no unwind path, so these never preempt but still finish correctly. */
    #[test]
    fn preempt_refused_under_native_reentry() {
        let class_src = "class C:\n    n = 0\n    while n < 3000:\n        n = n + 1\nprint(C.n)";
        let (out, count, _) = run_preempted(class_src, 50, 0);
        assert_eq!(out, vec!["3000"]);
        assert_eq!(count, 0, "class body must not preempt");

        let sort_src = "def key(x):\n    t = 0\n    while t < 2000:\n        t = t + 1\n    return -x\nitems = [3, 1, 2]\nitems.sort(key=key)\nprint(items)";
        let (out, count, _) = run_preempted(sort_src, 50, 0);
        assert_eq!(out, vec!["[3, 2, 1]"]);
        assert_eq!(count, 0, "sort key callback must not preempt");

        // `sorted` is its own opcode, reaching the callback outside the plain call path.
        let sorted_src = "def key(x):\n    t = 0\n    while t < 2000:\n        t = t + 1\n    return -x\nprint(sorted([3, 1, 2], key=key))";
        let (out, count, _) = run_preempted(sorted_src, 50, 0);
        assert_eq!(out, vec!["[3, 2, 1]"]);
        assert_eq!(count, 0, "sorted key callback must not preempt");

        let gen_src = "def gen():\n    for i in range(4):\n        t = 0\n        while t < 2000:\n            t = t + 1\n        yield i\nprint(sum(list(gen())))";
        let (out, count, _) = run_preempted(gen_src, 50, 0);
        assert_eq!(out, vec!["6"]);
        assert_eq!(count, 0, "generator body drained by a builtin must not preempt");
    }

    /* A method body stages like a plain call; two calls cover unfused and fused dispatch. */
    #[test]
    fn preempt_inside_method() {
        let src = "class Counter:\n    def __init__(self):\n        self.n = 0\n    def spin(self, limit):\n        while self.n < limit:\n            self.n = self.n + 1\n        return self.n\nc = Counter()\nc.spin(2000)\nprint(c.spin(4000))";
        let (plain, _, _) = run_preempted(src, 0, 0);
        assert_eq!(plain, vec!["4000"]);
        let (hopped, count, _) = run_preempted(src, 300, 2);
        assert!(count >= 5, "method body should preempt, got {count}");
        assert_eq!(hopped, plain);
    }

    /* Cooperative points keep working with preemption on, and a preempt blob still restores. */
    #[test]
    fn preempt_mixes_with_receive() {
        let src = "seen = []\nwhile True:\n    spin = 0\n    while spin < 1500:\n        spin = spin + 1\n    m = receive()\n    if m == 'stop':\n        break\n    seen.append(m)\nprint(','.join(seen))";
        let chunk = parse_ok(src);
        let mut vm = VM::with_limits(&chunk, Limits::sandbox());
        vm.set_preempt_interval(300);
        let mut preempts = 0;
        let mut blob = None;
        let events = ["a", "b", "stop"];
        let mut idx = 0;
        loop {
            match vm.run() {
                Ok(_) => break,
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => {
                    preempts += 1;
                    if blob.is_none() { blob = Some(snapshot::save(&vm, src)); }
                }
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    vm.push_event(events[idx]).expect("push_event");
                    idx += 1;
                }
                Err(e) => panic!("unexpected error: {e}"),
            }
        }
        assert!(preempts > 0, "spin loop should have preempted");
        assert_eq!(vm.output, vec!["a,b"]);

        let hop_chunk = parse_ok(src);
        let mut hop_vm = VM::with_limits(&hop_chunk, Limits::sandbox());
        hop_vm.set_preempt_interval(300);
        snapshot::restore(&mut hop_vm, &blob.expect("saved at a preempt")).expect("restore");
        let mut idx = 0;
        let events = ["x", "stop"];
        loop {
            match hop_vm.run() {
                Ok(_) => break,
                Err(VmErr::HostYield(SchedulerStatus::Preempted)) => {}
                Err(VmErr::HostYield(SchedulerStatus::PendingEvent)) => {
                    hop_vm.push_event(events[idx]).expect("push_event");
                    idx += 1;
                }
                Err(e) => panic!("unexpected error after restore: {e}"),
            }
        }
        assert_eq!(hop_vm.output, vec!["x"]);
    }

    /* Property: preempting at every back-edge must not change any corpus program's result. */
    #[test]
    fn vm_corpus_preempt_equivalence() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");
        let mut failures = Vec::new();
        for case in cases.iter().filter(|c| c.interactive_events.is_empty() && c.events.is_empty() && c.error.is_none()) {
            let chunk = parse_ok(&case.src);
            let mut plain = boot(&chunk, case);
            let direct = plain.run().map(|_| plain.output.clone());

            let chunk = parse_ok(&case.src);
            let mut vm = boot(&chunk, case);
            vm.set_preempt_interval(1);
            let preempted = loop {
                match vm.run() {
                    Ok(_) => break Ok(vm.output.clone()),
                    Err(VmErr::HostYield(SchedulerStatus::Preempted)) => continue,
                    Err(e) => break Err(e),
                }
            };
            match (direct, preempted) {
                (Ok(a), Ok(b)) if a == b => {}
                (Err(a), Err(b)) if a.to_string() == b.to_string() => {}
                (a, b) => failures.push(format!("{:?}\n   direct {:?}\n   preempted {:?}", case.src, a, b)),
            }
        }
        if !failures.is_empty() {
            let shown = failures.len().min(20);
            panic!("{} preempt divergence(s):\n{}", failures.len(), failures[..shown].join("\n"));
        }
    }

    /* GC runs on the ForIter back-edge; a preempt there must not unroot live values. */
    #[test]
    fn preempt_survives_gc() {
        let src = "acc = []\nfor i in range(3000):\n    acc.append(str(i))\n    if len(acc) > 40:\n        acc = acc[-2:]\nprint(len(acc), acc[-1])";
        let (plain, _, _) = run_preempted(src, 0, 0);
        let (hopped, count, _) = run_preempted(src, 200, 3);
        assert!(count >= 3, "for body should preempt, got {count}");
        assert_eq!(hopped, plain);
    }
}
