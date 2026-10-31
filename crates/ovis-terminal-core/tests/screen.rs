//! Holder output connected to revisioned screen, history, search, selection
//! and raw output reads (plan todo 17).

#[path = "../src/screen/qa/adversarial.rs"]
pub mod adversarial;
#[path = "../src/screen/qa/fixture.rs"]
pub mod fixture;
#[cfg(unix)]
#[path = "../src/screen/qa/live.rs"]
pub mod live;
#[path = "../src/screen/qa/rejections.rs"]
pub mod rejections;
#[path = "../src/screen/qa/wire.rs"]
pub mod wire;

#[cfg(unix)]
#[test]
fn happy() -> fixture::TestResult {
    live::happy()
}

#[test]
fn failure() -> fixture::TestResult {
    rejections::failure()
}
