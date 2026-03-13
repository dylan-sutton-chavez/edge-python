/*
compiler.rs
  Walks the AST and emits opcodes in a single pass.
  Knows nothing about execution — only about code generation.
*/

use log::{info};