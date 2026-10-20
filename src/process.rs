use crate::{App, Error, Result, log, models::*, now, storage, telemetry};
use std::{os::unix::process::ExitStatusExt, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt},
    process::Command,
};

pub async fn start(app: &App, name: &str) -> Result<Status> {
    let mut inner = app.inner.lock().await;
    if inner.child.is_some() {
        return Err(Error::conflict("The server is already active"));
    }
    let save = storage::save_path(&app.saves_path(), name)?;
    if !save.is_file() {
        return Err(Error::bad("The selected save does not exist"));
    }
    let binary = app
        .binary
        .as_ref()
        .ok_or_else(|| Error::bad("Configure FC_FACTORIO_BIN before starting a server"))?;
    let settings = storage::settings(&app.settings_path())?;
    let auto_pause = settings.settings.auto_pause;
    storage::update_settings(&app.settings_path(), settings)?;
    let session = telemetry::Session::new(auto_pause)?;
    let mut redactions = storage::log_redactions(&app.settings_path())?;
    redactions.push(session.redaction());
    let redactions = std::sync::Arc::new(redactions);
    let admins = storage::admins(&app.admins_path())?;
    if admins.revision == "missing" {
        storage::update_admins(&app.admins_path(), admins)?;
    }
    // Factorio writes runtime files below the managed data directory.
    let config = app.directory.join("config.ini");
    storage::ensure_regular(&config)?;
    if !config.exists() {
        let data = app
            .directory
            .to_str()
            .ok_or_else(|| Error::bad("Data directory must be UTF-8"))?;
        if data.contains(['\r', '\n']) {
            return Err(Error::bad("Data directory contains a newline"));
        }
        let contents =
            format!("[path]\nread-data=__PATH__executable__/../../data\nwrite-data={data}\n");
        storage::atomic_write(&config, contents.as_bytes(), false)?;
    }
    std::fs::create_dir_all(app.directory.join("mods"))?;
    storage::ensure_directory(&app.directory.join("mods"))?;
    let mut command = Command::new(binary);
    session.configure(&mut command);
    command
        .current_dir(&app.directory)
        .args(["--start-server"])
        .arg(save)
        .arg("--mod-directory")
        .arg(app.directory.join("mods"))
        .arg("--server-settings")
        .arg(app.settings_path())
        .arg("--server-adminlist")
        .arg(app.admins_path())
        .arg("--config")
        .arg(config)
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    let mut child = match command.spawn() {
        Ok(child) => child,
        Err(error) => {
            inner.status.state = ServerState::Failed;
            inner.status.since = now();
            inner.status.error = Some(format!("Unable to launch Factorio: {error}"));
            log(
                &mut inner,
                LogLevel::Error,
                format!("Unable to launch Factorio: {error}"),
            );
            return Err(error.into());
        }
    };
    inner.generation = inner.generation.saturating_add(1);
    let generation = inner.generation;
    if let Some(stdout) = child.stdout.take() {
        tokio::spawn(pump(
            app.clone(),
            stdout,
            LogLevel::Info,
            generation,
            redactions.clone(),
        ));
    }
    if let Some(stderr) = child.stderr.take() {
        tokio::spawn(pump(
            app.clone(),
            stderr,
            LogLevel::Warn,
            generation,
            redactions,
        ));
    }
    inner.child = Some(child);
    inner.telemetry.begin(session);
    tokio::spawn(telemetry::run(app.clone(), generation));
    inner.stopping_at = None;
    inner.status.state = ServerState::Starting;
    inner.status.since = now();
    inner.status.active_save = Some(name.into());
    inner.status.started_at = Some(now());
    inner.status.error = None;
    log(&mut inner, LogLevel::Info, format!("Starting {name}"));
    Ok(inner.status.clone())
}
pub async fn stop(app: &App) -> Result<Status> {
    let mut inner = app.inner.lock().await;
    if inner.status.state == ServerState::Stopping {
        return Ok(inner.status.clone());
    }
    let Some(child) = inner.child.as_mut() else {
        return Err(Error::conflict("The server is not active"));
    };
    let pid = child
        .id()
        .ok_or_else(|| Error::conflict("The server has already exited"))?;
    let pid = i32::try_from(pid).map_err(|_| Error::bad("Invalid server process identifier"))?;
    nix::sys::signal::kill(
        nix::unistd::Pid::from_raw(pid),
        nix::sys::signal::Signal::SIGINT,
    )
    .map_err(|error| Error::from(std::io::Error::from_raw_os_error(error as i32)))?;
    inner.status.state = ServerState::Stopping;
    inner.status.since = now();
    inner.stopping_at = Some(std::time::Instant::now());
    log(
        &mut inner,
        LogLevel::Info,
        "Stopping server; waiting for Factorio to save and exit".into(),
    );
    Ok(inner.status.clone())
}
pub async fn monitor(app: App) {
    let mut timer = tokio::time::interval(Duration::from_millis(200));
    loop {
        timer.tick().await;
        poll(&app).await;
    }
}
pub async fn poll(app: &App) {
    let mut inner = app.inner.lock().await;
    let timed_out = inner
        .stopping_at
        .is_some_and(|t| t.elapsed() > Duration::from_secs(120));
    let Some(child) = inner.child.as_mut() else {
        return;
    };
    if timed_out {
        if let Err(error) = child.start_kill() {
            log(
                &mut inner,
                LogLevel::Error,
                format!("Unable to terminate server: {error}"),
            );
        } else {
            log(
                &mut inner,
                LogLevel::Warn,
                "Server did not stop within 120 seconds; terminated it".into(),
            );
        }
        inner.stopping_at = None;
        inner.status.error = Some("Server exceeded the graceful shutdown timeout".into());
    }
    let result = match inner.child.as_mut() {
        Some(child) => child.try_wait(),
        None => return,
    };
    match result {
        Ok(Some(exit)) => {
            let clean = exit.success()
                || (inner.status.state == ServerState::Stopping
                    && inner.status.error.is_none()
                    && exit.signal() == Some(nix::libc::SIGINT));
            inner.status.state = if clean {
                ServerState::Stopped
            } else {
                ServerState::Failed
            };
            if !clean && inner.status.error.is_none() {
                inner.status.error = Some(format!("Factorio exited with {exit}"));
            }
            inner.status.since = now();
            inner.telemetry.end();
            inner.telemetry.activity(
                telemetry::ActivityKind::Stopped,
                format!("Server {}", if clean { "stopped" } else { "failed" }),
            );
            inner.status.started_at = None;
            inner.child = None;
            inner.stopping_at = None;
            log(
                &mut inner,
                if clean {
                    LogLevel::Info
                } else {
                    LogLevel::Error
                },
                format!("Factorio exited with {exit}"),
            );
        }
        Ok(None) => (),
        Err(error) => {
            log(
                &mut inner,
                LogLevel::Error,
                format!("Unable to check server process: {error}"),
            );
        }
    }
}
async fn record(app: &App, level: LogLevel, bytes: &[u8], generation: u64, redactions: &[String]) {
    let mut message = String::from_utf8_lossy(bytes)
        .trim_end_matches('\r')
        .to_owned();
    let level = if message.contains(" Error ") {
        LogLevel::Error
    } else if message.contains(" Warning ") {
        LogLevel::Warn
    } else {
        level
    };
    for secret in redactions {
        message = message.replace(secret, "[redacted]");
    }
    let mut inner = app.inner.lock().await;
    if inner.generation != generation {
        return;
    }
    let player_event = inner.telemetry.observe_log(&message);
    if inner.telemetry.auto_pause_enabled()
        && player_event
        && matches!(
            inner.status.state,
            ServerState::Running | ServerState::Paused
        )
    {
        let state = if inner.telemetry.auto_paused() {
            ServerState::Paused
        } else {
            ServerState::Running
        };
        if inner.status.state != state {
            inner.status.state = state;
            inner.status.since = now();
        }
    }
    if inner.generation == generation
        && inner.status.state == ServerState::Starting
        && message.contains("ServerMultiplayerManager")
        && message.contains("to(InGame)")
    {
        inner.status.state = if inner.telemetry.auto_paused() {
            ServerState::Paused
        } else {
            ServerState::Running
        };
        inner.status.since = now();
        inner
            .telemetry
            .activity(telemetry::ActivityKind::Started, "Server started".into());
    }
    if inner.generation == generation
        && let Some((_, rest)) = message.split_once("Factorio ")
        && let Some(version) = rest.split_whitespace().next()
        && version.chars().all(|c| c.is_ascii_digit() || c == '.')
        && version.contains('.')
    {
        inner.status.version = Some(version.into());
    }
    log(&mut inner, level, message);
}
async fn pump(
    app: App,
    mut reader: impl AsyncRead + Unpin,
    level: LogLevel,
    generation: u64,
    redactions: std::sync::Arc<Vec<String>>,
) {
    let mut chunk = [0_u8; 4096];
    let mut line = Vec::new();
    loop {
        match reader.read(&mut chunk).await {
            Ok(0) => {
                if !line.is_empty() {
                    record(&app, level.clone(), &line, generation, &redactions).await;
                }
                break;
            }
            Ok(size) => {
                for byte in &chunk[..size] {
                    if *byte == b'\n' {
                        record(&app, level.clone(), &line, generation, &redactions).await;
                        line.clear();
                    } else if line.len() < 16384 {
                        line.push(*byte);
                    }
                }
            }
            Err(error) => {
                let mut inner = app.inner.lock().await;
                if inner.generation == generation {
                    log(
                        &mut inner,
                        LogLevel::Error,
                        format!("Unable to read Factorio output: {error}"),
                    );
                }
                break;
            }
        }
    }
}
pub async fn shutdown(app: &App) {
    let active = app.inner.lock().await.child.is_some();
    if !active {
        return;
    }
    if let Err(error) = stop(app).await {
        eprintln!("Graceful shutdown request failed: {}", error.message);
    }
    let deadline = tokio::time::Instant::now() + Duration::from_secs(125);
    while tokio::time::Instant::now() < deadline {
        poll(app).await;
        if app.inner.lock().await.child.is_none() {
            return;
        }
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    let mut inner = app.inner.lock().await;
    if let Some(mut child) = inner.child.take()
        && let Err(error) = child.kill().await
    {
        eprintln!("Final server shutdown failed: {error}");
    }
}
