//! Markdown table helpers and the role table of the runtime contract reference.

use ovis_terminal_authz::{Action, Permission, Role};
use serde_json::Value;

/// The array at `pointer`, or nothing.
pub(super) fn list<'a>(value: &'a Value, pointer: &str) -> &'a [Value] {
    value
        .pointer(pointer)
        .and_then(Value::as_array)
        .map_or(&[], Vec::as_slice)
}

/// The string at `pointer`, or an empty one.
pub(super) fn text(value: &Value, pointer: &str) -> String {
    value
        .pointer(pointer)
        .and_then(Value::as_str)
        .map_or_else(String::new, str::to_owned)
}

/// `text` safe inside one table cell: pipes escaped, line breaks folded.
pub(super) fn cell(text: &str) -> String {
    text.replace('|', "\\|").replace('\n', " ")
}

/// One table row of `cells`.
pub(super) fn rows(cells: &[String]) -> String {
    let joined: Vec<String> = cells.iter().map(|value| cell(value)).collect();
    format!("| {} |\n", joined.join(" | "))
}

const ROLES: [(Role, &str); 3] = [
    (Role::Observer, "observer"),
    (Role::Interactor, "interactor"),
    (Role::Owner, "owner"),
];

const ACTIONS: [Action; 17] = [
    Action::InspectOperation,
    Action::CancelOperation,
    Action::Read,
    Action::Subscribe,
    Action::Input,
    Action::RawInput,
    Action::Resize,
    Action::Launch,
    Action::LayoutChange,
    Action::DocumentWrite,
    Action::Takeover,
    Action::PromptAnswer,
    Action::Report,
    Action::Administer,
    Action::PluginInvoke,
    Action::IssueCredential,
    Action::RevokeCredential,
];

/// The roles, the permissions each bundles, and the permission every action needs.
pub(super) fn roles() -> String {
    let mut out = String::from(
        "\n## Roles and permissions\n\nA credential grants permissions; a role is a named \
         bundle. No permission implies another, and only the owner role bundles takeover, \
         prompt answers and raw input. A program's process token is not a credential: it \
         proves which program owns a terminal when that program reports status, prompts or \
         records, and a token of another terminal is refused.\n\n\
         | role | permissions |\n|---|---|\n",
    );
    for (role, name) in ROLES {
        let granted: Vec<&str> = role.permissions().iter().map(|p| p.as_str()).collect();
        out.push_str(&rows(&[name.to_owned(), granted.join(", ")]));
    }
    out.push_str("\n| action | required permission |\n|---|---|\n");
    for action in ACTIONS {
        let required: Permission = action.required();
        out.push_str(&rows(&[
            action.as_str().to_owned(),
            required.as_str().to_owned(),
        ]));
    }
    out
}
