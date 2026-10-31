//! Endpoint descriptors resolve from explicit environment values on both platform forms.

use std::path::PathBuf;

use ovis_terminal_protocol::schema::limits::TransportKind;
use ovis_terminal_server::listen::{
    AddressSource, EndpointAddress, EndpointDescriptor, EndpointEnvironment, EnvVar,
    MAX_UNIX_SOCKET_PATH, Platform, SessionName,
};
use proptest::prelude::*;
use serde_json::json;

fn unix(env: &EndpointEnvironment) -> Result<EndpointDescriptor, &'static str> {
    EndpointDescriptor::resolve(env, Platform::Unix).map_err(|error| error.kind())
}

#[test]
fn the_override_wins_and_is_reported_as_such() {
    let env = EndpointEnvironment::default()
        .with(EnvVar::RuntimeDir, "/run/user/1000")
        .with(EnvVar::SocketOverride, "/tmp/custom/ovis.sock");
    let descriptor = unix(&env).unwrap();
    let described = serde_json::to_value(&descriptor).unwrap();
    assert_eq!(
        described,
        json!({
            "kind": "unix",
            "address": "/tmp/custom/ovis.sock",
            "session": "default",
            "source": "env_override",
            "env_override": "OVIS_TERMINAL_SOCKET",
        })
    );
}

#[test]
fn unix_defaults_fall_back_from_runtime_to_state_to_home() {
    let home = EndpointEnvironment::default().with(EnvVar::Home, "/home/alice");
    let state = home.clone().with(EnvVar::StateDir, "/var/state");
    let runtime = state.clone().with(EnvVar::RuntimeDir, "/run/user/1000");
    let path = |env: &EndpointEnvironment| unix(env).unwrap().unix_path().unwrap().to_path_buf();
    assert_eq!(
        path(&home),
        PathBuf::from("/home/alice/.local/state/ovis-terminal/default.sock")
    );
    assert_eq!(
        path(&state),
        PathBuf::from("/var/state/ovis-terminal/default.sock")
    );
    assert_eq!(
        path(&runtime),
        PathBuf::from("/run/user/1000/ovis-terminal/default.sock")
    );
    let relative = EndpointEnvironment::default().with(EnvVar::RuntimeDir, "relative/dir");
    assert_eq!(unix(&relative).unwrap_err(), "no_runtime_directory");
}

#[test]
fn unsafe_values_are_refused_before_anything_is_touched() {
    let base = EndpointEnvironment::default().with(EnvVar::RuntimeDir, "/run/user/1000");
    let long = format!("/{}", "d".repeat(MAX_UNIX_SOCKET_PATH));
    let cases = [
        (
            base.clone().with(EnvVar::Session, "../escape"),
            "invalid_session",
        ),
        (
            base.clone().with(EnvVar::SocketOverride, "relative.sock"),
            "invalid_endpoint_override",
        ),
        (
            base.clone().with(EnvVar::SocketOverride, long),
            "socket_path_too_long",
        ),
    ];
    for (env, kind) in cases {
        assert_eq!(unix(&env).unwrap_err(), kind);
    }
    let empty = base.with(EnvVar::SocketOverride, "");
    assert_eq!(unix(&empty).unwrap().source(), AddressSource::Default);
}

#[test]
fn windows_names_a_local_pipe_per_user_and_session() {
    let env = EndpointEnvironment::default()
        .with(EnvVar::User, "alice")
        .with(EnvVar::Session, "work");
    let descriptor = EndpointDescriptor::resolve(&env, Platform::Windows).unwrap();
    assert_eq!(descriptor.kind(), TransportKind::Pipe);
    let expected = EndpointAddress::Pipe(r"\\.\pipe\ovis-terminal-alice-work".to_owned());
    assert_eq!(descriptor.address(), &expected);
    let remote = env.with(EnvVar::SocketOverride, r"\\server\pipe\ovis");
    let refused = EndpointDescriptor::resolve(&remote, Platform::Windows).unwrap_err();
    assert_eq!(refused.kind(), "invalid_endpoint_override");
}

proptest! {
    #[test]
    fn every_valid_session_names_its_own_socket(session in "[A-Za-z0-9_-]{1,32}") {
        let env = EndpointEnvironment::default()
            .with(EnvVar::RuntimeDir, "/run/user/1000")
            .with(EnvVar::Session, session.clone());
        let descriptor = unix(&env).unwrap();
        prop_assert_eq!(descriptor.session().as_str(), session.as_str());
        let file = descriptor.unix_path().unwrap().file_name().unwrap().to_str().unwrap();
        prop_assert_eq!(file.strip_suffix(".sock"), Some(session.as_str()));
    }

    #[test]
    fn session_names_follow_the_published_rule(text in "\\PC{0,40}") {
        let allowed = (1..=32).contains(&text.len())
            && text.bytes().all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
        let parsed = SessionName::parse(&text).map(|name| name.as_str().to_owned());
        prop_assert_eq!(parsed.map_err(|error| error.kind()), if allowed { Ok(text) } else { Err("invalid_session") });
    }
}
