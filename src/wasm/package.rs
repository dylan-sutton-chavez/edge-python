use alloc::string::String;
use alloc::vec::Vec;

use crate::bridge::safe_bytes;
use crate::modules::bundle::Bundle;
use crate::modules::json::quote;
use crate::modules::{parse_manifest, rules};

use super::walk::split;
use super::write_out;

/* Why a registry turns away the manifest a package carries, with the lock beside it when it has one, zero length when it holds. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn manifest_check(m_ptr: *const u8, m_len: u32, l_ptr: *const u8, l_len: u32, system_ptr: *const u8, system_len: u32) -> u32 {
    let lock = unsafe { safe_bytes(l_ptr, l_len) };
    let system = split(unsafe { safe_bytes(system_ptr, system_len) }, '\n');
    let system: Vec<&str> = system.iter().map(String::as_str).collect();
    let checked = rules::check_package(unsafe { safe_bytes(m_ptr, m_len) }, (!lock.is_empty()).then_some(lock), &system);
    write_out(&checked.err().unwrap_or_default()) as u32
}

/* The fields a registry stores from the manifest at `ptr`, read as the rules read them, as JSON, or why it is not one. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn manifest_fields(ptr: *const u8, len: u32) -> u32 {
    let mut out = String::from("{");
    match parse_manifest(unsafe { safe_bytes(ptr, len) }) {
        Err(e) => {
            out.push_str("\"error\":");
            quote(&mut out, &e);
        }
        Ok(m) => {
            for (i, (key, value)) in [("name", m.name), ("version", m.version), ("description", m.description), ("repository", m.repository), ("edge", m.edge)].into_iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                quote(&mut out, key);
                out.push(':');
                match value {
                    Some(text) => quote(&mut out, &text),
                    None => out.push_str("null"),
                }
            }
        }
    }
    out.push('}');
    write_out(&out) as u32
}

/* Where each file of the bundle at `ptr` sits inside it, as JSON, or why it is not a bundle. */
#[unsafe(no_mangle)]
pub unsafe extern "C" fn bundle_index(ptr: *const u8, len: u32) -> u32 {
    let mut out = String::new();
    match Bundle::index(unsafe { safe_bytes(ptr, len) }) {
        Err(e) => {
            out.push_str("{\"error\":");
            quote(&mut out, &e);
        }
        Ok(index) => {
            out.push_str("{\"entry\":");
            quote(&mut out, &index.entry);
            out.push_str(",\"files\":[");
            for (i, (path, at, size)) in index.files.iter().enumerate() {
                if i > 0 {
                    out.push(',');
                }
                out.push('[');
                quote(&mut out, path);
                out.push(',');
                out.push_str(itoa::Buffer::new().format(*at));
                out.push(',');
                out.push_str(itoa::Buffer::new().format(*size));
                out.push(']');
            }
            out.push(']');
        }
    }
    out.push('}');
    write_out(&out) as u32
}
