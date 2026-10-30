//! Public integration tests for framing, continuation, and upgrade behavior.

use ovis_terminal_protocol::wire::{
    Channel, ConnectionDecoder, Continuation, FeedOutcome, Frame, FrameDecoder, FrameError,
    FrameKind, LogicalOffset, LogicalResult, MAX_FRAME_PAYLOAD, WireMessage, WireOffset,
};

fn sample_frames() -> Result<Vec<Frame>, Box<dyn std::error::Error>> {
    Ok(vec![
        Frame::new(
            Channel::CONTROL,
            FrameKind::Data,
            b"{\"id\":\"a\"}".to_vec(),
        )?,
        Frame::new(Channel::new(7), FrameKind::Data, vec![0, 0xff, b'x'])?,
    ])
}

#[test]
fn happy() -> Result<(), Box<dyn std::error::Error>> {
    assert_all_segmentations()?;
    assert_mixed_segmentations()?;
    assert_large_result_chunks()?;
    assert_hello_upgrade_boundary()?;
    Ok(())
}

fn assert_all_segmentations() -> Result<(), Box<dyn std::error::Error>> {
    let segmented_frames = [
        Frame::new(Channel::CONTROL, FrameKind::Data, vec![0])?,
        Frame::new(Channel::new(7), FrameKind::Data, vec![0xff])?,
    ];
    let encoded = segmented_frames
        .iter()
        .map(Frame::encode)
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    let cut_count = encoded
        .len()
        .checked_sub(1)
        .ok_or_else(|| std::io::Error::other("encoded stream has no bytes"))?;
    let pattern_count = 1_u32
        .checked_shl(u32::try_from(cut_count)?)
        .ok_or_else(|| std::io::Error::other("segmentation count overflow"))?;
    for mask in 0_u32..pattern_count {
        let mut frame_decoder = FrameDecoder::new();
        let mut records = Vec::new();
        let mut start = 0;
        for boundary in 1..encoded.len() {
            let bit = u32::try_from(
                boundary
                    .checked_sub(1)
                    .ok_or_else(|| std::io::Error::other("invalid split boundary"))?,
            )?;
            if mask & (1_u32 << bit) != 0 {
                let chunk = encoded
                    .get(start..boundary)
                    .ok_or_else(|| std::io::Error::other("invalid segment"))?;
                records.extend(frame_decoder.push(chunk)?);
                start = boundary;
            }
        }
        let chunk = encoded
            .get(start..)
            .ok_or_else(|| std::io::Error::other("invalid final segment"))?;
        records.extend(frame_decoder.push(chunk)?);
        frame_decoder.finish()?;
        assert_eq!(records, segmented_frames);
    }
    Ok(())
}

fn assert_mixed_segmentations() -> Result<(), Box<dyn std::error::Error>> {
    let expected_frames = sample_frames()?;
    let encoded = expected_frames
        .iter()
        .map(Frame::encode)
        .collect::<Result<Vec<_>, _>>()?
        .concat();
    for split in 1..encoded.len() {
        let mut frame_decoder = FrameDecoder::new();
        let prefix = encoded
            .get(..split)
            .ok_or_else(|| std::io::Error::other("invalid prefix"))?;
        let suffix = encoded
            .get(split..)
            .ok_or_else(|| std::io::Error::other("invalid suffix"))?;
        let mut records = frame_decoder.push(prefix)?;
        records.extend(frame_decoder.push(suffix)?);
        frame_decoder.finish()?;
        assert_eq!(records, expected_frames);
    }
    for chunk_size in 1..=encoded.len() {
        let mut frame_decoder = FrameDecoder::new();
        let mut records = Vec::new();
        for chunk in encoded.chunks(chunk_size) {
            records.extend(frame_decoder.push(chunk)?);
        }
        frame_decoder.finish()?;
        assert_eq!(records, expected_frames);
    }
    Ok(())
}

fn assert_large_result_chunks() -> Result<(), Box<dyn std::error::Error>> {
    let logical = vec![0x5a; 64 * 1024 * 1024];
    let mut total = 0_usize;
    for chunk in LogicalResult::new(&logical)?.chunks() {
        let chunk = chunk?;
        assert!(chunk.payload.len() <= MAX_FRAME_PAYLOAD);
        assert_eq!(chunk.metadata.total.get(), 64 * 1024 * 1024);
        assert_eq!(chunk.metadata.offset.get(), u64::try_from(total)?);
        assert!(chunk.payload.iter().all(|byte| *byte == 0x5a));
        assert!(!chunk.metadata.truncated);
        let next = total
            .checked_add(chunk.payload.len())
            .ok_or_else(|| std::io::Error::other("byte count overflow"))?;
        let continuation = if next < logical.len() {
            Some(Continuation::new(LogicalOffset::from_usize(next)?))
        } else {
            None
        };
        assert_eq!(chunk.metadata.continuation, continuation);
        total = next;
    }
    assert_eq!(total, logical.len());
    Ok(())
}

fn assert_hello_upgrade_boundary() -> Result<(), Box<dyn std::error::Error>> {
    let stream_frame = Frame::new(Channel::new(7), FrameKind::Data, vec![0, 0xff, b'x'])?;
    let wire_bytes = stream_frame.encode()?;
    assert_eq!(wire_bytes, [7, 0, 0, 0, 0, 3, 0, 0, 0, 0, 0xff, b'x']);
    let hello = b"{\"method\":\"hello\",\"params\":{\"framed\":true}}\n";
    let mut coalesced = hello.to_vec();
    coalesced.extend_from_slice(&wire_bytes);
    let mut connection = ConnectionDecoder::new();
    assert_eq!(
        connection.feed(&coalesced)?,
        FeedOutcome::Accepted {
            consumed: coalesced.len()
        }
    );
    assert!(matches!(
        connection.next_message()?,
        Some(WireMessage::ControlLine(_))
    ));
    connection.upgrade_to_framed()?;
    assert!(matches!(
        connection.next_message()?,
        Some(WireMessage::Frame(frame)) if frame == stream_frame
    ));
    Ok(())
}

#[test]
fn failure() -> Result<(), Box<dyn std::error::Error>> {
    let too_large = [0x00, 0x00, 0x00, 0x00, 0x00, 0x01, 0x00, 0x00, 0x01];
    let expected_length = MAX_FRAME_PAYLOAD
        .checked_add(1)
        .ok_or_else(|| std::io::Error::other("frame limit overflow"))?;
    let mut decoder = FrameDecoder::new();
    let oversize_error = decoder
        .push(&too_large)
        .err()
        .ok_or_else(|| std::io::Error::other("oversized frame was accepted"))?;
    assert_eq!(oversize_error.kind(), "frame_too_large");
    assert_ne!(oversize_error.hint(), "");
    assert!(matches!(
        oversize_error,
        FrameError::FrameTooLarge {
            length,
            limit,
            offset
        } if length == expected_length && limit == MAX_FRAME_PAYLOAD && offset == WireOffset::ZERO
    ));
    assert_eq!(decoder.buffered_len(), 9);
    assert_eq!(decoder.byte_offset().get(), 9);

    let mut truncated = FrameDecoder::new();
    truncated.push(&[0, 0, 0])?;
    let header_error = truncated
        .finish()
        .err()
        .ok_or_else(|| std::io::Error::other("truncated header was accepted"))?;
    assert_eq!(header_error.kind(), "truncated_frame");
    assert!(matches!(
        header_error,
        FrameError::TruncatedHeader {
            offset,
            actual: 3
        } if offset == WireOffset::ZERO
    ));

    let mut payload = FrameDecoder::new();
    payload.push(&[0, 0, 0, 0, 0, 2, 0, 0, 0, b'a'])?;
    let payload_error = payload
        .finish()
        .err()
        .ok_or_else(|| std::io::Error::other("truncated payload was accepted"))?;
    assert!(matches!(
        payload_error,
        FrameError::TruncatedPayload {
            offset,
            expected: 2,
            actual: 1
        } if offset == WireOffset::ZERO
    ));

    let mut invalid_control = ConnectionDecoder::new();
    invalid_control.feed(&[0xff, b'\n'])?;
    let utf8_error = invalid_control
        .next_message()
        .err()
        .ok_or_else(|| std::io::Error::other("invalid control UTF-8 was accepted"))?;
    assert_eq!(utf8_error.kind(), "invalid_control_utf8");
    assert!(std::error::Error::source(&utf8_error).is_some());
    Ok(())
}
