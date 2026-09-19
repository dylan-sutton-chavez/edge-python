/* Web builds fetch the official CDN, tests serve it from EDGE_CDN_BASE and never reach production. */
pub fn cdn_base() -> Result<String, String> {
    std::env::var("EDGE_CDN_BASE").map_err(|_| "set EDGE_CDN_BASE (npm run cdn:local in infra)".to_string())
}
