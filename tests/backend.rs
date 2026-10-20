#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use factorio_control::{App, api, models::*, process, storage};
use std::{
    fs,
    io::Write,
    os::unix::fs::{PermissionsExt, symlink},
    time::Duration,
};
use tower::ServiceExt;

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn app_error(error: factorio_control::Error) -> std::io::Error {
    std::io::Error::other(error.message)
}
#[test]
fn settings_preserve_unknown_fields_and_reject_stale_edits() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("settings.json");
    fs::write(
        &path,
        r#"{"name":"Original","token":"synthetic","visibility":{"public":false,"lan":true,"future":7},"future":{"x":1}}"#,
    )?;
    let mut document = storage::settings(&path).map_err(app_error)?;
    let stale = document.clone();
    document.settings.name = "Changed".into();
    let updated = storage::update_settings(&path, document).map_err(app_error)?;
    assert_eq!(updated.settings.name, "Changed");
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    assert_eq!(persisted["token"], "synthetic");
    assert_eq!(persisted["future"]["x"], 1);
    assert_eq!(persisted["visibility"]["future"], 7);
    let before = fs::read(&path)?;
    assert!(storage::update_settings(&path, stale).is_err());
    assert_eq!(fs::read(&path)?, before);
    fs::write(&path, b"invalid JSON")?;
    assert!(storage::settings(&path).is_err());
    assert_eq!(fs::read(&path)?, b"invalid JSON");
    Ok(())
}
#[test]
fn admins_are_separate_and_validation_preserves_working_file() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("server-adminlist.json");
    let mut document = storage::admins(&path).map_err(app_error)?;
    document.admins = vec!["Szer".into(), "paragith".into()];
    let mut saved = storage::update_admins(&path, document).map_err(app_error)?;
    assert_eq!(saved.admins.len(), 2);
    let before = fs::read(&path)?;
    saved.admins.push("szer".into());
    assert!(storage::update_admins(&path, saved).is_err());
    assert_eq!(fs::read(&path)?, before);
    Ok(())
}
#[test]
fn rejects_traversal_symlinks_and_invalid_archives() -> TestResult {
    let dir = tempfile::tempdir()?;
    assert!(storage::save_path(dir.path(), "../outside.zip").is_err());
    assert!(storage::save_path(dir.path(), "x\\outside.zip").is_err());
    let outside = dir.path().join("outside");
    fs::write(&outside, "unchanged")?;
    symlink(&outside, dir.path().join("link.zip"))?;
    assert!(storage::save_path(dir.path(), "link.zip").is_err());
    assert!(storage::atomic_write(&dir.path().join("link.zip"), b"new", true).is_err());
    assert_eq!(fs::read(&outside)?, b"unchanged");
    assert!(storage::validate_save(b"bad archive").is_err());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("world/level.dat0", zip::write::SimpleFileOptions::default())?;
    zip.write_all(b"synthetic world")?;
    let bytes = zip.finish()?.into_inner();
    storage::validate_save(&bytes).map_err(app_error)?;
    Ok(())
}
#[tokio::test]
async fn api_rejects_cross_origin_and_bad_content_types() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let router = api::router(app, dir.path());
    for (origin, content_type, expected) in [
        (
            "https://example.invalid",
            "application/json",
            StatusCode::FORBIDDEN,
        ),
        (
            "http://localhost:9400",
            "text/plain",
            StatusCode::UNSUPPORTED_MEDIA_TYPE,
        ),
        (
            "http://localhost:9400",
            "application/json",
            StatusCode::CONFLICT,
        ),
    ] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/server/stop")
            .header("Host", "localhost:9400")
            .header("Origin", origin)
            .header("Content-Type", content_type)
            .body(Body::from("{}"))?;
        let response = router.clone().oneshot(request).await?;
        assert_eq!(response.status(), expected);
        let bytes = to_bytes(response.into_body(), 4096).await?;
        let error: ApiError = serde_json::from_slice(&bytes)?;
        assert!(!error.message.is_empty());
    }
    let request = Request::builder()
        .uri("/api/status")
        .header("Host", "evil.invalid")
        .body(Body::empty())?;
    assert_eq!(
        router.oneshot(request).await?.status(),
        StatusCode::FORBIDDEN
    );
    Ok(())
}
#[tokio::test]
async fn process_reports_readiness_and_stops_owned_child() -> TestResult {
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("fake-factorio");
    fs::write(
        &script,
        "#!/bin/sh\ntrap 'exit 0' INT\necho 'Info ServerMultiplayerManager.cpp:809: changing state from(CreatingGame) to(InGame)'\nwhile :; do sleep 0.1; done\n",
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let app = App::new(dir.path().join("data"), Some(script)).map_err(app_error)?;
    fs::write(app.saves_path().join("world.zip"), b"synthetic")?;
    fs::write(
        app.settings_path(),
        r#"{"game_password":"InGame","auto_pause":false}"#,
    )?;
    let starting = process::start(&app, "world.zip").await.map_err(app_error)?;
    assert_eq!(starting.state, ServerState::Starting);
    assert!(process::start(&app, "world.zip").await.is_err());
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        if app.inner.lock().await.status.state == ServerState::Running {
            break;
        }
        assert!(
            tokio::time::Instant::now() < deadline,
            "readiness was not observed"
        );
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert_eq!(
        process::stop(&app).await.map_err(app_error)?.state,
        ServerState::Stopping
    );
    loop {
        process::poll(&app).await;
        if app.inner.lock().await.status.state == ServerState::Stopped {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline, "child did not exit");
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    assert!(app.inner.lock().await.child.is_none());
    Ok(())
}
#[tokio::test]
async fn process_failure_is_reported_and_logs_are_bounded() -> TestResult {
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("fake-factorio");
    fs::write(&script, "#!/bin/sh\necho 'startup failed' >&2\nexit 2\n")?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let app = App::new(dir.path().join("data"), Some(script)).map_err(app_error)?;
    fs::write(app.saves_path().join("world.zip"), b"synthetic")?;
    process::start(&app, "world.zip").await.map_err(app_error)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        process::poll(&app).await;
        if app.inner.lock().await.status.state == ServerState::Failed {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let mut inner = app.inner.lock().await;
    assert!(inner.status.error.is_some());
    for _ in 0..2100 {
        factorio_control::log(&mut inner, LogLevel::Info, "bounded".into());
    }
    assert_eq!(inner.logs.len(), 2000);
    Ok(())
}

#[tokio::test]
async fn upload_is_validated_never_overwrites_and_downloads_exact_bytes() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().join("data"), None).map_err(app_error)?;
    let router = api::router(app, dir.path());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("world/level.dat0", zip::write::SimpleFileOptions::default())?;
    zip.write_all(b"synthetic world")?;
    let bytes = zip.finish()?.into_inner();
    for expected in [StatusCode::CREATED, StatusCode::CONFLICT] {
        let request = Request::builder()
            .method("POST")
            .uri("/api/saves/world.zip")
            .header("Host", "localhost:9400")
            .header("Origin", "http://localhost:9400")
            .header("Content-Type", "application/zip")
            .body(Body::from(bytes.clone()))?;
        assert_eq!(router.clone().oneshot(request).await?.status(), expected);
    }
    let request = Request::builder()
        .uri("/api/saves/world.zip")
        .header("Host", "localhost:9400")
        .body(Body::empty())?;
    let response = router.clone().oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::OK);
    assert_eq!(
        to_bytes(response.into_body(), 65536).await?.as_ref(),
        bytes.as_slice()
    );
    let request = Request::builder()
        .uri("/api/missing")
        .header("Host", "localhost:9400")
        .body(Body::empty())?;
    let response = router.oneshot(request).await?;
    assert_eq!(response.status(), StatusCode::NOT_FOUND);
    let _: ApiError = serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
    Ok(())
}

fn save_upload_request(name: &str, body: Body) -> Result<Request<Body>, axum::http::Error> {
    Request::builder()
        .method("POST")
        .uri(format!("/api/saves/{name}"))
        .header("Host", "localhost:9400")
        .header("Origin", "http://localhost:9400")
        .header("Content-Type", "application/zip")
        .body(body)
}

async fn wait_for_upload(directory: &std::path::Path) -> TestResult {
    tokio::time::timeout(Duration::from_secs(5), async {
        loop {
            if fs::read_dir(directory)?.next().is_some() {
                return Ok::<_, std::io::Error>(());
            }
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await??;
    Ok(())
}

#[tokio::test]
async fn save_upload_streams_two_hundred_mib_without_content_length() -> TestResult {
    let dir = tempfile::tempdir()?;
    let source = tempfile::NamedTempFile::new_in(dir.path())?;
    let mut zip = zip::ZipWriter::new(source.reopen()?);
    zip.start_file(
        "world/level.dat0",
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored),
    )?;
    let chunk = [0_u8; 256 * 1024];
    for _ in 0..800 {
        zip.write_all(&chunk)?;
    }
    let file = zip.finish()?;
    let size = file.metadata()?.len();
    let app = App::new(dir.path().join("data"), None).map_err(app_error)?;
    let destination = app.saves_path().join("large.zip");
    let router = api::router(app, dir.path());
    let stream = tokio_util::io::ReaderStream::with_capacity(
        tokio::fs::File::open(source.path()).await?,
        256 * 1024,
    );
    let response = router
        .oneshot(save_upload_request("large.zip", Body::from_stream(stream))?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    assert_eq!(fs::metadata(&destination)?.len(), size);
    storage::validate_save_reader(fs::File::open(destination)?).map_err(app_error)?;
    Ok(())
}

#[tokio::test]
async fn save_upload_rechecks_conflicts_without_holding_app_lock() -> TestResult {
    use tokio::io::AsyncWriteExt;
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().join("data"), None).map_err(app_error)?;
    let saves = app.saves_path();
    let router = api::router(app.clone(), dir.path());
    let mut zip = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    zip.start_file("world/level.dat0", zip::write::SimpleFileOptions::default())?;
    zip.write_all(b"synthetic world")?;
    let bytes = zip.finish()?.into_inner();
    let (mut writer, reader) = tokio::io::duplex(1024);
    let request = save_upload_request(
        "world.zip",
        Body::from_stream(tokio_util::io::ReaderStream::new(reader)),
    )?;
    let uploading = tokio::spawn(router.clone().oneshot(request));
    writer.write_all(&bytes[..8]).await?;
    wait_for_upload(&saves).await?;
    let guard = tokio::time::timeout(Duration::from_secs(1), app.inner.lock()).await?;
    drop(guard);
    let response = router
        .oneshot(save_upload_request("world.zip", Body::from(bytes.clone()))?)
        .await?;
    assert_eq!(response.status(), StatusCode::CREATED);
    writer.write_all(&bytes[8..]).await?;
    drop(writer);
    assert_eq!(uploading.await??.status(), StatusCode::CONFLICT);
    assert_eq!(fs::read(saves.join("world.zip"))?, bytes);
    assert_eq!(fs::read_dir(saves)?.count(), 1);
    Ok(())
}

#[tokio::test]
async fn cancelled_invalid_and_declared_oversize_uploads_leave_no_files() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().join("data"), None).map_err(app_error)?;
    let saves = app.saves_path();
    let router = api::router(app, dir.path());
    let (writer, reader) = tokio::io::duplex(1024);
    let request = save_upload_request(
        "cancelled.zip",
        Body::from_stream(tokio_util::io::ReaderStream::new(reader)),
    )?;
    let uploading = tokio::spawn(router.clone().oneshot(request));
    wait_for_upload(&saves).await?;
    uploading.abort();
    assert!(uploading.await.is_err());
    drop(writer);
    assert_eq!(fs::read_dir(&saves)?.count(), 0);
    let invalid = save_upload_request("invalid.zip", Body::from("Not a ZIP"))?;
    assert_eq!(
        router.clone().oneshot(invalid).await?.status(),
        StatusCode::BAD_REQUEST
    );
    assert_eq!(fs::read_dir(&saves)?.count(), 0);
    let mut oversize = save_upload_request("oversize.zip", Body::empty())?;
    oversize.headers_mut().insert(
        "Content-Length",
        (storage::MAX_SAVE_BYTES + 1).to_string().parse()?,
    );
    assert_eq!(
        router.oneshot(oversize).await?.status(),
        StatusCode::PAYLOAD_TOO_LARGE
    );
    assert_eq!(fs::read_dir(saves)?.count(), 0);
    Ok(())
}

#[tokio::test]
async fn hosting_socket_is_not_readiness_and_config_secrets_are_redacted() -> TestResult {
    let dir = tempfile::tempdir()?;
    let script = dir.path().join("fake-factorio");
    fs::write(
        &script,
        "#!/bin/sh\necho 'Hosting game at IP ADDR:({0.0.0.0:34197})'\necho '   0.01 Error synthetic-token-value synthetic-password-value'\nsleep 0.2\nexit 1\n",
    )?;
    fs::set_permissions(&script, fs::Permissions::from_mode(0o700))?;
    let app = App::new(dir.path().join("data"), Some(script)).map_err(app_error)?;
    fs::write(app.saves_path().join("world.ZIP"), b"synthetic")?;
    fs::write(
        app.settings_path(),
        r#"{"token":"synthetic-token-value","game_password":"synthetic-password-value","require_user_verification":false}"#,
    )?;
    process::start(&app, "world.ZIP").await.map_err(app_error)?;
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(app.settings_path())?)?;
    assert_eq!(persisted["name"], "My Factorio server");
    assert_eq!(persisted["require_user_verification"], false);
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        process::poll(&app).await;
        let state = app.inner.lock().await.status.state.clone();
        assert_ne!(state, ServerState::Running);
        if state == ServerState::Failed {
            break;
        }
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    let inner = app.inner.lock().await;
    assert!(
        inner
            .logs
            .iter()
            .any(|line| line.message.contains("[redacted]"))
    );
    for entry in &inner.logs {
        assert!(!entry.message.contains("synthetic-token-value"));
        assert!(!entry.message.contains("synthetic-password-value"));
    }
    Ok(())
}

#[test]
fn validation_matches_visible_form_limits() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("settings.json");
    let mut document = storage::settings(&path).map_err(app_error)?;
    document.settings.name = "界".repeat(100);
    document.settings.description = "界".repeat(2000);
    document.settings.autosave_interval = 0;
    let mut saved = storage::update_settings(&path, document).map_err(app_error)?;
    saved.settings.name.push('界');
    assert!(storage::update_settings(&path, saved).is_err());
    let path = dir.path().join("admins.json");
    for invalid in ["name with space", "name,other", "name\tother"] {
        let mut document = storage::admins(&path).map_err(app_error)?;
        document.admins.push(invalid.into());
        assert!(storage::update_admins(&path, document).is_err());
    }
    let mut document = storage::admins(&path).map_err(app_error)?;
    document.admins.push("界".repeat(64));
    storage::update_admins(&path, document).map_err(app_error)?;
    Ok(())
}

#[test]
fn raw_and_typed_settings_share_revision_and_preserve_unknown_fields() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("server-settings.json");
    fs::write(
        &path,
        r#"{"name":"Factory","future":{"enabled":true},"visibility":{"public":false,"future":9}}"#,
    )?;
    let raw = storage::raw_settings(&path).map_err(app_error)?;
    assert_eq!(raw.settings["autosave_slots"], 5);
    assert_eq!(raw.settings["visibility"]["lan"], true);
    assert_eq!(raw.settings["visibility"]["future"], 9);
    let mut typed = storage::settings(&path).map_err(app_error)?;
    assert_eq!(raw.revision, typed.revision);
    typed.settings.tags = vec!["Space age".into()];
    typed.settings.auto_pause_when_players_connect = true;
    typed.settings.allow_commands = AllowCommands::Everyone(false);
    storage::update_settings(&path, typed).map_err(app_error)?;
    assert!(storage::update_raw_settings(&path, raw).is_err());
    let mut raw = storage::raw_settings(&path).map_err(app_error)?;
    assert_eq!(raw.settings["future"]["enabled"], true);
    assert_eq!(raw.settings["allow_commands"], false);
    raw.settings.remove("future");
    raw.settings
        .insert("new_option".into(), serde_json::json!([1, 2]));
    storage::update_raw_settings(&path, raw).map_err(app_error)?;
    let persisted: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    assert!(persisted.get("future").is_none());
    assert_eq!(persisted["new_option"], serde_json::json!([1, 2]));
    Ok(())
}

#[test]
fn raw_validation_preserves_working_file_and_can_repair_invalid_known_types() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("server-settings.json");
    fs::write(&path, r#"{"name":"Factory","max_players":"bad"}"#)?;
    assert!(storage::settings(&path).is_err());
    let mut raw = storage::raw_settings(&path).map_err(app_error)?;
    raw.settings
        .insert("max_players".into(), serde_json::json!(20));
    storage::update_raw_settings(&path, raw).map_err(app_error)?;
    let before = fs::read(&path)?;
    for (key, value) in [
        ("max_heartbeats_per_second", serde_json::json!(5)),
        ("max_heartbeats_per_second", serde_json::json!(241)),
        ("autosave_interval", serde_json::json!(-1)),
        ("tags", serde_json::json!("not an array")),
        ("allow_commands", serde_json::json!("everyone")),
        ("visibility", serde_json::json!(null)),
        ("token", serde_json::json!(false)),
    ] {
        let mut raw = storage::raw_settings(&path).map_err(app_error)?;
        raw.settings.insert(key.into(), value);
        assert!(
            storage::update_raw_settings(&path, raw).is_err(),
            "accepted {key}"
        );
        assert_eq!(fs::read(&path)?, before);
    }
    Ok(())
}

#[test]
fn selected_save_persists_and_rejects_stale_missing_or_unsafe_selection() -> TestResult {
    let dir = tempfile::tempdir()?;
    let data = dir.path().join("data");
    let app = App::new(data.clone(), None).map_err(app_error)?;
    fs::write(app.saves_path().join("factory.zip"), b"synthetic")?;
    let mut document = storage::save_selection(&app.save_selection_path()).map_err(app_error)?;
    assert!(document.save.is_none());
    let stale = document.clone();
    document.save = Some("factory.zip".into());
    storage::update_save_selection(&app.save_selection_path(), &app.saves_path(), document)
        .map_err(app_error)?;
    let restarted = App::new(data, None).map_err(app_error)?;
    let document = storage::save_selection(&restarted.save_selection_path()).map_err(app_error)?;
    assert_eq!(document.save.as_deref(), Some("factory.zip"));
    assert!(
        storage::update_save_selection(&app.save_selection_path(), &app.saves_path(), stale)
            .is_err()
    );
    for name in ["missing.zip", "../outside.zip"] {
        let mut invalid = document.clone();
        invalid.save = Some(name.into());
        assert!(
            storage::update_save_selection(&app.save_selection_path(), &app.saves_path(), invalid)
                .is_err()
        );
        assert_eq!(
            storage::save_selection(&app.save_selection_path())
                .map_err(app_error)?
                .save
                .as_deref(),
            Some("factory.zip")
        );
    }
    assert!(serde_json::from_str::<SaveSelectionDocument>(r#"{"revision":"missing"}"#).is_err());
    let mut clear = document;
    clear.save = None;
    assert!(
        storage::update_save_selection(&app.save_selection_path(), &app.saves_path(), clear)
            .map_err(app_error)?
            .save
            .is_none()
    );
    Ok(())
}

#[test]
fn manager_preferences_persist_validate_and_detect_conflicts() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("manager-settings.json");
    let mut document = storage::manager_settings(&path).map_err(app_error)?;
    assert!(!document.settings.enable_script_telemetry);
    let stale = document.clone();
    document.settings.poll_interval_seconds = 5;
    document.settings.ups_history_minutes = 60;
    let saved = storage::update_manager_settings(&path, document).map_err(app_error)?;
    assert_eq!(saved.settings.poll_interval_seconds, 5);
    assert!(storage::update_manager_settings(&path, stale).is_err());
    let before = fs::read(&path)?;
    for (interval, history) in [(0, 10), (61, 10), (2, 0), (2, 1441)] {
        let mut invalid = saved.clone();
        invalid.settings.poll_interval_seconds = interval;
        invalid.settings.ups_history_minutes = history;
        assert!(storage::update_manager_settings(&path, invalid).is_err());
        assert_eq!(fs::read(&path)?, before);
    }
    Ok(())
}

#[tokio::test]
async fn configuration_routes_use_json_and_revision_conflicts() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().join("data"), None).map_err(app_error)?;
    let router = api::router(app, dir.path());
    for endpoint in [
        "/api/settings/raw",
        "/api/manager-settings",
        "/api/saves/selection",
    ] {
        let get = Request::builder()
            .uri(endpoint)
            .header("Host", "localhost:9400")
            .body(Body::empty())?;
        let response = router.clone().oneshot(get).await?;
        assert_eq!(response.status(), StatusCode::OK);
        let mut document: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 65536).await?)?;
        document["revision"] = serde_json::json!("stale");
        let put = Request::builder()
            .method("PUT")
            .uri(endpoint)
            .header("Host", "localhost:9400")
            .header("Content-Type", "application/json")
            .body(Body::from(serde_json::to_vec(&document)?))?;
        assert_eq!(
            router.clone().oneshot(put).await?.status(),
            StatusCode::CONFLICT
        );
    }
    Ok(())
}
