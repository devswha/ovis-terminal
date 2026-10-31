//! The bearer a local runtime presents to a remote one; never printed.

use std::fmt;

/// Bearer text issued by a remote runtime for this runtime to present in `hello`.
#[derive(Clone, PartialEq, Eq)]
pub struct Credential(String);

impl Credential {
    /// Wraps bearer text received from a caller with the administration grant.
    #[must_use]
    pub const fn new(bearer: String) -> Self {
        Self(bearer)
    }

    /// The bearer text, for the handshake and for keying the boot proof.
    #[must_use]
    pub fn expose(&self) -> &str {
        &self.0
    }
}

impl fmt::Debug for Credential {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credential(<redacted>)")
    }
}

#[cfg(test)]
mod tests {
    use super::Credential;

    #[test]
    fn debug_output_never_contains_the_bearer() {
        let credential = Credential::new("ovt1.7.secret".to_owned());

        assert!(!format!("{credential:?}").contains("secret"));
        assert_eq!(credential.expose(), "ovt1.7.secret");
    }
}
