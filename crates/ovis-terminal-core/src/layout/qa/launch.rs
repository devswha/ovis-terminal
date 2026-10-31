//! Creation and launch against a real holder: separate outcomes, no lost records.

use std::path::{Path, PathBuf};
use std::sync::Arc;

use ovis_terminal_core::layout::{Call, Edit, EditRequest, LaunchOutcome};
use ovis_terminal_core::registry::WorkspaceId;
use ovis_terminal_core::terminals::{Host, Launch, LaunchState, Program, Terminals};
use ovis_terminal_holder::protocol::{HolderBuildId, HolderDescriptor, HolderInstanceId};
use ovis_terminal_holder::unix::{Holder, HolderConfig, HolderLimits};
use ovis_terminal_store::{Store, StoreOptions};
use uuid::Uuid;

use super::{Fixture, TestResult, created, op};

struct Runtime {
    holder: Holder,
    terminals: Option<Terminals>,
}

impl Runtime {
    fn open(fixture: &Fixture) -> Result<Self, Box<dyn std::error::Error>> {
        let holder = Holder::new(HolderConfig::new(
            HolderDescriptor::new(
                HolderBuildId::new("task-18".into()),
                HolderInstanceId::new(Uuid::now_v7().to_string()),
            ),
            HolderLimits::DEFAULT,
        ));
        let store = Store::open(&fixture.root.join("terminals"), StoreOptions::default())?;
        let host = Host::new(
            fixture.identity,
            Arc::clone(&fixture.authority),
            holder.clone(),
        );
        Ok(Self {
            terminals: Some(Terminals::new(store, host)?),
            holder,
        })
    }

    fn terminals(&mut self) -> Result<&mut Terminals, Box<dyn std::error::Error>> {
        self.terminals
            .as_mut()
            .ok_or_else(|| "terminals are closed".into())
    }
}

impl Drop for Runtime {
    fn drop(&mut self) {
        if let Some(terminals) = self.terminals.take() {
            let closed = terminals.close();
            assert!(closed.is_ok(), "close control: {closed:?}");
        }
        let stopped = self.holder.shutdown();
        assert!(stopped.is_ok(), "join owned holder workers: {stopped:?}");
    }
}

fn program(argv: &[&str], cwd: &Path) -> Result<Launch, Box<dyn std::error::Error>> {
    let program = Program::new(
        argv.iter().map(|arg| (*arg).to_owned()).collect(),
        cwd.to_path_buf(),
        [("PATH".into(), "/usr/bin:/bin".into())].into(),
    )?;
    Ok(Launch::new(
        program,
        serde_json::from_value(serde_json::json!({"cols": 80, "rows": 24}))?,
    )?)
}

fn create_tab(
    fixture: &Fixture,
    (runtime, workspace): (&mut Runtime, WorkspaceId),
    launch: Launch,
) -> Result<ovis_terminal_core::layout::Change, Box<dyn std::error::Error>> {
    let mut request = EditRequest::new(Edit::CreateTab { workspace });
    request.launch = Some(launch);
    let call = Call::new(&fixture.session, op()).launching(runtime.terminals()?);
    Ok(fixture.layouts()?.edit(call, &request)?)
}

/// A rejected launch still returns and retains the created tab and pane.
pub(super) fn failed_launch_returns_created_ids(
    fixture: &Fixture,
    workspace: WorkspaceId,
) -> TestResult {
    // Given: a real holder-backed launcher and a working directory that does not exist.
    let mut runtime = Runtime::open(fixture)?;
    let missing: PathBuf = fixture.root.join("missing-cwd");
    let launch = program(&["/bin/sh", "-c", "exit 0"], &missing)?;

    // When: a tab is created with that launch.
    let change = create_tab(fixture, (&mut runtime, workspace), launch)?;

    // Then: the IDs are returned, retained in the layout, and the launch failure is separate.
    let ids = created(&change);
    let (tab, pane) = (ids.tab.ok_or("tab")?, ids.pane.ok_or("pane")?);
    let LaunchOutcome::Rejected { error } = &change.launch else {
        return Err(format!("expected a rejected launch, got {:?}", change.launch).into());
    };
    assert_eq!(error.kind.token(), "invalid_params");
    assert!(
        error.op_id.is_some(),
        "the terminal operation stays inspectable"
    );
    let layout = fixture.layout()?;
    let kept = layout
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .find(|t| t.id == tab);
    let kept = kept.ok_or("created tab retained")?;
    assert_eq!(
        kept.panes
            .iter()
            .map(|p| (p.id, p.terminal))
            .collect::<Vec<_>>(),
        [(pane, None)]
    );
    assert_eq!(layout.revision, change.revision);
    Ok(())
}

/// A launched terminal is bound to its pane in a later, separately evented commit.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn launch_binds_terminal_to_created_pane() -> TestResult {
    // Given: an empty workspace and a real holder-backed launcher.
    let fixture = Fixture::new()?;
    let workspace = created(&fixture.edit(Edit::CreateWorkspace {
        name: "agents".into(),
        group_with: None,
    })?)
    .workspace
    .ok_or("workspace")?;
    failed_launch_returns_created_ids(&fixture, workspace)?;
    let mut runtime = Runtime::open(&fixture)?;

    // When: a tab is created with a program that can start.
    let launch = program(&["/bin/sh", "-c", "exit 0"], &fixture.root)?;
    let change = create_tab(&fixture, (&mut runtime, workspace), launch)?;

    // Then: the terminal is admitted and bound to exactly the created pane.
    let pane = created(&change).pane.ok_or("pane")?;
    let LaunchOutcome::Launched { terminal, spawn } = change.launch else {
        return Err(format!("expected a launch, got {:?}", change.launch).into());
    };
    assert_ne!(spawn, LaunchState::Failed);
    let layout = fixture.layout()?;
    let mut panes = layout
        .workspaces
        .iter()
        .flat_map(|w| &w.tabs)
        .flat_map(|t| &t.panes);
    let bound = panes.find(|p| p.id == pane).ok_or("pane retained")?;
    assert_eq!(bound.terminal, Some(terminal));
    let record = runtime.terminals()?.get(&fixture.session, terminal)?;
    assert_eq!(record.id(), terminal);
    Ok(())
}

/// Layout edits never write into a working directory a pane was launched in.
///
/// # Errors
///
/// Returns store, authority, operation or layout failures of the fixture.
///
/// # Panics
///
/// Panics when an observed layout fact violates the acceptance contract.
pub fn layout_never_mutates_a_checkout() -> TestResult {
    // Given: a disposable dirty checkout used as a launch directory.
    let fixture = Fixture::new()?;
    let checkout = fixture.root.join("checkout");
    std::fs::create_dir_all(&checkout)?;
    std::fs::write(checkout.join("dirty.txt"), b"uncommitted edit\n")?;
    let listing = |dir: &Path| -> Result<Vec<(PathBuf, Vec<u8>)>, std::io::Error> {
        let mut entries = Vec::new();
        for entry in std::fs::read_dir(dir)? {
            let path = entry?.path();
            entries.push((path.clone(), std::fs::read(&path)?));
        }
        entries.sort();
        Ok(entries)
    };
    let before = listing(&checkout)?;
    let mut runtime = Runtime::open(&fixture)?;
    let workspace = created(&fixture.edit(Edit::CreateWorkspace {
        name: "dirty".into(),
        group_with: None,
    })?)
    .workspace
    .ok_or("workspace")?;

    // When: a pane is launched there, then split and closed.
    let launch = program(&["/bin/sh", "-c", "exit 0"], &checkout)?;
    let tab = create_tab(&fixture, (&mut runtime, workspace), launch)?;
    let pane = created(&tab).pane.ok_or("pane")?;
    fixture.edit(Edit::Split {
        pane,
        axis: ovis_terminal_core::layout::Axis::Vertical,
        ratio_permille: 600,
    })?;
    fixture.edit(Edit::Close {
        target: ovis_terminal_core::layout::Target::Workspace(workspace),
        scope: ovis_terminal_core::layout::CloseScope::Target,
    })?;

    // Then: the checkout bytes and entries are exactly as before.
    assert_eq!(listing(&checkout)?, before);
    Ok(())
}
