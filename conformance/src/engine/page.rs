//! Reading a logical result through bounded `terminal.output` pages.

use base64::Engine as _;
use ovis_terminal_protocol::wire::MAX_FRAME_PAYLOAD;
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::conn::Wait;
use super::exchange::RECORD_BOUND_MS;
use super::session::Session;
use crate::error::RunnerError;
use crate::ids::Decimal;
use crate::scenario::matcher::Assertion;
use crate::scenario::model::{Pages, Split};

impl Session<'_> {
    /// Follows `next_offset` until an empty page, checking offsets, frame sizes and the digest.
    pub(crate) async fn pages(&mut self, pages: &Pages) -> Result<(), RunnerError> {
        let full_pages = pages
            .total_bytes
            .0
            .checked_div(pages.page_bytes.0)
            .unwrap_or(0);
        let most_pages = full_pages.saturating_add(2);
        let mut digest = Sha256::new();
        let mut offset = 0_u64;
        let mut largest_record = 0_usize;
        let mut count = 0_u64;
        loop {
            if count > most_pages {
                return Err(RunnerError::AssertionFailed {
                    check: "pages/terminates".to_owned(),
                });
            }
            let id = format!("page-{count}");
            let request = json!({"jsonrpc": "2.0", "id": id, "method": "terminal.output", "params": {
                "target": pages.target, "from_offset": offset.to_string(),
                "limit_bytes": pages.page_bytes,
            }});
            let conn = self.conn(&pages.conn)?;
            let bytes = conn.encode(&request)?;
            conn.write(&bytes, &Split::Whole).await?;
            let reply = conn
                .next(Wait::within(RECORD_BOUND_MS, "a page"))
                .await?
                .ok_or(RunnerError::Closed {
                    waiting_for: "a page",
                })?;
            largest_record = largest_record.max(conn.last_record_bytes());
            let data = reply
                .pointer("/result/data_b64")
                .and_then(Value::as_str)
                .and_then(|text| base64::engine::general_purpose::STANDARD.decode(text).ok())
                .ok_or(RunnerError::AssertionFailed {
                    check: format!("reply[{id}]/result/data_b64"),
                })?;
            let next = offset.saturating_add(Decimal::from_usize(data.len()).0);
            self.assert(vec![
                Assertion::equal(
                    format!("reply[{id}]/id"),
                    json!(id),
                    reply.get("id").cloned().unwrap_or(Value::Null),
                ),
                Assertion::equal(
                    format!("reply[{id}]/result/from_offset"),
                    json!(offset.to_string()),
                    reply
                        .pointer("/result/from_offset")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                Assertion::equal(
                    format!("reply[{id}]/result/next_offset"),
                    json!(next.to_string()),
                    reply
                        .pointer("/result/next_offset")
                        .cloned()
                        .unwrap_or(Value::Null),
                ),
                Assertion::equal(
                    format!("reply[{id}]/result/gap"),
                    json!(false),
                    reply.pointer("/result/gap").cloned().unwrap_or(Value::Null),
                ),
            ])?;
            digest.update(&data);
            count = count.saturating_add(1);
            if data.is_empty() {
                break;
            }
            offset = next;
        }
        let sha256 = crate::evidence::hex(&digest.finalize());
        self.assert(vec![
            Assertion::equal(
                "pages/total_bytes".to_owned(),
                json!(pages.total_bytes),
                json!(offset.to_string()),
            ),
            Assertion::equal(
                "pages/sha256".to_owned(),
                json!(pages.sha256),
                json!(sha256),
            ),
            Assertion::equal(
                "pages/largest_record_within_frame".to_owned(),
                json!(true),
                json!(largest_record <= MAX_FRAME_PAYLOAD),
            ),
            Assertion::equal("pages/multi_page".to_owned(), json!(true), json!(count > 2)),
        ])
    }
}
