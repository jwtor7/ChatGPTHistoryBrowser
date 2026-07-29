use serde::{Deserialize, Serialize};

use crate::{
    error::{AppResult, ErrorCode},
    models::ConversationDetail,
};

const MAX_CONVERSATION_EXPORT_BYTES: usize = 128 * 1024 * 1024;
const MAX_FILE_STEM_BYTES: usize = 96;
const MAX_PDF_TEXT_BYTES: usize = 8 * 1024 * 1024;
const MAX_PDF_PAGES: usize = 2_000;

const PRIVACY_NOTICE: &str = "This document contains private conversation data. \
Uploading or sharing it transfers that data under the destination provider's policies.";
const ATTACHMENT_NOTICE: &str = "Attachments are not included in this export.";
const UNTITLED_CONVERSATION: &str = "Untitled conversation";
const UNTITLED_FILE_STEM: &str = "Untitled-conversation";

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum ConversationExportFormat {
    Md,
    Pdf,
    Txt,
}

impl ConversationExportFormat {
    pub const fn extension(self) -> &'static str {
        match self {
            Self::Md => "md",
            Self::Pdf => "pdf",
            Self::Txt => "txt",
        }
    }

    pub const fn human_label(self) -> &'static str {
        match self {
            Self::Md => "Markdown",
            Self::Pdf => "PDF",
            Self::Txt => "Plain text",
        }
    }

    pub const fn content_type(self) -> &'static str {
        match self {
            Self::Md => "text/markdown; charset=utf-8",
            Self::Pdf => "application/pdf",
            Self::Txt => "text/plain; charset=utf-8",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "camelCase")]
pub struct ConversationExportEstimate {
    pub conversation_count: usize,
    pub message_count: usize,
    pub attachment_count: usize,
    pub byte_size: usize,
    pub file_name: String,
}

pub fn serialize_conversation_export(
    detail: &ConversationDetail,
    format: ConversationExportFormat,
) -> AppResult<(Vec<u8>, ConversationExportEstimate)> {
    let attachment_count = detail
        .messages
        .iter()
        .try_fold(0_usize, |total, message| {
            total.checked_add(message.attachments.len())
        })
        .ok_or(ErrorCode::ResourceLimit)?;

    let bytes = match format {
        ConversationExportFormat::Md => render_markdown(detail, attachment_count)?.into_bytes(),
        ConversationExportFormat::Pdf => render_pdf(detail, attachment_count)?,
        ConversationExportFormat::Txt => {
            render_plain_text(detail, attachment_count)?.into_bytes()
        }
    };
    if bytes.len() > MAX_CONVERSATION_EXPORT_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }

    let estimate = ConversationExportEstimate {
        conversation_count: 1,
        message_count: detail.messages.len(),
        attachment_count,
        byte_size: bytes.len(),
        file_name: conversation_export_file_name(&detail.title, format),
    };
    Ok((bytes, estimate))
}

pub fn conversation_export_file_name(title: &str, format: ConversationExportFormat) -> String {
    let mut stem = String::new();
    let mut separator_pending = false;

    if !looks_like_opaque_identifier(title.trim()) {
        for character in title.trim().chars() {
            if character.is_alphanumeric() {
                let separator_bytes = usize::from(separator_pending && !stem.is_empty());
                let Some(next_len) = stem
                    .len()
                    .checked_add(separator_bytes)
                    .and_then(|length| length.checked_add(character.len_utf8()))
                else {
                    break;
                };
                if next_len > MAX_FILE_STEM_BYTES {
                    break;
                }
                if separator_bytes == 1 {
                    stem.push('-');
                }
                stem.push(character);
                separator_pending = false;
            } else if !stem.is_empty() {
                separator_pending = true;
            }
        }
    }

    if stem.is_empty() {
        stem.push_str(UNTITLED_FILE_STEM);
    } else if is_windows_reserved_file_stem(&stem) {
        stem.insert_str(0, "Conversation-");
    }

    format!("{stem}.{}", format.extension())
}

fn render_markdown(detail: &ConversationDetail, attachment_count: usize) -> AppResult<String> {
    let mut output = String::new();
    let title = markdown_inline(&display_title(&detail.title));
    append_bounded(&mut output, "# ")?;
    append_bounded(&mut output, &title)?;
    append_bounded(&mut output, "\n\n> ")?;
    append_bounded(&mut output, PRIVACY_NOTICE)?;
    append_bounded(&mut output, "\n>\n> ")?;
    append_bounded(&mut output, ATTACHMENT_NOTICE)?;
    append_attachment_count_markdown(&mut output, attachment_count)?;
    append_bounded(&mut output, "\n")?;

    for (index, message) in detail.messages.iter().enumerate() {
        append_bounded(&mut output, "\n## ")?;
        append_bounded(&mut output, &(index + 1).to_string())?;
        append_bounded(&mut output, ". ")?;
        append_bounded(&mut output, &markdown_inline(&display_role(&message.role)))?;
        append_bounded(&mut output, "\n")?;
        if let Some(created_at) = finite_timestamp(message.created_at) {
            append_bounded(&mut output, "\nTimestamp (Unix seconds): `")?;
            append_bounded(&mut output, &created_at.to_string())?;
            append_bounded(&mut output, "`\n")?;
        }
        append_bounded(&mut output, "\n")?;
        if message.text.trim().is_empty() {
            append_bounded(&mut output, "_No text content._\n")?;
        } else {
            let safe_text = sanitize_markdown_export_text(&message.text)?;
            append_bounded(&mut output, &safe_text)?;
            if !safe_text.ends_with('\n') {
                append_bounded(&mut output, "\n")?;
            }
        }
    }

    Ok(output)
}

fn render_plain_text(
    detail: &ConversationDetail,
    attachment_count: usize,
) -> AppResult<String> {
    let mut output = String::new();
    let title = display_title(&detail.title);
    append_bounded(&mut output, &title)?;
    append_bounded(&mut output, "\n")?;
    let underline_length = title.chars().count().clamp(3, 72);
    append_bounded(&mut output, &"=".repeat(underline_length))?;
    append_bounded(&mut output, "\n\n")?;
    append_bounded(&mut output, PRIVACY_NOTICE)?;
    append_bounded(&mut output, "\n")?;
    append_bounded(&mut output, ATTACHMENT_NOTICE)?;
    append_attachment_count_plain(&mut output, attachment_count)?;
    append_bounded(&mut output, "\n")?;

    for (index, message) in detail.messages.iter().enumerate() {
        append_bounded(&mut output, "\n[")?;
        append_bounded(&mut output, &(index + 1).to_string())?;
        append_bounded(&mut output, "] ")?;
        append_bounded(&mut output, &display_role(&message.role))?;
        append_bounded(&mut output, "\n")?;
        append_bounded(&mut output, "----------------------------------------\n")?;
        if let Some(created_at) = finite_timestamp(message.created_at) {
            append_bounded(&mut output, "Timestamp (Unix seconds): ")?;
            append_bounded(&mut output, &created_at.to_string())?;
            append_bounded(&mut output, "\n\n")?;
        }
        if message.text.trim().is_empty() {
            append_bounded(&mut output, "(No text content.)\n")?;
        } else {
            let safe_text = sanitize_plain_export_text(&message.text)?;
            append_bounded(&mut output, &safe_text)?;
            if !safe_text.ends_with('\n') {
                append_bounded(&mut output, "\n")?;
            }
        }
    }

    Ok(output)
}

fn append_attachment_count_markdown(
    output: &mut String,
    attachment_count: usize,
) -> AppResult<()> {
    if attachment_count > 0 {
        append_bounded(output, " (")?;
        append_bounded(output, &attachment_count.to_string())?;
        append_bounded(
            output,
            if attachment_count == 1 {
                " attachment omitted.)"
            } else {
                " attachments omitted.)"
            },
        )?;
    }
    Ok(())
}

fn append_attachment_count_plain(
    output: &mut String,
    attachment_count: usize,
) -> AppResult<()> {
    if attachment_count > 0 {
        append_bounded(output, " (")?;
        append_bounded(output, &attachment_count.to_string())?;
        append_bounded(
            output,
            if attachment_count == 1 {
                " attachment omitted.)"
            } else {
                " attachments omitted.)"
            },
        )?;
    }
    Ok(())
}

#[cfg(target_os = "macos")]
#[allow(unsafe_code)]
fn render_pdf(detail: &ConversationDetail, attachment_count: usize) -> AppResult<Vec<u8>> {
    use objc2_core_foundation::{
        CFAttributedString, CFDictionary, CFMutableData, CFRange, CFString, CGAffineTransform,
        CGPoint, CGRect, CGSize,
    };
    use objc2_core_graphics::{
        CGContext, CGDataConsumer, CGPDFContextBeginPage, CGPDFContextClose,
        CGPDFContextCreate, CGPDFContextEndPage, CGPath,
    };
    use objc2_core_text::{CTFont, CTFontUIFontType, CTFramesetter, kCTFontAttributeName};

    const PAGE_WIDTH: f64 = 612.0;
    const PAGE_HEIGHT: f64 = 792.0;
    const PAGE_MARGIN: f64 = 54.0;
    const FONT_SIZE: f64 = 10.0;

    let plain_text = render_plain_text(detail, attachment_count)?;
    ensure_pdf_text_length(plain_text.len())?;
    let string = CFString::from_str(&plain_text);
    // SAFETY: These Core Text wrappers call Apple framework APIs with retained
    // Core Foundation values and documented nullable optional arguments.
    let font =
        unsafe { CTFont::new_ui_font_for_language(CTFontUIFontType::System, FONT_SIZE, None) }
            .ok_or(ErrorCode::Internal)?;
    let attributes = CFDictionary::<CFString, CTFont>::from_slices(
        // SAFETY: Core Text exports this process-lifetime CFString constant.
        &[unsafe { kCTFontAttributeName }],
        &[font.as_ref()],
    );
    // SAFETY: The dictionary's value type is CTFont, as required by the key.
    let attributed =
        unsafe { CFAttributedString::new(None, Some(&string), Some(attributes.as_opaque())) }
            .ok_or(ErrorCode::Internal)?;
    // SAFETY: `attributed` is a valid retained attributed string.
    let framesetter = unsafe { CTFramesetter::with_attributed_string(&attributed) };

    let media_box = CGRect::new(CGPoint::ZERO, CGSize::new(PAGE_WIDTH, PAGE_HEIGHT));
    let text_box = CGRect::new(
        CGPoint::new(PAGE_MARGIN, PAGE_MARGIN),
        CGSize::new(
            PAGE_WIDTH - PAGE_MARGIN * 2.0,
            PAGE_HEIGHT - PAGE_MARGIN * 2.0,
        ),
    );
    // SAFETY: A null transform is explicitly supported and means identity.
    let text_path = unsafe { CGPath::with_rect(text_box, std::ptr::null()) };
    let data = CFMutableData::new(None, 0).ok_or(ErrorCode::Internal)?;
    let consumer = CGDataConsumer::with_cf_data(Some(&data)).ok_or(ErrorCode::Internal)?;
    // SAFETY: `media_box` remains alive for this call and auxiliary metadata is
    // omitted, so there are no unchecked dictionary values.
    let context = unsafe { CGPDFContextCreate(Some(&consumer), &media_box, None) }
        .ok_or(ErrorCode::Internal)?;

    let mut location = 0;
    let text_length = attributed.length();
    let mut page_count = 0_usize;
    while location < text_length {
        if page_count >= MAX_PDF_PAGES {
            return Err(ErrorCode::ResourceLimit.into());
        }

        // SAFETY: The retained PDF context is open and page metadata is omitted.
        unsafe { CGPDFContextBeginPage(Some(&context), None) };
        CGContext::set_text_matrix(
            Some(&context),
            CGAffineTransform {
                a: 1.0,
                b: 0.0,
                c: 0.0,
                d: 1.0,
                tx: 0.0,
                ty: 0.0,
            },
        );
        // SAFETY: `location` advances only by Core Text's visible range and is
        // bounded by the attributed string length; frame attributes are absent.
        let frame = unsafe { framesetter.frame(CFRange::new(location, 0), &text_path, None) };
        // SAFETY: `frame` is a valid retained Core Text frame.
        let visible = unsafe { frame.visible_string_range() };
        if visible.length <= 0 {
            CGPDFContextEndPage(Some(&context));
            CGPDFContextClose(Some(&context));
            return Err(ErrorCode::Internal.into());
        }
        // SAFETY: Both the frame and the current page context are retained.
        unsafe { frame.draw(&context) };
        CGPDFContextEndPage(Some(&context));

        location = visible
            .location
            .checked_add(visible.length)
            .ok_or(ErrorCode::ResourceLimit)?;
        page_count += 1;
    }
    CGPDFContextClose(Some(&context));

    let bytes = data.to_vec();
    if !bytes.starts_with(b"%PDF-") {
        return Err(ErrorCode::Internal.into());
    }
    checked_export_len(0, bytes.len())?;
    Ok(bytes)
}

#[cfg(not(target_os = "macos"))]
fn render_pdf(_detail: &ConversationDetail, _attachment_count: usize) -> AppResult<Vec<u8>> {
    Err(ErrorCode::UnsupportedRecord.into())
}

fn checked_export_len(current: usize, additional: usize) -> AppResult<usize> {
    let length = current
        .checked_add(additional)
        .ok_or(ErrorCode::ResourceLimit)?;
    if length > MAX_CONVERSATION_EXPORT_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }
    Ok(length)
}

fn ensure_pdf_text_length(length: usize) -> AppResult<()> {
    if length > MAX_PDF_TEXT_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }
    Ok(())
}

fn append_bounded(output: &mut String, value: &str) -> AppResult<()> {
    checked_export_len(output.len(), value.len())?;
    output.push_str(value);
    Ok(())
}

fn display_title(title: &str) -> String {
    let cleaned = collapse_inline_whitespace(title);
    if cleaned.is_empty() || looks_like_opaque_identifier(&cleaned) {
        UNTITLED_CONVERSATION.to_string()
    } else {
        cleaned
    }
}

fn display_role(role: &str) -> String {
    let cleaned = collapse_inline_whitespace(role);
    if cleaned.is_empty() || looks_like_opaque_identifier(&cleaned) {
        return "Other".to_string();
    }
    match cleaned.to_ascii_lowercase().as_str() {
        "assistant" => "Assistant".to_string(),
        "system" => "System".to_string(),
        "tool" => "Tool".to_string(),
        "user" => "User".to_string(),
        _ => cleaned,
    }
}

fn collapse_inline_whitespace(value: &str) -> String {
    let mut output = String::new();
    let mut whitespace_pending = false;
    for character in value.chars() {
        if character.is_whitespace() || character.is_control() {
            whitespace_pending = !output.is_empty();
        } else {
            if whitespace_pending {
                output.push(' ');
                whitespace_pending = false;
            }
            output.push(character);
        }
    }
    output
}

fn markdown_inline(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len());
    for character in value.chars() {
        if matches!(
            character,
            '\\' | '`' | '*' | '_' | '{' | '}' | '[' | ']' | '<' | '>' | '#' | '|' | '!'
        ) {
            escaped.push('\\');
        }
        escaped.push(character);
    }
    escaped
}

fn sanitize_markdown_export_text(value: &str) -> AppResult<String> {
    let mut output = String::new();
    let mut remainder = value;
    let mut fence: Option<(u8, usize)> = None;

    while !remainder.is_empty() {
        let (raw_line, has_newline, next) = if let Some(index) = remainder.find('\n') {
            (&remainder[..index], true, &remainder[index + 1..])
        } else {
            (remainder, false, "")
        };
        let line = raw_line.strip_suffix('\r').unwrap_or(raw_line);

        if let Some((marker, minimum_length)) = fence {
            append_sanitized_text(&mut output, line, false)?;
            if is_markdown_closing_fence(line, marker, minimum_length) {
                fence = None;
            }
        } else if let Some((marker, length)) = markdown_opening_fence(line) {
            append_sanitized_text(&mut output, line, false)?;
            fence = Some((marker, length));
        } else {
            append_markdown_inline_text(&mut output, line)?;
        }

        if has_newline {
            append_bounded(&mut output, "\n")?;
        }
        remainder = next;
    }

    Ok(output)
}

fn sanitize_plain_export_text(value: &str) -> AppResult<String> {
    sanitize_export_text(value, false)
}

fn sanitize_export_text(value: &str, markdown: bool) -> AppResult<String> {
    let mut output = String::new();
    append_sanitized_text(&mut output, value, markdown)?;
    Ok(output)
}

fn append_sanitized_text(output: &mut String, value: &str, markdown: bool) -> AppResult<()> {
    let mut characters = value.chars().peekable();
    while let Some(character) = characters.next() {
        match character {
            '\r' => {
                if characters.peek() == Some(&'\n') {
                    characters.next();
                }
                append_bounded(output, "\n")?;
            }
            '\n' | '\t' => append_export_character(output, character)?,
            value if value.is_control() => {
                append_bounded(output, &format!("[U+{:04X}]", u32::from(value)))?;
            }
            '&' if markdown => append_bounded(output, "&amp;")?,
            '<' if markdown => append_bounded(output, "&lt;")?,
            '>' if markdown => append_bounded(output, "&gt;")?,
            '[' if markdown => append_bounded(output, "&#91;")?,
            ']' if markdown => append_bounded(output, "&#93;")?,
            value => append_export_character(output, value)?,
        }
    }
    Ok(())
}

fn markdown_fence_run(line: &str) -> Option<(u8, usize, usize)> {
    let bytes = line.as_bytes();
    let indent = bytes.iter().take_while(|byte| **byte == b' ').count();
    if indent > 3 {
        return None;
    }
    let marker = *bytes.get(indent)?;
    if !matches!(marker, b'`' | b'~') {
        return None;
    }
    let length = bytes[indent..]
        .iter()
        .take_while(|byte| **byte == marker)
        .count();
    (length >= 3).then_some((marker, length, indent + length))
}

fn markdown_opening_fence(line: &str) -> Option<(u8, usize)> {
    markdown_fence_run(line).and_then(|(marker, length, end)| {
        (marker != b'`' || !line[end..].contains('`')).then_some((marker, length))
    })
}

fn is_markdown_closing_fence(line: &str, marker: u8, minimum_length: usize) -> bool {
    markdown_fence_run(line).is_some_and(|(candidate, length, end)| {
        candidate == marker
            && length >= minimum_length
            && line[end..].bytes().all(|byte| matches!(byte, b' ' | b'\t'))
    })
}

fn append_markdown_inline_text(output: &mut String, line: &str) -> AppResult<()> {
    let bytes = line.as_bytes();
    let mut cursor = 0;

    while cursor < bytes.len() {
        let Some(relative_open) = bytes[cursor..].iter().position(|byte| *byte == b'`') else {
            append_sanitized_text(output, &line[cursor..], true)?;
            break;
        };
        let open = cursor + relative_open;
        append_sanitized_text(output, &line[cursor..open], true)?;
        let delimiter_length = bytes[open..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        let content_start = open + delimiter_length;

        if let Some((close, close_end)) =
            find_matching_backtick_run(bytes, content_start, delimiter_length)
        {
            append_sanitized_text(output, &line[open..content_start], false)?;
            append_sanitized_text(output, &line[content_start..close], false)?;
            append_sanitized_text(output, &line[close..close_end], false)?;
            cursor = close_end;
        } else {
            append_sanitized_text(output, &line[open..content_start], true)?;
            cursor = content_start;
        }
    }

    Ok(())
}

fn find_matching_backtick_run(
    bytes: &[u8],
    mut cursor: usize,
    delimiter_length: usize,
) -> Option<(usize, usize)> {
    while cursor < bytes.len() {
        let relative = bytes[cursor..].iter().position(|byte| *byte == b'`')?;
        let start = cursor + relative;
        let length = bytes[start..]
            .iter()
            .take_while(|byte| **byte == b'`')
            .count();
        if length == delimiter_length {
            return Some((start, start + length));
        }
        cursor = start + length;
    }
    None
}

fn append_export_character(output: &mut String, character: char) -> AppResult<()> {
    let mut encoded = [0_u8; 4];
    append_bounded(output, character.encode_utf8(&mut encoded))
}

fn finite_timestamp(value: Option<f64>) -> Option<f64> {
    value.filter(|timestamp| timestamp.is_finite())
}

fn looks_like_opaque_identifier(value: &str) -> bool {
    let bytes = value.as_bytes();
    if matches!(bytes.len(), 32 | 40 | 64) && bytes.iter().all(u8::is_ascii_hexdigit) {
        return true;
    }
    bytes.len() == 36
        && bytes.iter().enumerate().all(|(index, byte)| match index {
            8 | 13 | 18 | 23 => *byte == b'-',
            _ => byte.is_ascii_hexdigit(),
        })
}

fn is_windows_reserved_file_stem(stem: &str) -> bool {
    let upper = stem.to_ascii_uppercase();
    matches!(upper.as_str(), "CON" | "PRN" | "AUX" | "NUL")
        || upper
            .strip_prefix("COM")
            .or_else(|| upper.strip_prefix("LPT"))
            .is_some_and(|suffix| {
                suffix.len() == 1
                    && suffix
                        .as_bytes()
                        .first()
                        .is_some_and(|digit| matches!(digit, b'1'..=b'9'))
            })
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
            messages: vec![
                MessageView {
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
                },
                MessageView {
                    node_id: "33333333333333333333333333333333".to_string(),
                    role: "assistant".to_string(),
                    created_at: Some(1_735_689_700.0),
                    content_type: "text".to_string(),
                    text: "Line with (parentheses), a backslash \\, and café.".to_string(),
                    attachments: Vec::new(),
                    alternate_branches: Vec::new(),
                },
            ],
            diagnostics: Vec::new(),
        }
    }

    #[test]
    fn format_accepts_exact_query_values_and_exposes_metadata() {
        assert_eq!(
            serde_json::from_str::<ConversationExportFormat>("\"md\"").expect("md"),
            ConversationExportFormat::Md
        );
        assert_eq!(
            serde_json::from_str::<ConversationExportFormat>("\"pdf\"").expect("pdf"),
            ConversationExportFormat::Pdf
        );
        assert_eq!(
            serde_json::from_str::<ConversationExportFormat>("\"txt\"").expect("txt"),
            ConversationExportFormat::Txt
        );
        assert!(serde_json::from_str::<ConversationExportFormat>("\"json\"").is_err());
        assert!(serde_json::from_str::<ConversationExportFormat>("\"MD\"").is_err());
        assert_eq!(ConversationExportFormat::Md.extension(), "md");
        assert_eq!(ConversationExportFormat::Pdf.human_label(), "PDF");
        assert_eq!(
            ConversationExportFormat::Txt.content_type(),
            "text/plain; charset=utf-8"
        );
    }

    #[test]
    fn markdown_export_is_readable_and_estimated_exactly() {
        let detail = synthetic_detail();
        let (bytes, estimate) =
            serialize_conversation_export(&detail, ConversationExportFormat::Md)
                .expect("markdown");
        let document = String::from_utf8(bytes.clone()).expect("utf8");

        assert!(document.starts_with("# Fictional Lantern Notes\n"));
        assert!(document.contains("## 1. User"));
        assert!(document.contains("## 2. Assistant"));
        assert!(document.contains("A **synthetic** message about Example Island."));
        assert!(document.contains("Attachments are not included"));
        assert_eq!(estimate.conversation_count, 1);
        assert_eq!(estimate.message_count, 2);
        assert_eq!(estimate.attachment_count, 1);
        assert_eq!(estimate.byte_size, bytes.len());
        assert_eq!(estimate.file_name, "Fictional-Lantern-Notes.md");
    }

    #[test]
    fn plain_text_export_is_readable_and_estimated_exactly() {
        let detail = synthetic_detail();
        let (bytes, estimate) =
            serialize_conversation_export(&detail, ConversationExportFormat::Txt)
                .expect("plain text");
        let document = String::from_utf8(bytes.clone()).expect("utf8");

        assert!(document.starts_with("Fictional Lantern Notes\n"));
        assert!(document.contains("[1] User"));
        assert!(document.contains("[2] Assistant"));
        assert!(document.contains("A **synthetic** message about Example Island."));
        assert!(document.contains("1 attachment omitted"));
        assert_eq!(estimate.byte_size, bytes.len());
        assert_eq!(estimate.file_name, "Fictional-Lantern-Notes.txt");
    }

    #[test]
    fn exports_neutralize_active_markdown_and_terminal_controls() {
        let mut detail = synthetic_detail();
        detail.messages[0].text = concat!(
            "<img src=\"https://pixel.invalid/track.png\"> ",
            "![remote](https://pixel.invalid/track.png) ",
            "[destination](https://example.invalid) ",
            "\u{001b}]0;unsafe title\u{0007}\rnext line"
        )
        .to_string();

        let (markdown, _) =
            serialize_conversation_export(&detail, ConversationExportFormat::Md)
                .expect("markdown");
        let markdown = String::from_utf8(markdown).expect("markdown utf8");
        assert!(markdown.contains("&lt;img src=&quot;") || markdown.contains("&lt;img src=\""));
        assert!(markdown.contains("&#91;remote&#93;"));
        assert!(markdown.contains("&#91;destination&#93;"));
        assert!(!markdown.contains("<img"));
        assert!(!markdown.contains("!["));
        assert!(!markdown.contains("](https://"));
        assert!(!markdown.contains('\u{001b}'));
        assert!(!markdown.contains('\u{0007}'));
        assert!(markdown.contains("[U+001B]"));
        assert!(markdown.contains("[U+0007]\nnext line"));

        let (plain, _) = serialize_conversation_export(&detail, ConversationExportFormat::Txt)
            .expect("plain text");
        let plain = String::from_utf8(plain).expect("plain utf8");
        assert!(!plain.contains('\u{001b}'));
        assert!(!plain.contains('\u{0007}'));
        assert!(plain.contains("[U+001B]"));
        assert!(plain.contains("[U+0007]\nnext line"));
    }

    #[test]
    fn markdown_export_preserves_inline_and_fenced_code_verbatim() {
        let mut detail = synthetic_detail();
        detail.messages[0].text = concat!(
            "Use `values[0] < limit && ready` here.\n\n",
            "```rust\n",
            "if values[0] < limit && ready {\n",
            "    println!(\"<safe>\");\n",
            "}\n",
            "```\n\n",
            "<img src=\"https://pixel.invalid/track.png\">"
        )
        .to_string();

        let (markdown, _) =
            serialize_conversation_export(&detail, ConversationExportFormat::Md)
                .expect("markdown");
        let markdown = String::from_utf8(markdown).expect("markdown utf8");

        assert!(markdown.contains("`values[0] < limit && ready`"));
        assert!(markdown.contains("if values[0] < limit && ready {"));
        assert!(markdown.contains("println!(\"<safe>\");"));
        assert!(markdown.contains("&lt;img src="));
        assert!(!markdown.contains("<img src="));

        let invalid_fence = sanitize_markdown_export_text(
            "```invalid` info\n<img src=\"https://pixel.invalid/not-code.png\">",
        )
        .expect("invalid fence");
        assert!(invalid_fence.contains("&lt;img src="));
        assert!(!invalid_fence.contains("<img src="));
    }

    #[test]
    fn file_name_matches_exceptional_opportunity_scan_exactly() {
        assert_eq!(
            conversation_export_file_name(
                "**Exceptional Opportunity Scan",
                ConversationExportFormat::Md
            ),
            "Exceptional-Opportunity-Scan.md"
        );
    }

    #[test]
    fn file_name_handles_unicode_paths_fallback_reserved_names_and_length() {
        assert_eq!(
            conversation_export_file_name(
                "../../Café \u{2014} 安全/Quarter:Plan?.",
                ConversationExportFormat::Pdf
            ),
            "Café-安全-Quarter-Plan.pdf"
        );
        assert_eq!(
            conversation_export_file_name("../\\:*?\"<>|", ConversationExportFormat::Txt),
            "Untitled-conversation.txt"
        );
        assert_eq!(
            conversation_export_file_name(
                "0123456789abcdef0123456789abcdef",
                ConversationExportFormat::Md
            ),
            "Untitled-conversation.md"
        );
        assert_eq!(
            conversation_export_file_name("CON", ConversationExportFormat::Txt),
            "Conversation-CON.txt"
        );

        let long_title = "界".repeat(100);
        let file_name =
            conversation_export_file_name(&long_title, ConversationExportFormat::Pdf);
        let stem = file_name.strip_suffix(".pdf").expect("extension");
        assert!(stem.len() <= MAX_FILE_STEM_BYTES);
        assert!(stem.is_char_boundary(stem.len()));
        assert!(!file_name.contains('/'));
        assert!(!file_name.contains('\\'));
        assert!(!file_name.contains(':'));
    }

    #[test]
    fn all_formats_exclude_attachments_branches_and_opaque_metadata() {
        let detail = synthetic_detail();
        let mut formats = vec![ConversationExportFormat::Md, ConversationExportFormat::Txt];
        #[cfg(target_os = "macos")]
        formats.push(ConversationExportFormat::Pdf);
        for format in formats {
            let (bytes, _) = serialize_conversation_export(&detail, format).expect("export");
            let visible = String::from_utf8_lossy(&bytes);
            assert!(!visible.contains("private-name-must-not-export"));
            assert!(!visible.contains("private branch preview must not export"));
            assert!(!visible.contains(&detail.id));
            assert!(
                !visible.contains(
                    detail
                        .selected_leaf
                        .as_deref()
                        .expect("synthetic selected leaf")
                )
            );
            assert!(!visible.contains("fedcba9876543210fedcba9876543210"));
        }
    }

    #[test]
    fn estimate_serializes_camel_case_fields_including_file_name() {
        let detail = synthetic_detail();
        let (_, estimate) =
            serialize_conversation_export(&detail, ConversationExportFormat::Md)
                .expect("export");
        let json = serde_json::to_value(estimate).expect("json");

        assert_eq!(json["conversationCount"], 1);
        assert_eq!(json["messageCount"], 2);
        assert_eq!(json["attachmentCount"], 1);
        assert!(json["byteSize"].as_u64().is_some_and(|size| size > 0));
        assert_eq!(json["fileName"], "Fictional-Lantern-Notes.md");
        assert!(json.get("file_name").is_none());
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn pdf_opens_and_round_trips_visible_content() {
        let detail = synthetic_detail();
        let (bytes, estimate) =
            serialize_conversation_export(&detail, ConversationExportFormat::Pdf).expect("pdf");
        let (text, page_count) = extracted_pdf_text_and_page_count(&bytes);

        assert!(bytes.starts_with(b"%PDF-"));
        assert_eq!(page_count, 1);
        assert!(text.contains("Fictional Lantern Notes"));
        assert!(text.contains("Example Island"));
        assert!(text.contains("(parentheses)"));
        assert!(text.contains("café"));
        assert_eq!(estimate.byte_size, bytes.len());
        assert_eq!(estimate.file_name, "Fictional-Lantern-Notes.pdf");
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn pdf_paginates_long_conversations() {
        let mut detail = synthetic_detail();
        detail.messages[0].text = (0..180)
            .map(|line| format!("Synthetic page line {line} with enough readable words.\n"))
            .collect();
        let (bytes, _) = serialize_conversation_export(&detail, ConversationExportFormat::Pdf)
            .expect("multipage pdf");
        let (text, page_count) = extracted_pdf_text_and_page_count(&bytes);

        assert!(page_count >= 3, "expected multiple pages, got {page_count}");
        assert!(text.contains("Synthetic page line 179"));
    }

    #[test]
    #[cfg(target_os = "macos")]
    fn pdf_preserves_cjk_arabic_and_emoji_without_substitution() {
        let mut detail = synthetic_detail();
        detail.title = "Conversation 🚀 安全".to_string();
        detail.messages[0].text =
            "CJK 安全; Arabic مرحبا بالعالم; emoji 🚀🙂; end marker.".to_string();
        let (bytes, _) =
            serialize_conversation_export(&detail, ConversationExportFormat::Pdf).expect("pdf");
        let (text, _) = extracted_pdf_text_and_page_count(&bytes);

        assert!(text.contains("Conversation 🚀 安全"), "{text:?}");
        assert!(text.contains("CJK 安全"), "{text:?}");
        let extracted_arabic = text
            .split_once("Arabic ")
            .and_then(|(_, remainder)| remainder.split_once("; emoji"))
            .map(|(arabic, _)| arabic)
            .expect("Arabic run should remain extractable");
        // PDFKit exposes contextual Arabic presentation glyphs rather than the
        // original logical code points. Verify the shaped run remains present
        // and is never replaced with ASCII question marks or replacement chars.
        assert!(
            extracted_arabic
                .chars()
                .filter(|character| !character.is_ascii())
                .count()
                >= 8,
            "{text:?}"
        );
        assert!(!extracted_arabic.contains('?'), "{text:?}");
        assert!(!extracted_arabic.contains('\u{fffd}'), "{text:?}");
        assert!(text.contains('🚀'), "{text:?}");
        assert!(text.contains('🙂'), "{text:?}");
        assert!(!text.contains("Conversation ? ??"), "{text:?}");
    }

    #[test]
    fn size_guard_rejects_limit_overflow_without_allocating_it() {
        assert_eq!(
            checked_export_len(MAX_CONVERSATION_EXPORT_BYTES - 1, 1).expect("at limit"),
            MAX_CONVERSATION_EXPORT_BYTES
        );
        let error =
            checked_export_len(MAX_CONVERSATION_EXPORT_BYTES, 1).expect_err("over limit");
        assert_eq!(error.code(), ErrorCode::ResourceLimit);
        let overflow = checked_export_len(usize::MAX, 1).expect_err("usize overflow");
        assert_eq!(overflow.code(), ErrorCode::ResourceLimit);

        ensure_pdf_text_length(MAX_PDF_TEXT_BYTES).expect("PDF text at limit");
        let pdf_error =
            ensure_pdf_text_length(MAX_PDF_TEXT_BYTES + 1).expect_err("PDF text over limit");
        assert_eq!(pdf_error.code(), ErrorCode::ResourceLimit);
    }

    #[cfg(target_os = "macos")]
    #[allow(unsafe_code)]
    fn extracted_pdf_text_and_page_count(pdf: &[u8]) -> (String, usize) {
        use objc2::AnyThread;
        use objc2_foundation::NSData;
        use objc2_pdf_kit::PDFDocument;

        let data = NSData::with_bytes(pdf);
        // SAFETY: NSData owns the supplied bytes for the duration of parsing.
        let document = unsafe { PDFDocument::initWithData(PDFDocument::alloc(), &data) }
            .expect("PDFKit should open generated PDF");
        // SAFETY: `document` is initialized and retained.
        let text = unsafe { document.string() }
            .expect("generated PDF should contain extractable text")
            .to_string();
        // SAFETY: `document` is initialized and retained.
        let page_count = unsafe { document.pageCount() };
        (text, page_count)
    }
}
