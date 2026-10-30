//! The method list `hello` advertises: implemented methods only, in catalog order.

use std::collections::BTreeSet;

use super::error::SchemaError;
use super::ids::MethodName;
use super::methods::all_methods;

/// Returns the methods `hello` advertises for a dispatcher's registered handlers.
///
/// Specified methods without a handler are never advertised, so a client never
/// sees a method the runtime would refuse.
///
/// # Errors
///
/// Returns [`SchemaError::UncataloguedMethod`] for a handler the catalog does
/// not specify, and [`SchemaError::DuplicateName`] for a handler registered twice.
pub fn advertised_methods<'a>(
    registered: impl IntoIterator<Item = &'a str>,
) -> Result<Vec<MethodName>, SchemaError> {
    let mut handlers = BTreeSet::new();
    for method in registered {
        if all_methods().all(|spec| spec.name != method) {
            return Err(SchemaError::UncataloguedMethod {
                method: method.to_owned(),
            });
        }
        if !handlers.insert(method) {
            return Err(SchemaError::DuplicateName {
                name: method.to_owned(),
            });
        }
    }
    Ok(all_methods()
        .filter(|spec| handlers.contains(spec.name))
        .map(|spec| MethodName::new(spec.name.to_owned()))
        .collect())
}
