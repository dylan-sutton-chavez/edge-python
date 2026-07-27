use super::super::VM;
use super::super::types::*;
use crate::alloc::string::ToString;

impl<'a> VM<'a> {

    pub fn call_str(&mut self, argc: u16, chunk: &crate::modules::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 { return self.alloc_and_push_str(alloc::string::String::new()); }
        if argc >= 2 {
            // `str(bytes, encoding[, errors])` decodes, mirroring `bytes.decode`.
            let args = self.pop_n(argc as usize)?;
            if args[0].is_heap() && matches!(self.heap.get(args[0]), HeapObj::Bytes(_)) {
                return crate::modules::vm::handlers::builtin_methods::bytes::decode(self, args[0], &args[1..]);
            }
            return Err(cold_type("decoding to str: need a bytes-like object"));
        }
        let o = self.pop()?;
        let s = self.display_op(o, chunk, slots)?;
        self.alloc_and_push_str(s)
    }

    pub fn call_bool(&mut self, argc: u16, chunk: &crate::modules::parser::SSAChunk, slots: &mut [Val]) -> Result<(), VmErr> {
        if argc == 0 { self.push(Val::bool(false)); return Ok(()); }
        let o = self.pop()?;
        let t = self.truthy_op(o, chunk, slots)?;
        self.push(Val::bool(t));
        Ok(())
    }

    pub fn call_type(&mut self) -> Result<(), VmErr> {
        let o = self.pop()?;
        // A user instance's type is its own class object, so `type(x) is Cls` holds.
        if o.is_heap() && let HeapObj::Instance(cls, _) = self.heap.get(o) {
            let cls = *cls;
            self.push(cls);
            return Ok(());
        }
        let name = self.type_repr_name(o);
        let t = self.heap.alloc(HeapObj::Type(name))?;
        self.push(t);
        Ok(())
    }

    /* Name `type(x)` reports: a user instance's own class, an exception's concrete class, else the builtin type name. */
    fn type_repr_name(&self, o: Val) -> alloc::string::String {
        if o.is_heap() {
            match self.heap.get(o) {
                // Exception instances report their concrete class (e.g. `ZeroDivisionError`).
                HeapObj::ExcInstance(n, _) => return n.clone(),
                HeapObj::Instance(cls, _) => {
                    let cls = *cls;
                    if cls.is_heap() && let HeapObj::Class(n, _, _) = self.heap.get(cls) {
                        return n.clone();
                    }
                }
                _ => {}
            }
        }
        self.type_name(o).to_string() // interned, so shares the `set`/`int` singleton
    }
}
