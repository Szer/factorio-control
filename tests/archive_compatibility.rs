#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]

use axum::{
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use factorio_control::{App, api, mods};
use std::{
    fs,
    io::{Cursor, Write},
};
use tower::ServiceExt;

mod support;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn invalid_utf8_archive() -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
    writer.start_file("world/level.dat0", zip::write::SimpleFileOptions::default())?;
    writer.write_all(b"synthetic world")?;
    let mut bytes = writer.finish()?.into_inner();
    let mut changed = 0;
    for index in 0..bytes.len().saturating_sub(46) {
        let (flag, name) = match bytes.get(index..index + 4) {
            Some(b"PK\x03\x04") => (7, 30),
            Some(b"PK\x01\x02") => (9, 46),
            _ => continue,
        };
        *bytes.get_mut(index + flag).ok_or("Missing ZIP flag")? |= 0x08;
        *bytes.get_mut(index + name).ok_or("Missing ZIP name")? = 0xff;
        changed += 1;
    }
    assert_eq!(changed, 2);
    Ok(bytes)
}

#[tokio::test]
async fn invalid_utf8_zip_names_fail_without_changing_saves_or_mods() -> TestResult {
    let temp = tempfile::tempdir()?;
    let app = App::new(temp.path().join("data"), None).map_err(|e| e.message)?;
    app.inner.lock().await.status.version = Some("2.1.23".into());
    let working = app.saves_path().join("working.zip");
    fs::write(&working, b"preserved fixture")?;
    let router =
        support::authenticated(api::router(app.clone(), temp.path()), &app.directory).await?;
    let revision = mods::list(&app).map_err(|e| e.message)?.revision;
    let malformed = invalid_utf8_archive()?;
    let invalid_path = temp.path().join("invalid-name.zip");
    fs::write(&invalid_path, &malformed)?;
    let error = factorio_control::save_metadata::read(&invalid_path)
        .err()
        .ok_or("Invalid metadata archive was accepted")?;
    assert_eq!(
        error.message,
        "Archive contains an invalid UTF-8 entry name"
    );
    let mut legacy_encoded = malformed.clone();
    for index in 0..legacy_encoded.len().saturating_sub(46) {
        let flag = match legacy_encoded.get(index..index + 4) {
            Some(b"PK\x03\x04") => 7,
            Some(b"PK\x01\x02") => 9,
            _ => continue,
        };
        *legacy_encoded
            .get_mut(index + flag)
            .ok_or("Missing ZIP flag")? &= !0x08;
    }
    factorio_control::storage::validate_save(&legacy_encoded).map_err(|e| e.message)?;
    for route in ["/api/saves/broken.zip", "/api/mods/upload"] {
        let response = router
            .clone()
            .oneshot(
                Request::builder()
                    .method("POST")
                    .uri(route)
                    .header("Host", "localhost:9400")
                    .header("Origin", "http://localhost:9400")
                    .header("Content-Type", "application/zip")
                    .header("if-match", &revision)
                    .body(Body::from(malformed.clone()))?,
            )
            .await?;
        assert_eq!(response.status(), StatusCode::BAD_REQUEST, "{route}");
        let error: serde_json::Value =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
        assert_eq!(
            error["message"],
            "Archive contains an invalid UTF-8 entry name"
        );
        assert_eq!(fs::read(&working)?, b"preserved fixture");
        assert!(!app.saves_path().join("broken.zip").exists());
        assert_eq!(mods::list(&app).map_err(|e| e.message)?.revision, revision);
    }
    Ok(())
}
