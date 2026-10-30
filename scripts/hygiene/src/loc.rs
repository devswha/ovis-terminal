//! Pure lines of code, counted from the token stream rather than from text.

use std::collections::BTreeSet;

use proc_macro2::{Span, TokenStream, TokenTree};

/// Counts the lines of `tokens` that carry at least one non-comment token.
///
/// Doc comments arrive from the lexer as `#[doc = "..."]` attributes whose spans
/// cover the comment text; they are recognised by that source text and skipped.
pub fn pure_loc(tokens: TokenStream) -> usize {
    let mut lines = BTreeSet::new();
    collect(tokens, &mut lines);
    lines.len()
}

fn collect(tokens: TokenStream, lines: &mut BTreeSet<usize>) {
    for tree in tokens {
        if is_comment(tree.span()) {
            continue;
        }
        match tree {
            TokenTree::Group(group) => {
                mark(group.span_open(), lines);
                mark(group.span_close(), lines);
                collect(group.stream(), lines);
            }
            TokenTree::Ident(_) | TokenTree::Punct(_) | TokenTree::Literal(_) => {
                mark(tree.span(), lines);
            }
        }
    }
}

fn is_comment(span: Span) -> bool {
    span.source_text()
        .is_some_and(|text| text.starts_with("//") || text.starts_with("/*"))
}

fn mark(span: Span, lines: &mut BTreeSet<usize>) {
    lines.extend(span.start().line..=span.end().line);
}

#[cfg(test)]
mod tests {
    use proc_macro2::TokenStream;

    use super::pure_loc;

    fn count(source: &str) -> usize {
        pure_loc(source.parse::<TokenStream>().unwrap())
    }

    #[test]
    fn comments_blank_lines_and_docs_are_not_code() {
        let source =
            "//! crate doc\n\n// note\n/* block\n still block */\n/// item doc\nfn a() {}\n";
        assert_eq!(count(source), 1);
    }

    #[test]
    fn comment_markers_inside_strings_are_code() {
        assert_eq!(count("const S: &str = \"// not a comment\";\n"), 1);
    }

    #[test]
    fn every_line_of_a_multiline_literal_and_group_counts() {
        assert_eq!(count("const S: &str = \"a\nb\nc\";\nfn f() {\n}\n"), 5);
    }
}
