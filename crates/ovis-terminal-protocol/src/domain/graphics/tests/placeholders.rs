//! The screen-read placeholder shape and the `include_images` read option (plan todo 85).

use serde_json::{Value, json};

use super::super::{
    GraphicsBuffer, GraphicsImageId, GraphicsIndexEntry, GraphicsIndexPlacement,
    GraphicsPlaceholder, GraphicsProtocol, PlaceholderCells,
};
use crate::rpc::parse_request;
use crate::schema::decode_params;
use crate::schema::methods::terminal_read::{TerminalScreenParams, TerminalScreenResult};
use crate::schema::scalar::ByteCount;

type TestResult = Result<(), Box<dyn std::error::Error>>;

/// `base` with `key` set to `value`.
fn with(mut base: Value, key: &str, value: Value) -> Value {
    if let Some(fields) = base.as_object_mut() {
        fields.insert(key.to_owned(), value);
    }
    base
}

fn screen_request(include_images: Option<Value>) -> Result<crate::rpc::RpcRequest, String> {
    let mut params = json!({"target": {"address": "t1"}, "format": "text"});
    if let (Some(flag), Some(fields)) = (include_images, params.as_object_mut()) {
        fields.insert("include_images".to_owned(), flag);
    }
    let request =
        json!({"jsonrpc": "2.0", "id": "r", "method": "terminal.screen", "params": params});
    parse_request(&request.to_string()).map_err(|error| error.to_string())
}

#[test]
fn a_placeholder_names_its_store_entry_in_decimal_strings_and_never_a_payload() -> TestResult {
    let placeholder = GraphicsPlaceholder::new(
        (GraphicsImageId::new(u64::MAX), GraphicsProtocol::Sixel),
        PlaceholderCells::new((3, 7), (2, 4)),
        ByteCount::new(u64::MAX),
    );
    let encoded = serde_json::to_value(&placeholder)?;
    assert_eq!(
        encoded,
        json!({
            "image_id": "18446744073709551615",
            "format": "sixel",
            "row": 3,
            "col": 7,
            "rows": 2,
            "cols": 4,
            "bytes": "18446744073709551615"
        })
    );
    assert_eq!(
        serde_json::from_value::<GraphicsPlaceholder>(encoded)?,
        placeholder
    );
    Ok(())
}

#[test]
fn a_numeric_image_id_or_an_extra_payload_field_is_refused() {
    let base = json!({"image_id": "4", "format": "kitty", "row": 1, "col": 2, "rows": 2, "cols": 4, "bytes": "145"});
    assert!(serde_json::from_value::<GraphicsPlaceholder>(base.clone()).is_ok());
    let numeric = with(base.clone(), "image_id", json!(4));
    assert!(serde_json::from_value::<GraphicsPlaceholder>(numeric).is_err());
    let payload = with(base, "data_b64", json!("AAAA"));
    assert!(serde_json::from_value::<GraphicsPlaceholder>(payload).is_err());
}

#[test]
fn include_images_defaults_to_false_and_accepts_booleans() -> TestResult {
    let absent: TerminalScreenParams = decode_params(&screen_request(None)?)?;
    let on: TerminalScreenParams = decode_params(&screen_request(Some(json!(true)))?)?;
    let off: TerminalScreenParams = decode_params(&screen_request(Some(json!(false)))?)?;
    assert_eq!(
        (absent.include_images, on.include_images, off.include_images),
        (false, true, false)
    );
    Ok(())
}

#[test]
fn a_non_boolean_include_images_is_invalid_params_at_that_field() -> TestResult {
    for flag in [
        json!("yes"),
        json!("true"),
        json!(1),
        Value::Null,
        json!([true]),
    ] {
        let refused = decode_params::<TerminalScreenParams>(&screen_request(Some(flag.clone()))?)
            .err()
            .ok_or_else(|| format!("{flag} must not decode as a boolean"))?;
        assert_eq!(
            (refused.kind(), refused.field_path()),
            ("invalid_params", "params.include_images"),
            "{flag}"
        );
    }
    Ok(())
}

#[test]
fn a_screen_result_without_images_omits_the_field_and_one_with_them_round_trips() -> TestResult {
    let result = json!({
        "revision": "27",
        "output_offset": "169",
        "lines": ["", "ab"],
        "cursor": {"row": 4, "col": 11, "visible": true},
        "modes": {"enabled": [], "kitty_keyboard": 0, "revision": "0"},
        "title": null,
        "truncated": false
    });
    let plain: TerminalScreenResult = serde_json::from_value(result.clone())?;
    assert_eq!(plain.images, None);
    assert_eq!(serde_json::to_value(&plain)?.get("images"), None);
    let result = with(
        result,
        "images",
        json!([{"image_id": "4", "format": "kitty", "row": 1, "col": 2, "rows": 2, "cols": 4, "bytes": "145"}]),
    );
    let carried: TerminalScreenResult = serde_json::from_value(result.clone())?;
    assert_eq!(
        carried.images.as_deref().map(<[GraphicsPlaceholder]>::len),
        Some(1)
    );
    assert_eq!(serde_json::to_value(&carried)?, result);
    let indexed = with(
        result,
        "image_index",
        json!([{"image_id": "4", "format": "kitty", "bytes": "145", "placement": {"buffer": "primary", "row": 1, "col": 2, "rows": 2, "cols": 4}}]),
    );
    let full: TerminalScreenResult = serde_json::from_value(indexed.clone())?;
    assert_eq!(
        full.image_index.as_deref().map(<[GraphicsIndexEntry]>::len),
        Some(1)
    );
    assert_eq!(serde_json::to_value(&full)?, indexed);
    Ok(())
}

#[test]
fn an_index_entry_names_its_screen_or_null_and_never_a_payload() -> TestResult {
    let entries = vec![
        GraphicsIndexEntry::new(
            (GraphicsImageId::new(0), GraphicsProtocol::Kitty),
            ByteCount::new(17),
            None,
        ),
        GraphicsIndexEntry::new(
            (GraphicsImageId::new(u64::MAX), GraphicsProtocol::Iterm2),
            ByteCount::new(9),
            Some(GraphicsIndexPlacement::new(
                GraphicsBuffer::Alternate,
                PlaceholderCells::new((4, 3), (2, 5)),
            )),
        ),
    ];
    let encoded = serde_json::to_value(&entries)?;
    assert_eq!(
        encoded,
        json!([
            {"image_id": "0", "format": "kitty", "bytes": "17", "placement": null},
            {"image_id": "18446744073709551615", "format": "iterm2", "bytes": "9",
             "placement": {"buffer": "alternate", "row": 4, "col": 3, "rows": 2, "cols": 5}}
        ])
    );
    assert_eq!(
        serde_json::from_value::<Vec<GraphicsIndexEntry>>(encoded)?,
        entries
    );
    let payload = json!({"image_id": "0", "format": "kitty", "bytes": "17", "placement": null, "data_b64": "AA=="});
    assert!(serde_json::from_value::<GraphicsIndexEntry>(payload).is_err());
    Ok(())
}
