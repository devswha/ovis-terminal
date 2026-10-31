//! The hello boundary under every byte split, and what a refused hello leaves behind.

use std::sync::Arc;

use ovis_terminal_authz::{
    Authority, AuthorityConfig, EndpointPolicy, FileEvidence, LocalAccount, OwnerSpec,
    PeerEvidence, PrincipalId, RuntimeBinding, UnixSocketEvidence, UnixUid,
};
use ovis_terminal_core::registry::{
    HostBootId, HostIdentity, RegistryIdentities, RuntimeId, RuntimeIdentity, ServerBootId,
};
use ovis_terminal_protocol::rpc::RpcReply;
use ovis_terminal_protocol::schema::scalar::TimestampMs;
use ovis_terminal_protocol::wire::{Channel, Frame, FrameKind};
use proptest::prelude::*;

use super::{ConnectionCore, Step};
use crate::conn::ServerIdentity;
use crate::conn::hello::Shared;

const OWNER: u32 = 501;

fn core_for(peer: u32) -> (ConnectionCore, String) {
    let machine = "machine:0191f1a2-0000-7000-8000-000000000001"
        .parse()
        .unwrap();
    let host = HostIdentity::new(machine, HostBootId::try_from(1).unwrap());
    let runtime = RuntimeIdentity::new(
        host,
        RuntimeId::try_from(1).unwrap(),
        ServerBootId::try_from(1).unwrap(),
    );
    let binding = RuntimeBinding::<RegistryIdentities>::new(machine, runtime.runtime_id());
    let owner = OwnerSpec::new(
        PrincipalId::parse("owner").unwrap(),
        TimestampMs::new(u64::MAX),
    );
    let (authority, issued) = Authority::bootstrap(AuthorityConfig::new(binding, owner)).unwrap();
    let shared = Arc::new(Shared {
        authority: Arc::new(authority),
        identity: ServerIdentity::new(runtime, &["doc.get"]).unwrap(),
    });
    let owner_uid = UnixUid::new(OWNER);
    let evidence = UnixSocketEvidence::new(
        FileEvidence::new(owner_uid, 0o600),
        FileEvidence::new(owner_uid, 0o700),
        UnixUid::new(peer),
    );
    let admission = EndpointPolicy::new(LocalAccount::Unix(owner_uid))
        .admit(&PeerEvidence::UnixSocket(evidence));
    (
        ConnectionCore::new(shared, admission),
        issued.bearer().to_owned(),
    )
}

fn hello(generation: &str, bearer: &str) -> Vec<u8> {
    format!(
        r#"{{"jsonrpc":"2.0","id":"h","method":"hello","params":{{"generation":{generation},"client":{{"name":"t","version":"1"}},"framed":true,"credential":"{bearer}"}}}}"#
    )
    .into_bytes()
}

fn request(id: &str) -> String {
    format!(r#"{{"jsonrpc":"2.0","id":"{id}","method":"doc.get","params":{{}}}}"#)
}

fn frame(json: &str) -> Vec<u8> {
    let frame = Frame::new(Channel::CONTROL, FrameKind::Data, json.as_bytes().to_vec());
    frame.unwrap().encode().unwrap()
}

/// One comparable token per step: what was answered, dispatched or closed.
fn summary(steps: &[Step]) -> Vec<String> {
    steps
        .iter()
        .map(|step| match step {
            Step::Reply(RpcReply::Success(ok)) => format!("ok:{}", ok.id.as_str()),
            Step::Reply(RpcReply::Failure(failed)) => {
                format!(
                    "err:{}:{}",
                    failed.id.as_str(),
                    failed.error.data.kind.token()
                )
            }
            Step::Reply(_) => "reply:unknown".to_owned(),
            Step::Upgrade => "upgrade".to_owned(),
            Step::Dispatch { request, .. } => format!("dispatch:{}", request.id().as_str()),
            Step::Close(error) => format!("close:{}", error.kind()),
        })
        .collect()
}

fn feed_split(core: &mut ConnectionCore, bytes: &[u8], cuts: &[usize]) -> Vec<String> {
    let mut steps = Vec::new();
    let mut start = 0;
    for &cut in cuts.iter().chain(std::iter::once(&bytes.len())) {
        steps.extend(core.feed(bytes.get(start..cut).unwrap()));
        start = cut;
    }
    summary(&steps)
}

proptest! {
    #[test]
    fn framed_upgrade_parses_identically_under_every_split(
        raw_cuts in prop::collection::vec(any::<prop::sample::Index>(), 0..12),
    ) {
        let (mut core, bearer) = core_for(OWNER);
        let mut bytes = hello("\"1\"", &bearer);
        bytes.push(b'\n');
        bytes.extend(frame(&request("r1")));
        bytes.extend(frame(&request("r2")));
        let mut cuts: Vec<usize> = raw_cuts.iter().map(|cut| cut.index(bytes.len())).collect();
        cuts.sort_unstable();
        let observed = feed_split(&mut core, &bytes, &cuts);
        prop_assert_eq!(observed, ["ok:h", "upgrade", "dispatch:r1", "dispatch:r2"]);
    }
}

#[test]
fn refused_generation_does_not_upgrade_the_bytes_behind_it() {
    let (mut core, bearer) = core_for(OWNER);
    let mut bytes = hello("\"999\"", &bearer);
    bytes.push(b'\n');
    bytes.extend(request("r1").into_bytes());
    bytes.push(b'\n');
    let observed = feed_split(&mut core, &bytes, &[]);
    assert_eq!(
        observed,
        ["err:h:unsupported_generation", "err:r1:unauthenticated"]
    );
}

#[test]
fn generation_is_an_exact_decimal_string() {
    let (mut core, bearer) = core_for(OWNER);
    let cases = [
        ("\"18446744073709551615\"", "err:h:unsupported_generation"),
        ("\"18446744073709551616\"", "err:h:invalid_params"),
        ("1", "err:h:invalid_params"),
        ("\"01\"", "err:h:invalid_params"),
    ];
    for (generation, expected) in cases {
        let mut line = hello(generation, &bearer);
        line.push(b'\n');
        assert_eq!(
            feed_split(&mut core, &line, &[]),
            [expected],
            "{generation}"
        );
    }
}

#[test]
fn foreign_peer_is_answered_with_its_refusal_and_closed() {
    let (mut core, bearer) = core_for(OWNER + 1);
    let mut line = hello("\"1\"", &bearer);
    line.push(b'\n');
    line.extend(request("r1").into_bytes());
    line.push(b'\n');
    let observed = feed_split(&mut core, &line, &[]);
    assert_eq!(observed, ["err:h:peer_rejected", "close:peer_rejected"]);
}

#[test]
fn partial_hello_establishes_nothing_until_its_line_completes() {
    let (mut core, bearer) = core_for(OWNER);
    let mut line = hello("\"1\"", &bearer);
    line.push(b'\n');
    let (head, tail) = line.split_at(line.len() / 2);
    assert!(core.feed(head).is_empty());
    assert_eq!(summary(&core.feed(tail)), ["ok:h", "upgrade"]);
}
