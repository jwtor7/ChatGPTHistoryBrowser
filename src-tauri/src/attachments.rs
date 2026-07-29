use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};

use crate::{
    error::{AppResult, ErrorCode},
    models::{AttachmentStatus, PreviewKind, ProjectedAttachment},
    safe_root::{SafeExportRoot, is_safe_component},
};

pub const MAX_SIGNATURE_BYTES: usize = 64 * 1024;
pub const MAX_TEXT_PREVIEW_BYTES: u64 = 2 * 1024 * 1024;
pub const MAX_INLINE_MEDIA_BYTES: u64 = 64 * 1024 * 1024;
pub const MAX_DOWNLOAD_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MAX_IMAGE_DIMENSION: u32 = 8_192;
const MAX_IMAGE_PIXELS: u64 = 16_777_216;

#[derive(Clone)]
pub struct ResolvedAttachment {
    pub key: String,
    pub display_name: String,
    pub reference: Option<String>,
    pub source_name: Option<String>,
    pub claimed_mime: Option<String>,
    pub detected_mime: Option<String>,
    pub byte_size: Option<u64>,
    pub status: AttachmentStatus,
    pub preview_kind: PreviewKind,
}

pub struct ValidatedPreview {
    pub file: File,
    pub byte_size: u64,
    pub detected_mime: Option<String>,
    pub preview_kind: PreviewKind,
}

pub fn resolve_attachment(
    root: &SafeExportRoot,
    conversation_key: &str,
    node_id: &str,
    ordinal: usize,
    projected: &ProjectedAttachment,
) -> AppResult<ResolvedAttachment> {
    let key = attachment_key(conversation_key, node_id, ordinal);
    let source_name = match_attachment_file(root, projected);
    let Some(source_name) = source_name else {
        return Ok(ResolvedAttachment {
            key,
            display_name: projected.display_name.clone(),
            reference: projected.reference.clone(),
            source_name: None,
            claimed_mime: projected.claimed_mime.clone(),
            detected_mime: None,
            byte_size: None,
            status: AttachmentStatus::Missing,
            preview_kind: PreviewKind::Missing,
        });
    };

    let entry = root
        .attachment_entry(&source_name)
        .ok_or(ErrorCode::AttachmentUnavailable)?;
    if entry.size > MAX_DOWNLOAD_BYTES {
        return Ok(ResolvedAttachment {
            key,
            display_name: projected.display_name.clone(),
            reference: projected.reference.clone(),
            source_name: None,
            claimed_mime: projected.claimed_mime.clone(),
            detected_mime: None,
            byte_size: Some(entry.size),
            status: AttachmentStatus::Rejected,
            preview_kind: PreviewKind::Unsupported,
        });
    }

    let mut file = root.open_attachment(&source_name)?;
    let mut prefix = vec![0_u8; MAX_SIGNATURE_BYTES.min(entry.size as usize)];
    let count = file.read(&mut prefix)?;
    prefix.truncate(count);
    let (detected_mime, preview_kind) = bounded_preview_kind(&prefix, entry.size);

    Ok(ResolvedAttachment {
        key,
        display_name: projected.display_name.clone(),
        reference: projected.reference.clone(),
        source_name: Some(source_name),
        claimed_mime: projected.claimed_mime.clone(),
        detected_mime,
        byte_size: Some(entry.size),
        status: AttachmentStatus::Available,
        preview_kind,
    })
}

pub fn open_validated_preview(
    root: &SafeExportRoot,
    source_name: &str,
) -> AppResult<ValidatedPreview> {
    let entry = root
        .attachment_entry(source_name)
        .ok_or(ErrorCode::AttachmentUnavailable)?;
    let mut file = root.open_attachment(source_name)?;
    let mut prefix = vec![0_u8; MAX_SIGNATURE_BYTES.min(entry.size as usize)];
    let count = file.read(&mut prefix)?;
    prefix.truncate(count);
    let (detected_mime, preview_kind) = bounded_preview_kind(&prefix, entry.size);
    file.seek(SeekFrom::Start(0))?;
    Ok(ValidatedPreview {
        file,
        byte_size: entry.size,
        detected_mime,
        preview_kind,
    })
}

pub fn read_text_preview(root: &SafeExportRoot, source_name: &str) -> AppResult<String> {
    let entry = root
        .attachment_entry(source_name)
        .ok_or(ErrorCode::AttachmentUnavailable)?;
    if entry.size > MAX_TEXT_PREVIEW_BYTES {
        return Err(ErrorCode::UnsupportedPreview.into());
    }
    let mut file = root.open_attachment(source_name)?;
    let mut bytes = Vec::with_capacity(entry.size as usize);
    file.read_to_end(&mut bytes)?;
    if !looks_like_text(&bytes) {
        return Err(ErrorCode::UnsupportedPreview.into());
    }
    String::from_utf8(bytes).map_err(|_| ErrorCode::UnsupportedPreview.into())
}

pub fn read_range(
    root: &SafeExportRoot,
    source_name: &str,
    start: u64,
    length: usize,
) -> AppResult<Vec<u8>> {
    let entry = root
        .attachment_entry(source_name)
        .ok_or(ErrorCode::AttachmentUnavailable)?;
    let end = start
        .checked_add(length as u64)
        .ok_or(ErrorCode::InvalidRequest)?;
    if start > entry.size || end > entry.size {
        return Err(ErrorCode::InvalidRequest.into());
    }
    let mut file = root.open_attachment(source_name)?;
    file.seek(SeekFrom::Start(start))?;
    let mut bytes = vec![0_u8; length];
    file.read_exact(&mut bytes)?;
    Ok(bytes)
}

pub fn safe_download_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if matches!(character, '/' | '\\' | ':' | '"' | '\0') {
                '_'
            } else {
                character
            }
        })
        .take(240)
        .collect();
    let trimmed = cleaned.trim_matches([' ', '.']);
    if trimmed.is_empty() {
        "attachment.bin".to_string()
    } else {
        trimmed.to_string()
    }
}

fn match_attachment_file(
    root: &SafeExportRoot,
    projected: &ProjectedAttachment,
) -> Option<String> {
    let mut candidates = Vec::new();
    if let Some(reference) = projected.reference.as_deref() {
        add_reference_candidates(reference, &mut candidates);
    }
    add_reference_candidates(&projected.display_name, &mut candidates);

    for candidate in &candidates {
        if let Some(name) = root.match_attachment_name(candidate) {
            return Some(name.to_string());
        }
    }
    None
}

fn add_reference_candidates(value: &str, candidates: &mut Vec<String>) {
    let without_query = value.split(['?', '#']).next().unwrap_or_default().trim();
    let basename = without_query
        .rsplit(['/', '\\', ':'])
        .next()
        .unwrap_or_default();
    let normalized: String = basename
        .chars()
        .filter(|character| !character.is_control())
        .take(512)
        .collect();
    if normalized.is_empty() {
        return;
    }

    let mut variants = vec![normalized.clone()];
    if !normalized.to_ascii_lowercase().ends_with(".dat") {
        variants.push(format!("{normalized}.dat"));
    }
    if !normalized.starts_with("file-") && !normalized.starts_with("file_") {
        variants.push(format!("file-{normalized}.dat"));
        variants.push(format!("file_{normalized}.dat"));
    }
    for candidate in variants {
        if is_safe_component(&candidate) && !candidates.contains(&candidate) {
            candidates.push(candidate);
        }
    }
}

fn detect_preview_kind(bytes: &[u8]) -> (Option<String>, PreviewKind) {
    if let Some(kind) = infer::get(bytes) {
        let mime = kind.mime_type().to_string();
        let preview = match kind.mime_type() {
            "image/png" | "image/jpeg" if safe_image_dimensions(bytes, kind.mime_type()) => {
                PreviewKind::Image
            }
            value if value.starts_with("audio/") => PreviewKind::Audio,
            "video/mp4" | "video/webm" | "video/quicktime" | "video/mpeg" => PreviewKind::Video,
            "application/pdf" => PreviewKind::Pdf,
            _ => PreviewKind::Unsupported,
        };
        return (Some(mime), preview);
    }
    if looks_like_text(bytes) {
        return (Some("text/plain".to_string()), PreviewKind::Text);
    }
    (None, PreviewKind::Unsupported)
}

fn bounded_preview_kind(bytes: &[u8], byte_size: u64) -> (Option<String>, PreviewKind) {
    let (detected_mime, mut preview_kind) = detect_preview_kind(bytes);
    if byte_size > MAX_INLINE_MEDIA_BYTES && preview_kind != PreviewKind::Text {
        preview_kind = PreviewKind::Unsupported;
    }
    if byte_size > MAX_TEXT_PREVIEW_BYTES && preview_kind == PreviewKind::Text {
        preview_kind = PreviewKind::Unsupported;
    }
    (detected_mime, preview_kind)
}

fn safe_image_dimensions(bytes: &[u8], mime: &str) -> bool {
    let dimensions = match mime {
        "image/png" => png_dimensions(bytes),
        "image/jpeg" => jpeg_dimensions(bytes),
        _ => None,
    };
    dimensions.is_some_and(|(width, height)| {
        width > 0
            && height > 0
            && width <= MAX_IMAGE_DIMENSION
            && height <= MAX_IMAGE_DIMENSION
            && u64::from(width).saturating_mul(u64::from(height)) <= MAX_IMAGE_PIXELS
    })
}

fn png_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    const PNG_SIGNATURE: &[u8; 8] = b"\x89PNG\r\n\x1a\n";
    if bytes.len() < 24 || &bytes[..8] != PNG_SIGNATURE || &bytes[12..16] != b"IHDR" {
        return None;
    }
    Some((
        u32::from_be_bytes(bytes[16..20].try_into().ok()?),
        u32::from_be_bytes(bytes[20..24].try_into().ok()?),
    ))
}

fn jpeg_dimensions(bytes: &[u8]) -> Option<(u32, u32)> {
    if !bytes.starts_with(&[0xff, 0xd8]) {
        return None;
    }
    let mut position = 2;
    while position + 4 <= bytes.len() {
        while position < bytes.len() && bytes[position] == 0xff {
            position += 1;
        }
        let marker = *bytes.get(position)?;
        position += 1;
        if matches!(marker, 0x01 | 0xd8 | 0xd9 | 0xd0..=0xd7) {
            continue;
        }
        let length = usize::from(u16::from_be_bytes([
            *bytes.get(position)?,
            *bytes.get(position + 1)?,
        ]));
        if length < 2 || position.checked_add(length)? > bytes.len() {
            return None;
        }
        let is_start_of_frame = matches!(
            marker,
            0xc0 | 0xc1
                | 0xc2
                | 0xc3
                | 0xc5
                | 0xc6
                | 0xc7
                | 0xc9
                | 0xca
                | 0xcb
                | 0xcd
                | 0xce
                | 0xcf
        );
        if is_start_of_frame {
            if length < 7 {
                return None;
            }
            let height = u32::from(u16::from_be_bytes([
                *bytes.get(position + 3)?,
                *bytes.get(position + 4)?,
            ]));
            let width = u32::from(u16::from_be_bytes([
                *bytes.get(position + 5)?,
                *bytes.get(position + 6)?,
            ]));
            return Some((width, height));
        }
        position += length;
    }
    None
}

fn looks_like_text(bytes: &[u8]) -> bool {
    if bytes.contains(&0) || std::str::from_utf8(bytes).is_err() {
        return false;
    }
    let control_count = bytes
        .iter()
        .filter(|byte| byte.is_ascii_control() && !matches!(byte, b'\n' | b'\r' | b'\t'))
        .count();
    control_count.saturating_mul(20) <= bytes.len().max(1)
}

fn attachment_key(conversation_key: &str, node_id: &str, ordinal: usize) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"attachment-v1\0");
    hasher.update(conversation_key.as_bytes());
    hasher.update(node_id.as_bytes());
    hasher.update(&ordinal.to_le_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn synthetic_root() -> (TempDir, SafeExportRoot) {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("conversations-000.json"), b"[]").expect("write shard");
        fs::write(
            directory.path().join("file-synthetic-image.dat"),
            [
                0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'H',
                b'D', b'R', 0, 0, 0, 1, 0, 0, 0, 1,
            ],
        )
        .expect("write image");
        let root = SafeExportRoot::select(directory.path()).expect("valid root");
        (directory, root)
    }

    #[test]
    fn matches_asset_pointer_only_against_inventory() {
        let (_directory, root) = synthetic_root();
        let projected = ProjectedAttachment {
            reference: Some("file-service://file-synthetic-image".to_string()),
            display_name: "Synthetic image.png".to_string(),
            claimed_mime: Some("text/html".to_string()),
        };
        let resolved =
            resolve_attachment(&root, "conversation", "node", 0, &projected).expect("resolve");
        assert_eq!(resolved.status, AttachmentStatus::Available);
        assert_eq!(resolved.detected_mime.as_deref(), Some("image/png"));
        assert_eq!(resolved.preview_kind, PreviewKind::Image);
    }

    #[test]
    fn rejects_image_dimensions_that_exceed_the_pixel_budget() {
        let mut header = [
            0x89, b'P', b'N', b'G', b'\r', b'\n', 0x1a, b'\n', 0, 0, 0, 0, b'I', b'H', b'D',
            b'R', 0, 0, 0, 0, 0, 0, 0, 0,
        ];
        header[16..20].copy_from_slice(&MAX_IMAGE_DIMENSION.to_be_bytes());
        header[20..24].copy_from_slice(&MAX_IMAGE_DIMENSION.to_be_bytes());
        let (mime, preview) = detect_preview_kind(&header);
        assert_eq!(mime.as_deref(), Some("image/png"));
        assert_eq!(preview, PreviewKind::Unsupported);
    }

    #[test]
    fn preview_revalidates_substituted_content_from_the_streamed_handle() {
        let (directory, root) = synthetic_root();
        let path = directory.path().join("file-synthetic-image.dat");
        let original_metadata = fs::metadata(&path).expect("metadata");
        let original_modified = original_metadata.modified().expect("modified");
        let replacement = vec![0_u8; original_metadata.len() as usize];
        fs::write(&path, replacement).expect("replace attachment");
        fs::OpenOptions::new()
            .write(true)
            .open(&path)
            .expect("open attachment")
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .expect("restore timestamp");

        let validated =
            open_validated_preview(&root, "file-synthetic-image.dat").expect("revalidate");
        assert_eq!(validated.preview_kind, PreviewKind::Unsupported);
        assert_eq!(validated.detected_mime, None);
    }

    #[test]
    fn traversal_reference_never_becomes_a_path() {
        let (_directory, root) = synthetic_root();
        let projected = ProjectedAttachment {
            reference: Some("../../outside".to_string()),
            display_name: "Synthetic missing".to_string(),
            claimed_mime: None,
        };
        let resolved =
            resolve_attachment(&root, "conversation", "node", 0, &projected).expect("resolve");
        assert_eq!(resolved.status, AttachmentStatus::Missing);
    }

    #[test]
    fn download_names_remove_header_and_path_characters() {
        assert_eq!(
            safe_download_name("../Synthetic\r\nname.txt"),
            "_Syntheticname.txt"
        );
    }

    #[test]
    fn multi_gibibyte_attachment_is_rejected_without_reading_its_body() {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("conversations-000.json"), b"[]").expect("write shard");
        let sparse = fs::File::create(directory.path().join("file-synthetic-large.dat"))
            .expect("create sparse attachment");
        sparse
            .set_len(MAX_DOWNLOAD_BYTES + 1)
            .expect("size sparse attachment");
        let root = SafeExportRoot::select(directory.path()).expect("valid root");
        let projected = ProjectedAttachment {
            reference: Some("file-synthetic-large".to_string()),
            display_name: "Synthetic oversized attachment".to_string(),
            claimed_mime: Some("video/mp4".to_string()),
        };
        let resolved =
            resolve_attachment(&root, "conversation", "node", 0, &projected).expect("resolve");
        assert_eq!(resolved.status, AttachmentStatus::Rejected);
        assert_eq!(resolved.preview_kind, PreviewKind::Unsupported);
        assert!(resolved.source_name.is_none());
    }
}
