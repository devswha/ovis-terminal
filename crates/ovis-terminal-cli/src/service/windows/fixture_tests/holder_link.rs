//! The fixture's holder-wire client, opened as the fixture account.

use std::os::windows::io::{AsRawHandle as _, FromRawHandle as _, OwnedHandle};

use ovis_terminal_holder::protocol::{
    CoreBuildId, FrameDecoder, HOLDER_PROTOCOL_GENERATION, HeldTerminal, Hello, Input, Message,
    PreviousCoreState, Spawn, Spawned, TerminalId, TerminalIncarnation, TerminalTarget, WriteGrant,
    encode_message,
};
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};
use windows_sys::Win32::Security::{
    ImpersonateLoggedOnUser, LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT, LogonUserW,
    RevertToSelf,
};

use super::{MARKER, var, wide};

/// Opens `pipe` as the fixture account, through a network logon this thread impersonates.
fn open_as_fixture(pipe: &str) -> std::io::Result<NamedPipeClient> {
    let account = var("OVIS54_ACCOUNT");
    let (domain, name) = account.split_once('\\').unwrap();
    let (name, domain, password) = (wide(name), wide(domain), wide(&var("OVIS54_PASSWORD")));
    let mut raw = std::ptr::null_mut();
    let (kind, provider) = (LOGON32_LOGON_NETWORK, LOGON32_PROVIDER_DEFAULT);
    // SAFETY: every string is NUL-terminated and `raw` is a live out-pointer.
    let logged_on = unsafe {
        LogonUserW(
            name.as_ptr(),
            domain.as_ptr(),
            password.as_ptr(),
            kind,
            provider,
            &raw mut raw,
        )
    };
    assert_ne!(logged_on, 0, "{}", std::io::Error::last_os_error());
    // SAFETY: LogonUserW returned a new token nothing else owns.
    let token = unsafe { OwnedHandle::from_raw_handle(raw) };
    // SAFETY: the token is open; impersonation is reverted below.
    let impersonating = unsafe { ImpersonateLoggedOnUser(token.as_raw_handle()) };
    assert_ne!(impersonating, 0);
    let opened = ClientOptions::new().open(pipe);
    // SAFETY: ends the impersonation started above.
    let reverted = unsafe { RevertToSelf() };
    assert_ne!(reverted, 0);
    opened
}

pub(super) struct Core(NamedPipeClient, FrameDecoder, Vec<Message>);

impl Core {
    pub(super) async fn send(&mut self, message: Message) {
        self.0
            .write_all(&encode_message(&message).unwrap())
            .await
            .unwrap();
    }

    pub(super) async fn next(&mut self) -> Message {
        while self.2.is_empty() {
            let mut buffer = vec![0_u8; 65536];
            let read = self.0.read(&mut buffer).await.unwrap();
            assert_ne!(read, 0, "the holder closed the link");
            self.2
                .extend(self.1.push(buffer.get(..read).unwrap()).unwrap());
        }
        self.2.remove(0)
    }

    pub(super) async fn adopt(pipe: &str) -> (Self, Vec<HeldTerminal>) {
        let mut core = Self(
            open_as_fixture(pipe).unwrap(),
            FrameDecoder::default(),
            Vec::new(),
        );
        let hello = Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("fixture".to_owned()),
            previous: PreviousCoreState::Crash,
        };
        core.send(Message::Hello(hello)).await;
        let Message::Welcome(welcome) = core.next().await else {
            panic!("no welcome")
        };
        (core, welcome.terminals)
    }
}

fn target(spawned: &Spawned) -> TerminalTarget {
    TerminalTarget {
        terminal_id: spawned.terminal_id.clone(),
        terminal_incarnation: spawned.terminal_incarnation,
        process_incarnation: Some(spawned.process_incarnation),
    }
}

/// The launcher role: start the marker program through the holder, report, then wait to be killed.
pub(super) async fn launch(pipe: &str) {
    let (mut core, _) = Core::adopt(pipe).await;
    let system = var("SystemRoot");
    let env = [
        ("SystemRoot".to_owned(), system.clone()),
        ("PATH".to_owned(), format!(r"{system}\System32")),
    ]
    .into();
    let script = format!("echo {MARKER}& ping -n 600 127.0.0.1 >NUL");
    let argv = vec![
        format!(r"{system}\System32\cmd.exe"),
        "/d".to_owned(),
        "/c".to_owned(),
        script,
    ];
    core.send(Message::Spawn(Spawn {
        terminal_id: TerminalId::new("t-ci".to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv,
        cwd: system.into(),
        env,
        geometry: (80, 24),
    }))
    .await;
    let Message::Spawned(spawned) = core.next().await else {
        panic!("no spawned")
    };
    println!("SPAWNED {} {}", spawned.pid, spawned.start_token.as_str());
    let mut seen = Vec::new();
    while !String::from_utf8_lossy(&seen).contains(MARKER) {
        let Message::Output(output) = core.next().await else {
            continue;
        };
        if output.bytes.windows(4).any(|w| w == b"\x1b[6n") {
            let grant = WriteGrant {
                grant_epoch: 1,
                fence_epoch: 1,
                process_incarnation: spawned.process_incarnation,
            };
            core.send(Message::Input(Input {
                target: target(&spawned),
                sequence: 1,
                grant,
                bytes: b"\x1b[1;1R".to_vec(),
            }))
            .await;
        }
        seen.extend(output.bytes);
    }
    println!("MARKER-SEEN");
    std::future::pending::<()>().await;
}
