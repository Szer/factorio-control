use crate::hex;
use crate::{Error, Result, storage};
use argon2::{Algorithm, Argon2, Params, PasswordHash, PasswordHasher, PasswordVerifier, Version};
use axum::{
    Extension, Json, Router,
    body::Body,
    extract::{ConnectInfo, DefaultBodyLimit, State},
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
    net::{IpAddr, SocketAddr},
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
const LOGIN_ATTEMPTS: u32 = 5;
const LOGIN_WINDOW: Duration = Duration::from_secs(60);
const MAX_TRUSTED_PROXIES: usize = 64;
const MAX_FORWARDED_HOPS: usize = 16;
pub const BOOTSTRAP_FILE: &str = "bootstrap-admin.json";
const INITIALIZED_FILE: &str = ".panel-auth-initialized";

#[derive(Clone, Copy, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
#[serde(rename_all = "lowercase")]
pub enum Role {
    Admin,
    Operator,
    Viewer,
}

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize, ToSchema)]
pub struct Account {
    pub account_id: String,
    pub username: String,
    pub role: Role,
    pub must_change_password: bool,
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct SessionStatus {
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
    #[serde(default)]
    account_id: String,
    username: String,
    role: Role,
    password_hash: String,
    #[serde(default)]
    must_change_password: bool,
}
#[derive(Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct BootstrapCredentials {
    username: String,
    password: String,
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
#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
struct ClientIp(IpAddr);

struct Attempts {
    count: u32,
    started: Instant,
}
struct Runtime {
    revision: String,
    accounts: Vec<StoredAccount>,
    sessions: HashMap<String, Session>,
    attempts: HashMap<String, Attempts>,
    login_attempts: HashMap<ClientIp, Attempts>,
}

#[derive(Clone)]
pub struct Auth {
    path: PathBuf,
    public_origin: Option<url::Url>,
    trusted_proxies: Vec<IpAddr>,
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
        account_id: account.account_id.clone(),
        username: account.username.clone(),
        role: account.role,
        must_change_password: account.must_change_password,
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
    Argon2::new(
        Algorithm::Argon2id,
        Version::V0x13,
        Params::new(19456, 2, 1, Some(32))
            .map_err(|_| Error::bad("Invalid password hashing policy"))?,
    )
    .hash_password_with_salt(password.as_bytes(), &bytes)
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
    let mut identities = std::collections::HashSet::new();
    for account in &accounts {
        validate_username(&account.username)?;
        if !names.insert(account.username.clone())
            || (!account.account_id.is_empty()
                && (account.account_id.len() != 64
                    || !account
                        .account_id
                        .bytes()
                        .all(|byte| byte.is_ascii_hexdigit())
                    || !identities.insert(account.account_id.clone())))
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

fn new_account_id() -> Result<String> {
    let mut bytes = [0u8; 32];
    OsRng
        .try_fill_bytes(&mut bytes)
        .map_err(|_| Error::bad("Secure randomness is unavailable"))?;
    Ok(hex(&bytes))
}

fn migrate_account_ids(path: &Path) -> Result<(String, Vec<StoredAccount>)> {
    let (revision, mut accounts) = read_accounts(path)?;
    if accounts
        .iter()
        .all(|account| !account.account_id.is_empty())
    {
        return Ok((revision, accounts));
    }
    for account in &mut accounts {
        if account.account_id.is_empty() {
            account.account_id = new_account_id()?;
        }
    }
    if storage::read_document(path)?.0 != revision {
        return Err(Error::conflict(
            "Panel accounts changed during startup; restart the manager",
        ));
    }
    storage::atomic_write(
        path,
        &serde_json::to_vec_pretty(&StoredAccounts { accounts })?,
        true,
    )?;
    read_accounts(path)
}

fn mark_initialized(directory: &Path) -> Result<()> {
    let marker = directory.join(INITIALIZED_FILE);
    if storage::read_document(&marker)?.1.is_none() {
        storage::atomic_write(&marker, b"Panel authentication is required.\n", false)?;
    }
    Ok(())
}

fn remove_bootstrap_file(directory: &Path, accounts: &[StoredAccount]) -> Result<()> {
    if accounts.iter().any(|account| account.must_change_password) {
        return Ok(());
    }
    let path = directory.join(BOOTSTRAP_FILE);
    storage::ensure_regular(&path)?;
    match std::fs::remove_file(path) {
        Ok(()) => std::fs::File::open(directory)?.sync_all()?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error.into()),
    }
    Ok(())
}

fn initialize_accounts(directory: &Path) -> Result<()> {
    let path = directory.join("panel-accounts.json");
    if read_accounts(&path)?.0 != "missing" {
        mark_initialized(directory)?;
        return Ok(());
    }
    let credentials_path = directory.join(BOOTSTRAP_FILE);
    let existing = storage::read_document(&credentials_path)?.1;
    if existing.is_none()
        && storage::read_document(&directory.join(INITIALIZED_FILE))?
            .1
            .is_some()
    {
        return Err(Error::bad(
            "Panel account file is missing; restore it from backup",
        ));
    }
    let credentials = if let Some(bytes) = existing {
        let credentials: BootstrapCredentials = serde_json::from_slice(&bytes)
            .map_err(|_| Error::bad("Invalid bootstrap credential file"))?;
        use std::os::unix::fs::PermissionsExt;
        if credentials.username != "admin"
            || credentials.password.len() != 64
            || !credentials
                .password
                .bytes()
                .all(|byte| byte.is_ascii_hexdigit())
            || std::fs::metadata(&credentials_path)?.permissions().mode() & 0o077 != 0
        {
            return Err(Error::bad("Invalid or insecure bootstrap credential file"));
        }
        credentials
    } else {
        let mut bytes = [0u8; 32];
        OsRng
            .try_fill_bytes(&mut bytes)
            .map_err(|_| Error::bad("Secure randomness is unavailable"))?;
        let credentials = BootstrapCredentials {
            username: "admin".into(),
            password: hex(&bytes),
        };
        storage::atomic_write(
            &credentials_path,
            &serde_json::to_vec_pretty(&credentials)?,
            false,
        )?;
        credentials
    };
    mark_initialized(directory)?;
    let accounts = StoredAccounts {
        accounts: vec![StoredAccount {
            account_id: new_account_id()?,
            username: credentials.username,
            role: Role::Admin,
            password_hash: hash_password(&credentials.password)?,
            must_change_password: true,
        }],
    };
    storage::atomic_write(&path, &serde_json::to_vec_pretty(&accounts)?, false)?;
    eprintln!(
        "Temporary administrator created. Read {} and change its password at first sign-in.",
        credentials_path.display()
    );
    Ok(())
}

impl Auth {
    pub fn load(directory: &Path, public_url: Option<&str>) -> Result<Self> {
        Self::load_with_trusted_proxies(directory, public_url, None)
    }

    pub fn load_with_trusted_proxies(
        directory: &Path,
        public_url: Option<&str>,
        trusted_proxies: Option<&str>,
    ) -> Result<Self> {
        let trusted_proxies = parse_trusted_proxies(trusted_proxies)?;
        storage::ensure_directory(directory)?;
        let public_origin = public_url.filter(|value| !value.trim().is_empty()).map(|value| {
            let url = url::Url::parse(value).map_err(|_| Error::bad("FC_PUBLIC_URL must be an HTTP or HTTPS origin"))?;
            if !matches!(url.scheme(), "http" | "https") || url.host_str().is_none() || !url.username().is_empty() || url.password().is_some() || url.path() != "/" || url.query().is_some() || url.fragment().is_some() {
                return Err(Error::bad("FC_PUBLIC_URL must be an HTTP or HTTPS origin without credentials, path, query or fragment"));
            }
            Ok(url)
        }).transpose()?;
        initialize_accounts(directory)?;
        let path = directory.join("panel-accounts.json");
        let (revision, accounts) = migrate_account_ids(&path)?;
        remove_bootstrap_file(directory, &accounts)?;
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
            trusted_proxies,
            inner: Arc::new(Mutex::new(Runtime {
                revision,
                accounts,
                sessions: HashMap::new(),
                attempts: HashMap::new(),
                login_attempts: HashMap::new(),
            })),
            password_slots: Arc::new(Semaphore::new(2)),
            dummy_hash,
        })
    }

    pub async fn validate_bind(&self, bind: SocketAddr) -> Result<()> {
        if !bind.ip().is_loopback() {
            let mut runtime = self.inner.lock().await;
            self.refresh(&mut runtime)?;
            if runtime.accounts.is_empty() {
                return Err(Error::bad("Nonloopback FC_BIND requires panel accounts"));
            }
        }
        Ok(())
    }

    fn refresh(&self, runtime: &mut Runtime) -> Result<()> {
        let (revision, accounts) = read_accounts(&self.path)?;
        if accounts.iter().any(|account| account.account_id.is_empty()) {
            return Err(error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Panel accounts require identity migration; restart the manager",
            ));
        }
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
        if let Some(directory) = self.path.parent() {
            remove_bootstrap_file(directory, &runtime.accounts)?;
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
            if self
                .public_origin
                .as_ref()
                .is_some_and(|origin| origin.scheme() == "https")
            {
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

    fn client_ip(&self, peer: Option<SocketAddr>, headers: &HeaderMap) -> Result<ClientIp> {
        let peer = peer.ok_or_else(|| {
            error(
                StatusCode::SERVICE_UNAVAILABLE,
                "Login connection information is unavailable",
            )
        })?;
        let peer = normalized_ip(peer.ip());
        if !self.trusted_proxies.contains(&peer) {
            return Ok(ClientIp(peer));
        }
        let invalid = || Error::bad("Trusted proxy must send one valid X-Forwarded-For chain");
        let mut values = headers.get_all("x-forwarded-for").iter();
        let value = values.next().ok_or_else(invalid)?;
        if values.next().is_some() || value.len() > MAX_FORWARDED_HOPS * 46 {
            return Err(invalid());
        }
        let chain = value.to_str().map_err(|_| invalid())?;
        let mut addresses = Vec::new();
        for part in chain.split(',') {
            if addresses.len() >= MAX_FORWARDED_HOPS {
                return Err(invalid());
            }
            let address = part.trim().parse::<IpAddr>().map_err(|_| invalid())?;
            addresses.push(normalized_ip(address));
        }
        // Only proxies nearest the verified transport peer may extend the trust chain.
        addresses
            .iter()
            .rev()
            .copied()
            .find(|address| !self.trusted_proxies.contains(address))
            .or_else(|| addresses.first().copied())
            .map(ClientIp)
            .ok_or_else(invalid)
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
                let upload = path.strip_prefix("/api/saves/").is_some_and(|name| {
                    name.to_ascii_lowercase().ends_with(".zip") && !name.contains('/')
                }) || path == "/api/mods/upload";
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

fn normalized_ip(ip: IpAddr) -> IpAddr {
    match ip {
        IpAddr::V6(ip) => ip
            .to_ipv4_mapped()
            .map(IpAddr::V4)
            .unwrap_or(IpAddr::V6(ip)),
        ip => ip,
    }
}

fn parse_trusted_proxies(value: Option<&str>) -> Result<Vec<IpAddr>> {
    let mut proxies = Vec::new();
    if let Some(value) = value {
        for value in value.split(',') {
            if proxies.len() >= MAX_TRUSTED_PROXIES {
                return Err(Error::bad(
                    "FC_TRUSTED_PROXIES allows at most 64 exact IP addresses",
                ));
            }
            let ip = value.trim().parse::<IpAddr>().map_err(|_| {
                Error::bad("FC_TRUSTED_PROXIES must contain comma-separated exact IP addresses")
            })?;
            proxies.push(normalized_ip(ip));
        }
    }
    Ok(proxies)
}

fn accounts_document(runtime: &Runtime) -> AccountsDocument {
    AccountsDocument {
        revision: runtime.revision.clone(),
        accounts: runtime.accounts.iter().map(public_account).collect(),
    }
}
fn token_key(token: &str) -> String {
    hex(&Sha256::digest(token.as_bytes()))
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
        "/api/runtime-paths",
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
            if !matches!(
                (request.method(), request.uri().path()),
                (&Method::GET, "/api/auth/status") | (&Method::POST, "/api/auth/login")
            ) {
                let (account, _) =
                    identify(&mut runtime, request.headers()).ok_or_else(unauthorized)?;
                if account.must_change_password
                    && !matches!(
                        (request.method(), request.uri().path()),
                        (&Method::POST, "/api/auth/password" | "/api/auth/logout")
                    )
                {
                    return Err(error(
                        StatusCode::FORBIDDEN,
                        "Change the temporary password before using the panel",
                    ));
                }
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

#[utoipa::path(get, path="/api/auth/status", operation_id="auth_status", responses((status=200, body=SessionStatus)))]
pub async fn status(State(auth): State<Auth>, headers: HeaderMap) -> Result<Json<SessionStatus>> {
    let mut runtime = auth.inner.lock().await;
    auth.refresh(&mut runtime)?;
    let session = identify(&mut runtime, &headers);
    Ok(Json(SessionStatus {
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

fn limit_login(runtime: &mut Runtime, client: ClientIp) -> Result<()> {
    runtime
        .login_attempts
        .retain(|_, attempt| attempt.started.elapsed() < LOGIN_WINDOW);
    if !runtime.login_attempts.contains_key(&client)
        && runtime.login_attempts.len() >= MAX_RATE_KEYS
    {
        // A full cache must not become a lockout shared by every new client.
        if let Some(oldest) = runtime
            .login_attempts
            .iter()
            .min_by_key(|(_, attempt)| attempt.started)
            .map(|(ip, _)| *ip)
        {
            runtime.login_attempts.remove(&oldest);
        }
    }
    let attempt = runtime.login_attempts.entry(client).or_insert(Attempts {
        count: 0,
        started: Instant::now(),
    });
    if attempt.count >= LOGIN_ATTEMPTS {
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
    peer: Option<Extension<ConnectInfo<SocketAddr>>>,
    headers: HeaderMap,
    Json(input): Json<LoginRequest>,
) -> Result<Response> {
    let client = auth.client_ip(peer.map(|Extension(ConnectInfo(peer))| peer), &headers)?;
    let hash = {
        let mut runtime = auth.inner.lock().await;
        auth.refresh(&mut runtime)?;
        limit_login(&mut runtime, client)?;
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
    let mut response = Json(SessionStatus {
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
    if account.role != Role::Admin || account.must_change_password {
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
        account_id: new_account_id()?,
        username: input.username,
        role: input.role,
        password_hash,
        must_change_password: false,
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
        account.must_change_password = false;
    }
    if !accounts.iter().any(|account| account.role == Role::Admin) {
        return Err(Error::conflict("At least one administrator must remain"));
    }
    let response = auth.write_accounts(&mut runtime, accounts)?;
    runtime
        .sessions
        .retain(|_, session| session.username != input.username);
    remove_bootstrap_file(
        auth.path
            .parent()
            .ok_or_else(|| Error::bad("Missing account directory"))?,
        &runtime.accounts,
    )?;
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
    remove_bootstrap_file(
        auth.path
            .parent()
            .ok_or_else(|| Error::bad("Missing account directory"))?,
        &runtime.accounts,
    )?;
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
    if input.current_password == input.password {
        return Err(Error::bad("Choose a different password"));
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
    account.must_change_password = false;
    auth.write_accounts(&mut runtime, accounts)?;
    runtime
        .sessions
        .retain(|_, session| session.username != username);
    remove_bootstrap_file(
        auth.path
            .parent()
            .ok_or_else(|| Error::bad("Missing account directory"))?,
        &runtime.accounts,
    )?;
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
            account_id: new_account_id()?,
            username: username.into(),
            role: Role::Admin,
            password_hash: hash_password(password)?,
            must_change_password: false,
        }],
    };
    storage::atomic_write(&path, &serde_json::to_vec_pretty(&accounts)?, false)?;
    mark_initialized(directory)
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

    fn runtime() -> Runtime {
        Runtime {
            revision: String::new(),
            accounts: Vec::new(),
            sessions: HashMap::new(),
            attempts: HashMap::new(),
            login_attempts: HashMap::new(),
        }
    }

    #[test]
    fn login_limit_is_per_client_bounded_and_expires()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let mut runtime = runtime();
        let attacker = ClientIp("192.0.2.1".parse()?);
        let legitimate = ClientIp("192.0.2.2".parse()?);
        for _ in 0..LOGIN_ATTEMPTS {
            limit_login(&mut runtime, attacker).map_err(|error| error.message)?;
        }
        assert_eq!(
            limit_login(&mut runtime, attacker)
                .err()
                .map(|error| error.status),
            Some(StatusCode::TOO_MANY_REQUESTS)
        );
        limit_login(&mut runtime, legitimate).map_err(|error| error.message)?;
        if let Some(attempt) = runtime.login_attempts.get_mut(&attacker) {
            attempt.started = Instant::now() - LOGIN_WINDOW;
        }
        limit_login(&mut runtime, attacker).map_err(|error| error.message)?;
        assert_eq!(
            runtime
                .login_attempts
                .get(&attacker)
                .map(|attempt| attempt.count),
            Some(1)
        );
        for address in 0..MAX_RATE_KEYS + 100 {
            let address = IpAddr::V6(std::net::Ipv6Addr::from(
                address as u128 + 0x20010000000000000000000000000000,
            ));
            limit_login(&mut runtime, ClientIp(address)).map_err(|error| error.message)?;
            assert!(runtime.login_attempts.len() <= MAX_RATE_KEYS);
        }
        limit_login(&mut runtime, legitimate).map_err(|error| error.message)?;
        assert_eq!(runtime.login_attempts.len(), MAX_RATE_KEYS);
        Ok(())
    }

    #[test]
    fn trusted_proxy_configuration_is_explicit_and_validated()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        assert!(
            parse_trusted_proxies(None)
                .map_err(|error| error.message)?
                .is_empty()
        );
        let proxies = parse_trusted_proxies(Some(" 192.0.2.1, ::ffff:192.0.2.2, ::1 "))
            .map_err(|error| error.message)?;
        assert_eq!(
            proxies,
            vec![
                "192.0.2.1".parse::<IpAddr>()?,
                "192.0.2.2".parse()?,
                "::1".parse()?
            ]
        );
        for invalid in [
            "",
            " ",
            "proxy.example",
            "192.0.2.1/24",
            "192.0.2.1,",
            "0.0.0.0:8080",
        ] {
            assert!(parse_trusted_proxies(Some(invalid)).is_err());
        }
        let too_many = vec!["192.0.2.1"; MAX_TRUSTED_PROXIES + 1].join(",");
        assert!(parse_trusted_proxies(Some(&too_many)).is_err());
        Ok(())
    }

    #[tokio::test]
    async fn forwarded_client_identity_uses_only_verified_proxy_chain()
    -> std::result::Result<(), Box<dyn std::error::Error>> {
        let directory = tempfile::tempdir()?;
        let auth =
            Auth::load_with_trusted_proxies(directory.path(), None, Some("192.0.2.10,192.0.2.11"))
                .map_err(|error| error.message)?;
        let mut headers = HeaderMap::new();
        let direct = Some("198.51.100.1:1234".parse()?);
        let proxy = Some("192.0.2.10:1234".parse()?);
        assert!(auth.client_ip(None, &headers).is_err());
        assert!(auth.client_ip(proxy, &headers).is_err());
        headers.insert("x-forwarded-for", "not an address".parse()?);
        assert_eq!(
            auth.client_ip(direct, &headers)
                .map_err(|error| error.message)?,
            ClientIp("198.51.100.1".parse::<IpAddr>()?)
        );
        assert!(auth.client_ip(proxy, &headers).is_err());
        for chain in [
            "203.0.113.2, 198.51.100.1, 192.0.2.11",
            "198.51.100.1",
            "::ffff:198.51.100.1, ::ffff:192.0.2.11",
        ] {
            headers.insert("x-forwarded-for", chain.parse()?);
            assert_eq!(
                auth.client_ip(proxy, &headers)
                    .map_err(|error| error.message)?,
                ClientIp("198.51.100.1".parse::<IpAddr>()?)
            );
        }
        assert_eq!(
            auth.client_ip(Some("[::ffff:198.51.100.1]:1234".parse()?), &headers)
                .map_err(|error| error.message)?,
            ClientIp("198.51.100.1".parse::<IpAddr>()?)
        );
        for malformed in [
            "",
            ",",
            "198.51.100.1,",
            "unknown",
            "198.51.100.1:1234",
            "[2001:db8::1]",
        ] {
            headers.insert("x-forwarded-for", malformed.parse()?);
            assert!(auth.client_ip(proxy, &headers).is_err());
        }
        headers.insert(
            "x-forwarded-for",
            vec!["198.51.100.1"; MAX_FORWARDED_HOPS + 1]
                .join(",")
                .parse()?,
        );
        assert!(auth.client_ip(proxy, &headers).is_err());
        headers.insert(
            "x-forwarded-for",
            "x".repeat(MAX_FORWARDED_HOPS * 46 + 1).parse()?,
        );
        assert!(auth.client_ip(proxy, &headers).is_err());
        headers.insert("x-forwarded-for", "198.51.100.1".parse()?);
        headers.append("x-forwarded-for", "203.0.113.1".parse()?);
        assert!(auth.client_ip(proxy, &headers).is_err());
        Ok(())
    }

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
                account_id: "a".repeat(64),
                username: "owner".into(),
                role: Role::Admin,
                password_hash: String::new(),
                must_change_password: false,
            }],
            sessions: HashMap::from([(
                token_key(&token),
                Session {
                    username: "owner".into(),
                    expires_at: crate::now().saturating_sub(1),
                },
            )]),
            attempts: HashMap::new(),
            login_attempts: HashMap::new(),
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
