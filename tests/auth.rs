#![forbid(unsafe_code)]
#![deny(clippy::unwrap_used, clippy::expect_used, unused_must_use)]

pub use factorio_control::{Error, Result, now, storage};
#[path = "../src/auth.rs"]
pub mod auth;

use auth::{Auth, Role};
use axum::{
    Router,
    body::Body,
    http::{Method, Request, StatusCode, header},
    middleware,
    routing::{get, post},
};
use http_body_util::BodyExt;
use serde_json::{Value, json};
use std::{net::SocketAddr, path::Path};
use tower::ServiceExt;

const PASSWORD: &str = "test-only-password-123";
type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;

fn app(auth: Auth) -> Router {
    auth::routes(auth.clone())
        .route("/api/status", get(|| async { "{}" }))
        .route("/api/settings", get(|| async { "{}" }))
        .route("/api/server/start", post(|| async { "{}" }))
        .route("/api/saves/create", post(|| async { "{}" }))
        .route("/api/saves/example.zip", post(|| async { "{}" }))
        .layer(middleware::from_fn_with_state(auth, auth::guard))
}
fn configured(path: &Path, public_url: Option<&str>) -> std::result::Result<Auth, Error> {
    auth::bootstrap_admin(path, "owner", PASSWORD)?;
    Auth::load(path, public_url)
}
struct Reply {
    status: StatusCode,
    cookie: Option<String>,
    body: Value,
}
async fn call(
    router: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    input: Option<Value>,
    origin: Option<&str>,
    host: &str,
) -> std::result::Result<Reply, Box<dyn std::error::Error>> {
    let mut request = Request::builder()
        .method(method)
        .uri(path)
        .header(header::HOST, host);
    if let Some(cookie) = cookie {
        request = request.header(header::COOKIE, cookie);
    }
    if let Some(origin) = origin {
        request = request.header(header::ORIGIN, origin);
    }
    let body = if let Some(input) = input {
        request = request.header(header::CONTENT_TYPE, "application/json");
        serde_json::to_vec(&input)?
    } else {
        Vec::new()
    };
    let response = router
        .clone()
        .oneshot(request.body(Body::from(body))?)
        .await?;
    assert_eq!(
        response
            .headers()
            .get(header::CACHE_CONTROL)
            .and_then(|h| h.to_str().ok()),
        Some("no-store")
    );
    let status = response.status();
    let cookie = response
        .headers()
        .get(header::SET_COOKIE)
        .and_then(|h| h.to_str().ok())
        .map(str::to_owned);
    let bytes = response.into_body().collect().await?.to_bytes();
    let body = if bytes.is_empty() {
        Value::Null
    } else {
        serde_json::from_slice(&bytes)?
    };
    Ok(Reply {
        status,
        cookie,
        body,
    })
}
async fn local(
    router: &Router,
    method: Method,
    path: &str,
    cookie: Option<&str>,
    input: Option<Value>,
) -> std::result::Result<Reply, Box<dyn std::error::Error>> {
    call(
        router,
        method,
        path,
        cookie,
        input,
        Some("http://127.0.0.1:9400"),
        "127.0.0.1:9400",
    )
    .await
}
async fn sign_in(
    router: &Router,
    username: &str,
    password: &str,
) -> std::result::Result<String, Box<dyn std::error::Error>> {
    let reply = local(
        router,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username":username,"password":password})),
    )
    .await?;
    assert_eq!(reply.status, StatusCode::OK);
    let cookie = reply.cookie.ok_or("Missing session cookie")?;
    assert!(
        cookie.contains("HttpOnly")
            && cookie.contains("SameSite=Strict")
            && cookie.contains("Max-Age=28800")
    );
    Ok(cookie
        .split(';')
        .next()
        .ok_or("Missing session cookie")?
        .to_owned())
}

#[tokio::test]
async fn local_mode_is_only_available_without_account_file() -> TestResult {
    let temp = tempfile::tempdir()?;
    let auth = Auth::load(temp.path(), None).map_err(|e| e.message)?;
    auth.validate_bind("127.0.0.1:9400".parse::<SocketAddr>()?)
        .await
        .map_err(|e| e.message)?;
    assert!(
        auth.validate_bind("0.0.0.0:9400".parse::<SocketAddr>()?)
            .await
            .is_err()
    );
    let router = app(auth);
    let reply = local(&router, Method::GET, "/api/auth/status", None, None).await?;
    assert_eq!(reply.body["enabled"], false);
    assert_eq!(
        local(&router, Method::GET, "/api/status", None, None)
            .await?
            .status,
        StatusCode::OK
    );
    assert_eq!(
        local(&router, Method::GET, "/api/auth/accounts", None, None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    assert!(Auth::load(temp.path(), Some("https://panel.example")).is_err());
    Ok(())
}

#[tokio::test]
async fn password_hashes_are_private_and_bootstrap_cannot_overwrite() -> TestResult {
    let temp = tempfile::tempdir()?;
    configured(temp.path(), None).map_err(|e| e.message)?;
    let bytes = std::fs::read(temp.path().join("panel-accounts.json"))?;
    let text = String::from_utf8(bytes.clone())?;
    assert!(!text.contains(PASSWORD));
    assert!(text.contains("$argon2id$"));
    use std::os::unix::fs::PermissionsExt;
    assert_eq!(
        std::fs::metadata(temp.path().join("panel-accounts.json"))?
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert!(auth::bootstrap_admin(temp.path(), "another", PASSWORD).is_err());
    assert_eq!(
        std::fs::read(temp.path().join("panel-accounts.json"))?,
        bytes
    );
    Ok(())
}

#[tokio::test]
async fn session_authentication_logout_and_restart_revoke_access() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    assert_eq!(
        local(&router, Method::GET, "/api/status", None, None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    let cookie = sign_in(&router, "owner", PASSWORD).await?;
    let status = local(
        &router,
        Method::GET,
        "/api/auth/status",
        Some(&cookie),
        None,
    )
    .await?;
    assert_eq!(status.body["user"]["role"], "admin");
    assert!(
        status.body["expires_at"]
            .as_u64()
            .is_some_and(|expiry| expiry > now() && expiry <= now() + 28800)
    );
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&cookie), None)
            .await?
            .status,
        StatusCode::OK
    );
    let restarted = app(Auth::load(temp.path(), None).map_err(|e| e.message)?);
    assert_eq!(
        local(&restarted, Method::GET, "/api/status", Some(&cookie), None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    let logout = local(
        &router,
        Method::POST,
        "/api/auth/logout",
        Some(&cookie),
        Some(json!({})),
    )
    .await?;
    assert_eq!(logout.status, StatusCode::NO_CONTENT);
    assert!(
        logout
            .cookie
            .is_some_and(|cookie| cookie.contains("Max-Age=0"))
    );
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&cookie), None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn login_failures_are_generic_and_rate_limited() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    for attempt in 0..6 {
        let response = local(
            &router,
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({"username":"owner","password":"incorrect"})),
        )
        .await?;
        assert_eq!(
            response.status,
            if attempt == 5 {
                StatusCode::TOO_MANY_REQUESTS
            } else {
                StatusCode::UNAUTHORIZED
            }
        );
        if attempt < 5 {
            assert_eq!(response.body["message"], "Invalid account name or password");
        }
    }
    let unknown = local(
        &router,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username":"missing","password":"incorrect"})),
    )
    .await?;
    assert_eq!(unknown.status, StatusCode::UNAUTHORIZED);
    assert_eq!(unknown.body["message"], "Invalid account name or password");
    Ok(())
}

#[tokio::test]
async fn public_origin_and_loopback_modes_reject_cross_origin_requests() -> TestResult {
    let temp = tempfile::tempdir()?;
    let local_auth = configured(temp.path(), None).map_err(|e| e.message)?;
    let local_app = app(local_auth);
    assert_eq!(
        call(
            &local_app,
            Method::GET,
            "/api/status",
            None,
            None,
            None,
            "evil.example"
        )
        .await?
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &local_app,
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({})),
            Some("https://evil.example"),
            "127.0.0.1:9400"
        )
        .await?
        .status,
        StatusCode::FORBIDDEN
    );
    let public = Auth::load(temp.path(), Some("https://panel.example")).map_err(|e| e.message)?;
    public
        .validate_bind("0.0.0.0:9400".parse::<SocketAddr>()?)
        .await
        .map_err(|e| e.message)?;
    let public_app = app(public);
    assert_eq!(
        call(
            &public_app,
            Method::POST,
            "/api/auth/login",
            None,
            Some(json!({})),
            None,
            "panel.example"
        )
        .await?
        .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        call(
            &public_app,
            Method::GET,
            "/api/auth/status",
            None,
            None,
            None,
            "127.0.0.1:9400"
        )
        .await?
        .status,
        StatusCode::FORBIDDEN
    );
    let signed = call(
        &public_app,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username":"owner","password":PASSWORD})),
        Some("https://panel.example"),
        "panel.example",
    )
    .await?;
    assert_eq!(signed.status, StatusCode::OK);
    assert!(
        signed
            .cookie
            .is_some_and(|cookie| cookie.ends_with("; Secure"))
    );
    Ok(())
}

#[test]
fn public_origin_requires_https_and_no_credentials_or_path() -> TestResult {
    let temp = tempfile::tempdir()?;
    auth::bootstrap_admin(temp.path(), "owner", PASSWORD).map_err(|e| e.message)?;
    for value in [
        "http://panel.example",
        "https://user:pass@panel.example",
        "https://panel.example/path",
        "https://panel.example/?q=1",
        "https://panel.example/#fragment",
        "invalid",
    ] {
        assert!(Auth::load(temp.path(), Some(value)).is_err());
    }
    Ok(())
}

#[test]
fn roles_enforce_privileged_settings_and_game_mutations() {
    for path in [
        "/api/settings",
        "/api/settings/raw",
        "/api/manager-settings",
        "/api/admins",
        "/api/auth/accounts",
        "/api/auth/accounts/create",
    ] {
        assert!(!auth::permits(Role::Viewer, &Method::GET, path));
        assert!(!auth::permits(Role::Operator, &Method::GET, path));
        assert!(auth::permits(Role::Admin, &Method::GET, path));
    }
    for path in [
        "/api/server/start",
        "/api/server/command",
        "/api/server/force-stop",
        "/api/saves/create",
        "/api/saves/x.zip/archive",
        "/api/mods/portal/install",
        "/api/mods/bulk/apply",
        "/api/mods/list",
    ] {
        assert!(!auth::permits(Role::Viewer, &Method::POST, path));
        assert!(auth::permits(Role::Operator, &Method::POST, path));
    }
    for path in [
        "/api/status",
        "/api/telemetry",
        "/api/logs",
        "/api/logs/file",
        "/api/saves",
        "/api/saves/x.zip",
        "/api/mods",
        "/api/mods/list",
        "/api/mods/portal/search",
        "/api/mods/packs",
    ] {
        assert!(auth::permits(Role::Viewer, &Method::GET, path));
    }
    assert!(!auth::permits(
        Role::Operator,
        &Method::POST,
        "/api/future-sensitive-endpoint"
    ));
}

#[tokio::test]
async fn account_revision_role_changes_and_deletion_revoke_sessions() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    let owner = sign_in(&router, "owner", PASSWORD).await?;
    let document = local(
        &router,
        Method::GET,
        "/api/auth/accounts",
        Some(&owner),
        None,
    )
    .await?
    .body;
    assert!(!document.to_string().contains("password"));
    let create = local(&router, Method::POST, "/api/auth/accounts/create", Some(&owner), Some(json!({"revision":document["revision"],"username":"reader","password":PASSWORD,"role":"viewer"}))).await?;
    assert_eq!(create.status, StatusCode::OK);
    let reader = sign_in(&router, "reader", PASSWORD).await?;
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&reader), None)
            .await?
            .status,
        StatusCode::OK
    );
    assert_eq!(
        local(&router, Method::GET, "/api/settings", Some(&reader), None)
            .await?
            .status,
        StatusCode::FORBIDDEN
    );
    assert_eq!(
        local(
            &router,
            Method::POST,
            "/api/server/start",
            Some(&reader),
            Some(json!({}))
        )
        .await?
        .status,
        StatusCode::FORBIDDEN
    );
    let stale = local(&router, Method::POST, "/api/auth/accounts/update", Some(&owner), Some(json!({"revision":document["revision"],"username":"reader","password":null,"role":"operator"}))).await?;
    assert_eq!(stale.status, StatusCode::CONFLICT);
    let update = local(&router, Method::POST, "/api/auth/accounts/update", Some(&owner), Some(json!({"revision":create.body["revision"],"username":"reader","password":null,"role":"operator"}))).await?;
    assert_eq!(update.status, StatusCode::OK);
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&reader), None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    let operator = sign_in(&router, "reader", PASSWORD).await?;
    assert_eq!(
        local(
            &router,
            Method::POST,
            "/api/server/start",
            Some(&operator),
            Some(json!({}))
        )
        .await?
        .status,
        StatusCode::OK
    );
    let delete = local(
        &router,
        Method::POST,
        "/api/auth/accounts/delete",
        Some(&owner),
        Some(json!({"revision":update.body["revision"],"username":"reader"})),
    )
    .await?;
    assert_eq!(delete.status, StatusCode::OK);
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&operator), None)
            .await?
            .status,
        StatusCode::UNAUTHORIZED
    );
    Ok(())
}

#[tokio::test]
async fn last_administrator_and_self_deletion_are_protected() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    let cookie = sign_in(&router, "owner", PASSWORD).await?;
    let document = local(
        &router,
        Method::GET,
        "/api/auth/accounts",
        Some(&cookie),
        None,
    )
    .await?
    .body;
    let bytes = std::fs::read(temp.path().join("panel-accounts.json"))?;
    let demote = local(&router, Method::POST, "/api/auth/accounts/update", Some(&cookie), Some(json!({"revision":document["revision"],"username":"owner","role":"viewer","password":null}))).await?;
    assert_eq!(demote.status, StatusCode::CONFLICT);
    let delete = local(
        &router,
        Method::POST,
        "/api/auth/accounts/delete",
        Some(&cookie),
        Some(json!({"revision":document["revision"],"username":"owner"})),
    )
    .await?;
    assert_eq!(delete.status, StatusCode::BAD_REQUEST);
    assert_eq!(
        std::fs::read(temp.path().join("panel-accounts.json"))?,
        bytes
    );
    Ok(())
}

#[tokio::test]
async fn password_changes_revoke_all_sessions_and_require_current_password() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    let first = sign_in(&router, "owner", PASSWORD).await?;
    let second = sign_in(&router, "owner", PASSWORD).await?;
    let wrong = local(
        &router,
        Method::POST,
        "/api/auth/password",
        Some(&first),
        Some(json!({"current_password":"incorrect","password":"a-different-test-password"})),
    )
    .await?;
    assert_eq!(wrong.status, StatusCode::BAD_REQUEST);
    let changed = local(
        &router,
        Method::POST,
        "/api/auth/password",
        Some(&first),
        Some(json!({"current_password":PASSWORD,"password":"a-different-test-password"})),
    )
    .await?;
    assert_eq!(changed.status, StatusCode::NO_CONTENT);
    for cookie in [first, second] {
        assert_eq!(
            local(&router, Method::GET, "/api/status", Some(&cookie), None)
                .await?
                .status,
            StatusCode::UNAUTHORIZED
        );
    }
    let old = local(
        &router,
        Method::POST,
        "/api/auth/login",
        None,
        Some(json!({"username":"owner","password":PASSWORD})),
    )
    .await?;
    assert_eq!(old.status, StatusCode::UNAUTHORIZED);
    sign_in(&router, "owner", "a-different-test-password").await?;
    Ok(())
}

#[tokio::test]
async fn deleting_accounts_file_fails_closed_and_does_not_restore_local_mode() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(configured(temp.path(), None).map_err(|e| e.message)?);
    let cookie = sign_in(&router, "owner", PASSWORD).await?;
    std::fs::remove_file(temp.path().join("panel-accounts.json"))?;
    assert_eq!(
        local(&router, Method::GET, "/api/status", Some(&cookie), None)
            .await?
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    assert_eq!(
        local(&router, Method::GET, "/api/auth/status", None, None)
            .await?
            .status,
        StatusCode::SERVICE_UNAVAILABLE
    );
    Ok(())
}

#[tokio::test]
async fn json_save_creation_is_not_mistaken_for_zip_upload() -> TestResult {
    let temp = tempfile::tempdir()?;
    let router = app(Auth::load(temp.path(), None).map_err(|e| e.message)?);
    assert_eq!(
        local(
            &router,
            Method::POST,
            "/api/saves/create",
            None,
            Some(json!({"name":"test"}))
        )
        .await?
        .status,
        StatusCode::OK
    );
    assert_eq!(
        local(
            &router,
            Method::POST,
            "/api/saves/example.zip",
            None,
            Some(json!({}))
        )
        .await?
        .status,
        StatusCode::UNSUPPORTED_MEDIA_TYPE
    );
    Ok(())
}
