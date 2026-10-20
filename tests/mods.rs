#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used)]
use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use factorio_control::{
    App,
    mod_settings::{self, ModSettingChange, ModSettingKind, ModSettingsPatch},
    models::ServerState,
    mods,
};
use std::{
    collections::BTreeMap,
    fs,
    io::{Cursor, Write},
    os::unix::fs::symlink,
};
use tower::ServiceExt;
type TestResult = Result<(), Box<dyn std::error::Error>>;
fn app_error(error: factorio_control::Error) -> std::io::Error {
    std::io::Error::other(error.message)
}
fn archive(
    version: &str,
    dependencies: &[&str],
    extra_path: Option<&str>,
) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    let options = zip::write::SimpleFileOptions::default();
    writer.start_file(format!("demo_{version}/info.json"), options)?;
    writer.write_all(&serde_json::to_vec(&serde_json::json!({"name":"demo","version":version,"title":"Demo","author":"Synthetic fixture", "factorio_version":"2.1", "dependencies":dependencies}))?)?;
    writer.start_file(
        extra_path.map_or_else(|| format!("demo_{version}/data.lua"), String::from),
        options,
    )?;
    writer.write_all(b"-- synthetic mod fixture")?;
    Ok(writer.finish()?.into_inner())
}
fn string(value: &str) -> Vec<u8> {
    let mut bytes = vec![0];
    if value.len() < 255 {
        bytes.push(value.len() as u8);
    } else {
        bytes.push(255);
        bytes.extend((value.len() as u32).to_le_bytes());
    }
    bytes.extend(value.as_bytes());
    bytes
}
fn scalar(kind: u8, bytes: &[u8]) -> Vec<u8> {
    let mut value = vec![kind, 1];
    value.extend(bytes);
    value
}
fn dict(entries: &[(&str, Vec<u8>)]) -> Vec<u8> {
    let mut bytes = vec![5, 0];
    bytes.extend((entries.len() as u32).to_le_bytes());
    for (key, value) in entries {
        bytes.extend(string(key));
        bytes.extend(value);
    }
    bytes
}
fn fixture() -> Vec<u8> {
    let startup = dict(&[
        ("toggle", dict(&[("value", scalar(1, &[1]))])),
        (
            "number",
            dict(&[("value", scalar(2, &2.5_f64.to_le_bytes()))]),
        ),
        ("text", dict(&[("value", scalar(3, &string("original")))])),
        (
            "signed",
            dict(&[("value", scalar(6, &i64::MIN.to_le_bytes()))]),
        ),
        (
            "unsigned",
            dict(&[("value", scalar(7, &u64::MAX.to_le_bytes()))]),
        ),
        (
            "color",
            dict(&[(
                "value",
                dict(&[
                    ("r", scalar(2, &0.5_f64.to_le_bytes())),
                    ("g", scalar(2, &0.5_f64.to_le_bytes())),
                    ("b", scalar(2, &0.5_f64.to_le_bytes())),
                    ("a", scalar(2, &1.0_f64.to_le_bytes())),
                ]),
            )]),
        ),
    ]);
    let mut bytes = vec![2, 0, 1, 0, 23, 0, 0, 0, 0];
    bytes.extend(dict(&[
        ("startup", startup),
        (
            "runtime-global",
            dict(&[(
                "sentinel",
                scalar(3, &string("runtime bytes are preserved")),
            )]),
        ),
        ("runtime-per-user", dict(&[])),
    ]));
    bytes
}
fn change(value: &str) -> ModSettingChange {
    ModSettingChange {
        value: Some(value.into()),
        components: BTreeMap::new(),
    }
}
#[test]
fn startup_roundtrip_preserves_runtime_bytes_and_integer_precision() -> TestResult {
    let bytes = fixture();
    let original = mod_settings::decode(&bytes).map_err(app_error)?;
    assert_eq!(
        mod_settings::apply(&bytes, &BTreeMap::new()).map_err(app_error)?,
        bytes
    );
    assert!(
        original
            .settings
            .iter()
            .any(|setting| setting.name == "unsigned" && setting.value == u64::MAX.to_string())
    );
    assert!(
        original
            .settings
            .iter()
            .any(|setting| setting.name == "signed" && setting.value == i64::MIN.to_string())
    );
    let changes = BTreeMap::from([
        ("text".into(), change(&"é".repeat(300))),
        ("toggle".into(), change("false")),
        ("number".into(), change("8.25")),
        (
            "color".into(),
            ModSettingChange {
                value: None,
                components: BTreeMap::from([("r".into(), "0.75".into())]),
            },
        ),
    ]);
    let updated = mod_settings::apply(&bytes, &changes).map_err(app_error)?;
    let document = mod_settings::decode(&updated).map_err(app_error)?;
    assert_eq!(updated.get(..9), bytes.get(..9));
    let tail = string("runtime-global");
    let at = bytes
        .windows(tail.len())
        .position(|window| window == tail)
        .ok_or("missing runtime sentinel")?;
    assert!(updated.ends_with(bytes.get(at..).ok_or("invalid sentinel offset")?));
    assert!(
        document
            .settings
            .iter()
            .any(|setting| setting.name == "text" && setting.value == "é".repeat(300))
    );
    assert!(
        document
            .settings
            .iter()
            .any(|setting| setting.name == "toggle" && setting.value == "false")
    );
    assert!(document.settings.iter().any(|setting| {
        setting.name == "color"
            && setting.kind == ModSettingKind::Color
            && setting
                .components
                .iter()
                .any(|value| value.name == "r" && value.value == "0.75")
    }));
    Ok(())
}
#[test]
fn startup_rejects_malformed_files_invalid_values_and_stale_writes() -> TestResult {
    let bytes = fixture();
    for end in 0..bytes.len() {
        assert!(
            mod_settings::decode(&bytes[..end]).is_err(),
            "accepted truncation {end}"
        );
    }
    let mut trailing = bytes.clone();
    trailing.push(0);
    assert!(mod_settings::decode(&trailing).is_err());
    for (name, value) in [
        ("toggle", "1"),
        ("number", "NaN"),
        ("number", "inf"),
        ("unsigned", "-1"),
        ("signed", "9223372036854775808"),
        ("unknown", "1"),
    ] {
        assert!(
            mod_settings::apply(&bytes, &BTreeMap::from([(name.into(), change(value))])).is_err()
        );
    }
    let dir = tempfile::tempdir()?;
    let path = dir.path().join("mod-settings.dat");
    assert!(!mod_settings::read(&path).map_err(app_error)?.available);
    let initial = mod_settings::replace(&path, "missing", &bytes).map_err(app_error)?;
    let after = mod_settings::update(
        &path,
        ModSettingsPatch {
            revision: initial.revision.clone(),
            changes: BTreeMap::from([("number".into(), change("3.25"))]),
        },
    )
    .map_err(app_error)?;
    assert_ne!(after.revision, initial.revision);
    let current = fs::read(&path)?;
    assert!(mod_settings::replace(&path, &initial.revision, &bytes).is_err());
    assert!(mod_settings::replace(&path, &after.revision, b"invalid").is_err());
    assert_eq!(fs::read(&path)?, current);
    Ok(())
}
#[test]
fn upload_update_pin_and_conflict_keep_working_archive() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let original = mods::list(&app).map_err(app_error)?;
    let first_bytes = archive("1.0.0", &["base >= 2.1.0"], None)?;
    let first =
        mods::install(&app, &original.revision, &first_bytes, Some("2.1.23")).map_err(app_error)?;
    assert_eq!(first.mods.len(), 1);
    assert_eq!(
        first.mods.first().map(|item| item.version.as_str()),
        Some("1.0.0")
    );
    assert!(
        mods::install(
            &app,
            &original.revision,
            &archive("1.1.0", &["base"], None)?,
            None
        )
        .is_err()
    );
    assert!(mods::install(&app, &first.revision, b"not a zip", None).is_err());
    assert!(
        mods::install(
            &app,
            &first.revision,
            &archive("1.1.0", &["base"], None)?,
            Some("2.0.76")
        )
        .is_err()
    );
    assert_eq!(
        fs::read(dir.path().join("mods/demo_1.0.0.zip"))?,
        first_bytes
    );
    let second = mods::install(
        &app,
        &first.revision,
        &archive("1.1.0", &["base"], None)?,
        None,
    )
    .map_err(app_error)?;
    assert_eq!(second.mods.len(), 1);
    assert_eq!(
        second.mods.first().map(|item| item.version.as_str()),
        Some("1.1.0")
    );
    assert!(dir.path().join("mods/demo_1.0.0.zip").exists());
    let list: serde_json::Value =
        serde_json::from_slice(&fs::read(dir.path().join("mods/mod-list.json"))?)?;
    assert_eq!(list["mods"][1]["version"], "1.1.0");
    assert!(mods::install(&app, &second.revision, &first_bytes, None).is_err());
    Ok(())
}
#[test]
fn unsafe_archives_symlinks_and_unsatisfied_dependencies_do_not_mutate_mods() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    for path in [
        "../outside.lua",
        "demo_1.0.0/../../outside.lua",
        "demo_1.0.0\\evil",
        "other_1.0.0/data.lua",
    ] {
        assert!(
            mods::install(
                &app,
                &initial.revision,
                &archive("1.0.0", &["base"], Some(path))?,
                None
            )
            .is_err()
        );
    }
    assert!(
        mods::install(
            &app,
            &initial.revision,
            &archive("1.0.0", &["missing >= 1.0.0"], None)?,
            None
        )
        .is_err()
    );
    assert!(
        mods::install(
            &app,
            &initial.revision,
            &archive("1.0.0", &["! base"], None)?,
            None
        )
        .is_err()
    );
    assert_eq!(
        mods::list(&app).map_err(app_error)?.revision,
        initial.revision
    );
    let outside = dir.path().join("outside");
    fs::write(&outside, "protected")?;
    symlink(&outside, dir.path().join("mods/demo_1.0.0.zip"))?;
    assert!(mods::list(&app).is_err());
    assert_eq!(fs::read(outside)?, b"protected");
    Ok(())
}
#[tokio::test]
async fn routes_require_revisions_and_stopped_server_and_roundtrip_downloads() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    app.inner.lock().await.status.version = Some("2.1.23".into());
    let router = mods::routes().with_state(app.clone());
    let bytes = archive("1.0.0", &["base"], None)?;
    let missing_revision = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mods/upload")
                .body(Body::from(bytes.clone()))?,
        )
        .await?;
    assert_eq!(missing_revision.status(), StatusCode::BAD_REQUEST);
    app.inner.lock().await.status.state = ServerState::Running;
    let running = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mods/upload")
                .header("if-match", &initial.revision)
                .body(Body::from(bytes.clone()))?,
        )
        .await?;
    assert_eq!(running.status(), StatusCode::CONFLICT);
    app.inner.lock().await.status.state = ServerState::Stopped;
    let uploaded = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mods/upload")
                .header("if-match", &initial.revision)
                .body(Body::from(bytes.clone()))?,
        )
        .await?;
    assert_eq!(uploaded.status(), StatusCode::CREATED);
    let document: mods::ModsDocument =
        serde_json::from_slice(&to_bytes(uploaded.into_body(), 1024 * 1024).await?)?;
    let disabled = router
        .clone()
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri("/mods/demo/enabled")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&mods::ModEnabledRequest {
                    revision: document.revision,
                    enabled: false,
                })?))?,
        )
        .await?;
    assert_eq!(disabled.status(), StatusCode::OK);
    let document: mods::ModsDocument =
        serde_json::from_slice(&to_bytes(disabled.into_body(), 1024 * 1024).await?)?;
    assert!(document.mods.first().is_some_and(|item| !item.enabled));
    let downloaded = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mods/demo/download")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(downloaded.status(), StatusCode::OK);
    assert_eq!(
        &to_bytes(downloaded.into_body(), 1024 * 1024).await?[..],
        &bytes
    );
    let settings = router
        .clone()
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mods/settings/file")
                .header("if-match", "missing")
                .body(Body::from(fixture()))?,
        )
        .await?;
    assert_eq!(settings.status(), StatusCode::OK);
    let downloaded = router
        .clone()
        .oneshot(
            Request::builder()
                .uri("/mods/settings/file")
                .body(Body::empty())?,
        )
        .await?;
    assert_eq!(
        &to_bytes(downloaded.into_body(), 1024 * 1024).await?[..],
        &fixture()
    );
    let refreshed = router
        .clone()
        .oneshot(Request::builder().uri("/mods").body(Body::empty())?)
        .await?;
    let document: mods::ModsDocument =
        serde_json::from_slice(&to_bytes(refreshed.into_body(), 1024 * 1024).await?)?;
    let unauthenticated = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/mods/demo/update")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(&mods::ModUpdateRequest {
                    revision: document.revision,
                    version: "2.0.0".into(),
                })?))?,
        )
        .await?;
    assert_eq!(unauthenticated.status(), StatusCode::BAD_REQUEST);
    Ok(())
}

#[test]
fn archive_rejects_zip_bombs_links_and_crc_damage_but_allows_arbitrary_root() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    for scenario in ["bomb", "symlink", "crc", "valid"] {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        let options = zip::write::SimpleFileOptions::default()
            .compression_method(zip::CompressionMethod::Stored);
        writer.start_file("unrelated-root/info.json", options)?;
        writer.write_all(
            br#"{"name":"demo","version":"1.0.0","title":"Demo","author":"Synthetic fixture","factorio_version":"2.1"}"#,
        )?;
        match scenario {
            "bomb" => {
                writer.start_file(
                    "unrelated-root/bomb",
                    options.compression_method(zip::CompressionMethod::Deflated),
                )?;
                writer.write_all(&vec![0; 4 * 1024 * 1024])?;
            }
            "symlink" => {
                writer.add_symlink("unrelated-root/link", "../../outside", options)?;
            }
            _ => {
                writer.start_file("unrelated-root/data.lua", options)?;
                writer.write_all(b"sentinel data content")?;
            }
        }
        let mut bytes = writer.finish()?.into_inner();
        if scenario == "crc" {
            let at = bytes
                .windows(8)
                .position(|window| window == b"sentinel")
                .ok_or("missing CRC sentinel")?;
            if let Some(byte) = bytes.get_mut(at) {
                *byte = b'X';
            }
        }
        let result = mods::install(&app, &initial.revision, &bytes, None);
        if scenario == "valid" {
            result.map_err(app_error)?;
        } else {
            assert!(result.is_err(), "accepted {scenario}");
            assert_eq!(
                mods::list(&app).map_err(app_error)?.revision,
                initial.revision
            );
        }
    }
    Ok(())
}

fn metadata_archive(info: &serde_json::Value) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file(
        "fixture/info.json",
        zip::write::SimpleFileOptions::default(),
    )?;
    writer.write_all(&serde_json::to_vec(info)?)?;
    Ok(writer.finish()?.into_inner())
}
fn mod_metadata(name: &str, dependencies: &[&str]) -> serde_json::Value {
    serde_json::json!({"name":name,"version":"1.0.0","title":"Fixture","author":"Fixture author","factorio_version":"2.1","dependencies":dependencies})
}
#[test]
fn required_metadata_and_version_bounds_match_factorio() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    let mut missing_author = mod_metadata("demo", &["base"]);
    if let Some(object) = missing_author.as_object_mut() {
        object.remove("author");
    }
    assert!(
        mods::install(
            &app,
            &initial.revision,
            &metadata_archive(&missing_author)?,
            Some("2.1.23")
        )
        .is_err()
    );
    for author in [
        serde_json::json!(["Author"]),
        serde_json::Value::Null,
        serde_json::json!(42),
    ] {
        let mut info = mod_metadata("demo", &["base"]);
        info["author"] = author;
        assert!(
            mods::install(
                &app,
                &initial.revision,
                &metadata_archive(&info)?,
                Some("2.1.23")
            )
            .is_err()
        );
    }
    for version in ["65536.0.0", "0.65536.0", "0.0.65536", "1.0", "1.0.0.0"] {
        let mut info = mod_metadata("demo", &["base"]);
        info["version"] = version.into();
        assert!(
            mods::install(
                &app,
                &initial.revision,
                &metadata_archive(&info)?,
                Some("2.1.23")
            )
            .is_err()
        );
    }
    assert!(
        mods::install(
            &app,
            &initial.revision,
            &metadata_archive(&mod_metadata("recycler", &["base"]))?,
            Some("2.1.23")
        )
        .is_err()
    );
    let mut info = mod_metadata("demo", &["base>=2.1.0"]);
    info["version"] = "65535.65535.65535".into();
    mods::install(
        &app,
        &initial.revision,
        &metadata_archive(&info)?,
        Some("2.1.23"),
    )
    .map_err(app_error)?;
    Ok(())
}
#[test]
fn rejects_present_optional_dependency_cycles_but_allows_unordered_edges() -> TestResult {
    for prefix in ["?", "(?)", "+"] {
        let dir = tempfile::tempdir()?;
        let app = App::new(dir.path().into(), None).map_err(app_error)?;
        let initial = mods::list(&app).map_err(app_error)?;
        let dependency = format!("{prefix} a-demo>=1.0.0");
        let b = metadata_archive(&mod_metadata("b-demo", &[&dependency]))?;
        let after_b =
            mods::install(&app, &initial.revision, &b, Some("2.1.23")).map_err(app_error)?;
        let a = metadata_archive(&mod_metadata("a-demo", &["b-demo"]))?;
        let error = mods::install(&app, &after_b.revision, &a, Some("2.1.23"))
            .err()
            .ok_or("accepted cyclic dependency")?;
        assert!(error.message.contains("cycle"));
        assert_eq!(
            mods::list(&app).map_err(app_error)?.revision,
            after_b.revision
        );
        let a_unordered = metadata_archive(&mod_metadata("a-demo", &["~b-demo>=1.0.0"]))?;
        mods::install(&app, &after_b.revision, &a_unordered, Some("2.1.23")).map_err(app_error)?;
        mods::validate_start(&app, "2.1.23").map_err(app_error)?;
        fs::write(dir.path().join("mods/a-demo_1.0.0.zip"), &a)?;
        assert!(mods::validate_start(&app, "2.1.23").is_err());
    }
    Ok(())
}
#[test]
fn detects_installed_bundled_dependencies_and_honors_explicit_disabling() -> TestResult {
    let dir = tempfile::tempdir()?;
    let game = dir.path().join("factorio");
    let binary = game.join("bin/x64/factorio");
    fs::create_dir_all(binary.parent().ok_or("no binary parent")?)?;
    fs::write(&binary, b"synthetic binary")?;
    fs::create_dir_all(game.join("data/core"))?;
    fs::write(game.join("data/core/info.json"), br#"{"name":"core"}"#)?;
    for name in ["base", "quality", "space-age", "elevated-rails", "recycler"] {
        let path = game.join("data").join(name);
        fs::create_dir_all(&path)?;
        fs::write(
            path.join("info.json"),
            serde_json::to_vec(&serde_json::json!({"name":name,"version":"2.1.23"}))?,
        )?;
    }
    let app = App::new(dir.path().join("manager"), Some(binary)).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    let bytes = metadata_archive(&mod_metadata(
        "demo",
        &["quality>=2.1.0", "space-age", "recycler"],
    ))?;
    mods::install(&app, &initial.revision, &bytes, Some("2.1.23")).map_err(app_error)?;
    mods::validate_start(&app, "2.1.23").map_err(app_error)?;
    assert!(mods::validate_start(&app, "2.0.76").is_err());
    let path = app.directory.join("mods/mod-list.json");
    let mut document: serde_json::Value = serde_json::from_slice(&fs::read(&path)?)?;
    if let Some(entries) = document["mods"].as_array_mut() {
        entries.push(serde_json::json!({"name":"quality","enabled":false}));
    }
    fs::write(&path, serde_json::to_vec(&document)?)?;
    assert!(mods::validate_start(&app, "2.1.23").is_err());
    Ok(())
}
#[tokio::test]
async fn unknown_binary_version_cannot_bypass_http_mod_compatibility_checks() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    let bytes = archive("1.0.0", &["base"], None)?;
    let installed =
        mods::install(&app, &initial.revision, &bytes, Some("2.1.23")).map_err(app_error)?;
    let router = mods::routes().with_state(app.clone());
    let requests = [
        Request::builder()
            .method("POST")
            .uri("/mods/upload")
            .header("if-match", &installed.revision)
            .body(Body::from(archive("1.1.0", &["base"], None)?))?,
        Request::builder()
            .uri("/mods/demo/updates")
            .body(Body::empty())?,
        Request::builder()
            .method("POST")
            .uri("/mods/demo/update")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&mods::ModUpdateRequest {
                revision: installed.revision.clone(),
                version: "1.1.0".into(),
            })?))?,
        Request::builder()
            .method("PUT")
            .uri("/mods/demo/enabled")
            .header("content-type", "application/json")
            .body(Body::from(serde_json::to_vec(&mods::ModEnabledRequest {
                revision: installed.revision.clone(),
                enabled: false,
            })?))?,
    ];
    for request in requests {
        let response = router.clone().oneshot(request).await?;
        assert_eq!(response.status(), StatusCode::CONFLICT);
        let body = String::from_utf8(to_bytes(response.into_body(), 4096).await?.to_vec())?;
        assert!(body.contains("binary version is unavailable"));
    }
    assert_eq!(
        mods::list(&app).map_err(app_error)?.revision,
        installed.revision
    );
    Ok(())
}

#[test]
fn older_upload_is_stored_for_sync_without_changing_active_version() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let empty = mods::list(&app).map_err(app_error)?;
    let current = mods::install(
        &app,
        &empty.revision,
        &archive("2.0.0", &["base"], None)?,
        Some("2.1.23"),
    )
    .map_err(app_error)?;
    let stored = mods::install(
        &app,
        &current.revision,
        &archive("1.0.0", &["base"], None)?,
        Some("2.1.23"),
    )
    .map_err(app_error)?;
    assert!(dir.path().join("mods/demo_1.0.0.zip").exists());
    assert_eq!(
        stored.mods.first().map(|item| item.version.as_str()),
        Some("2.0.0")
    );
    assert_ne!(stored.revision, current.revision);
    Ok(())
}

#[test]
fn dependency_references_accept_legacy_names_without_weakening_requirements() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    let valid = metadata_archive(&mod_metadata(
        "demo",
        &["base", "! Squeak Through", "(?) Legacy Mod", "? 古いmod"],
    ))?;
    let installed =
        mods::install(&app, &initial.revision, &valid, Some("2.1.23")).map_err(app_error)?;
    mods::validate_start(&app, "2.1.23").map_err(app_error)?;
    let required = metadata_archive(&mod_metadata("needs-legacy", &["Legacy Mod >= 1.0.0"]))?;
    let error = mods::install(&app, &installed.revision, &required, Some("2.1.23"))
        .err()
        .ok_or("Expected missing required dependency")?;
    assert!(error.message.contains("requires enabled mod Legacy Mod"));
    let malformed = metadata_archive(&mod_metadata("invalid-ref", &["? bad\nname"]))?;
    let error = mods::install(&app, &installed.revision, &malformed, Some("2.1.23"))
        .err()
        .ok_or("Expected invalid dependency")?;
    assert!(
        error
            .message
            .contains("invalid-ref declares unsupported dependency")
    );
    Ok(())
}

#[test]
fn shortened_dependency_versions_follow_factorio_comparisons() -> TestResult {
    let dir = tempfile::tempdir()?;
    let app = App::new(dir.path().into(), None).map_err(app_error)?;
    let initial = mods::list(&app).map_err(app_error)?;
    let valid = metadata_archive(&mod_metadata(
        "short-version",
        &["base >= 2.1", "base > 2.1", "base < 2.2"],
    ))?;
    let installed =
        mods::install(&app, &initial.revision, &valid, Some("2.1.23")).map_err(app_error)?;
    for dependency in ["base = 2.1", "base >= 2", "base >= 2.1.0.0"] {
        let bytes = metadata_archive(&mod_metadata("invalid-short", &[dependency]))?;
        assert!(
            mods::install(&app, &installed.revision, &bytes, Some("2.1.23")).is_err(),
            "{dependency}"
        );
        assert!(!dir.path().join("mods/invalid-short_1.0.0.zip").exists());
    }
    Ok(())
}
