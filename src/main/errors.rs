use crate::abi::ErrorKind;
use crate::modules::vm::types::VmErr;
use crate::s;
use alloc::string::String;

use super::with_runtime;

/* VmErr classifier for the ABI boundary. */
pub(super) fn err_to_kind(e: &VmErr) -> ErrorKind {
    match e {
        VmErr::Type(_) | VmErr::TypeMsg(_) => ErrorKind::Type,
        VmErr::Value(_) => ErrorKind::Value,
        VmErr::Runtime(_) => ErrorKind::Runtime,
        VmErr::Attribute(_) | VmErr::Name(_) => ErrorKind::Attribute,
        VmErr::Raised(s) => {
            if s.starts_with("ValueError") { ErrorKind::Value }
            else if s.starts_with("IndexError") { ErrorKind::Index }
            else if s.starts_with("KeyError") { ErrorKind::Key }
            else { ErrorKind::Runtime }
        }
        _ => ErrorKind::Runtime,
    }
}

pub(super) fn stash_error(e: VmErr) {
    let kind = err_to_kind(&e);
    let msg = e.render();
    with_runtime(|rt| rt.error_stash.set_typed(kind, msg));
}

/* Inverse of `err_to_kind`: rebuilds a `VmErr` from (kind, msg). Exhaustive over `ErrorKind` so new variants can't slip into `Raised`. */
pub(super) fn error_from_kind(kind: u32, msg: String) -> VmErr {
    match ErrorKind::from_u32(kind) {
        Some(ErrorKind::Type) => VmErr::TypeMsg(msg),
        Some(ErrorKind::Value) => VmErr::Raised(s!("ValueError: ", str &msg)),
        Some(ErrorKind::Runtime) => VmErr::Raised(s!("RuntimeError: ", str &msg)),
        Some(ErrorKind::Attribute) => VmErr::Attribute(msg),
        Some(ErrorKind::Index) => VmErr::Raised(s!("IndexError: ", str &msg)),
        Some(ErrorKind::Key) => VmErr::Raised(s!("KeyError: ", str &msg)),
        // Custom kinds carry the user-defined class name in `msg` (`<ClassName>: <text>`); pass through unchanged.
        Some(ErrorKind::Custom) | None => VmErr::Raised(msg),
    }
}
