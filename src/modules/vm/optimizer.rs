/*
Post-SSA passes: constant folding (binop / Not / Minus on LoadConst) and Phi-noop elimination.
LoadName preserved for IC/super-ops/templates. Dead instructions removed, jump operands remapped.
*/

use crate::modules::parser::{OpCode, SSAChunk, Instruction, Value};
use super::types::Val;
use alloc::{vec, vec::Vec};

pub fn constant_fold(chunk: &mut SSAChunk) {
    let n = chunk.instructions.len();
    if n == 0 {
        for (_, body, _, _) in chunk.functions.iter_mut() { constant_fold(body); }
        for class_body in chunk.classes.iter_mut() { constant_fold(class_body); }
        return;
    }

    let mut dead = vec![false; n];

    for ip in 0..n {
        if dead[ip] { continue; }
        let opcode = chunk.instructions[ip].opcode;

        match opcode {
            OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Div
            | OpCode::Mod | OpCode::FloorDiv
            | OpCode::Eq | OpCode::NotEq
            | OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq
            | OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor
            | OpCode::Shl | OpCode::Shr => {
                try_fold_binop(chunk, &mut dead, ip);
            }
            OpCode::Not => try_fold_unary(chunk, &mut dead, ip, fold_not),
            OpCode::Minus => try_fold_unary(chunk, &mut dead, ip, fold_neg),
            _ => {}
        }
    }

    // Pass 2: Phi-noop elimination; capture source pairs before marking dead.
    let mut surviving_pairs: Vec<(u16, u16)> = Vec::new();
    if !chunk.phi_map.is_empty() && !chunk.phi_sources.is_empty() {
        for (ip, ins) in chunk.instructions.iter().enumerate() {
            if dead[ip] || ins.opcode != OpCode::Phi { continue; }
            let phi_idx = chunk.phi_map[ip];
            let Some(&(a, b)) = chunk.phi_sources.get(phi_idx) else { continue };
            // Sources + dest collapsed to one slot, `slots[X] = slots[X]` is a no-op.
            if a == b && a == ins.operand {
                dead[ip] = true;
            } else {
                surviving_pairs.push((a, b));
            }
        }
    }

    if dead.iter().any(|&d| d) {
        compact_with_jump_remap(chunk, &dead);
        // Rebuild phi_map; surviving Phis keep their relative order, so pair them sequentially.
        if !surviving_pairs.is_empty() {
            chunk.phi_sources = surviving_pairs;
            chunk.phi_map = vec![0; chunk.instructions.len()];
            let mut idx = 0usize;
            for (i, ins) in chunk.instructions.iter().enumerate() {
                if ins.opcode == OpCode::Phi {
                    chunk.phi_map[i] = idx;
                    idx += 1;
                }
            }
        } else if !chunk.phi_map.is_empty() {
            // All Phis were eliminated; clear both metadata vectors.
            chunk.phi_sources.clear();
            chunk.phi_map.clear();
        }
    }

    for (_, body, _, _) in chunk.functions.iter_mut() {
        constant_fold(body);
    }
    for class_body in chunk.classes.iter_mut() {
        constant_fold(class_body);
    }
}

#[inline]
fn is_jump_op(op: OpCode) -> bool {
    matches!(
        op,
        OpCode::Jump
        | OpCode::JumpIfFalse
        | OpCode::JumpIfFalseOrPop
        | OpCode::JumpIfTrueOrPop
        | OpCode::ForIter
        | OpCode::SetupExcept
        | OpCode::SetupFinally
    )
}

/* Build remap[i] = new index after compaction; dead entries forward to next live, n->new_len. */
fn compact_with_jump_remap(chunk: &mut SSAChunk, dead: &[bool]) {
    let n = chunk.instructions.len();
    let alive_count: usize = dead.iter().filter(|&&d| !d).count();

    let mut remap: Vec<usize> = Vec::with_capacity(n + 1);
    let mut new_pos = 0usize;
    for &is_dead in dead {
        remap.push(new_pos);
        if !is_dead { new_pos += 1; }
    }
    remap.push(alive_count);

    // Back-to-front so each dead entry forwards to the next live successor in one pass.
    for i in (0..n).rev() {
        if dead[i] { remap[i] = remap[i + 1]; }
    }

    for (ip, _) in dead.iter().enumerate().take(n) {
        if dead[ip] { continue; }
        let ins = &mut chunk.instructions[ip];
        if !is_jump_op(ins.opcode) { continue; }
        let target = ins.operand as usize;
        let new_target = if target > n { target } else { remap[target] };
        if let Ok(v) = u16::try_from(new_target) { ins.operand = v; }
    }

    // Remap stmt_pos ips through `remap[]` to follow compaction shifts.
    for (ip_at, _) in chunk.stmt_pos.iter_mut() {
        let old = *ip_at as usize;
        if old < remap.len() { *ip_at = remap[old] as u32; }
    }

    let mut idx = 0usize;
    chunk.instructions.retain(|_| {
        let keep = !dead[idx];
        idx += 1;
        keep
    });
}

fn write_const_load(chunk: &mut SSAChunk, pos: usize, v: Val) -> bool {
    let new_ins = if v.is_bool() {
        Instruction {
            opcode: if v.as_bool() { OpCode::LoadTrue } else { OpCode::LoadFalse },
            operand: 0,
        }
    } else if v.is_none() {
        Instruction { opcode: OpCode::LoadNone, operand: 0 }
    } else if let Some(idx) = find_or_push_const(chunk, v) {
        Instruction { opcode: OpCode::LoadConst, operand: idx }
    } else {
        return false;
    };
    chunk.instructions[pos] = new_ins;
    true
}

fn find_or_push_const(chunk: &mut SSAChunk, v: Val) -> Option<u16> {
    let target: Value = if v.is_int() {
        Value::Int(v.as_int())
    } else if v.is_float() {
        Value::Float(v.as_float())
    } else {
        return None;
    };
    let pos = chunk.constants.iter().position(|c| match (c, &target) {
        // -0.0 and 0.0 compare `==` but must not share a slot, or the folded sign is lost.
        (Value::Float(a), Value::Float(b)) => a.to_bits() == b.to_bits(),
        _ => c == &target,
    });
    if let Some(pos) = pos {
        return u16::try_from(pos).ok();
    }
    let idx = chunk.constants.len();
    if idx >= u16::MAX as usize { return None; }
    chunk.constants.push(target);
    u16::try_from(idx).ok()
}

/* Widen an int/float Val to f64 (caller guarantees numeric). */
fn as_f64(v: Val) -> f64 { if v.is_int() { v.as_int() as f64 } else { v.as_float() } }

fn const_to_val(constants: &[Value], idx: u16) -> Option<Val> {
    match constants.get(idx as usize)? {
        Value::Int(i) if (Val::INT_MIN..=Val::INT_MAX).contains(i) => Some(Val::int(*i)),
        Value::Float(f) => Some(Val::float(*f)),
        Value::Bool(b) => Some(Val::bool(*b)),
        _ => None,
    }
}

/* Nearest live instruction before `from`; skips entries already marked dead by inner folds. */
fn prev_live(dead: &[bool], from: usize) -> Option<usize> {
    let mut i = from;
    while i > 0 {
        i -= 1;
        if !dead[i] { return Some(i); }
    }
    None
}

fn try_fold_binop(chunk: &mut SSAChunk, dead: &mut [bool], ip: usize) {
    let Some(prev1_ip) = prev_live(dead, ip) else { return };
    let Some(prev2_ip) = prev_live(dead, prev1_ip) else { return };

    let p2 = chunk.instructions[prev2_ip];
    let p1 = chunk.instructions[prev1_ip];
    if p2.opcode != OpCode::LoadConst || p1.opcode != OpCode::LoadConst { return; }

    let (Some(a), Some(b)) = (
        const_to_val(&chunk.constants, p2.operand),
        const_to_val(&chunk.constants, p1.operand),
    ) else { return };

    let opcode = chunk.instructions[ip].opcode;
    let Some(result) = fold_binop(opcode, a, b) else { return };

    if !write_const_load(chunk, prev2_ip, result) { return; }
    dead[prev1_ip] = true;
    dead[ip] = true;
}

/* Fold a unary op over the preceding LoadConst; `fold` supplies the per-op table. */
fn try_fold_unary(chunk: &mut SSAChunk, dead: &mut [bool], ip: usize, fold: impl Fn(Val) -> Option<Val>) {
    let Some(prev1_ip) = prev_live(dead, ip) else { return };
    let p1 = chunk.instructions[prev1_ip];
    if p1.opcode != OpCode::LoadConst { return; }
    let Some(v) = const_to_val(&chunk.constants, p1.operand) else { return };

    if let Some(r) = fold(v)
        && write_const_load(chunk, prev1_ip, r)
    {
        dead[ip] = true;
    }
}

fn fold_not(v: Val) -> Option<Val> {
    if v.is_bool() { Some(Val::bool(!v.as_bool())) }
    else if v.is_int() { Some(Val::bool(v.as_int() == 0)) }
    else if v.is_float() { Some(Val::bool(v.as_float() == 0.0)) }
    else if v.is_none() { Some(Val::bool(true)) }
    else { None }
}

fn fold_neg(v: Val) -> Option<Val> {
    if v.is_int() {
        let r = -(v.as_int() as i128);
        if (Val::INT_MIN as i128..=Val::INT_MAX as i128).contains(&r) { Some(Val::int(r as i64)) } else { None }
    } else if v.is_float() {
        Some(Val::float(-v.as_float()))
    } else { None }
}

fn fold_binop(op: OpCode, a: Val, b: Val) -> Option<Val> {
    if matches!(op, OpCode::Eq | OpCode::NotEq | OpCode::Lt | OpCode::Gt | OpCode::LtEq | OpCode::GtEq) {
        if !((a.is_int() || a.is_float()) && (b.is_int() || b.is_float())) { return None; }
        let (af, bf) = (as_f64(a), as_f64(b));
        return Some(Val::bool(match op {
            OpCode::Eq => af == bf,
            OpCode::NotEq => af != bf,
            OpCode::Lt => af < bf,
            OpCode::Gt => af > bf,
            OpCode::LtEq => af <= bf,
            OpCode::GtEq => af >= bf,
            _ => return None,
        }));
    }

    if a.is_int() && b.is_int() {
        let (ai, bi) = (a.as_int() as i128, b.as_int() as i128);
        let r = match op {
            OpCode::Add => ai.checked_add(bi)?,
            OpCode::Sub => ai.checked_sub(bi)?,
            OpCode::Mul => ai.checked_mul(bi)?,
            // floored mod/div (sign follows divisor), not Euclidean; matches the runtime path.
            OpCode::Mod => if bi == 0 { return None; } else { let r = ai % bi; if r != 0 && (r < 0) != (bi < 0) { r + bi } else { r } },
            OpCode::FloorDiv => if bi == 0 { return None; } else { let q = ai / bi; let r = ai - q * bi; if r != 0 && (r < 0) != (bi < 0) { q - 1 } else { q } },
            OpCode::BitAnd => ai & bi,
            OpCode::BitOr => ai | bi,
            OpCode::BitXor => ai ^ bi,
            OpCode::Shl => if !(0..63).contains(&bi) { return None; } else { ai.checked_shl(bi as u32)? },
            OpCode::Shr => if !(0..63).contains(&bi) { return None; } else { ai >> bi },
            _ => return None,
        };
        if (Val::INT_MIN as i128..=Val::INT_MAX as i128).contains(&r) {
            return Some(Val::int(r as i64));
        }
        return None;
    }

    if (a.is_int() || a.is_float()) && (b.is_int() || b.is_float()) {
        let (af, bf) = (as_f64(a), as_f64(b));
        return Some(Val::float(match op {
            OpCode::Add => af + bf,
            OpCode::Sub => af - bf,
            OpCode::Mul => af * bf,
            OpCode::Div => if bf == 0.0 { return None; } else { af / bf },
            _ => return None,
        }));
    }

    None
}
