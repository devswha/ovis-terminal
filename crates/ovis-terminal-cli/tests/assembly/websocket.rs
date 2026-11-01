//! `serve --websocket-port 0`: hello over an RFC 6455 binary message answers like the socket.

use std::collections::BTreeSet;
use std::io::{BufRead as _, BufReader, Write as _};
use std::net::TcpStream;
use std::time::Duration;

use serde_json::{Value, json};

use super::support::{Served, TestResult};

/// One masked binary frame carrying `payload` (client frames must be masked).
fn binary_frame(payload: &[u8]) -> TestResult<Vec<u8>> {
    let mask = [0x5a_u8, 0x17, 0xc3, 0x09];
    let mut frame = vec![0x82];
    match u16::try_from(payload.len()) {
        Ok(length) if length < 126 => frame.push(0x80 | u8::try_from(length)?),
        Ok(length) => {
            frame.push(0x80 | 0x7e);
            frame.extend(length.to_be_bytes());
        }
        Err(_) => return Err("payload too large for this test client".into()),
    }
    frame.extend(mask);
    frame.extend(
        payload
            .iter()
            .zip(mask.iter().cycle())
            .map(|(byte, key)| byte ^ key),
    );
    Ok(frame)
}

/// Reads one unmasked server frame: its opcode and payload.
fn read_frame(stream: &mut impl std::io::Read) -> TestResult<(u8, Vec<u8>)> {
    let mut head = [0_u8; 2];
    stream.read_exact(&mut head)?;
    let length = match head[1] & 0x7f {
        126 => {
            let mut wide = [0_u8; 2];
            stream.read_exact(&mut wide)?;
            u64::from(u16::from_be_bytes(wide))
        }
        127 => {
            let mut wide = [0_u8; 8];
            stream.read_exact(&mut wide)?;
            u64::from_be_bytes(wide)
        }
        short => u64::from(short),
    };
    let mut payload = vec![0_u8; usize::try_from(length)?];
    stream.read_exact(&mut payload)?;
    Ok((head[0] & 0x0f, payload))
}

#[test]
fn hello_over_websocket_advertises_both_transports_and_the_same_methods() -> TestResult {
    let served = Served::start_with(&["--websocket-port", "0"])?;
    let address = served
        .websocket()
        .ok_or("ready line names the WebSocket listener")?;
    let mut stream = TcpStream::connect(address)?;
    stream.set_read_timeout(Some(Duration::from_secs(10)))?;
    write!(
        stream,
        "GET / HTTP/1.1\r\nHost: {address}\r\nUpgrade: websocket\r\nConnection: Upgrade\r\nSec-WebSocket-Key: dGhlIHNhbXBsZSBub25jZQ==\r\nSec-WebSocket-Version: 13\r\n\r\n"
    )?;
    let mut reader = BufReader::new(stream.try_clone()?);
    let mut status = String::new();
    reader.read_line(&mut status)?;
    assert!(status.starts_with("HTTP/1.1 101"), "{status}");
    loop {
        let mut header = String::new();
        reader.read_line(&mut header)?;
        if header == "\r\n" {
            break;
        }
    }
    let hello = json!({"jsonrpc": "2.0", "id": "h", "method": "hello", "params": {
        "generation": "1", "client": {"name": "t31", "version": "0"}, "credential": served.bearer(),
    }});
    stream.write_all(&binary_frame(format!("{hello}\n").as_bytes())?)?;
    let (opcode, payload) = read_frame(&mut reader)?;
    assert_eq!(opcode, 2, "every server record is one binary message");
    let reply: Value = serde_json::from_slice(&payload)?;
    assert_eq!(
        reply.pointer("/result/endpoint_kind"),
        Some(&json!("websocket"))
    );
    assert_eq!(
        reply.pointer("/result/capabilities/transports"),
        Some(&json!(["unix", "websocket"]))
    );
    let methods: BTreeSet<String> = reply
        .pointer("/result/methods")
        .and_then(Value::as_array)
        .ok_or_else(|| reply.to_string())?
        .iter()
        .filter_map(Value::as_str)
        .map(str::to_owned)
        .collect();
    assert_eq!(methods, served.advertised()?);
    drop((reader, stream));
    served.stop()?;
    Ok(())
}
