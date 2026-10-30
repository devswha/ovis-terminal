//! Request and reply types of every v1 method, partitioned by namespace.

pub mod agent;
pub mod attach;
pub mod discovery;
pub mod doc;
pub mod events;
pub mod input;
pub mod layout;
pub mod machine;
pub mod plugin;
pub mod server;
pub mod service;
pub mod status;
pub mod template;
pub mod terminal;
pub mod terminal_read;
pub mod update;
pub mod worktree;

use super::method::MethodSpec;

const NAMESPACES: &[&[MethodSpec]] = &[
    server::METHODS,
    terminal::METHODS,
    terminal_read::METHODS,
    input::METHODS,
    attach::METHODS,
    events::METHODS,
    status::METHODS,
    agent::METHODS,
    template::METHODS,
    layout::METHODS,
    worktree::METHODS,
    plugin::METHODS,
    doc::METHODS,
    machine::METHODS,
    service::METHODS,
    update::METHODS,
    discovery::METHODS,
];

/// Every method of the full v1 catalog, in publication order.
pub fn all_methods() -> impl Iterator<Item = &'static MethodSpec> {
    NAMESPACES.iter().flat_map(|namespace| namespace.iter())
}
