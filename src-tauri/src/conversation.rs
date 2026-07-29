use std::collections::{BTreeMap, HashMap, HashSet};

use serde_json::{Map, Value};

use crate::{
    error::{AppResult, ErrorCode},
    models::{ProjectedAttachment, ProjectedConversation, ProjectedDiagnostic, ProjectedNode},
};

const MAX_NODES_PER_CONVERSATION: usize = 50_000;
const MAX_CHILDREN_PER_NODE: usize = 10_000;
const MAX_ATTACHMENTS_PER_CONVERSATION: usize = 2_000;
const MAX_MESSAGE_TEXT_BYTES: usize = 4 * 1024 * 1024;
const MAX_TITLE_BYTES: usize = 8 * 1024;
const MAX_IDENTIFIER_BYTES: usize = 8 * 1024;
const MAX_FILENAME_BYTES: usize = 1_024;
const MAX_MIME_BYTES: usize = 256;
const MAX_ACTIVE_PATH_DEPTH: usize = 50_000;
const MAX_INTERNAL_MARKER_BYTES: usize = 16 * 1024;
const INTERNAL_MARKER_OPEN: &str = "\u{e200}";
const INTERNAL_MARKER_CLOSE: &str = "\u{e201}";
const INTERNAL_MARKER_SEPARATOR: &str = "\u{e202}";
const INTERNAL_MARKER_KINDS: [&str; 3] = ["cite", "navlist", "filecite"];

pub fn project_conversation(
    raw: &Value,
    shard_name: &str,
    ordinal: u64,
) -> AppResult<ProjectedConversation> {
    let object = raw.as_object().ok_or(ErrorCode::UnsupportedRecord)?;
    let mapping = object
        .get("mapping")
        .and_then(Value::as_object)
        .ok_or(ErrorCode::UnsupportedRecord)?;
    if mapping.len() > MAX_NODES_PER_CONVERSATION {
        return Err(ErrorCode::ResourceLimit.into());
    }

    let source_id = first_string(object, &["id", "conversation_id"], MAX_IDENTIFIER_BYTES);
    let key = opaque_conversation_key(shard_name, ordinal, source_id.as_deref());
    let mut raw_to_opaque = HashMap::with_capacity(mapping.len());
    for (map_key, node_value) in mapping {
        let raw_id = node_value
            .as_object()
            .and_then(|node| first_string(node, &["id"], MAX_IDENTIFIER_BYTES))
            .unwrap_or_else(|| truncate_utf8(map_key, MAX_IDENTIFIER_BYTES));
        raw_to_opaque.insert(raw_id.clone(), opaque_node_key(&key, &raw_id));
        raw_to_opaque
            .entry(truncate_utf8(map_key, MAX_IDENTIFIER_BYTES))
            .or_insert_with(|| opaque_node_key(&key, map_key));
    }

    let mut diagnostics = BTreeMap::new();
    let mut nodes = Vec::with_capacity(mapping.len());
    let mut attachment_count = 0_usize;
    for (map_key, node_value) in mapping {
        let Some(node) = node_value.as_object() else {
            record_diagnostic(&mut diagnostics, "INVALID_NODE");
            continue;
        };
        let raw_id = first_string(node, &["id"], MAX_IDENTIFIER_BYTES)
            .unwrap_or_else(|| truncate_utf8(map_key, MAX_IDENTIFIER_BYTES));
        let Some(node_id) = raw_to_opaque.get(&raw_id).cloned() else {
            record_diagnostic(&mut diagnostics, "MISSING_NODE_ID");
            continue;
        };
        let parent_raw = first_string(node, &["parent"], MAX_IDENTIFIER_BYTES);
        let parent_node_id = parent_raw
            .as_ref()
            .and_then(|parent| raw_to_opaque.get(parent))
            .cloned();
        if parent_raw.is_some() && parent_node_id.is_none() {
            record_diagnostic(&mut diagnostics, "MISSING_PARENT");
        }

        let child_raw_ids = node
            .get("children")
            .and_then(Value::as_array)
            .map_or(&[][..], Vec::as_slice);
        if child_raw_ids.len() > MAX_CHILDREN_PER_NODE {
            return Err(ErrorCode::ResourceLimit.into());
        }
        let mut child_node_ids = Vec::with_capacity(child_raw_ids.len());
        for child in child_raw_ids {
            let Some(raw_child) = child.as_str() else {
                record_diagnostic(&mut diagnostics, "INVALID_CHILD_REFERENCE");
                continue;
            };
            if let Some(opaque) = raw_to_opaque.get(raw_child) {
                child_node_ids.push(opaque.clone());
            } else {
                record_diagnostic(&mut diagnostics, "MISSING_CHILD");
            }
        }

        let message = node.get("message").and_then(Value::as_object);
        let message_id = message
            .and_then(|value| first_string(value, &["id"], MAX_IDENTIFIER_BYTES))
            .map(|raw_message_id| opaque_message_key(&key, &raw_message_id));
        let role = message
            .and_then(|value| value.get("author"))
            .and_then(Value::as_object)
            .and_then(|author| author.get("role"))
            .and_then(Value::as_str)
            .map(normalize_role)
            .unwrap_or("other")
            .to_string();
        let created_at = message
            .and_then(|value| value.get("create_time"))
            .and_then(finite_number);
        let content = message.and_then(|value| value.get("content"));
        let content_type = content
            .and_then(Value::as_object)
            .and_then(|value| value.get("content_type"))
            .and_then(Value::as_str)
            .map(|value| truncate_utf8(value, 128))
            .unwrap_or_else(|| "text".to_string());
        let text = extract_message_text(content)?;
        let mut attachments = extract_attachments(message)?;
        attachment_count = attachment_count
            .checked_add(attachments.len())
            .ok_or(ErrorCode::ResourceLimit)?;
        if attachment_count > MAX_ATTACHMENTS_PER_CONVERSATION {
            return Err(ErrorCode::ResourceLimit.into());
        }
        deduplicate_attachments(&mut attachments);

        nodes.push(ProjectedNode {
            node_id,
            parent_node_id,
            child_node_ids,
            message_id,
            role,
            created_at,
            content_type,
            text,
            attachments,
        });
    }

    let raw_current = object
        .get("current_node")
        .and_then(Value::as_str)
        .map(|value| truncate_utf8(value, MAX_IDENTIFIER_BYTES));
    let mut current_node = raw_current
        .as_ref()
        .and_then(|value| raw_to_opaque.get(value))
        .cloned();
    if raw_current.is_some() && current_node.is_none() {
        record_diagnostic(&mut diagnostics, "MISSING_CURRENT_NODE");
    }
    if current_node.is_none() {
        current_node = choose_fallback_leaf(&nodes);
        if current_node.is_some() {
            record_diagnostic(&mut diagnostics, "FALLBACK_ACTIVE_LEAF");
        }
    }
    if let Some(current) = current_node.as_ref() {
        validate_active_path(current, &nodes, &mut diagnostics);
    }

    Ok(ProjectedConversation {
        key,
        source_id: None,
        title: object
            .get("title")
            .and_then(Value::as_str)
            .map(|value| truncate_utf8(value, MAX_TITLE_BYTES))
            .filter(|value| !value.trim().is_empty())
            .unwrap_or_else(|| "Untitled conversation".to_string()),
        created_at: object.get("create_time").and_then(finite_number),
        updated_at: object.get("update_time").and_then(finite_number),
        archived: first_bool(object, &["is_archived", "archived"]),
        starred: first_bool(object, &["is_starred", "starred"]),
        current_node,
        nodes,
        diagnostics: diagnostics
            .into_iter()
            .map(|(code, count)| ProjectedDiagnostic { code, count })
            .collect(),
    })
}

fn extract_message_text(content: Option<&Value>) -> AppResult<String> {
    let Some(content) = content else {
        return Ok(String::new());
    };
    let mut output = String::new();
    match content {
        Value::String(value) => push_text(&mut output, value)?,
        Value::Object(object) => {
            if let Some(parts) = object.get("parts").and_then(Value::as_array) {
                for part in parts {
                    match part {
                        Value::String(value) => push_text(&mut output, value)?,
                        Value::Object(part_object) => {
                            if let Some(value) = first_string(
                                part_object,
                                &["text", "content"],
                                MAX_MESSAGE_TEXT_BYTES,
                            ) {
                                push_text(&mut output, &value)?;
                            }
                        }
                        _ => {}
                    }
                }
            } else if let Some(value) =
                first_string(object, &["text", "content"], MAX_MESSAGE_TEXT_BYTES)
            {
                push_text(&mut output, &value)?;
            }
        }
        _ => {}
    }
    Ok(normalize_internal_markers(&output))
}

fn normalize_internal_markers(value: &str) -> String {
    let mut output = String::with_capacity(value.len());
    let mut cursor = 0;

    while let Some(relative_open) = value[cursor..].find(INTERNAL_MARKER_OPEN) {
        let open = cursor + relative_open;
        output.push_str(&value[cursor..open]);
        let after_open = open + INTERNAL_MARKER_OPEN.len();
        let marker_tail = &value[after_open..];
        let recognized_prefix = INTERNAL_MARKER_KINDS.iter().find_map(|kind| {
            let prefix = format!("{kind}{INTERNAL_MARKER_SEPARATOR}");
            marker_tail.starts_with(&prefix).then_some(prefix.len())
        });

        if let Some(prefix_len) = recognized_prefix {
            let payload_start = after_open + prefix_len;
            let mut bounded_end = value.len().min(
                payload_start
                    .saturating_add(MAX_INTERNAL_MARKER_BYTES)
                    .saturating_add(INTERNAL_MARKER_CLOSE.len()),
            );
            while bounded_end > payload_start && !value.is_char_boundary(bounded_end) {
                bounded_end -= 1;
            }
            if let Some(relative_close) =
                value[payload_start..bounded_end].find(INTERNAL_MARKER_CLOSE)
            {
                let close = payload_start + relative_close;
                let payload = &value[payload_start..close];
                if !payload.is_empty() && !payload.contains(INTERNAL_MARKER_OPEN) {
                    cursor = close + INTERNAL_MARKER_CLOSE.len();
                    continue;
                }
            }
        }

        output.push_str(INTERNAL_MARKER_OPEN);
        cursor = after_open;
    }

    output.push_str(&value[cursor..]);
    output
}

fn push_text(output: &mut String, value: &str) -> AppResult<()> {
    let separator_bytes = usize::from(!output.is_empty());
    let proposed = output
        .len()
        .checked_add(value.len())
        .and_then(|size| size.checked_add(separator_bytes))
        .ok_or(ErrorCode::ResourceLimit)?;
    if proposed > MAX_MESSAGE_TEXT_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }
    if !output.is_empty() {
        output.push('\n');
    }
    output.push_str(value);
    Ok(())
}

fn extract_attachments(
    message: Option<&Map<String, Value>>,
) -> AppResult<Vec<ProjectedAttachment>> {
    let Some(message) = message else {
        return Ok(Vec::new());
    };
    let mut attachments = Vec::new();
    if let Some(metadata_attachments) = message
        .get("metadata")
        .and_then(Value::as_object)
        .and_then(|metadata| metadata.get("attachments"))
        .and_then(Value::as_array)
    {
        for value in metadata_attachments {
            push_attachment_candidate(&mut attachments, value, true);
        }
    }
    if let Some(parts) = message
        .get("content")
        .and_then(Value::as_object)
        .and_then(|content| content.get("parts"))
        .and_then(Value::as_array)
    {
        for value in parts {
            push_attachment_candidate(&mut attachments, value, false);
        }
    }

    Ok(attachments)
}

fn push_attachment_candidate(
    attachments: &mut Vec<ProjectedAttachment>,
    value: &Value,
    from_attachment_metadata: bool,
) {
    let Some(object) = value.as_object() else {
        return;
    };
    let explicit_file_reference = first_string(
        object,
        &["file_id", "asset_pointer", "file_name", "filename"],
        MAX_IDENTIFIER_BYTES,
    );
    let metadata_reference = from_attachment_metadata
        .then(|| first_string(object, &["id", "name", "title"], MAX_IDENTIFIER_BYTES));
    let reference = explicit_file_reference
        .clone()
        .or_else(|| metadata_reference.flatten());
    let name = first_string(
        object,
        &["name", "file_name", "filename", "title"],
        MAX_FILENAME_BYTES,
    );
    let explicit_mime = first_string(object, &["mime_type"], MAX_MIME_BYTES);
    let recognized_named_metadata =
        name.is_some() && (from_attachment_metadata || explicit_mime.is_some());

    if explicit_file_reference.is_none() && !recognized_named_metadata {
        return;
    }

    let mime =
        explicit_mime.or_else(|| first_string(object, &["content_type"], MAX_MIME_BYTES));
    attachments.push(ProjectedAttachment {
        reference,
        display_name: sanitize_display_name(name.as_deref().unwrap_or("Attachment")),
        claimed_mime: mime.map(|value| sanitize_mime(&value)),
    });
}

fn deduplicate_attachments(attachments: &mut Vec<ProjectedAttachment>) {
    let mut seen = HashSet::new();
    attachments.retain(|attachment| {
        let key = (
            attachment.reference.clone(),
            attachment.display_name.clone(),
            attachment.claimed_mime.clone(),
        );
        seen.insert(key)
    });
}

fn sanitize_display_name(value: &str) -> String {
    let cleaned: String = value
        .chars()
        .filter(|character| !character.is_control())
        .map(|character| {
            if matches!(character, '/' | '\\' | ':' | '\0') {
                '_'
            } else {
                character
            }
        })
        .collect();
    let truncated = truncate_utf8(cleaned.trim(), 240);
    if truncated.is_empty() {
        "Attachment".to_string()
    } else {
        truncated
    }
}

fn sanitize_mime(value: &str) -> String {
    value
        .bytes()
        .filter(|byte| {
            byte.is_ascii_alphanumeric() || matches!(byte, b'/' | b'+' | b'-' | b'.')
        })
        .take(MAX_MIME_BYTES)
        .map(char::from)
        .collect()
}

fn choose_fallback_leaf(nodes: &[ProjectedNode]) -> Option<String> {
    let referenced_parents: HashSet<&str> = nodes
        .iter()
        .filter_map(|node| node.parent_node_id.as_deref())
        .collect();
    nodes
        .iter()
        .filter(|node| !referenced_parents.contains(node.node_id.as_str()))
        .max_by(|left, right| {
            left.created_at
                .partial_cmp(&right.created_at)
                .unwrap_or(std::cmp::Ordering::Equal)
                .then_with(|| left.node_id.cmp(&right.node_id))
        })
        .map(|node| node.node_id.clone())
}

fn record_diagnostic(diagnostics: &mut BTreeMap<&'static str, u32>, code: &'static str) {
    let count = diagnostics.entry(code).or_default();
    *count = count.saturating_add(1);
}

fn validate_active_path(
    current: &str,
    nodes: &[ProjectedNode],
    diagnostics: &mut BTreeMap<&'static str, u32>,
) {
    let parents: HashMap<&str, Option<&str>> = nodes
        .iter()
        .map(|node| (node.node_id.as_str(), node.parent_node_id.as_deref()))
        .collect();
    let mut visited = HashSet::new();
    let mut cursor = Some(current);
    let mut depth = 0_usize;
    while let Some(node_id) = cursor {
        if !visited.insert(node_id) {
            record_diagnostic(diagnostics, "ACTIVE_PATH_CYCLE");
            return;
        }
        depth += 1;
        if depth > MAX_ACTIVE_PATH_DEPTH {
            record_diagnostic(diagnostics, "ACTIVE_PATH_LIMIT");
            return;
        }
        cursor = parents.get(node_id).copied().flatten();
    }
}

fn normalize_role(value: &str) -> &'static str {
    match value.to_ascii_lowercase().as_str() {
        "user" => "user",
        "assistant" => "assistant",
        "system" => "system",
        "tool" => "tool",
        _ => "other",
    }
}

fn first_string(
    object: &Map<String, Value>,
    keys: &[&str],
    max_bytes: usize,
) -> Option<String> {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(Value::as_str)
        .map(|value| truncate_utf8(value, max_bytes))
}

fn first_bool(object: &Map<String, Value>, keys: &[&str]) -> Option<bool> {
    keys.iter()
        .filter_map(|key| object.get(*key))
        .find_map(Value::as_bool)
}

fn finite_number(value: &Value) -> Option<f64> {
    value.as_f64().filter(|number| number.is_finite())
}

fn opaque_conversation_key(shard_name: &str, ordinal: u64, source_id: Option<&str>) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(b"conversation-v1\0");
    hasher.update(shard_name.as_bytes());
    hasher.update(&ordinal.to_le_bytes());
    if let Some(source_id) = source_id {
        hasher.update(source_id.as_bytes());
    }
    hasher.finalize().to_hex()[..32].to_string()
}

fn opaque_node_key(conversation_key: &str, source_id: &str) -> String {
    opaque_child_key(b"node-v1\0", conversation_key, source_id)
}

fn opaque_message_key(conversation_key: &str, source_id: &str) -> String {
    opaque_child_key(b"message-v1\0", conversation_key, source_id)
}

fn opaque_child_key(domain: &[u8], conversation_key: &str, source_id: &str) -> String {
    let mut hasher = blake3::Hasher::new();
    hasher.update(domain);
    hasher.update(conversation_key.as_bytes());
    hasher.update(source_id.as_bytes());
    hasher.finalize().to_hex()[..32].to_string()
}

pub fn truncate_utf8(value: &str, max_bytes: usize) -> String {
    if value.len() <= max_bytes {
        return value.to_string();
    }
    let mut boundary = max_bytes;
    while boundary > 0 && !value.is_char_boundary(boundary) {
        boundary -= 1;
    }
    value[..boundary].to_string()
}

#[cfg(test)]
mod tests {
    use serde_json::json;

    use super::*;

    #[test]
    fn reconstructs_projected_relationships_and_normalizes_roles() {
        let value = json!({
            "id": "synthetic-conversation",
            "title": "Synthetic title",
            "current_node": "node-b",
            "mapping": {
                "node-a": {
                    "id": "node-a",
                    "parent": null,
                    "children": ["node-b"],
                    "message": {
                        "id": "message-a",
                        "author": {"role": "user"},
                        "content": {"content_type": "text", "parts": ["Synthetic prompt"]}
                    }
                },
                "node-b": {
                    "id": "node-b",
                    "parent": "node-a",
                    "children": [],
                    "message": {
                        "id": "message-b",
                        "author": {"role": "assistant"},
                        "content": {"content_type": "text", "parts": ["Synthetic answer"]}
                    }
                }
            }
        });

        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        assert_eq!(projected.nodes.len(), 2);
        assert_eq!(projected.nodes[0].role, "user");
        assert_eq!(projected.nodes[1].role, "assistant");
        assert_eq!(
            projected.nodes[1].parent_node_id.as_deref(),
            Some(projected.nodes[0].node_id.as_str())
        );
        assert_eq!(
            projected.current_node.as_deref(),
            Some(projected.nodes[1].node_id.as_str())
        );
    }

    #[test]
    fn sanitizes_attachment_names_without_using_them_as_paths() {
        let value = json!({
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "author": {"role": "tool"},
                        "content": {
                            "parts": [{
                                "file_id": "synthetic-file",
                                "name": "../Synthetic unsafe\r\nname.txt",
                                "mime_type": "text/plain"
                            }]
                        }
                    }
                }
            }
        });
        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        let attachment = &projected.nodes[0].attachments[0];
        assert_eq!(attachment.display_name, ".._Synthetic unsafename.txt");
    }

    #[test]
    fn preserves_structured_transcript_text_without_creating_an_attachment() {
        let value = json!({
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "author": {"role": "assistant"},
                        "content": {
                            "parts": [{
                                "text": "Synthetic spoken transcript",
                                "content_type": "audio_transcription"
                            }]
                        }
                    }
                }
            }
        });

        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        assert_eq!(projected.nodes[0].text, "Synthetic spoken transcript");
        assert!(projected.nodes[0].attachments.is_empty());
    }

    #[test]
    fn retains_explicit_and_metadata_attachment_candidates() {
        let value = json!({
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "author": {"role": "user"},
                        "metadata": {
                            "attachments": [{
                                "id": "synthetic-metadata-file",
                                "name": "fictional-note.txt",
                                "mime_type": "text/plain"
                            }]
                        },
                        "content": {
                            "parts": [{
                                "asset_pointer": "file-service://synthetic-pointer",
                                "content_type": "image/png"
                            }]
                        }
                    }
                }
            }
        });

        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        assert_eq!(projected.nodes[0].attachments.len(), 2);
        assert_eq!(
            projected.nodes[0].attachments[0].reference.as_deref(),
            Some("synthetic-metadata-file")
        );
        assert_eq!(
            projected.nodes[0].attachments[1].reference.as_deref(),
            Some("file-service://synthetic-pointer")
        );
    }

    #[test]
    fn removes_only_well_formed_known_internal_marker_envelopes() {
        let marker_only = "\u{e200}cite\u{e202}turn0search0\u{e201}";
        let inline =
            "Before \u{e200}navlist\u{e202}turn0news0\u{e202}turn0news1\u{e201} after.";
        let multiple = "A\u{e200}cite\u{e202}turn0search0\u{e201}B\u{e200}filecite\u{e202}turn0file0\u{e202}L1-L2\u{e201}C";
        let malformed = "Keep \u{e200}cite\u{e202}unterminated";
        let ordinary_private_use = "Keep \u{e200}fictional\u{e202}value\u{e201} unchanged";

        assert_eq!(normalize_internal_markers(marker_only), "");
        assert_eq!(normalize_internal_markers(inline), "Before  after.");
        assert_eq!(normalize_internal_markers(multiple), "ABC");
        assert_eq!(normalize_internal_markers(malformed), malformed);
        assert_eq!(
            normalize_internal_markers(ordinary_private_use),
            ordinary_private_use
        );
    }

    #[test]
    fn marker_normalization_preserves_surrounding_unicode_exactly() {
        let value = "Lantern 🏮 café\u{301}\u{e200}cite\u{e202}turn0search0\u{e201}—done";
        assert_eq!(
            normalize_internal_markers(value).as_bytes(),
            "Lantern 🏮 café\u{301}—done".as_bytes()
        );
    }

    #[test]
    fn long_malformed_marker_with_unicode_at_scan_limit_is_preserved() {
        let mut value = format!("{INTERNAL_MARKER_OPEN}cite{INTERNAL_MARKER_SEPARATOR}");
        value.push_str(&"x".repeat(MAX_INTERNAL_MARKER_BYTES + 2));
        value.push('🧪');

        let fixture = json!({
            "current_node": "synthetic-node",
            "mapping": {
                "synthetic-node": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "author": {"role": "assistant"},
                        "content": {"parts": [value.clone()]}
                    }
                }
            }
        });
        let projected =
            project_conversation(&fixture, "conversations-000.json", 0).expect("project");

        assert_eq!(projected.nodes[0].text, value);
    }

    #[test]
    fn detects_active_path_cycles() {
        let value = json!({
            "current_node": "node-a",
            "mapping": {
                "node-a": {"parent": "node-b", "children": ["node-b"], "message": null},
                "node-b": {"parent": "node-a", "children": ["node-a"], "message": null}
            }
        });
        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        assert!(
            projected
                .diagnostics
                .iter()
                .any(|diagnostic| diagnostic.code == "ACTIVE_PATH_CYCLE")
        );
    }

    #[test]
    fn aggregates_repeated_diagnostics_into_fixed_code_counts() {
        let invalid_children = (0..MAX_CHILDREN_PER_NODE)
            .map(|_| Value::Null)
            .collect::<Vec<_>>();
        let value = json!({
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": invalid_children,
                    "message": null
                }
            }
        });

        let projected =
            project_conversation(&value, "conversations-000.json", 0).expect("project");
        assert_eq!(projected.diagnostics.len(), 1);
        assert_eq!(projected.diagnostics[0].code, "INVALID_CHILD_REFERENCE");
        assert_eq!(
            projected.diagnostics[0].count,
            u32::try_from(MAX_CHILDREN_PER_NODE).expect("bounded child count")
        );
    }

    #[test]
    fn skips_empty_compatibility_records_as_unsupported() {
        assert!(matches!(
            project_conversation(&json!({}), "conversations-000.json", 0),
            Err(crate::error::AppError::Public(ErrorCode::UnsupportedRecord))
        ));
    }
}
