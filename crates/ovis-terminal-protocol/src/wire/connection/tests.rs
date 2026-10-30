use std::error::Error;

use super::{ConnectionDecoder, FeedOutcome, WireMessage};
use crate::wire::{
    Channel, FRAME_HEADER_LEN, Frame, FrameKind, MAX_FRAME_PAYLOAD, MAX_READY_BYTES,
    MAX_READY_RECORDS,
};

fn upgraded_connection() -> Result<ConnectionDecoder, Box<dyn Error>> {
    let mut connection = ConnectionDecoder::new();
    let hello = b"{\"result\":{\"framed\":true}}\n";
    assert_eq!(
        connection.feed(hello)?,
        FeedOutcome::Accepted {
            consumed: hello.len()
        }
    );
    assert!(matches!(
        connection.next_message()?,
        Some(WireMessage::ControlLine(_))
    ));
    connection.upgrade_to_framed()?;
    Ok(connection)
}

#[test]
fn queued_frames_obey_record_and_byte_limits() -> Result<(), Box<dyn Error>> {
    let mut by_bytes = upgraded_connection()?;
    let large_frame = Frame::new(
        Channel::new(7),
        FrameKind::Data,
        vec![0x5a; MAX_FRAME_PAYLOAD],
    )?
    .encode()?;
    for _ in 0..2 {
        assert_eq!(
            by_bytes.feed(&large_frame)?,
            FeedOutcome::Accepted {
                consumed: large_frame.len()
            }
        );
    }
    let FeedOutcome::NeedDrain { consumed } = by_bytes.feed(&large_frame)? else {
        return Err(std::io::Error::other("ready byte budget did not backpressure").into());
    };
    assert_eq!(consumed, FRAME_HEADER_LEN);
    assert_eq!(by_bytes.queued_records(), 2);
    assert_eq!(by_bytes.queued_bytes(), MAX_READY_BYTES);
    assert!(matches!(
        by_bytes.next_message()?,
        Some(WireMessage::Frame(frame))
            if frame.payload.len() == MAX_FRAME_PAYLOAD
                && frame.payload.iter().all(|byte| *byte == 0x5a)
    ));
    let suffix = large_frame
        .get(consumed..)
        .ok_or_else(|| std::io::Error::other("backpressure consumed beyond the header"))?;
    assert_eq!(
        by_bytes.feed(suffix)?,
        FeedOutcome::Accepted {
            consumed: suffix.len()
        }
    );
    assert_eq!(by_bytes.queued_records(), 2);
    assert_eq!(by_bytes.queued_bytes(), MAX_READY_BYTES);
    drop(by_bytes);
    drop(large_frame);

    let mut by_records = upgraded_connection()?;
    let empty_frame = Frame::new(Channel::new(7), FrameKind::Data, Vec::new())?.encode()?;
    for _ in 0..MAX_READY_RECORDS {
        assert_eq!(
            by_records.feed(&empty_frame)?,
            FeedOutcome::Accepted {
                consumed: empty_frame.len()
            }
        );
    }
    assert_eq!(
        by_records.feed(&empty_frame)?,
        FeedOutcome::NeedDrain { consumed: 0 }
    );
    assert_eq!(by_records.queued_records(), MAX_READY_RECORDS);
    assert_eq!(by_records.queued_bytes(), 0);
    Ok(())
}
