use serde_json::json;

use super::*;

#[test]
fn equality_compares_canonical_work_not_json_key_order() -> Result<(), OperationError> {
    // Given identical operation identities with reordered typed parameter objects.
    let id = OpId::new("01800000-0000-7000-8000-000000000001".into());
    let first = Request::new(
        id.clone(),
        "doc.set".into(),
        serde_json::from_str(r#"{"name":"a","value":{"b":2,"a":1}}"#)?,
    )?;
    let reordered = Request::new(
        id.clone(),
        "doc.set".into(),
        serde_json::from_str(r#"{"value":{"a":1,"b":2},"name":"a"}"#)?,
    )?;
    // When canonical fingerprints are compared for coalescing.
    assert_eq!(first, reordered);
    // Then changed parameters or a different method never join that execution.
    assert_ne!(
        first,
        Request::new(
            id.clone(),
            "doc.set".into(),
            json!({"name":"b","value":{"a":1,"b":2}})
        )?
    );
    assert_ne!(
        first,
        Request::new(
            id,
            "doc.get".into(),
            json!({"name":"a","value":{"a":1,"b":2}})
        )?
    );
    Ok(())
}
