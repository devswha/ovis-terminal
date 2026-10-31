//! Torn snapshot sequence rejection at the public reconstruction boundary.

use ovis_terminal_core::events::{
    Limits, SnapshotAssembler, SnapshotLease, SnapshotPage, SnapshotTornReason, StateSnapshot,
    SubscribeRequest,
};
use ovis_terminal_protocol::schema::scalar::ByteOffset;
use proptest::prelude::*;

use super::{
    Changed, Coordinator, EventError, Fixture, Producer, SnapshotSections, StoreOptions,
    TestResult, change, identity,
};

type TestSnapshot = StateSnapshot<SnapshotSections<(), Vec<String>>>;

struct PageCase {
    fixture: Fixture,
    core: Coordinator<Changed, Producer>,
    lease: SnapshotLease,
    pages: Vec<SnapshotPage>,
}

impl PageCase {
    fn new() -> Result<Self, Box<dyn std::error::Error>> {
        let fixture = Fixture::new();
        let mut limits = Limits::default();
        limits.page_bytes = 64;
        let core = Coordinator::new(
            fixture.open(StoreOptions::default())?,
            identity(1)?,
            Producer,
        )
        .with_limits(limits)?;
        core.commit(change(&"x".repeat(4096), 0)?)?;
        let subscription = core.subscribe_paged(SubscribeRequest::default())?;
        let lease = subscription.snapshot.ok_or("snapshot lease missing")?;
        let pages = collect_pages(&core, &lease)?;
        Ok(Self {
            fixture,
            core,
            lease,
            pages,
        })
    }

    fn close(self) -> TestResult {
        self.core.release_snapshot(&self.lease.token)?;
        self.core.close()?;
        drop(self.fixture);
        Ok(())
    }
}

fn collect_pages(
    core: &Coordinator<Changed, Producer>,
    lease: &SnapshotLease,
) -> Result<Vec<SnapshotPage>, EventError> {
    let mut pages = Vec::new();
    let mut offset = ByteOffset::new(0);
    loop {
        let page = core.page(&lease.token, offset)?;
        let next = page.next_offset;
        pages.push(page);
        match next {
            Some(next) => offset = next,
            None => return Ok(pages),
        }
    }
}

fn assemble(lease: &SnapshotLease, pages: &[SnapshotPage]) -> Result<TestSnapshot, EventError> {
    let mut assembler = SnapshotAssembler::new(lease.clone());
    for page in pages {
        assembler.push(page)?;
    }
    assembler.finish()
}

fn assert_torn(error: &EventError, page: u64, reason: SnapshotTornReason) {
    assert_eq!(error.kind(), "snapshot_torn");
    assert_eq!(
        error
            .snapshot_page_index()
            .map(ovis_terminal_core::events::PageIndex::get),
        Some(page)
    );
    assert_eq!(error.snapshot_torn_reason(), Some(reason));
    assert_ne!(error.hint(), "");
}

#[test]
fn missing_interior_page_is_snapshot_torn() -> TestResult {
    // Given a complete sequence with one interior page removed.
    let mut case = PageCase::new()?;
    case.pages.remove(2);
    // When the remaining pages are assembled.
    let error = assemble(&case.lease, &case.pages).expect_err("missing page must fail");
    // Then the first missing index is reported before state is exposed.
    assert_torn(&error, 2, SnapshotTornReason::MissingPage);
    case.close()
}

#[test]
fn missing_final_page_is_snapshot_torn() -> TestResult {
    // Given a complete sequence without its final page.
    let mut case = PageCase::new()?;
    let final_index = case.pages.len().checked_sub(1).ok_or("pages missing")?;
    case.pages.pop();
    // When reconstruction is finalized.
    let error = assemble(&case.lease, &case.pages).expect_err("missing final page must fail");
    // Then the absent final index is reported.
    assert_torn(
        &error,
        u64::try_from(final_index)?,
        SnapshotTornReason::MissingPage,
    );
    case.close()
}

#[test]
fn duplicated_page_is_snapshot_torn() -> TestResult {
    // Given one page delivered twice.
    let mut case = PageCase::new()?;
    let duplicate = case.pages.get(1).ok_or("second page missing")?.clone();
    case.pages.insert(2, duplicate);
    // When reconstruction consumes the duplicate.
    let error = assemble(&case.lease, &case.pages).expect_err("duplicate page must fail");
    // Then the next expected index identifies the sequence break.
    assert_torn(&error, 2, SnapshotTornReason::DuplicatePage);
    case.close()
}

#[test]
fn reordered_pages_are_snapshot_torn() -> TestResult {
    // Given two adjacent pages in reverse order.
    let mut case = PageCase::new()?;
    case.pages.swap(1, 2);
    // When reconstruction reaches the out-of-order page.
    let error = assemble(&case.lease, &case.pages).expect_err("reordered pages must fail");
    // Then the first missing index is reported.
    assert_torn(&error, 1, SnapshotTornReason::MissingPage);
    case.close()
}

#[test]
fn truncated_final_page_is_snapshot_torn() -> TestResult {
    // Given a final page missing its last byte.
    let mut case = PageCase::new()?;
    let final_index = case.pages.len().checked_sub(1).ok_or("pages missing")?;
    case.pages
        .get_mut(final_index)
        .ok_or("final page missing")?
        .data
        .pop();
    // When reconstruction consumes the shortened page.
    let error = assemble(&case.lease, &case.pages).expect_err("truncated page must fail");
    // Then the damaged page index is reported.
    assert_torn(
        &error,
        u64::try_from(final_index)?,
        SnapshotTornReason::TruncatedPage,
    );
    case.close()
}

#[test]
fn page_metadata_uses_decimal_strings_and_rejects_numbers() -> TestResult {
    // Given one emitted digest-bound snapshot page.
    let case = PageCase::new()?;
    let page = case.pages.first().ok_or("first page missing")?;
    // When its wire representation is inspected and counter forms are mutated.
    let value = serde_json::to_value(page)?;
    assert!(
        value
            .get("page_index")
            .is_some_and(serde_json::Value::is_string)
    );
    assert!(
        value
            .get("page_count")
            .is_some_and(serde_json::Value::is_string)
    );
    assert!(value.get("total").is_some_and(serde_json::Value::is_string));
    assert!(
        value
            .get("offset")
            .is_some_and(serde_json::Value::is_string)
    );
    assert!(
        value
            .get("digest")
            .and_then(serde_json::Value::as_str)
            .is_some_and(|digest| digest.starts_with("sha256:") && digest.len() == 71)
    );
    for field in ["page_index", "page_count", "total", "offset"] {
        let mut invalid = value.clone();
        *invalid.get_mut(field).ok_or("counter field missing")? = serde_json::json!(1);
        assert!(serde_json::from_value::<SnapshotPage>(invalid).is_err());
    }
    // Then only canonical decimal strings cross the page boundary.
    assert_eq!(serde_json::from_value::<SnapshotPage>(value)?, *page);
    case.close()
}

proptest! {
    #[test]
    fn any_removed_page_is_snapshot_torn(removed in 0usize..16) {
        // Given a multi-page snapshot with an arbitrary early page removed.
        let mut case = PageCase::new()
            .map_err(|error| TestCaseError::fail(error.to_string()))?;
        prop_assume!(removed < case.pages.len());
        case.pages.remove(removed);
        // When the incomplete sequence is reconstructed.
        let result = assemble(&case.lease, &case.pages);
        // Then no removed page can produce complete state.
        prop_assert!(matches!(result, Err(ref error) if error.kind() == "snapshot_torn"));
        case.close().map_err(|error| TestCaseError::fail(error.to_string()))?;
    }
}
