use proptest::prelude::*;

use super::{FrameDecoder, Inbound, Opcode, close, encode};

const KEY: [u8; 4] = [0x37, 0xfa, 0x21, 0x3d];

/// A client frame: `first` is the FIN/RSV/opcode byte; the payload is masked with KEY.
fn client(first: u8, payload: &[u8]) -> Vec<u8> {
    let mut frame = vec![first];
    match payload.len() {
        short @ 0..126 => frame.push(0x80 | u8::try_from(short).unwrap_or(0)),
        medium @ 126..=65_535 => {
            frame.push(0x80 | 0x7E);
            frame.extend_from_slice(&u16::try_from(medium).unwrap_or(0).to_be_bytes());
        }
        long => {
            frame.push(0x80 | 0x7F);
            frame.extend_from_slice(&u64::try_from(long).unwrap_or(0).to_be_bytes());
        }
    }
    frame.extend_from_slice(&KEY);
    frame.extend(payload.iter().zip(KEY.iter().cycle()).map(|(b, k)| b ^ k));
    frame
}

fn decode_all(bytes: &[u8], max: usize) -> Result<Vec<Inbound>, &'static str> {
    let mut decoder = FrameDecoder::new(max);
    decoder.feed(bytes);
    let mut out = Vec::new();
    while let Some(frame) = decoder.next().map_err(|error| error.kind())? {
        out.push(frame);
    }
    Ok(out)
}

#[test]
fn rfc_6455_masked_sample_decodes_and_server_frames_match_the_rfc_bytes() {
    // RFC 6455 section 5.7: a masked "Hello" (as binary here; text is refused).
    let sample = [
        0x82, 0x85, 0x37, 0xfa, 0x21, 0x3d, 0x7f, 0x9f, 0x4d, 0x51, 0x58,
    ];
    assert_eq!(
        decode_all(&sample, 16),
        Ok(vec![Inbound::Data(b"Hello".to_vec())])
    );
    assert_eq!(encode(Opcode::Binary, b"Hello"), b"\x82\x05Hello");
    let medium = encode(Opcode::Binary, &[0; 256]);
    assert_eq!(medium.get(..4), Some(&[0x82, 126, 0x01, 0x00][..]));
    let long = encode(Opcode::Binary, &vec![0; 65_536]);
    assert_eq!(
        long.get(..10),
        Some(&[0x82, 127, 0, 0, 0, 0, 0, 1, 0, 0][..])
    );
    assert_eq!(
        close(1003, "invalid_ws_message").get(..4),
        Some(&b"\x88\x14\x03\xeb"[..])
    );
}

#[test]
fn fragments_pings_and_close_are_decoded_in_order() {
    let bytes = [
        client(0x02, b"{\"a\""),
        client(0x89, b"beat"),
        client(0x80, b":1}\n"),
        client(0x8A, b""),
        client(0x88, &1000_u16.to_be_bytes()),
    ]
    .concat();
    let expected = vec![
        Inbound::Data(b"{\"a\"".to_vec()),
        Inbound::Ping(b"beat".to_vec()),
        Inbound::Data(b":1}\n".to_vec()),
        Inbound::Pong,
        Inbound::Close(Some(1000)),
    ];
    assert_eq!(decode_all(&bytes, 64), Ok(expected));
}

#[test]
fn violations_are_typed() {
    let unmasked = [0x82, 0x01, b'x'];
    let cases: [(Vec<u8>, &str); 9] = [
        (client(0x81, b"text"), "invalid_ws_message"),
        (unmasked.to_vec(), "ws_protocol"),
        (client(0xC2, b"x"), "ws_protocol"),
        (client(0x80, b"x"), "ws_protocol"),
        (
            [client(0x02, b"a"), client(0x82, b"b")].concat(),
            "ws_protocol",
        ),
        (client(0x09, b"x"), "ws_protocol"),
        (client(0x89, &[0; 126]), "ws_protocol"),
        (client(0x88, &[3]), "ws_protocol"),
        (client(0x83, b""), "ws_protocol"),
    ];
    for (bytes, kind) in cases {
        let decoded = decode_all(&bytes, 1024);
        assert!(
            matches!(decoded, Err(found) if found == kind),
            "{bytes:02x?}: {decoded:?}"
        );
    }
    let oversized = client(0x82, &[0; 17]);
    assert_eq!(decode_all(&oversized, 16), Err("message_too_large"));
    // A declared length is refused before its payload is buffered.
    let declared = [
        0x82, 0xFF, 0x7F, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 0xFF, 1, 2, 3, 4,
    ];
    assert_eq!(decode_all(&declared, 16), Err("message_too_large"));
}

proptest! {
    #[test]
    fn frames_decode_identically_under_every_split(
        payloads in proptest::collection::vec(proptest::collection::vec(any::<u8>(), 0..300), 1..6),
        cuts in proptest::collection::btree_set(0_usize..2_000, 0..8),
    ) {
        let bytes: Vec<u8> = payloads.iter().flat_map(|payload| client(0x82, payload)).collect();
        let mut decoder = FrameDecoder::new(1024);
        let mut decoded = Vec::new();
        let mut start = 0;
        let ends = cuts.into_iter().filter(|cut| *cut < bytes.len()).chain([bytes.len()]);
        for end in ends {
            decoder.feed(bytes.get(start..end).unwrap_or_default());
            start = end;
            while let Some(frame) = decoder.next().map_err(|error| TestCaseError::fail(error.kind()))? {
                decoded.push(frame);
            }
        }
        let expected: Vec<Inbound> = payloads.into_iter().map(Inbound::Data).collect();
        prop_assert_eq!(decoded, expected);
    }
}
