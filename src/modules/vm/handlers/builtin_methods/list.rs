/*
Built-in methods for `list` receivers. Arity is checked by the dispatcher; `mutating` is marked by the dispatcher when `MethodDesc::mutating` is true.
*/

use super::prelude::*;

/* `list.__next__()`: consume the front item; StopIteration when empty (iter() yields a list). */
pub fn next_method(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let HeapObj::List(rc) = vm.heap.get(recv) else { return Err(cold_type("__next__: receiver is not a list")); };
    let rc = rc.clone();
    let mut v = rc.borrow_mut();
    if v.is_empty() { return Err(VmErr::Raised(crate::s!("StopIteration"))); }
    let item = v.remove(0);
    drop(v);
    vm.push(item);
    Ok(())
}

pub fn index(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let items = list_clone(vm, recv)?;
    let len = items.len() as i64;
    // Optional start/end clamp like CPython; negatives count from the end. Bools count as ints.
    let as_i = |v: Val| -> i64 { if v.is_bool() { v.as_bool() as i64 } else { v.as_int() } };
    let norm = |v: Val| (if as_i(v) < 0 { len + as_i(v) } else { as_i(v) }).clamp(0, len) as usize;
    let start = pos.get(1).filter(|v| v.is_int() || v.is_bool()).map_or(0, |&v| norm(v));
    let stop = pos.get(2).filter(|v| v.is_int() || v.is_bool()).map_or(items.len(), |&v| norm(v)).max(start);
    let idx = (start..stop)
        .find(|&i| eq_vals_with_heap(items[i], pos[0], &vm.heap))
        .map(|i| i as i64)
        .ok_or(cold_value("value not found in list"))?;
    vm.push(Val::int(idx));
    Ok(())
}

pub fn count(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let items = list_clone(vm, recv)?;
    let n = items.iter().filter(|&&v| eq_vals_with_heap(v, pos[0], &vm.heap)).count() as i64;
    vm.push(Val::int(n));
    Ok(())
}

pub fn copy(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let items = list_clone(vm, recv)?;
    vm.alloc_and_push_list(items)
}

pub fn append(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    list_mut(vm, recv, "append: receiver is not a list", |list| {
        list.push(pos[0]); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn clear(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    list_mut(vm, recv, "clear: receiver is not a list", |list| {
        list.clear(); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn reverse(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    list_mut(vm, recv, "reverse: receiver is not a list", |list| {
        list.reverse(); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn extend(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let items = vm.extract_iter(pos[0], true)?;
    list_mut(vm, recv, "extend: receiver is not a list", |list| {
        list.extend_from_slice(&items); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn insert(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
    list_mut(vm, recv, "insert: receiver is not a list", |list| {
        let i = pos[0].as_int();
        let ui = if i < 0 {
            (list.len() as i64).saturating_add(i).max(0) as usize
        } else {
            (i as usize).min(list.len())
        };
        list.insert(ui, pos[1]);
        Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn remove(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let items = list_clone(vm, recv)?;
    let idx = items.iter()
        .position(|&v| eq_vals_with_heap(v, pos[0], &vm.heap))
        .ok_or(cold_value("list.remove: value not found"))?;
    list_mut(vm, recv, "remove: receiver is not a list", |list| {
        list.remove(idx); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn pop(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let popped = list_mut(vm, recv, "pop: receiver is not a list", |list| {
        if list.is_empty() { return Err(cold_index("pop from empty list")); }
        if pos.is_empty() { return Ok(list.pop().unwrap()); }
        if !pos[0].is_int() { return Err(cold_type("list indices must be integers")); }
        let i = pos[0].as_int();
        let ui = if i < 0 {
            let adj = (list.len() as i64).saturating_add(i);
            if adj < 0 { return Err(cold_index("pop index out of range")); }
            adj as usize
        } else { i as usize };
        if ui >= list.len() { return Err(cold_index("pop index out of range")); }
        Ok(list.remove(ui))
    })?;
    vm.push(popped); Ok(())
}

// list.sort() is intercepted in try_dispatch_non_func_callable, which has chunk/slots for __lt__.
pub fn sort(_vm: &mut VM, _recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    Err(cold_type("list.sort dispatched without a frame"))
}
