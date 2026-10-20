use crate::{App, Error, Result, models::*, process, storage};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{DefaultBodyLimit, Path, State, rejection::JsonRejection},
    http::{Request, StatusCode, header},
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use tower_http::services::{ServeDir, ServeFile};
use utoipa::OpenApi;

#[derive(OpenApi)]
#[openapi(
    paths(
        status,
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
        ApiError
    ))
)]
pub struct Api;

pub fn router(app: App, static_path: &std::path::Path) -> Router {
    let routes = Router::new()
        .merge(crate::mods::routes())
        .route("/status", get(status))
        .route("/telemetry", get(telemetry))
        .route("/saves", get(saves))
        .route(
            "/saves/{name}",
            get(download)
                .post(upload)
                .layer(DefaultBodyLimit::max(64 * 1024 * 1024)),
        )
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
        .nest("/api", routes)
        .fallback_service(
            ServeDir::new(static_path)
                .not_found_service(ServeFile::new(static_path.join("index.html"))),
        )
        .layer(middleware::from_fn(guard))
        .with_state(app)
}
async fn guard(request: Request<Body>, next: Next) -> Response {
    let host = request
        .headers()
        .get(header::HOST)
        .and_then(|h| h.to_str().ok())
        .and_then(|h| h.parse::<axum::http::uri::Authority>().ok());
    let valid_host = host.as_ref().is_some_and(|h| {
        h.host() == "localhost"
            || h.host()
                .trim_matches(['[', ']'])
                .parse::<std::net::IpAddr>()
                .is_ok_and(|ip| ip.is_loopback())
    });
    if !valid_host {
        return (
            StatusCode::FORBIDDEN,
            Json(ApiError {
                message: "Only loopback Host headers are accepted".into(),
            }),
        )
            .into_response();
    }
    if !matches!(
        *request.method(),
        axum::http::Method::GET | axum::http::Method::HEAD | axum::http::Method::OPTIONS
    ) {
        if let Some(origin) = request.headers().get(header::ORIGIN) {
            let valid =
                origin
                    .to_str()
                    .ok()
                    .and_then(|o| url::Url::parse(o).ok())
                    .is_some_and(|origin| {
                        host.as_ref().is_some_and(|host| {
                            matches!(origin.scheme(), "http" | "https")
                                && origin.host_str().map(|h| h.trim_matches(['[', ']']))
                                    == Some(host.host().trim_matches(['[', ']']))
                                && origin.port_or_known_default()
                                    == Some(host.port_u16().unwrap_or(
                                        if origin.scheme() == "https" { 443 } else { 80 },
                                    ))
                        })
                    });
            if !valid {
                return (
                    StatusCode::FORBIDDEN,
                    Json(ApiError {
                        message: "Cross-origin mutations are not accepted".into(),
                    }),
                )
                    .into_response();
            }
        }
        let content_type = request
            .headers()
            .get(header::CONTENT_TYPE)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.split(';').next());
        let zip_upload = request.method() == axum::http::Method::POST
            && (request.uri().path().starts_with("/api/saves/")
                || request.uri().path() == "/api/mods/upload");
        let binary_upload = request.method() == axum::http::Method::POST
            && request.uri().path() == "/api/mods/settings/file";
        if content_type
            != Some(if zip_upload {
                "application/zip"
            } else if binary_upload {
                "application/octet-stream"
            } else {
                "application/json"
            })
        {
            return (
                StatusCode::UNSUPPORTED_MEDIA_TYPE,
                Json(ApiError {
                    message: "Unexpected request Content-Type".into(),
                }),
            )
                .into_response();
        }
    }
    next.run(request).await
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
    Ok(Json(process::start(&app, &json(input)?.save).await?))
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
#[utoipa::path(post, path="/api/saves/{name}", params(("name"=String, Path)), request_body(content=String, content_type="application/zip"), responses((status=201, body=Save), (status=400, body=ApiError), (status=409, body=ApiError)))]
async fn upload(
    State(app): State<App>,
    Path(name): Path<String>,
    bytes: std::result::Result<Bytes, axum::extract::rejection::BytesRejection>,
) -> Result<(StatusCode, Json<Save>)> {
    let bytes = bytes.map_err(|e| Error {
        status: e.status(),
        message: e.body_text(),
    })?;
    let _guard = app.inner.lock().await;
    let path = storage::save_path(&app.saves_path(), &name)?;
    if path.exists() {
        return Err(Error::conflict("A save with that name already exists"));
    }
    storage::validate_save(&bytes)?;
    storage::atomic_write(&path, &bytes, false)?;
    Ok((
        StatusCode::CREATED,
        Json(storage::save_info(&app.saves_path(), &name)?),
    ))
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
