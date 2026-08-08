/*
One file per `edge` subcommand. Each exposes the entry point `main.rs` dispatches to.
*/

pub mod build;
pub mod init;
pub mod pkg;
pub mod repl;
pub mod serve;
pub mod swarm;
pub mod test;
pub mod uninstall;
