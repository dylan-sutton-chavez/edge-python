/*
parser.rs
  Consumes tokens, understands grammar (expressions, statements, blocks).
  Produces a minimal AST or feeds directly into compiler.rs.
*/

use log::{info};