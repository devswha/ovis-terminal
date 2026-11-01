//! Every advertised bound is reachable and refused with the lane's own typed error at it,
//! through the real operation ledger and store: the refusal settles the operation instead
//! of failing it on a ledger or store limit.

use super::*;

const KIB: usize = 1024;

fn user(id: &str, texts: &[usize]) -> Value {
    let blocks: Vec<Value> = texts
        .iter()
        .map(|len| json!({"type": "text", "text": "x".repeat(*len)}))
        .collect();
    json!({"record_id": id, "role": "user", "blocks": blocks})
}

fn refused(record: &OpRecord) -> Result<(String, Option<String>), Box<dyn Error>> {
    let error = refusal(record)?;
    let path = error
        .field_path
        .as_ref()
        .map(|path| path.as_str().to_owned());
    Ok((error.kind.token().to_owned(), path))
}

#[test]
fn a_64_kib_text_is_accepted_and_one_byte_more_is_refused_at_its_path() -> TestResult {
    let h = harness()?;
    h.register()?;

    let fits: RecordsAppended = result(&h.append(&json!([user("u1", &[64 * KIB])]))?)?;
    let over = h.append(&json!([user("u2", &[64 * KIB + 1])]))?;

    assert_eq!(fits.total_records.get(), 1);
    let read = h.page()?;
    assert_eq!(fits.digest.as_str(), digest_of(&read.records)?);
    assert_eq!(
        refused(&over)?,
        (
            "invalid_records".to_owned(),
            Some("params.records[0].blocks[0].text".to_owned())
        )
    );
    Ok(())
}

#[test]
fn a_record_above_256_kib_is_refused_at_the_record() -> TestResult {
    let h = harness()?;
    h.register()?;

    let over = h.append(&json!([user("u1", &[64 * KIB; 4])]))?;

    assert_eq!(
        refused(&over)?,
        (
            "invalid_records".to_owned(),
            Some("params.records[0]".to_owned())
        )
    );
    assert_eq!(
        h.read_refusal(AGENT)?.kind.token(),
        "conversation_unavailable"
    );
    Ok(())
}

#[test]
fn the_10000th_record_is_accepted_and_the_10001st_is_refused_as_too_large() -> TestResult {
    // Given: 9,984 records in full appends of 64.
    let h = harness()?;
    h.register()?;
    let batch = |from: usize, count: usize| -> Value {
        Value::Array(
            (from..from + count)
                .map(|n| text(&format!("r{n}"), "user", "x"))
                .collect(),
        )
    };
    for start in (0..9_984).step_by(64) {
        result::<RecordsAppended>(&h.append(&batch(start, 64))?)?;
    }

    // When: 16 more reach 10,000, then one more is appended.
    let full: RecordsAppended = result(&h.append(&batch(9_984, 16))?)?;
    let over = h.append(&batch(10_000, 1))?;

    // Then: the bound holds exactly, the refusal is the lane's own, and nothing moved.
    assert_eq!(full.total_records.get(), 10_000);
    assert_eq!(refused(&over)?, ("too_large".to_owned(), None));
    let page = h.page()?;
    assert_eq!(page.page.total, 10_000);
    assert_eq!(page.revision, full.revision);
    Ok(())
}

#[test]
fn records_beyond_8_mib_are_refused_as_too_large_with_counters_unchanged() -> TestResult {
    // Given: records of three 64 KiB texts, appended one per operation.
    let h = harness()?;
    h.register()?;
    let record = |n: usize| json!([user(&format!("r{n:0>3}"), &[64 * KIB; 3])]);
    let limit = 8 * 1024 * 1024;

    // When: records are appended until the lane refuses one.
    let mut kept: Vec<RecordsAppended> = Vec::new();
    let over = loop {
        let outcome = h.append(&record(kept.len()))?;
        if outcome.error.is_some() {
            break outcome;
        }
        kept.push(result(&outcome)?);
    };

    // Then: the refusal is the lane's too_large exactly where the next record would cross
    // 8 MiB, and the counters stay where the last accepted append left them.
    assert_eq!(refused(&over)?, ("too_large".to_owned(), None));
    let [.., before, last] = kept.as_slice() else {
        return Err("fewer than two records fit".into());
    };
    let size = last.total_bytes.get() - before.total_bytes.get();
    assert!(last.total_bytes.get() <= limit);
    assert!(last.total_bytes.get() + size > limit);
    assert_eq!(last.total_records.get(), u64::try_from(kept.len())?);
    assert_eq!(h.page()?.page.total, u32::try_from(kept.len())?);
    Ok(())
}

#[test]
fn a_full_append_of_longest_ids_and_labels_fits_the_receipt_and_one_byte_more_is_refused()
-> TestResult {
    // Given: a running turn.
    let h = harness()?;
    h.register()?;
    h.report(AgentState::Working, (1, 1_000))?;
    let id = |n: usize| format!("{n:0>128}");
    let records = Value::Array((0..64).map(|n| text(&id(n), "user", "x")).collect());
    let label = "m".repeat(256);
    let meta = serde_json::from_value(json!({"model": label, "reasoning": label}))?;

    // When: 64 records with 128-byte ids and 256-byte turn labels are appended.
    let full = h.append_as((TOKEN, &records), Some(meta))?;

    // Then: the receipt holds all 64 identities and the turn within the ledger bound.
    let appended: RecordsAppended = result(&full)?;
    assert_eq!(appended.record_ids.len(), 64);
    assert!(serde_json::to_vec(&appended)?.len() < 16 * KIB);
    let long_id = h.append(&json!([text(&"i".repeat(129), "user", "x")]))?;
    assert_eq!(
        refused(&long_id)?,
        (
            "invalid_records".to_owned(),
            Some("params.records[0].record_id".to_owned())
        )
    );
    let long_label = serde_json::from_value(json!({"model": "m".repeat(257)}))?;
    let over = h.append_as((TOKEN, &json!([])), Some(long_label))?;
    assert_eq!(
        refused(&over)?,
        (
            "invalid_records".to_owned(),
            Some("params.turn.model".to_owned())
        )
    );
    Ok(())
}
