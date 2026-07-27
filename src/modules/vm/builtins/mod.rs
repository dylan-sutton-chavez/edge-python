// Builtins split by domain. Submodules add `impl VM` methods to the same type.

use super::VM;

pub mod async_ops;
pub mod attr;
pub mod bytes_helpers;
pub mod container;
pub mod conversion;
pub mod identity;
pub mod index;
pub mod io;
pub mod numeric;
pub mod sequence;

/* Parent map for built-in exception types, walked by `matches_exc_class`. Only the standard tree is encoded, user classes stay flat. */
const EXC_PARENTS: &[(&str, &str)] = &[
    ("RuntimeError", "Exception"),
    ("ValueError", "Exception"),
    ("TypeError", "Exception"),
    ("KeyError", "LookupError"),
    ("IndexError", "LookupError"),
    ("LookupError", "Exception"),
    ("AttributeError", "Exception"),
    ("ZeroDivisionError", "ArithmeticError"),
    ("OverflowError", "ArithmeticError"),
    ("ArithmeticError", "Exception"),
    ("OSError", "Exception"),
    ("NameError", "Exception"),
    ("StopIteration", "Exception"),
    ("StopAsyncIteration", "Exception"),
    ("AssertionError", "Exception"),
    // `SystemExit` sits under `BaseException`, so `except Exception` does not swallow it.
    ("SystemExit", "BaseException"),
    ("NotImplementedError", "RuntimeError"),
    ("RecursionError", "RuntimeError"),
    ("MemoryError", "Exception"),
    ("TimeoutError", "Exception"),
    ("CancelledError", "Exception"),
    ("Exception", "BaseException"),
];

pub(in crate::modules::vm) fn matches_exc_class(actual: &str, expected: &str) -> bool {
    let mut cur = actual;
    loop {
        if cur == expected { return true; }
        match EXC_PARENTS.iter().find(|(c, _)| *c == cur) {
            Some(&(_, p)) => cur = p,
            None => return false,
        }
    }
}

impl<'a> VM<'a> {
    #[inline]
    pub(in crate::modules::vm) fn mark_impure(&mut self) {
        if let Some(top) = self.observed_impure.last_mut() {
            *top = true;
        }
    }
}
