/*
Built-in methods for `dict` receivers. Arity is checked by the dispatcher; `mutating` is marked by the dispatcher when `MethodDesc::mutating` is true.
*/

use super::prelude::*;

pub fn keys(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let entries = dict_entries(vm, recv)?;
    let keys: Vec<Val> = entries.into_iter().map(|(k, _)| k).collect();
    vm.alloc_and_push_list(keys)
}

pub fn values(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let entries = dict_entries(vm, recv)?;
    let vals: Vec<Val> = entries.into_iter().map(|(_, v)| v).collect();
    vm.alloc_and_push_list(vals)
}

pub fn items(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let entries = dict_entries(vm, recv)?;
    let mut items: Vec<Val> = Vec::with_capacity(entries.len());
    for (k, vv) in entries {
        let t = vm.heap.alloc(HeapObj::Tuple(vec![k, vv]))?;
        items.push(t);
    }
    vm.alloc_and_push_list(items)
}

// `dict.copy()`, shallow copy; mutations don't affect the original.
pub fn copy(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let entries = dict_entries(vm, recv)?;
    let mut dm = DictMap::with_capacity(entries.len());
    for (k, v) in entries { dm.insert(k, v, &vm.heap); }
    vm.alloc_and_push_dict(dm)
}

// `dict.clear()`, remove all entries in place.
pub fn clear(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    dict_mut(vm, recv, "clear: receiver is not a dict", |dict, _heap| {
        dict.clear(); Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

// `dict.popitem()`, pop the last (k, v); KeyError on empty dict.
pub fn popitem(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let pair = dict_mut(vm, recv, "popitem: receiver is not a dict", |dict, heap| {
        let (k, v) = dict.entries.last().copied().ok_or(cold_key("popitem(): dictionary is empty"))?;
        dict.remove(&k, heap);
        Ok((k, v))
    })?;
    vm.alloc_and_push_tuple(vec![pair.0, pair.1])
}

// `dict.fromkeys(iterable, value=None)` classmethod: new dict mapping each key to `value`.
pub fn fromkeys(vm: &mut VM, _recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let keys = vm.extract_iter(pos[0], true)?;
    let value = pos.get(1).copied().unwrap_or(Val::none());
    let mut dm = DictMap::with_capacity(keys.len());
    for k in keys { dm.insert(k, value, &vm.heap); }
    vm.alloc_and_push_dict(dm)
}

pub fn get(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let default = if pos.len() == 2 { pos[1] } else { Val::none() };
    let result = match vm.heap.get(recv) {
        HeapObj::Dict(rc) => rc.borrow().get(&pos[0], &vm.heap).copied().unwrap_or(default),
        _ => return Err(cold_type("get: receiver is not a dict")),
    };
    vm.push(result); Ok(())
}

pub fn update(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    // Merge each source in order; dispatcher packs kwargs as trailing dict.
    let mut pairs: Vec<(Val, Val)> = Vec::new();
    for &src in pos {
        if let Some(HeapObj::Dict(rc)) = vm.heap.try_get(src) {
            pairs.extend(rc.borrow().entries.iter().copied());
        } else {
            for it in vm.extract_iter(src, true)? {
                let pair = match vm.heap.try_get(it) {
                    Some(HeapObj::Tuple(v)) if v.len() == 2 => (v[0], v[1]),
                    Some(HeapObj::List(v)) if v.borrow().len() == 2 => { let v = v.borrow(); (v[0], v[1]) }
                    _ => return Err(cold_value("dictionary update sequence element must have length 2")),
                };
                pairs.push(pair);
            }
        }
    }
    dict_mut(vm, recv, "update: receiver is not a dict", |dict, heap| {
        for (k, v) in pairs { dict.insert(k, v, heap); }
        Ok(())
    })?;
    vm.push(Val::none()); Ok(())
}

pub fn pop(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let default = if pos.len() == 2 { Some(pos[1]) } else { None };
    let removed = dict_mut(vm, recv, "pop: receiver is not a dict", |dict, heap| Ok(dict.remove(&pos[0], heap)))?;
    let result = match removed {
        Some(val) => val,
        None => match default {
            Some(d) => d,
            // raises KeyError whose str is the missing key's repr.
            None => return Err(VmErr::Raised(crate::s!("KeyError: ", str &vm.repr(pos[0])))),
        },
    };
    vm.push(result); Ok(())
}

pub fn setdefault(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    let default = if pos.len() > 1 { pos[1] } else { Val::none() };
    let result = dict_mut(vm, recv, "setdefault: receiver is not a dict", |dict, heap| {
        if let Some(v) = dict.get(&pos[0], heap).copied() { Ok(v) }
        else { dict.insert(pos[0], default, heap); Ok(default) }
    })?;
    vm.push(result); Ok(())
}
