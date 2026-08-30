use std::cell::RefCell;

use crate::packages::NativeBinding;
use crate::vm::types::{HeapPool, Val, VmErr};

use super::str_arg;

// A message a actor emitted, drained by the scheduler after each run step.
pub struct Outgoing {
    pub group: String,
    pub body: String,
}

thread_local! {
    static OUTBOX: RefCell<Vec<Outgoing>> = const { RefCell::new(Vec::new()) };
}

// Drains everything a actor sent during its last run step.
pub fn drain_outbox() -> Vec<Outgoing> {
    OUTBOX.with(|o| core::mem::take(&mut *o.borrow_mut()))
}

/* Message passing between actor actors, send is fire and forget over the scheduler. */
pub(super) fn bindings() -> Vec<NativeBinding> {
    vec![NativeBinding::from_fn("send", send, false)]
}

// Queues a message to a group, delivered to one of its actors round robin.
fn send(heap: &mut HeapPool, args: &[Val], _: Option<Val>) -> Result<Val, VmErr> {
    let group = str_arg(heap, args, 0, "actor.send")?;
    let body = str_arg(heap, args, 1, "actor.send")?;
    OUTBOX.with(|o| o.borrow_mut().push(Outgoing { group, body }));
    Ok(Val::none())
}
