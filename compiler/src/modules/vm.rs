/*
vm.rs
  Executes bytecode instruction by instruction.
  Owns the call stack, local variables, inline cache, and quickening.
  The only file that runs at runtime.
*/

use log::{info};