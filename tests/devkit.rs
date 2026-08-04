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
