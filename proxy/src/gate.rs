// The developer-facing error shown when a plugin trips the guard.
pub fn block_message() -> &'static str {
    "standard packages can't make syscalls, move this to a system package"
}
