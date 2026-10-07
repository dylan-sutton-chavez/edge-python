pub fn cdn_base() -> Result<String, String> {
    // Web builds and remote modules fetch the official CDN, tests serve it from EDGE_CDN_BASE.
    std::env::var("EDGE_CDN_BASE").map_err(|_| "set EDGE_CDN_BASE (make serve)".to_string())
}
