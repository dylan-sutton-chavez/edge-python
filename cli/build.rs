fn main() {
    // dlopen'd std plugins resolve their edge_* imports against the binary's dynamic symbols.
    if std::env::var("CARGO_CFG_TARGET_OS").as_deref() == Ok("linux") {
        println!("cargo::rustc-link-arg-bin=edge=-rdynamic");
    }
}
