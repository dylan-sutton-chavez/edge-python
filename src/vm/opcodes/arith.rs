use super::*;

use cache::OpcodeCache;
use value_ops::cached_binop;

/* IC: forward dunder name only; reflected ops are handled by the slow path's `NotImplemented` deopt. */
fn binary_dunder_name(op: OpCode) -> Option<&'static str> {
    super::dunder::binary_dunder_names(op).map(|(l, _)| l)
}

/* IC: same for comparison opcodes; reflected pairs collapse to the forward name. */
fn compare_dunder_name(op: OpCode) -> Option<&'static str> {
    super::dunder::compare_dunder_names(op).map(|(l, _, _)| l)
}

impl<'a> VM<'a> {

    /* Add/Sub/Mul/Div with IC; Mod/Pow/FloorDiv on i128 with overflow trap; Minus is unary. */
    pub(crate) fn handle_arith(&mut self, op: OpCode, rip: usize, cache: &mut OpcodeCache, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if op == OpCode::Minus {
            // -i128::MIN overflows; everything else fits.
            return self.exec_unary(rip, cache, chunk, slots, "__neg__", |v| Val::float(-v.as_float()), i128::checked_neg, "unary - requires a number");
        }
        if op == OpCode::Pos {
            // +float returns the value unchanged; bool drops its tag to int.
            return self.exec_unary(rip, cache, chunk, slots, "__pos__", |v| v, Some, "unary + requires a number");
        }

        let (a, b) = self.pop2()?;

        // `name += rhs`: a left list extends in place with any iterable (Python __iadd__); other types behave as Add.
        let op = if op == OpCode::InPlaceAdd {
            // Guard is_heap: `i += 1` operands are inline ints, and heap.get on a non-heap Val indexes garbage.
            let a_is_list = a.is_heap() && matches!(self.heap.get(a), HeapObj::List(_));
            if a_is_list {
                // Snapshot rhs first so `xs += xs` doubles correctly; TypeError if rhs isn't iterable.
                let rhs = self.iter_to_vec_general(b)?;
                if let HeapObj::List(la) = self.heap.get(a) { la.borrow_mut().extend_from_slice(&rhs); }
                self.push(a);
                return Ok(());
            }
            OpCode::Add
        } else { op };

        // Root operands: the dunder runs user code that can GC, and we read a/b after it (record + fallback).
        let roots = self.temp_roots.len();
        self.temp_roots.push(a);
        self.temp_roots.push(b);
        let dunder = self.try_binary_dunder(op, a, b, chunk, slots);
        self.temp_roots.truncate(roots);

        // instance dunder protocol, try user-defined operator before any builtin coercion.
        if let Some(r) = dunder? {
            // record the resolved class+method so the IC can fire on subsequent iterations of a hot loop.
            if let Some(name) = binary_dunder_name(op) {
                self.record_dunder_hit(rip, cache, a, name, 2);
            }
            self.push(r);
            return Ok(());
        }

        // Register-based FastOps (Add/Sub/Mul/Mod/FloorDiv) are cached; Div/Pow are not.
        if matches!(op, OpCode::Add | OpCode::Sub | OpCode::Mul | OpCode::Mod | OpCode::FloorDiv) {
            cached_binop!(self.heap, rip, &op, a, b, cache);
        }

        let result = match op {
            OpCode::Add => self.add_vals(a, b)?,
            OpCode::Sub => self.sub_vals(a, b)?,
            OpCode::Mul => self.mul_vals(a, b)?,
            OpCode::Div => self.div_vals(a, b)?,
            OpCode::Mod => self.exec_mod(a, b, chunk, slots)?,
            OpCode::Pow => self.exec_pow(a, b)?,
            OpCode::FloorDiv => self.exec_floordiv(a, b)?,
            _ => return Err(cold_runtime("non-arith opcode in handle_arith")),
        };
        self.push(result);
        Ok(())
    }

    /* Unary `-`/`+`: instance dunder takes precedence over numeric coercion; `ffl` maps the float case, `fint` the i128 case. */
    #[allow(clippy::too_many_arguments)]
    fn exec_unary(&mut self, rip: usize, cache: &mut OpcodeCache, chunk: &SSAChunk, slots: &mut [Val],
                  name: &'static str, ffl: fn(Val) -> Val, fint: fn(i128) -> Option<i128>, err: &'static str) -> Result<(), VmErr> {
        let v = self.pop()?;
        if let Some(r) = self.try_call_dunder(v, name, &[], chunk, slots)? {
            // monomorphic unary-instance sites promote like binary ops.
            self.record_dunder_hit(rip, cache, v, name, 1);
            self.push(r);
            return Ok(());
        }
        let result = if v.is_float() {
            ffl(v)
        } else if let Some(i) = self.as_i128(v) {
            self.int_to_val(fint(i))?
        } else {
            return Err(cold_type(err));
        };
        self.push(result);
        Ok(())
    }

    fn exec_mod(&mut self, a: Val, b: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Val, VmErr> {
        // `str % args` is printf-style formatting, not modulo.
        if a.is_heap() && matches!(self.heap.get(a), HeapObj::Str(_)) {
            return self.str_percent_format(a, b, chunk, slots);
        }
        if a.is_float() || b.is_float() {
            let af = self.to_f64_coerce(a).map_err(|_| cold_type("% requires numeric operands"))?;
            let bf = self.to_f64_coerce(b).map_err(|_| cold_type("% requires numeric operands"))?;
            if bf == 0.0 { return Err(VmErr::ZeroDiv); }
            // Floor-division semantics: result takes the divisor's sign.
            let r = af - ffloor(af / bf) * bf;
            return Ok(Val::float(r));
        }
        let (Some(ai), Some(bi)) = (self.as_i128(a), self.as_i128(b)) else { return Err(cold_type("% requires numeric operands")); };
        if bi == 0 { return Err(VmErr::ZeroDiv); }
        // Floor-mod on i128: result takes the divisor's sign. `checked_rem` guards against i128::MIN % -1 (which would overflow).
        let r = ai.checked_rem(bi).ok_or(cold_overflow())?;
        let r = if (r != 0) && ((r < 0) != (bi < 0)) { r + bi } else { r };
        self.int_to_val(Some(r))
    }

    /* printf-style `str % args`: translates each `%[flags][width][.prec]conv` into the `{:spec}` mini-language and reuses `format_value`. A tuple spreads; else one value. */
    fn str_percent_format(&mut self, fmt_val: Val, arg: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Val, VmErr> {
        let fmt = match self.heap.get(fmt_val) { HeapObj::Str(s) => s.clone(), _ => return Err(cold_type("% requires a string")) };
        let args: alloc::vec::Vec<Val> = match self.heap.try_get(arg) {
            Some(HeapObj::Tuple(t)) => t.clone(),
            _ => alloc::vec![arg],
        };
        let chars: alloc::vec::Vec<char> = fmt.chars().collect();
        let mut out = String::new();
        let mut ai = 0usize;
        let mut i = 0usize;
        while i < chars.len() {
            let c = chars[i];
            if c != '%' { out.push(c); i += 1; continue; }
            i += 1;
            if i < chars.len() && chars[i] == '%' { out.push('%'); i += 1; continue; }
            // flags
            let (mut left, mut zero, mut plus, mut space, mut alt) = (false, false, false, false, false);
            while i < chars.len() {
                match chars[i] {
                    '-' => left = true, '0' => zero = true, '+' => plus = true, ' ' => space = true, '#' => alt = true,
                    _ => break,
                }
                i += 1;
            }
            // width: digits, or `*` reads it (with sign) from the next argument.
            let mut width = String::new();
            if i < chars.len() && chars[i] == '*' {
                i += 1;
                let w = self.star_arg_int(&args, &mut ai)?;
                if w < 0 { left = true; } // negative `*` width left-aligns, like Python
                width = crate::s!(int w.unsigned_abs());
            } else {
                while i < chars.len() && chars[i].is_ascii_digit() { width.push(chars[i]); i += 1; }
            }
            let mut prec = String::new();
            let mut has_prec = false;
            if i < chars.len() && chars[i] == '.' {
                i += 1;
                if i < chars.len() && chars[i] == '*' {
                    i += 1;
                    let p = self.star_arg_int(&args, &mut ai)?;
                    // Negative `.*` precision is ignored, matching Python.
                    if p >= 0 { has_prec = true; prec = crate::s!(int p); }
                } else {
                    has_prec = true;
                    while i < chars.len() && chars[i].is_ascii_digit() { prec.push(chars[i]); i += 1; }
                }
            }
            if i >= chars.len() { return Err(cold_value("incomplete format")); }
            let conv = chars[i]; i += 1;
            let val = *args.get(ai).ok_or(cold_type("not enough arguments for format string"))?;
            ai += 1;
            // Map printf conversion -> (format value, spec type char, is-numeric).
            let (fval, ty, numeric): (Val, Option<char>, bool) = match conv {
                's' => { let s = self.display(val); (self.heap.alloc(HeapObj::Str(s))?, None, false) }
                'r' => { let s = self.repr(val); (self.heap.alloc(HeapObj::Str(s))?, None, false) }
                'd' | 'i' | 'u' => (self.coerce_format_int(val, chunk, slots)?, Some('d'), true),
                'x' => (self.coerce_format_int(val, chunk, slots)?, Some('x'), true),
                'X' => (self.coerce_format_int(val, chunk, slots)?, Some('X'), true),
                'o' => (self.coerce_format_int(val, chunk, slots)?, Some('o'), true),
                'c' => (val, Some('c'), false),
                'f' | 'F' => (val, Some('f'), true),
                'e' => (val, Some('e'), true),
                'E' => (val, Some('E'), true),
                'g' => (val, Some('g'), true),
                'G' => (val, Some('G'), true),
                _ => return Err(cold_value("unsupported format character")),
            };
            // Build the equivalent `{:spec}` string. printf right-aligns by default (incl. strings).
            let mut spec = String::new();
            if left { spec.push('<'); }
            else if !(zero && numeric) { spec.push('>'); }
            if plus { spec.push('+'); } else if space { spec.push(' '); }
            if alt { spec.push('#'); }
            if zero && numeric && !left { spec.push('0'); }
            spec.push_str(&width);
            if has_prec { spec.push('.'); spec.push_str(if prec.is_empty() { "0" } else { &prec }); }
            if let Some(t) = ty { spec.push(t); }
            let rendered = crate::vm::format_spec::format_value(fval, &spec, &self.heap).map_err(crate::vm::format_spec::fmt_err)?;
            out.push_str(&rendered);
        }
        // Every supplied arg must be consumed, like Python.
        if ai != args.len() {
            return Err(cold_type("not all arguments converted during string formatting"));
        }
        self.heap.alloc(HeapObj::Str(out))
    }

    /* Reads a `*` width/precision argument as an i64; non-integers raise TypeError like Python. */
    fn star_arg_int(&self, args: &[Val], ai: &mut usize) -> Result<i64, VmErr> {
        let v = *args.get(*ai).ok_or(cold_type("not enough arguments for format string"))?;
        *ai += 1;
        if v.is_bool() { return Ok(v.as_bool() as i64); }
        if v.is_int() { return Ok(v.as_int()); }
        Err(cold_type("* wants int"))
    }

    /* `%d`/`%x` on a user instance defers to its `__int__`, matching Python. */
    fn coerce_format_int(&mut self, v: Val, chunk: &SSAChunk, slots: &mut [Val]) -> Result<Val, VmErr> {
        if v.is_heap() && matches!(self.heap.get(v), HeapObj::Instance(..))
            && let Some(r) = self.try_call_dunder(v, "__int__", &[], chunk, slots)? {
            if r.is_int() || (r.is_heap() && matches!(self.heap.get(r), HeapObj::LongInt(_))) { return Ok(r); }
            return Err(cold_type("__int__ returned non-int"));
        }
        Ok(v)
    }

    fn exec_floordiv(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if a.is_float() || b.is_float() {
            let af = self.to_f64_coerce(a).map_err(|_| cold_type("// requires numeric operands"))?;
            let bf = self.to_f64_coerce(b).map_err(|_| cold_type("// requires numeric operands"))?;
            if bf == 0.0 { return Err(VmErr::ZeroDiv); }
            // ffloor() handles all magnitudes; `as i64` would overflow for large floats.
            return Ok(Val::float(ffloor(af / bf)));
        }
        let (Some(ai), Some(bi)) = (self.as_i128(a), self.as_i128(b)) else { return Err(cold_type("// requires numeric operands")); };
        if bi == 0 { return Err(VmErr::ZeroDiv); }
        // Floor-div on i128: round toward negative infinity. checked_div guards i128::MIN / -1 overflow.
        let q = ai.checked_div(bi).ok_or(cold_overflow())?;
        let r = ai - q * bi;
        let q = if (r != 0) && ((r < 0) != (bi < 0)) { q - 1 } else { q };
        self.int_to_val(Some(q))
    }

    fn exec_pow(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        self.pow_vals(a, b, "** requires numeric operands")
    }

    /* i128 bitwise + Shl/Shr (overflow trap); BitNot unary. Set/Set on |/&/^ means union/intersection/symmetric-diff; other types use the bitwise path. */
    pub(crate) fn handle_bitwise(&mut self, op: OpCode, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        // Augmented set bitwise reuses the plain path but mutates the left set in place.
        let inplace = matches!(op, OpCode::InPlaceBitOr | OpCode::InPlaceBitAnd | OpCode::InPlaceBitXor);
        let op = match op {
            OpCode::InPlaceBitOr => OpCode::BitOr,
            OpCode::InPlaceBitAnd => OpCode::BitAnd,
            OpCode::InPlaceBitXor => OpCode::BitXor,
            other => other,
        };
        if op == OpCode::BitNot {
            let v = self.pop()?;
            if let Some(r) = self.try_call_dunder(v, "__invert__", &[], chunk, slots)? {
                self.push(r);
                return Ok(());
            }
            let i = self.as_i128(v).ok_or(cold_type("~ requires an integer"))?;
            let out = self.int_to_val(Some(!i))?;
            self.push(out);
            return Ok(());
        }

        let (a, b) = self.pop2()?;

        // User instance operands dispatch __or__/__and__/__xor__ (and reflected) first.
        let roots = self.temp_roots.len();
        self.temp_roots.push(a);
        self.temp_roots.push(b);
        let dunder = self.try_binary_dunder(op, a, b, chunk, slots);
        self.temp_roots.truncate(roots);
        if let Some(r) = dunder? { self.push(r); return Ok(()); }

        if self.is_set_like(a) && self.is_set_like(b)
            && matches!(op, OpCode::BitAnd | OpCode::BitOr | OpCode::BitXor) {
            return if inplace { self.set_iop_and_push(a, b, op) } else { self.set_binop_and_push(a, b, op) };
        }
        // `dict | dict` (and `|=`) merges, right operand winning.
        if op == OpCode::BitOr && a.is_heap() && b.is_heap()
            && matches!(self.heap.get(a), HeapObj::Dict(_))
            && matches!(self.heap.get(b), HeapObj::Dict(_)) {
            let mut merged = DictMap::with_capacity(0);
            if let HeapObj::Dict(d) = self.heap.get(a) { for (k, v) in d.borrow().entries.iter() { merged.insert(*k, *v, &self.heap); } }
            if let HeapObj::Dict(d) = self.heap.get(b) { for (k, v) in d.borrow().entries.iter() { merged.insert(*k, *v, &self.heap); } }
            return self.alloc_and_push_dict(merged);
        }
        let result = match op {
            OpCode::BitAnd => self.bitwise_op(a, b, |x, y| x & y)?,
            OpCode::BitOr => self.bitwise_op(a, b, |x, y| x | y)?,
            OpCode::BitXor => self.bitwise_op(a, b, |x, y| x ^ y)?,
            OpCode::Shl => self.exec_shl(a, b)?,
            OpCode::Shr => self.exec_shr(a, b)?,
            _ => return Err(cold_runtime("non-bitwise opcode in handle_bitwise")),
        };
        self.push(result);
        Ok(())
    }

    fn exec_shl(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if !b.is_int() { return Err(cold_type("shift count must be an integer")); }
        let shift = b.as_int();
        if shift < 0 { return Err(cold_value("negative shift count")); }
        if shift >= 128 { return Err(cold_overflow()); }
        let ai = self.as_i128(a).ok_or(cold_type("<< requires an integer"))?;
        self.int_to_val(ai.checked_shl(shift as u32))
    }

    fn exec_shr(&mut self, a: Val, b: Val) -> Result<Val, VmErr> {
        if !b.is_int() { return Err(cold_type("shift count must be an integer")); }
        let shift = b.as_int();
        if shift < 0 { return Err(cold_value("negative shift count")); }
        let ai = self.as_i128(a).ok_or(cold_type(">> requires an integer"))?;
        // i128 >> is arithmetic (floor on negatives); `.min(127)` dodges shift-count UB.
        self.int_to_val(Some(ai >> shift.min(127)))
    }

    pub(crate) fn handle_compare(&mut self, op: OpCode, rip: usize, cache: &mut OpcodeCache, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let (a, b) = self.pop2()?;
        // Record type-key for every compare op; `cache::specialize` picks the FastOp variant.
        cached_binop!(self.heap, rip, &op, a, b, cache);

        // Root operands: the dunder runs user code that can GC, and we read a/b after it (record + fallback).
        let roots = self.temp_roots.len();
        self.temp_roots.push(a);
        self.temp_roots.push(b);
        let dunder = self.try_compare_dunder(op, a, b, chunk, slots);
        self.temp_roots.truncate(roots);

        // try the user-defined comparison dunder before falling back to numeric/string compare.
        if let Some(r) = dunder? {
            // monomorphic comparison sites cache the resolved method like arithmetic ones.
            if let Some(name) = compare_dunder_name(op) {
                self.record_dunder_hit(rip, cache, a, name, 2);
            }
            self.push(r);
            return Ok(());
        }

        // Set/Set uses subset/superset, NOT total order, the numeric `LtEq = !lt_vals(b, a)` identity is wrong here ({1,2} <= {2,3} would come back True), so we bypass `lt_vals`.
        if self.is_set_like(a) && self.is_set_like(b) {
            return self.set_compare_and_push(a, b, op);
        }

        let result = match op {
            OpCode::Eq => eq_vals_with_heap(a, b, &self.heap),
            OpCode::NotEq => !eq_vals_with_heap(a, b, &self.heap),
            OpCode::Lt => self.lt_vals(a, b)?,
            OpCode::Gt => self.lt_vals(b, a)?,
            OpCode::LtEq => !self.lt_vals(b, a)?,
            OpCode::GtEq => !self.lt_vals(a, b)?,
            _ => return Err(cold_runtime("non-compare opcode in handle_compare")),
        };
        self.push(Val::bool(result));
        Ok(())
    }

    // Only plain `not`; And/Or are short-circuited by the parser via Jump-If-Or-Pop.
    pub(crate) fn handle_logic(&mut self, op: OpCode, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        match op {
            OpCode::Not => {
                let v = self.pop()?;
                let t = self.truthy_op(v, chunk, slots)?;
                self.push(Val::bool(!t));
            }
            _ => return Err(cold_runtime("non-logic opcode in handle_logic")),
        }
        Ok(())
    }

    /* `is` / `is not` compare tag bits inline; `in` / `not in` delegate to contains(). */
    pub(crate) fn handle_identity(&mut self, op: OpCode, chunk: &SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        let (a, b) = self.pop2()?;
        let result = match op {
            OpCode::In => self.contains_op(b, a, chunk, slots)?,
            OpCode::NotIn => !self.contains_op(b, a, chunk, slots)?,
            OpCode::Is => a.0 == b.0,
            OpCode::IsNot => a.0 != b.0,
            _ => return Err(cold_runtime("non-identity opcode in handle_identity")),
        };
        self.push(Val::bool(result));
        Ok(())
    }
}
