use axum::{
    Json,
    http::StatusCode,
    response::{IntoResponse, Response},
};
use serde::Serialize;
use thiserror::Error;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize)]
#[serde(rename_all = "SCREAMING_SNAKE_CASE")]
pub enum ErrorCode {
    InvalidRequest,
    Unauthorized,
    InvalidExport,
    ExportNotSelected,
    PathRejected,
    UnsupportedRecord,
    RecordTooLarge,
    MalformedJson,
    ResourceLimit,
    IndexBusy,
    IndexCancelled,
    IndexUnavailable,
    ConversationNotFound,
    AttachmentNotFound,
    AttachmentUnavailable,
    UnsupportedPreview,
    Internal,
}

impl ErrorCode {
    pub const fn message(self) -> &'static str {
        match self {
            Self::InvalidRequest => "The request could not be accepted.",
            Self::Unauthorized => "This local session is not authorized.",
            Self::InvalidExport => "The selected folder is not a supported export.",
            Self::ExportNotSelected => "Select an export folder first.",
            Self::PathRejected => "The requested filesystem location was rejected.",
            Self::UnsupportedRecord => "An unsupported record was skipped.",
            Self::RecordTooLarge => "A record exceeded the safe processing limit.",
            Self::MalformedJson => "A conversation shard is malformed.",
            Self::ResourceLimit => "A safe processing limit was reached.",
            Self::IndexBusy => "An indexing job is already running.",
            Self::IndexCancelled => "Indexing was cancelled safely.",
            Self::IndexUnavailable => "The local index is unavailable.",
            Self::ConversationNotFound => "The conversation was not found.",
            Self::AttachmentNotFound => "The attachment was not found.",
            Self::AttachmentUnavailable => "The attachment file is unavailable.",
            Self::UnsupportedPreview => "This attachment cannot be previewed safely.",
            Self::Internal => "The application could not complete the request.",
        }
    }
}

#[derive(Debug, Error)]
pub enum AppError {
    #[error("{0:?}")]
    Public(ErrorCode),
    #[error("io")]
    Io(#[from] std::io::Error),
    #[error("database")]
    Database(#[from] rusqlite::Error),
    #[error("json")]
    Json(#[from] serde_json::Error),
    #[error("internal")]
    Internal,
}

impl AppError {
    pub const fn code(&self) -> ErrorCode {
        match self {
            Self::Public(code) => *code,
            Self::Io(_) | Self::Database(_) | Self::Json(_) | Self::Internal => {
                ErrorCode::Internal
            }
        }
    }

    pub const fn status(&self) -> StatusCode {
        match self.code() {
            ErrorCode::Unauthorized => StatusCode::UNAUTHORIZED,
            ErrorCode::ConversationNotFound | ErrorCode::AttachmentNotFound => {
                StatusCode::NOT_FOUND
            }
            ErrorCode::IndexBusy => StatusCode::CONFLICT,
            ErrorCode::Internal => StatusCode::INTERNAL_SERVER_ERROR,
            ErrorCode::AttachmentUnavailable => StatusCode::GONE,
            _ => StatusCode::BAD_REQUEST,
        }
    }
}

impl From<ErrorCode> for AppError {
    fn from(value: ErrorCode) -> Self {
        Self::Public(value)
    }
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorEnvelope {
    error: ErrorBody,
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct ErrorBody {
    code: ErrorCode,
    message: &'static str,
}

impl IntoResponse for AppError {
    fn into_response(self) -> Response {
        let code = self.code();
        (
            self.status(),
            Json(ErrorEnvelope {
                error: ErrorBody {
                    code,
                    message: code.message(),
                },
            }),
        )
            .into_response()
    }
}

pub type AppResult<T> = Result<T, AppError>;
