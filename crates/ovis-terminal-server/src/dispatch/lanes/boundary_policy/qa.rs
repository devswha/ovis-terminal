use std::collections::BTreeSet;
use std::time::Duration;

use ovis_terminal_authz::{
    Action, Authority, AuthorityConfig, EndpointPolicy, LocalAccount, OwnerSpec, PeerEvidence,
    Permission, Presentation, PrincipalId, RemoteTransport, Role, RuntimeBinding, UnixUid,
};
use ovis_terminal_core::ops::{OperationEvent, OperationProducer};
use ovis_terminal_core::registry::{MachineId, RegistryIdentities, RuntimeId};
use ovis_terminal_protocol::domain::boundary_policy::EndpointAllowlist;
use ovis_terminal_protocol::schema::methods::machine::MachineAddParams;
use ovis_terminal_protocol::schema::methods::terminal::TerminalRunParams;
use ovis_terminal_protocol::schema::openrpc_document;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use serde_json::{Value, json};

use super::{MUTATION_ROLES, admit_launch, required_action};
use crate::dispatch::Method;
use crate::dispatch::lanes::machine_identity::{MachineAdd, MachineLane};

type TestResult = Result<(), Box<dyn std::error::Error>>;

fn field<'a>(value: &'a Value, pointer: &str) -> &'a Value {
    value.pointer(pointer).unwrap_or(&Value::Null)
}

fn published_mutations() -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let document = openrpc_document()?;
    let methods = field(&document, "/methods").as_array().ok_or("methods")?;
    Ok(methods
        .iter()
        .filter(|method| field(method, "/x-effect") == "mutation")
        .filter_map(|method| field(method, "/name").as_str().map(str::to_owned))
        .collect())
}

#[test]
fn the_matrix_names_exactly_the_published_mutations() -> TestResult {
    let published = published_mutations()?;
    let roles: BTreeSet<String> = MUTATION_ROLES
        .iter()
        .map(|(name, _)| (*name).to_owned())
        .collect();

    assert_eq!(roles, published);
    assert!(published.len() > 40, "{}", published.len());
    Ok(())
}

/// The observer requests of the generated `security/validation` failure case.
fn scenario_observer_methods() -> Result<BTreeSet<String>, Box<dyn std::error::Error>> {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../conformance/scenarios/security/validation.json");
    let scenario: Value = serde_json::from_slice(&std::fs::read(path)?)?;
    let cases = field(&scenario, "/cases").as_array().ok_or("cases")?;
    let failure = cases
        .iter()
        .find(|case| field(case, "/case") == "failure")
        .ok_or("failure case")?;
    let steps = field(failure, "/steps").as_array().ok_or("steps")?;
    Ok(steps
        .iter()
        .filter(|step| field(step, "/send/conn") == "observer")
        .flat_map(|step| {
            field(step, "/send/requests")
                .as_array()
                .cloned()
                .unwrap_or_default()
        })
        .filter_map(|request| field(&request, "/method").as_str().map(str::to_owned))
        .collect())
}

#[test]
fn the_conformance_matrix_sends_every_published_mutation_as_the_observer() -> TestResult {
    let mut expected = published_mutations()?;
    expected.remove("attach.open");

    assert_eq!(scenario_observer_methods()?, expected);
    Ok(())
}

#[test]
fn the_matrix_is_sorted_so_every_entry_is_found() {
    let found = MUTATION_ROLES
        .iter()
        .filter(|(name, action)| required_action(name) == Some(*action))
        .count();

    assert_eq!(found, MUTATION_ROLES.len());
    assert_eq!(required_action("terminal.list"), None);
    assert_eq!(required_action("events.subscribe"), None);
}

#[test]
fn an_observer_holds_the_permission_of_no_mutation_but_an_observing_attach() {
    let observer = Role::Observer.permissions();

    let allowed: Vec<&str> = MUTATION_ROLES
        .iter()
        .filter(|(_, action)| observer.contains(&action.required()))
        .map(|(name, _)| *name)
        .collect();

    assert_eq!(allowed, ["attach.open"]);
    assert_eq!(required_action("attach.open"), Some(Action::Subscribe));
}

#[test]
fn guarded_controls_need_their_own_grant_not_ordinary_input() {
    let required = [
        "attach.takeover",
        "prompt.answer",
        "input.raw",
        "plugin.invoke",
    ]
    .map(|method| required_action(method).map(Action::required));

    assert_eq!(
        required,
        [
            Some(Permission::Takeover),
            Some(Permission::PromptAnswer),
            Some(Permission::RawInput),
            Some(Permission::Plugin)
        ]
    );
}

fn run(program: &Value, env: &Value) -> Result<TerminalRunParams, serde_json::Error> {
    serde_json::from_value(json!({
        "op_id": "o1", "geometry": {"cols": 80, "rows": 24}, "program": program, "env": env,
    }))
}

fn admitted(program: &Value, env: &Value) -> Result<&'static str, Box<dyn std::error::Error>> {
    Ok(admit_launch(&run(program, env)?).map_or_else(|error| error.kind(), |()| "admitted"))
}

#[test]
fn a_launch_keeps_shell_metacharacters_literal_and_refuses_reserved_or_oversized_input()
-> TestResult {
    let literal = json!({"argv": ["/bin/echo", "$(id);rm -rf /|cat"]});
    let largest = json!({"argv": ["/bin/echo", "x".repeat(32 * 1024)]});
    let huge = json!({"argv": ["/bin/echo", "x".repeat(32 * 1024 + 1)]});

    let outcomes = [
        admitted(&literal, &json!({"ROLE": "build"}))?,
        admitted(&literal, &json!({"OVIS_TERMINAL_PROCESS_TOKEN": "spoof"}))?,
        admitted(&largest, &Value::Null)?,
        admitted(&huge, &Value::Null)?,
        admitted(&json!({"argv": []}), &Value::Null)?,
        admitted(&json!({"template": {"name": "build"}}), &Value::Null)?,
    ];

    assert_eq!(
        outcomes,
        [
            "admitted",
            "forbidden",
            "admitted",
            "too_large",
            "invalid_params",
            "admitted"
        ]
    );
    Ok(())
}

#[test]
fn a_launch_refusal_names_its_parameter_and_bound_on_the_wire() -> TestResult {
    let huge = json!({"argv": ["/bin/echo", "x".repeat(32 * 1024 + 1)]});
    let spoof = json!({"OVIS_TERMINAL_PROCESS_TOKEN": "spoof-58"});

    let wire = |params: TerminalRunParams| {
        admit_launch(&params)
            .err()
            .map(|error| serde_json::to_value(error.boundary().data()))
    };
    let oversized = wire(run(&huge, &Value::Null)?)
        .transpose()?
        .unwrap_or_default();
    let reserved = wire(run(&json!({"argv": ["/bin/echo"]}), &spoof)?)
        .transpose()?
        .unwrap_or_default();

    assert_eq!(field(&oversized, "/kind"), "too_large");
    assert_eq!(field(&oversized, "/field_path"), "params.program.argv");
    assert_eq!(
        field(&oversized, "/bound"),
        &json!({"limit": "argv_element_bytes", "value": "32768"})
    );
    assert_eq!(field(&reserved, "/kind"), "forbidden");
    assert_eq!(field(&reserved, "/field_path"), "params.env");
    assert!(!reserved.to_string().contains("spoof-58"));
    Ok(())
}

fn add(transport: &Value) -> Result<MachineAddParams, serde_json::Error> {
    serde_json::from_value(json!({
        "op_id": "o1", "name": "build", "transport": transport, "credential": "c",
    }))
}

#[test]
fn machine_add_admits_only_allowlisted_endpoints_before_any_connection() -> TestResult {
    let machine: MachineId = "machine:0191f1a2-0000-7000-8000-000000000058".parse()?;
    let binding: RuntimeBinding<RegistryIdentities> =
        RuntimeBinding::new(machine, RuntimeId::try_from(58_u64)?);
    let owner = OwnerSpec::new(PrincipalId::parse("owner")?, TimestampMs::new(u64::MAX));
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner))?;
    let admission = EndpointPolicy::new(LocalAccount::Unix(UnixUid::new(1000)))
        .admit(&PeerEvidence::Remote(RemoteTransport::Ssh))?;
    let session = authority.authenticate(
        &admission,
        Presentation::new(issued.bearer(), TimestampMs::new(1)),
    )?;
    let lane = MachineLane::new(machine, Duration::from_secs(1));
    let allow = EndpointAllowlist::hosts(["build.example".to_owned()]);
    let restricted = MachineAdd::restricted(&lane, allow);
    let open = MachineAdd::new(&lane);

    let outcomes = [
        (&restricted, json!({"ssh": "dev@build.example"})),
        (&restricted, json!({"ssh": "dev@evil.example"})),
        (
            &restricted,
            json!({"argv": ["/usr/bin/nc", "build.example", "22"]}),
        ),
        (&open, json!({"ssh": "dev@evil.example"})),
    ]
    .map(|(method, transport)| {
        add(&transport).map(|params| {
            <MachineAdd as Method<OperationEvent, OperationProducer>>::resolve(
                method, &params, &session,
            )
            .map_or_else(|error| error.kind(), |_| "admitted")
        })
    });

    assert_eq!(
        outcomes.map(Result::ok),
        [
            Some("admitted"),
            Some("forbidden"),
            Some("forbidden"),
            Some("admitted")
        ]
    );
    Ok(())
}
