use super::*;
use crate::hex;
use crate::save_metadata;

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncAction {
    Keep,
    Enable,
    Switch,
    Download,
    Bundled,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SyncMod {
    pub name: String,
    pub version: String,
    pub action: SyncAction,
}
#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SyncPlan {
    pub revision: String,
    pub save: String,
    pub factorio_version: String,
    pub engine_version: String,
    pub mods: Vec<SyncMod>,
    pub disable: Vec<String>,
    pub blockers: Vec<String>,
}
#[derive(Deserialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct SyncRequest {
    pub revision: String,
    #[serde(default)]
    pub operation_id: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
#[serde(rename_all = "snake_case")]
pub enum SyncPhase {
    Preparing,
    Resolving,
    Downloading,
    Validating,
    Applying,
    Complete,
    Failed,
}

impl SyncPhase {
    fn active(&self) -> bool {
        !matches!(self, Self::Complete | Self::Failed)
    }
}

#[derive(Clone, Debug, Serialize, Deserialize, ToSchema)]
pub struct SyncProgress {
    pub operation_id: String,
    pub save: String,
    pub phase: SyncPhase,
    pub completed: usize,
    pub total: usize,
    pub current_mod: Option<String>,
    pub error: Option<String>,
}

struct ProgressGuard {
    state: std::sync::Arc<std::sync::Mutex<Option<SyncProgress>>>,
    operation_id: String,
    finished: bool,
}

impl ProgressGuard {
    fn start(app: &App, save: &str, operation_id: Option<String>) -> Result<Self> {
        static NEXT_OPERATION: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let operation_id = operation_id.unwrap_or_else(|| {
            format!(
                "sync-{}-{}",
                crate::now(),
                NEXT_OPERATION.fetch_add(1, std::sync::atomic::Ordering::Relaxed)
            )
        });
        if operation_id.is_empty()
            || operation_id.len() > 64
            || !operation_id
                .bytes()
                .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_')
        {
            return Err(Error::bad("Invalid sync operation ID"));
        }
        let mut state = app
            .sync_progress
            .lock()
            .map_err(|_| Error::bad("Sync progress is unavailable"))?;
        if state
            .as_ref()
            .is_some_and(|progress| progress.phase.active())
        {
            return Err(Error::conflict(
                "Another save mod sync is already in progress",
            ));
        }
        *state = Some(SyncProgress {
            operation_id: operation_id.clone(),
            save: save.into(),
            phase: SyncPhase::Preparing,
            completed: 0,
            total: 0,
            current_mod: None,
            error: None,
        });
        Ok(Self {
            state: app.sync_progress.clone(),
            operation_id,
            finished: false,
        })
    }

    fn update(
        &self,
        phase: SyncPhase,
        completed: usize,
        total: usize,
        current_mod: Option<String>,
    ) -> Result<()> {
        let mut state = self
            .state
            .lock()
            .map_err(|_| Error::bad("Sync progress is unavailable"))?;
        if let Some(progress) = state.as_mut().filter(|progress| {
            progress.operation_id == self.operation_id && progress.phase.active()
        }) {
            progress.phase = phase;
            progress.completed = completed;
            progress.total = total;
            progress.current_mod = current_mod;
        }
        Ok(())
    }

    fn finish(&mut self, error: Option<String>) {
        let mut state = match self.state.lock() {
            Ok(state) => state,
            Err(poisoned) => poisoned.into_inner(),
        };
        if let Some(progress) = state.as_mut().filter(|progress| {
            progress.operation_id == self.operation_id && progress.phase.active()
        }) {
            progress.phase = if error.is_some() {
                SyncPhase::Failed
            } else {
                SyncPhase::Complete
            };
            progress.current_mod = None;
            progress.error = error;
        }
        self.finished = true;
    }
}

impl Drop for ProgressGuard {
    fn drop(&mut self) {
        if !self.finished {
            self.finish(Some(
                "Sync was interrupted. Review the plan before retrying".into(),
            ));
        }
    }
}

#[utoipa::path(get, path="/api/saves/{name}/mods/sync", params(("name"=String, Path)), responses((status=200, body=Option<SyncProgress>), (status=400, body=ApiError)))]
pub async fn progress(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<Option<SyncProgress>>> {
    let state = app
        .sync_progress
        .lock()
        .map_err(|_| Error::bad("Sync progress is unavailable"))?;
    Ok(Json(
        state
            .as_ref()
            .filter(|progress| progress.save == name)
            .cloned(),
    ))
}

fn inventory(app: &App) -> Result<Vec<InstalledMod>> {
    let mut result = Vec::new();
    for item in fs::read_dir(directory(app)?)? {
        let item = item?;
        let file_name = item.file_name().to_string_lossy().into_owned();
        if !file_name.ends_with(".zip") {
            continue;
        }
        storage::ensure_regular(&item.path())?;
        let size_bytes = item.metadata()?.len();
        if size_bytes > MAX_ARCHIVE as u64 || result.len() >= MAX_MODS * 10 {
            return Err(Error::bad("Installed mod archives exceed supported limits"));
        }
        let mut archive =
            zip::ZipArchive::new(fs::File::open(item.path())?).map_err(|_| zip_error())?;
        let info = read_info(&mut archive)?;
        result.push(InstalledMod {
            name: info.name,
            version: info.version,
            title: info.title,
            author: info.author,
            description: info.description,
            factorio_version: info.factorio_version,
            dependencies: info.dependencies,
            enabled: true,
            file_name,
            size_bytes,
        });
    }
    Ok(result)
}

fn build_plan(
    app: &App,
    name: &str,
    engine: &str,
) -> Result<(SyncPlan, Vec<InstalledMod>, ModList)> {
    let save_path = storage::save_path(&app.saves_path(), name)?;
    let metadata = save_metadata::read(&save_path)?;
    let save_stat = fs::metadata(save_path)?;
    let current = list(app)?;
    let archives = inventory(app)?;
    let (mod_list, _) = read_list(&directory(app)?)?;
    let bundled = bundled_mods(app)?;
    let mut hash = Sha256::new();
    hash.update(current.revision.as_bytes());
    hash.update(serde_json::to_vec(&metadata)?);
    hash.update(engine.as_bytes());
    hash.update(name.as_bytes());
    hash.update(save_stat.len().to_le_bytes());
    hash.update(
        save_stat
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::bad("Invalid save timestamp"))?
            .as_nanos()
            .to_le_bytes(),
    );
    let mut blockers = Vec::new();
    if family(&metadata.factorio_version) != family(engine) {
        blockers.push(format!("Save uses Factorio {}; configured game is {engine}. Use the matching game series before syncing.", metadata.factorio_version));
    }
    if version(&metadata.factorio_version)? > version(engine)? {
        blockers.push("This save was made with a newer Factorio version. Update the configured game before syncing.".into());
    }
    if metadata.mods.len() > MAX_MODS || !metadata.mods.iter().any(|item| item.name == "base") {
        return Err(Error::bad("Save has an invalid mod list"));
    }
    let mut wanted = HashSet::new();
    let mut mods = Vec::new();
    for item in &metadata.mods {
        if !valid_name(&item.name) || !wanted.insert(item.name.clone()) {
            return Err(Error::bad("Save contains invalid or duplicate mod names"));
        }
        version(&item.version)?;
        let action = if protected(&item.name) || bundled.contains_key(&item.name) {
            if !bundled.contains_key(&item.name) && item.name != "base" {
                blockers.push(format!(
                    "Bundled mod {} is missing from the configured game installation.",
                    item.name
                ));
            }
            SyncAction::Bundled
        } else if archives
            .iter()
            .any(|local| local.name == item.name && local.version == item.version)
        {
            match current.mods.iter().find(|local| local.name == item.name) {
                Some(local) if local.version == item.version && local.enabled => SyncAction::Keep,
                Some(local) if local.version == item.version => SyncAction::Enable,
                _ => SyncAction::Switch,
            }
        } else {
            SyncAction::Download
        };
        mods.push(SyncMod {
            name: item.name.clone(),
            version: item.version.clone(),
            action,
        });
    }
    let downloads = mods
        .iter()
        .filter(|item| matches!(item.action, SyncAction::Download))
        .count();
    if downloads > 0 && !current.portal_authenticated {
        blockers.push(format!(
            "{downloads} {} downloads. {}",
            if downloads == 1 {
                "mod requires"
            } else {
                "mods require"
            },
            credential_requirement(app)?
        ));
    }
    let mut disable = HashSet::new();
    for item in current.mods.iter().filter(|item| item.enabled) {
        if !wanted.contains(&item.name) {
            disable.insert(item.name.clone());
        }
    }
    for entry in &mod_list.mods {
        if entry.enabled && !wanted.contains(&entry.name) {
            disable.insert(entry.name.clone());
        }
    }
    for name in bundled.keys() {
        if !wanted.contains(name)
            && mod_list
                .mods
                .iter()
                .find(|entry| &entry.name == name)
                .is_none_or(|entry| entry.enabled)
        {
            disable.insert(name.clone());
        }
    }
    let mut disable: Vec<String> = disable.into_iter().collect();
    disable.sort();
    Ok((
        SyncPlan {
            revision: hex(&hash.finalize()),
            save: name.into(),
            factorio_version: metadata.factorio_version,
            engine_version: engine.into(),
            mods,
            disable,
            blockers,
        },
        archives,
        mod_list,
    ))
}

#[utoipa::path(get, path="/api/saves/{name}/mods", params(("name"=String, Path)), responses((status=200, body=SyncPlan), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn plan(State(app): State<App>, Path(name): Path<String>) -> Result<Json<SyncPlan>> {
    let engine = known_engine(&*app.inner.lock().await)?.to_owned();
    app.mod_storage(move |app| {
        let (plan, _, _) = build_plan(&app, &name, &engine)?;
        Ok(Json(plan))
    })
    .await
}

async fn download_exact(
    app: &App,
    item: &SyncMod,
    engine: &str,
    client: &reqwest::Client,
    progress: &ProgressGuard,
    completed: usize,
    total: usize,
) -> Result<tempfile::NamedTempFile> {
    let (username, token) = credentials(app)?
        .ok_or_else(|| Error::bad("Configure Factorio username/token to download missing mods"))?;
    let releases = releases(client, &item.name).await?;
    let release = releases.iter().find(|release| release.version == item.version && family(&release.info_json.factorio_version) == family(engine))
        .ok_or_else(|| Error::bad(&format!("{} {} is not available for this game version on the mod portal. Upload that exact ZIP to sync this save.", item.name, item.version)))?;
    let mut url = url::Url::parse("https://mods.factorio.com")
        .map_err(|_| Error::bad("Invalid mod portal URL"))?;
    if !release.download_url.starts_with("/download/")
        || release.download_url.contains(['?', '#', '\\'])
        || release.download_url.contains("..")
    {
        return Err(Error::bad("Invalid mod portal download path"));
    }
    url.set_path(&release.download_url);
    url.query_pairs_mut()
        .append_pair("username", &username)
        .append_pair("token", &token);
    progress.update(
        SyncPhase::Downloading,
        completed,
        total,
        Some(item.name.clone()),
    )?;
    let response = client
        .get(url)
        .send()
        .await
        .map_err(|_| Error::bad("Could not download the required mod from Factorio"))?;
    workflows::download_response(
        response,
        release.clone(),
        item.name.clone(),
        app.directory.clone(),
    )
    .await
}

fn candidate_list(plan: &SyncPlan, old: &ModList, inventory: &[InstalledMod]) -> ModList {
    let mut desired = old.clone();
    for entry in &mut desired.mods {
        entry.enabled = false;
    }
    for local in inventory {
        if !desired.mods.iter().any(|entry| entry.name == local.name) {
            set_entry(&mut desired, &local.name, false, &local.version);
        }
    }
    for name in &plan.disable {
        if !desired.mods.iter().any(|entry| &entry.name == name) {
            desired.mods.push(ModListEntry {
                name: name.clone(),
                enabled: false,
                version: None,
                other: BTreeMap::new(),
            });
        }
    }
    for item in &plan.mods {
        set_entry(&mut desired, &item.name, true, &item.version);
        if matches!(item.action, SyncAction::Bundled) {
            for entry in &mut desired.mods {
                if entry.name == item.name {
                    entry.version = None;
                }
            }
        }
    }
    desired
}

#[utoipa::path(post, path="/api/saves/{name}/mods/sync", params(("name"=String, Path)), request_body=SyncRequest, responses((status=200, body=ModsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn apply(
    State(app): State<App>,
    Path(name): Path<String>,
    input: std::result::Result<Json<SyncRequest>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    let mut progress = ProgressGuard::start(&app, &name, input.operation_id.clone())?;
    let result = apply_impl(&app, &name, input, &progress).await;
    progress.finish(result.as_ref().err().map(|error| error.message.clone()));
    result
}

async fn apply_impl(
    app: &App,
    name: &str,
    input: SyncRequest,
    progress: &ProgressGuard,
) -> Result<Json<ModsDocument>> {
    let guard = app.mod_operations.clone().lock_owned().await;
    let engine = {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        known_engine(&inner)?.to_owned()
    };
    let snapshot = app.clone();
    let save = name.to_owned();
    let (plan, storage_guard) = blocking(move || {
        let guard = guard;
        workflows::ensure_consistent(&snapshot)?;
        let (plan, _, _) = build_plan(&snapshot, &save, &engine)?;
        if plan.revision != input.revision {
            return Err(Error::conflict(
                "Save or mods changed. Review the sync plan again",
            ));
        }
        if !plan.blockers.is_empty() {
            return Err(Error::bad(&plan.blockers.join(" ")));
        }
        Ok((plan, guard))
    })
    .await?;
    let directory = directory(app)?;
    let staging = tempfile::Builder::new()
        .prefix(".sync-")
        .tempdir_in(&app.directory)?;
    let client = client()?;
    let total = plan
        .mods
        .iter()
        .filter(|item| matches!(item.action, SyncAction::Download))
        .count();
    let mut completed = 0;
    for item in &plan.mods {
        if matches!(item.action, SyncAction::Download) {
            progress.update(
                SyncPhase::Resolving,
                completed,
                total,
                Some(item.name.clone()),
            )?;
            let file = download_exact(
                app,
                item,
                &plan.engine_version,
                &client,
                progress,
                completed,
                total,
            )
            .await
            .map_err(|error| Error {
                status: error.status,
                message: format!("{} {}: {}", item.name, item.version, error.message),
            })?;
            let target = staging
                .path()
                .join(format!("{}_{}.zip", item.name, item.version));
            blocking(move || copy_archive(file.path(), &target)).await?;
            completed += 1;
            progress.update(SyncPhase::Downloading, completed, total, None)?;
        }
    }
    progress.update(SyncPhase::Validating, completed, total, None)?;
    progress.update(SyncPhase::Applying, completed, total, None)?;
    let app = app.clone();
    let name = name.to_owned();
    blocking(move || {
        let _guard = storage_guard;
        workflows::ensure_consistent(&app)?;
        let (fresh, mut archives, old) = build_plan(&app, &name, &plan.engine_version)?;
        if fresh.revision != plan.revision {
            return Err(Error::conflict(
                "Save or mods changed during download. Review the sync plan again",
            ));
        }
        let mut candidates = Vec::new();
        for item in &plan.mods {
            if matches!(item.action, SyncAction::Bundled) {
                continue;
            }
            if let Some(local) = archives
                .iter()
                .find(|local| local.name == item.name && local.version == item.version)
            {
                workflows::archive_info(&directory.join(&local.file_name))?;
                let mut local = local.clone();
                local.enabled = true;
                candidates.push(local);
            } else {
                let file_name = format!("{}_{}.zip", item.name, item.version);
                let (info, size_bytes) = workflows::archive_info(&staging.path().join(&file_name))?;
                let local = InstalledMod {
                    name: info.name,
                    version: info.version,
                    title: info.title,
                    author: info.author,
                    description: info.description,
                    factorio_version: info.factorio_version,
                    dependencies: info.dependencies,
                    enabled: true,
                    file_name,
                    size_bytes,
                };
                archives.push(local.clone());
                candidates.push(local);
            }
        }
        let desired = candidate_list(&plan, &old, &archives);
        if desired.mods.len() > MAX_MODS {
            return Err(Error::bad("Sync would exceed the supported mod-list size"));
        }
        check_enabled_compatibility(&candidates, &plan.engine_version)?;
        check_dependencies(
            &candidates,
            &desired,
            Some(&plan.engine_version),
            &bundled_mods(&app)?,
        )?;
        let staged: Vec<_> = plan
            .mods
            .iter()
            .filter(|item| matches!(item.action, SyncAction::Download))
            .map(|item| format!("{}_{}.zip", item.name, item.version))
            .collect();
        workflows::begin_transaction(&app, staged.clone(), Vec::new())?;
        let publish = (|| {
            for file_name in staged {
                copy_archive(
                    &staging.path().join(&file_name),
                    &directory.join(&file_name),
                )?;
            }
            workflows::write_activation(&app, &desired, None)
        })();
        if let Err(error) = publish {
            workflows::ensure_consistent(&app)?;
            return Err(error);
        }
        Ok(Json(list(&app)?))
    })
    .await
}

#[cfg(test)]
mod tests {
    use super::*;
    use axum::{
        body::{Body, to_bytes},
        http::Request,
    };
    use std::io::Write;
    use tower::ServiceExt;
    type TestResult = std::result::Result<(), Box<dyn std::error::Error>>;
    fn err(error: Error) -> std::io::Error {
        std::io::Error::other(error.message)
    }
    fn save(app: &App, mods: &[(&str, [u8; 3])]) -> TestResult {
        // Metadata prefix from a Factorio 2.1.23 freeplay save.
        let mut bytes = b"\x02\x00\x01\x00\x17\x00\x02\x00\x00\x00\x08freeplay\x04base\x01\x00\x00\x00\x00\x00\x00\x01\x02\x01\x17\xc6\x56\x01\x00\x01\x00\x00\xa0\x00".to_vec();
        bytes.push(u8::try_from(mods.len() + 1)?);
        for (name, version) in std::iter::once(("base", [2, 1, 23])).chain(mods.iter().copied()) {
            bytes.push(u8::try_from(name.len())?);
            bytes.extend(name.as_bytes());
            bytes.extend(version);
            bytes.extend([0; 4]);
        }
        let mut writer = zip::ZipWriter::new(fs::File::create(app.saves_path().join("world.zip"))?);
        writer.start_file(
            "world/level-init.dat",
            zip::write::SimpleFileOptions::default(),
        )?;
        writer.write_all(&bytes)?;
        writer.finish()?;
        Ok(())
    }
    fn archive(app: &App, name: &str, version: &str, dependencies: &[&str]) -> TestResult {
        let folder = directory(app).map_err(err)?;
        let mut writer = zip::ZipWriter::new(fs::File::create(
            folder.join(format!("{name}_{version}.zip")),
        )?);
        writer.start_file(
            format!("{name}_{version}/info.json"),
            zip::write::SimpleFileOptions::default(),
        )?;
        writer.write_all(&serde_json::to_vec(&serde_json::json!({"name":name,"version":version,"title":name,"author":"Fixture","factorio_version":"2.1","dependencies":dependencies}))?)?;
        writer.finish()?;
        Ok(())
    }
    async fn setup() -> std::result::Result<(tempfile::TempDir, App), Box<dyn std::error::Error>> {
        let temp = tempfile::tempdir()?;
        let app = App::new(temp.path().into(), None).map_err(err)?;
        app.inner.lock().await.status.version = Some("2.1.23".into());
        Ok((temp, app))
    }
    #[tokio::test]
    async fn sync_switches_exact_local_version_and_disables_extras_via_json_api() -> TestResult {
        let (_temp, app) = setup().await?;
        save(&app, &[("demo", [1, 0, 0])])?;
        archive(&app, "demo", "1.0.0", &["base"])?;
        archive(&app, "demo", "2.0.0", &["base"])?;
        archive(&app, "extra", "1.0.0", &["base"])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert!(
            plan.mods
                .iter()
                .any(|item| item.name == "demo" && matches!(item.action, SyncAction::Switch))
        );
        assert_eq!(plan.disable, vec!["extra"]);
        workflows::begin_transaction(&app, Vec::new(), Vec::new()).map_err(err)?;
        let response = crate::test_support::authenticated(
            crate::api::router(app.clone(), &_temp.path().join("static")),
            &app.directory,
        )
        .await?
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/api/saves/world.zip/mods/sync")
                .header("host", "localhost:9400")
                .header("content-type", "application/json")
                .body(Body::from(serde_json::to_vec(
                    &serde_json::json!({"revision":plan.revision}),
                )?))?,
        )
        .await?;
        let status = response.status();
        let bytes = to_bytes(response.into_body(), 1024 * 1024).await?;
        assert_eq!(
            status,
            StatusCode::OK,
            "{}",
            String::from_utf8_lossy(&bytes)
        );
        let result: ModsDocument = serde_json::from_slice(&bytes)?;
        assert!(
            result
                .mods
                .iter()
                .any(|item| item.name == "demo" && item.version == "1.0.0" && item.enabled)
        );
        assert!(
            result
                .mods
                .iter()
                .any(|item| item.name == "extra" && !item.enabled)
        );
        assert!(
            directory(&app)
                .map_err(err)?
                .join("demo_2.0.0.zip")
                .exists()
        );
        validate_start(&app, "2.1.23").map_err(err)?;
        Ok(())
    }
    #[tokio::test]
    async fn missing_download_credentials_and_stale_plans_preserve_mods() -> TestResult {
        let (_temp, app) = setup().await?;
        save(&app, &[("missing", [1, 0, 0]), ("another", [1, 0, 0])])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.blockers[0].starts_with("2 mods require downloads."));
        assert!(
            apply(
                State(app.clone()),
                Path("world.zip".into()),
                Ok(Json(SyncRequest {
                    revision: plan.revision,
                    operation_id: None,
                }))
            )
            .await
            .is_err()
        );
        assert!(!directory(&app).map_err(err)?.join("mod-list.json").exists());
        save(&app, &[])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        archive(&app, "extra", "1.0.0", &["base"])?;
        let result = apply(
            State(app.clone()),
            Path("world.zip".into()),
            Ok(Json(SyncRequest {
                revision: plan.revision,
                operation_id: None,
            })),
        )
        .await;
        assert!(matches!(
            result,
            Err(Error {
                status: StatusCode::CONFLICT,
                ..
            })
        ));
        assert!(!directory(&app).map_err(err)?.join("mod-list.json").exists());
        Ok(())
    }
    #[tokio::test]
    async fn credential_warning_identifies_missing_field_and_clears_after_save() -> TestResult {
        let (_temp, app) = setup().await?;
        save(&app, &[("missing", [1, 0, 0])])?;
        fs::write(app.settings_path(), br#"{"token":"fixture-token"}"#)?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert_eq!(plan.blockers.len(), 1);
        assert!(plan.blockers[0].contains("Your token is saved. Add your Factorio.com username"));
        assert!(!plan.blockers[0].contains("fixture-token"));
        fs::write(app.settings_path(), br#"{"username":"fixture-user"}"#)?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert!(plan.blockers[0].contains("Your username is saved. Add your Factorio.com token"));
        fs::write(
            app.settings_path(),
            br#"{"username":"fixture-user","token":"fixture-token"}"#,
        )?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert!(plan.blockers.is_empty());
        Ok(())
    }
    #[tokio::test]
    async fn incompatible_dependencies_and_running_server_block_activation() -> TestResult {
        let (_temp, app) = setup().await?;
        save(&app, &[("demo", [1, 0, 0])])?;
        archive(&app, "demo", "1.0.0", &["missing"])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        assert!(
            apply(
                State(app.clone()),
                Path("world.zip".into()),
                Ok(Json(SyncRequest {
                    revision: plan.revision,
                    operation_id: None,
                }))
            )
            .await
            .is_err()
        );
        assert!(!directory(&app).map_err(err)?.join("mod-list.json").exists());
        save(&app, &[])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        app.inner.lock().await.status.state = ServerState::Running;
        let result = apply(
            State(app.clone()),
            Path("world.zip".into()),
            Ok(Json(SyncRequest {
                revision: plan.revision,
                operation_id: None,
            })),
        )
        .await;
        assert!(matches!(
            result,
            Err(Error {
                status: StatusCode::CONFLICT,
                ..
            })
        ));
        Ok(())
    }
    #[tokio::test]
    async fn progress_reports_success_and_only_matches_the_requested_save() -> TestResult {
        let (temp, app) = setup().await?;
        save(&app, &[])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        let result = apply(
            State(app.clone()),
            Path("world.zip".into()),
            Ok(Json(SyncRequest {
                revision: plan.revision,
                operation_id: Some("fixture-operation".into()),
            })),
        )
        .await
        .map_err(err)?;
        assert!(result.0.mods.is_empty());
        let response = crate::test_support::authenticated(
            crate::api::router(app.clone(), &temp.path().join("static")),
            &app.directory,
        )
        .await?
        .oneshot(
            Request::builder()
                .uri("/api/saves/world.zip/mods/sync")
                .header("host", "localhost:9400")
                .body(Body::empty())?,
        )
        .await?;
        assert_eq!(response.status(), StatusCode::OK);
        let state: Option<SyncProgress> =
            serde_json::from_slice(&to_bytes(response.into_body(), 4096).await?)?;
        let state = state.ok_or("Missing completed progress")?;
        assert_eq!(state.operation_id, "fixture-operation");
        assert_eq!(state.phase, SyncPhase::Complete);
        assert_eq!((state.completed, state.total), (0, 0));
        assert!(state.error.is_none());
        assert!(
            progress(State(app), Path("other.zip".into()))
                .await
                .map_err(err)?
                .0
                .is_none()
        );
        Ok(())
    }

    #[tokio::test]
    async fn canceled_sync_releases_active_operation_and_records_failure() -> TestResult {
        let (_temp, app) = setup().await?;
        let inner = app.inner.lock().await;
        let running_app = app.clone();
        let operation = tokio::spawn(async move {
            apply(
                State(running_app),
                Path("world.zip".into()),
                Ok(Json(SyncRequest {
                    revision: "fixture-revision".into(),
                    operation_id: Some("canceled-operation".into()),
                })),
            )
            .await
        });
        tokio::time::timeout(Duration::from_secs(1), async {
            loop {
                if progress(State(app.clone()), Path("world.zip".into()))
                    .await
                    .map_err(err)?
                    .0
                    .is_some()
                {
                    return Ok::<(), std::io::Error>(());
                }
                tokio::task::yield_now().await;
            }
        })
        .await??;
        assert!(matches!(
            ProgressGuard::start(&app, "other.zip", Some("another-operation".into())),
            Err(Error {
                status: StatusCode::CONFLICT,
                ..
            })
        ));
        operation.abort();
        assert!(operation.await.is_err_and(|error| error.is_cancelled()));
        drop(inner);
        let state = progress(State(app.clone()), Path("world.zip".into()))
            .await
            .map_err(err)?
            .0
            .ok_or("Missing failure progress")?;
        assert_eq!(state.phase, SyncPhase::Failed);
        assert!(
            state
                .error
                .is_some_and(|error| error.contains("interrupted"))
        );
        let mut retry =
            ProgressGuard::start(&app, "other.zip", Some("retry-operation".into())).map_err(err)?;
        retry.finish(None);
        Ok(())
    }

    #[tokio::test]
    async fn failed_validation_is_reported_and_invalid_ids_are_rejected() -> TestResult {
        let (_temp, app) = setup().await?;
        for invalid in ["", "has spaces", "invalid/operation", &"x".repeat(65)] {
            assert!(ProgressGuard::start(&app, "world.zip", Some(invalid.into())).is_err());
        }
        save(&app, &[("demo", [1, 0, 0])])?;
        archive(&app, "demo", "1.0.0", &["missing"])?;
        let (plan, _, _) = build_plan(&app, "world.zip", "2.1.23").map_err(err)?;
        let result = apply(
            State(app.clone()),
            Path("world.zip".into()),
            Ok(Json(SyncRequest {
                revision: plan.revision,
                operation_id: Some("validation-failure".into()),
            })),
        )
        .await;
        let error = result.err().ok_or("Expected dependency failure")?;
        let state = progress(State(app.clone()), Path("world.zip".into()))
            .await
            .map_err(err)?
            .0
            .ok_or("Missing failure progress")?;
        assert_eq!(state.phase, SyncPhase::Failed);
        assert_eq!(state.error.as_deref(), Some(error.message.as_str()));
        assert!(!directory(&app).map_err(err)?.join("mod-list.json").exists());
        Ok(())
    }

    #[tokio::test]
    async fn active_operation_cannot_be_overwritten_by_another_guard() -> TestResult {
        let (_temp, app) = setup().await?;
        let mut active =
            ProgressGuard::start(&app, "world.zip", Some("first-operation".into())).map_err(err)?;
        active
            .update(SyncPhase::Downloading, 1, 3, Some("demo".into()))
            .map_err(err)?;
        assert!(ProgressGuard::start(&app, "other.zip", Some("second-operation".into())).is_err());
        let state = progress(State(app.clone()), Path("world.zip".into()))
            .await
            .map_err(err)?
            .0
            .ok_or("Missing active progress")?;
        assert_eq!(state.operation_id, "first-operation");
        assert_eq!(state.phase, SyncPhase::Downloading);
        assert_eq!((state.completed, state.total), (1, 3));
        assert_eq!(state.current_mod.as_deref(), Some("demo"));
        active.finish(None);
        Ok(())
    }
}
