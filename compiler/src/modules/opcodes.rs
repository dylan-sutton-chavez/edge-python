/*
opcodes.rs
  One enum with every opcode: ADD_INT, LOAD, STORE, JUMP, etc.
  Shared by compiler.rs (emit) and vm.rs (execute). Nothing else.
*/

use log::{info};