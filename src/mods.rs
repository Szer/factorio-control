#[path = "save_sync.rs"]
pub mod save_sync;
use crate::{
    App, Error, Result,
    mod_settings::{self, ModSettingsDocument, ModSettingsPatch},
    models::{ApiError, ServerState},
    storage,
};
use axum::{
    Json, Router,
    body::{Body, Bytes},
    extract::{
        DefaultBodyLimit, Path, State,
        rejection::{BytesRejection, JsonRejection},
    },
    http::{HeaderMap, StatusCode, header},
    response::{IntoResponse, Response},
    routing::{get, post, put},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    collections::{BTreeMap, HashSet},
    fs,
    io::{Cursor, Read},
    path::{Path as FsPath, PathBuf},
    time::{Duration, UNIX_EPOCH},
};
use utoipa::ToSchema;

const MAX_ARCHIVE: usize = 64 * 1024 * 1024;
const MAX_EXPANDED: u64 = 256 * 1024 * 1024;
const MAX_MODS: usize = 500;

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct InstalledMod {
    pub name: String,
    pub version: String,
    pub title: String,
    pub author: String,
    pub description: String,
    pub factorio_version: String,
    pub dependencies: Vec<String>,
    pub enabled: bool,
    pub file_name: String,
    pub size_bytes: u64,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModsDocument {
    pub revision: String,
    pub mods: Vec<InstalledMod>,
    pub portal_authenticated: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ModEnabledRequest {
    pub revision: String,
    pub enabled: bool,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ModUpdateRequest {
    pub revision: String,
    pub version: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModUpdates {
    pub name: String,
    pub installed_version: String,
    #[schema(required = true)]
    pub latest_version: Option<String>,
    pub authenticated: bool,
    pub factorio_version: String,
    #[schema(required = true)]
    pub message: Option<String>,
}
#[derive(Clone, Deserialize)]
struct ModInfo {
    name: String,
    version: String,
    title: String,
    author: String,
    #[serde(default)]
    description: String,
    #[serde(default = "default_factorio_version")]
    factorio_version: String,
    #[serde(default = "default_dependencies")]
    dependencies: Vec<String>,
}
fn default_factorio_version() -> String {
    "0.12".into()
}
fn default_dependencies() -> Vec<String> {
    vec!["base".into()]
}
#[derive(Clone, Deserialize, Serialize)]
struct ModList {
    mods: Vec<ModListEntry>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}
#[derive(Clone, Deserialize, Serialize)]
struct ModListEntry {
    name: String,
    enabled: bool,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    version: Option<String>,
    #[serde(flatten)]
    other: BTreeMap<String, serde_json::Value>,
}
impl Default for ModList {
    fn default() -> Self {
        Self {
            mods: vec![ModListEntry {
                name: "base".into(),
                enabled: true,
                version: None,
                other: BTreeMap::new(),
            }],
            other: BTreeMap::new(),
        }
    }
}
fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 100
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'-' || b == b'_')
}
fn version(value: &str) -> Result<[u32; 3]> {
    let mut parts = value.split('.');
    let mut result = [0; 3];
    for item in &mut result {
        let part = parts
            .next()
            .ok_or_else(|| Error::bad("Mod version must have three numeric components"))?;
        if part.is_empty() || !part.bytes().all(|b| b.is_ascii_digit()) {
            return Err(Error::bad("Invalid mod version"));
        }
        *item = part
            .parse()
            .map_err(|_| Error::bad("Invalid mod version"))?;
        if *item > 65535 {
            return Err(Error::bad("Mod version components must be 0–65535"));
        }
    }
    if parts.next().is_some() {
        return Err(Error::bad("Invalid mod version"));
    }
    Ok(result)
}
fn family(value: &str) -> Option<(u32, u32)> {
    let mut parts = value.split('.');
    Some((parts.next()?.parse().ok()?, parts.next()?.parse().ok()?))
}
fn protected(name: &str) -> bool {
    matches!(
        name,
        "base" | "core" | "space-age" | "quality" | "elevated-rails" | "recycler"
    )
}
fn validate_info(info: &ModInfo) -> Result<()> {
    if !valid_name(&info.name) || protected(&info.name) {
        return Err(Error::bad("Invalid or reserved mod name"));
    }
    version(&info.version)?;
    if info.title.chars().count() > 100
        || info.description.len() > 128 * 1024
        || info.dependencies.len() > 1000
        || info.dependencies.iter().any(|d| d.len() > 1024)
        || family(&info.factorio_version).is_none()
        || info.factorio_version.split('.').count() != 2
    {
        return Err(Error::bad("Invalid mod metadata"));
    }
    Ok(())
}
fn zip_error() -> Error {
    Error::bad("Mod upload is not a valid readable ZIP archive")
}
fn read_info<R: Read + std::io::Seek>(archive: &mut zip::ZipArchive<R>) -> Result<ModInfo> {
    if archive.len() > 10_000 {
        return Err(Error::bad("Mod archive has too many entries"));
    }
    let mut info_path = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| zip_error())?;
        let name = entry.name();
        let mut parts = name.split('/');
        if parts.next().is_some()
            && parts.next() == Some("info.json")
            && parts.next().is_none()
            && info_path.replace(index).is_some()
        {
            return Err(Error::bad("Mod archive contains multiple info.json files"));
        }
    }
    let mut entry = archive
        .by_index(
            info_path.ok_or_else(|| Error::bad("Mod ZIP must contain name_version/info.json"))?,
        )
        .map_err(|_| zip_error())?;
    if entry.size() > 1024 * 1024 {
        return Err(Error::bad("Mod info.json exceeds 1 MiB"));
    }
    let mut bytes = Vec::new();
    (&mut entry)
        .take(1024 * 1024 + 1)
        .read_to_end(&mut bytes)
        .map_err(|_| zip_error())?;
    if bytes.len() > 1024 * 1024 {
        return Err(Error::bad("Mod info.json exceeds 1 MiB"));
    }
    let info: ModInfo =
        serde_json::from_slice(&bytes).map_err(|_| Error::bad("Invalid mod info.json"))?;
    validate_info(&info)?;
    Ok(info)
}
fn validate_archive(bytes: &[u8]) -> Result<ModInfo> {
    if bytes.len() > MAX_ARCHIVE {
        return Err(Error::bad("Mod ZIP exceeds 64 MiB"));
    }
    let mut archive = zip::ZipArchive::new(Cursor::new(bytes)).map_err(|_| zip_error())?;
    let info = read_info(&mut archive)?;
    let prefix = archive
        .file_names()
        .find_map(|name| {
            let (root, file) = name.split_once('/')?;
            (file == "info.json").then(|| format!("{root}/"))
        })
        .ok_or_else(|| Error::bad("Mod ZIP must have one root directory containing info.json"))?;
    let mut paths = HashSet::new();
    let mut expanded = 0_u64;
    for index in 0..archive.len() {
        let mut entry = archive.by_index(index).map_err(|_| zip_error())?;
        if entry.enclosed_name().is_none()
            || entry.name().contains('\\')
            || entry
                .name()
                .split('/')
                .any(|part| matches!(part, "." | ".."))
            || !entry.name().starts_with(&prefix)
            || !paths.insert(entry.name().to_owned())
            || entry
                .unix_mode()
                .is_some_and(|mode| !matches!(mode & 0o170000, 0 | 0o100000 | 0o040000))
        {
            return Err(Error::bad(
                "Mod ZIP contains unsafe, duplicate, or out-of-root paths",
            ));
        }
        expanded = expanded
            .checked_add(entry.size())
            .ok_or_else(|| Error::bad("Mod ZIP expands beyond limit"))?;
        if expanded > MAX_EXPANDED
            || entry.size()
                > entry
                    .compressed_size()
                    .saturating_mul(250)
                    .saturating_add(1024 * 1024)
        {
            return Err(Error::bad("Mod ZIP expands beyond safe limits"));
        }
        let expected = entry.size();
        let read = std::io::copy(
            &mut (&mut entry).take(expected.saturating_add(1)),
            &mut std::io::sink(),
        )
        .map_err(|_| zip_error())?;
        if read != expected {
            return Err(zip_error());
        }
    }
    Ok(info)
}
pub fn directory(app: &App) -> Result<PathBuf> {
    let path = app.directory.join("mods");
    if !path.exists() {
        fs::create_dir(&path)?;
    }
    storage::ensure_directory(&path)?;
    Ok(path)
}
fn bundled_mods(app: &App) -> Result<BTreeMap<String, String>> {
    let mut bundled = BTreeMap::new();
    let Some(binary) = app.binary.as_ref() else {
        return Ok(bundled);
    };
    let binary = match binary.canonicalize() {
        Ok(path) => path,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(bundled),
        Err(error) => return Err(error.into()),
    };
    let Some(root) = binary
        .parent()
        .and_then(FsPath::parent)
        .and_then(FsPath::parent)
    else {
        return Ok(bundled);
    };
    let data = root.join("data");
    if !data.exists() {
        return Ok(bundled);
    }
    storage::ensure_directory(&data)?;
    #[derive(Deserialize)]
    struct Builtin {
        name: String,
        version: Option<String>,
    }
    for entry in fs::read_dir(data)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let (_, bytes) = storage::read_document(&entry.path().join("info.json"))?;
        let Some(bytes) = bytes else {
            continue;
        };
        let info: Builtin = serde_json::from_slice(&bytes)?;
        if info.name == "core" {
            continue;
        }
        if !valid_name(&info.name) {
            return Err(Error::bad("Invalid bundled mod name"));
        }
        let value = info
            .version
            .ok_or_else(|| Error::bad("Bundled mod is missing its version"))?;
        version(&value)?;
        bundled.insert(info.name, value);
    }
    Ok(bundled)
}
fn read_list(path: &FsPath) -> Result<(ModList, String)> {
    let (revision, bytes) = storage::read_document(&path.join("mod-list.json"))?;
    let list: ModList = match bytes {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => ModList::default(),
    };
    let mut names = HashSet::new();
    if list.mods.len() > MAX_MODS
        || list
            .mods
            .iter()
            .any(|entry| !valid_name(&entry.name) || !names.insert(entry.name.clone()))
    {
        return Err(Error::bad("Invalid or duplicate mod-list.json entries"));
    }
    Ok((list, revision))
}
fn credentials(app: &App) -> Result<Option<(String, String)>> {
    let username = std::env::var("FACTORIO_USERNAME")
        .ok()
        .filter(|value| !value.trim().is_empty());
    let token = std::env::var("FACTORIO_TOKEN")
        .ok()
        .filter(|value| !value.trim().is_empty());
    if let (Some(username), Some(token)) = (username, token) {
        return Ok(Some((username, token)));
    }
    let (_, bytes) = storage::read_document(&app.settings_path())?;
    let config: serde_json::Value = match bytes {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => return Ok(None),
    };
    match (
        config.get("username").and_then(serde_json::Value::as_str),
        config.get("token").and_then(serde_json::Value::as_str),
    ) {
        (Some(username), Some(token))
            if !username.trim().is_empty() && !token.trim().is_empty() =>
        {
            Ok(Some((username.into(), token.into())))
        }
        _ => Ok(None),
    }
}
pub fn list(app: &App) -> Result<ModsDocument> {
    let path = directory(app)?;
    let (mod_list, revision) = read_list(&path)?;
    let mut hash = Sha256::new();
    hash.update(revision.as_bytes());
    let mut files = Vec::new();
    for item in fs::read_dir(&path)? {
        let item = item?;
        let name = item.file_name().to_string_lossy().into_owned();
        if name.ends_with(".zip") {
            storage::ensure_regular(&item.path())?;
            files.push((name, item.path()));
        }
    }
    if files.len() > MAX_MODS * 10 {
        return Err(Error::bad("Too many mod ZIP files"));
    }
    files.sort_by(|a, b| a.0.cmp(&b.0));
    let mut selected: BTreeMap<String, InstalledMod> = BTreeMap::new();
    for (file_name, path) in files {
        let meta = fs::metadata(&path)?;
        if meta.len() > MAX_ARCHIVE as u64 {
            return Err(Error::bad("Installed mod ZIP exceeds 64 MiB"));
        }
        hash.update(file_name.as_bytes());
        hash.update(meta.len().to_le_bytes());
        hash.update(
            meta.modified()?
                .duration_since(UNIX_EPOCH)
                .map_err(|_| Error::bad("Invalid mod file timestamp"))?
                .as_nanos()
                .to_le_bytes(),
        );
        let mut archive = zip::ZipArchive::new(fs::File::open(&path)?).map_err(|_| zip_error())?;
        let info = read_info(&mut archive)?;
        let pinned = mod_list.mods.iter().find(|entry| entry.name == info.name);
        if pinned
            .and_then(|entry| entry.version.as_ref())
            .is_some_and(|target| target != &info.version)
        {
            continue;
        }
        if let Some(previous) = selected.get(&info.name)
            && version(&previous.version)? >= version(&info.version)?
        {
            continue;
        }
        selected.insert(
            info.name.clone(),
            InstalledMod {
                name: info.name,
                version: info.version,
                title: info.title,
                author: info.author,
                description: info.description,
                factorio_version: info.factorio_version,
                dependencies: info.dependencies,
                enabled: pinned.is_none_or(|entry| entry.enabled),
                file_name,
                size_bytes: meta.len(),
            },
        );
    }
    Ok(ModsDocument {
        revision: format!("{:x}", hash.finalize()),
        mods: selected.into_values().collect(),
        portal_authenticated: credentials(app)?.is_some(),
    })
}
fn stopped(inner: &crate::Inner) -> Result<()> {
    if inner.child.is_some()
        || !matches!(
            inner.status.state,
            ServerState::Stopped | ServerState::Failed
        )
    {
        return Err(Error::conflict(
            "Stop the server before changing mods or startup settings",
        ));
    }
    Ok(())
}
fn current(app: &App, revision: &str) -> Result<ModsDocument> {
    let document = list(app)?;
    if document.revision != revision {
        return Err(Error::conflict("Mods changed. Reload before saving"));
    }
    Ok(document)
}
fn set_entry(list: &mut ModList, name: &str, enabled: bool, version: &str) {
    if let Some(entry) = list.mods.iter_mut().find(|entry| entry.name == name) {
        entry.enabled = enabled;
        entry.version = Some(version.into());
    } else {
        list.mods.push(ModListEntry {
            name: name.into(),
            enabled,
            version: Some(version.into()),
            other: BTreeMap::new(),
        });
    }
}
fn check_compatibility(info: &ModInfo, engine: Option<&str>) -> Result<()> {
    if let Some(engine) = engine
        && family(engine) != family(&info.factorio_version)
    {
        return Err(Error::bad(
            "Mod targets a different Factorio major/minor version",
        ));
    }
    Ok(())
}
fn known_engine(inner: &crate::Inner) -> Result<&str> {
    inner.status.version.as_deref().filter(|value| version(value).is_ok()).ok_or_else(|| Error::conflict("Factorio binary version is unavailable. Configure a working binary and restart the manager before changing or checking mods"))
}
fn check_enabled_compatibility(mods: &[InstalledMod], engine: &str) -> Result<()> {
    version(engine)?;
    for item in mods.iter().filter(|item| item.enabled) {
        if family(&item.factorio_version) != family(engine) {
            return Err(Error::bad(&format!(
                "{} targets Factorio {}, but the configured binary is {engine}",
                item.name, item.factorio_version
            )));
        }
    }
    Ok(())
}
fn check_dependencies(
    mods: &[InstalledMod],
    mod_list: &ModList,
    engine: Option<&str>,
    bundled: &BTreeMap<String, String>,
) -> Result<()> {
    let mut edges: BTreeMap<&str, HashSet<&str>> = BTreeMap::new();
    for item in mods.iter().filter(|item| item.enabled) {
        edges.entry(&item.name).or_default();
        for dependency in &item.dependencies {
            let mut text = dependency.trim();
            let optional = text.starts_with(['?', '+']) || text.starts_with("(?)");
            let incompatible = text.starts_with('!');
            let unordered = text.starts_with('~');
            if let Some(rest) = text.strip_prefix("(?)") {
                text = rest.trim();
            } else if text.starts_with(['?', '!', '~', '+']) {
                text = text
                    .get(1..)
                    .ok_or_else(|| Error::bad("Invalid mod dependency"))?
                    .trim();
            }
            let (name, comparison) = match text.find(['<', '>', '=']) {
                Some(index) => (
                    text.get(..index)
                        .ok_or_else(|| Error::bad("Invalid mod dependency"))?
                        .trim(),
                    text.get(index..)
                        .ok_or_else(|| Error::bad("Invalid mod dependency"))?
                        .trim(),
                ),
                None => (text, ""),
            };
            if !valid_name(name) {
                return Err(Error::bad("Unsupported mod dependency syntax"));
            }
            let constraint = if comparison.is_empty() {
                None
            } else {
                let operator_len = if comparison.starts_with("<=") || comparison.starts_with(">=") {
                    2
                } else {
                    1
                };
                let operator = comparison
                    .get(..operator_len)
                    .ok_or_else(|| Error::bad("Invalid dependency comparison"))?;
                let required = comparison
                    .get(operator_len..)
                    .ok_or_else(|| Error::bad("Invalid dependency version"))?
                    .trim();
                Some((operator, version(required)?))
            };
            let installed = mods
                .iter()
                .find(|candidate| candidate.name == name && candidate.enabled);
            let builtin_enabled = mod_list
                .mods
                .iter()
                .find(|entry| entry.name == name)
                .is_none_or(|entry| entry.enabled);
            let builtin_version = bundled.get(name).map(String::as_str).or(if name == "base" {
                engine
            } else {
                None
            });
            let built_in = builtin_enabled && (builtin_version.is_some() || name == "base");
            let present = installed.is_some() || built_in;
            if incompatible && present {
                return Err(Error::bad(&format!(
                    "{} conflicts with enabled dependency {name}",
                    item.name
                )));
            }
            if incompatible || optional && !present {
                continue;
            }
            if !present {
                return Err(Error::bad(&format!(
                    "{} requires enabled mod {name}; upload and enable that dependency first",
                    item.name
                )));
            }
            if !unordered && let Some(installed) = installed {
                edges.entry(&item.name).or_default().insert(&installed.name);
            }
            if let Some((operator, required)) = constraint {
                let Some(actual) = installed
                    .map(|candidate| candidate.version.as_str())
                    .or(builtin_version)
                else {
                    continue;
                };
                let actual = version(actual)?;
                let satisfied = match operator {
                    "=" => actual == required,
                    ">=" => actual >= required,
                    ">" => actual > required,
                    "<=" => actual <= required,
                    "<" => actual < required,
                    _ => return Err(Error::bad("Unsupported mod dependency comparison")),
                };
                if !satisfied {
                    return Err(Error::bad(&format!(
                        "{} has an unsatisfied version requirement for {name}",
                        item.name
                    )));
                }
            }
        }
    }
    while !edges.is_empty() {
        let ready: Vec<&str> = edges
            .iter()
            .filter(|(_, dependencies)| dependencies.is_empty())
            .map(|(name, _)| *name)
            .collect();
        if ready.is_empty() {
            return Err(Error::bad(
                "Enabled mods contain a dependency load-order cycle",
            ));
        }
        for name in ready {
            edges.remove(name);
            for dependencies in edges.values_mut() {
                dependencies.remove(name);
            }
        }
    }
    Ok(())
}
pub fn validate_start(app: &App, engine: &str) -> Result<()> {
    let document = list(app)?;
    let (mod_list, _) = read_list(&directory(app)?)?;
    let bundled = bundled_mods(app)?;
    for entry in mod_list.mods.iter().filter(|entry| entry.enabled) {
        if !protected(&entry.name)
            && !bundled.contains_key(&entry.name)
            && !document.mods.iter().any(|item| item.name == entry.name)
        {
            return Err(Error::bad(&format!(
                "Enabled mod {} is missing its selected ZIP",
                entry.name
            )));
        }
    }
    check_enabled_compatibility(&document.mods, engine)?;
    check_dependencies(&document.mods, &mod_list, Some(engine), &bundled)
}
pub fn install(
    app: &App,
    revision: &str,
    bytes: &[u8],
    engine: Option<&str>,
) -> Result<ModsDocument> {
    let info = validate_archive(bytes)?;
    check_compatibility(&info, engine)?;
    let document = current(app, revision)?;
    let previous = document.mods.iter().find(|entry| entry.name == info.name);
    if previous.is_some_and(|previous| previous.version == info.version) {
        return Err(Error::conflict("That mod version is already installed"));
    }
    let activate = match previous {
        Some(previous) => version(&info.version)? > version(&previous.version)?,
        None => true,
    };
    let path = directory(app)?;
    let file_name = format!("{}_{}.zip", info.name, info.version);
    let target = path.join(&file_name);
    storage::ensure_regular(&target)?;
    if target.exists() {
        return Err(Error::conflict("That mod ZIP already exists on disk"));
    }
    let (mut mod_list, _) = read_list(&path)?;
    if previous.is_none() && mod_list.mods.len() >= MAX_MODS {
        return Err(Error::bad("Too many installed mods"));
    }
    let mut candidates = document.mods.clone();
    if activate {
        set_entry(
            &mut mod_list,
            &info.name,
            previous.is_none_or(|entry| entry.enabled),
            &info.version,
        );
        candidates.retain(|item| item.name != info.name);
        candidates.push(InstalledMod {
            name: info.name.clone(),
            version: info.version.clone(),
            title: info.title.clone(),
            author: info.author.clone(),
            description: info.description.clone(),
            factorio_version: info.factorio_version.clone(),
            dependencies: info.dependencies.clone(),
            enabled: previous.is_none_or(|item| item.enabled),
            file_name: file_name.clone(),
            size_bytes: bytes.len() as u64,
        });
    } else if let Some(previous) = previous {
        // An older ZIP is available for save sync without changing the active version.
        set_entry(
            &mut mod_list,
            &previous.name,
            previous.enabled,
            &previous.version,
        );
    }
    check_dependencies(&candidates, &mod_list, engine, &bundled_mods(app)?)?;
    let list_bytes = serde_json::to_vec_pretty(&mod_list)?;
    storage::atomic_write(&target, bytes, false)?;
    if let Err(error) = storage::atomic_write(&path.join("mod-list.json"), &list_bytes, true) {
        if fs::remove_file(&target).is_err() {
            return Err(Error { status: StatusCode::INTERNAL_SERVER_ERROR, message: "Could not activate mod and could not remove its uploaded ZIP; inspect mods directory before starting".into() });
        }
        return Err(error);
    }
    list(app)
}
fn json<T>(value: std::result::Result<Json<T>, JsonRejection>) -> Result<T> {
    value.map(|Json(value)| value).map_err(|e| Error {
        status: e.status(),
        message: e.body_text(),
    })
}
fn body(value: std::result::Result<Bytes, BytesRejection>) -> Result<Bytes> {
    value.map_err(|e| Error {
        status: e.status(),
        message: e.body_text(),
    })
}
fn expected(headers: &HeaderMap) -> Result<&str> {
    headers
        .get(header::IF_MATCH)
        .and_then(|value| value.to_str().ok())
        .filter(|value| !value.is_empty())
        .map(|value| value.trim_matches('"'))
        .ok_or_else(|| Error::bad("If-Match revision header is required"))
}
pub fn routes() -> Router<App> {
    Router::new()
        .route("/saves/{name}/mods", get(save_sync::plan))
        .route("/saves/{name}/mods/sync", post(save_sync::apply))
        .route("/mods", get(get_mods))
        .route(
            "/mods/upload",
            post(upload_mod).layer(DefaultBodyLimit::max(MAX_ARCHIVE)),
        )
        .route("/mods/{name}/download", get(download_mod))
        .route("/mods/{name}/enabled", put(set_enabled))
        .route("/mods/{name}/updates", get(get_updates))
        .route("/mods/{name}/update", post(update_mod))
        .route(
            "/mods/settings",
            get(get_mod_settings)
                .put(put_mod_settings)
                .layer(DefaultBodyLimit::max(mod_settings::MAX_BYTES)),
        )
        .route(
            "/mods/settings/file",
            get(download_settings)
                .post(upload_settings)
                .layer(DefaultBodyLimit::max(mod_settings::MAX_BYTES)),
        )
}
#[utoipa::path(get, path="/api/mods", responses((status=200, body=ModsDocument), (status=400, body=ApiError)))]
pub async fn get_mods(State(app): State<App>) -> Result<Json<ModsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(list(&app)?))
}
#[utoipa::path(post, path="/api/mods/upload", params(("If-Match"=String, Header)), request_body(content=String, content_type="application/zip"), responses((status=201, body=ModsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn upload_mod(
    State(app): State<App>,
    headers: HeaderMap,
    bytes: std::result::Result<Bytes, BytesRejection>,
) -> Result<(StatusCode, Json<ModsDocument>)> {
    let bytes = body(bytes)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    Ok((
        StatusCode::CREATED,
        Json(install(
            &app,
            expected(&headers)?,
            &bytes,
            Some(known_engine(&inner)?),
        )?),
    ))
}
#[utoipa::path(get, path="/api/mods/{name}/download", params(("name"=String, Path)), responses((status=200, content_type="application/zip"), (status=404, body=ApiError)))]
pub async fn download_mod(State(app): State<App>, Path(name): Path<String>) -> Result<Response> {
    let _guard = app.inner.lock().await;
    let document = list(&app)?;
    let selected = document
        .mods
        .iter()
        .find(|item| item.name == name)
        .ok_or_else(|| Error {
            status: StatusCode::NOT_FOUND,
            message: "Mod not installed".into(),
        })?;
    let path = directory(&app)?.join(&selected.file_name);
    storage::ensure_regular(&path)?;
    let file = tokio::fs::File::open(path).await?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/zip"),
            (header::CONTENT_DISPOSITION, "attachment"),
        ],
        Body::from_stream(tokio_util::io::ReaderStream::new(file)),
    )
        .into_response())
}
#[utoipa::path(put, path="/api/mods/{name}/enabled", params(("name"=String, Path)), request_body=ModEnabledRequest, responses((status=200, body=ModsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn set_enabled(
    State(app): State<App>,
    Path(name): Path<String>,
    input: std::result::Result<Json<ModEnabledRequest>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    let document = current(&app, &input.revision)?;
    let selected = document
        .mods
        .iter()
        .find(|item| item.name == name)
        .ok_or_else(|| Error::bad("Mod not installed"))?;
    let path = directory(&app)?;
    let (mut mod_list, _) = read_list(&path)?;
    set_entry(&mut mod_list, &name, input.enabled, &selected.version);
    let mut candidates = document.mods.clone();
    for candidate in &mut candidates {
        if candidate.name == name {
            candidate.enabled = input.enabled;
        }
    }
    let engine = known_engine(&inner)?;
    check_enabled_compatibility(&candidates, engine)?;
    check_dependencies(&candidates, &mod_list, Some(engine), &bundled_mods(&app)?)?;
    storage::atomic_write(
        &path.join("mod-list.json"),
        &serde_json::to_vec_pretty(&mod_list)?,
        true,
    )?;
    Ok(Json(list(&app)?))
}
#[utoipa::path(get, path="/api/mods/settings", responses((status=200, body=ModSettingsDocument), (status=400, body=ApiError)))]
pub async fn get_mod_settings(State(app): State<App>) -> Result<Json<ModSettingsDocument>> {
    let _guard = app.inner.lock().await;
    Ok(Json(mod_settings::read(
        &directory(&app)?.join("mod-settings.dat"),
    )?))
}
#[utoipa::path(put, path="/api/mods/settings", request_body=ModSettingsPatch, responses((status=200, body=ModSettingsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn put_mod_settings(
    State(app): State<App>,
    input: std::result::Result<Json<ModSettingsPatch>, JsonRejection>,
) -> Result<Json<ModSettingsDocument>> {
    let input = json(input)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    Ok(Json(mod_settings::update(
        &directory(&app)?.join("mod-settings.dat"),
        input,
    )?))
}
#[utoipa::path(get, path="/api/mods/settings/file", responses((status=200, content_type="application/octet-stream"), (status=404, body=ApiError)))]
pub async fn download_settings(State(app): State<App>) -> Result<Response> {
    let _guard = app.inner.lock().await;
    let bytes =
        mod_settings::read_bytes(&directory(&app)?.join("mod-settings.dat"))?.ok_or_else(|| {
            Error {
                status: StatusCode::NOT_FOUND,
                message: "No mod-settings.dat exists yet".into(),
            }
        })?;
    Ok((
        [
            (header::CONTENT_TYPE, "application/octet-stream"),
            (
                header::CONTENT_DISPOSITION,
                "attachment; filename=\"mod-settings.dat\"",
            ),
        ],
        bytes,
    )
        .into_response())
}
#[utoipa::path(post, path="/api/mods/settings/file", params(("If-Match"=String, Header)), request_body(content=String, content_type="application/octet-stream"), responses((status=200, body=ModSettingsDocument), (status=400, body=ApiError), (status=409, body=ApiError)))]
pub async fn upload_settings(
    State(app): State<App>,
    headers: HeaderMap,
    bytes: std::result::Result<Bytes, BytesRejection>,
) -> Result<Json<ModSettingsDocument>> {
    let bytes = body(bytes)?;
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    Ok(Json(mod_settings::replace(
        &directory(&app)?.join("mod-settings.dat"),
        expected(&headers)?,
        &bytes,
    )?))
}

#[derive(Clone, Deserialize)]
struct Release {
    version: String,
    download_url: String,
    sha1: String,
    info_json: ReleaseInfo,
}
#[derive(Clone, Deserialize)]
struct ReleaseInfo {
    factorio_version: String,
}
#[derive(Deserialize)]
struct PortalMod {
    name: String,
    releases: Vec<Release>,
}
fn client() -> Result<reqwest::Client> {
    reqwest::Client::builder()
        .timeout(Duration::from_secs(90))
        .connect_timeout(Duration::from_secs(10))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            let url = attempt.url();
            if attempt.previous().len() < 4
                && url.scheme() == "https"
                && url
                    .host_str()
                    .is_some_and(|host| host == "factorio.com" || host.ends_with(".factorio.com"))
                && url.port().is_none_or(|port| port == 443)
            {
                attempt.follow()
            } else {
                attempt.stop()
            }
        }))
        .build()
        .map_err(|_| Error::bad("Could not initialize mod portal connection"))
}
async fn response_bytes(mut response: reqwest::Response, limit: usize) -> Result<Vec<u8>> {
    if !response.status().is_success() {
        return Err(Error { status: StatusCode::BAD_GATEWAY, message: "Mod portal rejected the request. For downloads, check Factorio username/token and game ownership".into() });
    }
    if response
        .content_length()
        .is_some_and(|size| size > limit as u64)
    {
        return Err(Error::bad("Mod portal response exceeds size limit"));
    }
    let mut bytes = Vec::new();
    while let Some(chunk) = response.chunk().await.map_err(|_| Error {
        status: StatusCode::BAD_GATEWAY,
        message: "Mod portal transfer failed".into(),
    })? {
        if bytes.len().saturating_add(chunk.len()) > limit {
            return Err(Error::bad("Mod portal response exceeds size limit"));
        }
        bytes.extend(chunk);
    }
    Ok(bytes)
}
async fn releases(client: &reqwest::Client, name: &str) -> Result<Vec<Release>> {
    if !valid_name(name) {
        return Err(Error::bad("Invalid mod name"));
    }
    let response = client
        .get(format!("https://mods.factorio.com/api/mods/{name}"))
        .send()
        .await
        .map_err(|_| Error {
            status: StatusCode::BAD_GATEWAY,
            message: "Could not reach the Factorio mod portal".into(),
        })?;
    let document: PortalMod =
        serde_json::from_slice(&response_bytes(response, 4 * 1024 * 1024).await?)
            .map_err(|_| Error::bad("Invalid mod portal response"))?;
    if document.name != name || document.releases.len() > 10_000 {
        return Err(Error::bad("Unexpected mod portal response"));
    }
    Ok(document.releases)
}
fn newest<'a>(
    releases: &'a [Release],
    installed: &InstalledMod,
    engine: &str,
) -> Option<&'a Release> {
    let installed_version = version(&installed.version).ok()?;
    releases
        .iter()
        .filter(|item| {
            family(&item.info_json.factorio_version) == family(engine)
                && version(&item.version).is_ok_and(|v| v > installed_version)
        })
        .max_by_key(|item| version(&item.version).ok())
}
#[utoipa::path(get, path="/api/mods/{name}/updates", params(("name"=String, Path)), responses((status=200, body=ModUpdates), (status=400, body=ApiError), (status=409, body=ApiError), (status=502, body=ApiError)))]
pub async fn get_updates(
    State(app): State<App>,
    Path(name): Path<String>,
) -> Result<Json<ModUpdates>> {
    let (installed, engine, authenticated) = {
        let inner = app.inner.lock().await;
        let document = list(&app)?;
        let installed = document
            .mods
            .into_iter()
            .find(|item| item.name == name)
            .ok_or_else(|| Error::bad("Mod not installed"))?;
        let engine = known_engine(&inner)?.to_owned();
        (installed, engine, document.portal_authenticated)
    };
    let releases = releases(&client()?, &name).await?;
    let latest = newest(&releases, &installed, &engine).map(|release| release.version.clone());
    Ok(Json(ModUpdates {
        name,
        installed_version: installed.version,
        latest_version: latest,
        authenticated,
        factorio_version: engine,
        message: if authenticated {
            None
        } else {
            Some("Set Factorio username and token in server settings or FACTORIO_USERNAME/FACTORIO_TOKEN to download updates. ZIP upload works without portal credentials.".into())
        },
    }))
}
#[utoipa::path(post, path="/api/mods/{name}/update", params(("name"=String, Path)), request_body=ModUpdateRequest, responses((status=200, body=ModsDocument), (status=400, body=ApiError), (status=409, body=ApiError), (status=502, body=ApiError)))]
pub async fn update_mod(
    State(app): State<App>,
    Path(name): Path<String>,
    input: std::result::Result<Json<ModUpdateRequest>, JsonRejection>,
) -> Result<Json<ModsDocument>> {
    let input = json(input)?;
    let (installed, engine, username, token) = {
        let inner = app.inner.lock().await;
        stopped(&inner)?;
        let document = current(&app, &input.revision)?;
        let installed = document
            .mods
            .into_iter()
            .find(|item| item.name == name)
            .ok_or_else(|| Error::bad("Mod not installed"))?;
        let engine = known_engine(&inner)?.to_owned();
        let (username, token) = credentials(&app)?.ok_or_else(|| Error::bad("Portal downloads require Factorio username/token in server settings or FACTORIO_USERNAME/FACTORIO_TOKEN. You can upload a newer ZIP without credentials."))?;
        (installed, engine, username, token)
    };
    let client = client()?;
    let releases = releases(&client, &name).await?;
    let release = newest(&releases, &installed, &engine)
        .filter(|release| release.version == input.version)
        .ok_or_else(|| {
            Error::conflict(
                "Requested update is no longer the latest compatible release. Check updates again",
            )
        })?;
    let mut url = url::Url::parse("https://mods.factorio.com")
        .map_err(|_| Error::bad("Invalid mod portal base URL"))?;
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
    let response = client.get(url).send().await.map_err(|_| Error {
        status: StatusCode::BAD_GATEWAY,
        message: "Could not download mod from the Factorio portal".into(),
    })?;
    let bytes = response_bytes(response, MAX_ARCHIVE).await?;
    if release.sha1.to_ascii_lowercase() != format!("{:x}", sha1::Sha1::digest(&bytes)) {
        return Err(Error::bad(
            "Downloaded mod checksum does not match the portal",
        ));
    }
    let info = validate_archive(&bytes)?;
    if info.name != name || info.version != input.version {
        return Err(Error::bad(
            "Downloaded mod identity does not match the selected release",
        ));
    }
    let inner = app.inner.lock().await;
    stopped(&inner)?;
    Ok(Json(install(
        &app,
        &input.revision,
        &bytes,
        Some(known_engine(&inner)?),
    )?))
}
