//! Documents and retention against the real coordinator, operation ledger and SQLite writer.

mod bounds;
mod cas;
mod fixture;
#[path = "../retention/qa/planning.rs"]
mod planning;
#[path = "../retention/qa/sweeps.rs"]
mod sweeps;
mod wire;
