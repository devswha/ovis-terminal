//! Browser origins allowed to open a WebSocket, checked before the upgrade completes.
//!
//! Browsers attach `Origin` to every WebSocket upgrade, so a page from another site can
//! be told apart from the configured clients. A request without `Origin` is a
//! non-browser client; it still has to authenticate with `hello.credential`.

use std::collections::BTreeSet;
use std::str::FromStr;

use super::WsError;

/// A browser origin, `scheme://host[:port]` over http or https, stored lowercase.
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Origin(String);

impl FromStr for Origin {
    type Err = WsError;

    fn from_str(text: &str) -> Result<Self, Self::Err> {
        let lower = text.to_ascii_lowercase();
        let authority = lower
            .strip_prefix("http://")
            .or_else(|| lower.strip_prefix("https://"))
            .ok_or(WsError::InvalidOrigin)?;
        let (host, port, host_bytes): (_, _, &[u8]) = match authority.strip_prefix('[') {
            Some(bracketed) => {
                let (host, tail) = bracketed.split_once(']').ok_or(WsError::InvalidOrigin)?;
                let port = match tail {
                    "" => None,
                    _ => Some(tail.strip_prefix(':').ok_or(WsError::InvalidOrigin)?),
                };
                (host, port, b":.")
            }
            None => match authority.split_once(':') {
                Some((host, port)) => (host, Some(port), b".-"),
                None => (authority, None, b".-"),
            },
        };
        let host_ok = !host.is_empty()
            && host
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || host_bytes.contains(&byte));
        let port_ok = port.is_none_or(|port| port.parse::<u16>().is_ok_and(|port| port != 0));
        if host_ok && port_ok {
            Ok(Self(lower))
        } else {
            Err(WsError::InvalidOrigin)
        }
    }
}

impl Origin {
    /// The serialized origin.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }
}

/// The browser origins a listener accepts; empty admits non-browser clients only.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct AllowedOrigins {
    origins: BTreeSet<Origin>,
}

impl AllowedOrigins {
    /// Allows exactly `origins`.
    #[must_use]
    pub fn new(origins: impl IntoIterator<Item = Origin>) -> Self {
        Self {
            origins: origins.into_iter().collect(),
        }
    }

    /// Admits an upgrade presenting no `Origin` header or exactly one configured origin;
    /// refuses a repeated, unreadable or unconfigured one.
    pub(super) fn admit<'a>(
        &self,
        presented: impl IntoIterator<Item = &'a [u8]>,
    ) -> Result<(), WsError> {
        let mut presented = presented.into_iter();
        let origin = match (presented.next(), presented.next()) {
            (None, _) => return Ok(()),
            (Some(origin), None) => origin,
            (Some(_), Some(_)) => return Err(WsError::OriginRejected),
        };
        let configured = std::str::from_utf8(origin)
            .ok()
            .and_then(|text| text.parse::<Origin>().ok())
            .is_some_and(|origin| self.origins.contains(&origin));
        if configured {
            Ok(())
        } else {
            Err(WsError::OriginRejected)
        }
    }
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{AllowedOrigins, Origin};

    type TestResult = Result<(), Box<dyn std::error::Error>>;

    fn upgrade<'a>(origins: &[&'a str]) -> impl Iterator<Item = &'a [u8]> {
        origins.iter().copied().map(str::as_bytes)
    }

    #[test]
    fn origins_parse_only_as_http_scheme_host_and_port() -> TestResult {
        let parsed: Origin = "HTTPS://Ovis.Example:8443".parse()?;
        assert_eq!(parsed.as_str(), "https://ovis.example:8443");
        assert!("http://[::1]:3000".parse::<Origin>().is_ok());
        assert!("http://[::1]".parse::<Origin>().is_ok());
        let refused = [
            "null",
            "file://",
            "https://",
            "ws://a",
            "https://a/path",
            "http://a:0",
            "http://user@a",
            "http://[::1]x",
        ];
        for refused in refused {
            assert!(refused.parse::<Origin>().is_err(), "{refused}");
        }
        Ok(())
    }

    #[test]
    fn only_configured_browser_origins_and_non_browser_clients_are_admitted() -> TestResult {
        let allowed = AllowedOrigins::new(["https://ovis.example".parse()?]);
        assert!(
            allowed.admit(upgrade(&[])).is_ok(),
            "no Origin: not a browser"
        );
        assert!(allowed.admit(upgrade(&["https://OVIS.example"])).is_ok());
        let refused = [
            &["https://evil.example"][..],
            &["null"],
            &["https://ovis.example.evil"],
            &["https://ovis.example", "https://ovis.example"],
        ];
        for origins in refused {
            let kind = allowed
                .admit(upgrade(origins))
                .map_err(|error| error.kind());
            assert_eq!(kind, Err("origin_rejected"), "{origins:?}");
        }
        let invalid_utf8: [&[u8]; 1] = [b"https://ovis.example\xff"];
        assert!(allowed.admit(invalid_utf8).is_err());
        let none = AllowedOrigins::default();
        let kind = none
            .admit(upgrade(&["https://ovis.example"]))
            .map_err(|e| e.kind());
        assert_eq!(
            kind,
            Err("origin_rejected"),
            "browsers are refused by default"
        );
        Ok(())
    }

    proptest! {
        #[test]
        fn a_parsed_origin_round_trips_through_its_text(host in "[a-z][a-z0-9-]{0,20}(\\.[a-z]{2,6})?", port in 1_u16..) {
            let text = format!("https://{host}:{port}");
            let parsed: Origin = text.parse().map_err(|error| TestCaseError::fail(format!("{error}")))?;
            prop_assert_eq!(parsed.as_str(), text.as_str());
            let reparsed: Origin = parsed.as_str().parse().map_err(|error| TestCaseError::fail(format!("{error}")))?;
            prop_assert_eq!(reparsed, parsed);
        }
    }
}
