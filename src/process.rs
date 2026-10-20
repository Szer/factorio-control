use crate::{App, Error, Result, log, models::*, now, storage, telemetry};
use std::{os::unix::process::ExitStatusExt, process::Stdio, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWriteExt},
    process::Command,
};

pub async fn start(app: &App, name: &str) -> Result<Status> {
    let _operation = app
        .save_operations
        .try_lock()
        .map_err(|_| Error::conflict("A save operation is in progress"))?;
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
    if let Some(version) = inner.status.version.as_deref() {
        crate::mods::validate_start(app, version)?;
    }
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
    let manager = storage::manager_settings(&app.manager_settings_path())?.settings;
    let mut command = Command::new(binary);
    session.configure(&mut command);
    command
        .current_dir(&app.directory)
        .args(["--start-server"])
        .arg(save)
        .arg("--bind")
        .arg(manager.game_bind_address)
        .arg("--port")
        .arg(manager.game_port.to_string())
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
    let original = message.clone();
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
        && original.contains("ServerMultiplayerManager")
        && original.contains("to(InGame)")
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
        && let Some((_, rest)) = original.split_once("Factorio ")
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

pub async fn start_selected(app: &App, name: Option<&str>) -> Result<Status> {
    let name = match name {
        Some(name) => name.to_owned(),
        None => {
            let _inner = app.inner.lock().await;
            storage::selected_save(
                &storage::save_selection(&app.save_selection_path())?,
                &app.saves_path(),
            )?
        }
    };
    start(app, &name).await
}
pub fn validate_command(input: &ConsoleCommandRequest) -> Result<()> {
    if input.command.trim().is_empty()
        || input.command.len() > 4096
        || input.command.chars().any(char::is_control)
    {
        return Err(Error::bad(
            "Command must be a single nonempty line of at most 4096 bytes without control characters",
        ));
    }
    let command = input
        .command
        .split_whitespace()
        .next()
        .unwrap_or_default()
        .to_ascii_lowercase();
    let safe = [
        "/help",
        "/h",
        "/players",
        "/p",
        "/admins",
        "/version",
        "/time",
        "/evolution",
        "/seed",
    ];
    if command.starts_with('/') && !safe.contains(&command.as_str()) && !input.acknowledge_scripting
    {
        return Err(Error::bad(
            "Acknowledge that console commands can change the world or disable achievements",
        ));
    }
    Ok(())
}
pub async fn send_command(app: &App, input: ConsoleCommandRequest) -> Result<CommandResponse> {
    validate_command(&input)?;
    let mut inner = app.inner.lock().await;
    if !matches!(
        inner.status.state,
        ServerState::Running | ServerState::Paused
    ) {
        return Err(Error::conflict("Commands require a running server"));
    }
    let stdin = inner
        .child
        .as_mut()
        .and_then(|child| child.stdin.as_mut())
        .ok_or_else(|| Error::conflict("Server console is unavailable"))?;
    let line = format!("{}\n", input.command);
    match tokio::time::timeout(Duration::from_secs(2), stdin.write_all(line.as_bytes())).await {
        Ok(Ok(())) => Ok(CommandResponse { accepted: true }),
        Ok(Err(_)) => {
            if let Some(child) = inner.child.as_mut() {
                child.stdin.take();
            }
            Err(Error::conflict(
                "Unable to write to the server console; restart before sending commands",
            ))
        }
        Err(_) => {
            // A timed out write may be partial; prevent a later command from completing it.
            if let Some(child) = inner.child.as_mut() {
                child.stdin.take();
            }
            Err(Error::conflict(
                "Server console write timed out; restart the server before sending commands",
            ))
        }
    }
}
pub async fn force_stop(app: &App, confirm: bool) -> Result<Status> {
    if !confirm {
        return Err(Error::bad(
            "Confirm emergency termination; unsaved progress will be lost",
        ));
    }
    let mut inner = app.inner.lock().await;
    let child = inner
        .child
        .as_mut()
        .ok_or_else(|| Error::conflict("The server is not active"))?;
    child.start_kill()?;
    inner.status.state = ServerState::Stopping;
    inner.status.since = now();
    inner.status.error =
        Some("Server was forcibly terminated; unsaved progress may be lost".into());
    inner.stopping_at = None;
    log(
        &mut inner,
        LogLevel::Warn,
        "Emergency server termination requested".into(),
    );
    Ok(inner.status.clone())
}
pub async fn create_save(app: &App, name: &str) -> Result<Save> {
    let _operation = app
        .save_operations
        .try_lock()
        .map_err(|_| Error::conflict("A save operation is in progress"))?;
    let temporary = {
        let inner = app.inner.lock().await;
        if inner.child.is_some() {
            return Err(Error::conflict("Stop the server before creating a save"));
        }
        let destination = storage::save_path(&app.saves_path(), name)?;
        if destination.exists() {
            return Err(Error::conflict("A save with that name already exists"));
        }
        if let Some(version) = inner.status.version.as_deref() {
            crate::mods::validate_start(app, version)?;
        }
        let temporary = tempfile::Builder::new()
            .prefix(".create-")
            .tempdir_in(&app.directory)?;
        let mods = temporary.path().join("mods");
        std::fs::create_dir(&mods)?;
        let source = app.directory.join("mods");
        if source.exists() {
            storage::ensure_directory(&source)?;
            for entry in std::fs::read_dir(source)? {
                let entry = entry?;
                let name = entry.file_name();
                let Some(name) = name.to_str() else {
                    continue;
                };
                if name.ends_with(".zip") || matches!(name, "mod-list.json" | "mod-settings.dat") {
                    storage::ensure_regular(&entry.path())?;
                    std::fs::copy(entry.path(), mods.join(name))?;
                }
            }
        }
        let data = temporary
            .path()
            .to_str()
            .ok_or_else(|| Error::bad("Data directory must be UTF-8"))?;
        if data.contains(['\n', '\r']) {
            return Err(Error::bad("Data directory contains a newline"));
        }
        storage::atomic_write(
            &temporary.path().join("config.ini"),
            format!("[path]\nread-data=__PATH__executable__/../../data\nwrite-data={data}\n")
                .as_bytes(),
            false,
        )?;
        temporary
    };
    let binary = app
        .binary
        .as_ref()
        .ok_or_else(|| Error::bad("Configure FC_FACTORIO_BIN before creating a save"))?;
    let output = temporary.path().join("world.zip");
    let mut child = Command::new(binary)
        .current_dir(temporary.path())
        .arg("--create")
        .arg(&output)
        .arg("--mod-directory")
        .arg(temporary.path().join("mods"))
        .arg("--config")
        .arg(temporary.path().join("config.ini"))
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::null())
        .kill_on_drop(true)
        .spawn()?;
    let exit = match tokio::time::timeout(Duration::from_secs(180), child.wait()).await {
        Ok(result) => result?,
        Err(_) => {
            child.kill().await?;
            return Err(Error::bad("World creation exceeded three minutes"));
        }
    };
    if !exit.success() {
        return Err(Error::bad(&format!(
            "Factorio could not create the world ({exit})"
        )));
    }
    storage::ensure_regular(&output)?;
    let mut file = std::fs::File::open(&output)?;
    if file.metadata()?.len() > storage::MAX_SAVE_BYTES {
        return Err(Error::bad("Created world exceeds 1 GiB"));
    }
    storage::validate_save_reader(&mut file)?;
    file.sync_all()?;
    let _inner = app.inner.lock().await;
    let destination = storage::save_path(&app.saves_path(), name)?;
    std::fs::hard_link(&output, destination).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::conflict("A save with that name already exists")
        } else {
            error.into()
        }
    })?;
    std::fs::File::open(app.saves_path())?.sync_all()?;
    storage::save_info(&app.saves_path(), name)
}
