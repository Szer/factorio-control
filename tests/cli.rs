#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
use std::{
    io::Write,
    process::{Command, Stdio},
};

#[test]
fn invalid_session_lifetimes_fail_startup_without_bootstrapping_accounts()
-> Result<(), Box<dyn std::error::Error>> {
    for value in ["", "0", "-1", "1.5", "30days", "4294967296"] {
        let temporary = tempfile::tempdir()?;
        let data = temporary.path().join("data");
        let output = Command::new(env!("CARGO_BIN_EXE_factorio-control"))
            .env("FC_DATA_DIR", &data)
            .env("FC_BIND", "127.0.0.1:0")
            .env("FC_SESSION_TTL_SECONDS", value)
            .env_remove("FC_PUBLIC_URL")
            .env_remove("FC_TRUSTED_PROXIES")
            .env_remove("FC_FACTORIO_BIN")
            .output()?;
        assert!(!output.status.success());
        assert!(String::from_utf8_lossy(&output.stderr).contains("FC_SESSION_TTL_SECONDS"));
        assert!(!data.join("panel-accounts.json").exists());
        assert!(!data.join("bootstrap-admin.json").exists());
    }
    Ok(())
}

#[test]
fn offline_health_check_fails_without_creating_data() -> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let data = temporary.path().join("data");
    let output = Command::new(env!("CARGO_BIN_EXE_factorio-control"))
        .arg("--healthcheck")
        .env("FC_BIND", "127.0.0.1:0")
        .env("FC_DATA_DIR", &data)
        .env_remove("FC_PUBLIC_URL")
        .env_remove("FC_SESSION_TTL_SECONDS")
        .output()?;
    assert!(!output.status.success());
    assert!(!data.exists());
    Ok(())
}

#[test]
fn bootstrap_creates_a_fresh_data_directory_and_never_overwrites_accounts()
-> Result<(), Box<dyn std::error::Error>> {
    let temporary = tempfile::tempdir()?;
    let data = temporary.path().join("new-data");
    let run = || -> Result<std::process::Output, Box<dyn std::error::Error>> {
        let mut child = Command::new(env!("CARGO_BIN_EXE_factorio-control"))
            .args(["--create-admin", "fixture-admin"])
            .env("FC_DATA_DIR", &data)
            .env("FC_BIND", "127.0.0.1:0")
            .env_remove("FC_PUBLIC_URL")
            .env_remove("FC_SESSION_TTL_SECONDS")
            .env_remove("FC_FACTORIO_BIN")
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()?;
        let mut input = child.stdin.take().ok_or("Missing test input pipe")?;
        input.write_all(b"fixture-password-only-123\n")?;
        drop(input);
        Ok(child.wait_with_output()?)
    };
    let first = run()?;
    assert!(first.status.success());
    let file = data.join("panel-accounts.json");
    let original = std::fs::read(&file)?;
    assert!(!String::from_utf8_lossy(&original).contains("fixture-password-only-123"));
    let second = run()?;
    assert!(!second.status.success());
    assert!(original == std::fs::read(file)?);
    Ok(())
}

#[test]
fn first_start_bootstraps_without_printing_the_password() -> Result<(), Box<dyn std::error::Error>>
{
    let temporary = tempfile::tempdir()?;
    let data = temporary.path().join("data");
    let mut child = Command::new(env!("CARGO_BIN_EXE_factorio-control"))
        .env("FC_DATA_DIR", &data)
        .env("FC_BIND", "127.0.0.1:0")
        .env_remove("FC_PUBLIC_URL")
        .env_remove("FC_SESSION_TTL_SECONDS")
        .env_remove("FC_FACTORIO_BIN")
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()?;
    let deadline = std::time::Instant::now() + std::time::Duration::from_secs(5);
    let account_path = data.join("panel-accounts.json");
    while !account_path.exists() && std::time::Instant::now() < deadline {
        if child.try_wait()?.is_some() {
            break;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    if child.try_wait()?.is_none() {
        child.kill()?;
    }
    let output = child.wait_with_output()?;
    assert!(account_path.exists());
    let credentials: serde_json::Value =
        serde_json::from_slice(&std::fs::read(data.join("bootstrap-admin.json"))?)?;
    let password = credentials["password"]
        .as_str()
        .ok_or("Missing fixture password")?;
    assert!(!String::from_utf8_lossy(&output.stdout).contains(password));
    assert!(!String::from_utf8_lossy(&output.stderr).contains(password));
    Ok(())
}

#[test]
fn empty_optional_trusted_proxies_allow_main_to_listen() -> Result<(), Box<dyn std::error::Error>> {
    use std::io::{BufRead, BufReader};
    use std::sync::mpsc;
    use std::time::Duration;

    for (bind, value) in [("127.0.0.1:0", ""), ("0.0.0.0:0", "  ")] {
        let temporary = tempfile::tempdir()?;
        let mut child = Command::new(env!("CARGO_BIN_EXE_factorio-control"))
            .env("FC_DATA_DIR", temporary.path().join("data"))
            .env("FC_BIND", bind)
            .env("FC_TRUSTED_PROXIES", value)
            .env("FC_PUBLIC_URL", value)
            .env_remove("FC_SESSION_TTL_SECONDS")
            .env_remove("FC_FACTORIO_BIN")
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::piped())
            .spawn()?;
        let stderr = child.stderr.take().ok_or("Missing test error pipe")?;
        let (sender, receiver) = mpsc::channel();
        let reader = std::thread::spawn(move || {
            for line in BufReader::new(stderr).lines() {
                let Ok(line) = line else { break };
                if line.starts_with("Factorio Control listening on http://") {
                    let _sent = sender.send(true);
                    return;
                }
            }
            let _sent = sender.send(false);
        });
        let listening = receiver.recv_timeout(Duration::from_secs(10));
        let running = child.try_wait()?.is_none();
        if running {
            child.kill()?;
        }
        child.wait()?;
        reader.join().map_err(|_| "Startup output reader failed")?;
        assert!(
            matches!(listening, Ok(true)) && running,
            "Empty optional proxy configuration must reach a live listening server"
        );
    }
    Ok(())
}
