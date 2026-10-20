#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
use std::{
    io::Write,
    process::{Command, Stdio},
};

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
