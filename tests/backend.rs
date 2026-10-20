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
    fs::write(app.settings_path(), r#"{"game_password":"InGame"}"#)?;
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
