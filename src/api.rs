use crate::{App, Error, Result, auth, models::*, process, storage};
use axum::{
    Json, Router,
    body::Body,
    extract::{Path, State, rejection::JsonRejection},
    http::{Request, StatusCode, header},
    middleware,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use http_body_util::BodyExt;
use tokio::io::AsyncWriteExt;
use tower_http::services::{ServeDir, ServeFile};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        status,
        runtime_paths,
        telemetry,
        saves,
        get_settings,
        put_settings,
        get_raw_settings,
        put_raw_settings,
        get_manager_settings,
        put_manager_settings,
        get_save_selection,
        put_save_selection,
        get_admins,
        put_admins,
        logs,
        start,
        stop,
        upload,
        download,
        create_save,
        archive_save,
        archived_saves,
        restore_save,
        command,
        force_stop,
        game_log,
        crate::mods::save_sync::plan,
        crate::mods::save_sync::apply,
        crate::mods::save_sync::progress,
        crate::mods::workflows::search,
        crate::mods::workflows::portal_versions,
        crate::mods::workflows::install_portal,
        crate::mods::workflows::bulk_preview,
        crate::mods::workflows::bulk_apply,
        crate::mods::workflows::export_list,
        crate::mods::workflows::import_list,
        crate::mods::workflows::packs,
        crate::mods::workflows::create_pack,
        crate::mods::workflows::delete_pack,
        crate::mods::workflows::restore_pack,
        crate::mods::workflows::remove_mod,
        crate::mods::workflows::export_all,
        crate::mods::workflows::operation_progress,
        crate::mods::get_mods,
        crate::mods::upload_mod,
        crate::mods::download_mod,
        crate::mods::set_enabled,
        crate::mods::get_updates,
        crate::mods::update_mod,
        crate::mods::get_mod_settings,
        crate::mods::put_mod_settings,
        crate::mods::download_settings,
        crate::mods::upload_settings
    ),
    components(schemas(
        Status,
        ServerState,
        Save,
        SettingsDocument,
        ServerSettings,
        AllowCommands,
        CommandPermission,
        RawSettingsDocument,
        ManagerSettings,
        ManagerSettingsDocument,
        SaveSelectionDocument,
        Visibility,
        AdminsDocument,
        LogEntry,
        LogLevel,
        StartRequest,
        CreateSaveRequest,
        SaveRevisionRequest,
        ArchivedSave,
        ConsoleCommandRequest,
        CommandResponse,
        ForceStopRequest,
        GameLogDocument,
        RuntimePaths,
        crate::mods::workflows::RestorePackRequest,
        ApiError
    ))
)]
pub struct Api;

pub fn router(app: App, static_path: &std::path::Path) -> Router {
    match auth::Auth::load(&app.directory, None) {
        Ok(authentication) => router_with_auth(app, static_path, authentication),
        Err(_) => Router::new().fallback(|| async {
            (
                StatusCode::INTERNAL_SERVER_ERROR,
                Json(ApiError {
                    message: "Panel authentication configuration could not be loaded".into(),
                }),
            )
        }),
    }
}
pub fn router_with_auth(
    app: App,
    static_path: &std::path::Path,
    authentication: auth::Auth,
) -> Router {
    let routes = Router::new()
        .merge(crate::mods::routes())
        .route("/status", get(status))
        .route("/runtime-paths", get(runtime_paths))
        .route("/telemetry", get(telemetry))
        .route("/saves", get(saves))
        .route("/saves/create", post(create_save))
        .route("/saves/archived", get(archived_saves))
        .route("/saves/{name}/archive", post(archive_save))
        .route("/saves/archived/{id}/restore", post(restore_save))
        .route("/saves/{name}", get(download).post(upload))
        .route(
            "/saves/selection",
            get(get_save_selection).put(put_save_selection),
        )
        .route("/settings", get(get_settings).put(put_settings))
        .route("/settings/raw", get(get_raw_settings).put(put_raw_settings))
        .route(
            "/manager-settings",
            get(get_manager_settings).put(put_manager_settings),
        )
        .route("/admins", get(get_admins).put(put_admins))
        .route("/logs", get(logs))
        .route("/logs/file", get(game_log))
        .route("/server/command", post(command))
        .route("/server/force-stop", post(force_stop))
        .route("/server/start", post(start))
        .route("/server/stop", post(stop))
        .fallback(|| async {
            (
                StatusCode::NOT_FOUND,
                Json(ApiError {
                    message: "API endpoint not found".into(),
                }),
            )
        });
    Router::new()
        .merge(auth::routes(authentication.clone()))
        .nest("/api", routes)
        .fallback_service(
            ServeDir::new(static_path)
                .not_found_service(ServeFile::new(static_path.join("index.html"))),
        )
        .layer(middleware::from_fn_with_state(authentication, auth::guard))
        .with_state(app)
}
fn json<T>(value: std::result::Result<Json<T>, JsonRejection>) -> Result<T> {
    value.map(|Json(v)| v).map_err(|e| Error {
        status: e.status(),
        message: e.body_text(),
    })
}
#[utoipa::path(get, path="/api/status", responses((status=200, body=Status)))]
async fn status(State(app): State<App>) -> Json<Status> {
    Json(app.inner.lock().await.status.clone())
}
#[utoipa::path(get, path="/api/saves", responses((status=200, body=Vec<Save>), (status=400, body=ApiError)))]
async fn saves(State(app): State<App>) -> Result<Json<Vec<Save>>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::saves(&app.saves_path())?))
}
#[utoipa::path(get, path="/api/settings", responses((status=200, body=SettingsDocument), (status=400, body=ApiError)))]
async fn get_settings(State(app): State<App>) -> Result<Json<SettingsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::settings(&app.settings_path())?))
}
#[utoipa::path(put, path="/api/settings", request_body=SettingsDocument, responses((status=200, body=SettingsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn put_settings(
    State(app): State<App>,
    input: std::result::Result<Json<SettingsDocument>, JsonRejection>,
) -> Result<Json<SettingsDocument>> {
    let document = json(input)?;
    let _guard = app.inner.lock().await;
    Ok(Json(storage::update_settings(
        &app.settings_path(),
        document,
    )?))
}
#[utoipa::path(get, path="/api/settings/raw", responses((status=200, body=RawSettingsDocument), (status=400, body=ApiError)))]
async fn get_raw_settings(State(app): State<App>) -> Result<Json<RawSettingsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::raw_settings(&app.settings_path())?))
}
#[utoipa::path(put, path="/api/settings/raw", request_body=RawSettingsDocument, responses((status=200, body=RawSettingsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn put_raw_settings(
    State(app): State<App>,
    input: std::result::Result<Json<RawSettingsDocument>, JsonRejection>,
) -> Result<Json<RawSettingsDocument>> {
    let document = json(input)?;
    let _guard = app.inner.lock().await;
    Ok(Json(storage::update_raw_settings(
        &app.settings_path(),
        document,
    )?))
}
#[utoipa::path(get, path="/api/manager-settings", responses((status=200, body=ManagerSettingsDocument), (status=400, body=ApiError)))]
async fn get_manager_settings(State(app): State<App>) -> Result<Json<ManagerSettingsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::manager_settings(
        &app.manager_settings_path(),
    )?))
}
#[utoipa::path(put, path="/api/manager-settings", request_body=ManagerSettingsDocument, responses((status=200, body=ManagerSettingsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn put_manager_settings(
    State(app): State<App>,
    input: std::result::Result<Json<ManagerSettingsDocument>, JsonRejection>,
) -> Result<Json<ManagerSettingsDocument>> {
    let document = json(input)?;
    let _guard = app.inner.lock().await;
    Ok(Json(storage::update_manager_settings(
        &app.manager_settings_path(),
        document,
    )?))
}
#[utoipa::path(get, path="/api/saves/selection", responses((status=200, body=SaveSelectionDocument), (status=400, body=ApiError)))]
async fn get_save_selection(State(app): State<App>) -> Result<Json<SaveSelectionDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::save_selection(&app.save_selection_path())?))
}
#[utoipa::path(put, path="/api/saves/selection", request_body=SaveSelectionDocument, responses((status=200, body=SaveSelectionDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn put_save_selection(
    State(app): State<App>,
    input: std::result::Result<Json<SaveSelectionDocument>, JsonRejection>,
) -> Result<Json<SaveSelectionDocument>> {
    let document = json(input)?;
    let _guard = app.inner.lock().await;
    Ok(Json(storage::update_save_selection(
        &app.save_selection_path(),
        &app.saves_path(),
        document,
    )?))
}
#[utoipa::path(get, path="/api/admins", responses((status=200, body=AdminsDocument), (status=400, body=ApiError)))]
async fn get_admins(State(app): State<App>) -> Result<Json<AdminsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(storage::admins(&app.admins_path())?))
}
#[utoipa::path(put, path="/api/admins", request_body=AdminsDocument, responses((status=200, body=AdminsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn put_admins(
    State(app): State<App>,
    input: std::result::Result<Json<AdminsDocument>, JsonRejection>,
) -> Result<Json<AdminsDocument>> {
    let document = json(input)?;
    let _guard = app.inner.lock().await;
    Ok(Json(storage::update_admins(&app.admins_path(), document)?))
}
#[utoipa::path(get, path="/api/logs", responses((status=200, body=Vec<LogEntry>)))]
async fn logs(State(app): State<App>) -> Json<Vec<LogEntry>> {
    Json(app.inner.lock().await.logs.iter().cloned().collect())
}
#[utoipa::path(post, path="/api/server/start", request_body=StartRequest, responses((status=200, body=Status), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn start(
    State(app): State<App>,
    input: std::result::Result<Json<StartRequest>, JsonRejection>,
) -> Result<Json<Status>> {
    Ok(Json(
        process::start_selected(&app, json(input)?.save.as_deref()).await?,
    ))
}
#[utoipa::path(post, path="/api/server/stop", responses((status=200, body=Status), (status=409, body=ApiError)))]
async fn stop(
    State(app): State<App>,
    input: std::result::Result<Json<serde_json::Value>, JsonRejection>,
) -> Result<Json<Status>> {
    let value = json(input)?;
    if !value.as_object().is_some_and(|o| o.is_empty()) {
        return Err(Error::bad("Stop expects an empty JSON object"));
    }
    Ok(Json(process::stop(&app).await?))
}
#[utoipa::path(post, path="/api/saves/{name}", params(("name"=String, Path)), request_body(content=String, content_type="application/zip"), responses((status=201, body=Save), (status=400, body=ApiError), (status=409, body=ApiError), (status=413, body=ApiError)))]
async fn upload(
    State(app): State<App>,
    Path(name): Path<String>,
    request: Request<Body>,
) -> Result<(StatusCode, Json<Save>)> {
    if request
        .headers()
        .get(header::CONTENT_LENGTH)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.parse::<u64>().ok())
        .is_some_and(|length| length > storage::MAX_SAVE_BYTES)
    {
        return Err(save_upload_too_large());
    }
    {
        let _guard = app.inner.lock().await;
        let path = storage::save_path(&app.saves_path(), &name)?;
        if path.exists() {
            return Err(Error::conflict("A save with that name already exists"));
        }
    }
    let temporary = receive_save(
        &app.saves_path(),
        request.into_body(),
        storage::MAX_SAVE_BYTES,
    )
    .await?;
    let _guard = app.inner.lock().await;
    let path = storage::save_path(&app.saves_path(), &name)?;
    storage::persist_save(temporary, &path)?;
    Ok((
        StatusCode::CREATED,
        Json(storage::save_info(&app.saves_path(), &name)?),
    ))
}
async fn receive_save(
    directory: &std::path::Path,
    mut body: Body,
    limit: u64,
) -> Result<tempfile::NamedTempFile> {
    let temporary = tempfile::Builder::new()
        .prefix(".upload-")
        .tempfile_in(directory)?;
    let (file, temporary_path) = temporary.into_parts();
    let mut file = tokio::fs::File::from_std(file);
    let mut received = 0_u64;
    while let Some(frame) = body.frame().await {
        let frame = frame.map_err(|_| Error::bad("Save upload was interrupted"))?;
        if let Ok(bytes) = frame.into_data() {
            if bytes.len() as u64 > limit - received {
                return Err(save_upload_too_large());
            }
            received += bytes.len() as u64;
            file.write_all(&bytes).await?;
        }
    }
    file.sync_all().await?;
    let file = file.into_std().await;
    tokio::task::spawn_blocking(move || {
        let mut temporary = tempfile::NamedTempFile::from_parts(file, temporary_path);
        storage::validate_save_reader(temporary.as_file_mut())?;
        Ok::<_, Error>(temporary)
    })
    .await
    .map_err(|_| Error {
        status: StatusCode::INTERNAL_SERVER_ERROR,
        message: "Save validation failed".into(),
    })?
}
fn save_upload_too_large() -> Error {
    Error {
        status: StatusCode::PAYLOAD_TOO_LARGE,
        message: "Save upload exceeds 1 GiB".into(),
    }
}
#[utoipa::path(get, path="/api/saves/{name}", params(("name"=String, Path)), responses((status=200, content_type="application/zip"), (status=400, body=ApiError), (status=404, body=ApiError)))]
async fn download(State(app): State<App>, Path(name): Path<String>) -> Result<Response> {
    let _guard = app.inner.lock().await;
    let path = storage::save_path(&app.saves_path(), &name)?;
    let file = tokio::fs::File::open(path).await?;
    let stream = tokio_util::io::ReaderStream::new(file);
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        Body::from_stream(stream),
    )
        .into_response())
}

#[utoipa::path(get, path="/api/telemetry", responses((status=200, body=crate::telemetry::Snapshot), (status=400, body=ApiError)))]
async fn telemetry(State(app): State<App>) -> Result<Json<crate::telemetry::Snapshot>> {
    let inner = app.inner.lock().await;
    let manager = storage::manager_settings(&app.manager_settings_path())?.settings;
    Ok(Json(inner.telemetry.snapshot(
        manager.enable_script_telemetry,
        manager.poll_interval_seconds,
    )))
}

#[cfg(test)]
mod upload_tests {
    use super::*;
    use std::io;

    #[tokio::test]
    async fn chunked_upload_limit_removes_temporary_file()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let (mut writer, reader) = tokio::io::duplex(128);
        let producer = tokio::spawn(async move {
            writer.write_all(&[0; 128]).await?;
            writer.write_all(&[0; 128]).await
        });
        let body = Body::from_stream(tokio_util::io::ReaderStream::with_capacity(reader, 128));
        let result = receive_save(directory.path(), body, 128).await;
        assert!(matches!(
            result,
            Err(Error {
                status: StatusCode::PAYLOAD_TOO_LARGE,
                ..
            })
        ));
        producer.await??;
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
        Ok(())
    }

    #[tokio::test]
    async fn failed_body_removes_temporary_file()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        struct FailedReader;
        impl tokio::io::AsyncRead for FailedReader {
            fn poll_read(
                self: std::pin::Pin<&mut Self>,
                _: &mut std::task::Context<'_>,
                _: &mut tokio::io::ReadBuf<'_>,
            ) -> std::task::Poll<io::Result<()>> {
                std::task::Poll::Ready(Err(io::Error::other("Disconnected")))
            }
        }
        let directory = tempfile::tempdir()?;
        let body = Body::from_stream(tokio_util::io::ReaderStream::new(FailedReader));
        let result = receive_save(directory.path(), body, storage::MAX_SAVE_BYTES).await;
        assert!(matches!(
            result,
            Err(Error {
                status: StatusCode::BAD_REQUEST,
                ..
            })
        ));
        assert_eq!(std::fs::read_dir(directory.path())?.count(), 0);
        Ok(())
    }
}

#[utoipa::path(post, path="/api/saves/create", request_body=CreateSaveRequest, responses((status=201, body=Save), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn create_save(
    State(app): State<App>,
    input: std::result::Result<Json<CreateSaveRequest>, JsonRejection>,
) -> Result<(StatusCode, Json<Save>)> {
    Ok((
        StatusCode::CREATED,
        Json(process::create_save(&app, &json(input)?.name).await?),
    ))
}
#[utoipa::path(get, path="/api/saves/archived", responses((status=200, body=Vec<ArchivedSave>), (status=400, body=ApiError)))]
async fn archived_saves(State(app): State<App>) -> Result<Json<Vec<ArchivedSave>>> {
    let _inner = app.inner.lock().await;
    Ok(Json(storage::archived_saves(&app.directory)?))
}
#[utoipa::path(post, path="/api/saves/{name}/archive", params(("name"=String, Path)), request_body=SaveRevisionRequest, responses((status=200, body=ArchivedSave), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn archive_save(
    State(app): State<App>,
    Path(name): Path<String>,
    input: std::result::Result<Json<SaveRevisionRequest>, JsonRejection>,
) -> Result<Json<ArchivedSave>> {
    let request = json(input)?;
    let _operation = app
        .save_operations
        .try_lock()
        .map_err(|_| Error::conflict("A save operation is in progress"))?;
    let inner = app.inner.lock().await;
    if inner.child.is_some() {
        return Err(Error::conflict("Stop the server before archiving saves"));
    }
    let selection = storage::save_selection(&app.save_selection_path())?;
    if storage::selected_save(&selection, &app.saves_path())
        .ok()
        .as_deref()
        == Some(&name)
    {
        return Err(Error::conflict(
            "Select another save before archiving the active or selected save",
        ));
    }
    Ok(Json(storage::archive_save(
        &app.directory,
        &name,
        &request.revision,
    )?))
}
#[utoipa::path(post, path="/api/saves/archived/{id}/restore", params(("id"=String, Path)), request_body=SaveRevisionRequest, responses((status=200, body=Save), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn restore_save(
    State(app): State<App>,
    Path(id): Path<String>,
    input: std::result::Result<Json<SaveRevisionRequest>, JsonRejection>,
) -> Result<Json<Save>> {
    let request = json(input)?;
    let _operation = app
        .save_operations
        .try_lock()
        .map_err(|_| Error::conflict("A save operation is in progress"))?;
    let inner = app.inner.lock().await;
    if inner.child.is_some() {
        return Err(Error::conflict("Stop the server before restoring saves"));
    }
    Ok(Json(storage::restore_save(
        &app.directory,
        &id,
        &request.revision,
    )?))
}
#[utoipa::path(post, path="/api/server/command", request_body=ConsoleCommandRequest, responses((status=200, body=CommandResponse), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn command(
    State(app): State<App>,
    input: std::result::Result<Json<ConsoleCommandRequest>, JsonRejection>,
) -> Result<Json<CommandResponse>> {
    Ok(Json(process::send_command(&app, json(input)?).await?))
}
#[utoipa::path(post, path="/api/server/force-stop", request_body=ForceStopRequest, responses((status=200, body=Status), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn force_stop(
    State(app): State<App>,
    input: std::result::Result<Json<ForceStopRequest>, JsonRejection>,
) -> Result<Json<Status>> {
    Ok(Json(process::force_stop(&app, json(input)?.confirm).await?))
}
#[utoipa::path(get, path="/api/logs/file", responses((status=200, body=GameLogDocument), (status=400, body=ApiError)))]
async fn game_log(State(app): State<App>) -> Result<Json<GameLogDocument>> {
    let _inner = app.inner.lock().await;
    Ok(Json(storage::game_log(
        &app.directory,
        &app.settings_path(),
    )?))
}

#[utoipa::path(get, path="/api/runtime-paths", responses((status=200, body=RuntimePaths), (status=400, body=ApiError)))]
async fn runtime_paths(State(app): State<App>) -> Result<Json<RuntimePaths>> {
    let _inner = app.inner.lock().await;
    Ok(Json(storage::runtime_paths(
        &app.directory,
        app.binary.as_deref(),
    )?))
}
