use serde::{Deserialize, Serialize};

use crate::error::ErrorCode;

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AppStatus {
    pub export_selected: bool,
    pub shard_count: usize,
    pub attachment_file_count: usize,
    pub index: IndexProgress,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ExportValidation {
    pub supported: bool,
    pub shard_count: usize,
    pub attachment_file_count: usize,
    pub total_json_bytes: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum IndexPhase {
    #[default]
    Idle,
    Discovering,
    Indexing,
    Cancelling,
    Complete,
    Cancelled,
    Failed,
}

#[derive(Debug, Clone, Default, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct IndexProgress {
    pub phase: IndexPhase,
    pub failure_code: Option<ErrorCode>,
    pub shards_total: usize,
    pub shards_complete: usize,
    pub bytes_total: u64,
    pub bytes_processed: u64,
    pub conversations_indexed: u64,
    pub conversations_skipped: u64,
    pub diagnostics: u64,
}

#[derive(Debug, Default, Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
pub struct ConversationQuery {
    pub page: Option<u32>,
    pub page_size: Option<u32>,
    pub search: Option<String>,
    pub date_from: Option<f64>,
    pub date_to: Option<f64>,
    pub role: Option<String>,
    pub archived: Option<bool>,
    pub starred: Option<bool>,
    pub has_attachments: Option<bool>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationPage {
    pub items: Vec<ConversationListItem>,
    pub page: u32,
    pub page_size: u32,
    pub total: u64,
    pub has_more: bool,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationListItem {
    pub id: String,
    pub title: String,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub archived: Option<bool>,
    pub starred: Option<bool>,
    pub has_attachments: bool,
    pub message_count: u32,
    pub match_preview: Option<String>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationDetail {
    pub id: String,
    pub title: String,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub archived: Option<bool>,
    pub starred: Option<bool>,
    pub selected_leaf: Option<String>,
    pub messages: Vec<MessageView>,
    pub diagnostics: Vec<DiagnosticView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct MessageView {
    pub node_id: String,
    pub role: String,
    pub created_at: Option<f64>,
    pub content_type: String,
    pub text: String,
    pub attachments: Vec<AttachmentView>,
    pub alternate_branches: Vec<BranchView>,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct BranchView {
    pub leaf_node_id: String,
    pub role: String,
    pub preview: String,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct AttachmentView {
    pub id: String,
    pub display_name: String,
    pub claimed_mime: Option<String>,
    pub detected_mime: Option<String>,
    pub byte_size: Option<u64>,
    pub status: AttachmentStatus,
    pub preview_kind: PreviewKind,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum AttachmentStatus {
    Available,
    Missing,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum PreviewKind {
    Image,
    Audio,
    Video,
    Pdf,
    Text,
    Unsupported,
    Missing,
}

#[derive(Debug, Clone, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct DiagnosticView {
    pub code: String,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct ProjectedDiagnostic {
    pub code: &'static str,
    pub count: u32,
}

#[derive(Debug, Clone)]
pub struct ProjectedConversation {
    pub key: String,
    pub source_id: Option<String>,
    pub title: String,
    pub created_at: Option<f64>,
    pub updated_at: Option<f64>,
    pub archived: Option<bool>,
    pub starred: Option<bool>,
    pub current_node: Option<String>,
    pub nodes: Vec<ProjectedNode>,
    pub diagnostics: Vec<ProjectedDiagnostic>,
}

#[derive(Debug, Clone)]
pub struct ProjectedNode {
    pub node_id: String,
    pub parent_node_id: Option<String>,
    pub child_node_ids: Vec<String>,
    pub message_id: Option<String>,
    pub role: String,
    pub created_at: Option<f64>,
    pub content_type: String,
    pub text: String,
    pub attachments: Vec<ProjectedAttachment>,
}

#[derive(Debug, Clone)]
pub struct ProjectedAttachment {
    pub reference: Option<String>,
    pub display_name: String,
    pub claimed_mime: Option<String>,
}
