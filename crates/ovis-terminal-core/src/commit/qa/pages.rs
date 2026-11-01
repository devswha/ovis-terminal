//! Pages of one pinned revision carry total, continuation and truncation facts and stop
//! at an explicit expiry.

use ovis_terminal_core::commit::{Boundary, CommitSource, PageTtl, Pager, Window};
use ovis_terminal_core::events::StateSnapshot;
use ovis_terminal_protocol::schema::scalar::{ByteOffset, TimestampMs};
use sha2::{Digest, Sha256};

use super::*;

const PAGE: usize = 1024;

fn small_pages() -> Limits {
    let mut limits = Limits::default();
    limits.page_bytes = PAGE;
    limits.pinned_snapshots = 1;
    limits
}

fn commit_n(boundary: &Boundary<Probe, Producer>, n: u64) -> TestResult {
    for i in 0..n {
        let record = wrote("registry", i)?;
        boundary.commit(CommitSource::Registry, |_| Ok(record))?;
    }
    Ok(())
}

const fn at(millis: u64) -> TimestampMs {
    TimestampMs::new(millis)
}

fn read_all(pager: &Pager<Probe, Producer>, window: &Window) -> Result<Vec<u8>, Box<dyn Error>> {
    let (mut bytes, mut offset) = (Vec::new(), Some(ByteOffset::new(0)));
    while let Some(start) = offset {
        let page = pager.page(&window.token, (start, 4 * PAGE, at(1_100)))?;
        assert_eq!(
            (page.cursor, page.total, page.offset),
            (window.cursor, window.total, start)
        );
        assert_eq!(page.expires_ms, window.expires_ms);
        let remaining = usize::try_from(window.total.get())?.saturating_sub(bytes.len());
        assert_eq!(page.data.len(), PAGE.min(remaining));
        assert_eq!(
            page.truncated,
            page.continuation.is_some(),
            "cut by the page bound"
        );
        bytes.extend_from_slice(&page.data);
        offset = page.continuation;
    }
    Ok(bytes)
}

#[test]
fn pages_stay_on_one_revision_until_their_expiry() -> TestResult {
    // Given three committed records and a window pinned at t=1000 for 500 ms.
    let fixture = Fixture::new();
    let core = fixture.limited(small_pages(), Producer { filler: 10_000 })?;
    let boundary = Boundary::new(Arc::clone(&core), 1)?;
    commit_n(&boundary, 3)?;
    let pager = Pager::new(Arc::clone(&core), PageTtl::new(500)?);
    let window = pager.open(at(1_000))?;
    assert_eq!(window.expires_ms, at(1_500));
    // When two more records commit before the pages are read.
    commit_n(&boundary, 2)?;
    let bytes = read_all(&pager, &window)?;
    // Then the reassembled pages are the pinned revision, end to end.
    assert_eq!(window.cursor.seq.get(), 3);
    assert_eq!(u64::try_from(bytes.len())?, window.total.get());
    assert_eq!(
        window.digest.as_str(),
        format!("sha256:{:x}", Sha256::digest(&bytes))
    );
    let snapshot: StateSnapshot<Captured> = serde_json::from_slice(&bytes)?;
    assert_eq!(snapshot.sections.seqs, vec![1, 2, 3]);
    // And an unaligned, short read returns exactly those bytes and is not truncated.
    let short = pager.page(&window.token, (ByteOffset::new(1_030), 5, at(1_499)))?;
    assert_eq!(
        (short.data.as_slice(), short.truncated),
        (bytes.get(1_030..1_035).ok_or("short range")?, false)
    );
    assert_eq!(short.continuation, Some(ByteOffset::new(1_035)));
    // And at the expiry the page is refused and the only pin slot is free again.
    let expired = pager
        .page(&window.token, (ByteOffset::new(0), PAGE, at(1_500)))
        .expect_err("expired");
    assert_eq!(expired.kind(), "page_expired");
    let gone = pager.page(&window.token, (ByteOffset::new(0), PAGE, at(1_500)));
    assert_eq!(gone.map(drop).map_err(|e| e.kind()), Err("stale_snapshot"));
    let fresh = pager.open(at(1_600))?;
    assert_eq!(fresh.cursor.seq.get(), 5);
    let wire = serde_json::to_value(&fresh)?;
    let expiry = wire
        .pointer("/expires_ms")
        .and_then(serde_json::Value::as_str);
    let total = wire
        .pointer("/total")
        .is_some_and(serde_json::Value::is_string);
    assert_eq!((expiry, total), (Some("2100"), true));
    pager.release(&fresh.token)?;
    drop((pager, boundary));
    close(core)
}

#[test]
fn reaping_releases_exactly_the_expired_windows() -> TestResult {
    // Given one window expiring at t=2500.
    let fixture = Fixture::new();
    let core = fixture.limited(small_pages(), Producer::default())?;
    let pager = Pager::new(Arc::clone(&core), PageTtl::new(500)?);
    let window = pager.open(at(2_000))?;
    // When the clock is reaped just before and then at the expiry.
    assert_eq!(pager.reap(at(2_499))?, 0);
    assert!(
        pager
            .page(&window.token, (ByteOffset::new(0), PAGE, at(2_499)))
            .is_ok()
    );
    assert_eq!(pager.reap(at(2_500))?, 1);
    // Then the token is stale and invalid bounds are typed refusals.
    let stale = pager.page(&window.token, (ByteOffset::new(0), PAGE, at(2_000)));
    assert_eq!(stale.map(drop).map_err(|e| e.kind()), Err("stale_snapshot"));
    assert_eq!(
        PageTtl::new(0).map(drop).map_err(|e| e.kind()),
        Err("invalid_params")
    );
    let next = pager.open(at(2_600))?;
    let zero = pager.page(&next.token, (ByteOffset::new(0), 0, at(2_600)));
    assert_eq!(zero.map(drop).map_err(|e| e.kind()), Err("invalid_params"));
    let past = pager.page(
        &next.token,
        (ByteOffset::new(next.total.get()), 1, at(2_600)),
    );
    assert_eq!(past.map(drop).map_err(|e| e.kind()), Err("invalid_params"));
    pager.release(&next.token)?;
    drop(pager);
    close(core)
}
