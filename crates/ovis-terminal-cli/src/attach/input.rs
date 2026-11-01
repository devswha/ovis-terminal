//! Typed bytes and the one key the client keeps for itself.
//!
//! There is no prefix key and no client mode: Ctrl-\ detaches, and every other byte,
//! including escape sequences this client does not recognise and bytes that are not
//! UTF-8, is forwarded unchanged through `input.raw`, whose refusal is reported as the
//! server's typed error instead of being dropped.

/// The detach key: Ctrl-\ (FS, 0x1c), read as a byte while the terminal is raw.
pub(super) const DETACH_KEY: u8 = 0x1c;

/// The bytes typed before the detach key, and whether the key was typed. Bytes after
/// the key are never forwarded: the client is detaching.
pub(super) fn split_detach(bytes: &[u8]) -> (&[u8], bool) {
    bytes
        .iter()
        .position(|byte| *byte == DETACH_KEY)
        .map_or((bytes, false), |at| {
            (bytes.get(..at).unwrap_or_default(), true)
        })
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{DETACH_KEY, split_detach};

    #[test]
    fn unknown_sequences_and_non_utf8_bytes_are_forwarded_unchanged() {
        let typed = b"\x1b[99;99~\x1bO\x7f\xff\xfe\x00\x03\x1a\x1b[200~";
        assert_eq!(split_detach(typed), (&typed[..], false));
    }

    #[test]
    fn the_detach_key_ends_forwarding_mid_chunk() {
        assert_eq!(
            split_detach(b"ls\r\x1crm -rf"),
            (&b"ls\r"[..], true),
            "bytes after Ctrl-\\ must not reach the program"
        );
        assert_eq!(split_detach(b"\x1c"), (&b""[..], true));
    }

    proptest! {
        #[test]
        fn forwarded_bytes_are_exactly_the_prefix_before_the_first_detach_key(
            bytes in proptest::collection::vec(any::<u8>(), 0..256)
        ) {
            let (forwarded, detach) = split_detach(&bytes);
            let first = bytes.iter().position(|byte| *byte == DETACH_KEY);
            prop_assert_eq!(detach, first.is_some());
            prop_assert_eq!(forwarded, bytes.get(..first.unwrap_or(bytes.len())).unwrap_or_default());
            prop_assert!(!forwarded.contains(&DETACH_KEY));
        }
    }
}
