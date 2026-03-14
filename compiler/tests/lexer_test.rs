#[cfg(test)]
mod lexer_test {

    /*
    Loads lexer test cases from JSON and asserts token output matches expected values.
    */

    use compiler::modules::lexer::{lexer, TokenType};

    #[test]
    fn test_cases() {
        let raw = include_str!("cases/lexer_cases.json");
        let cases: Vec<(String, Vec<String>)> = serde_json::from_str(raw).expect("invalid JSON");

        for (src, expected) in cases {
            let got: Vec<String> = lexer(&src).map(|t| format!("{:?}", t)).collect();
            assert_eq!(got, expected, "failed on: {:?}", src);
        }
    }

    #[test]
    fn test_fstring() {
        let toks: Vec<TokenType> = lexer(r#"f"hola {x}""#).collect();
        assert_eq!(toks[0], TokenType::FstringStart);
        assert!(toks.contains(&TokenType::FstringMiddle));
        assert!(toks.contains(&TokenType::FstringEnd));
    }

}