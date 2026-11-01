use std::error::Error;
use std::sync::Arc;

use ovis_terminal_core::health::{Health, Observed, RecoveryLedger};
use ovis_terminal_core::recovery::Incidents;
use ovis_terminal_core::subscriptions::{Request, Selector, Start, Started};
use ovis_terminal_protocol::domain::health::{ConsumerLoss, ConsumerRef, LossRange};
use ovis_terminal_protocol::schema::ids::{self, ClientId};
use ovis_terminal_protocol::schema::methods::server::{ConsumerQueue, ServerMetricsResult};
use ovis_terminal_protocol::schema::scalar::{EventSeq, TimestampMs};

use super::fixture::{Bounds, Data, Producer, TestResult, World, identity};
use super::pulls::{scan_to, until_missed};

fn reporting(world: &World) -> Result<(Arc<Health>, std::path::PathBuf), Box<dyn Error>> {
    let dir = std::env::temp_dir().join(format!("ovis-sub-health-{}", uuid::Uuid::now_v7()));
    std::fs::create_dir_all(&dir)?;
    let runtime = identity(1)?;
    let boot = ids::ServerBootId::new(runtime.server_boot_id().to_string());
    let ledger = RecoveryLedger::open(Incidents::new(&dir), (boot, TimestampMs::new(1)), None)?;
    let health = Arc::new(Health::new(runtime, ledger));
    if !world.hub.report_to(Arc::clone(&health)) {
        return Err("the hub already reported elsewhere".into());
    }
    Ok((health, dir))
}

fn subscribe(
    world: &World,
    client: Option<&str>,
) -> Result<Started<Data, Producer>, Box<dyn Error>> {
    let request = Request::new(Start::Head, Selector::all());
    let request = match client {
        Some(client) => request.for_client(ClientId::new(client.to_owned())),
        None => request,
    };
    Ok(world.hub.subscribe(request)?)
}

fn metrics(health: &Health) -> Result<ServerMetricsResult, Box<dyn Error>> {
    Ok(health.metrics(&Observed::new(EventSeq::new(0), Vec::new()))?)
}

fn row<'a>(
    read: &'a ServerMetricsResult,
    started: &Started<Data, Producer>,
) -> Option<&'a ConsumerQueue> {
    read.consumers.iter().find(|row| {
        row.consumer
            == ConsumerRef::Subscription {
                subscription_id: started.feed.id().clone(),
            }
    })
}

#[test]
fn a_real_stalled_subscription_reports_its_queue_loss_and_resync() -> TestResult {
    // Given a hub reporting to health, a witness that keeps reading and a stalled
    // consumer whose queue holds at most two events.
    let mut bounds = Bounds::v1()?;
    bounds.queue.queue_events = 2;
    let world = World::open(bounds)?;
    let (health, dir) = reporting(&world)?;
    let witness = subscribe(&world, Some("witness"))?;
    let stalled = subscribe(&world, Some("viewer"))?;
    let anonymous = subscribe(&world, None)?;
    let first = world.set(1, "one")?.seq.get();
    scan_to(&witness.feed, first)?;
    // Then the stalled consumer's undelivered event is queued bytes, the witness's is not.
    let queued = metrics(&health)?;
    assert_eq!(queued.subscriptions.get(), 2);
    assert_eq!(queued.streams.get(), 0);
    assert_eq!(
        queued.consumers_untracked.get(),
        1,
        "a subscription without a client is counted, not invented"
    );
    assert!(
        row(&queued, &stalled)
            .ok_or("stalled row")?
            .queue_bytes
            .get()
            > 0
    );
    assert_eq!(
        row(&queued, &witness)
            .ok_or("witness row")?
            .queue_bytes
            .get(),
        0
    );
    // When two more commits overflow its queue.
    let mut newest = first;
    for value in ["two", "three"] {
        newest = world.set(1, value)?.seq.get();
        scan_to(&witness.feed, newest)?;
    }
    // Then its queue is dropped and the matching range it missed is reported, not resynced.
    let lost = metrics(&health)?;
    let lost_row = row(&lost, &stalled).ok_or("stalled row")?;
    let range = LossRange::Events {
        first_seq: EventSeq::new(first),
        last_seq: EventSeq::new(newest),
    };
    assert_eq!(lost_row.queue_bytes.get(), 0);
    assert_eq!(lost_row.losses, vec![ConsumerLoss::dropped(range)]);
    // When it reads again and recovers from a fresh snapshot.
    until_missed(&stalled.feed)?;
    // Then the same range is marked resynchronized and counted.
    let resynced = metrics(&health)?;
    let resynced_row = row(&resynced, &stalled).ok_or("stalled row")?;
    assert_eq!(resynced_row.losses, vec![ConsumerLoss::replaced(range)]);
    assert_eq!(resynced_row.resynced_events.get(), range.events());
    // When the consumer goes away, its row goes with it.
    drop(stalled);
    let after = metrics(&health)?;
    assert_eq!(after.subscriptions.get(), 1);
    drop((witness, anonymous));
    world.finish()?;
    std::fs::remove_dir_all(dir)?;
    Ok(())
}
