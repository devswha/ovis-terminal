//! Geometry transitions and unchanged-grid repaint (plan todo 26) against the real authority,
//! lease table, durable coordinator, screen and, on Unix, a real holder running bash.

use std::error::Error;

use ovis_terminal_core::geometry::{GeometryEpoch, GeometryError};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

#[path = "../src/geometry/qa/adversarial.rs"]
mod adversarial;
#[path = "../src/geometry/qa/authority.rs"]
mod authority;
#[cfg(unix)]
#[path = "../src/geometry/qa/bash.rs"]
pub mod bash;
#[path = "../src/input/sequencer/qa/fixture.rs"]
pub mod fixture;
#[path = "../src/leases/qa/fixture.rs"]
pub mod lease_fixture;
#[path = "../src/geometry/qa/model.rs"]
mod model;
#[cfg(unix)]
#[path = "../src/geometry/qa/pty.rs"]
pub mod pty;
#[path = "../src/geometry/qa/rig.rs"]
pub mod rig;
#[path = "../src/geometry/qa/wire.rs"]
mod wire;

use fixture::OBSERVER;
use rig::{Rig, dispatched, holder_target, op, screen};

#[cfg(unix)]
#[test]
fn happy() -> TestResult {
    let transcript = bash::unmodified_bash_receives_input_at_the_new_grid()?;
    println!("{}", serde_json::to_string(&transcript)?);
    Ok(())
}

#[test]
fn failure() -> TestResult {
    // Given the operator owns geometry of an 80x24 screen and a phone observes.
    let mut rig = Rig::new(screen(holder_target(1), 80, 24)?)?;
    let phone = rig.fixture.client("phone", OBSERVER)?;
    let watch = rig.fixture.fresh()?;
    rig.leases.observe(phone.caller(), watch)?;

    // When the observer resizes.
    let observer = rig.leases.authorize_resize(phone.caller(), watch).err();

    // Then it is forbidden and nothing changed.
    assert_eq!(observer.map(|error| error.kind()), Some("forbidden"));
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(0));

    // When input and repaint arrive before the VT applied and the holder acknowledged 100x30.
    let dispatch = dispatched(rig.resize(&op(), (100, 30))??)?;
    let committed_change = rig.commit(dispatch)?;
    let committed = [
        rig.geometry.input_ready().err(),
        rig.geometry.repaint().err(),
    ];
    let early_ack = rig.acknowledged(GeometryEpoch::new(1)).err();
    rig.geometry.apply(&committed_change)?;
    let applied = rig.geometry.input_ready().err();

    // Then each is not_ready for epoch 1; no output silence makes them ready.
    let gates = [&committed[0], &committed[1], &early_ack, &applied]
        .map(|error| error.as_ref().map(GeometryError::kind));
    assert_eq!(gates, [Some("not_ready"); 4]);

    // When the holder acknowledges, then old and unissued epochs arrive.
    rig.acknowledged(GeometryEpoch::new(1))?;
    let reapplied = rig.geometry.apply(&committed_change).err();
    let repeated = rig.acknowledged(GeometryEpoch::new(1)).err();
    let ahead = rig.acknowledged(GeometryEpoch::new(7)).err();

    // Then old epochs are stale_geometry, unissued ones invalid, and readiness at 1 stands.
    let kinds = [reapplied, repeated, ahead].map(|error| error.map(|error| error.kind()));
    assert_eq!(
        kinds,
        [
            Some("stale_geometry"),
            Some("stale_geometry"),
            Some("invalid_params")
        ]
    );
    assert_eq!(rig.geometry.input_ready()?, GeometryEpoch::new(1));
    Ok(())
}
