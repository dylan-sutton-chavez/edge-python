fn main() {
    // Test binaries dlopen native plugins that resolve edge_* against this binary symbols.
    match std::env::var("CARGO_CFG_TARGET_OS").as_deref() {
        Ok("linux") => println!("cargo::rustc-link-arg-tests=-rdynamic"),
        Ok("macos") => println!("cargo::rustc-link-arg-tests=-Wl,-export_dynamic"),
        _ => {}
    }
}
