use ovis_terminal_protocol::schema::methods::server::StopScope;

use super::*;

#[test]
fn a_control_only_stop_signals_no_program_and_lists_it_as_preserved() -> TestResult {
    // Given: a live child.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();

    // When: control stops with the control-only scope.
    let receipt = control.stop(op(), StopScope::ControlOnly)?;

    // Then: the child runs on, the receipt names it preserved, and control serves nothing.
    assert_eq!(receipt.preserved, vec![process.clone()]);
    assert_eq!(receipt.ended, Vec::new());
    assert!(running(&process)?);
    let after = control.with_terminals(|terminals, session| terminals.list(session));
    assert_eq!(
        after.map(drop).map_err(|e| e.kind()),
        Err("control_stopped")
    );
    let wire = serde_json::to_value(receipt.transition())?;
    assert_eq!(
        wire.get("server_boot_id_new").cloned().unwrap_or_default(),
        serde_json::Value::Null
    );
    assert_eq!(
        wire.get("preserved_incarnations")
            .cloned()
            .unwrap_or_default(),
        serde_json::json!([process.generation().get().to_string()])
    );
    assert_eq!(
        wire.get("ended_incarnations").cloned().unwrap_or_default(),
        serde_json::json!([])
    );
    control.close()?;
    Ok(())
}

#[test]
fn a_program_stop_names_each_reaped_incarnation_as_ended_and_none_as_preserved() -> TestResult {
    // Given: a live child.
    let fixture = Fixture::new()?;
    let (control, _) = fixture.open()?;
    let record = spawn(&control, fixture.launch("printf ARMED; read gate")?)?;
    marker(&control, (record.id(), b"ARMED"))?;
    let process = record.lifecycle.process().ok_or("process")?.clone();

    // When: control stops together with its programs.
    let receipt = control.stop(op(), StopScope::ControlAndPrograms)?;

    // Then: the receipt names the reaped incarnation ended and the process is gone.
    assert_eq!(receipt.ended, vec![process.clone()]);
    assert_eq!(receipt.preserved, Vec::new(), "{receipt:?}");
    assert!(!running(&process)?);
    let wire = serde_json::to_value(receipt.transition())?;
    assert_eq!(
        wire.get("ended_incarnations").cloned().unwrap_or_default(),
        serde_json::json!([process.generation().get().to_string()])
    );
    control.close()?;
    Ok(())
}
