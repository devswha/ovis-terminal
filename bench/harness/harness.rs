//! Reusable pseudo-terminal latency and soak harnesses.
//!
//! Both drive real terminals through the pinned portable-pty backend: each
//! terminal runs `cat` with echo off, the harness writes load lines followed by
//! a unique marker and measures until the marker comes back. The latency
//! harness keeps the local profile (16 terminals, 1 MiB/s aggregate input)
//! with its 100 ms verdict; the soak harness reports capacity separately.

mod error;
mod histogram;
mod latency;
mod memory;
mod soak;
mod terminal;

pub use error::HarnessError;
pub use histogram::{LatencyHistogram, LatencySummary};
pub use latency::{LatencyProfile, LatencyReport, run_latency};
pub use memory::ResidentMemory;
pub use soak::{SoakProfile, SoakReport, run_soak};
