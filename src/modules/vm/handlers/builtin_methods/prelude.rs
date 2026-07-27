/*
Internal prelude for `builtin_methods`. Per-type files do `use super::prelude::*;` for VM, Val, HeapObj, helpers, receiver-unwrap primitives.
*/

pub(super) use super::super::{VM, Val, VmErr, HeapObj, DictMap};
pub(super) use super::super::methods_helpers::{
    recv_str, recv_bytes, val_to_str, extract_sequence,
    list_clone, list_mut, dict_entries, dict_mut, set_clone, set_mut,
    iter_to_vec, capitalize_first, title_case,
};
pub(super) use crate::modules::vm::types::{cold_type, cold_value, cold_key, cold_index, cold_heap, cold_overflow, eq_vals_with_heap, ValSet, HeapPool};
pub(super) use alloc::{string::{String, ToString}, vec, vec::Vec};
