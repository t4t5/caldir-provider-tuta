use std::io::Write;
use std::process::{Command, Stdio};

use anyhow::{Context, Result, bail};
use serde_json::{Value, json};
use tempfile::TempDir;

#[test]
fn provider_crud_against_dedicated_account() -> Result<()> {
    let Ok(email) = std::env::var("TUTA_TEST_EMAIL") else {
        eprintln!("skipping integration test: TUTA_TEST_EMAIL is not set");
        return Ok(());
    };
    let passphrase =
        std::env::var("TUTA_TEST_PASSPHRASE").context("TUTA_TEST_PASSPHRASE must be set")?;
    let storage = TempDir::new()?;

    let connect = rpc(
        storage.path(),
        json!({
            "command": "connect",
            "params": {
                "options": {},
                "data": {
                    "email": email,
                    "passphrase": passphrase
                }
            }
        }),
    )?;
    let account = success_data(&connect)?["account_identifier"]
        .as_str()
        .context("connect response has no account_identifier")?
        .to_string();
    let calendars = rpc(
        storage.path(),
        json!({
            "command": "list_calendars",
            "params": { "account_identifier": account }
        }),
    )?;
    let calendar = success_data(&calendars)?
        .as_array()
        .and_then(|calendars| calendars.first())
        .context("test account has no calendar")?;
    let calendar_id = calendar["remote"]["tuta_calendar"]
        .as_str()
        .context("calendar has no tuta_calendar")?
        .to_string();
    let remote = json!({
        "tuta_account": account,
        "tuta_calendar": calendar_id,
    });
    let uid = format!(
        "caldir-tuta-integration-{}@example.invalid",
        std::process::id()
    );
    let initial = event_ics(&uid, "Created", "20260718T100000Z", "20260718T110000Z");
    let created = event_rpc(storage.path(), "create_event", &remote, &initial)?;
    assert!(created.contains("X-TUTA-ITEM:"));

    let listed = list_events(storage.path(), &remote)?;
    assert!(listed.iter().any(|event| event.contains(&uid)));

    let updated = event_rpc(
        storage.path(),
        "update_event",
        &remote,
        &created.replace("SUMMARY:Created", "SUMMARY:Updated"),
    )?;
    assert!(updated.contains("SUMMARY:Updated"));

    let rescheduled = event_rpc(
        storage.path(),
        "update_event",
        &remote,
        &updated
            .replace("20260718T100000Z", "20260718T120000Z")
            .replace("20260718T110000Z", "20260718T130000Z"),
    )?;
    assert!(rescheduled.contains("20260718T120000Z"));

    event_rpc(storage.path(), "delete_event", &remote, &rescheduled)?;
    assert!(
        !list_events(storage.path(), &remote)?
            .iter()
            .any(|event| event.contains(&uid))
    );
    Ok(())
}

fn event_ics(uid: &str, summary: &str, start: &str, end: &str) -> String {
    format!(
        "BEGIN:VCALENDAR\r\nVERSION:2.0\r\nPRODID:CALDIR\r\nBEGIN:VEVENT\r\nUID:{uid}\r\nDTSTART:{start}\r\nDTEND:{end}\r\nSUMMARY:{summary}\r\nEND:VEVENT\r\nEND:VCALENDAR\r\n"
    )
}

fn list_events(storage: &std::path::Path, remote: &Value) -> Result<Vec<String>> {
    let response = rpc(
        storage,
        json!({
            "command": "list_events",
            "params": {
                "tuta_account": remote["tuta_account"],
                "tuta_calendar": remote["tuta_calendar"],
                "from": "2026-01-01T00:00:00+00:00",
                "to": "2027-01-01T00:00:00+00:00"
            }
        }),
    )?;
    success_data(&response)?
        .as_array()
        .context("list_events response is not an array")?
        .iter()
        .map(|value| {
            value
                .as_str()
                .map(str::to_string)
                .context("list_events entry is not ICS")
        })
        .collect()
}

fn event_rpc(
    storage: &std::path::Path,
    command: &str,
    remote: &Value,
    event: &str,
) -> Result<String> {
    let response = rpc(
        storage,
        json!({
            "command": command,
            "params": {
                "tuta_account": remote["tuta_account"],
                "tuta_calendar": remote["tuta_calendar"],
                "event": event
            }
        }),
    )?;
    if command == "delete_event" {
        success_data(&response)?;
        return Ok(String::new());
    }
    success_data(&response)?
        .as_str()
        .map(str::to_string)
        .context("event response is not ICS")
}

fn rpc(storage: &std::path::Path, request: Value) -> Result<Value> {
    let mut child = Command::new(env!("CARGO_BIN_EXE_caldir-provider-tuta"))
        .env("CALDIR_PROVIDER_STORAGE_DIR", storage)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .context("Failed to spawn provider")?;
    writeln!(
        child.stdin.as_mut().context("Provider stdin unavailable")?,
        "{request}"
    )?;
    let output = child.wait_with_output()?;
    if !output.status.success() {
        bail!(
            "provider exited with {}: {}",
            output.status,
            String::from_utf8_lossy(&output.stderr)
        );
    }
    serde_json::from_slice(&output.stdout).with_context(|| {
        format!(
            "Provider returned invalid JSON: stdout={}, stderr={}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        )
    })
}

fn success_data(response: &Value) -> Result<&Value> {
    if response["status"] != "success" {
        bail!("provider returned error: {response}");
    }
    Ok(&response["data"])
}
