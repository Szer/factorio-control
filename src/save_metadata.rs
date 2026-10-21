use crate::{Error, Result};
use serde::{Deserialize, Serialize};
use std::{
    collections::HashSet,
    fs::File,
    io::{Read, Seek, SeekFrom},
    path::Path,
};
use utoipa::ToSchema;

const MAX_HEADER_BYTES: u64 = 1024 * 1024;
const MAX_STRING_BYTES: usize = 4096;
const MAX_MODS: u32 = 4096;
const MAX_ARCHIVE_ENTRIES: usize = 50_000;

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SaveMod {
    pub name: String,
    pub version: String,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ToSchema)]
pub struct SaveMetadata {
    pub factorio_version: String,
    pub mods: Vec<SaveMod>,
}

fn invalid() -> Error {
    Error::bad("Invalid or truncated Factorio save metadata")
}

/// Reads only the bounded metadata header; never extracts files or loads the world.
pub fn read(path: &Path) -> Result<SaveMetadata> {
    let mut source = File::open(path)?;
    let mut archive = zip::ZipArchive::new(source.try_clone()?)
        .map_err(|_| Error::bad("Invalid save ZIP archive"))?;
    if archive.len() > MAX_ARCHIVE_ENTRIES {
        return Err(Error::bad("Save ZIP contains too many files"));
    }
    validate_directory_count(
        &mut source,
        archive.central_directory_start(),
        archive.len(),
    )?;
    let mut names = HashSet::new();
    let mut header_index = None;
    for index in 0..archive.len() {
        let entry = archive.by_index(index).map_err(|_| invalid())?;
        crate::storage::validate_zip_encoding(entry.name_raw(), entry.flags().as_u16())?;
        let entry_name = entry.name().map_err(|_| invalid())?;
        if entry.enclosed_name().is_none() || !names.insert(entry_name.clone().into_owned()) {
            return Err(Error::bad("Save ZIP contains unsafe or duplicate paths"));
        }
        let Some((root, name)) = entry_name.split_once('/') else {
            continue;
        };
        if name != "level-init.dat" {
            continue;
        }
        if root.is_empty() || root == "." || root == ".." || entry.is_symlink() {
            return Err(invalid());
        }
        if header_index.replace(index).is_some() {
            return Err(Error::bad("Save ZIP contains multiple metadata headers"));
        }
    }
    let index = header_index.ok_or_else(|| {
        Error::bad("Save has no level-init.dat metadata; use a Factorio 2.0 or 2.1 save")
    })?;
    let entry = archive.by_index(index).map_err(|_| invalid())?;
    parse_header(entry.take(MAX_HEADER_BYTES))
}

// The ZIP library coalesces duplicate names; count raw directory records before trusting its index.
fn validate_directory_count(file: &mut File, offset: u64, expected: usize) -> Result<()> {
    file.seek(SeekFrom::Start(offset))?;
    for count in 0..=expected {
        let mut signature = [0; 4];
        file.read_exact(&mut signature).map_err(|_| invalid())?;
        if count == expected {
            return if signature == *b"PK\x05\x06" || signature == *b"PK\x06\x06" {
                Ok(())
            } else {
                Err(Error::bad(
                    "Save ZIP contains duplicate or invalid directory records",
                ))
            };
        }
        if signature != *b"PK\x01\x02" {
            return Err(invalid());
        }
        let mut record = [0; 42];
        file.read_exact(&mut record).map_err(|_| invalid())?;
        let name = u16::from_le_bytes([record[24], record[25]]);
        let extra = u16::from_le_bytes([record[26], record[27]]);
        let comment = u16::from_le_bytes([record[28], record[29]]);
        file.seek(SeekFrom::Current(
            i64::from(name) + i64::from(extra) + i64::from(comment),
        ))?;
    }
    Err(invalid())
}

struct Header<R: Read>(R);
impl<R: Read> Header<R> {
    fn bytes<const N: usize>(&mut self) -> Result<[u8; N]> {
        let mut value = [0; N];
        self.0.read_exact(&mut value).map_err(|_| invalid())?;
        Ok(value)
    }
    fn byte(&mut self) -> Result<u8> {
        Ok(self.bytes::<1>()?[0])
    }
    fn boolean(&mut self) -> Result<()> {
        if self.byte()? > 1 {
            return Err(invalid());
        }
        Ok(())
    }
    fn u16(&mut self) -> Result<u16> {
        Ok(u16::from_le_bytes(self.bytes()?))
    }
    fn u32(&mut self) -> Result<u32> {
        Ok(u32::from_le_bytes(self.bytes()?))
    }
    fn short(&mut self) -> Result<u16> {
        match self.byte()? {
            255 => self.u16(),
            value => Ok(u16::from(value)),
        }
    }
    fn count(&mut self) -> Result<u32> {
        match self.byte()? {
            255 => self.u32(),
            value => Ok(u32::from(value)),
        }
    }
    fn string(&mut self) -> Result<String> {
        let count = usize::try_from(self.count()?).map_err(|_| invalid())?;
        if count > MAX_STRING_BYTES {
            return Err(Error::bad(
                "Save metadata string exceeds the supported size",
            ));
        }
        let mut bytes = vec![0; count];
        self.0.read_exact(&mut bytes).map_err(|_| invalid())?;
        String::from_utf8(bytes).map_err(|_| invalid())
    }
    fn version(&mut self) -> Result<String> {
        Ok(format!(
            "{}.{}.{}",
            self.short()?,
            self.short()?,
            self.short()?
        ))
    }
}

fn parse_header(input: impl Read) -> Result<SaveMetadata> {
    let mut reader = Header(input);
    let major = reader.u16()?;
    let minor = reader.u16()?;
    let patch = reader.u16()?;
    let _save_revision = reader.u16()?;
    if major != 2 || minor > 1 {
        return Err(Error::bad(
            "Mod sync supports Factorio 2.0 and 2.1 save headers",
        ));
    }
    let factorio_version = format!("{major}.{minor}.{patch}");
    reader.boolean()?;
    let _campaign = reader.string()?;
    let _scenario = reader.string()?;
    let _scenario_mod = reader.string()?;
    let _difficulty = reader.byte()?;
    reader.boolean()?; // Scenario finished.
    reader.boolean()?; // Player won.
    let _next_level = reader.string()?;
    for _ in 0..4 {
        reader.boolean()?;
    }
    let _loaded_from = reader.version()?;
    let _loaded_from_build = reader.u32()?;
    let _allowed_commands = reader.byte()?;
    let _map_flags = reader.bytes::<4>()?;
    let count = reader.count()?;
    if count == 0 || count > MAX_MODS {
        return Err(Error::bad("Save metadata contains an invalid mod count"));
    }
    let mut mods = Vec::new();
    let mut names = HashSet::new();
    for _ in 0..count {
        let name = reader.string()?;
        if name.is_empty()
            || name.len() > 100
            || !name
                .bytes()
                .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
            || !names.insert(name.clone())
        {
            return Err(Error::bad(
                "Save metadata contains an invalid or duplicate mod name",
            ));
        }
        let version = reader.version()?;
        let _checksum = reader.u32()?;
        mods.push(SaveMod { name, version });
    }
    if !mods.iter().any(|entry| entry.name == "base") {
        return Err(Error::bad("Save metadata is missing the base mod"));
    }
    Ok(SaveMetadata {
        factorio_version,
        mods,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::{Cursor, Write};

    // Header emitted by Factorio 2.1.23 with metadata-probe 1.255.300 enabled.
    const MODDED_HEADER: &[u8] = b"\x02\x00\x01\x00\x17\x00\x02\x00\x00\x00\x08freeplay\x04base\x01\x00\x00\x00\x00\x00\x00\x01\x02\x01\x17\xc6\x56\x01\x00\x01\x00\x00\xa0\x00\x02\x04base\x02\x01\x17\x3d\x6d\x5c\xe6\x0emetadata-probe\x01\xff\xff\x00\xff\x2c\x01\x00\x00\x00\x00";

    fn archive(entries: &[(&str, &[u8])]) -> Result<tempfile::NamedTempFile> {
        let file = tempfile::NamedTempFile::new()?;
        let mut writer = zip::ZipWriter::new(file.reopen()?);
        for (name, bytes) in entries {
            writer
                .start_file(*name, zip::write::SimpleFileOptions::default())
                .map_err(|_| invalid())?;
            writer.write_all(bytes)?;
        }
        writer.finish().map_err(|_| invalid())?;
        Ok(file)
    }

    #[test]
    fn reads_real_header_with_extended_version_numbers() -> Result<()> {
        let file = archive(&[("test/level-init.dat", MODDED_HEADER)])?;
        let parsed = read(file.path())?;
        assert_eq!(parsed.factorio_version, "2.1.23");
        assert_eq!(
            parsed.mods,
            vec![
                SaveMod {
                    name: "base".into(),
                    version: "2.1.23".into()
                },
                SaveMod {
                    name: "metadata-probe".into(),
                    version: "1.255.300".into()
                },
            ]
        );
        Ok(())
    }

    #[test]
    fn every_truncated_prefix_is_rejected() {
        for end in 0..MODDED_HEADER.len() {
            assert!(
                parse_header(Cursor::new(&MODDED_HEADER[..end])).is_err(),
                "prefix {end}"
            );
        }
    }

    #[test]
    fn rejects_unsupported_version_and_oversized_strings() {
        let mut bytes = MODDED_HEADER.to_vec();
        bytes[2] = 2;
        assert!(parse_header(Cursor::new(&bytes)).is_err());
        bytes = MODDED_HEADER[..9].to_vec();
        bytes.extend([255, 255, 255, 255, 255]);
        assert!(parse_header(Cursor::new(&bytes)).is_err());
    }

    #[test]
    fn rejects_duplicate_mod_names_and_missing_base() {
        let mut bytes = MODDED_HEADER.to_vec();
        bytes[58..72].copy_from_slice(b"metadata/probe");
        assert!(parse_header(Cursor::new(bytes)).is_err());
        let mut bytes = MODDED_HEADER[..45].to_vec();
        bytes.extend_from_slice(&MODDED_HEADER[45..57]);
        bytes.extend_from_slice(&MODDED_HEADER[45..57]);
        assert!(parse_header(Cursor::new(bytes)).is_err());
        let mut bytes = MODDED_HEADER.to_vec();
        bytes[46..50].copy_from_slice(b"fake");
        assert!(parse_header(Cursor::new(bytes)).is_err());
    }

    #[test]
    fn rejects_ambiguous_or_unsafe_zip_headers() -> Result<()> {
        let file = archive(&[
            ("a/level-init.dat", MODDED_HEADER),
            ("b/level-init.dat", MODDED_HEADER),
        ])?;
        assert!(read(file.path()).is_err());
        let file = archive(&[("../level-init.dat", MODDED_HEADER)])?;
        assert!(read(file.path()).is_err());
        let file = archive(&[("test/other.dat", MODDED_HEADER)])?;
        assert!(read(file.path()).is_err());
        Ok(())
    }

    #[test]
    fn rejects_duplicate_zip_names_hidden_by_library_index() -> Result<()> {
        let file = archive(&[
            ("a/level-init.dat", MODDED_HEADER),
            ("b/level-init.dat", MODDED_HEADER),
        ])?;
        let mut bytes = std::fs::read(file.path())?;
        let needle = b"b/level-init.dat";
        for index in 0..bytes.len().saturating_sub(needle.len()) {
            if bytes.get(index..index + needle.len()) == Some(needle) {
                bytes[index] = b'a';
            }
        }
        std::fs::write(file.path(), bytes)?;
        assert!(read(file.path()).is_err());
        Ok(())
    }

    #[test]
    #[ignore = "Set FC_TEST_SAVE to a real Factorio save"]
    fn reads_real_save() -> Result<()> {
        let path =
            std::env::var("FC_TEST_SAVE").map_err(|_| Error::bad("FC_TEST_SAVE is required"))?;
        let metadata = read(Path::new(&path))?;
        assert!(metadata.factorio_version.starts_with("2."));
        assert!(metadata.mods.iter().any(|m| m.name == "base"));
        Ok(())
    }
}
