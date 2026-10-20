use crate::{Error, Result, models::*};
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
    time::UNIX_EPOCH,
};

pub const MAX_SETTINGS_BYTES: u64 = 1024 * 1024;

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
pub fn settings(path: &Path) -> Result<SettingsDocument> {
    let (revision, bytes) = read_document(path)?;
    let map = settings_map(bytes.as_deref())?;
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
    Ok(SettingsDocument {
        revision,
        settings: serde_json::from_value(Value::Object(visible))?,
    })
}
pub fn update_settings(path: &Path, document: SettingsDocument) -> Result<SettingsDocument> {
    if document.settings.name.trim().is_empty()
        || document.settings.name.chars().count() > 100
        || document.settings.description.chars().count() > 2000
    {
        return Err(Error::bad(
            "Provide a server name up to 100 characters and description up to 2000 characters",
        ));
    }
    if document.settings.max_players > 65535 || document.settings.autosave_interval > 1440 {
        return Err(Error::bad(
            "Player limit must be 0–65535 and autosave interval 0–1440 minutes",
        ));
    }
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
    atomic_write(path, &serde_json::to_vec_pretty(&map)?, true)?;
    settings(path)
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
    Ok(Save {
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
    let mut archive = zip::ZipArchive::new(std::io::Cursor::new(bytes))
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
