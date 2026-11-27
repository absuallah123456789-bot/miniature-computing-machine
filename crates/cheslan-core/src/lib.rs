//! ChesLan core: a dependency-light, pure-Rust chess rules engine.
pub mod eco;
pub mod game;
pub mod position;
pub mod review;
pub mod san;
pub mod types;

#[cfg(test)]
mod tests;
