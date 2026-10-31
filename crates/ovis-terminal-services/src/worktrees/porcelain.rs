//! `git worktree list --porcelain -z`: NUL-terminated attributes, an empty attribute
//! ends each record, and unknown attributes are skipped for forward compatibility.

use std::path::PathBuf;

use super::WorktreeError;

/// One checkout as Git lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(super) struct Listed {
    pub path: PathBuf,
    pub branch: Option<String>,
    pub bare: bool,
    pub locked: bool,
    pub prunable: bool,
}

impl Listed {
    fn at(path: &str) -> Self {
        Self {
            path: PathBuf::from(path),
            branch: None,
            bare: false,
            locked: false,
            prunable: false,
        }
    }

    fn apply(&mut self, key: &str, value: &str) {
        match key {
            "branch" => {
                self.branch = value.strip_prefix("refs/heads/").map(str::to_owned);
            }
            "bare" => self.bare = true,
            "locked" => self.locked = true,
            "prunable" => self.prunable = true,
            _ => {}
        }
    }
}

/// Parses the whole listing; the primary checkout is first.
pub(super) fn parse(bytes: &[u8]) -> Result<Vec<Listed>, WorktreeError> {
    let mut entries = Vec::new();
    let mut current: Option<Listed> = None;
    for field in bytes.split(|byte| *byte == 0) {
        if field.is_empty() {
            entries.extend(current.take());
            continue;
        }
        let text = std::str::from_utf8(field).map_err(|_| WorktreeError::Porcelain)?;
        let (key, value) = text.split_once(' ').unwrap_or((text, ""));
        if key == "worktree" {
            entries.extend(current.replace(Listed::at(value)));
        } else {
            current
                .as_mut()
                .ok_or(WorktreeError::Porcelain)?
                .apply(key, value);
        }
    }
    entries.extend(current);
    Ok(entries)
}

#[cfg(test)]
mod tests {
    use proptest::prelude::*;

    use super::{Listed, parse};

    fn render(entries: &[Listed]) -> Vec<u8> {
        let mut out = Vec::new();
        for entry in entries {
            let mut fields = vec![
                format!("worktree {}", entry.path.display()),
                "HEAD 0123456789abcdef0123456789abcdef01234567".to_owned(),
            ];
            fields.push(entry.branch.as_ref().map_or_else(
                || "detached".to_owned(),
                |branch| format!("branch refs/heads/{branch}"),
            ));
            fields.extend(entry.bare.then(|| "bare".to_owned()));
            fields.extend(entry.locked.then(|| "locked held by test".to_owned()));
            fields.extend(entry.prunable.then(|| "prunable gitdir missing".to_owned()));
            fields.push("future-attribute value".to_owned());
            for field in fields {
                out.extend(field.into_bytes());
                out.push(0);
            }
            out.push(0);
        }
        out
    }

    fn entry() -> impl Strategy<Value = Listed> {
        (
            "/[a-z0-9 ._-]{1,12}(/[a-z0-9 ._-]{1,12}){0,3}",
            proptest::option::of("[a-z0-9][a-z0-9/._-]{0,16}"),
            any::<(bool, bool, bool)>(),
        )
            .prop_map(|(path, branch, (bare, locked, prunable))| Listed {
                path: path.into(),
                branch,
                bare,
                locked,
                prunable,
            })
    }

    proptest! {
        #[test]
        fn listings_round_trip(entries in proptest::collection::vec(entry(), 0..6)) {
            let parsed = parse(&render(&entries));
            prop_assert!(parsed.is_ok());
            prop_assert_eq!(parsed.ok(), Some(entries));
        }
    }

    #[test]
    fn an_attribute_before_any_worktree_is_malformed() {
        let parsed = parse(b"branch refs/heads/main\0\0");
        assert_eq!(
            parsed.map_err(|error| error.kind()).err(),
            Some("vcs_failed")
        );
    }
}
