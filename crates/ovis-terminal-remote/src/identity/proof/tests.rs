use ovis_terminal_authz::Attestation;
use ovis_terminal_protocol::domain::machine_identity::{BootProof, BootStatement, ProofMac};
use ovis_terminal_protocol::schema::ids::MachineId;
use ovis_terminal_protocol::schema::methods::server::HelloResult;
use serde_json::json;

use super::{Trust, fresh_challenge};
use crate::identity::{Credential, IdentityError};

const MACHINE: &str = "machine:0191f1a2-0000-7000-8000-000000000043";

type TestResult<T = ()> = Result<T, Box<dyn std::error::Error>>;

fn hello(boot: &str) -> TestResult<HelloResult> {
    Ok(serde_json::from_value(json!({
        "machine_id": MACHINE, "runtime_lifetime": "runtime:7", "server_boot_id": boot,
        "generation": "1", "version": "0.1.0", "methods": [],
        "capabilities": {"transports": [], "stream_modes": [], "framed": true},
        "limits": ovis_terminal_protocol::schema::limits::Limits::v1(),
        "server_time_ms": "0", "endpoint_kind": "stdio",
    }))?)
}

fn answered(mut result: HelloResult, statement: BootStatement, bearer: &str) -> HelloResult {
    let mac = Attestation::of_bearer(bearer, &statement.signed_bytes());
    result.boot_proof = Some(BootProof::new(
        statement,
        ProofMac::from_bytes(mac.to_bytes()),
    ));
    result
}

fn trust(machine: &str, bearer: &str) -> TestResult<Trust> {
    let credential = Credential::new(bearer.to_owned());
    Ok(Trust::required(
        Some(MachineId::new(machine.to_owned())),
        credential,
    )?)
}

fn kind<T>(result: Result<T, IdentityError>) -> &'static str {
    result.err().map_or("ok", |error| error.kind())
}

#[test]
fn a_fresh_proof_with_the_credential_for_the_expected_machine_is_trusted() -> TestResult {
    let challenge = fresh_challenge();
    let base = hello("boot:2")?;
    let statement = BootStatement::answering(challenge, &base);

    let trusted = trust(MACHINE, "ovt1.1.bearer")?
        .admit(answered(base, statement, "ovt1.1.bearer"), challenge);

    let trusted = trusted?;
    assert_eq!(trusted.machine_id().as_str(), MACHINE);
    assert_eq!(trusted.hello().server_boot_id.as_str(), "boot:2");
    Ok(())
}

#[test]
fn a_replayed_proof_of_an_earlier_challenge_is_stale() -> TestResult {
    let cached = fresh_challenge();
    let base = hello("boot:1")?;
    let replay = answered(
        base.clone(),
        BootStatement::answering(cached, &base),
        "ovt1.1.b",
    );

    let result = trust(MACHINE, "ovt1.1.b")?.admit(replay, fresh_challenge());

    assert_eq!(kind(result), "boot_proof_stale");
    Ok(())
}

#[test]
fn a_proof_not_keyed_by_the_presented_credential_is_forged() -> TestResult {
    let challenge = fresh_challenge();
    let base = hello("boot:1")?;
    let forged = answered(
        base.clone(),
        BootStatement::answering(challenge, &base),
        "ovt1.9.other",
    );

    let result = trust(MACHINE, "ovt1.1.b")?.admit(forged, challenge);

    assert_eq!(kind(result), "auth_failed");
    Ok(())
}

#[test]
fn a_proof_for_another_boot_than_the_hello_is_inconsistent() -> TestResult {
    let challenge = fresh_challenge();
    let statement = BootStatement::answering(challenge, &hello("boot:1")?);
    let restarted = answered(hello("boot:2")?, statement, "ovt1.1.b");

    let result = trust(MACHINE, "ovt1.1.b")?.admit(restarted, challenge);

    assert_eq!(kind(result), "remote_incompatible");
    Ok(())
}

#[test]
fn a_hello_without_a_proof_is_incompatible() -> TestResult {
    let result = trust(MACHINE, "b")?.admit(hello("boot:1")?, fresh_challenge());

    assert_eq!(kind(result), "remote_incompatible");
    Ok(())
}

#[test]
fn a_genuine_proof_of_another_machine_is_a_trust_mismatch() -> TestResult {
    let challenge = fresh_challenge();
    let base = hello("boot:1")?;
    let genuine = answered(
        base.clone(),
        BootStatement::answering(challenge, &base),
        "ovt1.1.b",
    );

    let result = trust("wrong-machine", "ovt1.1.b")?.admit(genuine, challenge);

    assert_eq!(kind(result), "trust_mismatch");
    Ok(())
}

#[test]
fn trust_must_be_configured_and_challenges_are_never_reused() {
    assert_eq!(
        kind(Trust::required(None, Credential::new("b".to_owned()))),
        "trust_required"
    );
    assert_ne!(fresh_challenge(), fresh_challenge());
}
