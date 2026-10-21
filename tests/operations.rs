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
mod support;

type TestResult = Result<(), Box<dyn std::error::Error>>;
fn failure(error: factorio_control::Error) -> std::io::Error {
    std::io::Error::other(error.message)
}
fn world(path: &std::path::Path) -> TestResult {
    let mut archive = zip::ZipWriter::new(fs::File::create(path)?);
    archive.start_file("world/level.dat", zip::write::SimpleFileOptions::default())?;
    archive.write_all(b"synthetic")?;
    archive.finish()?;
    Ok(())
}
fn script(path: &std::path::Path, body: &str) -> TestResult {
    fs::write(path, format!("#!/bin/sh\n{body}\n"))?;
    fs::set_permissions(path, fs::Permissions::from_mode(0o700))?;
    Ok(())
}
async fn post(
    app: &App,
    path: &str,
    body: serde_json::Value,
) -> Result<(StatusCode, serde_json::Value), Box<dyn std::error::Error>> {
    let response = support::authenticated(api::router(app.clone(), &app.directory), &app.directory)
        .await?
        .oneshot(
            Request::builder()
                .method("POST")
                .uri(path)
                .header("Host", "localhost:9400")
                .header("Content-Type", "application/json")
                .body(Body::from(serde_json::to_vec(&body)?))?,
        )
        .await?;
    let status = response.status();
    let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
    Ok((status, serde_json::from_slice(&bytes)?))
}
#[test]
fn latest_mode_preserves_legacy_and_chooses_autosaves() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("saves"))?;
    let path = dir.path().join("selection.json");
    fs::write(&path, br#""world.zip""#)?;
    let selection = storage::save_selection(&path).map_err(failure)?;
    assert!(!selection.load_latest);
    assert_eq!(selection.save.as_deref(), Some("world.zip"));
    fs::write(dir.path().join("saves/world.zip"), b"world")?;
    fs::write(dir.path().join("saves/_autosave2.zip"), b"autosave")?;
    let latest = fs::File::open(dir.path().join("saves/_autosave2.zip"))?;
    latest.set_times(
        fs::FileTimes::new().set_modified(std::time::SystemTime::now() + Duration::from_secs(2)),
    )?;
    let selected = storage::update_save_selection(
        &path,
        &dir.path().join("saves"),
        SaveSelectionDocument {
            revision: selection.revision,
            save: None,
            load_latest: true,
        },
    )
    .map_err(failure)?;
    assert_eq!(
        storage::selected_save(&selected, &dir.path().join("saves")).map_err(failure)?,
        "_autosave2.zip"
    );
    assert!(
        storage::update_save_selection(
            &path,
            &dir.path().join("saves"),
            SaveSelectionDocument {
                save: Some("world.zip".into()),
                ..selected
            }
        )
        .is_err()
    );
    Ok(())
}
#[test]
fn archive_restore_preserves_bytes_revisions_and_collisions() -> TestResult {
    let dir = tempfile::tempdir()?;
    fs::create_dir(dir.path().join("saves"))?;
    let saves = dir.path().join("saves");
    world(&saves.join("world.zip"))?;
    let before = fs::read(saves.join("world.zip"))?;
    let original = storage::save_info(&saves, "world.zip").map_err(failure)?;
    assert!(storage::archive_save(dir.path(), "world.zip", "stale").is_err());
    let archived =
        storage::archive_save(dir.path(), "world.zip", &original.revision).map_err(failure)?;
    assert!(!saves.join("world.zip").exists());
    assert_eq!(
        storage::archived_saves(dir.path()).map_err(failure)?.len(),
        1
    );
    fs::write(saves.join("world.zip"), b"new world")?;
    assert!(storage::restore_save(dir.path(), &archived.id, &archived.revision).is_err());
    assert_eq!(fs::read(saves.join("world.zip"))?, b"new world");
    fs::remove_file(saves.join("world.zip"))?;
    assert!(storage::restore_save(dir.path(), &archived.id, "stale").is_err());
    let restored =
        storage::restore_save(dir.path(), &archived.id, &archived.revision).map_err(failure)?;
    assert_eq!(restored.revision, original.revision);
    assert_eq!(fs::read(saves.join("world.zip"))?, before);
    assert!(
        storage::archived_saves(dir.path())
            .map_err(failure)?
            .is_empty()
    );
    assert!(storage::restore_save(dir.path(), "../saves", "stale").is_err());
    Ok(())
}
#[test]
fn game_log_is_bounded_redacted_and_refuses_symlinks() -> TestResult {
    let dir = tempfile::tempdir()?;
    let settings = dir.path().join("server-settings.json");
    fs::write(
        &settings,
        br#"{"token":"synthetic-secret","game_password":"another-secret"}"#,
    )?;
    let log = dir.path().join("factorio-current.log");
    let mut content = "x".repeat(300 * 1024);
    content.push_str(
        "\nold token=expired-secret\nhello synthetic-secret another-secret\nready\u{0}\n",
    );
    fs::write(&log, content)?;
    let document = storage::game_log(dir.path(), &settings).map_err(failure)?;
    assert!(document.truncated);
    assert!(document.content.len() < 256 * 1024);
    assert!(!document.content.contains("secret"));
    assert!(!document.content.contains('\0'));
    assert!(document.content.ends_with("ready"));
    fs::remove_file(&log)?;
    symlink(&settings, &log)?;
    assert!(storage::game_log(dir.path(), &settings).is_err());
    Ok(())
}
#[test]
fn legacy_manager_defaults_network_and_validates_ip_port() -> TestResult {
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("manager.json");
    fs::write(
        &path,
        br#"{"enable_script_telemetry":false,"poll_interval_seconds":2,"ups_history_minutes":10}"#,
    )?;
    let mut document = storage::manager_settings(&path).map_err(failure)?;
    assert_eq!(document.settings.game_bind_address, "0.0.0.0");
    assert_eq!(document.settings.game_port, 34197);
    document.settings.game_port = 0;
    assert!(storage::update_manager_settings(&path, document.clone()).is_err());
    document.settings.game_port = 34198;
    document.settings.game_bind_address = "localhost".into();
    assert!(storage::update_manager_settings(&path, document.clone()).is_err());
    document.settings.game_bind_address = "::1".into();
    assert_eq!(
        storage::update_manager_settings(&path, document)
            .map_err(failure)?
            .settings
            .game_port,
        34198
    );
    Ok(())
}
#[test]
fn console_rejects_injection_and_requires_danger_acknowledgement() {
    for command in ["", " ", "x\ny", "x\ry", "x\0y", "x\ty", &"x".repeat(4097)] {
        assert!(
            process::validate_command(&ConsoleCommandRequest {
                command: command.into(),
                acknowledge_scripting: true
            })
            .is_err()
        );
    }
    for command in [
        "/c game.print('x')",
        "  /SC game.print('x')",
        "/command x",
        "/silent-command x",
        "/measured-command x",
        "/cheat",
        "/editor",
        "/unknown-future-alias x",
    ] {
        assert!(
            process::validate_command(&ConsoleCommandRequest {
                command: command.into(),
                acknowledge_scripting: false
            })
            .is_err()
        );
        assert!(
            process::validate_command(&ConsoleCommandRequest {
                command: command.into(),
                acknowledge_scripting: true
            })
            .is_ok()
        );
    }
    assert!(
        process::validate_command(&ConsoleCommandRequest {
            command: "/players".into(),
            acknowledge_scripting: false
        })
        .is_ok()
    );
}
#[tokio::test]
async fn api_archiving_requires_stopped_unselected_and_current_revision() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(failure)?;
    world(&app.saves_path().join("world.zip"))?;
    let save = storage::save_info(&app.saves_path(), "world.zip").map_err(failure)?;
    let selection = storage::save_selection(&app.save_selection_path()).map_err(failure)?;
    let selected = storage::update_save_selection(
        &app.save_selection_path(),
        &app.saves_path(),
        SaveSelectionDocument {
            save: Some("world.zip".into()),
            ..selection
        },
    )
    .map_err(failure)?;
    let (status, _) = post(
        &app,
        "/api/saves/world.zip/archive",
        serde_json::json!({"revision":save.revision}),
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    storage::update_save_selection(
        &app.save_selection_path(),
        &app.saves_path(),
        SaveSelectionDocument {
            save: None,
            ..selected
        },
    )
    .map_err(failure)?;
    let (status, archived) = post(
        &app,
        "/api/saves/world.zip/archive",
        serde_json::json!({"revision":save.revision}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let archive: ArchivedSave = serde_json::from_value(archived)?;
    let (status, _) = post(
        &app,
        &format!("/api/saves/archived/{}/restore", archive.id),
        serde_json::json!({"revision":archive.revision}),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    Ok(())
}
#[tokio::test]
async fn create_world_uses_isolated_runtime_no_overwrite_and_validates_output() -> TestResult {
    let dir = tempfile::tempdir()?;
    world(&dir.path().join("fixture.zip"))?;
    let binary = dir.path().join("fake-factorio");
    script(&binary, &format!("test \\\"$1\\\" = --create || exit 4\ncp '{}' \\\"$2\\\"\nprintf changed > mods/mod-list.json", dir.path().join("fixture.zip").display()).replace("\\\"", "\""))?;
    let app = App::new(dir.path().join("data"), Some(binary.clone())).map_err(failure)?;
    fs::create_dir_all(app.directory.join("mods"))?;
    fs::write(app.directory.join("mods/mod-list.json"), br#"{"mods":[]}"#)?;
    let (status, _) = post(
        &app,
        "/api/saves/create",
        serde_json::json!({"name":"new.zip"}),
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    storage::validate_save(&fs::read(app.saves_path().join("new.zip"))?).map_err(failure)?;
    assert_eq!(
        fs::read(app.directory.join("mods/mod-list.json"))?,
        br#"{"mods":[]}"#
    );
    assert!(process::create_save(&app, "new.zip").await.is_err());
    assert!(process::create_save(&app, "../bad.zip").await.is_err());
    script(&binary, "printf 'not a zip' > \"$2\"")?;
    assert!(process::create_save(&app, "bad.zip").await.is_err());
    assert!(!app.saves_path().join("bad.zip").exists());
    assert_eq!(
        fs::read_dir(&app.directory)?
            .filter_map(Result::ok)
            .filter(|entry| entry.file_name().to_string_lossy().starts_with(".create-"))
            .count(),
        0
    );
    Ok(())
}
#[tokio::test]
async fn commands_and_force_stop_only_touch_owned_running_child() -> TestResult {
    let dir = tempfile::tempdir()?;
    let binary = dir.path().join("fake-factorio");
    script(
        &binary,
        "printf '%s\\n' \"$@\" > args.txt\necho 'ServerMultiplayerManager to(InGame)'\nwhile IFS= read -r line; do printf '%s\\n' \"$line\" >> commands.txt; done",
    )?;
    let app = App::new(dir.path().join("data"), Some(binary)).map_err(failure)?;
    fs::write(app.saves_path().join("world.zip"), b"synthetic")?;
    let command = ConsoleCommandRequest {
        command: "/players".into(),
        acknowledge_scripting: false,
    };
    assert!(process::send_command(&app, command.clone()).await.is_err());
    assert!(process::force_stop(&app, true).await.is_err());
    let mut manager = storage::manager_settings(&app.manager_settings_path()).map_err(failure)?;
    manager.settings.game_bind_address = "127.0.0.1".into();
    manager.settings.game_port = 34199;
    storage::update_manager_settings(&app.manager_settings_path(), manager).map_err(failure)?;
    fs::write(app.settings_path(), br#"{"auto_pause":false}"#)?;
    process::start(&app, "world.zip").await.map_err(failure)?;
    let running_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while app.inner.lock().await.status.state != ServerState::Running {
        assert!(tokio::time::Instant::now() < running_deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert!(process::create_save(&app, "other.zip").await.is_err());
    assert!(process::force_stop(&app, false).await.is_err());
    let save = storage::save_info(&app.saves_path(), "world.zip").map_err(failure)?;
    assert_eq!(
        post(
            &app,
            "/api/saves/world.zip/archive",
            serde_json::json!({"revision":save.revision})
        )
        .await?
        .0,
        StatusCode::CONFLICT
    );
    process::send_command(&app, command)
        .await
        .map_err(failure)?;
    let command_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        match fs::read_to_string(app.directory.join("commands.txt")) {
            Ok(contents) if contents == "/players\n" => break,
            Ok(_) => {}
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(error) => return Err(error.into()),
        }
        assert!(tokio::time::Instant::now() < command_deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(
        fs::read_to_string(app.directory.join("commands.txt"))?,
        "/players\n"
    );
    let args = fs::read_to_string(app.directory.join("args.txt"))?;
    assert!(args.contains("--bind\n127.0.0.1\n--port\n34199\n"));
    process::force_stop(&app, true).await.map_err(failure)?;
    let exit_deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    loop {
        process::poll(&app).await;
        if app.inner.lock().await.child.is_none() {
            break;
        }
        assert!(tokio::time::Instant::now() < exit_deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    assert_eq!(app.inner.lock().await.status.state, ServerState::Failed);
    assert!(
        app.inner
            .lock()
            .await
            .logs
            .iter()
            .all(|entry| !entry.message.contains("/players"))
    );
    Ok(())
}
#[tokio::test]
async fn create_operation_gate_blocks_launch_without_blocking_status() -> TestResult {
    let dir = tempfile::tempdir()?;
    let binary = dir.path().join("fake-factorio");
    script(&binary, "sleep 0.2\nexit 1")?;
    let app = App::new(dir.path().join("data"), Some(binary)).map_err(failure)?;
    fs::write(app.saves_path().join("world.zip"), b"synthetic")?;
    let task_app = app.clone();
    let task = tokio::spawn(async move { process::create_save(&task_app, "new.zip").await });
    tokio::time::sleep(Duration::from_millis(30)).await;
    assert!(app.save_operations.try_lock().is_err());
    assert!(process::start(&app, "world.zip").await.is_err());
    assert!(app.inner.try_lock().is_ok());
    assert!(task.await?.is_err());
    assert!(app.save_operations.try_lock().is_ok());
    Ok(())
}

#[tokio::test]
async fn malformed_mod_transaction_blocks_launch_even_when_version_is_unknown() -> TestResult {
    let dir = tempfile::tempdir()?;
    let binary = dir.path().join("fake-factorio");
    script(&binary, "touch launched; exit 0")?;
    let app = App::new(dir.path().join("data"), Some(binary)).map_err(failure)?;
    world(&app.saves_path().join("world.zip"))?;
    fs::write(app.directory.join("mod-transaction.json"), b"{}")?;
    let start = process::start(&app, "world.zip").await;
    assert!(matches!(start, Err(error) if error.status == StatusCode::BAD_REQUEST));
    let create = process::create_save(&app, "new.zip").await;
    assert!(matches!(create, Err(error) if error.status == StatusCode::BAD_REQUEST));
    assert!(!app.directory.join("launched").exists());
    assert!(!app.saves_path().join("new.zip").exists());
    Ok(())
}

#[test]
fn runtime_paths_reads_only_path_keys_and_rejects_oversized_or_linked_config() -> TestResult {
    let dir = tempfile::tempdir()?;
    let config = dir.path().join("config.ini");
    fs::write(&config, b"[path]\nread-data=/game/data\nwrite-data=/game/state\n[other]\nread-data=ignore\npassword=synthetic\n")?;
    let paths = storage::runtime_paths(dir.path(), None).map_err(failure)?;
    assert_eq!(paths.read_data, "/game/data");
    assert_eq!(paths.write_data, "/game/state");
    assert!(paths.game_binary.is_none());
    assert!(!serde_json::to_string(&paths)?.contains("synthetic"));
    fs::write(&config, vec![b'x'; 64 * 1024 + 1])?;
    assert!(storage::runtime_paths(dir.path(), None).is_err());
    fs::remove_file(&config)?;
    symlink("untrusted.ini", &config)?;
    assert!(storage::runtime_paths(dir.path(), None).is_err());
    assert!(!factorio_control::auth::permits(
        factorio_control::auth::Role::Viewer,
        &axum::http::Method::GET,
        "/api/runtime-paths"
    ));
    Ok(())
}

#[test]
fn temporary_saves_and_foreign_archive_directories_do_not_break_recovery() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(failure)?;
    world(&app.saves_path().join("world.zip"))?;
    fs::write(app.saves_path().join("_autosave1.tmp.zip"), b"incomplete")?;
    fs::write(app.saves_path().join("_autosave2.TMP.ZIP"), b"incomplete")?;
    let selected = storage::selected_save(
        &SaveSelectionDocument {
            revision: "missing".into(),
            save: None,
            load_latest: true,
        },
        &app.saves_path(),
    )
    .map_err(failure)?;
    assert_eq!(selected, "world.zip");
    assert_eq!(storage::saves(&app.saves_path()).map_err(failure)?.len(), 1);
    let archived = storage::archive_save(
        &app.directory,
        "world.zip",
        &storage::save_info(&app.saves_path(), "world.zip")
            .map_err(failure)?
            .revision,
    )
    .map_err(failure)?;
    fs::create_dir(app.directory.join("archived-saves/lost+found"))?;
    let listing = storage::archived_saves(&app.directory).map_err(failure)?;
    assert_eq!(listing.len(), 1);
    assert_eq!(listing[0].id, archived.id);
    Ok(())
}

#[test]
fn player_lists_validate_without_rewriting_migrated_content() -> TestResult {
    let dir = tempfile::tempdir()?;
    let whitelist = dir.path().join("server-whitelist.json");
    let banlist = dir.path().join("server-banlist.json");
    assert!(storage::prepare_player_lists(&whitelist, &banlist, true).is_err());
    let content = br#"["allowed-player"]"#;
    fs::write(&whitelist, content)?;
    assert!(
        storage::validate_whitelist_choice(&dir.path().join("manager-settings.json"), &whitelist)
            .is_err()
    );
    fs::write(
        dir.path().join("manager-settings.json"),
        br#"{"use_server_whitelist":false}"#,
    )?;
    storage::validate_whitelist_choice(&dir.path().join("manager-settings.json"), &whitelist)
        .map_err(failure)?;
    let banned = br#"[{"username":"banned-player","reason":"fixture"}]"#;
    fs::write(&banlist, banned)?;
    storage::prepare_player_lists(&whitelist, &banlist, true).map_err(failure)?;
    assert_eq!(fs::read(&whitelist)?, content);
    assert_eq!(fs::read(&banlist)?, banned);
    fs::write(&whitelist, b"{}")?;
    assert!(storage::prepare_player_lists(&whitelist, &banlist, true).is_err());
    fs::remove_file(&whitelist)?;
    symlink(&banlist, &whitelist)?;
    assert!(storage::prepare_player_lists(&whitelist, &banlist, false).is_err());
    Ok(())
}

#[tokio::test]
async fn migrated_player_lists_have_explicit_paths_and_whitelist_switch() -> TestResult {
    let dir = tempfile::tempdir()?;
    let binary = dir.path().join("fake-factorio");
    script(
        &binary,
        "printf '%s\\n' \"$@\" > args.txt\nwhile :; do sleep 0.1; done",
    )?;
    let app = App::new(dir.path().join("data"), Some(binary)).map_err(failure)?;
    world(&app.saves_path().join("world.zip"))?;
    fs::write(
        app.directory.join("server-whitelist.json"),
        br#"["allowed"]"#,
    )?;
    fs::write(
        app.directory.join("server-banlist.json"),
        br#"[{"username":"banned","reason":"fixture"}]"#,
    )?;
    for enabled in [true, false] {
        let mut settings =
            storage::manager_settings(&app.manager_settings_path()).map_err(failure)?;
        settings.settings.use_server_whitelist = enabled;
        storage::update_manager_settings(&app.manager_settings_path(), settings)
            .map_err(failure)?;
        process::start(&app, "world.zip").await.map_err(failure)?;
        let path = app.directory.join("args.txt");
        let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
        while !path.exists() {
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        let arguments = fs::read_to_string(&path)?;
        let args: Vec<_> = arguments.lines().collect();
        assert!(args.windows(2).any(|p| p
            == [
                "--use-server-whitelist",
                if enabled { "true" } else { "false" }
            ]));
        for (flag, name) in [
            ("--server-whitelist", "server-whitelist.json"),
            ("--server-banlist", "server-banlist.json"),
        ] {
            let expected = app.directory.join(name);
            assert!(
                args.windows(2)
                    .any(|p| p[0] == flag && Some(p[1]) == expected.to_str())
            );
        }
        process::force_stop(&app, true).await.map_err(failure)?;
        while app.inner.lock().await.child.is_some() {
            process::poll(&app).await;
            assert!(tokio::time::Instant::now() < deadline);
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
        fs::remove_file(path)?;
    }
    Ok(())
}

#[tokio::test]
async fn blocked_console_pipe_does_not_block_status_or_emergency_stop() -> TestResult {
    let dir = tempfile::tempdir()?;
    let binary = dir.path().join("fake-factorio");
    script(
        &binary,
        "echo 'ServerMultiplayerManager to(InGame)'\nexec sleep 10",
    )?;
    let app = App::new(dir.path().join("data"), Some(binary)).map_err(failure)?;
    world(&app.saves_path().join("world.zip"))?;
    fs::write(app.settings_path(), br#"{"auto_pause":false}"#)?;
    process::start(&app, "world.zip").await.map_err(failure)?;
    let deadline = tokio::time::Instant::now() + Duration::from_secs(5);
    while app.inner.lock().await.status.state != ServerState::Running {
        assert!(tokio::time::Instant::now() < deadline);
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    let writer_app = app.clone();
    let writer = tokio::spawn(async move {
        for _ in 0..128 {
            process::send_command(
                &writer_app,
                ConsoleCommandRequest {
                    command: "x".repeat(4096),
                    acknowledge_scripting: false,
                },
            )
            .await?;
        }
        Ok::<_, factorio_control::Error>(())
    });
    tokio::time::sleep(Duration::from_millis(50)).await;
    assert!(
        !writer.is_finished(),
        "fixture must fill the child's unread pipe"
    );
    let inner = tokio::time::timeout(Duration::from_millis(200), app.inner.lock()).await?;
    assert_eq!(inner.status.state, ServerState::Running);
    drop(inner);
    tokio::time::timeout(Duration::from_millis(200), process::force_stop(&app, true))
        .await?
        .map_err(failure)?;
    assert!(writer.await?.is_err());
    process::shutdown(&app).await;
    Ok(())
}
