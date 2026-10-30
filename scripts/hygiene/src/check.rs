//! Runs every rule against one source text.

use std::path::Path;

use proc_macro2::{LexError, TokenStream};

use crate::MAX_PURE_LOC;
use crate::loc::pure_loc;
use crate::params::excess_parameters;
use crate::report::{Finding, Violation};

/// Checks `source`, reported under `path`, against every hygiene rule.
///
/// A file that does not parse yields a single [`Finding::Syntax`] violation,
/// because the other rules cannot be evaluated on it.
pub fn check_source(path: &Path, source: &str) -> Vec<Violation> {
    let file = match syn::parse_file(source) {
        Ok(file) => file,
        Err(error) => {
            let line = error.span().start().line;
            let finding = Finding::Syntax {
                message: error.to_string(),
            };
            return vec![Violation::new(path.to_path_buf(), line, finding)];
        }
    };
    let tokens = match source.parse::<TokenStream>() {
        Ok(tokens) => tokens,
        Err(error) => return vec![lex_violation(path, &error)],
    };
    let mut found = Vec::new();
    let count = pure_loc(tokens);
    if count > MAX_PURE_LOC {
        found.push(Violation::new(
            path.to_path_buf(),
            1,
            Finding::PureLoc { count },
        ));
    }
    found.extend(excess_parameters(path, &file));
    found
}

fn lex_violation(path: &Path, error: &LexError) -> Violation {
    let line = error.span().start().line;
    Violation::new(
        path.to_path_buf(),
        line,
        Finding::Syntax {
            message: error.to_string(),
        },
    )
}
