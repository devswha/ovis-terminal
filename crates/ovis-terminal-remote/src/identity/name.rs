//! Machine names parsed once at the boundary.

use std::fmt;

use ovis_terminal_protocol::schema::ids::MachineName;

use super::IdentityError;

const MAX_NAME_BYTES: usize = 63;

/// A valid machine name: 1 to 63 bytes of `a-z`, `0-9`, `-`, `_` and `.`, starting with a
/// letter or digit. It never contains `/` (the address separator) or `:` (the id separator).
#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub struct Name(MachineName);

impl Name {
    /// The name as sent on the wire.
    #[must_use]
    pub const fn as_machine_name(&self) -> &MachineName {
        &self.0
    }
}

impl TryFrom<&MachineName> for Name {
    type Error = IdentityError;

    fn try_from(raw: &MachineName) -> Result<Self, Self::Error> {
        let text = raw.as_str();
        let allowed =
            |byte: &u8| byte.is_ascii_lowercase() || byte.is_ascii_digit() || b"-_.".contains(byte);
        let starts_alphanumeric = text
            .bytes()
            .next()
            .is_some_and(|first| first.is_ascii_lowercase() || first.is_ascii_digit());
        if starts_alphanumeric && text.len() <= MAX_NAME_BYTES && text.bytes().all(|b| allowed(&b))
        {
            Ok(Self(raw.clone()))
        } else {
            Err(IdentityError::InvalidName)
        }
    }
}

impl fmt::Display for Name {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(self.0.as_str())
    }
}

#[cfg(test)]
mod tests {
    use ovis_terminal_protocol::schema::ids::MachineName;

    use super::Name;

    fn parse(text: &str) -> Result<Name, &'static str> {
        Name::try_from(&MachineName::new(text.to_owned())).map_err(|error| error.kind())
    }

    #[test]
    fn plain_names_parse() {
        for text in ["box", "build-01", "a", "eu.west_2", &"x".repeat(63)] {
            assert_eq!(
                parse(text).map(|name| name.to_string()),
                Ok(text.to_owned())
            );
        }
    }

    #[test]
    fn shell_metacharacters_addresses_and_ids_are_invalid_names() {
        let hostile = [
            "",
            "-box",
            ".box",
            "Box",
            "box/t7",
            "machine:1",
            "box;rm -rf ~",
            "$(id)",
            "`id`",
            "box name",
            "box\n",
            "box|cat",
            "b\u{e9}",
            &"x".repeat(64),
        ];
        for text in hostile {
            assert_eq!(parse(text), Err("invalid_name"), "{text:?}");
        }
    }
}
