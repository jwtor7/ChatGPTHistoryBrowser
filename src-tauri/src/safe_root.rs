use std::{
    collections::HashMap,
    fs::{self, File},
    path::{Component, Path, PathBuf},
    time::UNIX_EPOCH,
};

use cap_std::{ambient_authority, fs::Dir};

use crate::{
    error::{AppResult, ErrorCode},
    models::ExportValidation,
};

const MAX_SHARDS: usize = 10_000;
const MAX_ATTACHMENT_FILES: usize = 1_000_000;
const MAX_TOTAL_JSON_BYTES: u64 = 64 * 1024 * 1024 * 1024;

#[derive(Clone)]
pub struct SafeFileEntry {
    pub name: String,
    pub size: u64,
    pub modified_nanos: u128,
}

pub struct SafeExportRoot {
    canonical: PathBuf,
    directory: Dir,
    shards: Vec<SafeFileEntry>,
    attachment_files: HashMap<String, SafeFileEntry>,
    attachment_casefold: HashMap<String, Option<String>>,
}

impl SafeExportRoot {
    pub fn select(path: &Path) -> AppResult<Self> {
        let root_metadata = fs::symlink_metadata(path).map_err(|_| ErrorCode::InvalidExport)?;
        if root_metadata.file_type().is_symlink()
            || is_reparse_point(&root_metadata)
            || !root_metadata.is_dir()
        {
            return Err(ErrorCode::PathRejected.into());
        }

        let canonical = fs::canonicalize(path).map_err(|_| ErrorCode::InvalidExport)?;
        let directory = Dir::open_ambient_dir(&canonical, ambient_authority())
            .map_err(|_| ErrorCode::InvalidExport)?;
        let mut shards = Vec::new();
        let mut attachment_files = HashMap::new();
        let entries = directory.entries().map_err(|_| ErrorCode::InvalidExport)?;

        for entry_result in entries {
            let entry = entry_result.map_err(|_| ErrorCode::InvalidExport)?;
            let Some(name) = entry.file_name().to_str().map(ToOwned::to_owned) else {
                continue;
            };
            if !is_safe_component(&name) {
                continue;
            }

            let file_type = entry.file_type().map_err(|_| ErrorCode::InvalidExport)?;
            if file_type.is_symlink() || !file_type.is_file() {
                continue;
            }
            let file = entry
                .open()
                .map(cap_std::fs::File::into_std)
                .map_err(|_| ErrorCode::InvalidExport)?;
            let metadata = file.metadata().map_err(|_| ErrorCode::InvalidExport)?;
            if has_multiple_links(&metadata)
                || opened_file_has_multiple_links(&file)?
                || is_reparse_point(&metadata)
            {
                continue;
            }

            let safe_entry = SafeFileEntry {
                name: name.clone(),
                size: metadata.len(),
                modified_nanos: modified_nanos(&metadata),
            };

            if is_conversation_shard(&name) {
                if shards.len() >= MAX_SHARDS {
                    return Err(ErrorCode::ResourceLimit.into());
                }
                shards.push(safe_entry);
            } else if is_attachment_candidate(&name) {
                if attachment_files.len() >= MAX_ATTACHMENT_FILES {
                    return Err(ErrorCode::ResourceLimit.into());
                }
                attachment_files.insert(name, safe_entry);
            }
        }

        shards.sort_by(|left, right| {
            shard_sort_key(&left.name).cmp(&shard_sort_key(&right.name))
        });
        if shards.is_empty() {
            return Err(ErrorCode::InvalidExport.into());
        }
        let total_json_bytes = shards.iter().try_fold(0_u64, |total, shard| {
            total
                .checked_add(shard.size)
                .ok_or(ErrorCode::ResourceLimit)
        })?;
        if total_json_bytes > MAX_TOTAL_JSON_BYTES {
            return Err(ErrorCode::ResourceLimit.into());
        }

        let mut attachment_casefold = HashMap::<String, Option<String>>::new();
        for name in attachment_files.keys() {
            let folded = name.to_ascii_lowercase();
            attachment_casefold
                .entry(folded)
                .and_modify(|matched| *matched = None)
                .or_insert_with(|| Some(name.clone()));
        }
        let selected = Self {
            canonical,
            directory,
            shards,
            attachment_files,
            attachment_casefold,
        };
        for shard in selected.shards() {
            let _file = selected.open_entry(shard)?;
        }
        Ok(selected)
    }

    pub fn validation(&self) -> ExportValidation {
        ExportValidation {
            supported: true,
            shard_count: self.shards.len(),
            attachment_file_count: self.attachment_files.len(),
            total_json_bytes: self.shards.iter().map(|entry| entry.size).sum(),
        }
    }

    pub fn shards(&self) -> &[SafeFileEntry] {
        &self.shards
    }

    pub fn attachment_count(&self) -> usize {
        self.attachment_files.len()
    }

    pub fn attachment_inventory_fingerprint(&self) -> String {
        let mut entries = self.attachment_files.values().collect::<Vec<_>>();
        entries.sort_by(|left, right| left.name.cmp(&right.name));
        let mut hasher = blake3::Hasher::new();
        hasher.update(b"attachment-inventory-v1\0");
        for entry in entries {
            hasher.update(&(entry.name.len() as u64).to_le_bytes());
            hasher.update(entry.name.as_bytes());
            hasher.update(&entry.size.to_le_bytes());
            hasher.update(&entry.modified_nanos.to_le_bytes());
        }
        hasher.finalize().to_hex().to_string()
    }

    pub fn attachment_entry(&self, name: &str) -> Option<&SafeFileEntry> {
        self.attachment_files.get(name)
    }

    pub fn match_attachment_name(&self, candidate: &str) -> Option<&str> {
        if let Some((name, _)) = self.attachment_files.get_key_value(candidate) {
            return Some(name);
        }
        self.attachment_casefold
            .get(&candidate.to_ascii_lowercase())
            .and_then(Option::as_deref)
    }

    pub fn open_entry(&self, entry: &SafeFileEntry) -> AppResult<File> {
        self.open_name(&entry.name, Some(entry))
    }

    pub fn open_attachment(&self, name: &str) -> AppResult<File> {
        let entry = self
            .attachment_files
            .get(name)
            .ok_or(ErrorCode::AttachmentUnavailable)?;
        self.open_name(name, Some(entry))
    }

    pub fn source_fingerprint(&self) -> Vec<(String, u64, u128)> {
        self.shards
            .iter()
            .map(|entry| (entry.name.clone(), entry.size, entry.modified_nanos))
            .chain(
                self.attachment_files
                    .values()
                    .map(|entry| (entry.name.clone(), entry.size, entry.modified_nanos)),
            )
            .collect()
    }

    pub fn remains_unchanged(&self, baseline: &[(String, u64, u128)]) -> bool {
        baseline.iter().all(|(name, size, modified)| {
            let path = self.canonical.join(name);
            let Ok(metadata) = fs::symlink_metadata(path) else {
                return false;
            };
            !metadata.file_type().is_symlink()
                && metadata.is_file()
                && metadata.len() == *size
                && modified_nanos(&metadata) == *modified
        })
    }

    pub fn cache_is_outside_root(&self, cache_path: &Path) -> bool {
        let candidate = cache_path
            .canonicalize()
            .unwrap_or_else(|_| cache_path.to_path_buf());
        !candidate.starts_with(&self.canonical) && !self.canonical.starts_with(&candidate)
    }

    pub fn write_destination_is_outside_root(&self, destination: &Path) -> bool {
        let Some(parent) = destination
            .parent()
            .and_then(|parent| fs::canonicalize(parent).ok())
        else {
            return false;
        };

        #[cfg(unix)]
        {
            use cap_std::fs::MetadataExt as CapMetadataExt;
            use std::os::unix::fs::MetadataExt as StdMetadataExt;

            let Ok(selected) = self.directory.dir_metadata() else {
                return false;
            };
            for ancestor in parent.ancestors() {
                let Ok(candidate) = fs::metadata(ancestor) else {
                    return false;
                };
                if CapMetadataExt::dev(&selected) == StdMetadataExt::dev(&candidate)
                    && CapMetadataExt::ino(&selected) == StdMetadataExt::ino(&candidate)
                {
                    return false;
                }
            }
            true
        }

        #[cfg(not(unix))]
        {
            !parent.starts_with(&self.canonical)
        }
    }

    pub fn opaque_cache_key(&self) -> String {
        blake3::hash(self.canonical.as_os_str().as_encoded_bytes())
            .to_hex()
            .to_string()
    }

    fn open_name(&self, name: &str, expected: Option<&SafeFileEntry>) -> AppResult<File> {
        if !is_safe_component(name) {
            return Err(ErrorCode::PathRejected.into());
        }

        let candidate = self.canonical.join(name);
        let before = fs::symlink_metadata(&candidate).map_err(|_| ErrorCode::PathRejected)?;
        if before.file_type().is_symlink() || !before.is_file() || has_multiple_links(&before) {
            return Err(ErrorCode::PathRejected.into());
        }

        let canonical_file =
            fs::canonicalize(&candidate).map_err(|_| ErrorCode::PathRejected)?;
        if canonical_file.parent() != Some(self.canonical.as_path()) {
            return Err(ErrorCode::PathRejected.into());
        }

        // Open relative to the already-open directory capability. This keeps later
        // resolution inside the selected root even if the ambient path is renamed
        // or replaced after selection.
        let file = self
            .directory
            .open(name)
            .map(cap_std::fs::File::into_std)
            .map_err(|_| ErrorCode::PathRejected)?;
        let after = file.metadata().map_err(|_| ErrorCode::PathRejected)?;
        if !after.is_file()
            || has_multiple_links(&after)
            || opened_file_has_multiple_links(&file)?
            || is_reparse_point(&after)
        {
            return Err(ErrorCode::PathRejected.into());
        }
        if before.len() != after.len() || modified_nanos(&before) != modified_nanos(&after) {
            return Err(ErrorCode::PathRejected.into());
        }
        if let Some(expected) = expected
            && (expected.size != after.len()
                || expected.modified_nanos != modified_nanos(&after))
        {
            return Err(ErrorCode::PathRejected.into());
        }
        Ok(file)
    }
}

fn is_conversation_shard(name: &str) -> bool {
    if name == "conversations.json" {
        return true;
    }
    let Some(middle) = name
        .strip_prefix("conversations-")
        .and_then(|value| value.strip_suffix(".json"))
    else {
        return false;
    };
    !middle.is_empty() && middle.bytes().all(|byte| byte.is_ascii_digit())
}

fn is_attachment_candidate(name: &str) -> bool {
    let lowercase = name.to_ascii_lowercase();
    lowercase.ends_with(".dat")
        && (lowercase.starts_with("file-") || lowercase.starts_with("file_"))
}

fn shard_sort_key(name: &str) -> (u64, &str) {
    if name == "conversations.json" {
        return (0, name);
    }
    let ordinal = name
        .strip_prefix("conversations-")
        .and_then(|value| value.strip_suffix(".json"))
        .and_then(|value| value.parse::<u64>().ok())
        .unwrap_or(u64::MAX);
    (ordinal.saturating_add(1), name)
}

pub fn is_safe_component(value: &str) -> bool {
    if value.is_empty()
        || value == "."
        || value == ".."
        || value.chars().any(char::is_control)
        || value.contains([
            '/', '\\', ':', '\0', '\u{2044}', '\u{2215}', '\u{29f5}', '\u{ff0f}', '\u{ff3c}',
        ])
        || value.ends_with([' ', '.'])
        || is_windows_device_name(value)
    {
        return false;
    }
    let mut components = Path::new(value).components();
    matches!(components.next(), Some(Component::Normal(_))) && components.next().is_none()
}

fn is_windows_device_name(value: &str) -> bool {
    let stem = value
        .split('.')
        .next()
        .unwrap_or_default()
        .trim_end_matches([' ', '.'])
        .to_ascii_uppercase();
    matches!(stem.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || stem
            .strip_prefix("COM")
            .or_else(|| stem.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.len() == 1 && suffix.bytes().all(|byte| matches!(byte, b'1'..=b'9'))
            })
}

fn modified_nanos(metadata: &fs::Metadata) -> u128 {
    metadata
        .modified()
        .ok()
        .and_then(|time| time.duration_since(UNIX_EPOCH).ok())
        .map_or(0, |duration| duration.as_nanos())
}

#[cfg(unix)]
fn has_multiple_links(metadata: &fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(not(unix))]
fn has_multiple_links(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn opened_file_has_multiple_links(file: &File) -> AppResult<bool> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a valid handle for the duration of this call, and the Windows API
    // initializes the complete output structure only when it reports success.
    let succeeded = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr())
    };
    if succeeded == 0 {
        return Err(ErrorCode::PathRejected.into());
    }
    // SAFETY: the successful API call above initialized the output structure.
    let information = unsafe { information.assume_init() };
    Ok(information.nNumberOfLinks > 1)
}

#[cfg(not(windows))]
fn opened_file_has_multiple_links(_file: &File) -> AppResult<bool> {
    Ok(false)
}

#[cfg(windows)]
fn is_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::{fs::OpenOptions, io::Write};

    use tempfile::TempDir;

    use super::*;

    #[test]
    fn accepts_numbered_shards_and_dat_attachments() {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("conversations-000.json"), b"[]").expect("write shard");
        fs::write(directory.path().join("file-synthetic.dat"), b"safe")
            .expect("write attachment");

        let root = SafeExportRoot::select(directory.path()).expect("valid export");
        assert_eq!(root.validation().shard_count, 1);
        assert_eq!(root.validation().attachment_file_count, 1);
    }

    #[test]
    fn rejects_a_symlinked_root() {
        let directory = TempDir::new().expect("temp directory");
        let target = directory.path().join("target");
        fs::create_dir(&target).expect("create target");
        fs::write(target.join("conversations-000.json"), b"[]").expect("write shard");
        let linked = directory.path().join("linked");

        #[cfg(unix)]
        {
            std::os::unix::fs::symlink(&target, &linked).expect("create symlink");
            assert!(SafeExportRoot::select(&linked).is_err());
        }
    }

    #[test]
    fn rejects_non_component_names() {
        for value in [
            "",
            ".",
            "..",
            "../item",
            "folder/item",
            r"folder\item",
            "C:device",
            "synthetic\u{2215}item",
            "CON.txt",
            "LPT9",
            "trailing.",
        ] {
            assert!(!is_safe_component(value));
        }
        assert!(is_safe_component("file-synthetic.dat"));
    }

    #[cfg(unix)]
    #[test]
    fn rejects_hard_linked_shards() {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        let source = directory.path().join("synthetic-source");
        fs::write(&source, b"[]").expect("write source");
        fs::hard_link(&source, export.join("conversations-000.json"))
            .expect("create hard link");
        assert!(SafeExportRoot::select(&export).is_err());
    }

    #[test]
    fn ambient_root_replacement_cannot_redirect_an_existing_capability() {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        fs::write(export.join("conversations-000.json"), b"[]").expect("write original shard");
        let root = SafeExportRoot::select(&export).expect("select root");
        let original = directory.path().join("original");
        fs::rename(&export, &original).expect("rename selected root");
        fs::create_dir(&export).expect("replace ambient root");
        fs::write(export.join("conversations-000.json"), b"[{}]")
            .expect("write replacement shard");

        assert!(root.open_entry(&root.shards()[0]).is_err());
    }

    #[test]
    fn detects_source_changes_without_reading_content() {
        let directory = TempDir::new().expect("temp directory");
        let shard = directory.path().join("conversations-000.json");
        fs::write(&shard, b"[]").expect("write shard");
        let root = SafeExportRoot::select(directory.path()).expect("valid export");
        let baseline = root.source_fingerprint();

        let mut file = OpenOptions::new()
            .append(true)
            .open(shard)
            .expect("open shard");
        file.write_all(b" ").expect("change shard");
        assert!(!root.remains_unchanged(&baseline));
    }

    #[test]
    fn write_destinations_cannot_modify_the_selected_root() {
        let directory = TempDir::new().expect("temp directory");
        let selected = directory.path().join("selected");
        fs::create_dir(&selected).expect("create selected root");
        fs::write(selected.join("conversations-000.json"), b"[]").expect("write shard");
        let root = SafeExportRoot::select(&selected).expect("valid export");

        assert!(!root.write_destination_is_outside_root(&selected.join("portable.json")));
        assert!(
            root.write_destination_is_outside_root(
                directory
                    .path()
                    .parent()
                    .expect("outside parent")
                    .join("portable.json")
                    .as_path()
            )
        );

        let moved = directory.path().join("moved");
        fs::rename(&selected, &moved).expect("move selected root");
        assert!(!root.write_destination_is_outside_root(&moved.join("portable.json")));
    }
}
