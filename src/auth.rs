use crate::{Error, Result, storage};
use argon2::{Argon2, PasswordHash, PasswordHasher, PasswordVerifier, password_hash::SaltString};
use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, State},
    http::{HeaderMap, Method, Request, StatusCode, header},
    middleware::Next,
    response::{IntoResponse, Response},
    routing::{get, post},
};
use rand_core::{OsRng, RngCore};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::HashMap,
    net::SocketAddr,
    path::{Path, PathBuf},
    sync::Arc,
    time::{Duration, Instant},
};
use tokio::sync::{Mutex, Semaphore};
use utoipa::ToSchema;

const COOKIE: &str = "fc_session";
const SESSION_SECONDS: u64 = 8 * 60 * 60;
const MAX_ACCOUNTS: usize = 100;
const MAX_SESSIONS: usize = 1000;
const MAX_RATE_KEYS: usize = 1024;

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Operator,
    Viewer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct Account {
    pub username: String,
    pub role: Role,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct SessionStatus {
    pub enabled: bool,
    pub user: Option<Account>,
    pub expires_at: Option<u64>,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct AccountsDocument {
    pub revision: String,
    pub accounts: Vec<Account>,
}

#[derive(Deserialize, ToSchema)]
pub struct LoginRequest {
    pub username: String,
    pub password: String,
}
#[derive(Deserialize, ToSchema)]
pub struct CreateAccount {
    pub revision: String,
    pub username: String,
    pub password: String,
    pub role: Role,
}
#[derive(Deserialize, ToSchema)]
pub struct UpdateAccount {
    pub revision: String,
    pub username: String,
    pub password: Option<String>,
    pub role: Role,
}
#[derive(Deserialize, ToSchema)]
pub struct DeleteAccount {
    pub revision: String,
    pub username: String,
}
#[derive(Deserialize, ToSchema)]
pub struct PasswordChange {
    pub current_password: String,
    pub password: String,
}

#[derive(Clone, Deserialize, Serialize)]
struct StoredAccount {
    username: String,
    role: Role,
    password_hash: String,
}
#[derive(Default, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct StoredAccounts {
    accounts: Vec<StoredAccount>,
}
struct Session {
    username: String,
    expires_at: u64,
}
struct Attempts {
    count: u32,
    started: Instant,
}
struct Runtime {
    revision: String,
    accounts: Vec<StoredAccount>,
    sessions: HashMap<String, Session>,
    attempts: HashMap<String, Attempts>,
}

#[derive(Clone)]
pub struct Auth {
    path: PathBuf,
    public_origin: Option<url::Url>,
    inner: Arc<Mutex<Runtime>>,
    password_slots: Arc<Semaphore>,
    dummy_hash: Arc<str>,
}

fn error(status: StatusCode, message: &str) -> Error {
    Error {
        status,
        message: message.into(),
    }
}
fn unauthorized() -> Error {
    error(StatusCode::UNAUTHORIZED, "Sign in to continue")
}
fn forbidden() -> Error {
    error(
        StatusCode::FORBIDDEN,
        "Your account does not have permission for this action",
    )
}
fn public_account(account: &StoredAccount) -> Account {
    Account {
        username: account.username.clone(),
        role: account.role,
    }
}

fn validate_username(username: &str) -> Result<()> {
    if !(1..=64).contains(&username.len())
        || !username
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || matches!(b, b'-' | b'_' | b'.'))
    {
        return Err(Error::bad(
            "Account names must contain 1–64 letters, digits, dots, underscores or hyphens",
        ));
    }
    Ok(())
}
fn validate_password(password: &str) -> Result<()> {
    if !(12..=1024).contains(&password.len()) {
        return Err(Error::bad("Passwords must contain 12–1024 bytes"));
    }
    Ok(())
}
fn hash_password(password: &str) -> Result<String> {
    validate_password(password)?;
    let mut bytes = [0u8; 16];
    OsRng.try_fill_bytes(&mut bytes).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Secure randomness is unavailable",
        )
    })?;
    let salt = SaltString::encode_b64(&bytes).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Unable to protect password",
        )
    })?;
    Argon2::default()
        .hash_password(password.as_bytes(), &salt)
        .map(|hash| hash.to_string())
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to protect password",
            )
        })
}
fn verify_password(password: &str, stored: &str) -> bool {
    PasswordHash::new(stored).is_ok_and(|hash| {
        Argon2::default()
            .verify_password(password.as_bytes(), &hash)
            .is_ok()
    })
}
fn read_accounts(path: &Path) -> Result<(String, Vec<StoredAccount>)> {
    let (revision, bytes) = storage::read_document(path)?;
    let accounts = match bytes {
        Some(bytes) => {
            serde_json::from_slice::<StoredAccounts>(&bytes)
                .map_err(|_| {
                    error(
                        StatusCode::INTERNAL_SERVER_ERROR,
                        "Invalid panel account file",
                    )
                })?
                .accounts
        }
        None => Vec::new(),
    };
    if revision != "missing"
        && (accounts.is_empty()
            || accounts.len() > MAX_ACCOUNTS
            || !accounts.iter().any(|a| a.role == Role::Admin))
    {
        return Err(error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Panel accounts require at least one administrator and at most 100 accounts",
        ));
    }
    let mut names = std::collections::HashSet::new();
    for account in &accounts {
        validate_username(&account.username)?;
        if !names.insert(account.username.clone())
            || !account.password_hash.starts_with("$argon2id$")
            || PasswordHash::new(&account.password_hash).is_err()
        {
            return Err(error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Invalid panel account file",
            ));
        }
    }
    Ok((revision, accounts))
}

impl Auth {
    pub fn load(directory: &Path, public_url: Option<&str>) -> Result<Self> {
        storage::ensure_directory(directory)?;
        let public_origin = public_url.map(|value| {
            let url = url::Url::parse(value).map_err(|_| Error::bad("FC_PUBLIC_URL must be an HTTPS origin"))?;
            if url.scheme() != "https" || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
                return Err(Error::bad("FC_PUBLIC_URL must be an HTTPS origin without credentials, path, query or fragment"));
            }
            Ok(url)
        }).transpose()?;
        let path = directory.join("panel-accounts.json");
        let (revision, accounts) = read_accounts(&path)?;
        if public_origin.is_some() && accounts.is_empty() {
            return Err(Error::bad(
                "Bootstrap an administrator before configuring FC_PUBLIC_URL",
            ));
        }
        let mut secret = [0u8; 32];
        OsRng.try_fill_bytes(&mut secret).map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Secure randomness is unavailable",
            )
        })?;
        let dummy_hash = hash_password(&hex(&secret))?.into();
        Ok(Self {
            path,
            public_origin,
            inner: Arc::new(Mutex::new(Runtime {
                revision,
                accounts,
                sessions: HashMap::new(),
                attempts: HashMap::new(),
            })),
            password_slots: Arc::new(Semaphore::new(2)),
            dummy_hash,
        })
    }

    pub async fn validate_bind(&self, bind: SocketAddr) -> Result<()> {
        if !bind.ip().is_loopback() {
            let mut runtime = self.inner.lock().await;
            self.refresh(&mut runtime)?;
            if runtime.accounts.is_empty() || self.public_origin.is_none() {
                return Err(Error::bad(
                    "Nonloopback FC_BIND requires panel accounts and an explicit HTTPS FC_PUBLIC_URL",
                ));
            }
        }
        Ok(())
    }

    fn refresh(&self, runtime: &mut Runtime) -> Result<()> {
        let (revision, accounts) = read_accounts(&self.path)?;
        if revision != runtime.revision {
            if accounts.is_empty() && !runtime.accounts.is_empty() {
                return Err(error(
                    StatusCode::SERVICE_UNAVAILABLE,
                    "Panel account file is missing; access is disabled",
                ));
            }
            runtime.revision = revision;
            runtime.accounts = accounts;
            runtime.sessions.clear();
        }
        Ok(())
    }

    fn write_accounts(
        &self,
        runtime: &mut Runtime,
        accounts: Vec<StoredAccount>,
    ) -> Result<AccountsDocument> {
        let bytes = serde_json::to_vec_pretty(&StoredAccounts { accounts })?;
        storage::atomic_write(&self.path, &bytes, true)?;
        let (revision, accounts) = read_accounts(&self.path)?;
        runtime.revision = revision;
        runtime.accounts = accounts;
        Ok(accounts_document(runtime))
    }

    fn cookie(&self, token: &str, max_age: u64) -> Result<axum::http::HeaderValue> {
        format!(
            "{COOKIE}={token}; Path=/; HttpOnly; SameSite=Strict; Max-Age={max_age}{}",
            if self.public_origin.is_some() {
                "; Secure"
            } else {
                ""
            }
        )
        .parse()
        .map_err(|_| {
            error(
                StatusCode::INTERNAL_SERVER_ERROR,
                "Unable to create session cookie",
            )
        })
    }

    fn check_request(&self, request: &Request<Body>) -> Result<()> {
        let host = request
            .headers()
            .get(header::HOST)
            .and_then(|h| h.to_str().ok())
            .and_then(|h| h.parse::<axum::http::uri::Authority>().ok())
            .ok_or_else(|| error(StatusCode::FORBIDDEN, "A valid Host header is required"))?;
        let expected = if let Some(origin) = &self.public_origin {
            let authority = origin[url::Position::BeforeHost..url::Position::AfterPort].to_owned();
            if !host.as_str().eq_ignore_ascii_case(&authority) {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "Host must match FC_PUBLIC_URL",
                ));
            }
            origin.origin().ascii_serialization()
        } else {
            if host.host() != "localhost"
                && !host
                    .host()
                    .trim_matches(['[', ']'])
                    .parse::<std::net::IpAddr>()
                    .is_ok_and(|ip| ip.is_loopback())
            {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "Only loopback Host headers are accepted",
                ));
            }
            format!("http://{host}")
        };
        if !matches!(
            *request.method(),
            Method::GET | Method::HEAD | Method::OPTIONS
        ) {
            let origin = request
                .headers()
                .get(header::ORIGIN)
                .and_then(|h| h.to_str().ok());
            if origin.is_some_and(|origin| origin != expected)
                || (self.public_origin.is_some() && origin.is_none())
            {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "Same-origin requests are required",
                ));
            }
            if request
                .headers()
                .get("sec-fetch-site")
                .is_some_and(|value| value != "same-origin" && value != "none")
            {
                return Err(error(
                    StatusCode::FORBIDDEN,
                    "Cross-site requests are not accepted",
                ));
            }
            if matches!(
                *request.method(),
                Method::POST | Method::PUT | Method::PATCH
            ) {
                let path = request.uri().path();
                let upload = path
                    .strip_prefix("/api/saves/")
                    .is_some_and(|name| name.ends_with(".zip") && !name.contains('/'))
                    || path == "/api/mods/upload";
                let expected = if upload && request.method() == Method::POST {
                    "application/zip"
                } else if path == "/api/mods/settings/file" && request.method() == Method::POST {
                    "application/octet-stream"
                } else {
                    "application/json"
                };
                if request
                    .headers()
                    .get(header::CONTENT_TYPE)
                    .and_then(|h| h.to_str().ok())
                    .and_then(|h| h.split(';').next())
                    != Some(expected)
                {
                    return Err(error(
                        StatusCode::UNSUPPORTED_MEDIA_TYPE,
                        "Unexpected request Content-Type",
                    ));
                }
            }
        }
        Ok(())
    }
}

fn accounts_document(runtime: &Runtime) -> AccountsDocument {
    AccountsDocument {
        revision: runtime.revision.clone(),
        accounts: runtime.accounts.iter().map(public_account).collect(),
    }
}
fn hex(bytes: &[u8]) -> String {
    bytes.iter().map(|b| format!("{b:02x}")).collect()
}
fn token_key(token: &str) -> String {
    format!("{:x}", Sha256::digest(token.as_bytes()))
}
fn request_token(headers: &HeaderMap) -> Option<&str> {
    let cookies = headers.get(header::COOKIE)?.to_str().ok()?;
    let mut matches = cookies
        .split(';')
        .filter_map(|part| part.trim().strip_prefix(&format!("{COOKIE}=")));
    let token = matches.next()?;
    if matches.next().is_some()
        || token.len() != 64
        || !token.bytes().all(|b| b.is_ascii_hexdigit())
    {
        return None;
    }
    Some(token)
}
fn identify(runtime: &mut Runtime, headers: &HeaderMap) -> Option<(Account, u64)> {
    runtime
        .sessions
        .retain(|_, session| session.expires_at > crate::now());
    let session = runtime.sessions.get(&token_key(request_token(headers)?))?;
    let account = runtime
        .accounts
        .iter()
        .find(|account| account.username == session.username)?;
    Some((public_account(account), session.expires_at))
}

pub fn permits(role: Role, method: &Method, path: &str) -> bool {
    if role == Role::Admin {
        return true;
    }
    if [
        "/api/settings",
        "/api/manager-settings",
        "/api/admins",
        "/api/auth/accounts",
    ]
    .iter()
    .any(|prefix| path == *prefix || path.starts_with(&format!("{prefix}/")))
    {
        return false;
    }
    if path == "/api/auth/password" || path == "/api/auth/logout" {
        return true;
    }
    if role == Role::Operator
        && ["/api/server/", "/api/saves", "/api/mods"]
            .iter()
            .any(|prefix| path == *prefix || path.starts_with(prefix))
    {
        return true;
    }
    if !matches!(*method, Method::GET | Method::HEAD) {
        return false;
    }
    matches!(
        path,
        "/api/status"
            | "/api/telemetry"
            | "/api/logs"
            | "/api/logs/file"
            | "/api/saves"
            | "/api/mods"
    ) || path.starts_with("/api/saves/")
        || path.starts_with("/api/mods/")
}

pub async fn guard(State(auth): State<Auth>, mut request: Request<Body>, next: Next) -> Response {
    let result = async {
        auth.check_request(&request)?;
        if request.uri().path().starts_with("/api/") {
            let mut runtime = auth.inner.lock().await;
            auth.refresh(&mut runtime)?;
            if !runtime.accounts.is_empty()
                && !matches!(
                    (request.method(), request.uri().path()),
                    (&Method::GET, "/api/auth/status") | (&Method::POST, "/api/auth/login")
                )
            {
                let (account, _) =
                    identify(&mut runtime, request.headers()).ok_or_else(unauthorized)?;
                if !permits(account.role, request.method(), request.uri().path()) {
                    return Err(forbidden());
                }
                request.extensions_mut().insert(account);
            }
        }
        Ok(())
    }
    .await;
    let mut response = match result {
        Ok(()) => next.run(request).await,
        Err(error) => error.into_response(),
    };
    response.headers_mut().insert(
        header::CACHE_CONTROL,
        axum::http::HeaderValue::from_static("no-store"),
    );
    response.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        axum::http::HeaderValue::from_static("nosniff"),
    );
    response.headers_mut().insert(
        "x-frame-options",
        axum::http::HeaderValue::from_static("DENY"),
    );
    response
}

pub fn routes<S: Clone + Send + Sync + 'static>(auth: Auth) -> Router<S> {
    Router::new()
        .route("/api/auth/status", get(status))
        .route("/api/auth/login", post(login))
        .route("/api/auth/logout", post(logout))
        .route("/api/auth/password", post(change_password))
        .route("/api/auth/accounts", get(accounts))
        .route("/api/auth/accounts/create", post(create_account))
        .route("/api/auth/accounts/update", post(update_account))
        .route("/api/auth/accounts/delete", post(delete_account))
        .layer(DefaultBodyLimit::max(16 * 1024))
        .with_state(auth)
}

#[utoipa::path(get, path="/api/auth/status", responses((status=200, body=SessionStatus)))]
pub async fn status(State(auth): State<Auth>, headers: HeaderMap) -> Result<Json<SessionStatus>> {
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    let session = identify(&mut runtime, &headers);
    Ok(Json(SessionStatus {
        enabled: !runtime.accounts.is_empty(),
        user: session.as_ref().map(|(account, _)| account.clone()),
        expires_at: session.map(|(_, expiry)| expiry),
    }))
}

fn rate_limit(runtime: &mut Runtime, key: &str, limit: u32, duration: Duration) -> Result<()> {
    runtime
        .attempts
        .retain(|_, attempt| attempt.started.elapsed() < Duration::from_secs(900));
    if !runtime.attempts.contains_key(key) && runtime.attempts.len() >= MAX_RATE_KEYS {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts; try again later",
        ));
    }
    let attempt = runtime.attempts.entry(key.to_owned()).or_insert(Attempts {
        count: 0,
        started: Instant::now(),
    });
    if attempt.started.elapsed() >= duration {
        attempt.count = 0;
        attempt.started = Instant::now();
    }
    if attempt.count >= limit {
        return Err(error(
            StatusCode::TOO_MANY_REQUESTS,
            "Too many login attempts; try again later",
        ));
    }
    attempt.count += 1;
    Ok(())
}

#[utoipa::path(post, path="/api/auth/login", request_body=LoginRequest, responses((status=200, body=SessionStatus), (status=401), (status=429)))]
pub async fn login(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<LoginRequest>,
) -> Result<Response> {
    let hash = {
        let mut runtime = auth.inner.lock().await;
        auth.refresh(&mut runtime)?;
        if runtime.accounts.is_empty() {
            return Err(Error::bad(
                "Panel authentication is not configured; bootstrap an administrator from the command line",
            ));
        }
        rate_limit(&mut runtime, "global", 60, Duration::from_secs(60))?;
        rate_limit(
            &mut runtime,
            &format!("account:{}", input.username),
            5,
            Duration::from_secs(900),
        )?;
        if validate_username(&input.username).is_err() || input.password.len() > 1024 {
            return Err(error(
                StatusCode::UNAUTHORIZED,
                "Invalid account name or password",
            ));
        }
        runtime
            .accounts
            .iter()
            .find(|account| account.username == input.username)
            .map(|account| account.password_hash.clone())
            .unwrap_or_else(|| auth.dummy_hash.to_string())
    };
    let verified_hash = hash.clone();
    let verified = password_job(&auth, move || {
        verify_password(&input.password, &verified_hash)
    })
    .await?;
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    let account = runtime
        .accounts
        .iter()
        .find(|account| account.username == input.username && account.password_hash == hash)
        .filter(|_| verified)
        .map(public_account)
        .ok_or_else(|| error(StatusCode::UNAUTHORIZED, "Invalid account name or password"))?;
    runtime
        .sessions
        .retain(|_, session| session.expires_at > crate::now());
    if runtime.sessions.len() >= MAX_SESSIONS {
        return Err(error(
            StatusCode::SERVICE_UNAVAILABLE,
            "Session capacity reached; try again later",
        ));
    }
    if let Some(token) = request_token(&headers) {
        runtime.sessions.remove(&token_key(token));
    }
    let mut token = [0u8; 32];
    OsRng.try_fill_bytes(&mut token).map_err(|_| {
        error(
            StatusCode::INTERNAL_SERVER_ERROR,
            "Secure randomness is unavailable",
        )
    })?;
    let token = hex(&token);
    let expires_at = crate::now().saturating_add(SESSION_SECONDS);
    runtime.sessions.insert(
        token_key(&token),
        Session {
            username: account.username.clone(),
            expires_at,
        },
    );
    runtime
        .attempts
        .remove(&format!("account:{}", account.username));
    let mut response = Json(SessionStatus {
        enabled: true,
        user: Some(account),
        expires_at: Some(expires_at),
    })
    .into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth.cookie(&token, SESSION_SECONDS)?);
    Ok(response)
}

#[utoipa::path(post, path="/api/auth/logout", responses((status=204)))]
pub async fn logout(State(auth): State<Auth>, headers: HeaderMap) -> Result<Response> {
    let mut runtime = auth.inner.lock().await;
    if let Some(token) = request_token(&headers) {
        runtime.sessions.remove(&token_key(token));
    }
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth.cookie("", 0)?);
    Ok(response)
}

fn require_admin(runtime: &mut Runtime, headers: &HeaderMap) -> Result<Account> {
    let (account, _) = identify(runtime, headers).ok_or_else(unauthorized)?;
    if account.role != Role::Admin {
        return Err(forbidden());
    }
    Ok(account)
}
fn check_revision(runtime: &Runtime, revision: &str) -> Result<()> {
    if runtime.revision != revision {
        return Err(Error::conflict(
            "Panel accounts changed; reload before saving",
        ));
    }
    Ok(())
}
#[utoipa::path(get, path="/api/auth/accounts", responses((status=200, body=AccountsDocument)))]
pub async fn accounts(
    State(auth): State<Auth>,
    headers: HeaderMap,
) -> Result<Json<AccountsDocument>> {
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    require_admin(&mut runtime, &headers)?;
    Ok(Json(accounts_document(&runtime)))
}
async fn protected_hash(auth: &Auth, password: String) -> Result<String> {
    validate_password(&password)?;
    password_job(auth, move || hash_password(&password)).await?
}
async fn password_job<T: Send + 'static>(
    auth: &Auth,
    job: impl FnOnce() -> T + Send + 'static,
) -> Result<T> {
    let permit = auth
        .password_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| {
            error(
                StatusCode::TOO_MANY_REQUESTS,
                "Password service is busy; try again shortly",
            )
        })?;
    tokio::task::spawn_blocking(move || {
        let _permit = permit;
        job()
    })
    .await
    .map_err(|_| error(StatusCode::INTERNAL_SERVER_ERROR, "Password service failed"))
}

#[utoipa::path(post, path="/api/auth/accounts/create", request_body=CreateAccount, responses((status=200, body=AccountsDocument)))]
pub async fn create_account(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<CreateAccount>,
) -> Result<Json<AccountsDocument>> {
    validate_username(&input.username)?;
    {
        let mut runtime = auth.inner.lock().await;
        auth.refresh(&mut runtime)?;
        require_admin(&mut runtime, &headers)?;
        check_revision(&runtime, &input.revision)?;
    }
    let password_hash = protected_hash(&auth, input.password).await?;
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    require_admin(&mut runtime, &headers)?;
    check_revision(&runtime, &input.revision)?;
    if runtime.accounts.len() >= MAX_ACCOUNTS
        || runtime
            .accounts
            .iter()
            .any(|account| account.username == input.username)
    {
        return Err(Error::conflict(
            "Account already exists or account limit reached",
        ));
    }
    let mut accounts = runtime.accounts.clone();
    accounts.push(StoredAccount {
        username: input.username,
        role: input.role,
        password_hash,
    });
    Ok(Json(auth.write_accounts(&mut runtime, accounts)?))
}

#[utoipa::path(post, path="/api/auth/accounts/update", request_body=UpdateAccount, responses((status=200, body=AccountsDocument)))]
pub async fn update_account(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<UpdateAccount>,
) -> Result<Json<AccountsDocument>> {
    {
        let mut runtime = auth.inner.lock().await;
        auth.refresh(&mut runtime)?;
        require_admin(&mut runtime, &headers)?;
        check_revision(&runtime, &input.revision)?;
    }
    let password_hash = match input.password {
        Some(password) => Some(protected_hash(&auth, password).await?),
        None => None,
    };
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    require_admin(&mut runtime, &headers)?;
    check_revision(&runtime, &input.revision)?;
    let mut accounts = runtime.accounts.clone();
    let account = accounts
        .iter_mut()
        .find(|account| account.username == input.username)
        .ok_or_else(|| Error::bad("Account not found"))?;
    account.role = input.role;
    if let Some(password_hash) = password_hash {
        account.password_hash = password_hash;
    }
    if !accounts.iter().any(|account| account.role == Role::Admin) {
        return Err(Error::conflict("At least one administrator must remain"));
    }
    let response = auth.write_accounts(&mut runtime, accounts)?;
    runtime
        .sessions
        .retain(|_, session| session.username != input.username);
    Ok(Json(response))
}

#[utoipa::path(post, path="/api/auth/accounts/delete", request_body=DeleteAccount, responses((status=200, body=AccountsDocument)))]
pub async fn delete_account(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<DeleteAccount>,
) -> Result<Json<AccountsDocument>> {
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    let current = require_admin(&mut runtime, &headers)?;
    check_revision(&runtime, &input.revision)?;
    if current.username == input.username {
        return Err(Error::bad("You cannot delete your own account"));
    }
    let mut accounts = runtime.accounts.clone();
    accounts.retain(|account| account.username != input.username);
    if accounts.len() == runtime.accounts.len() {
        return Err(Error::bad("Account not found"));
    }
    if !accounts.iter().any(|account| account.role == Role::Admin) {
        return Err(Error::conflict("At least one administrator must remain"));
    }
    let response = auth.write_accounts(&mut runtime, accounts)?;
    runtime
        .sessions
        .retain(|_, session| session.username != input.username);
    Ok(Json(response))
}

#[utoipa::path(post, path="/api/auth/password", request_body=PasswordChange, responses((status=204)))]
pub async fn change_password(
    State(auth): State<Auth>,
    headers: HeaderMap,
    Json(input): Json<PasswordChange>,
) -> Result<Response> {
    let (username, hash, revision) = {
        let mut runtime = auth.inner.lock().await;
        auth.refresh(&mut runtime)?;
        let (account, _) = identify(&mut runtime, &headers).ok_or_else(unauthorized)?;
        rate_limit(
            &mut runtime,
            &format!("password:{}", account.username),
            5,
            Duration::from_secs(900),
        )?;
        let stored = runtime
            .accounts
            .iter()
            .find(|stored| stored.username == account.username)
            .ok_or_else(unauthorized)?;
        (
            account.username,
            stored.password_hash.clone(),
            runtime.revision.clone(),
        )
    };
    if input.current_password.len() > 1024 {
        return Err(Error::bad("Current password is invalid"));
    }
    let verified_hash = hash.clone();
    let verified = password_job(&auth, move || {
        verify_password(&input.current_password, &verified_hash)
    })
    .await?;
    if !verified {
        return Err(Error::bad("Current password is invalid"));
    }
    let password_hash = protected_hash(&auth, input.password).await?;
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    identify(&mut runtime, &headers).ok_or_else(unauthorized)?;
    check_revision(&runtime, &revision)?;
    let mut accounts = runtime.accounts.clone();
    let account = accounts
        .iter_mut()
        .find(|account| account.username == username && account.password_hash == hash)
        .ok_or_else(unauthorized)?;
    account.password_hash = password_hash;
    auth.write_accounts(&mut runtime, accounts)?;
    runtime
        .sessions
        .retain(|_, session| session.username != username);
    let mut response = StatusCode::NO_CONTENT.into_response();
    response
        .headers_mut()
        .insert(header::SET_COOKIE, auth.cookie("", 0)?);
    Ok(response)
}

pub fn bootstrap_admin(directory: &Path, username: &str, password: &str) -> Result<()> {
    storage::ensure_directory(directory)?;
    validate_username(username)?;
    let path = directory.join("panel-accounts.json");
    let (revision, accounts) = read_accounts(&path)?;
    if revision != "missing" || !accounts.is_empty() {
        return Err(Error::conflict(
            "Panel accounts already exist; use account management",
        ));
    }
    let accounts = StoredAccounts {
        accounts: vec![StoredAccount {
            username: username.into(),
            role: Role::Admin,
            password_hash: hash_password(password)?,
        }],
    };
    storage::atomic_write(&path, &serde_json::to_vec_pretty(&accounts)?, false)
}

#[derive(utoipa::OpenApi)]
#[openapi(
    paths(
        status,
        login,
        logout,
        accounts,
        create_account,
        update_account,
        delete_account,
        change_password
    ),
    components(schemas(
        Role,
        Account,
        SessionStatus,
        AccountsDocument,
        LoginRequest,
        CreateAccount,
        UpdateAccount,
        DeleteAccount,
        PasswordChange
    ))
)]
pub struct Api;

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn canceled_password_requests_hold_capacity_until_blocking_work_finishes()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let auth = Auth::load(directory.path(), None).map_err(|error| error.message)?;
        let worker_auth = auth.clone();
        let (started, wait_started) = tokio::sync::oneshot::channel();
        let (release, wait_release) = std::sync::mpsc::channel();
        let worker = tokio::spawn(async move {
            password_job(&worker_auth, move || {
                let announced = started.send(()).is_ok();
                announced && wait_release.recv().is_ok()
            })
            .await
        });
        wait_started.await?;
        worker.abort();
        assert!(worker.await.is_err());
        let second_slot = auth.password_slots.clone().try_acquire_owned()?;
        assert!(auth.password_slots.clone().try_acquire_owned().is_err());
        drop(second_slot);
        release.send(())?;
        let _slots =
            tokio::time::timeout(Duration::from_secs(2), auth.password_slots.acquire_many(2))
                .await??;
        Ok(())
    }

    #[test]
    fn expired_sessions_are_removed_before_authorization() {
        let token = "a".repeat(64);
        let mut runtime = Runtime {
            revision: "fixture".into(),
            accounts: vec![StoredAccount {
                username: "owner".into(),
                role: Role::Admin,
                password_hash: String::new(),
            }],
            sessions: HashMap::from([(
                token_key(&token),
                Session {
                    username: "owner".into(),
                    expires_at: crate::now().saturating_sub(1),
                },
            )]),
            attempts: HashMap::new(),
        };
        let mut headers = HeaderMap::new();
        let cookie = format!("{COOKIE}={token}").parse();
        assert!(cookie.is_ok());
        if let Ok(cookie) = cookie {
            headers.insert(header::COOKIE, cookie);
        }
        assert!(identify(&mut runtime, &headers).is_none());
        assert!(runtime.sessions.is_empty());
    }
}
