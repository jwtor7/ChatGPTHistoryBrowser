use serde::{Deserialize, Serialize};

use crate::{
    error::{AppResult, ErrorCode},
    models::ConversationDetail,
};

const FORMAT_ID: &str = "chatgpt-history-browser.portable-context";
const FORMAT_VERSION: u32 = 1;
const MAX_PORTABLE_EXPORT_BYTES: usize = 128 * 1024 * 1024;

const PRIVACY_NOTICE: &str = "This plaintext package contains private conversation data. \
Importing or uploading it transfers that data under the destination provider's policies.";

const IMPORT_PROMPT: &str = "Use the supplied conversation as background context. Preserve \
who said what, distinguish source text from new conclusions, and ask before treating inferred \
preferences or facts as durable memory.";

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableContextPackage {
    pub format: String,
    pub version: u32,
    pub created_by: String,
    pub privacy_notice: String,
    pub import_prompt: String,
    pub conversation: PortableConversation,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableConversation {
    pub id: String,
    pub title: String,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub archived: Option<bool>,
    pub starred: Option<bool>,
    pub selected_leaf: Option<String>,
    pub attachment_count: usize,
    pub attachments_included: bool,
    pub markdown: String,
    pub messages: Vec<PortableMessage>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableMessage {
    pub ordinal: usize,
    pub node_id: String,
    pub role: String,
    pub created_at: Option<f64>,
    pub content_type: String,
    pub text: String,
    pub alternate_branches: Vec<PortableBranch>,
}

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableBranch {
    pub leaf_node_id: String,
    pub role: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct PortableExportEstimate {
    pub conversation_count: usize,
    pub message_count: usize,
    pub attachment_count: usize,
    pub byte_size: usize,
}

pub fn serialize_portable_context(
    detail: &ConversationDetail,
) -> AppResult<(Vec<u8>, PortableExportEstimate)> {
    let attachment_count = detail
        .messages
        .iter()
        .try_fold(0_usize, |total, message| {
            total.checked_add(message.attachments.len())
        })
        .ok_or(ErrorCode::ResourceLimit)?;
    let messages = detail
        .messages
        .iter()
        .enumerate()
        .map(|(index, message)| PortableMessage {
            ordinal: index + 1,
            node_id: message.node_id.clone(),
            role: message.role.clone(),
            created_at: message.created_at,
            content_type: message.content_type.clone(),
            text: message.text.clone(),
            alternate_branches: message
                .alternate_branches
                .iter()
                .map(|branch| PortableBranch {
                    leaf_node_id: branch.leaf_node_id.clone(),
                    role: branch.role.clone(),
                })
                .collect(),
        })
        .collect::<Vec<_>>();
    let package = PortableContextPackage {
        format: FORMAT_ID.to_string(),
        version: FORMAT_VERSION,
        created_by: format!("ChatGPT History Browser {}", env!("CARGO_PKG_VERSION")),
        privacy_notice: PRIVACY_NOTICE.to_string(),
        import_prompt: IMPORT_PROMPT.to_string(),
        conversation: PortableConversation {
            id: detail.id.clone(),
            title: detail.title.clone(),
            created_at: detail.created_at,
            updated_at: detail.updated_at,
            archived: detail.archived,
            starred: detail.starred,
            selected_leaf: detail.selected_leaf.clone(),
            attachment_count,
            attachments_included: false,
            markdown: render_markdown(detail),
            messages,
        },
    };
    let mut bytes = serde_json::to_vec_pretty(&package)?;
    bytes.push(b'\n');
    if bytes.len() > MAX_PORTABLE_EXPORT_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }
    let estimate = PortableExportEstimate {
        conversation_count: 1,
        message_count: detail.messages.len(),
        attachment_count,
        byte_size: bytes.len(),
    };
    Ok((bytes, estimate))
}

pub fn portable_file_name(conversation_id: &str) -> AppResult<String> {
    if conversation_id.len() != 32
        || !conversation_id.bytes().all(|byte| byte.is_ascii_hexdigit())
    {
        return Err(ErrorCode::InvalidRequest.into());
    }
    Ok(format!("context-{conversation_id}.portable.json"))
}

fn render_markdown(detail: &ConversationDetail) -> String {
    let title = if detail.title.trim().is_empty() {
        "Untitled conversation"
    } else {
        detail.title.trim()
    };
    let mut markdown = format!(
        "# {title}\n\n> Portable active-path export from ChatGPT History Browser. \
Attachments are not included.\n"
    );
    for (index, message) in detail.messages.iter().enumerate() {
        let role = if message.role.trim().is_empty() {
            "Other"
        } else {
            message.role.trim()
        };
        markdown.push_str(&format!("\n## {}. {}\n\n", index + 1, role));
        if let Some(created_at) = message.created_at {
            markdown.push_str(&format!("Timestamp (Unix seconds): `{created_at}`\n\n"));
        }
        markdown.push_str(&message.text);
        if !message.text.ends_with('\n') {
            markdown.push('\n');
        }
    }
    markdown
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::models::{
        AttachmentStatus, AttachmentView, BranchView, MessageView, PreviewKind,
    };

    fn synthetic_detail() -> ConversationDetail {
        ConversationDetail {
            id: "0123456789abcdef0123456789abcdef".to_string(),
            title: "Fictional Lantern Notes".to_string(),
            created_at: Some(1_735_689_600.0),
            updated_at: Some(1_735_689_700.0),
            archived: Some(false),
            starred: Some(true),
            selected_leaf: Some("abcdef0123456789abcdef0123456789".to_string()),
            messages: vec![MessageView {
                node_id: "fedcba9876543210fedcba9876543210".to_string(),
                role: "user".to_string(),
                created_at: Some(1_735_689_600.0),
                content_type: "text".to_string(),
                text: "A **synthetic** message about Example Island.".to_string(),
                attachments: vec![AttachmentView {
                    id: "11111111111111111111111111111111".to_string(),
                    display_name: "private-name-must-not-export.txt".to_string(),
                    claimed_mime: Some("text/plain".to_string()),
                    detected_mime: Some("text/plain".to_string()),
                    byte_size: Some(42),
                    status: AttachmentStatus::Available,
                    preview_kind: PreviewKind::Text,
                }],
                alternate_branches: vec![BranchView {
                    leaf_node_id: "22222222222222222222222222222222".to_string(),
                    role: "assistant".to_string(),
                    preview: "private branch preview must not export".to_string(),
                }],
            }],
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn portable_package_round_trips_active_path_without_attachment_content() {
        let detail = synthetic_detail();
        let (bytes, estimate) = serialize_portable_context(&detail).expect("serialize");
        let parsed: PortableContextPackage =
            serde_json::from_slice(&bytes).expect("round trip");

        assert_eq!(parsed.format, FORMAT_ID);
        assert_eq!(parsed.version, FORMAT_VERSION);
        assert_eq!(parsed.conversation.id, detail.id);
        assert_eq!(parsed.conversation.selected_leaf, detail.selected_leaf);
        assert_eq!(parsed.conversation.attachment_count, 1);
        assert!(!parsed.conversation.attachments_included);
        assert_eq!(parsed.conversation.messages[0].ordinal, 1);
        assert_eq!(parsed.conversation.messages[0].role, "user");
        assert_eq!(
            parsed.conversation.messages[0].alternate_branches[0].leaf_node_id,
            "22222222222222222222222222222222"
        );
        assert!(parsed.conversation.markdown.contains("**synthetic**"));
        assert_eq!(estimate.conversation_count, 1);
        assert_eq!(estimate.message_count, 1);
        assert_eq!(estimate.attachment_count, 1);
        assert_eq!(estimate.byte_size, bytes.len());

        let serialized = String::from_utf8(bytes).expect("utf8 json");
        assert!(!serialized.contains("private-name-must-not-export"));
        assert!(!serialized.contains("private branch preview must not export"));
    }

    #[test]
    fn portable_file_name_is_opaque_and_deterministic() {
        let name = portable_file_name("0123456789abcdef0123456789abcdef").expect("name");
        assert_eq!(
            name,
            "context-0123456789abcdef0123456789abcdef.portable.json"
        );
        assert!(portable_file_name("../not-opaque").is_err());
    }
}
