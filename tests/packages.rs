/*
JSON-driven runner for `packages` (imports, resolver, CallExtern, code-module inlining).
Case schema in `cases/packages.json`: src, output, error, input, modules, manifests, optional expect_externs/expect_functions/error_span_covers.
*/

#[cfg(test)]
mod test {
    use std::collections::HashMap;

    use compiler::modules::lexer::lex;
    use compiler::modules::parser::Parser;
    use compiler::modules::vm::VM;
    use compiler::modules::vm::types::{SchedulerStatus, VmErr};
    use compiler::modules::packages::NativeBinding;

    use crate::common::{TestResolver, test_native};

    /* `native`/`code` drive resolve(); optional `bytes` drives fetch_bytes() for `#sha256-` cases. */
    #[derive(serde::Deserialize)]
    #[serde(untagged)]
    enum ModuleDef {
        Native {
            native: Vec<String>,
            #[serde(default)]
            bytes: Option<String>,
        },
        Code {
            code: String,
            #[serde(default)]
            bytes: Option<String>,
        },
    }

    /* Per-directory `packages.json` for walk-up cases; flat fixtures use `aliases` instead. */
    #[derive(serde::Deserialize)]
    struct ManifestDef {
        #[serde(default)]
        imports: HashMap<String, String>,
        #[serde(default)]
        extends: Option<String>,
    }

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
        modules: HashMap<String, ModuleDef>,
        /* Synthetic root `packages.json`; nested entries in `manifests` shadow this. */
        #[serde(default)]
        aliases: HashMap<String, String>,
        /* Nested manifests by directory; exercises walk-up, `extends`, and circular-extends paths. */
        #[serde(default)]
        manifests: HashMap<String, ManifestDef>,
        #[serde(default)]
        expect_externs: Option<usize>,
        #[serde(default)]
        expect_functions: Option<usize>,
        #[serde(default)]
        error_span_covers: Option<String>,
        /* String values injected one-at-a-time after each PendingHostCall yield, simulating the JS bridge's `set_host_result`. */
        #[serde(default)]
        host_results: Vec<String>,
        /* Per-call deliveries (by call_id) simulating out-of-order host resolution: `value` -> set_host_result_by_id, `error` -> set_host_error_by_id. */
        #[serde(default)]
        host_deliveries: Vec<Delivery>,
    }

    #[derive(serde::Deserialize)]
    struct Delivery {
        id: u32,
        #[serde(default)]
        value: Option<String>,
        #[serde(default)]
        error: Option<String>,
    }

    fn build_resolver(modules: &HashMap<String, ModuleDef>, aliases: &HashMap<String, String>, manifests: &HashMap<String, ManifestDef>) -> TestResolver {
        let mut r = TestResolver::new();
        for (spec, def) in modules {
            match def {
                ModuleDef::Native { native, bytes } => {
                    let bindings: Vec<NativeBinding> = native.iter()
                        .map(|n| test_native(n)
                        .unwrap_or_else(|| panic!("unknown test native: {}", n)))
                        .collect();
                    r = r.with_native(spec, bindings);
                    if let Some(b) = bytes { r = r.with_bytes(spec, b.clone().into_bytes()); }
                }
                ModuleDef::Code { code, bytes } => {
                    r = r.with_code(spec, code);
                    if let Some(b) = bytes { r = r.with_bytes(spec, b.clone().into_bytes()); }
                }
            }
            /* Auto self-alias bare-name modules so flat fixtures work without explicit `aliases`. */
            if !spec.contains('/') {
                r = r.with_alias(spec, spec);
            }
        }
        for (name, target) in aliases {
            r = r.with_alias(name, target);
        }
        for (dir, m) in manifests {
            let pairs: Vec<(&str, &str)> = m.imports.iter().map(|(k, v)| (k.as_str(), v.as_str())).collect();
            r = r.with_manifest(dir, &pairs, m.extends.as_deref());
        }
        r
    }

    /* NoopResolver path: imports must produce a clean diagnostic, not panic. Lives in Rust since the JSON runner always builds a TestResolver. */
    #[test]
    fn noop_resolver_default() {
        let src = "from json import dumps";
        let (tokens, _) = lex(src);
        let (_, errs) = Parser::new(src, tokens.into_iter()).parse();
        assert!(errs.iter().any(|d| d.msg.contains("not found") || d.msg.contains("not configured")), "expected NoopResolver error, got: {:?}", errs.iter().map(|e| &e.msg).collect::<Vec<_>>());
    }

    #[test]
    fn packages_cases() {
        let cases: Vec<Case> = serde_json::from_str(
            include_str!("cases/packages.json")
        ).expect("invalid JSON");

        for case in cases {
            let resolver = Box::new(build_resolver(&case.modules, &case.aliases, &case.manifests));
            let (tokens, _lex_errs) = lex(&case.src);
            let parser = Parser::with_resolver(&case.src, tokens.into_iter(), resolver);
            let (chunk, parse_errs) = parser.parse();

            // Parse-time errors: match `error` substring plus optional span anchor.
            if !parse_errs.is_empty() {
                let combined = parse_errs.iter().map(|d| d.msg.as_str()).collect::<Vec<_>>().join(" | ");
                let expected = case.error.as_deref().unwrap_or_else(|| panic!("unexpected parse error on {:?}: {}", case.src, combined));
                assert!(combined.contains(expected), "parse error mismatch on {:?}: got '{}', expected substring '{}'", case.src, combined, expected);
                if let Some(needle) = &case.error_span_covers {
                    let pos = case.src.find(needle).unwrap_or_else(|| panic!("error_span_covers '{}' not found in src", needle));
                    let d = &parse_errs[0];
                    assert_eq!(d.start, pos, "diag start mismatch on {:?}", case.src);
                    assert_eq!(d.end, pos + needle.len(), "diag end mismatch on {:?}", case.src);
                }
                continue;
            }

            // Compile-time chunk-shape assertions (no parse errors).
            if let Some(n) = case.expect_externs { assert_eq!(chunk.extern_table.len(), n, "extern_table size mismatch on {:?}", case.src); }
            if let Some(n) = case.expect_functions { assert_eq!(chunk.functions.len(), n, "functions count mismatch on {:?}", case.src); }

            let mut vm = VM::new(&chunk);
            vm.input_buffer = case.input.clone();
            // Drive loop: resume on PendingHostCall by injecting the next host_results entry as a string Val.
            let mut hr_idx = 0usize;
            let result = loop {
                match vm.run() {
                    Ok(v) => break Ok(v),
                    Err(VmErr::HostYield(SchedulerStatus::PendingHostCall)) if hr_idx < case.host_deliveries.len() => {
                        let d = &case.host_deliveries[hr_idx];
                        match &d.error {
                            Some(msg) => { vm.push_host_error_by_id(d.id.into(), msg); }
                            None => { vm.push_host_result_by_id(d.id.into(), d.value.as_deref().unwrap_or("")).expect("push_host_result_by_id"); }
                        }
                        hr_idx += 1;
                    }
                    Err(VmErr::HostYield(SchedulerStatus::PendingHostCall)) if hr_idx < case.host_results.len() => {
                        vm.push_host_result(&case.host_results[hr_idx]).expect("push_host_result");
                        hr_idx += 1;
                    }
                    Err(e) => break Err(e),
                }
            };
            match result {
                Ok(_) => {
                    if case.error.is_some() { panic!("expected error on {:?}, got success with output {:?}", case.src, vm.output); }
                    assert_eq!(vm.output, case.output, "output mismatch on: {:?}", case.src);
                }
                Err(e) => match &case.error {
                    Some(expected) => assert!(e.to_string().contains(expected.as_str()), "runtime error mismatch on {:?}: got '{}', expected substring '{}'", case.src, e, expected),
                    None => panic!("VM error on {:?}: {}", case.src, e),
                }
            }
        }
    }
}
