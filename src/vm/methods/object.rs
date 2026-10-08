use super::prelude::*;

// `it.__next__()` of a builtin iterator, StopIteration once it is spent.
pub fn iter_next(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let item = vm.iter_step(recv)?.ok_or_else(|| VmErr::Raised(String::from("StopIteration")))?;
    vm.push(item); Ok(())
}

// `it.__iter__()` is the iterator itself.
pub fn iter_self(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> { vm.push(recv); Ok(()) }

// `BaseException.__init__(self, *args)` keeps the arguments as `args`.
pub fn exc_init(vm: &mut VM, recv: Val, pos: &[Val]) -> Result<(), VmErr> {
    vm.set_exc_args(recv, pos.to_vec())?;
    vm.push(Val::none()); Ok(())
}

// `BaseException.__str__(self)`, the message `args` gives.
pub fn exc_str(vm: &mut VM, recv: Val, _pos: &[Val]) -> Result<(), VmErr> {
    let text = vm.display(recv);
    vm.alloc_and_push_str(text)
}
