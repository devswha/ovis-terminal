//! The runtime contract reference: the stated behaviour (`contract.md`) followed by tables
//! rendered from the OpenRPC document, the role table and the command-line contract of
//! this build, so the reference cannot drift from the types it describes.

use serde_json::Value;

use super::tables::{cell, list, rows, text};

pub(super) const BEHAVIOUR: &str = include_str!("contract.md");

pub(super) fn methods(document: &Value) -> String {
    let mut out = String::from(
        "\n## Methods\n\nEvery method of the catalog. `hello.methods` lists the ones a \
         server routes; any other answers `method_not_found`. A mutation takes a \
         client-minted `op_id`; resending it never repeats the effect.\n\n\
         | method | effect | targeting | params | result | error kinds | summary |\n\
         |---|---|---|---|---|---|---|\n",
    );
    for method in list(document, "/methods") {
        let errors: Vec<&str> = list(method, "/errors")
            .iter()
            .filter_map(|error| error.get("message").and_then(Value::as_str))
            .collect();
        out.push_str(&rows(&[
            text(method, "/name"),
            text(method, "/x-effect"),
            text(method, "/x-targeting"),
            schema_name(method, "/x-params/$ref"),
            schema_name(method, "/result/schema/$ref"),
            errors.join(", "),
            text(method, "/summary"),
        ]));
    }
    out
}

fn schema_name(value: &Value, pointer: &str) -> String {
    let reference = text(value, pointer);
    reference
        .rsplit('/')
        .next()
        .map_or_else(String::new, str::to_owned)
}

pub(super) fn events(document: &Value) -> String {
    let mut out = String::from(
        "\n## Events\n\nEvent types delivered in the `event` notification envelope.\n\n\
         | type | summary |\n|---|---|\n",
    );
    for event in list(document, "/x-events") {
        out.push_str(&rows(&[text(event, "/name"), text(event, "/summary")]));
    }
    out.push_str("\nNotifications: ");
    let names: Vec<String> = list(document, "/x-notifications")
        .iter()
        .map(|notification| format!("`{}`", text(notification, "/name")))
        .collect();
    out.push_str(&names.join(", "));
    out.push_str(".\n");
    out
}

pub(super) fn error_kinds(document: &Value) -> String {
    let mut out = String::from(
        "\n## Error kinds\n\nEvery `error.data.kind` a server returns, its JSON-RPC code and \
         the recovery it calls for. Branch on the kind, never on the message.\n\n\
         | kind | code | recovery |\n|---|---|---|\n",
    );
    for kind in list(document, "/x-error-kinds") {
        let code = kind.get("code").map_or_else(String::new, Value::to_string);
        out.push_str(&rows(&[
            text(kind, "/kind"),
            code,
            text(kind, "/description"),
        ]));
    }
    out
}

pub(super) fn limits(document: &Value) -> String {
    let mut out = String::from(
        "\n## Limits\n\nDefault bounds `hello` advertises, as canonical decimal strings.\n\n\
         | limit | default |\n|---|---|\n",
    );
    if let Some(Value::Object(defaults)) = document.pointer("/x-limits/defaults") {
        for (name, value) in defaults {
            let shown = value
                .as_str()
                .map_or_else(|| value.to_string(), str::to_owned);
            out.push_str(&rows(&[name.clone(), shown]));
        }
    }
    out
}

pub(super) fn command_line(cli: &Value) -> String {
    let mut out = String::from(
        "\n## Command line\n\n`--json` (or `OVIS_JSON=1`) selects JSON whatever stdout is: \
         a result is one JSON document on stdout, a failure is one `{\"error\": {...}}` \
         record on stderr with `origin` (`cli` or `server`), `kind`, `hint` and \
         `exit_code`. The command line is parsed and typed before any connection. The \
         commands differ by platform (`serve`, `hold` and `terminal attach` are Unix \
         only), so they are listed by the manual (`ovis-terminal --skill`) and by \
         `ovis-terminal schema cli` of each build, not here; everything below holds on \
         every platform.\n",
    );
    out.push_str("\n| exit code | name | kinds |\n|---|---|---|\n");
    for status in list(cli, "/exit_codes") {
        let kinds: Vec<&str> = list(status, "/kinds")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let code = status
            .get("code")
            .map_or_else(String::new, Value::to_string);
        out.push_str(&rows(&[code, text(status, "/name"), kinds.join(", ")]));
    }
    let failures: Vec<String> = list(cli, "/failure_kinds")
        .iter()
        .filter_map(Value::as_str)
        .map(|kind| format!("`{kind}`"))
        .collect();
    out.push_str("\nKinds this binary reports itself and exits 1 with: ");
    out.push_str(&failures.join(", "));
    out.push_str(". Any other kind a newer server returns also exits 1.\n");
    out.push_str("\nEnvironment: ");
    let names: Vec<String> = list(cli, "/environment")
        .iter()
        .filter_map(Value::as_str)
        .map(|name| format!("`{}`", cell(name)))
        .collect();
    out.push_str(&names.join(", "));
    out.push_str(".\n");
    out
}
