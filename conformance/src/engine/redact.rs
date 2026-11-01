//! Redacting credentials from recorded evidence.

use super::conn::REDACTED;

/// Replaces every `ovt1.c<n>.<64 hex>` bearer in `text`, such as one a rotation handed out,
/// so no credential reaches the evidence.
pub(crate) fn scrub(text: &str) -> String {
    const PREFIX: &str = "ovt1.c";
    let mut out = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(start) = rest.find(PREFIX) {
        let (before, candidate) = rest.split_at(start);
        out.push_str(before);
        let body = candidate.get(PREFIX.len()..).unwrap_or_default();
        let digits = body.bytes().take_while(u8::is_ascii_digit).count();
        let secret = body.get(digits..).and_then(|tail| tail.strip_prefix('.'));
        let hex = secret.map_or(0, |tail| {
            tail.bytes()
                .take_while(|byte| byte.is_ascii_digit() || (b'a'..=b'f').contains(byte))
                .count()
        });
        if digits > 0 && hex == 64 {
            out.push_str(REDACTED);
            let length = PREFIX
                .len()
                .saturating_add(digits)
                .saturating_add(1)
                .saturating_add(hex);
            rest = candidate.get(length..).unwrap_or_default();
        } else {
            out.push_str(PREFIX);
            rest = body;
        }
    }
    out.push_str(rest);
    out
}

#[cfg(test)]
mod tests {
    #[test]
    fn bearer_shaped_tokens_are_scrubbed_and_lookalikes_kept() {
        let secret = "0123456789abcdef".repeat(4);
        let text = format!(
            r#"{{"bearer":"ovt1.c12.{secret}","short":"ovt1.c3.abc","bare":"ovt1.c.{secret}"}}"#
        );

        let scrubbed = super::scrub(&text);

        assert_eq!(
            scrubbed,
            format!(
                r#"{{"bearer":"<redacted-credential>","short":"ovt1.c3.abc","bare":"ovt1.c.{secret}"}}"#
            )
        );
    }
}
