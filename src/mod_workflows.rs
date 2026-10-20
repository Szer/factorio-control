use super::*;
use axum::extract::Query;
use std::io::Write;

const MAX_BATCH_BYTES: u64 = 1024 * 1024 * 1024;
static OPERATIONS: tokio::sync::Mutex<()> = tokio::sync::Mutex::const_new(());
static PROGRESS: std::sync::Mutex<Option<(PathBuf, ModOperationProgress)>> =
    std::sync::Mutex::new(None);
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModOperationProgress {
    pub active: bool,
    pub stage: String,
    pub completed: usize,
    pub total: usize,
    pub current: Option<String>,
}
struct ProgressGuard {
    directory: PathBuf,
}
impl ProgressGuard {
    fn new(app: &App, total: usize) -> Result<Self> {
        let guard = Self {
            directory: app.directory.clone(),
        };
        let mut progress = PROGRESS
            .lock()
            .map_err(|_| Error::bad("Mod progress is unavailable"))?;
        *progress = Some((
            guard.directory.clone(),
            ModOperationProgress {
                active: true,
                stage: "Preparing".into(),
                completed: 0,
                total,
                current: None,
            },
        ));
        Ok(guard)
    }
    fn update(&self, stage: &str, completed: usize, current: Option<&str>) -> Result<()> {
        let mut progress = PROGRESS
            .lock()
            .map_err(|_| Error::bad("Mod progress is unavailable"))?;
        if let Some((directory, value)) = progress.as_mut()
            && *directory == self.directory
        {
            value.stage = stage.into();
            value.completed = completed;
            value.current = current.map(str::to_owned);
        }
        Ok(())
    }
    fn complete(&self) -> Result<()> {
        let mut progress = PROGRESS
            .lock()
            .map_err(|_| Error::bad("Mod progress is unavailable"))?;
        if let Some((directory, value)) = progress.as_mut()
            && *directory == self.directory
        {
            value.active = false;
            value.stage = "Complete".into();
            value.completed = value.total;
            value.current = None;
        }
        Ok(())
    }
}
impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if let Ok(mut progress) = PROGRESS.lock()
            && let Some((directory, value)) = progress.as_mut()
            && *directory == self.directory
            && value.active
        {
            value.active = false;
            value.stage = "Failed or interrupted".into();
            value.current = None;
        }
    }
}

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct RevisionRequest {
    pub revision: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PortalInstallRequest {
    pub revision: String,
    pub name: String,
    pub version: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalSearchEntry {
    pub name: String,
    pub title: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalRelease {
    pub version: String,
    pub factorio_version: String,
    pub released_at: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PortalReleases {
    pub name: String,
    pub releases: Vec<PortalRelease>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BulkChange {
    pub name: String,
    pub current_version: String,
    pub version: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct BulkPlan {
    pub revision: String,
    pub changes: Vec<BulkChange>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListEntry {
    pub name: String,
    pub enabled: bool,
    pub version: Option<String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ListDocument {
    pub revision: String,
    pub mods: Vec<ListEntry>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct PackRequest {
    pub revision: String,
    pub name: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct PackSummary {
    pub name: String,
    pub mods: usize,
}
#[derive(Clone, Deserialize, Serialize)]
struct PackArchive {
    file_name: String,
    sha256: String,
}
#[derive(Clone, Deserialize, Serialize)]
struct Pack {
    list: ModList,
    archives: Vec<PackArchive>,
    settings: Option<Vec<u8>>,
}
#[derive(Deserialize)]
pub struct SearchQuery {
    pub q: String,
}
#[derive(Deserialize)]
pub struct RevisionQuery {
    pub revision: String,
}
#[derive(Deserialize, Serialize)]
struct Journal {
    list: Option<Vec<u8>>,
    settings: Option<Vec<u8>>,
    added: Vec<String>,
    moved: Vec<String>,
}

fn subdirectory(parent: &FsPath, name: &str) -> Result<PathBuf> {
    let path = parent.join(name);
    if !path.exists() {
        fs::create_dir(&path)?;
    }
    storage::ensure_directory(&path)?;
    Ok(path)
}
fn pack_path(app: &App, name: &str) -> Result<PathBuf> {
    if !valid_name(name) || name.len() > 64 {
        return Err(Error::bad(
            "Pack names use 1–64 letters, numbers, hyphens or underscores",
        ));
    }
    Ok(subdirectory(&app.directory, "mod-packs")?.join(format!("{name}.json")))
}
fn journal_path(app: &App) -> PathBuf {
    app.directory.join("mod-transaction.json")
}
pub fn ensure_consistent(app: &App) -> Result<()> {
    if journal_path(app).exists() {
        return Err(Error::conflict(
            "An interrupted mod transaction needs recovery. Open mod management and retry the operation before starting",
        ));
    }
    Ok(())
}
fn restore_file(path: &FsPath, bytes: Option<&[u8]>) -> Result<()> {
    match bytes {
        Some(bytes) => storage::atomic_write(path, bytes, true),
        None => {
            storage::ensure_regular(path)?;
            if path.exists() {
                fs::remove_file(path)?;
            }
            Ok(())
        }
    }
}
fn recover(app: &App) -> Result<()> {
    let path = journal_path(app);
    storage::ensure_regular(&path)?;
    if !path.exists() {
        return Ok(());
    }
    let mut bytes = Vec::new();
    fs::File::open(&path)?
        .take(128 * 1024 * 1024 + 1)
        .read_to_end(&mut bytes)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(Error::bad("Mod transaction exceeds recovery limit"));
    }
    let journal: Journal = serde_json::from_slice(&bytes)?;
    let mods = directory(app)?;
    restore_file(&mods.join("mod-settings.dat"), journal.settings.as_deref())?;
    restore_file(&mods.join("mod-list.json"), journal.list.as_deref())?;
    for name in &journal.added {
        if !safe_archive_name(name) {
            return Err(Error::bad("Invalid transaction archive"));
        }
        let path = mods.join(name);
        storage::ensure_regular(&path)?;
        if path.exists() {
            fs::remove_file(path)?;
        }
    }
    if !journal.moved.is_empty() {
        storage::ensure_directory(&mods.join(".archive"))?;
    }
    for name in &journal.moved {
        if !safe_archive_name(name) {
            return Err(Error::bad("Invalid transaction archive"));
        }
        let source = mods.join(".archive").join(name);
        storage::ensure_regular(&source)?;
        if source.exists() && !mods.join(name).exists() {
            storage::ensure_regular(&mods.join(name))?;
            fs::rename(source, mods.join(name))?;
        }
    }
    fs::File::open(&mods)?.sync_all()?;
    if !journal.moved.is_empty() {
        fs::File::open(mods.join(".archive"))?.sync_all()?;
    }
    fs::remove_file(path)?;
    fs::File::open(&app.directory)?.sync_all()?;
    Ok(())
}
fn begin_transaction(app: &App, added: Vec<String>, moved: Vec<String>) -> Result<()> {
    let path = directory(app)?;
    let (_, list) = storage::read_document(&path.join("mod-list.json"))?;
    let settings = mod_settings::read_bytes(&path.join("mod-settings.dat"))?;
    let bytes = serde_json::to_vec(&Journal {
        list,
        settings,
        added,
        moved,
    })?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(Error::bad("Mod transaction backup exceeds limit"));
    }
    storage::atomic_write(&journal_path(app), &bytes, false)
}
fn write_activation(app: &App, list: &ModList, settings: Option<Option<&[u8]>>) -> Result<()> {
    let path = directory(app)?;
    if !journal_path(app).exists() {
        begin_transaction(app, Vec::new(), Vec::new())?;
    }
    let result = (|| {
        if let Some(settings) = settings {
            restore_file(&path.join("mod-settings.dat"), settings)?;
        }
        storage::atomic_write(
            &path.join("mod-list.json"),
            &serde_json::to_vec_pretty(list)?,
            true,
        )?;
        Ok(())
    })();
    if let Err(error) = result {
        recover(app)?;
        return Err(error);
    }
    fs::remove_file(journal_path(app))?;
    fs::File::open(&app.directory)?.sync_all()?;
    Ok(())
}
fn installed(info: ModInfo, enabled: bool, size_bytes: u64) -> InstalledMod {
    InstalledMod {
        file_name: format!("{}_{}.zip", info.name, info.version),
        name: info.name,
        version: info.version,
        title: info.title,
        author: info.author,
        description: info.description,
        factorio_version: info.factorio_version,
        dependencies: info.dependencies,
        enabled,
        size_bytes,
    }
}
fn validate_selection(
    app: &App,
    list: &ModList,
    candidates: &[InstalledMod],
    engine: &str,
) -> Result<()> {
    let bundled = bundled_mods(app)?;
    if list.mods.len() > MAX_MODS {
        return Err(Error::bad("Too many mods"));
    }
    let mut names = HashSet::new();
    for entry in &list.mods {
        if !valid_name(&entry.name) || !names.insert(&entry.name) {
            return Err(Error::bad("Invalid or duplicate mod-list entries"));
        }
        if let Some(value) = &entry.version {
            version(value)?;
        }
        if (entry.enabled || entry.version.is_some())
            && !protected(&entry.name)
            && !bundled.contains_key(&entry.name)
            && !candidates.iter().any(|item| {
                item.name == entry.name
                    && entry
                        .version
                        .as_ref()
                        .is_none_or(|value| value == &item.version)
            })
        {
            return Err(Error::bad(&format!(
                "{} is missing its selected ZIP; install it before importing this list",
                entry.name
            )));
        }
    }
    if list
        .mods
        .iter()
        .any(|item| item.name == "base" && !item.enabled)
    {
        return Err(Error::bad("Factorio base must remain enabled"));
    }
    check_enabled_compatibility(candidates, engine)?;
    check_dependencies(candidates, list, Some(engine), &bundled)
}
fn archive_info(path: &FsPath) -> Result<(ModInfo, u64)> {
    storage::ensure_regular(path)?;
    let size = fs::metadata(path)?.len();
    if size > MAX_ARCHIVE as u64 {
        return Err(Error::bad("Mod archive exceeds 64 MiB"));
    }
    let bytes = fs::read(path)?;
    Ok((validate_archive(&bytes)?, size))
}
fn selection(app: &App, requested: &ModList) -> Result<(ModList, Vec<InstalledMod>)> {
    let path = directory(app)?;
    let document = list(app)?;
    let mut list = requested.clone();
    let mut candidates = Vec::new();
    for entry in &list.mods {
        if protected(&entry.name) {
            continue;
        }
        let selected = match &entry.version {
            Some(value) => {
                version(value)?;
                if !valid_name(&entry.name) {
                    return Err(Error::bad("Invalid mod name"));
                }
                let target = path.join(format!("{}_{}.zip", entry.name, value));
                if !target.exists() {
                    None
                } else {
                    let (info, size) = archive_info(&target)?;
                    if info.name != entry.name || info.version != *value {
                        return Err(Error::bad("Mod ZIP identity does not match selection"));
                    }
                    Some(installed(info, entry.enabled, size))
                }
            }
            None => document
                .mods
                .iter()
                .find(|item| item.name == entry.name)
                .cloned()
                .map(|mut item| {
                    item.enabled = entry.enabled;
                    item
                }),
        };
        if let Some(item) = selected {
            candidates.push(item);
        }
    }
    // Unlisted archives remain stored but cannot become implicitly enabled.
    for item in &document.mods {
        if !list.mods.iter().any(|entry| entry.name == item.name) {
            set_entry(&mut list, &item.name, false, &item.version);
            let mut item = item.clone();
            item.enabled = false;
            candidates.push(item);
        }
    }
    for item in &candidates {
        set_entry(&mut list, &item.name, item.enabled, &item.version);
    }
    Ok((list, candidates))
}
fn verify_download(name: &str, release: &Release, bytes: &[u8]) -> Result<ModInfo> {
    if release.sha1.len() != 40
        || release.sha1.to_ascii_lowercase() != format!("{:x}", sha1::Sha1::digest(bytes))
    {
        return Err(Error::bad(
            "Downloaded mod checksum does not match the portal",
        ));
    }
    let info = validate_archive(bytes)?;
    if info.name != name || info.version != release.version {
        return Err(Error::bad(
            "Downloaded mod identity does not match the selected release",
        ));
    }
    Ok(info)
}
async fn download(
    client: &reqwest::Client,
    name: &str,
    release: &Release,
    credentials: &(String, String),
) -> Result<Vec<u8>> {
    if !release.download_url.starts_with("/download/")
        || release.download_url.contains(['?', '#', '\\', '%'])
        || release.download_url.contains("..")
    {
        return Err(Error::bad("Invalid mod portal download path"));
    }
    let mut url = url::Url::parse("https://mods.factorio.com")
        .map_err(|_| Error::bad("Invalid portal URL"))?;
    url.set_path(&release.download_url);
    url.query_pairs_mut()
        .append_pair("username", &credentials.0)
        .append_pair("token", &credentials.1);
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| Error::bad("Mod download failed"))?;
    let bytes = response_bytes(response, MAX_ARCHIVE).await?;
    verify_download(name, release, &bytes)?;
    Ok(bytes)
}
fn commit_staged(
    app: &App,
    revision: &str,
    staged: &[(ModInfo, PathBuf)],
    engine: &str,
) -> Result<ModsDocument> {
    let document = current(app, revision)?;
    let path = directory(app)?;
    let (mut mod_list, _) = read_list(&path)?;
    let mut candidates = document.mods.clone();
    for (info, file) in staged {
        let enabled = candidates
            .iter()
            .find(|item| item.name == info.name)
            .is_none_or(|item| item.enabled);
        candidates.retain(|item| item.name != info.name);
        candidates.push(installed(info.clone(), enabled, fs::metadata(file)?.len()));
        set_entry(&mut mod_list, &info.name, enabled, &info.version);
    }
    validate_selection(app, &mod_list, &candidates, engine)?;
    let added: Vec<_> = staged
        .iter()
        .map(|(info, _)| format!("{}_{}.zip", info.name, info.version))
        .filter(|name| !path.join(name).exists())
        .collect();
    begin_transaction(app, added, Vec::new())?;
    let result = (|| {
        for (info, file) in staged {
            let target = path.join(format!("{}_{}.zip", info.name, info.version));
            storage::ensure_regular(&target)?;
            if target.exists() {
                if fs::read(&target)? != fs::read(file)? {
                    return Err(Error::conflict(
                        "A different archive already exists for this mod version",
                    ));
                }
            } else {
                storage::atomic_write(&target, &fs::read(file)?, false)?;
            }
        }
        write_activation(app, &mod_list, None)
    })();
    if let Err(error) = result {
        recover(app)?;
        return Err(error);
    }
    list(app)
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/mods/operation", get(operation_progress))
        .route("/mods/portal/search", get(search))
        .route("/mods/portal/{name}/releases", get(portal_versions))
        .route("/mods/portal/install", post(install_portal))
        .route("/mods/bulk/preview", post(bulk_preview))
        .route("/mods/bulk/apply", post(bulk_apply))
        .route("/mods/packs", get(packs).post(create_pack))
        .route("/mods/packs/{name}", axum::routing::delete(delete_pack))
        .route("/mods/packs/{name}/restore", post(restore_pack))
        .route("/mods/export", get(export_all))
        .route("/mods/list", get(export_list).put(import_list))
        .route("/mods/{name}", axum::routing::delete(remove_mod))
        .layer(DefaultBodyLimit::max(1024 * 1024))
}
#[utoipa::path(get,path="/api/mods/operation",responses((status=200,body=Option<ModOperationProgress>)))]
pub async fn operation_progress(
    State(app): State<App>,
) -> Result<Json<Option<ModOperationProgress>>> {
    let progress = PROGRESS
        .lock()
        .map_err(|_| Error::bad("Mod progress is unavailable"))?;
    Ok(Json(
        progress
            .as_ref()
            .filter(|(directory, _)| *directory == app.directory)
            .map(|(_, value)| value.clone()),
    ))
}
#[utoipa::path(get,path="/api/mods/portal/search",params(("q"=String,Query)),responses((status=200,body=Vec<PortalSearchEntry>)))]
pub async fn search(
    State(app): State<App>,
    Query(query): Query<SearchQuery>,
) -> Result<Json<Vec<PortalSearchEntry>>> {
    let query = query.q.trim();
    if query.is_empty() || query.len() > 100 {
        return Err(Error::bad("Search uses 1–100 characters"));
    }
    let (engine, credentials) = {
        let inner = app.inner.lock().await;
        (
            known_engine(&inner)?.to_owned(),
            credentials(&app)?.ok_or_else(|| {
                Error::bad("Portal search requires your Factorio username and token")
            })?,
        )
    };
    let _permit = UPDATE_CHECKS
        .acquire()
        .await
        .map_err(|_| Error::bad("Portal unavailable"))?;
    let response = client()?.post("https://mods.factorio.com/api/search").json(&serde_json::json!({"username":credentials.0,"token":credentials.1,"query":query,"version":engine,"page":1,"page_size":25,"highlight_pre_tag":"","highlight_post_tag":""})).send().await.map_err(|_| Error::bad("Portal search failed"))?;
    #[derive(Deserialize)]
    struct SearchResponse {
        results: Vec<PortalSearchEntry>,
    }
    let document: SearchResponse =
        serde_json::from_slice(&response_bytes(response, 4 * 1024 * 1024).await?)?;
    if document.results.len() > 25
        || document
            .results
            .iter()
            .any(|item| !valid_name(&item.name) || item.title.len() > 1000)
    {
        return Err(Error::bad("Unexpected portal search response"));
    }
    Ok(Json(document.results))
}
#[utoipa::path(get,path="/api/mods/portal/{name}/releases",params(("name"=String,Path)),responses((status=200,body=PortalReleases)))]
pub async fn portal_versions(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<PortalReleases>> {
    let engine = {
        let inner = app.inner.lock().await;
        known_engine(&inner)?.to_owned()
    };
    let _permit = UPDATE_CHECKS
        .acquire()
        .await
        .map_err(|_| Error::bad("Portal unavailable"))?;
    let mut releases: Vec<_> = full_releases(&client()?, &name)
        .await?
        .into_iter()
        .filter(|release| {
            version(&release.version).is_ok()
                && engine_compatible(
                    &release.info_json.factorio_version,
                    &release.info_json.dependencies,
                    &engine,
                )
        })
        .map(|release| PortalRelease {
            version: release.version,
            factorio_version: release.info_json.factorio_version,
            released_at: release.released_at,
        })
        .collect();
    releases.sort_by_key(|item| std::cmp::Reverse(version(&item.version).ok()));
    Ok(Json(PortalReleases { name, releases }))
}
async fn apply_changes(
    app: &App,
    revision: &str,
    changes: &[(String, String)],
) -> Result<ModsDocument> {
    let _operation = OPERATIONS.lock().await;
    if changes.is_empty() || changes.len() > MAX_MODS {
        return Err(Error::bad("Select 1–500 mod versions"));
    }
    let (engine, credentials) = {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        recover(app)?;
        current(app, revision)?;
        (
            known_engine(&inner)?.to_owned(),
            credentials(app)?
                .ok_or_else(|| Error::bad("Mod downloads require a Factorio username and token"))?,
        )
    };
    let progress = ProgressGuard::new(app, changes.len())?;
    let mut seen = HashSet::new();
    let client = client()?;
    let staging = tempfile::tempdir_in(&app.directory)?;
    let mut staged = Vec::new();
    let mut total = 0_u64;
    for (index, (name, target)) in changes.iter().enumerate() {
        progress.update("Checking release", index, Some(name))?;
        if !valid_name(name) || protected(name) || !seen.insert(name) {
            return Err(Error::bad("Invalid or duplicate mod selection"));
        }
        version(target)?;
        let releases = full_releases(&client, name).await?;
        let release = releases
            .iter()
            .find(|release| {
                release.version == *target
                    && engine_compatible(
                        &release.info_json.factorio_version,
                        &release.info_json.dependencies,
                        &engine,
                    )
            })
            .ok_or_else(|| Error::bad("Selected mod release is unavailable or incompatible"))?;
        progress.update("Downloading", index, Some(name))?;
        let bytes = download(&client, name, release, &credentials).await?;
        total = total.saturating_add(bytes.len() as u64);
        if total > MAX_BATCH_BYTES {
            return Err(Error::bad("Mod batch exceeds 1 GiB"));
        }
        let info = verify_download(name, release, &bytes)?;
        let path = staging.path().join(format!("{name}_{target}.zip"));
        storage::atomic_write(&path, &bytes, false)?;
        staged.push((info, path));
    }
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    if known_engine(&inner)? != engine {
        return Err(Error::conflict(
            "Factorio version changed; review mod choices again",
        ));
    }
    progress.update("Validating and activating", changes.len(), None)?;
    let result = commit_staged(app, revision, &staged, &engine)?;
    progress.complete()?;
    Ok(result)
}
#[utoipa::path(post,path="/api/mods/portal/install",request_body=PortalInstallRequest,responses((status=200,body=ModsDocument)))]
pub async fn install_portal(
    State(app): State<App>,
    input: std::result::Result<Json<PortalInstallRequest>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    Ok(Json(
        apply_changes(&app, &input.revision, &[(input.name, input.version)]).await?,
    ))
}
#[utoipa::path(post,path="/api/mods/bulk/preview",request_body=RevisionRequest,responses((status=200,body=BulkPlan)))]
pub async fn bulk_preview(
    State(app): State<App>,
    input: std::result::Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<BulkPlan>> {
    let input = json(input)?;
    let (document, engine) = {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        (
            current(&app, &input.revision)?,
            known_engine(&inner)?.to_owned(),
        )
    };
    let _permit = UPDATE_CHECKS
        .acquire()
        .await
        .map_err(|_| Error::bad("Portal unavailable"))?;
    let client = client()?;
    let mut changes = Vec::new();
    for item in document.mods {
        let releases = full_releases(&client, &item.name).await?;
        if let Some(latest) = newest(&releases, &engine)
            && version(&latest.version)? > version(&item.version)?
        {
            changes.push(BulkChange {
                name: item.name,
                current_version: item.version,
                version: latest.version.clone(),
            });
        }
    }
    {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        current(&app, &input.revision)?;
    }
    Ok(Json(BulkPlan {
        revision: input.revision,
        changes,
    }))
}
#[utoipa::path(post,path="/api/mods/bulk/apply",request_body=BulkPlan,responses((status=200,body=ModsDocument)))]
pub async fn bulk_apply(
    State(app): State<App>,
    input: std::result::Result<Json<BulkPlan>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        let document = current(&app, &input.revision)?;
        for change in &input.changes {
            if !document
                .mods
                .iter()
                .any(|item| item.name == change.name && item.version == change.current_version)
                || version(&change.version)? <= version(&change.current_version)?
            {
                return Err(Error::conflict(
                    "Bulk update plan is stale or does not contain upgrades",
                ));
            }
        }
    }
    Ok(Json(
        apply_changes(
            &app,
            &input.revision,
            &input
                .changes
                .into_iter()
                .map(|item| (item.name, item.version))
                .collect::<Vec<_>>(),
        )
        .await?,
    ))
}
#[utoipa::path(get,path="/api/mods/list",responses((status=200,body=ListDocument)))]
pub async fn export_list(State(app): State<App>) -> Result<Json<ListDocument>> {
    let _inner = app.inner.lock().await;
    let document = list(&app)?;
    let (mut list, _) = read_list(&directory(&app)?)?;
    for item in &document.mods {
        set_entry(&mut list, &item.name, item.enabled, &item.version);
    }
    Ok(Json(ListDocument {
        revision: document.revision,
        mods: list
            .mods
            .into_iter()
            .map(|item| ListEntry {
                name: item.name,
                enabled: item.enabled,
                version: item.version,
            })
            .collect(),
    }))
}
#[utoipa::path(put,path="/api/mods/list",request_body=ListDocument,responses((status=200,body=ModsDocument)))]
pub async fn import_list(
    State(app): State<App>,
    input: std::result::Result<Json<ListDocument>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    recover(&app)?;
    current(&app, &input.revision)?;
    let requested = ModList {
        mods: input
            .mods
            .into_iter()
            .map(|item| ModListEntry {
                name: item.name,
                enabled: item.enabled,
                version: item.version,
                other: BTreeMap::new(),
            })
            .collect(),
        other: BTreeMap::new(),
    };
    let (list, candidates) = selection(&app, &requested)?;
    validate_selection(&app, &list, &candidates, known_engine(&inner)?)?;
    write_activation(&app, &list, None)?;
    Ok(Json(list_mods(&app)?))
}
fn list_mods(app: &App) -> Result<ModsDocument> {
    super::list(app)
}
#[utoipa::path(get,path="/api/mods/packs",responses((status=200,body=Vec<PackSummary>)))]
pub async fn packs(State(app): State<App>) -> Result<Json<Vec<PackSummary>>> {
    let _inner = app.inner.lock().await;
    let path = subdirectory(&app.directory, "mod-packs")?;
    let mut result = Vec::new();
    for file in fs::read_dir(path)? {
        let file = file?;
        if !file.file_name().to_string_lossy().ends_with(".json") {
            continue;
        }
        if result.len() >= 100 {
            return Err(Error::bad("Too many mod packs"));
        }
        let name = file
            .file_name()
            .to_string_lossy()
            .trim_end_matches(".json")
            .to_owned();
        let pack = read_pack(&app, &name)?;
        result.push(PackSummary {
            name,
            mods: pack.archives.len(),
        });
    }
    result.sort_by(|a, b| a.name.cmp(&b.name));
    Ok(Json(result))
}
fn read_pack(app: &App, name: &str) -> Result<Pack> {
    let path = pack_path(app, name)?;
    storage::ensure_regular(&path)?;
    let file = fs::File::open(path)?;
    let mut bytes = Vec::new();
    file.take(128 * 1024 * 1024 + 1).read_to_end(&mut bytes)?;
    if bytes.len() > 128 * 1024 * 1024 {
        return Err(Error::bad("Mod pack exceeds limit"));
    }
    Ok(serde_json::from_slice(&bytes)?)
}
#[utoipa::path(post,path="/api/mods/packs",request_body=PackRequest,responses((status=200,body=Vec<PackSummary>)))]
pub async fn create_pack(
    State(app): State<App>,
    input: std::result::Result<Json<PackRequest>, JsonRejection>,
) -> Result<Json<Vec<PackSummary>>> {
    let input = json(input)?;
    {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        recover(&app)?;
        let document = current(&app, &input.revision)?;
        let path = pack_path(&app, &input.name)?;
        let root = directory(&app)?;
        let (mut list, _) = read_list(&root)?;
        let mut archives = Vec::new();
        if fs::read_dir(
            path.parent()
                .ok_or_else(|| Error::bad("Missing pack directory"))?,
        )?
        .count()
            >= 100
        {
            return Err(Error::bad("At most 100 mod packs are supported"));
        }
        for item in document.mods {
            set_entry(&mut list, &item.name, item.enabled, &item.version);
            let (_, _) = archive_info(&root.join(&item.file_name))?;
            archives.push(PackArchive {
                sha256: format!(
                    "{:x}",
                    Sha256::digest(fs::read(root.join(&item.file_name))?)
                ),
                file_name: item.file_name,
            });
        }
        let pack = Pack {
            list,
            archives,
            settings: mod_settings::read_bytes(&root.join("mod-settings.dat"))?,
        };
        storage::atomic_write(&path, &serde_json::to_vec(&pack)?, false)?;
    }
    packs(State(app)).await
}
#[utoipa::path(delete,path="/api/mods/packs/{name}",params(("name"=String,Path),("revision"=String,Query)),responses((status=200,body=Vec<PackSummary>)))]
pub async fn delete_pack(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(query): Query<RevisionQuery>,
) -> Result<Json<Vec<PackSummary>>> {
    {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        current(&app, &query.revision)?;
        let path = pack_path(&app, &name)?;
        storage::ensure_regular(&path)?;
        fs::remove_file(path)?;
    }
    packs(State(app)).await
}
#[utoipa::path(post,path="/api/mods/packs/{name}/restore",params(("name"=String,Path)),request_body=RevisionRequest,responses((status=200,body=ModsDocument)))]
pub async fn restore_pack(
    State(app): State<App>,
    Path(name): Path<String>,
    input: std::result::Result<Json<RevisionRequest>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    recover(&app)?;
    current(&app, &input.revision)?;
    let pack = read_pack(&app, &name)?;
    let root = directory(&app)?;
    if pack.archives.len() > MAX_MODS
        || pack
            .archives
            .iter()
            .any(|item| !safe_archive_name(&item.file_name))
    {
        return Err(Error::bad("Invalid mod pack archive references"));
    }
    let added: Vec<_> = pack
        .archives
        .iter()
        .filter(|item| !root.join(&item.file_name).exists())
        .map(|item| item.file_name.clone())
        .collect();
    begin_transaction(&app, added, Vec::new())?;
    let result = (|| {
        for archive in &pack.archives {
            if !safe_archive_name(&archive.file_name) {
                return Err(Error::bad("Invalid archive reference in pack"));
            }
            let path = root.join(&archive.file_name);
            let source = if path.exists() {
                path.clone()
            } else {
                subdirectory(&root, ".archive")?.join(&archive.file_name)
            };
            let (_, _) = archive_info(&source)?;
            let bytes = fs::read(&source)?;
            if format!("{:x}", Sha256::digest(&bytes)) != archive.sha256 {
                return Err(Error::bad("Mod pack archive checksum changed"));
            }
            if !path.exists() {
                storage::atomic_write(&path, &bytes, false)?;
            }
        }
        if let Some(bytes) = &pack.settings {
            let staging = tempfile::tempdir_in(&app.directory)?;
            let path = staging.path().join("mod-settings.dat");
            storage::atomic_write(&path, bytes, false)?;
            mod_settings::read(&path)?;
        }
        let (list, candidates) = selection(&app, &pack.list)?;
        validate_selection(&app, &list, &candidates, known_engine(&inner)?)?;
        write_activation(&app, &list, Some(pack.settings.as_deref()))
    })();
    if let Err(error) = result {
        recover(&app)?;
        return Err(error);
    }
    Ok(Json(list_mods(&app)?))
}
fn safe_archive_name(value: &str) -> bool {
    value.ends_with(".zip")
        && !value.contains(['/', '\\'])
        && !value.starts_with('.')
        && value.len() <= 220
}
#[utoipa::path(delete,path="/api/mods/{name}",params(("name"=String,Path),("revision"=String,Query)),responses((status=200,body=ModsDocument)))]
pub async fn remove_mod(
    State(app): State<App>,
    Path(name): Path<String>,
    Query(query): Query<RevisionQuery>,
) -> Result<Json<ModsDocument>> {
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    recover(&app)?;
    let document = current(&app, &query.revision)?;
    if !valid_name(&name) || protected(&name) || !document.mods.iter().any(|item| item.name == name)
    {
        return Err(Error::bad("Mod is not installed or cannot be removed"));
    }
    let root = directory(&app)?;
    let (mut mod_list, _) = read_list(&root)?;
    mod_list.mods.retain(|item| item.name != name);
    let candidates: Vec<_> = document
        .mods
        .into_iter()
        .filter(|item| item.name != name)
        .collect();
    validate_selection(&app, &mod_list, &candidates, known_engine(&inner)?)?;
    let archive = subdirectory(&root, ".archive")?;
    let mut moves = Vec::new();
    for file in fs::read_dir(&root)? {
        let file = file?;
        if file.file_name().to_string_lossy().ends_with(".zip")
            && archive_info(&file.path())?.0.name == name
        {
            moves.push(file.file_name().to_string_lossy().into_owned());
        }
    }
    begin_transaction(&app, Vec::new(), moves)?;
    let result = (|| {
        for file in fs::read_dir(&root)? {
            let file = file?;
            if !file.file_name().to_string_lossy().ends_with(".zip") {
                continue;
            }
            let (info, _) = archive_info(&file.path())?;
            if info.name == name {
                let target = archive.join(file.file_name());
                storage::ensure_regular(&target)?;
                if target.exists() {
                    if fs::read(&target)? != fs::read(file.path())? {
                        return Err(Error::conflict("Archived version has different content"));
                    }
                    fs::remove_file(file.path())?;
                } else {
                    fs::rename(file.path(), &target)?;
                }
            }
        }
        fs::File::open(&archive)?.sync_all()?;
        fs::File::open(&root)?.sync_all()?;
        write_activation(&app, &mod_list, None)
    })();
    if let Err(error) = result {
        recover(&app)?;
        return Err(error);
    }
    Ok(Json(list_mods(&app)?))
}
#[utoipa::path(get,path="/api/mods/export",responses((status=200,content_type="application/zip")))]
pub async fn export_all(State(app): State<App>) -> Result<Response> {
    let _inner = app.inner.lock().await;
    let document = list(&app)?;
    let root = directory(&app)?;
    let temporary = tempfile::tempfile_in(&app.directory)?;
    let mut writer = zip::ZipWriter::new(temporary);
    let options =
        zip::write::SimpleFileOptions::default().compression_method(zip::CompressionMethod::Stored);
    let mut total = 0_u64;
    for item in document.mods {
        let path = root.join(&item.file_name);
        storage::ensure_regular(&path)?;
        total = total.saturating_add(fs::metadata(&path)?.len());
        if total > MAX_BATCH_BYTES {
            return Err(Error::bad("Mod export exceeds 1 GiB"));
        }
        writer
            .start_file(item.file_name, options)
            .map_err(|_| Error::bad("Could not build mod export"))?;
        std::io::copy(&mut fs::File::open(path)?, &mut writer)?;
    }
    let (list, _) = read_list(&root)?;
    writer
        .start_file("mod-list.json", options)
        .map_err(|_| Error::bad("Could not build mod export"))?;
    writer.write_all(&serde_json::to_vec_pretty(&list)?)?;
    if let Some(bytes) = mod_settings::read_bytes(&root.join("mod-settings.dat"))? {
        writer
            .start_file("mod-settings.dat", options)
            .map_err(|_| Error::bad("Could not build mod export"))?;
        writer.write_all(&bytes)?;
    }
    let mut file = writer
        .finish()
        .map_err(|_| Error::bad("Could not finish mod export"))?;
    use std::io::Seek;
    file.rewind()?;
    let file = tokio::fs::File::from_std(file);
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"factorio-mods.zip\"",
            ),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}

#[cfg(test)]
mod tests {
    use super::*;
    fn fixture(name: &str, value: &str, dependencies: &[&str]) -> Result<Vec<u8>> {
        let mut writer = zip::ZipWriter::new(Cursor::new(Vec::new()));
        writer
            .start_file(
                format!("{name}_{value}/info.json"),
                zip::write::SimpleFileOptions::default(),
            )
            .map_err(|_| zip_error())?;
        writer.write_all(&serde_json::to_vec(&serde_json::json!({"name":name,"version":value,"title":name,"author":"Synthetic fixture","factorio_version":"2.1","dependencies":dependencies}))?)?;
        Ok(writer.finish().map_err(|_| zip_error())?.into_inner())
    }
    fn app() -> Result<(tempfile::TempDir, App)> {
        let directory = tempfile::tempdir()?;
        let app = App::new(directory.path().into(), None)?;
        Ok((directory, app))
    }
    fn put(app: &App, name: &str, value: &str, dependencies: &[&str]) -> Result<()> {
        let bytes = fixture(name, value, dependencies)?;
        install(app, &list(app)?.revision, &bytes, Some("2.1.23"))?;
        Ok(())
    }
    fn stage(
        app: &App,
        name: &str,
        value: &str,
        dependencies: &[&str],
    ) -> Result<(tempfile::NamedTempFile, ModInfo)> {
        let bytes = fixture(name, value, dependencies)?;
        let mut file = tempfile::NamedTempFile::new_in(&app.directory)?;
        file.write_all(&bytes)?;
        Ok((file, validate_archive(&bytes)?))
    }
    fn settings(value: bool) -> Vec<u8> {
        fn string(value: &str) -> Vec<u8> {
            let mut bytes = vec![0, value.len() as u8];
            bytes.extend(value.as_bytes());
            bytes
        }
        fn dict(entries: Vec<(&str, Vec<u8>)>) -> Vec<u8> {
            let mut bytes = vec![5, 0];
            bytes.extend((entries.len() as u32).to_le_bytes());
            for (key, value) in entries {
                bytes.extend(string(key));
                bytes.extend(value);
            }
            bytes
        }
        let startup = dict(vec![(
            "fixture",
            dict(vec![("value", vec![1, 1, u8::from(value)])]),
        )]);
        let mut bytes = vec![2, 0, 1, 0, 23, 0, 0, 0, 0];
        bytes.extend(dict(vec![
            ("startup", startup),
            ("runtime-global", dict(vec![])),
            ("runtime-per-user", dict(vec![])),
        ]));
        bytes
    }
    #[test]
    fn checksum_identity_and_corruption_are_rejected() -> Result<()> {
        let bytes = fixture("alpha", "1.0.0", &["base"])?;
        let mut release = Release {
            version: "1.0.0".into(),
            released_at: None,
            download_url: "/download/fixture".into(),
            sha1: "0".repeat(40),
            info_json: ReleaseInfo {
                factorio_version: "2.1".into(),
                dependencies: vec![],
            },
        };
        assert!(verify_download("alpha", &release, &bytes).is_err());
        release.sha1 = format!("{:x}", sha1::Sha1::digest(&bytes));
        assert!(verify_download("other", &release, &bytes).is_err());
        assert!(verify_download("alpha", &release, &bytes).is_ok());
        release.sha1 = format!("{:x}", sha1::Sha1::digest(b"not a zip"));
        assert!(verify_download("alpha", &release, b"not a zip").is_err());
        Ok(())
    }
    #[test]
    fn dependency_failure_and_stale_revision_leave_no_files() -> Result<()> {
        let (_temporary, app) = app()?;
        put(&app, "alpha", "1.0.0", &["base"])?;
        let revision = list(&app)?.revision;
        let (file, info) = stage(&app, "beta", "1.0.0", &["missing >= 2.0.0"])?;
        let staged = vec![(info, file.path().into())];
        assert!(commit_staged(&app, &revision, &staged, "2.1.23").is_err());
        assert_eq!(list(&app)?.revision, revision);
        assert!(!directory(&app)?.join("beta_1.0.0.zip").exists());
        assert_eq!(
            commit_staged(&app, "stale", &staged, "2.1.23")
                .err()
                .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[test]
    fn complete_batch_validates_dependencies_together() -> Result<()> {
        let (_temporary, app) = app()?;
        put(&app, "alpha", "1.0.0", &["base"])?;
        let revision = list(&app)?.revision;
        let (a, alpha) = stage(&app, "alpha", "2.0.0", &["base"])?;
        let (b, beta) = stage(&app, "beta", "1.0.0", &["alpha >= 2.0.0"])?;
        let result = commit_staged(
            &app,
            &revision,
            &[(alpha, a.path().into()), (beta, b.path().into())],
            "2.1.23",
        )?;
        assert_eq!(result.mods.len(), 2);
        assert!(
            result
                .mods
                .iter()
                .any(|item| item.name == "alpha" && item.version == "2.0.0")
        );
        assert!(directory(&app)?.join("alpha_1.0.0.zip").exists());
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[test]
    fn failed_archive_commit_rolls_back_earlier_files() -> Result<()> {
        let (_temporary, app) = app()?;
        put(&app, "alpha", "1.0.0", &["base"])?;
        put(&app, "omega", "1.0.0", &["base"])?;
        let revision = list(&app)?.revision;
        let (a, alpha) = stage(&app, "alpha", "2.0.0", &["base"])?;
        let (o, omega) = stage(&app, "omega", "1.0.0", &["base >= 2.1"])?;
        assert!(
            commit_staged(
                &app,
                &revision,
                &[(alpha, a.path().into()), (omega, o.path().into())],
                "2.1.23"
            )
            .is_err()
        );
        assert_eq!(list(&app)?.revision, revision);
        assert!(!directory(&app)?.join("alpha_2.0.0.zip").exists());
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[tokio::test]
    async fn mod_list_rejects_missing_zip_duplicates_and_dependant_removal() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        put(&app, "beta", "1.0.0", &["alpha"])?;
        let revision = list(&app)?.revision;
        let result = remove_mod(
            State(app.clone()),
            Path("alpha".into()),
            Query(RevisionQuery {
                revision: revision.clone(),
            }),
        )
        .await;
        assert!(result.is_err());
        assert_eq!(list(&app)?.revision, revision);
        for entries in [
            vec![ListEntry {
                name: "missing".into(),
                enabled: true,
                version: Some("1.0.0".into()),
            }],
            vec![
                ListEntry {
                    name: "alpha".into(),
                    enabled: true,
                    version: None,
                },
                ListEntry {
                    name: "alpha".into(),
                    enabled: false,
                    version: None,
                },
            ],
        ] {
            assert!(
                import_list(
                    State(app.clone()),
                    Ok(Json(ListDocument {
                        revision: revision.clone(),
                        mods: entries
                    }))
                )
                .await
                .is_err()
            );
            assert_eq!(list(&app)?.revision, revision);
        }
        Ok(())
    }
    #[tokio::test]
    async fn packs_restore_removed_archives_and_startup_settings() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        let path = directory(&app)?.join("mod-settings.dat");
        storage::atomic_write(&path, &settings(true), false)?;
        let _ = create_pack(
            State(app.clone()),
            Ok(Json(PackRequest {
                revision: list(&app)?.revision,
                name: "original".into(),
            })),
        )
        .await?;
        let _ = remove_mod(
            State(app.clone()),
            Path("alpha".into()),
            Query(RevisionQuery {
                revision: list(&app)?.revision,
            }),
        )
        .await?;
        assert!(list(&app)?.mods.is_empty());
        assert!(directory(&app)?.join(".archive/alpha_1.0.0.zip").exists());
        storage::atomic_write(&path, &settings(false), true)?;
        let _ = restore_pack(
            State(app.clone()),
            Path("original".into()),
            Ok(Json(RevisionRequest {
                revision: list(&app)?.revision,
            })),
        )
        .await?;
        assert_eq!(list(&app)?.mods.len(), 1);
        assert_eq!(fs::read(path)?, settings(true));
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[tokio::test]
    async fn pack_checksum_failure_leaves_selection_unchanged() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        let _ = create_pack(
            State(app.clone()),
            Ok(Json(PackRequest {
                revision: list(&app)?.revision,
                name: "original".into(),
            })),
        )
        .await?;
        storage::atomic_write(
            &directory(&app)?.join("alpha_1.0.0.zip"),
            &fixture("alpha", "1.0.0", &["base >= 2.1"])?,
            true,
        )?;
        let before = list(&app)?.revision;
        assert!(
            restore_pack(
                State(app.clone()),
                Path("original".into()),
                Ok(Json(RevisionRequest {
                    revision: before.clone()
                }))
            )
            .await
            .is_err()
        );
        assert_eq!(list(&app)?.revision, before);
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[test]
    fn interrupted_transaction_blocks_start_and_recovers_all_changes() -> Result<()> {
        let (_temporary, app) = app()?;
        put(&app, "alpha", "1.0.0", &["base"])?;
        let revision = list(&app)?.revision;
        let root = directory(&app)?;
        let archive = subdirectory(&root, ".archive")?;
        begin_transaction(
            &app,
            vec!["beta_1.0.0.zip".into()],
            vec!["alpha_1.0.0.zip".into()],
        )?;
        fs::rename(
            root.join("alpha_1.0.0.zip"),
            archive.join("alpha_1.0.0.zip"),
        )?;
        storage::atomic_write(
            &root.join("beta_1.0.0.zip"),
            &fixture("beta", "1.0.0", &["base"])?,
            false,
        )?;
        storage::atomic_write(&root.join("mod-list.json"), b"{\"mods\":[]}", true)?;
        assert!(validate_start(&app, "2.1.23").is_err());
        recover(&app)?;
        assert_eq!(list(&app)?.revision, revision);
        assert!(!root.join("beta_1.0.0.zip").exists());
        assert!(!journal_path(&app).exists());
        Ok(())
    }
    #[tokio::test]
    async fn exports_are_bounded_disk_backed_zip_and_running_mutations_fail() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        let response = export_all(State(app.clone())).await?;
        let bytes = axum::body::to_bytes(response.into_body(), 1024 * 1024)
            .await
            .map_err(|_| Error::bad("Read export failed"))?;
        let mut zip = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| zip_error())?;
        assert!(zip.by_name("alpha_1.0.0.zip").is_ok());
        assert!(zip.by_name("mod-list.json").is_ok());
        app.inner.lock().await.status.state = ServerState::Running;
        assert_eq!(
            remove_mod(
                State(app.clone()),
                Path("alpha".into()),
                Query(RevisionQuery {
                    revision: list(&app)?.revision
                })
            )
            .await
            .err()
            .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        Ok(())
    }
    #[tokio::test]
    async fn settings_changes_invalidate_pack_restore_revision() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        let path = directory(&app)?.join("mod-settings.dat");
        storage::atomic_write(&path, &settings(true), false)?;
        let _ = create_pack(
            State(app.clone()),
            Ok(Json(PackRequest {
                revision: list(&app)?.revision,
                name: "original".into(),
            })),
        )
        .await?;
        let before = list(&app)?.revision;
        storage::atomic_write(&path, &settings(false), true)?;
        let result = restore_pack(
            State(app.clone()),
            Path("original".into()),
            Ok(Json(RevisionRequest { revision: before })),
        )
        .await;
        assert_eq!(
            result.err().map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        assert_eq!(fs::read(path)?, settings(false));
        Ok(())
    }
    #[tokio::test]
    async fn pending_journal_blocks_existing_mod_and_settings_writes() -> Result<()> {
        let (_temporary, app) = app()?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        put(&app, "alpha", "1.0.0", &["base"])?;
        let before = list(&app)?.revision;
        begin_transaction(&app, Vec::new(), Vec::new())?;
        assert!(
            install(
                &app,
                &before,
                &fixture("beta", "1.0.0", &["base"])?,
                Some("2.1.23")
            )
            .is_err()
        );
        assert_eq!(
            set_enabled(
                State(app.clone()),
                Path("alpha".into()),
                Ok(Json(ModEnabledRequest {
                    revision: before.clone(),
                    enabled: false
                }))
            )
            .await
            .err()
            .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        assert_eq!(
            put_mod_settings(
                State(app.clone()),
                Ok(Json(ModSettingsPatch {
                    revision: "missing".into(),
                    changes: BTreeMap::new()
                }))
            )
            .await
            .err()
            .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        let mut headers = HeaderMap::new();
        headers.insert(
            header::IF_MATCH,
            header::HeaderValue::from_static("missing"),
        );
        assert_eq!(
            upload_settings(State(app.clone()), headers, Ok(Bytes::from(settings(true))))
                .await
                .err()
                .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        assert_eq!(
            save_sync::apply(
                State(app.clone()),
                Path("fixture.zip".into()),
                Ok(Json(save_sync::SyncRequest {
                    revision: "stale".into(),
                    operation_id: Some("journal-test".into())
                }))
            )
            .await
            .err()
            .map(|error| error.status),
            Some(StatusCode::CONFLICT)
        );
        recover(&app)?;
        assert_eq!(list(&app)?.revision, before);
        assert!(!directory(&app)?.join("mod-settings.dat").exists());
        Ok(())
    }
}
