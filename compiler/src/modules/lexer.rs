/* 
lexer.rs
  Reads raw bytes, emits tokens with start/end positions.
  No strings, no copies — offsets into the original buffer only.
*/

use log::{info};