//! The service lanes the fixture serves through `ServiceLanes` (plan todo 52): plugins
//! from a fixture plugin root holding one plugin whose program fails and one whose program
//! is missing.

use std::path::Path;
use std::sync::Arc;

use ovis_terminal_core::registry::RuntimeIdentity;
use ovis_terminal_server::dispatch::services::ServiceLanes;
use ovis_terminal_services::plugins::Plugins;
use serde_json::json;

use super::handler::Dispatch;
use super::serve::at;
use crate::error::RunnerError;

fn install(root: &Path, (id, manifest): (&str, &serde_json::Value)) -> Result<(), RunnerError> {
    let directory = root.join(id);
    std::fs::create_dir_all(&directory).map_err(at("create a fixture plugin"))?;
    std::fs::write(directory.join("plugin.json"), manifest.to_string())
        .map_err(at("write a fixture plugin manifest"))
}

/// Installs the fixture plugins below `<root>/plugins` and registers the plugin routes.
pub(crate) fn register(
    dispatch: &mut Dispatch,
    root: &Path,
    runtime: RuntimeIdentity,
) -> Result<(), RunnerError> {
    let plugins = root.join("plugins");
    let script = plugins.join("failing").join("fail.sh");
    let failing = json!({"plugin_id": "failing", "version": "1.0.0", "enabled": true,
        "actions": [{"action_id": "fail", "argv": ["/bin/sh", script], "contexts": ["machine"]}]});
    let broken = json!({"plugin_id": "broken", "version": "1.0.0", "enabled": true,
        "actions": [{"action_id": "run", "argv": [plugins.join("broken").join("missing")],
            "contexts": ["machine"]}]});
    let flood = plugins.join("chatty").join("flood.sh");
    let chatty = json!({"plugin_id": "chatty", "version": "1.0.0", "enabled": true,
        "actions": [{"action_id": "flood", "argv": ["/bin/sh", flood], "contexts": ["machine"]}]});
    install(&plugins, ("chatty", &chatty))?;
    std::fs::write(&flood, "head -c 10000 /dev/zero | tr '\\000' x\n")
        .map_err(at("write the flooding plugin"))?;
    install(&plugins, ("failing", &failing))?;
    install(&plugins, ("broken", &broken))?;
    std::fs::write(&script, "printf 'boom\\n'\nexit 3\n")
        .map_err(at("write the failing plugin"))?;
    ServiceLanes::new(runtime.machine_id())
        .with_plugins(Arc::new(Plugins::new(plugins)))
        .register(dispatch)
        .map_err(at("register the service lanes"))
}
