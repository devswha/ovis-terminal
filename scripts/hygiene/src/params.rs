//! Parameter counts of every function signature in a parsed file.

use std::path::Path;

use syn::visit::{self, Visit};
use syn::{File, Signature};

use crate::MAX_PARAMETERS;
use crate::report::{Finding, Violation};

/// Reports every signature in `file` with more than [`MAX_PARAMETERS`] inputs.
///
/// Free functions, inherent and trait methods, trait declarations and foreign
/// functions are all visited; a `self` receiver counts as a parameter.
pub fn excess_parameters(path: &Path, file: &File) -> Vec<Violation> {
    let mut collector = Collector {
        path,
        found: Vec::new(),
    };
    collector.visit_file(file);
    collector.found
}

struct Collector<'path> {
    path: &'path Path,
    found: Vec<Violation>,
}

impl Visit<'_> for Collector<'_> {
    fn visit_signature(&mut self, signature: &Signature) {
        let count = signature.inputs.len();
        if count > MAX_PARAMETERS {
            let finding = Finding::Parameters {
                function: signature.ident.to_string(),
                count,
            };
            let line = signature.ident.span().start().line;
            self.found
                .push(Violation::new(self.path.to_path_buf(), line, finding));
        }
        visit::visit_signature(self, signature);
    }
}
