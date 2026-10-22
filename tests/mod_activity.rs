#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]

use axum::{
    Router,
    body::{Body, to_bytes},
    http::{Request, StatusCode},
};
use factorio_control::{App, api, mods};
use serde_json::{Value, json};
use std::io::{Cursor, Write};
use tower::ServiceExt;
mod support;
type TestResult = Result<(), Box<dyn std::error::Error>>;

fn archive(version: &str) -> Result<Vec<u8>, Box<dyn std::error::Error>> {
    let mut zip = zip::ZipWriter::new(Cursor::new(Vec::new()));
    zip.start_file(
        format!("demo_{version}/info.json"),
        zip::write::SimpleFileOptions::default(),
    )?;
    zip.write_all(&serde_json::to_vec(&json!({"name":"demo","version":version,"title":"Demo","author":"Synthetic fixture","factorio_version":"2.1","dependencies":["base"]}))?)?;
    Ok(zip.finish()?.into_inner())
}
async fn request(
    router: &Router,
    method: &str,
    path: &str,
    revision: &str,
    body: Vec<u8>,
) -> Result<(StatusCode, Value), Box<dyn std::error::Error>> {
    let response = router
        .clone()
        .oneshot(
            Request::builder()
                .method(method)
                .uri(path)
                .header("host", "localhost:9400")
                .header("origin", "http://localhost:9400")
                .header(
                    "content-type",
                    if path == "/api/mods/upload" {
                        "application/zip"
                    } else {
                        "application/json"
                    },
                )
                .header("if-match", revision)
                .body(Body::from(body))?,
        )
        .await?;
    let status = response.status();
    let value = serde_json::from_slice(&to_bytes(response.into_body(), 1024 * 1024).await?)?;
    Ok((status, value))
}
async fn activity(router: &Router) -> Result<Vec<Value>, Box<dyn std::error::Error>> {
    let (status, value) = request(router, "GET", "/api/telemetry", "", Vec::new()).await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(value["enabled"], false);
    Ok(value["activity"]
        .as_array()
        .ok_or("Missing activity")?
        .clone())
}

#[tokio::test]
async fn committed_mod_changes_appear_once_and_failed_or_inactive_changes_do_not() -> TestResult {
    let temp = tempfile::tempdir()?;
    let app = App::new(temp.path().join("data"), None).map_err(|e| e.message)?;
    app.inner.lock().await.status.version = Some("2.1.23".into());
    let router =
        support::authenticated(api::router(app.clone(), temp.path()), &app.directory).await?;
    let revision = mods::list(&app).map_err(|e| e.message)?.revision;
    let (status, added) = request(
        &router,
        "POST",
        "/api/mods/upload",
        &revision,
        archive("1.0.0")?,
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let events = activity(&router).await?;
    assert_eq!(events.len(), 1);
    assert_eq!(events[0]["kind"], "mod_added");
    assert_eq!(events[0]["message"], "demo 1.0.0");
    assert!(
        events[0]["timestamp"]
            .as_u64()
            .is_some_and(|time| time <= factorio_control::now())
    );
    let added_revision = added["revision"].as_str().ok_or("Missing revision")?;
    let (status, _) = request(
        &router,
        "POST",
        "/api/mods/upload",
        &revision,
        archive("2.0.0")?,
    )
    .await?;
    assert_eq!(status, StatusCode::CONFLICT);
    let (status, _) = request(
        &router,
        "POST",
        "/api/mods/upload",
        added_revision,
        b"not a zip".to_vec(),
    )
    .await?;
    assert_eq!(status, StatusCode::BAD_REQUEST);
    assert_eq!(activity(&router).await?, events);
    let (status, updated) = request(
        &router,
        "POST",
        "/api/mods/upload",
        added_revision,
        archive("2.0.0")?,
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    let events = activity(&router).await?;
    assert_eq!(events.len(), 2);
    assert_eq!(events[0]["kind"], "mod_updated");
    assert_eq!(events[0]["message"], "demo: 1.0.0 → 2.0.0");
    let updated_revision = updated["revision"].as_str().ok_or("Missing revision")?;
    let (status, packs) = request(
        &router,
        "POST",
        "/api/mods/packs",
        "",
        serde_json::to_vec(&json!({"revision":updated_revision,"name":"snapshot"}))?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let pack_revision = packs[0]["revision"]
        .as_str()
        .ok_or("Missing pack revision")?
        .to_owned();
    assert_eq!(activity(&router).await?, events);
    let (status, stored) = request(
        &router,
        "POST",
        "/api/mods/upload",
        updated_revision,
        archive("0.5.0")?,
    )
    .await?;
    assert_eq!(status, StatusCode::CREATED);
    assert_eq!(activity(&router).await?, events);
    let revision = stored["revision"].as_str().ok_or("Missing revision")?;
    let (status, _) = request(
        &router,
        "DELETE",
        &format!("/api/mods/demo?revision={revision}"),
        "",
        Vec::new(),
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let events = activity(&router).await?;
    assert_eq!(events.len(), 3);
    assert_eq!(events[0]["kind"], "mod_removed");
    assert_eq!(events[0]["message"], "demo 2.0.0");
    assert!(
        events
            .windows(2)
            .all(|pair| pair[0]["id"].as_u64() > pair[1]["id"].as_u64())
    );
    assert_eq!(activity(&router).await?, events);
    let revision = mods::list(&app).map_err(|e| e.message)?.revision;
    let (status, restored) = request(
        &router,
        "POST",
        "/api/mods/packs/snapshot/restore",
        "",
        serde_json::to_vec(&json!({"revision":revision,"pack_revision":pack_revision}))?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    let events = activity(&router).await?;
    assert_eq!(events.len(), 4);
    assert_eq!(events[0]["kind"], "mod_added");
    assert_eq!(events[0]["message"], "demo 2.0.0");
    let (status, _) = request(
        &router,
        "POST",
        "/api/mods/packs/snapshot/restore",
        "",
        serde_json::to_vec(
            &json!({"revision":restored["revision"],"pack_revision":pack_revision}),
        )?,
    )
    .await?;
    assert_eq!(status, StatusCode::OK);
    assert_eq!(activity(&router).await?, events);
    Ok(())
}
