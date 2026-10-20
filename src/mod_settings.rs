use crate::{Error, Result, storage};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs, io::Read, path::Path};
use utoipa::ToSchema;

pub const MAX_BYTES: usize = 16 * 1024 * 1024;

#[derive(Clone, Debug, Deserialize, Serialize, ToSchema, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ModSettingKind {
    Boolean,
    Number,
    String,
    Integer,
    UnsignedInteger,
    Color,
    Unsupported,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModSettingComponent {
    pub name: String,
    pub kind: ModSettingKind,
    pub value: String,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModSetting {
    pub name: String,
    pub kind: ModSettingKind,
    pub value: String,
    pub components: Vec<ModSettingComponent>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
pub struct ModSettingsDocument {
    pub revision: String,
    pub available: bool,
    pub settings: Vec<ModSetting>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ModSettingChange {
    #[serde(default)]
    pub value: Option<String>,
    #[serde(default)]
    pub components: BTreeMap<String, String>,
}
#[derive(Clone, Debug, Deserialize, Serialize, ToSchema)]
#[serde(deny_unknown_fields)]
pub struct ModSettingsPatch {
    pub revision: String,
    pub changes: BTreeMap<String, ModSettingChange>,
}

struct Node {
    kind: u8,
    start: usize,
    end: usize,
    value: String,
    children: BTreeMap<String, Node>,
}
struct Parser<'a> {
    bytes: &'a [u8],
    pos: usize,
    count: usize,
}
impl<'a> Parser<'a> {
    fn take(&mut self, count: usize) -> Result<&'a [u8]> {
        let end = self
            .pos
            .checked_add(count)
            .ok_or_else(|| Error::bad("Invalid mod-settings.dat length"))?;
        let data = self
            .bytes
            .get(self.pos..end)
            .ok_or_else(|| Error::bad("Truncated mod-settings.dat"))?;
        self.pos = end;
        Ok(data)
    }
    fn byte(&mut self) -> Result<u8> {
        self.take(1)?
            .first()
            .copied()
            .ok_or_else(|| Error::bad("Truncated mod-settings.dat"))
    }
    fn u32(&mut self) -> Result<u32> {
        let bytes: [u8; 4] = self
            .take(4)?
            .try_into()
            .map_err(|_| Error::bad("Invalid number"))?;
        Ok(u32::from_le_bytes(bytes))
    }
    fn string(&mut self) -> Result<String> {
        match self.byte()? {
            1 => return Ok(String::new()),
            0 => {}
            _ => return Err(Error::bad("Invalid mod-settings.dat string flag")),
        }
        let short = self.byte()?;
        let length = if short == 255 {
            self.u32()? as usize
        } else {
            usize::from(short)
        };
        if length > MAX_BYTES {
            return Err(Error::bad("Mod setting string exceeds size limit"));
        }
        String::from_utf8(self.take(length)?.to_vec())
            .map_err(|_| Error::bad("Mod setting is not UTF-8"))
    }
    fn tree(&mut self, depth: usize) -> Result<Node> {
        self.count += 1;
        if depth > 64 || self.count > 100_000 {
            return Err(Error::bad("Mod settings tree is too complex"));
        }
        let start = self.pos;
        let kind = self.byte()?;
        if self.byte()? > 1 {
            return Err(Error::bad("Invalid mod settings property flag"));
        }
        let mut node = Node {
            kind,
            start,
            end: 0,
            value: String::new(),
            children: BTreeMap::new(),
        };
        match kind {
            0 => {}
            1 => {
                node.value = match self.byte()? {
                    0 => "false".into(),
                    1 => "true".into(),
                    _ => return Err(Error::bad("Invalid mod setting boolean")),
                }
            }
            2 | 6 | 7 => {
                let bytes: [u8; 8] = self
                    .take(8)?
                    .try_into()
                    .map_err(|_| Error::bad("Invalid mod setting number"))?;
                node.value = match kind {
                    2 => {
                        let value = f64::from_le_bytes(bytes);
                        if !value.is_finite() {
                            return Err(Error::bad("Mod setting number must be finite"));
                        }
                        value.to_string()
                    }
                    6 => i64::from_le_bytes(bytes).to_string(),
                    _ => u64::from_le_bytes(bytes).to_string(),
                };
            }
            3 => node.value = self.string()?,
            4 | 5 => {
                let count = self.u32()?;
                if count > 100_000 {
                    return Err(Error::bad("Too many mod setting properties"));
                }
                for _ in 0..count {
                    let key = self.string()?;
                    let child = self.tree(depth + 1)?;
                    if kind == 5 && node.children.insert(key, child).is_some() {
                        return Err(Error::bad("Duplicate mod setting property"));
                    }
                }
            }
            _ => return Err(Error::bad("Unsupported mod-settings.dat property type")),
        }
        node.end = self.pos;
        Ok(node)
    }
}
fn scalar_kind(kind: u8) -> ModSettingKind {
    match kind {
        1 => ModSettingKind::Boolean,
        2 => ModSettingKind::Number,
        3 => ModSettingKind::String,
        6 => ModSettingKind::Integer,
        7 => ModSettingKind::UnsignedInteger,
        _ => ModSettingKind::Unsupported,
    }
}
fn values(bytes: &[u8]) -> Result<BTreeMap<String, Node>> {
    if bytes.len() < 9 || bytes.len() > MAX_BYTES || bytes.get(8) != Some(&0) {
        return Err(Error::bad("Invalid mod-settings.dat header or size"));
    }
    let mut parser = Parser {
        bytes,
        pos: 9,
        count: 0,
    };
    let mut root = parser.tree(0)?;
    if root.kind != 5 || parser.pos != bytes.len() {
        return Err(Error::bad("Invalid mod-settings.dat root or trailing data"));
    }
    let startup = root
        .children
        .remove("startup")
        .ok_or_else(|| Error::bad("Startup settings section missing"))?;
    if startup.kind != 5 {
        return Err(Error::bad("Startup settings must be a dictionary"));
    }
    let mut values = BTreeMap::new();
    for (name, mut entry) in startup.children {
        if entry.kind != 5 {
            return Err(Error::bad("Invalid startup settings entry"));
        }
        let value = entry
            .children
            .remove("value")
            .ok_or_else(|| Error::bad("Startup settings entry has no value"))?;
        values.insert(name, value);
    }
    Ok(values)
}
fn describe(name: &str, node: &Node) -> ModSetting {
    let mut setting = ModSetting {
        name: name.into(),
        kind: scalar_kind(node.kind),
        value: node.value.clone(),
        components: Vec::new(),
    };
    if node.kind == 5 {
        for (key, child) in &node.children {
            if !matches!(key.as_str(), "r" | "g" | "b" | "a") || !matches!(child.kind, 2 | 6 | 7) {
                return setting;
            }
            setting.components.push(ModSettingComponent {
                name: key.clone(),
                kind: scalar_kind(child.kind),
                value: child.value.clone(),
            });
        }
        if ["r", "g", "b"]
            .iter()
            .all(|k| node.children.contains_key(*k))
        {
            setting.kind = ModSettingKind::Color;
        } else {
            setting.components.clear();
        }
    }
    setting
}
pub fn decode(bytes: &[u8]) -> Result<ModSettingsDocument> {
    let nodes = values(bytes)?;
    Ok(ModSettingsDocument {
        revision: format!("{:x}", Sha256::digest(bytes)),
        available: true,
        settings: nodes
            .iter()
            .map(|(name, node)| describe(name, node))
            .collect(),
    })
}
pub fn read_bytes(path: &Path) -> Result<Option<Vec<u8>>> {
    storage::ensure_regular(path)?;
    let file = match fs::File::open(path) {
        Ok(file) => file,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(e) => return Err(e.into()),
    };
    let mut bytes = Vec::new();
    file.take((MAX_BYTES + 1) as u64).read_to_end(&mut bytes)?;
    if bytes.len() > MAX_BYTES {
        return Err(Error::bad("Mod settings exceed 16 MiB"));
    }
    Ok(Some(bytes))
}
pub fn read(path: &Path) -> Result<ModSettingsDocument> {
    match read_bytes(path)? {
        Some(bytes) => decode(&bytes),
        None => Ok(ModSettingsDocument {
            revision: "missing".into(),
            available: false,
            settings: Vec::new(),
        }),
    }
}
fn encode(node: &Node, value: &str) -> Result<Vec<u8>> {
    let invalid = || Error::bad("Invalid startup setting value for its stored type");
    match node.kind {
        1 => match value {
            "true" => Ok(vec![1]),
            "false" => Ok(vec![0]),
            _ => Err(invalid()),
        },
        2 => {
            let number: f64 = value.parse().map_err(|_| invalid())?;
            if !number.is_finite() {
                return Err(invalid());
            }
            Ok(number.to_le_bytes().to_vec())
        }
        6 => Ok(value
            .parse::<i64>()
            .map_err(|_| invalid())?
            .to_le_bytes()
            .to_vec()),
        7 => Ok(value
            .parse::<u64>()
            .map_err(|_| invalid())?
            .to_le_bytes()
            .to_vec()),
        3 => {
            if value.len() > 1024 * 1024 {
                return Err(Error::bad("Startup setting string exceeds 1 MiB"));
            }
            let mut bytes = vec![0];
            if value.len() < 255 {
                bytes.push(value.len() as u8);
            } else {
                bytes.push(255);
                bytes.extend((value.len() as u32).to_le_bytes());
            }
            bytes.extend(value.as_bytes());
            Ok(bytes)
        }
        _ => Err(Error::bad(
            "Unsupported startup setting type; upload a game-generated mod-settings.dat instead",
        )),
    }
}
pub fn apply(bytes: &[u8], changes: &BTreeMap<String, ModSettingChange>) -> Result<Vec<u8>> {
    let nodes = values(bytes)?;
    let mut patches = Vec::new();
    for (name, change) in changes {
        let node = nodes
            .get(name)
            .ok_or_else(|| Error::bad("Unknown startup setting"))?;
        if node.kind == 5 {
            if change.value.is_some()
                || change.components.is_empty()
                || describe(name, node).kind != ModSettingKind::Color
            {
                return Err(Error::bad("Color setting requires color components"));
            }
            for (name, value) in &change.components {
                let child = node
                    .children
                    .get(name)
                    .ok_or_else(|| Error::bad("Unknown color component"))?;
                let number: f64 = value
                    .parse()
                    .map_err(|_| Error::bad("Color components must be between 0 and 1"))?;
                if !number.is_finite() || !(0.0..=1.0).contains(&number) {
                    return Err(Error::bad("Color components must be between 0 and 1"));
                }
                if value != &child.value {
                    patches.push((child.start + 2, child.end, encode(child, value)?));
                }
            }
        } else {
            let value = change
                .value
                .as_ref()
                .ok_or_else(|| Error::bad("Startup setting requires a value"))?;
            if !change.components.is_empty() {
                return Err(Error::bad("Scalar setting cannot have color components"));
            }
            if value != &node.value {
                patches.push((node.start + 2, node.end, encode(node, value)?));
            }
        }
    }
    patches.sort_by_key(|(start, _, _)| *start);
    let mut result = Vec::with_capacity(bytes.len());
    let mut pos = 0;
    for (start, end, replacement) in patches {
        result.extend(
            bytes
                .get(pos..start)
                .ok_or_else(|| Error::bad("Overlapping startup settings changes"))?,
        );
        result.extend(replacement);
        pos = end;
        if result.len() > MAX_BYTES {
            return Err(Error::bad("Mod settings exceed 16 MiB"));
        }
    }
    result.extend(
        bytes
            .get(pos..)
            .ok_or_else(|| Error::bad("Invalid startup setting boundary"))?,
    );
    decode(&result)?;
    Ok(result)
}
pub fn replace(path: &Path, revision: &str, bytes: &[u8]) -> Result<ModSettingsDocument> {
    let document = decode(bytes)?;
    let previous = read_bytes(path)?;
    let current_revision = previous.as_ref().map_or_else(
        || "missing".into(),
        |data| format!("{:x}", Sha256::digest(data)),
    );
    if current_revision != revision {
        return Err(Error::conflict(
            "Startup settings changed. Reload before saving",
        ));
    }
    storage::atomic_write(path, bytes, true)?;
    Ok(document)
}
pub fn update(path: &Path, patch: ModSettingsPatch) -> Result<ModSettingsDocument> {
    let bytes = read_bytes(path)?.ok_or_else(|| {
        Error::bad("Upload a game-generated mod-settings.dat to configure startup settings")
    })?;
    if format!("{:x}", Sha256::digest(&bytes)) != patch.revision {
        return Err(Error::conflict(
            "Startup settings changed. Reload before saving",
        ));
    }
    let result = apply(&bytes, &patch.changes)?;
    replace(path, &patch.revision, &result)
}
