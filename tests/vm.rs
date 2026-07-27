#[cfg(test)]
mod test {

    use compiler::modules::lexer::lex;
    use compiler::modules::parser::Parser;
    use compiler::modules::vm::VM;
    use compiler::modules::vm::types::Limits;
    use compiler::modules::vm::types::{SchedulerStatus, VmErr};

    #[derive(serde::Deserialize)]
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
    }

    /* Sets iterate in hash order, so canonicalize a set/frozenset line by sorting
       its elements. Assumes scalar elements with no nested ", ". Non-sets pass through. */
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

    /* Runs every vm.json case under `Limits::sandbox()` rather than `none()`: the budget / heap / call-depth guards are off under `none` (`sandbox_off` short-circuits them), so only the bounded profile exercises the charge_step / charge_steps / back-edge-budget paths and lets cases assert that runaway allocation, recursion, and materialisation surface as `MemoryError` / `RecursionError` instead of hanging. Every case must therefore stay within the sandbox budget. */
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
            compiler::modules::vm::optimizer::constant_fold(&mut chunk);
            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            vm.input_buffer = case.input.clone();
            for evt in &case.events { vm.push_event(evt).expect("push_event"); }
            let result = drive(&mut vm, &case.interactive_events);

            // Collect every mismatch (not panic-on-first) so one run lists all regressions.
            match result {
                Ok(_obj) => {
                    if normalize(&vm.output) != normalize(&case.output) {
                        failures.push(format!("OUTPUT {:?}\n   got {:?}\n   want {:?}", case.src, vm.output, case.output));
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

    /* Reruns every vm.json case in strict_input mode (host-supplied buffer; reading past = RuntimeError) under `Limits::sandbox()` (see `test_cases` for why the bounded profile). Lex/parse errors are also asserted here. */
    #[test]
    fn strict_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/vm.json")).expect("invalid JSON");

        for case in cases {
            let (tokens, lex_errs) = lex(&case.src);
            // Lex errors are surfaced as diagnostics; match against the expected error and move on.
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
            // Match production: fold before running.
            compiler::modules::vm::optimizer::constant_fold(&mut chunk);

            let mut vm = VM::with_limits(&chunk, Limits::sandbox());
            vm.strict_input = true;
            vm.input_buffer = case.input.clone();
            for evt in &case.events { vm.push_event(evt).expect("push_event"); }
            let expects_input_error = case.input.is_empty() && (case.src.contains("input(") || case.src.contains("input ("));

            match drive(&mut vm, &case.interactive_events) {
                Ok(_) => {
                    assert!(!expects_input_error, "expected input() to error under strict mode for: {:?}", case.src);
                    assert_eq!(normalize(&vm.output), normalize(&case.output), "output mismatch on: {:?}", case.src);
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
}
