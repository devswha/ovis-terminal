use ovis_terminal_protocol::domain::restart::InterruptedWait;
use ovis_terminal_protocol::schema::ids::TerminalId as WireTerminalId;
use ovis_terminal_protocol::schema::scalar::ByteOffset;

use super::*;
use crate::listen::{Continuity, EndpointIdentity};

#[tokio::test(flavor = "multi_thread")]
async fn a_restart_over_the_socket_keeps_the_child_and_the_lifetime_and_changes_the_boot()
-> TestResult {
    // Given: a live child, a wait armed on it, and a client of boot 1.
    let fixture = block_in_place(Fixture::new)?;
    let record = fixture.spawn("printf ARMED; read gate")?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let armed = fixture.control.waits().arm(InterruptedWait::new(
        OpId::new(op_id()),
        WireTerminalId::new(record.id().to_string()),
        ByteOffset::new(0),
    ));
    let served = fixture.serve().await?;
    let (mut client, earlier) = fixture.connect().await?;

    // When: the client asks for a planned restart.
    let reply = client
        .call("server.restart", &json!({"op_id": op_id()}))
        .await?;

    // Then: the receipt names both boots and the preserved incarnation; nothing ended,
    // and the in-flight wait was ended `cancelled` rather than left to time out.
    assert_eq!(
        reply.pointer("/result/server_boot_id_old"),
        Some(&json!("boot:1")),
        "{reply}"
    );
    assert_eq!(
        reply.pointer("/result/server_boot_id_new"),
        Some(&json!("boot:2"))
    );
    let generation = process.generation().get().to_string();
    assert_eq!(
        reply.pointer("/result/preserved_incarnations"),
        Some(&json!([generation]))
    );
    assert_eq!(
        reply.pointer("/result/ended_incarnations"),
        Some(&json!([]))
    );
    assert!(armed.scope().is_cancelled());
    drop(armed);

    // And: the successor answers hello under the same lifetime and a new boot, while the
    // same process still runs with its start token.
    served.hand_over().await?;
    let closing = client.next().await?;
    assert_eq!(
        closing.get("method"),
        Some(&json!("$/closing")),
        "{closing}"
    );
    assert_eq!(
        closing.pointer("/params/reason"),
        Some(&json!("server_restart"))
    );
    assert_eq!(
        closing.pointer("/params/incarnations_preserved"),
        Some(&json!(true))
    );
    assert_eq!(
        client.record().await?,
        None,
        "the old boot's connection ends after it"
    );
    let served = fixture.serve().await?;
    let (_client, hello) = fixture.connect().await?;
    assert_eq!(hello.runtime_lifetime, earlier.runtime_lifetime);
    assert_eq!(hello.server_boot_id.as_str(), "boot:2");
    let continuity = EndpointIdentity::of(&hello).continuity(&EndpointIdentity::of(&earlier));
    assert_eq!(continuity, Continuity::ControlRestarted);
    assert!(running(&process)?);
    let adopted = fixture.terminals(|terminals, session| terminals.get(session, record.id()))?;
    let after = adopted
        .lifecycle
        .process()
        .ok_or("the reconnect found no process")?;
    assert_eq!(
        (after.pid(), after.start_token(), after.generation()),
        (process.pid(), process.start_token(), process.generation()),
        "the reconnect reaches the same process incarnation"
    );
    served.retire().await?;
    block_in_place(|| drop(fixture));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn an_unknown_stop_scope_is_invalid_params_and_nothing_stops_while_control_only_keeps_programs()
-> TestResult {
    // Given: a live child and a client.
    let fixture = block_in_place(Fixture::new)?;
    let record = fixture.spawn("printf ARMED; read gate")?;
    let process = record.lifecycle.process().ok_or("process")?.clone();
    let served = fixture.serve().await?;
    let (mut client, _) = fixture.connect().await?;

    // When: a stop names no scope, or a scope that does not exist.
    let invented = json!({"op_id": op_id(), "scope": "everything-implicitly"});
    let invented = client.call("server.stop", &invented).await?;
    let missing = client
        .call("server.stop", &json!({"op_id": op_id()}))
        .await?;

    // Then: both are invalid_params at params.scope, and the program and control live on.
    for reply in [&invented, &missing] {
        assert_eq!(
            reply.pointer("/error/data/kind"),
            Some(&json!("invalid_params")),
            "{reply}"
        );
        assert_eq!(
            reply.pointer("/error/data/field_path"),
            Some(&json!("params.scope"))
        );
        assert!(reply.get("result").is_none());
    }
    assert!(running(&process)?);
    assert_eq!(
        fixture
            .terminals(|terminals, session| terminals.list(session))?
            .len(),
        1
    );

    // And: an explicit control-only stop keeps the program and refuses a later restart.
    let stopped = client
        .call(
            "server.stop",
            &json!({"op_id": op_id(), "scope": "control_only"}),
        )
        .await?;
    let generation = process.generation().get().to_string();
    assert_eq!(
        stopped.pointer("/result/preserved_incarnations"),
        Some(&json!([generation])),
        "{stopped}"
    );
    assert_eq!(
        stopped.pointer("/result/ended_incarnations"),
        Some(&json!([]))
    );
    assert_eq!(
        stopped.pointer("/result/server_boot_id_new"),
        Some(&Value::Null)
    );
    assert!(running(&process)?);
    let refused = client
        .call("server.restart", &json!({"op_id": op_id()}))
        .await?;
    assert_eq!(
        refused.pointer("/error/data/kind"),
        Some(&json!("control_stopped")),
        "{refused}"
    );
    served.retire().await?;
    block_in_place(|| drop(fixture));
    Ok(())
}
