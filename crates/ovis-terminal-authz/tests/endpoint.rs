//! Local endpoint admission: socket and named-pipe permissions, peer identity and the
//! first-instance pipe lifecycle. Admission proves who connected; it grants nothing.

use std::error::Error;

use ovis_terminal_authz::{
    Ace, AceKind, AuthzError, Dacl, EndpointPolicy, FileEvidence, LocalAccount, PeerEvidence,
    PipeInstance, PipeInstanceEvidence, PipeNamespace, PipeSecurity, RemoteClients,
    UnixSocketEvidence, UnixUid, WindowsSid,
};

type TestResult<T = ()> = Result<T, Box<dyn Error>>;

const OWNER: UnixUid = UnixUid::new(1000);
const OWNER_SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1001";
const OTHER_SID: &str = "S-1-5-21-1004336348-1177238915-682003330-1002";
const EVERYONE_SID: &str = "S-1-1-0";

fn kind(result: Result<impl Sized, AuthzError>) -> Option<&'static str> {
    result.err().map(|error| error.kind())
}

const fn unix_policy() -> EndpointPolicy {
    EndpointPolicy::new(LocalAccount::Unix(OWNER))
}

const fn socket(socket: FileEvidence, directory: FileEvidence, peer: UnixUid) -> PeerEvidence {
    PeerEvidence::UnixSocket(UnixSocketEvidence::new(socket, directory, peer))
}

fn sid(text: &str) -> TestResult<WindowsSid> {
    Ok(WindowsSid::parse(text)?)
}

fn owner_only_dacl() -> TestResult<Dacl> {
    Ok(Dacl::Entries(vec![Ace::new(
        AceKind::Allow,
        sid(OWNER_SID)?,
    )]))
}

fn instance(
    role: PipeInstance,
    remote: RemoteClients,
    dacl: Dacl,
) -> TestResult<PipeInstanceEvidence> {
    Ok(PipeInstanceEvidence::new(
        role,
        remote,
        PipeSecurity::new(sid(OWNER_SID)?, dacl),
    ))
}

#[test]
fn private_socket_admits_only_the_owner_peer() {
    let policy = unix_policy();
    let private = (
        FileEvidence::new(OWNER, 0o600),
        FileEvidence::new(OWNER, 0o700),
    );

    let owner = policy.admit(&socket(private.0, private.1, OWNER));
    let stranger = policy.admit(&socket(private.0, private.1, UnixUid::new(1001)));
    let root = policy.admit(&socket(private.0, private.1, UnixUid::new(0)));

    assert_eq!(
        [kind(owner), kind(stranger), kind(root)],
        [None, Some("peer_rejected"), Some("peer_rejected")]
    );
}

#[test]
fn socket_reachable_by_group_or_others_is_refused() {
    let policy = unix_policy();
    let private_dir = FileEvidence::new(OWNER, 0o700);
    let cases = [
        socket(FileEvidence::new(OWNER, 0o660), private_dir, OWNER),
        socket(FileEvidence::new(OWNER, 0o606), private_dir, OWNER),
        socket(
            FileEvidence::new(OWNER, 0o600),
            FileEvidence::new(OWNER, 0o755),
            OWNER,
        ),
        socket(
            FileEvidence::new(UnixUid::new(1001), 0o600),
            private_dir,
            OWNER,
        ),
        socket(
            FileEvidence::new(OWNER, 0o600),
            FileEvidence::new(UnixUid::new(0), 0o700),
            OWNER,
        ),
    ];

    let kinds = cases
        .iter()
        .map(|evidence| kind(policy.admit(evidence)))
        .collect::<Vec<_>>();

    assert_eq!(kinds, [Some("endpoint_insecure"); 5]);
}

#[test]
fn pipe_requires_owner_only_dacl_remote_rejection_and_owner_client() -> TestResult {
    let policy = EndpointPolicy::new(LocalAccount::Windows(sid(OWNER_SID)?));
    let pipe = |evidence: PipeInstanceEvidence, client: &str| -> TestResult<PeerEvidence> {
        Ok(PeerEvidence::NamedPipe {
            instance: evidence,
            client: sid(client)?,
        })
    };
    let everyone = Dacl::Entries(vec![
        Ace::new(AceKind::Allow, sid(OWNER_SID)?),
        Ace::new(AceKind::Allow, sid(EVERYONE_SID)?),
    ]);
    let deny_then_owner = Dacl::Entries(vec![
        Ace::new(AceKind::Deny, sid(EVERYONE_SID)?),
        Ace::new(AceKind::Allow, sid(OWNER_SID)?),
    ]);
    let cases = [
        pipe(
            instance(
                PipeInstance::First,
                RemoteClients::Rejected,
                owner_only_dacl()?,
            )?,
            OWNER_SID,
        )?,
        pipe(
            instance(
                PipeInstance::First,
                RemoteClients::Rejected,
                deny_then_owner,
            )?,
            OWNER_SID,
        )?,
        pipe(
            instance(PipeInstance::First, RemoteClients::Rejected, Dacl::Null)?,
            OWNER_SID,
        )?,
        pipe(
            instance(PipeInstance::First, RemoteClients::Rejected, everyone)?,
            OWNER_SID,
        )?,
        pipe(
            instance(
                PipeInstance::First,
                RemoteClients::Accepted,
                owner_only_dacl()?,
            )?,
            OWNER_SID,
        )?,
        pipe(
            instance(
                PipeInstance::Additional,
                RemoteClients::Rejected,
                owner_only_dacl()?,
            )?,
            OTHER_SID,
        )?,
    ];

    let kinds = cases
        .iter()
        .map(|evidence| kind(policy.admit(evidence)))
        .collect::<Vec<_>>();

    assert_eq!(
        kinds,
        [
            None,
            None,
            Some("endpoint_insecure"),
            Some("endpoint_insecure"),
            Some("endpoint_insecure"),
            Some("peer_rejected")
        ]
    );
    Ok(())
}

#[test]
fn pipe_namespace_claims_the_name_again_after_every_full_close() -> TestResult {
    let mut namespace = PipeNamespace::new(sid(OWNER_SID)?);
    let first = instance(
        PipeInstance::First,
        RemoteClients::Rejected,
        owner_only_dacl()?,
    )?;
    let additional = instance(
        PipeInstance::Additional,
        RemoteClients::Rejected,
        owner_only_dacl()?,
    )?;

    let unclaimed_additional = kind(namespace.open(&additional));
    namespace.open(&first)?;
    let second_first = kind(namespace.open(&first));
    namespace.open(&additional)?;
    namespace.close()?;
    namespace.open(&additional)?;
    namespace.close()?;
    namespace.close()?;
    let after_full_close = kind(namespace.open(&additional));
    let over_close = kind(PipeNamespace::new(sid(OWNER_SID)?).close());

    assert_eq!(
        [
            unclaimed_additional,
            second_first,
            after_full_close,
            over_close
        ],
        [Some("endpoint_insecure"); 4]
    );
    assert_eq!(kind(namespace.open(&first)), None);
    Ok(())
}

#[test]
fn malformed_sid_is_invalid_params() {
    let kinds = ["", "1-5-21", "S-1-x", "S-1-5-"].map(|text| kind(WindowsSid::parse(text)));

    assert_eq!(kinds, [Some("invalid_params"); 4]);
}
