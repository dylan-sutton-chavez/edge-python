use compiler::devkit::{escape, parse_object, JsonVal};

#[test]
fn parse_object_keeps_unicode_and_escapes() {
    // Raw multibyte UTF-8, a basic \u escape, and a surrogate pair, all in one value.
    let src = "{\"body\": \"a\u{f1}o \\u00e9 \\ud83d\\ude00\", \"headers\": {\"se\u{f1}al\": \"\u{2192}\"}}";
    let pairs = parse_object(src).unwrap();
    match &pairs[0].1 {
        JsonVal::Str(s) => assert_eq!(s, "a\u{f1}o \u{e9} \u{1f600}"),
        _ => panic!("body should be a string"),
    }
    match &pairs[1].1 {
        JsonVal::Map(m) => assert_eq!(m[0], ("se\u{f1}al".to_string(), "\u{2192}".to_string())),
        _ => panic!("headers should be a map"),
    }
}

#[test]
fn escape_round_trips_through_parse() {
    let raw = "line\n\u{f1}\t\"q\" \u{1}";
    let json = format!("{{\"v\": \"{}\"}}", escape(raw));
    match &parse_object(&json).unwrap()[0].1 {
        JsonVal::Str(s) => assert_eq!(s, raw),
        _ => panic!("expected string"),
    }
}

fn parsed_str(src: &str) -> String {
    match &parse_object(src).unwrap()[0].1 {
        JsonVal::Str(s) => s.clone(),
        _ => panic!("expected string"),
    }
}

#[test]
fn parse_object_decodes_backspace() {
    assert_eq!(parsed_str("{\"v\": \"a\\bb\"}"), "a\u{8}b");
}

#[test]
fn parse_object_decodes_form_feed() {
    assert_eq!(parsed_str("{\"v\": \"a\\fb\"}"), "a\u{c}b");
}

#[test]
fn parse_object_decodes_mixed_escapes() {
    assert_eq!(parsed_str("{\"v\": \"\\b\\f\\n\\t\\r\"}"), "\u{8}\u{c}\n\t\r");
}

#[test]
fn parse_object_decodes_escapes_in_nested_map() {
    let src = "{\"headers\": {\"x\": \"a\\fb\\bc\"}}";
    match &parse_object(src).unwrap()[0].1 {
        JsonVal::Map(m) => assert_eq!(m[0], ("x".to_string(), "a\u{c}b\u{8}c".to_string())),
        _ => panic!("expected map"),
    }
}

#[test]
fn parse_object_still_rejects_unknown_escapes() {
    assert_eq!(parse_object("{\"v\": \"a\\xb\"}").err().unwrap(), "unsupported escape");
}
