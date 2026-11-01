//! Attaching a line completes only once the actor took it, never when it was only queued.

use std::future::Future as _;
use std::pin::pin;
use std::sync::Arc;
use std::task::{Context, Waker};
use std::time::Duration;

use ovis_terminal_protocol::schema::methods::machine::MachineState;
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use ovis_terminal_remote::actors::{Actors, Limits, Line, Request};
use ovis_terminal_remote::orphans::OrphanError;
use ovis_terminal_remote::transport::TransportError;
use serde_json::json;
use tokio::sync::{Semaphore, oneshot};

use super::super::attach::attach_applied;
use super::harness::remote_id;
use super::{HANG, TestResult};

/// A line whose writes wait for a permit; the first write reports that it started.
#[derive(Debug)]
struct Held {
    entered: Option<oneshot::Sender<()>>,
    release: Arc<Semaphore>,
}

impl Line for Held {
    async fn send(&mut self, _: &[u8]) -> Result<(), TransportError> {
        if let Some(entered) = self.entered.take() {
            entered.send(()).unwrap_or_default();
        }
        let permit = self.release.acquire().await;
        permit
            .map(tokio::sync::SemaphorePermit::forget)
            .map_err(|_| std::io::Error::from(std::io::ErrorKind::BrokenPipe).into())
    }

    async fn receive(&mut self) -> Result<Vec<u8>, TransportError> {
        std::future::pending().await
    }
}

#[tokio::test]
async fn an_attach_returns_only_after_the_busy_actor_took_the_new_line() -> TestResult {
    let machine = remote_id(0xe1);
    let (actors, _inbound) = Actors::new(Limits::new(8, Duration::from_secs(60), 256));
    let actors = Arc::new(actors);
    actors.add(serde_json::from_value(json!({
        "machine_id": machine, "name": "busy", "aliases": [], "state": "connecting",
        "remote_lifetime": "runtime:7", "remote_boot_id": "1", "last_seen_seq": null,
        "version": "1.0.0", "revision": "1",
    }))?)?;
    let release = Arc::new(Semaphore::new(0));
    let (entered, started) = oneshot::channel();
    let mut health = actors.watch_health(&machine)?;
    let first = Held {
        entered: Some(entered),
        release: Arc::clone(&release),
    };
    actors.attach(&machine, first).await?;
    tokio::time::timeout(HANG, health.wait_for(|health| health.line_epoch.get() == 1)).await??;
    let calling = Arc::clone(&actors);
    let target = machine.clone();
    let call = tokio::spawn(async move {
        let request = Request::new("server.status", json!({}), Duration::from_secs(60));
        calling.call(&target, request).await
    });
    tokio::time::timeout(HANG, started).await??;

    let second = Held {
        entered: None,
        release: Arc::clone(&release),
    };
    let handed = async {
        actors
            .attach(&machine, second)
            .await
            .map_err(OrphanError::from)
    };
    let mut applied = pin!(attach_applied(&actors, &machine, (handed, HANG)));
    let first_poll = applied
        .as_mut()
        .poll(&mut Context::from_waker(Waker::noop()));
    release.add_permits(1);
    tokio::time::timeout(HANG, applied).await??;
    let after = actors.health(&machine)?;

    assert!(
        first_poll.is_pending(),
        "the attach returned while the actor was still writing the earlier call"
    );
    assert_eq!(after.line_epoch.get(), 2);
    assert_eq!(after.state, OpenEnum::Known(MachineState::Connected));
    drop(tokio::time::timeout(HANG, call).await?);
    Ok(())
}
