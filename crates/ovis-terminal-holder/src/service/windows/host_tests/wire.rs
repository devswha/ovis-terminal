//! A test core speaking the holder wire over the pipe.

use std::collections::BTreeMap;
use std::time::Duration;

use ovis_terminal_protocol::domain::windows_service::PipeName;
use tokio::io::{AsyncReadExt as _, AsyncWriteExt as _};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

use crate::protocol::{
    CoreBuildId, FrameDecoder, HOLDER_PROTOCOL_GENERATION, Hello, Input, Message,
    PreviousCoreState, Spawn, Spawned, TerminalId, TerminalIncarnation, TerminalTarget, WriteGrant,
    encode_message,
};

const BOUND: Duration = Duration::from_secs(60);

pub(super) struct Core {
    pipe: NamedPipeClient,
    decoder: FrameDecoder,
    queued: Vec<Message>,
}

impl Core {
    pub(super) fn connect(name: &PipeName) -> Self {
        let pipe = ClientOptions::new().open(name.as_str()).unwrap();
        Self {
            pipe,
            decoder: FrameDecoder::default(),
            queued: Vec::new(),
        }
    }

    pub(super) async fn send(&mut self, message: Message) {
        self.pipe
            .write_all(&encode_message(&message).unwrap())
            .await
            .unwrap();
    }

    pub(super) async fn next(&mut self) -> Message {
        loop {
            if !self.queued.is_empty() {
                return self.queued.remove(0);
            }
            let mut buffer = vec![0_u8; 65536];
            let read = tokio::time::timeout(BOUND, self.pipe.read(&mut buffer))
                .await
                .unwrap()
                .unwrap();
            assert_ne!(read, 0, "the holder closed the link");
            self.queued
                .extend(self.decoder.push(buffer.get(..read).unwrap()).unwrap());
        }
    }

    pub(super) async fn adopt(&mut self, previous: PreviousCoreState) -> crate::protocol::Welcome {
        let hello = Hello {
            generation: HOLDER_PROTOCOL_GENERATION,
            core_build: CoreBuildId::new("test-core".to_owned()),
            previous,
        };
        self.send(Message::Hello(hello)).await;
        match self.next().await {
            Message::Welcome(welcome) => welcome,
            other => panic!("expected welcome, got {other:?}"),
        }
    }

    /// Collects output until `needle` appears, answering ConPTY's cursor query so the
    /// pseudoconsole starts the program.
    pub(super) async fn output_until(&mut self, spawned: &Spawned, needle: &str) -> Vec<u8> {
        let mut seen = Vec::new();
        let mut sequence = 0;
        while !String::from_utf8_lossy(&seen).contains(needle) {
            let Message::Output(output) = self.next().await else {
                continue;
            };
            seen.extend_from_slice(&output.bytes);
            if output.bytes.windows(4).any(|w| w == b"\x1b[6n") {
                sequence = u64::checked_add(sequence, 1).unwrap();
                self.send(Message::Input(input(spawned, sequence, b"\x1b[1;1R")))
                    .await;
            }
        }
        seen
    }
}

pub(super) fn target(spawned: &Spawned) -> TerminalTarget {
    TerminalTarget {
        terminal_id: spawned.terminal_id.clone(),
        terminal_incarnation: spawned.terminal_incarnation,
        process_incarnation: Some(spawned.process_incarnation),
    }
}

fn input(spawned: &Spawned, sequence: u64, bytes: &[u8]) -> Input {
    let grant = WriteGrant {
        grant_epoch: 1,
        fence_epoch: 1,
        process_incarnation: spawned.process_incarnation,
    };
    Input {
        target: target(spawned),
        sequence,
        grant,
        bytes: bytes.to_vec(),
    }
}

pub(super) fn spawn(id: &str, script: &str) -> Message {
    let system = std::env::var("SystemRoot").unwrap();
    let env = BTreeMap::from([
        ("SystemRoot".to_owned(), system.clone()),
        ("PATH".to_owned(), format!(r"{system}\System32")),
    ]);
    Message::Spawn(Spawn {
        terminal_id: TerminalId::new(id.to_owned()),
        terminal_incarnation: TerminalIncarnation::new(1),
        argv: vec![
            format!(r"{system}\System32\cmd.exe"),
            "/d".to_owned(),
            "/c".to_owned(),
            script.to_owned(),
        ],
        cwd: std::env::temp_dir(),
        env,
        geometry: (80, 24),
    })
}

pub(super) fn runtime() -> tokio::runtime::Runtime {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .unwrap()
}
