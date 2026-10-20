use crate::{Error, Result, models::*};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Seek, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;
pub const MAX_SAVE_BYTES: u64 = 1024 * 1024 * 1024;

pub fn ensure_regular(path: &Path) -> Result<()> {
    match fs::symlink_metadata(path) {
        Ok(meta) if meta.file_type().is_file() => Ok(()),
        Ok(_) => Err(Error::bad(
            "Expected a regular file; symlinks are not supported",
        )),
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
        Err(e) => Err(e.into()),
    }
}
pub fn ensure_directory(path: &Path) -> Result<()> {
    let meta = fs::symlink_metadata(path)?;
    if !meta.is_dir() || meta.file_type().is_symlink() {
        return Err(Error::bad("Expected a directory without a symlink"));
    }
    Ok(())
}
pub fn read_document(path: &Path) -> Result<(String, Option<Vec<u8>>)> {
    ensure_regular(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(("missing".into(), None)),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    file.take(MAX_SETTINGS_BYTES + 1).read_to_end(&mut bytes)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(Error::bad("Configuration file exceeds 1 MiB"));
    }
    let revision = format!("{:x}", Sha256::digest(&bytes));
    Ok((revision, Some(bytes)))
}
pub fn atomic_write(path: &Path, bytes: &[u8], overwrite: bool) -> Result<()> {
    ensure_regular(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::bad("Missing parent directory"))?;
    ensure_directory(parent)?;
    let mut file = tempfile::NamedTempFile::new_in(parent)?;
    file.write_all(bytes)?;
    file.as_file().sync_all()?;
    if overwrite {
        file.persist(path).map_err(|e| Error::from(e.error))?;
    } else {
        file.persist_noclobber(path).map_err(|e| {
            if e.error.kind() == std::io::ErrorKind::AlreadyExists {
                Error::conflict("A save with that name already exists")
            } else {
                e.error.into()
            }
        })?;
    }
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}
fn settings_map(bytes: Option<&[u8]>) -> Result<Map<String, Value>> {
    match bytes {
        Some(bytes) => serde_json::from_slice::<Value>(bytes)?
            .as_object()
            .cloned()
            .ok_or_else(|| Error::bad("Settings must be a JSON object")),
        None => Ok(Map::new()),
    }
}
fn typed_settings(map: &Map<String, Value>) -> Result<ServerSettings> {
    let defaults = serde_json::to_value(ServerSettings::default())?;
    let mut visible = defaults
        .as_object()
        .cloned()
        .ok_or_else(|| Error::bad("Invalid defaults"))?;
    for (key, value) in &mut visible {
        if let Some(stored) = map.get(key) {
            if key == "visibility" {
                if let (Some(target), Some(source)) = (value.as_object_mut(), stored.as_object()) {
                    for field in ["public", "lan"] {
                        if let Some(v) = source.get(field) {
                            target.insert(field.into(), v.clone());
                        }
                    }
                } else {
                    return Err(Error::bad("visibility must be an object"));
                }
            } else {
                *value = stored.clone();
            }
        }
    }
    Ok(serde_json::from_value(Value::Object(visible))?)
}
fn validate_settings(settings: &ServerSettings) -> Result<()> {
    if settings.name.trim().is_empty()
        || settings.name.chars().count() > 100
        || settings.description.chars().count() > 2000
    {
        return Err(Error::bad(
            "Provide a server name up to 100 characters and description up to 2000 characters",
        ));
    }
    if settings.max_players > 65535 || settings.autosave_interval > 1440 {
        return Err(Error::bad(
            "Player limit must be 0–65535 and autosave interval 0–1440 minutes",
        ));
    }
    if !(6..=240).contains(&settings.max_heartbeats_per_second) {
        return Err(Error::bad(
            "Network heartbeat rate must be 6–240 per second",
        ));
    }
    if settings.minimum_segment_size > settings.maximum_segment_size {
        return Err(Error::bad(
            "Minimum segment size must not exceed maximum segment size",
        ));
    }
    Ok(())
}
pub fn settings(path: &Path) -> Result<SettingsDocument> {
    let (revision, bytes) = read_document(path)?;
    let map = settings_map(bytes.as_deref())?;
    Ok(SettingsDocument {
        revision,
        settings: typed_settings(&map)?,
    })
}
pub fn update_settings(path: &Path, document: SettingsDocument) -> Result<SettingsDocument> {
    validate_settings(&document.settings)?;
    let (revision, bytes) = read_document(path)?;
    if revision != document.revision {
        return Err(Error::conflict(
            "Settings changed on disk. Reload before saving",
        ));
    }
    let mut map = settings_map(bytes.as_deref())?;
    let incoming = serde_json::to_value(document.settings)?;
    if let Some(fields) = incoming.as_object() {
        for (key, value) in fields {
            if key == "visibility" {
                let mut visibility = map
                    .get(key)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                if let Some(fields) = value.as_object() {
                    visibility.extend(fields.clone());
                }
                map.insert(key.clone(), Value::Object(visibility));
            } else {
                map.insert(key.clone(), value.clone());
            }
        }
    }
    write_configuration(path, &map)?;
    settings(path)
}
fn write_configuration(path: &Path, value: &impl serde::Serialize) -> Result<()> {
    let bytes = serde_json::to_vec_pretty(value)?;
    if bytes.len() as u64 > MAX_SETTINGS_BYTES {
        return Err(Error::bad("Configuration file exceeds 1 MiB"));
    }
    atomic_write(path, &bytes, true)
}
pub fn raw_settings(path: &Path) -> Result<RawSettingsDocument> {
    let (revision, bytes) = read_document(path)?;
    let mut settings = serde_json::to_value(ServerSettings::default())?
        .as_object()
        .cloned()
        .ok_or_else(|| Error::bad("Invalid defaults"))?;
    for (key, value) in settings_map(bytes.as_deref())? {
        if key == "visibility"
            && let (Some(target), Some(source)) = (
                settings.get_mut(&key).and_then(Value::as_object_mut),
                value.as_object(),
            )
        {
            target.extend(source.clone());
            continue;
        }
        settings.insert(key, value);
    }
    Ok(RawSettingsDocument { revision, settings })
}
pub fn update_raw_settings(
    path: &Path,
    document: RawSettingsDocument,
) -> Result<RawSettingsDocument> {
    validate_settings(&typed_settings(&document.settings)?)?;
    let (revision, _) = read_document(path)?;
    if revision != document.revision {
        return Err(Error::conflict(
            "Settings changed on disk. Reload before saving",
        ));
    }
    write_configuration(path, &document.settings)?;
    raw_settings(path)
}
fn validate_manager_settings(settings: &ManagerSettings) -> Result<()> {
    if settings
        .game_bind_address
        .parse::<std::net::IpAddr>()
        .is_err()
        || settings.game_port == 0
    {
        return Err(Error::bad(
            "Game bind address must be an IP address and port must be 1–65535",
        ));
    }
    if !(1..=60).contains(&settings.poll_interval_seconds)
        || !(1..=1440).contains(&settings.ups_history_minutes)
    {
        return Err(Error::bad(
            "Refresh interval must be 1–60 seconds and UPS history 1–1440 minutes",
        ));
    }
    Ok(())
}
pub fn manager_settings(path: &Path) -> Result<ManagerSettingsDocument> {
    let (revision, bytes) = read_document(path)?;
    let settings = match bytes {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => ManagerSettings::default(),
    };
    validate_manager_settings(&settings)?;
    Ok(ManagerSettingsDocument { revision, settings })
}
pub fn update_manager_settings(
    path: &Path,
    document: ManagerSettingsDocument,
) -> Result<ManagerSettingsDocument> {
    validate_manager_settings(&document.settings)?;
    let (revision, _) = read_document(path)?;
    if revision != document.revision {
        return Err(Error::conflict(
            "Manager settings changed on disk. Reload before saving",
        ));
    }
    write_configuration(path, &document.settings)?;
    manager_settings(path)
}
pub fn save_selection(path: &Path) -> Result<SaveSelectionDocument> {
    let (revision, bytes) = read_document(path)?;
    let (save, load_latest) = match bytes {
        Some(bytes) => {
            let value: Value = serde_json::from_slice(&bytes)?;
            if value.is_object() {
                #[derive(serde::Deserialize)]
                #[serde(deny_unknown_fields)]
                struct Selection {
                    save: Option<String>,
                    load_latest: bool,
                }
                let selection: Selection = serde_json::from_value(value)?;
                (selection.save, selection.load_latest)
            } else {
                (serde_json::from_value(value)?, false)
            }
        }
        None => (None, false),
    };
    if load_latest && save.is_some() {
        return Err(Error::bad(
            "Latest-save mode cannot also select a fixed save",
        ));
    }
    Ok(SaveSelectionDocument {
        revision,
        save,
        load_latest,
    })
}
pub fn update_save_selection(
    path: &Path,
    saves: &Path,
    document: SaveSelectionDocument,
) -> Result<SaveSelectionDocument> {
    if document.load_latest && document.save.is_some() {
        return Err(Error::bad(
            "Latest-save mode cannot also select a fixed save",
        ));
    }
    if let Some(name) = &document.save {
        save_info(saves, name)?;
    }
    let (revision, _) = read_document(path)?;
    if revision != document.revision {
        return Err(Error::conflict(
            "Selected save changed on disk. Reload before saving",
        ));
    }
    write_configuration(
        path,
        &serde_json::json!({"save": document.save, "load_latest": document.load_latest}),
    )?;
    save_selection(path)
}
pub fn admins(path: &Path) -> Result<AdminsDocument> {
    let (revision, bytes) = read_document(path)?;
    let admins = match bytes {
        Some(bytes) => serde_json::from_slice(&bytes)?,
        None => Vec::new(),
    };
    Ok(AdminsDocument { revision, admins })
}
pub fn update_admins(path: &Path, document: AdminsDocument) -> Result<AdminsDocument> {
    if document.admins.len() > 1000 {
        return Err(Error::bad("At most 1000 administrators are supported"));
    }
    let mut names = std::collections::HashSet::new();
    for name in &document.admins {
        if name.trim() != name
            || name.is_empty()
            || name.chars().count() > 64
            || name
                .chars()
                .any(|c| c.is_control() || c.is_whitespace() || c == ',')
            || !names.insert(name.to_lowercase())
        {
            return Err(Error::bad(
                "Administrator names must be unique, nonempty, up to 64 characters, without whitespace, commas or control characters",
            ));
        }
    }
    let (revision, _) = read_document(path)?;
    if revision != document.revision {
        return Err(Error::conflict(
            "Administrators changed on disk. Reload before saving",
        ));
    }
    atomic_write(path, &serde_json::to_vec_pretty(&document.admins)?, true)?;
    admins(path)
}
pub fn save_path(directory: &Path, name: &str) -> Result<PathBuf> {
    ensure_directory(directory)?;
    if name.is_empty()
        || name.len() > 240
        || !name.to_ascii_lowercase().ends_with(".zip")
        || name.contains(['/', '\\'])
        || name.chars().any(char::is_control)
        || name.eq_ignore_ascii_case(".zip")
    {
        return Err(Error::bad("Save name must be a basename ending in .zip"));
    }
    let path = directory.join(name);
    ensure_regular(&path)?;
    Ok(path)
}
pub fn save_info(directory: &Path, name: &str) -> Result<Save> {
    let meta = fs::metadata(save_path(directory, name)?)?;
    use std::os::unix::fs::MetadataExt;
    let revision = format!(
        "{:x}",
        Sha256::digest(
            format!(
                "{}:{}:{}:{}:{}",
                meta.dev(),
                meta.ino(),
                meta.len(),
                meta.mtime(),
                meta.mtime_nsec()
            )
            .as_bytes()
        )
    );
    Ok(Save {
        revision,
        name: name.into(),
        size_bytes: meta.len(),
        modified_at: meta
            .modified()?
            .duration_since(UNIX_EPOCH)
            .map_err(|_| Error::bad("Invalid file timestamp"))?
            .as_secs(),
    })
}
pub fn saves(directory: &Path) -> Result<Vec<Save>> {
    ensure_directory(directory)?;
    let mut result = Vec::new();
    for entry in fs::read_dir(directory)? {
        let entry = entry?;
        if !entry.file_type()?.is_file() {
            continue;
        }
        let Some(name) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        if name.to_ascii_lowercase().ends_with(".zip") {
            result.push(save_info(directory, &name)?);
        }
    }
    result.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then(a.name.cmp(&b.name)));
    Ok(result)
}
pub fn validate_save(bytes: &[u8]) -> Result<()> {
    validate_save_reader(std::io::Cursor::new(bytes))
}
pub fn validate_save_reader(reader: impl Read + Seek) -> Result<()> {
    let mut archive = zip::ZipArchive::new(reader)
        .map_err(|_| Error::bad("Upload is not a readable ZIP archive"))?;
    if archive.len() > 10000 {
        return Err(Error::bad("Archive contains too many entries"));
    }
    let mut level = false;
    for i in 0..archive.len() {
        let entry = archive
            .by_index(i)
            .map_err(|_| Error::bad("Archive contains an unreadable entry"))?;
        if entry.enclosed_name().is_none() {
            return Err(Error::bad("Archive contains an unsafe entry path"));
        }
        if entry.name().ends_with("/level.dat") || entry.name().ends_with("/level.dat0") {
            level = true;
        }
    }
    if !level {
        return Err(Error::bad(
            "Archive does not contain a Factorio world (level.dat or level.dat0)",
        ));
    }
    Ok(())
}

pub fn persist_save(temporary: tempfile::NamedTempFile, path: &Path) -> Result<()> {
    ensure_regular(path)?;
    let parent = path
        .parent()
        .ok_or_else(|| Error::bad("Missing parent directory"))?;
    ensure_directory(parent)?;
    temporary.persist_noclobber(path).map_err(|error| {
        if error.error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::conflict("A save with that name already exists")
        } else {
            error.error.into()
        }
    })?;
    fs::File::open(parent)?.sync_all()?;
    Ok(())
}

pub fn log_redactions(path: &Path) -> Result<Vec<String>> {
    let (_, bytes) = read_document(path)?;
    let map = settings_map(bytes.as_deref())?;
    let mut secrets = Vec::new();
    for key in ["token", "game_password", "password", "rcon_password"] {
        if let Some(secret) = map
            .get(key)
            .and_then(Value::as_str)
            .filter(|v| !v.is_empty())
        {
            secrets.push(secret.to_owned());
            secrets.push(url::form_urlencoded::byte_serialize(secret.as_bytes()).collect());
        }
    }
    secrets.sort_by_key(|secret| std::cmp::Reverse(secret.len()));
    secrets.dedup();
    Ok(secrets)
}

pub fn selected_save(selection: &SaveSelectionDocument, directory: &Path) -> Result<String> {
    if selection.load_latest {
        let mut latest: Option<(std::time::SystemTime, String)> = None;
        for save in saves(directory)? {
            let modified = fs::metadata(save_path(directory, &save.name)?)?.modified()?;
            if latest.as_ref().is_none_or(|(time, name)| {
                modified > *time || (modified == *time && save.name < *name)
            }) {
                latest = Some((modified, save.name));
            }
        }
        return latest
            .map(|(_, name)| name)
            .ok_or_else(|| Error::bad("No saves are available"));
    }
    selection
        .save
        .clone()
        .ok_or_else(|| Error::bad("Select a save before starting the server"))
}
fn archive_root(directory: &Path) -> Result<PathBuf> {
    let root = directory.join("archived-saves");
    fs::create_dir_all(&root)?;
    ensure_directory(&root)?;
    Ok(root)
}
fn archive_directory(directory: &Path, id: &str) -> Result<PathBuf> {
    if !id.starts_with("archive-")
        || id.len() > 80
        || !id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-')
    {
        return Err(Error::bad("Invalid archive identifier"));
    }
    let path = archive_root(directory)?.join(id);
    ensure_directory(&path)?;
    Ok(path)
}
pub fn archived_saves(directory: &Path) -> Result<Vec<ArchivedSave>> {
    let mut result = Vec::new();
    for entry in fs::read_dir(archive_root(directory)?)? {
        let entry = entry?;
        if !entry.file_type()?.is_dir() {
            continue;
        }
        let Some(id) = entry.file_name().to_str().map(str::to_owned) else {
            continue;
        };
        let path = archive_directory(directory, &id)?;
        for save in saves(&path)? {
            result.push(ArchivedSave {
                id: id.clone(),
                name: save.name,
                size_bytes: save.size_bytes,
                modified_at: save.modified_at,
                revision: save.revision,
            });
        }
    }
    result.sort_by(|a, b| b.modified_at.cmp(&a.modified_at).then(a.name.cmp(&b.name)));
    Ok(result)
}
pub fn archive_save(directory: &Path, name: &str, revision: &str) -> Result<ArchivedSave> {
    let saves = directory.join("saves");
    let save = save_info(&saves, name)?;
    if save.revision != revision {
        return Err(Error::conflict(
            "Save changed on disk. Reload before archiving",
        ));
    }
    let root = archive_root(directory)?;
    let temporary = tempfile::Builder::new()
        .prefix("archive-")
        .tempdir_in(&root)?;
    let id = temporary
        .path()
        .file_name()
        .and_then(|name| name.to_str())
        .ok_or_else(|| Error::bad("Invalid archive identifier"))?
        .to_owned();
    fs::rename(save_path(&saves, name)?, temporary.path().join(name))?;
    // Retain the directory only after the save has moved successfully.
    let kept = temporary.keep();
    fs::File::open(&kept)?.sync_all()?;
    fs::File::open(&saves)?.sync_all()?;
    fs::File::open(root)?.sync_all()?;
    Ok(ArchivedSave {
        id,
        name: save.name,
        size_bytes: save.size_bytes,
        modified_at: save.modified_at,
        revision: save.revision,
    })
}
pub fn restore_save(directory: &Path, id: &str, revision: &str) -> Result<Save> {
    let archive = archive_directory(directory, id)?;
    let entries = saves(&archive)?;
    if entries.len() != 1 {
        return Err(Error::bad("Archive must contain exactly one save"));
    }
    let save = entries
        .first()
        .ok_or_else(|| Error::bad("Archive is empty"))?;
    if save.revision != revision {
        return Err(Error::conflict(
            "Archived save changed on disk. Reload before restoring",
        ));
    }
    let destination = directory.join("saves");
    let path = save_path(&destination, &save.name)?;
    let source = save_path(&archive, &save.name)?;
    fs::hard_link(&source, &path).map_err(|error| {
        if error.kind() == std::io::ErrorKind::AlreadyExists {
            Error::conflict("A save with that name already exists")
        } else {
            error.into()
        }
    })?;
    fs::File::open(&destination)?.sync_all()?;
    fs::remove_file(source)?;
    fs::remove_dir(archive)?;
    fs::File::open(archive_root(directory)?)?.sync_all()?;
    save_info(&destination, &save.name)
}
pub fn game_log(directory: &Path, settings: &Path) -> Result<GameLogDocument> {
    use std::io::{Seek, SeekFrom};
    const LIMIT: u64 = 256 * 1024;
    let path = directory.join("factorio-current.log");
    ensure_regular(&path)?;
    let mut file = match fs::File::open(path) {
        Ok(file) => file,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Ok(GameLogDocument {
                name: "factorio-current.log".into(),
                content: String::new(),
                truncated: false,
            });
        }
        Err(error) => return Err(error.into()),
    };
    let length = file.metadata()?.len();
    let truncated = length > LIMIT;
    if truncated {
        file.seek(SeekFrom::Start(length - LIMIT))?;
    }
    let mut bytes = Vec::new();
    file.take(LIMIT).read_to_end(&mut bytes)?;
    if truncated {
        bytes = bytes
            .iter()
            .position(|byte| *byte == b'\n')
            .map_or_else(Vec::new, |position| bytes[position + 1..].to_vec());
    }
    let mut content = String::from_utf8_lossy(&bytes)
        .chars()
        .filter(|character| !character.is_control() || matches!(character, '\n' | '\t'))
        .collect::<String>();
    for secret in log_redactions(settings)? {
        content = content.replace(&secret, "[redacted]");
    }
    // Authentication lines can contain expired credentials absent from current settings.
    content = content
        .lines()
        .map(|line| {
            let lower = line.to_ascii_lowercase();
            if [
                "token",
                "password",
                "authorization",
                "authenticate",
                "api-key",
                "api_key",
            ]
            .iter()
            .any(|key| lower.contains(key))
            {
                "[authentication detail redacted]"
            } else {
                line
            }
        })
        .collect::<Vec<_>>()
        .join("\n");
    Ok(GameLogDocument {
        name: "factorio-current.log".into(),
        content,
        truncated,
    })
}
