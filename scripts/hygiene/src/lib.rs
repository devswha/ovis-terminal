//! Syntax-aware workspace hygiene checks.
//!
//! Every Rust source file in the workspace, including tests and checked-in
//! generated code, must stay within [`MAX_PURE_LOC`] pure lines of code and every
//! function signature within [`MAX_PARAMETERS`] parameters (receiver included).
//! A pure line of code is a line that carries at least one Rust token; blank
//! lines, line and block comments, and doc comments do not count, while every
//! line of a multi-line string literal does.

pub mod check;
pub mod fixtures;
pub mod loc;
pub mod params;
pub mod report;
pub mod walk;

/// Largest number of pure lines of code allowed in one source file.
pub const MAX_PURE_LOC: usize = 250;

/// Largest number of parameters allowed in one function signature.
pub const MAX_PARAMETERS: usize = 3;
