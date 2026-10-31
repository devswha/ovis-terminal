//! systemd user units and launchd agents for one session's holder.
//!
//! Every definition carries an ownership marker naming its label, so removal touches only
//! a definition this runtime wrote. The holder never restarts on its own: relaunching a
//! program whose survival is unproven is a separate, explicit decision.

use std::fmt::Write as _;
use std::path::Path;

use ovis_terminal_protocol::schema::methods::service::ServiceManager;

use super::{ServiceError, ServicePaths};

const MARKER: &str = "X-OvisTerminalLabel=";

/// The ownership marker line of `paths`' label.
pub(super) fn marker(paths: &ServicePaths) -> String {
    format!("{MARKER}{}", paths.session().label().as_str())
}

/// Whether `text` is a definition this runtime wrote for `paths`.
pub(super) fn owned(text: &str, paths: &ServicePaths) -> bool {
    let marker = marker(paths);
    text.lines()
        .any(|line| line.trim_start_matches("<!-- ").trim_end_matches(" -->") == marker)
}

fn text(path: &Path) -> Result<&str, ServiceError> {
    path.to_str()
        .filter(|text| !text.contains(['\n', '\r']))
        .ok_or_else(|| ServiceError::UnsupportedPath {
            path: path.to_path_buf(),
        })
}

/// The holder command a manager runs.
fn command<'a>(
    paths: &'a ServicePaths,
    program: &'a Path,
    manager: &'a str,
) -> Result<[&'a str; 7], ServiceError> {
    Ok([
        text(program)?,
        "service",
        "run",
        "--session",
        paths.session().as_str(),
        "--manager",
        manager,
    ])
}

fn systemd_quote(value: &str) -> String {
    let escaped = value
        .replace('\\', "\\\\")
        .replace('"', "\\\"")
        .replace('%', "%%")
        .replace('$', "$$");
    format!("\"{escaped}\"")
}

/// A `Type=notify` user unit: `systemctl start` returns once the holder sent `READY=1`.
pub(super) fn systemd(paths: &ServicePaths, program: &Path) -> Result<String, ServiceError> {
    let exec: Vec<String> = command(paths, program, "systemd-user")?
        .iter()
        .map(|arg| systemd_quote(arg))
        .collect();
    let mut unit = format!(
        "[Unit]\nDescription=ovis-terminal PTY holder for session {}\n{}\n\n[Service]\nType=notify\nNotifyAccess=main\nRestart=no\nExecStart={}\n",
        paths.session().as_str(),
        marker(paths),
        exec.join(" ")
    );
    for (var, value) in paths.environment().vars() {
        let assignment = format!("{}={}", var.name(), text(value)?);
        writeln!(unit, "Environment={}", systemd_quote(&assignment)).map_err(|_| {
            ServiceError::UnsupportedPath {
                path: value.to_path_buf(),
            }
        })?;
    }
    Ok(unit)
}

fn xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

/// A launchd agent that runs the holder once when bootstrapped and never keeps it alive.
pub(super) fn launchd(paths: &ServicePaths, program: &Path) -> Result<String, ServiceError> {
    let mut arguments = String::new();
    for arg in command(paths, program, "launchd")? {
        write!(arguments, "<string>{}</string>", xml(arg)).map_err(|_| {
            ServiceError::UnsupportedPath {
                path: program.to_path_buf(),
            }
        })?;
    }
    let environment = paths
        .environment()
        .vars()
        .map(|(var, value)| {
            Ok(format!(
                "<key>{}</key><string>{}</string>",
                var.name(),
                xml(text(value)?)
            ))
        })
        .collect::<Result<String, ServiceError>>()?;
    Ok(format!(
        "<?xml version=\"1.0\" encoding=\"UTF-8\"?>\n<!-- {} -->\n<!DOCTYPE plist PUBLIC \"-//Apple//DTD PLIST 1.0//EN\" \"http://www.apple.com/DTDs/PropertyList-1.0.dtd\">\n<plist version=\"1.0\"><dict><key>Label</key><string>{}</string><key>ProgramArguments</key><array>{arguments}</array><key>EnvironmentVariables</key><dict>{environment}</dict><key>RunAtLoad</key><true/><key>KeepAlive</key><false/><key>ProcessType</key><string>Interactive</string></dict></plist>\n",
        marker(paths),
        xml(&paths.session().launchd_label()),
    ))
}

/// The definition file `manager` uses for `paths`.
pub(super) fn location(
    paths: &ServicePaths,
    manager: ServiceManager,
) -> Result<std::path::PathBuf, ServiceError> {
    match manager {
        ServiceManager::SystemdUser => paths.systemd_unit(),
        ServiceManager::Launchd => paths.launchd_plist(),
        _ => None,
    }
    .ok_or(ServiceError::NoRuntimeDirectory)
}

#[cfg(test)]
mod tests {
    use super::{launchd, owned, systemd};
    use crate::service::unix::test_root::TestRoot;

    #[test]
    fn a_unit_quotes_every_argument_names_its_owner_and_never_restarts() {
        let root = TestRoot::new("unit");
        let paths = root.paths("s1");
        let unit = systemd(
            &paths,
            std::path::Path::new("/opt/o t/100%$x\"/ovis-terminal"),
        )
        .unwrap();
        assert!(unit.contains("ExecStart=\"/opt/o t/100%%$$x\\\"/ovis-terminal\" \"service\" \"run\" \"--session\" \"s1\" \"--manager\" \"systemd-user\"\n"), "{unit}");
        assert!(unit.contains("\nType=notify\n") && unit.contains("\nRestart=no\n"));
        assert!(unit.contains(&format!(
            "Environment=\"XDG_RUNTIME_DIR={}/run\"",
            root.path().display()
        )));
        assert!(owned(&unit, &paths));
        assert!(
            !owned(&unit, &root.paths("s2")),
            "another session claimed this unit"
        );
        assert!(!owned("[Unit]\nDescription=someone else\n", &paths));
        let refused = systemd(&paths, std::path::Path::new("/bin/x\ny"))
            .map(drop)
            .map_err(|e| e.kind());
        assert_eq!(refused, Err("unsupported_path"));
    }

    #[test]
    fn an_agent_escapes_xml_names_its_owner_and_is_not_kept_alive() {
        let root = TestRoot::new("plist");
        let paths = root.paths("s1");
        let plist = launchd(&paths, std::path::Path::new("/opt/a&b/<ovis-terminal>")).unwrap();
        assert!(plist.contains(
            "<string>/opt/a&amp;b/&lt;ovis-terminal&gt;</string><string>service</string>"
        ));
        assert!(plist.contains("<key>Label</key><string>dev.ovis-terminal.holder.s1</string>"));
        assert!(plist.contains("<key>KeepAlive</key><false/>"));
        assert!(owned(&plist, &paths) && !owned(&plist, &root.paths("s2")));
    }
}
