#[cfg(test)]
mod test {

    use compiler::modules::lexer::lex;

    #[derive(serde::Deserialize)]
    struct Case {
        src: String,
        tokens: Vec<String>,
    }

    #[test]
    fn test_cases() {
        let cases: Vec<Case> = serde_json::from_str(include_str!("cases/lexer.json")).expect("invalid JSON");

        for case in cases {
            // Debug-format tokens for snapshot comparison (test-only).
            let (toks, _) = lex(&case.src);
            let got: Vec<String> = toks.iter().map(|t| format!("{:?}", t.kind)).collect();
            assert_eq!(got, case.tokens, "failed on: {:?}", case.src);
        }
    }
}
