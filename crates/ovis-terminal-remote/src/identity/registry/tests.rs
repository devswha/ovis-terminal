use ovis_terminal_authz::Attestation;
use ovis_terminal_protocol::domain::machine_identity::{BootProof, BootStatement, ProofMac};
use ovis_terminal_protocol::schema::common::MachineRef;
use ovis_terminal_protocol::schema::ids::{MachineId, MachineName};
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use serde_json::json;

use super::{Connected, Machines};
use crate::identity::{Credential, Name, Trust, Trusted, fresh_challenge};

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

const BEARER: &str = "ovt1.1.bearer";

fn trusted(machine: &str) -> TestResult<Trusted> {
    let mut hello: HelloResult = serde_json::from_value(json!({
        "machine_id": machine, "runtime_lifetime": "runtime:1", "server_boot_id": "boot:1",
        "generation": "1", "version": "0.1.0", "methods": [],
        "capabilities": {"transports": [], "stream_modes": [], "framed": true},
        "limits": ovis_terminal_protocol::schema::limits::Limits::v1(),
        "server_time_ms": "0", "endpoint_kind": "stdio",
    }))?;
    let challenge = fresh_challenge();
    let statement = BootStatement::answering(challenge, &hello);
    let mac = Attestation::of_bearer(BEARER, &statement.signed_bytes());
    hello.boot_proof = Some(BootProof::new(
        statement,
        ProofMac::from_bytes(mac.to_bytes()),
    ));
    let trust = Trust::required(
        Some(MachineId::new(machine.to_owned())),
        Credential::new(BEARER.to_owned()),
    )?;
    Ok(trust.admit(hello, challenge)?)
}

fn name(text: &str) -> TestResult<Name> {
    Ok(Name::try_from(&MachineName::new(text.to_owned()))?)
}

fn by_name(text: &str) -> MachineRef {
    MachineRef::Name(MachineName::new(text.to_owned()))
}

fn add(machines: &mut Machines<&'static str>, machine: &str, text: &str) -> TestResult {
    let hold = machines.hold(name(text)?)?;
    machines.insert(hold, Connected::new(trusted(machine)?, "link"))?;
    Ok(())
}

const A: &str = "machine:0191f1a2-0000-7000-8000-00000000000a";
const B: &str = "machine:0191f1a2-0000-7000-8000-00000000000b";

#[test]
fn a_binding_resolved_before_a_rename_keeps_its_destination() -> TestResult {
    let mut machines = Machines::default();
    add(&mut machines, A, "box")?;
    let bound = machines.bind(&by_name("box")).expect("bound");

    let renamed = machines.rename(&bound, name("box2")?).expect("renamed");

    assert_eq!(bound.machine_id().as_str(), A);
    assert_eq!(renamed.machine_id.as_str(), A);
    assert_eq!(renamed.name.as_str(), "box2");
    assert_eq!(renamed.aliases, vec![MachineName::new("box".to_owned())]);
    assert_eq!(renamed.revision.get(), 2);
    assert_eq!(machines.bind(&by_name("box")).expect("old name"), bound);
    assert_eq!(machines.bind(&by_name("box2")).expect("new name"), bound);
    assert_eq!(machines.link(&bound).copied().expect("link"), "link");
    Ok(())
}

#[test]
fn an_old_name_cannot_be_taken_to_redirect_queued_work() -> TestResult {
    let mut machines = Machines::default();
    add(&mut machines, A, "box")?;
    let bound = machines.bind(&by_name("box")).expect("bound");
    machines.rename(&bound, name("box2")?).expect("renamed");

    let hijack = machines
        .hold(name("box")?)
        .map(|_| ())
        .map_err(|error| error.kind());

    assert_eq!(hijack, Err("duplicate_name"));
    assert_eq!(machines.bind(&by_name("box")).expect("alias"), bound);
    Ok(())
}

#[test]
fn renaming_onto_another_machines_name_or_a_held_name_is_refused() -> TestResult {
    let mut machines = Machines::default();
    add(&mut machines, A, "a")?;
    add(&mut machines, B, "b")?;
    let held = machines.hold(name("pending")?).expect("held");
    let bound = machines.bind(&by_name("a")).expect("bound");

    let onto_other = machines
        .rename(&bound, name("b")?)
        .map_err(|error| error.kind());
    let onto_held = machines
        .rename(&bound, name("pending")?)
        .map_err(|error| error.kind());

    assert_eq!(onto_other.err(), Some("duplicate_name"));
    assert_eq!(onto_held.err(), Some("duplicate_name"));
    machines.release(held);
    assert!(machines.hold(name("pending")?).is_ok());
    Ok(())
}

#[test]
fn a_machine_proven_twice_is_refused_and_its_name_is_released() -> TestResult {
    let mut machines = Machines::default();
    add(&mut machines, A, "a")?;
    let hold = machines.hold(name("again")?).expect("held");

    let again = machines
        .insert(hold, Connected::new(trusted(A)?, "link"))
        .map_err(|error| error.kind());

    assert_eq!(again.err(), Some("machine_exists"));
    assert!(machines.hold(name("again")?).is_ok());
    Ok(())
}

#[test]
fn unknown_and_invalid_references_resolve_to_nothing() -> TestResult {
    let mut machines = Machines::default();
    add(&mut machines, A, "a")?;
    let unknown_id = MachineRef::MachineId(MachineId::new(B.to_owned()));

    for reference in [by_name("ghost"), by_name("A;rm"), unknown_id] {
        assert_eq!(
            machines.bind(&reference).map_err(|error| error.kind()),
            Err("not_found")
        );
    }
    let by_id = machines.bind(&MachineRef::MachineId(MachineId::new(A.to_owned())));
    assert_eq!(by_id.expect("bound").machine_id().as_str(), A);
    Ok(())
}
