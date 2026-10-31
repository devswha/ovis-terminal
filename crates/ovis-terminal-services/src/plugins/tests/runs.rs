use base64::Engine as _;
use ovis_terminal_protocol::schema::methods::plugin::{PluginRunRecord, PluginRunState};
use ovis_terminal_protocol::schema::scalar::OpenEnum;
use serde_json::{Value, json};
use tokio::sync::watch;

use super::super::runs::OUTPUT_BYTES;
use super::{Root, TestResult, invoke, manifest, run_id, set, sh_action};

async fn run_script(name: &str, body: &str, input: &Value) -> TestResult<PluginRunRecord> {
    let root = Root::new(name)?;
    let directory = root.plugin("fixture", &json!({}))?;
    let script = directory.join("action.sh");
    std::fs::write(&script, body)?;
    let mut action = sh_action("act", &script.to_string_lossy());
    if let Some(object) = input.as_object()
        && !object.is_empty()
    {
        set(&mut action, "argv", json!(["/bin/sh", script, "{text}"]))?;
        set(
            &mut action,
            "input_schema",
            json!({"type": "object", "additionalProperties": false,
            "properties": {"text": {"type": "string"}}, "required": ["text"]}),
        )?;
    }
    root.plugin("fixture", &manifest(true, &json!([action])))?;
    let plugins = root.plugins();
    let prepared = plugins.prepare(run_id(), &invoke("act", input)?)?;
    let (_cancel, cancelled) = watch::channel(false);
    Ok(plugins.run(prepared, cancelled).await?)
}

fn output(record: &PluginRunRecord) -> TestResult<Vec<u8>> {
    Ok(base64::engine::general_purpose::STANDARD.decode(&record.output_b64)?)
}

#[tokio::test(flavor = "multi_thread")]
async fn printed_ok_with_a_failing_exit_settles_as_failed() -> TestResult {
    let record = run_script("ok-exit-7", "printf 'OK\\n'\nexit 7\n", &json!({})).await?;

    assert_eq!(record.state, OpenEnum::Known(PluginRunState::Failed));
    assert_eq!(record.exit_code, Some(7));
    assert_eq!(output(&record)?, b"OK\n");
    assert_eq!(record.output_floor.get(), 0);
    assert!(!record.truncated);
    assert_eq!(
        record.created_terminals,
        Vec::<ovis_terminal_protocol::schema::ids::TerminalId>::new()
    );
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn output_beyond_the_bound_keeps_the_tail_and_its_floor() -> TestResult {
    let body = "i=0\nwhile [ $i -lt 1000 ]; do printf '%09d\\n' $i; i=$((i+1)); done\nprintf 'END\\n' >&2\n";

    let record = run_script("bounded", body, &json!({})).await?;

    let kept = output(&record)?;
    assert_eq!(record.state, OpenEnum::Known(PluginRunState::Succeeded));
    assert_eq!(kept.len(), OUTPUT_BYTES);
    assert_eq!(record.output_floor.get(), 10_004 - 4_096);
    assert!(record.truncated);
    assert!(kept.ends_with(b"000000999\nEND\n"));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn shell_operators_in_input_stay_one_literal_argument() -> TestResult {
    let hostile = "x; echo pwned $(id) | tee /tmp/ovt46-pwned && `id`";
    let body = "printf '%s\\n' \"$#\" \"$1\"\n";

    let record = run_script("inject", body, &json!({"text": hostile})).await?;

    assert_eq!(record.exit_code, Some(0));
    assert_eq!(output(&record)?, format!("1\n{hostile}\n").into_bytes());
    assert!(!std::path::Path::new("/tmp/ovt46-pwned").exists());
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn the_program_sees_only_the_allowlisted_environment() -> TestResult {
    let record = run_script("env", "/usr/bin/env\n", &json!({})).await?;

    let text = String::from_utf8(output(&record)?)?;
    let names: Vec<&str> = text
        .lines()
        .filter_map(|line| line.split_once('='))
        .map(|(name, _)| name)
        .collect();
    assert!(names.contains(&"OVIS_PLUGIN_RUN_ID"), "{text}");
    for name in names {
        assert!(
            ["PATH", "LANG", "PWD", "SHLVL", "_"].contains(&name)
                || name.starts_with("OVIS_PLUGIN_"),
            "unexpected variable {name}"
        );
    }
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn cancellation_kills_a_hung_program_and_records_cancelled() -> TestResult {
    let root = Root::new("hung")?;
    let directory = root.plugin("fixture", &json!({}))?;
    let script = directory.join("hang.sh");
    std::fs::write(&script, "exec sleep 600\n")?;
    root.plugin(
        "fixture",
        &manifest(true, &json!([sh_action("hang", &script.to_string_lossy())])),
    )?;
    let plugins = root.plugins();
    let prepared = plugins.prepare(run_id(), &invoke("hang", &json!({}))?)?;
    let (cancel, cancelled) = watch::channel(true);

    let record = tokio::time::timeout(
        std::time::Duration::from_secs(30),
        plugins.run(prepared, cancelled),
    )
    .await??;

    drop(cancel);
    assert_eq!(record.state, OpenEnum::Known(PluginRunState::Cancelled));
    assert_eq!(record.exit_code, None);
    assert_eq!(plugins.live_record(&run_id())?, Some(record));
    Ok(())
}

#[tokio::test(flavor = "multi_thread")]
async fn a_live_run_links_created_terminals_until_it_settles() -> TestResult {
    let root = Root::new("links")?;
    root.plugin(
        "fixture",
        &manifest(true, &json!([sh_action("ok", "/dev/null")])),
    )?;
    let plugins = root.plugins();
    let prepared = plugins.prepare(run_id(), &invoke("ok", &json!({}))?)?;
    let (_cancel, cancelled) = watch::channel(false);
    plugins.run(prepared, cancelled).await?;

    let late = plugins.link_created(
        &run_id(),
        ovis_terminal_protocol::schema::ids::TerminalId::new("t9".to_owned()),
    );

    assert_eq!(late.map_err(|error| error.kind()), Err("not_found"));
    Ok(())
}
