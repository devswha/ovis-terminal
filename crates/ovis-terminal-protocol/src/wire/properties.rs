use std::fmt::Debug;

use proptest::prelude::any;
use proptest::test_runner::TestCaseError;
use proptest::{collection, prop_assert, prop_assert_eq, proptest};

use super::{Channel, Frame, FrameDecoder, FrameError, FrameKind, MAX_FRAME_PAYLOAD, WireOffset};

fn test_error(error: impl Debug) -> TestCaseError {
    TestCaseError::fail(format!("{error:?}"))
}

#[test]
fn partial_header_resumes_after_each_byte_position() -> Result<(), TestCaseError> {
    let frame =
        Frame::new(Channel::new(7), FrameKind::Data, vec![0, 0xff, b'x']).map_err(test_error)?;
    let wire = frame.encode().map_err(test_error)?;
    for split in 1..=super::FRAME_HEADER_LEN {
        let mut decoder = FrameDecoder::new();
        let header = wire
            .get(..split)
            .ok_or_else(|| TestCaseError::fail("header prefix was outside the encoded frame"))?;
        let first = decoder.push(header).map_err(test_error)?;
        prop_assert!(first.is_empty());
        let payload = wire
            .get(split..)
            .ok_or_else(|| TestCaseError::fail("header suffix was outside the encoded frame"))?;
        let resumed = decoder.push(payload).map_err(test_error)?;
        decoder.finish().map_err(test_error)?;
        prop_assert_eq!(resumed, vec![frame.clone()]);
    }
    Ok(())
}

proptest! {
    #[test]
    fn generated_frame_sequences_are_split_invariant(
        entries in collection::vec(
            (
                any::<u32>(),
                0_u8..=2,
                collection::vec(any::<u8>(), 0..5),
            ),
            0..=4,
        ),
        cut_mask in any::<u64>(),
    ) {
        let mut expected = Vec::new();
        let mut wire = Vec::new();
        for (channel, kind_code, payload) in entries {
            let kind = match kind_code {
                0 => FrameKind::Data,
                1 => FrameKind::Credit,
                2 => FrameKind::Close,
                _ => return Err(TestCaseError::fail("strategy generated an unknown kind")),
            };
            let frame = Frame::new(Channel::new(channel), kind, payload).map_err(test_error)?;
            wire.extend_from_slice(&frame.encode().map_err(test_error)?);
            expected.push(frame);
        }

        let mut decoder = FrameDecoder::new();
        let mut records = Vec::new();
        let mut start = 0_usize;
        for boundary in 1..wire.len() {
            let bit = boundary.checked_sub(1).ok_or_else(|| {
                TestCaseError::fail("generated split boundary was not positive")
            })?;
            let bit = u32::try_from(bit).map_err(test_error)?;
            if cut_mask & (1_u64 << bit) != 0 {
                let bytes = wire.get(start..boundary).ok_or_else(|| {
                    TestCaseError::fail("generated split was outside the wire sequence")
                })?;
                records.extend(decoder.push(bytes).map_err(test_error)?);
                start = boundary;
            }
        }
        let bytes = wire.get(start..).ok_or_else(|| {
            TestCaseError::fail("generated final segment was outside the wire sequence")
        })?;
        records.extend(decoder.push(bytes).map_err(test_error)?);
        decoder.finish().map_err(test_error)?;
        prop_assert_eq!(records, expected);
    }

    #[test]
    fn arbitrary_payload_bytes_round_trip(
        channel in any::<u32>(),
        kind_code in 0_u8..=2,
        payload in collection::vec(any::<u8>(), 0..257),
    ) {
        let kind = match kind_code {
            0 => FrameKind::Data,
            1 => FrameKind::Credit,
            2 => FrameKind::Close,
            _ => return Err(TestCaseError::fail("strategy generated an unknown kind")),
        };
        let frame = Frame::new(Channel::new(channel), kind, payload).map_err(test_error)?;
        let wire = frame.encode().map_err(test_error)?;
        let mut decoder = FrameDecoder::new();
        let decoded = decoder.push(&wire).map_err(test_error)?;
        decoder.finish().map_err(test_error)?;
        prop_assert_eq!(decoded, vec![frame]);
    }

    #[test]
    fn arbitrary_oversized_lengths_return_frame_too_large(
        channel in any::<u32>(),
        declared in 16_777_217_u32..=u32::MAX,
    ) {
        let mut header = Vec::with_capacity(super::FRAME_HEADER_LEN);
        header.extend_from_slice(&channel.to_le_bytes());
        header.push(0);
        header.extend_from_slice(&declared.to_le_bytes());
        let mut decoder = FrameDecoder::new();
        let Err(error) = decoder.push(&header) else {
            return Err(TestCaseError::fail("oversized frame header was accepted"));
        };
        prop_assert_eq!(error.kind(), "frame_too_large");
        let declared_size = usize::try_from(declared).map_err(test_error)?;
        let FrameError::FrameTooLarge {
            length,
            limit,
            offset,
        } = error
        else {
            return Err(TestCaseError::fail("oversized frame returned a different error kind"));
        };
        prop_assert_eq!(length, declared_size);
        prop_assert_eq!(limit, MAX_FRAME_PAYLOAD);
        prop_assert_eq!(offset, WireOffset::ZERO);
        prop_assert_eq!(decoder.buffered_len(), super::FRAME_HEADER_LEN);
    }
}
