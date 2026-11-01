//! Syntax rules evaluated on one parsed Rust file.

use proc_macro2::{Span, TokenTree};
use quote::ToTokens as _;
use syn::punctuated::Punctuated;
use syn::visit::{self, Visit};
use syn::{
    Attribute, BinOp, Expr, ExprCast, File, Lit, Local, Macro, Meta, Pat, Signature, Token, Type,
};

use super::report::Finding;

/// Every rule id `source-check` can report about a Rust source file.
pub(super) const SOURCE_RULES: &[&str] = &[
    "syntax",
    "max-pure-loc",
    "max-parameters",
    "dropped-result",
    "allow-attribute",
    "numeric-cast",
    "tautological-assert",
];

const NUMERIC: &[&str] = &[
    "u8", "u16", "u32", "u64", "u128", "usize", "i8", "i16", "i32", "i64", "i128", "isize", "f32",
    "f64",
];

const ASSERTS: &[&str] = &["assert", "debug_assert"];
const COMPARING_ASSERTS: &[&str] = &[
    "assert_eq",
    "assert_ne",
    "debug_assert_eq",
    "debug_assert_ne",
];

/// The findings of every syntax rule in `file`, reported under `path`.
pub(super) fn syntax_rules(path: &str, file: &File, max_params: usize) -> Vec<Finding> {
    let mut rules = Rules {
        path,
        max_params,
        found: Vec::new(),
    };
    rules.visit_file(file);
    rules.found
}

struct Rules<'path> {
    path: &'path str,
    max_params: usize,
    found: Vec<Finding>,
}

impl Rules<'_> {
    fn push(&mut self, (rule, span): (&'static str, Span), detail: String) {
        let location = format!("{}:{}", self.path, span.start().line);
        self.found.push(Finding::new(rule, location, detail));
    }
}

impl<'ast> Visit<'ast> for Rules<'_> {
    fn visit_signature(&mut self, signature: &'ast Signature) {
        let count = signature.inputs.len();
        if count > self.max_params {
            let detail = format!(
                "fn {} takes {count} parameters, receiver included (limit {})",
                signature.ident, self.max_params
            );
            self.push(("max-parameters", signature.ident.span()), detail);
        }
        visit::visit_signature(self, signature);
    }

    fn visit_local(&mut self, local: &'ast Local) {
        if matches!(local.pat, Pat::Wild(_))
            && let Some(init) = &local.init
            && produces_value(&init.expr)
        {
            let detail = "`let _ =` discards what a call returned; handle or propagate it";
            self.push(("dropped-result", local.let_token.span), detail.to_owned());
        }
        visit::visit_local(self, local);
    }

    fn visit_attribute(&mut self, attribute: &'ast Attribute) {
        if silences_lints(attribute) {
            let detail = "`allow` silences a lint; use #[expect(lint, reason = ...)] on one item";
            self.push(
                ("allow-attribute", attribute.pound_token.span),
                detail.to_owned(),
            );
        }
        visit::visit_attribute(self, attribute);
    }

    fn visit_expr_cast(&mut self, cast: &'ast ExprCast) {
        if let Some(target) = numeric_target(&cast.ty) {
            let detail = format!("`as {target}` cast; use From for widening or try_from");
            self.push(("numeric-cast", cast.as_token.span), detail);
        }
        visit::visit_expr_cast(self, cast);
    }

    fn visit_macro(&mut self, mac: &'ast Macro) {
        if let Some(detail) = tautology(mac) {
            let span = mac.bang_token.span;
            self.push(("tautological-assert", span), detail);
        }
        visit::visit_macro(self, mac);
    }
}

/// Calls, awaits, `?` and macro invocations produce a value worth handling.
const fn produces_value(expr: &Expr) -> bool {
    matches!(
        expr,
        Expr::Call(_) | Expr::MethodCall(_) | Expr::Await(_) | Expr::Try(_) | Expr::Macro(_)
    )
}

fn silences_lints(attribute: &Attribute) -> bool {
    if attribute.path().is_ident("allow") {
        return true;
    }
    match &attribute.meta {
        Meta::List(list) if attribute.path().is_ident("cfg_attr") => list
            .tokens
            .clone()
            .into_iter()
            .any(|tree| matches!(tree, TokenTree::Ident(ident) if ident == "allow")),
        Meta::List(_) | Meta::Path(_) | Meta::NameValue(_) => false,
    }
}

fn numeric_target(ty: &Type) -> Option<String> {
    let Type::Path(path) = ty else {
        return None;
    };
    let ident = path.path.get_ident()?.to_string();
    (path.qself.is_none() && NUMERIC.contains(&ident.as_str())).then_some(ident)
}

fn text(expr: &Expr) -> String {
    expr.to_token_stream().to_string()
}

/// Whether evaluating `expr` twice must give the same value: no call, macro or await, so
/// `assert_ne!(fresh(), fresh())` stays a real check while `assert!(x >= x)` is not.
fn pure(expr: &Expr) -> bool {
    match expr {
        Expr::Path(_) | Expr::Lit(_) => true,
        Expr::Field(field) => pure(&field.base),
        Expr::Reference(reference) => pure(&reference.expr),
        Expr::Paren(paren) => pure(&paren.expr),
        Expr::Unary(unary) => pure(&unary.expr),
        Expr::Binary(binary) => pure(&binary.left) && pure(&binary.right),
        Expr::Tuple(tuple) => tuple.elems.iter().all(pure),
        _ => false,
    }
}

fn same(left: &Expr, right: &Expr) -> bool {
    pure(left) && text(left) == text(right)
}

/// Why an assertion cannot fail, if it compares an expression with itself or asserts `true`.
fn tautology(mac: &Macro) -> Option<String> {
    let name = mac.path.segments.last()?.ident.to_string();
    let comparing = COMPARING_ASSERTS.contains(&name.as_str());
    if !comparing && !ASSERTS.contains(&name.as_str()) {
        return None;
    }
    let args = mac
        .parse_body_with(Punctuated::<Expr, Token![,]>::parse_terminated)
        .ok()?;
    let mut args = args.into_iter();
    let first = args.next()?;
    if comparing {
        let second = args.next()?;
        return same(&first, &second)
            .then(|| format!("{name}! compares `{}` with itself", text(&first)));
    }
    match first {
        Expr::Lit(literal) if matches!(literal.lit, Lit::Bool(ref flag) if flag.value) => {
            Some(format!("{name}!(true) cannot fail"))
        }
        Expr::Binary(binary) if reflexive(&binary.op) && same(&binary.left, &binary.right) => Some(
            format!("{name}! compares `{}` with itself", text(&binary.left)),
        ),
        _ => None,
    }
}

/// Comparisons that hold for any value compared with itself.
const fn reflexive(op: &BinOp) -> bool {
    matches!(op, BinOp::Eq(_) | BinOp::Le(_) | BinOp::Ge(_))
}

#[cfg(test)]
mod tests;
