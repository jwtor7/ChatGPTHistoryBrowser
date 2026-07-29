use std::{
    collections::HashSet,
    future::Future,
    io::Write,
    net::{Ipv4Addr, SocketAddrV4, TcpListener as StdTcpListener},
    path::{Path, PathBuf},
    pin::Pin,
    sync::{Arc, RwLock},
    task::{Context, Poll},
    time::Duration,
};

use axum::{
    Json, Router,
    body::Body,
    extract::{DefaultBodyLimit, Path as AxumPath, Query, State},
    http::{
        HeaderMap, HeaderValue, Method, Request, StatusCode,
        header::{
            AUTHORIZATION, CACHE_CONTROL, CONTENT_DISPOSITION, CONTENT_LENGTH, CONTENT_TYPE,
            HOST, ORIGIN,
        },
    },
    middleware::{self, Next},
    response::{IntoResponse, Response},
    routing::{get, post},
};
use base64::{Engine as _, engine::general_purpose::URL_SAFE_NO_PAD};
use rand::RngExt;
use serde::Serialize;
use subtle::ConstantTimeEq;
use tokio::{
    io::{AsyncRead, ReadBuf},
    sync::{OwnedSemaphorePermit, Semaphore},
};
use tokio_util::{io::ReaderStream, sync::CancellationToken};
use tower_http::{
    services::{ServeDir, ServeFile},
    timeout::TimeoutLayer,
};

use crate::{
    attachments::{
        MAX_INLINE_MEDIA_BYTES, open_validated_preview, read_text_preview,
        safe_download_name_for_detected_type,
    },
    error::{AppError, AppResult, ErrorCode},
    indexer::{ArchiveSession, IndexCoordinator},
    models::{AppStatus, ConversationQuery, ExportValidation, IndexProgress, PreviewKind},
    portable_export::{
        ConversationExportEstimate, ConversationExportFormat, MAX_CONVERSATION_EXPORT_BYTES,
        MAX_CONVERSATION_SET_SIZE, serialize_conversation_export,
        serialize_conversation_set_export,
    },
    safe_root::SafeExportRoot,
};

const MAX_REQUEST_BYTES: usize = 16 * 1024;
const MAX_CONCURRENT_PREVIEWS: usize = 4;
const MAX_CONCURRENT_PDF_EXPORTS: usize = 1;
const MAX_CONVERSATION_SET_MESSAGES: usize = 100_000;
const MAX_CONVERSATION_SET_ATTACHMENTS: usize = 100_000;
const PREVIEW_BUDGET_UNIT: u64 = 1024 * 1024;
const PREVIEW_BUDGET_UNITS: usize = 64;
const REQUEST_TIMEOUT: Duration = Duration::from_secs(30);
const MAX_WEB_ASSET_COUNT: usize = 10_000;
const MAX_WEB_ASSET_BYTES: u64 = 512 * 1024 * 1024;
const MAX_WEB_ASSET_DEPTH: usize = 16;

type MainThreadTask = Box<dyn FnOnce() + Send + 'static>;
type MainThreadDispatcher =
    Arc<dyn Fn(MainThreadTask) -> AppResult<()> + Send + Sync + 'static>;

#[derive(Clone)]
pub struct ServerState {
    token: Arc<String>,
    expected_host: Arc<String>,
    expected_origin: Arc<String>,
    session: Arc<RwLock<Option<Arc<ArchiveSession>>>>,
    indexer: IndexCoordinator,
    preview_slots: Arc<Semaphore>,
    preview_bytes: Arc<Semaphore>,
    pdf_export_slots: Arc<Semaphore>,
    main_thread_dispatcher: Option<MainThreadDispatcher>,
}

pub struct BoundServer {
    pub origin: String,
    pub token: String,
    pub shutdown: CancellationToken,
}

struct GuardedPreviewReader {
    file: tokio::fs::File,
    _slot_permit: OwnedSemaphorePermit,
    _byte_permit: OwnedSemaphorePermit,
}

impl AsyncRead for GuardedPreviewReader {
    fn poll_read(
        mut self: Pin<&mut Self>,
        context: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.file).poll_read(context, buffer)
    }
}

impl ServerState {
    fn new(token: String, host: String, origin: String) -> Self {
        Self {
            token: Arc::new(token),
            expected_host: Arc::new(host),
            expected_origin: Arc::new(origin),
            session: Arc::new(RwLock::new(None)),
            indexer: IndexCoordinator::default(),
            preview_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_PREVIEWS)),
            preview_bytes: Arc::new(Semaphore::new(PREVIEW_BUDGET_UNITS)),
            pdf_export_slots: Arc::new(Semaphore::new(MAX_CONCURRENT_PDF_EXPORTS)),
            main_thread_dispatcher: None,
        }
    }

    fn with_main_thread_dispatcher(mut self, dispatcher: MainThreadDispatcher) -> Self {
        self.main_thread_dispatcher = Some(dispatcher);
        self
    }

    fn session(&self) -> AppResult<Arc<ArchiveSession>> {
        self.session
            .read()
            .map_err(|_| ErrorCode::Internal)?
            .clone()
            .ok_or_else(|| ErrorCode::ExportNotSelected.into())
    }

    fn replace_session(&self, session: Arc<ArchiveSession>) -> AppResult<()> {
        if self.indexer.is_running() {
            return Err(ErrorCode::IndexBusy.into());
        }
        let mut current = self.session.write().map_err(|_| ErrorCode::Internal)?;
        *current = Some(session);
        let _ = self.indexer.reset()?;
        Ok(())
    }
}

pub fn bind_loopback(
    web_root: PathBuf,
) -> AppResult<(StdTcpListener, ServerState, BoundServer)> {
    validate_web_root(&web_root)?;
    let listener = StdTcpListener::bind(SocketAddrV4::new(Ipv4Addr::LOCALHOST, 0))
        .map_err(|_| ErrorCode::Internal)?;
    listener
        .set_nonblocking(true)
        .map_err(|_| ErrorCode::Internal)?;
    let address = listener.local_addr().map_err(|_| ErrorCode::Internal)?;
    if address.ip() != std::net::IpAddr::V4(Ipv4Addr::LOCALHOST) || address.port() == 0 {
        return Err(ErrorCode::Internal.into());
    }

    let mut token_bytes = [0_u8; 32];
    rand::rng().fill(&mut token_bytes);
    let token = URL_SAFE_NO_PAD.encode(token_bytes);
    let host = format!("127.0.0.1:{}", address.port());
    let origin = format!("http://{host}");
    let state = ServerState::new(token.clone(), host, origin.clone());
    let shutdown = CancellationToken::new();
    Ok((
        listener,
        state,
        BoundServer {
            origin,
            token,
            shutdown,
        },
    ))
}

pub fn bind_desktop_loopback(
    web_root: PathBuf,
    app_handle: tauri::AppHandle,
) -> AppResult<(StdTcpListener, ServerState, BoundServer)> {
    let (listener, state, bound) = bind_loopback(web_root)?;
    let dispatcher: MainThreadDispatcher = Arc::new(move |task| {
        app_handle
            .run_on_main_thread(task)
            .map_err(|_| AppError::Internal)
    });
    Ok((
        listener,
        state.with_main_thread_dispatcher(dispatcher),
        bound,
    ))
}

pub fn spawn_loopback(
    listener: StdTcpListener,
    state: ServerState,
    web_root: PathBuf,
    shutdown: CancellationToken,
) {
    let router = build_router(state, web_root);
    let _task = tauri::async_runtime::spawn(async move {
        let Ok(listener) = tokio::net::TcpListener::from_std(listener) else {
            return;
        };
        let _result = axum::serve(listener, router)
            .with_graceful_shutdown(shutdown.cancelled_owned())
            .await;
    });
}

fn build_router(state: ServerState, web_root: PathBuf) -> Router {
    let bounded_api = Router::new()
        .route("/api/status", get(status))
        .route("/api/index/start", post(start_index))
        .route("/api/index/cancel", post(cancel_index))
        .route("/api/index/discard", post(discard_index))
        .route("/api/index/status", get(index_status))
        .route("/api/conversations", get(list_conversations))
        .route("/api/conversations/{id}", get(get_conversation))
        .route(
            "/api/conversations/{id}/export",
            get(conversation_export_estimate),
        )
        .route(
            "/api/conversation-set/export/estimate",
            post(conversation_set_export_estimate),
        )
        .route("/api/attachments/{id}/content", get(attachment_content))
        .route("/api/attachments/{id}/text", get(attachment_text))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize_api))
        .layer(TimeoutLayer::with_status_code(
            StatusCode::REQUEST_TIMEOUT,
            REQUEST_TIMEOUT,
        ));
    let interactive_api = Router::new()
        .route("/api/export/pick", post(pick_export))
        .route(
            "/api/conversations/{id}/export",
            post(save_conversation_export),
        )
        .route(
            "/api/conversation-set/export",
            post(save_conversation_set_export),
        )
        .route("/api/attachments/{id}/save", post(save_attachment))
        .route_layer(middleware::from_fn_with_state(state.clone(), authorize_api));
    let api = bounded_api.merge(interactive_api);
    let static_files = ServeDir::new(web_root.clone())
        .append_index_html_on_directories(true)
        .fallback(ServeFile::new(web_root.join("index.html")));

    Router::new()
        .merge(api)
        .fallback_service(static_files)
        .layer(DefaultBodyLimit::max(MAX_REQUEST_BYTES))
        .layer(middleware::from_fn(security_headers))
        .layer(middleware::from_fn_with_state(state.clone(), enforce_host))
        .with_state(state)
}

async fn enforce_host(
    State(state): State<ServerState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let valid = request
        .headers()
        .get(HOST)
        .and_then(|value| value.to_str().ok())
        .is_some_and(|value| value == state.expected_host.as_str());
    if !valid {
        return AppError::from(ErrorCode::Unauthorized).into_response();
    }
    next.run(request).await
}

async fn authorize_api(
    State(state): State<ServerState>,
    request: Request<Body>,
    next: Next,
) -> Response {
    let Some(value) = request
        .headers()
        .get(AUTHORIZATION)
        .and_then(|value| value.to_str().ok())
        .and_then(|value| value.strip_prefix("Bearer "))
    else {
        return AppError::from(ErrorCode::Unauthorized).into_response();
    };
    let expected = state.token.as_bytes();
    let supplied = value.as_bytes();
    if expected.len() != supplied.len() || expected.ct_eq(supplied).unwrap_u8() != 1 {
        return AppError::from(ErrorCode::Unauthorized).into_response();
    }

    if !matches!(*request.method(), Method::GET | Method::HEAD) {
        let valid_origin = request
            .headers()
            .get(ORIGIN)
            .and_then(|value| value.to_str().ok())
            .is_some_and(|value| value == state.expected_origin.as_str());
        if !valid_origin {
            return AppError::from(ErrorCode::Unauthorized).into_response();
        }
    }
    next.run(request).await
}

async fn security_headers(request: Request<Body>, next: Next) -> Response {
    let mut response = next.run(request).await;
    let headers = response.headers_mut();
    set_header(
        headers,
        "content-security-policy",
        "default-src 'none'; script-src 'self'; style-src 'self'; style-src-elem 'self'; style-src-attr 'unsafe-inline'; img-src 'self' blob:; media-src 'self' blob:; connect-src 'self'; font-src 'self'; worker-src 'self' blob:; object-src 'none'; frame-src 'none'; frame-ancestors 'none'; base-uri 'none'; form-action 'none'",
    );
    set_header(headers, "x-content-type-options", "nosniff");
    set_header(headers, "x-frame-options", "DENY");
    set_header(headers, "referrer-policy", "no-referrer");
    set_header(headers, "cross-origin-resource-policy", "same-origin");
    set_header(headers, "cross-origin-opener-policy", "same-origin");
    set_header(
        headers,
        "permissions-policy",
        "accelerometer=(), camera=(), geolocation=(), gyroscope=(), microphone=(), payment=(), usb=()",
    );
    headers.insert(CACHE_CONTROL, HeaderValue::from_static("no-store"));
    response
}

fn set_header(headers: &mut HeaderMap, name: &'static str, value: &'static str) {
    headers.insert(
        axum::http::header::HeaderName::from_static(name),
        HeaderValue::from_static(value),
    );
}

async fn status(State(state): State<ServerState>) -> Json<AppStatus> {
    let session = state.session.read().ok().and_then(|value| value.clone());
    Json(AppStatus {
        export_selected: session.is_some(),
        shard_count: session
            .as_ref()
            .map_or(0, |session| session.root.shards().len()),
        attachment_file_count: session
            .as_ref()
            .map_or(0, |session| session.root.attachment_count()),
        index: state.indexer.progress(),
    })
}

async fn pick_export(State(state): State<ServerState>) -> AppResult<Response> {
    if state.indexer.is_running() {
        return Err(ErrorCode::IndexBusy.into());
    }
    let dispatcher = state
        .main_thread_dispatcher
        .as_ref()
        .ok_or(AppError::Internal)?;
    let selected = build_future_on_main_thread(dispatcher, || {
        let selection = rfd::AsyncFileDialog::new()
            .set_title("Select an extracted ChatGPT export")
            .pick_folder();
        async move {
            selection
                .await
                .map(|selected| selected.path().to_path_buf())
        }
    })
    .await?;
    let Some(path) = selected else {
        return Ok(StatusCode::NO_CONTENT.into_response());
    };
    let (session, validation) = create_session(path).await?;
    state.replace_session(session)?;
    Ok(Json(validation).into_response())
}

async fn build_future_on_main_thread<T, Build, Built>(
    dispatcher: &MainThreadDispatcher,
    build: Build,
) -> AppResult<T>
where
    T: Send + 'static,
    Build: FnOnce() -> Built + Send + 'static,
    Built: Future<Output = T> + Send + 'static,
{
    let (sender, receiver) = tokio::sync::oneshot::channel();
    dispatcher(Box::new(move || {
        let _ = sender.send(build());
    }))?;
    let future = receiver.await.map_err(|_| AppError::Internal)?;
    Ok(future.await)
}

async fn create_session(path: PathBuf) -> AppResult<(Arc<ArchiveSession>, ExportValidation)> {
    tokio::task::spawn_blocking(move || {
        let root = SafeExportRoot::select(&path)?;
        let validation = root.validation();
        let session = Arc::new(ArchiveSession::new(root)?);
        Ok((session, validation))
    })
    .await
    .map_err(|_| AppError::Internal)?
}

async fn start_index(State(state): State<ServerState>) -> AppResult<Json<IndexProgress>> {
    let session = state.session()?;
    state.indexer.start(session)?;
    Ok(Json(state.indexer.progress()))
}

async fn cancel_index(State(state): State<ServerState>) -> AppResult<Json<IndexProgress>> {
    Ok(Json(state.indexer.cancel()?))
}

async fn discard_index(State(state): State<ServerState>) -> AppResult<Json<IndexProgress>> {
    if state.indexer.is_running() {
        return Err(ErrorCode::IndexBusy.into());
    }
    let session = state.session()?;
    tokio::task::spawn_blocking(move || session.store.discard())
        .await
        .map_err(|_| AppError::Internal)??;
    Ok(Json(state.indexer.reset()?))
}

async fn index_status(State(state): State<ServerState>) -> Json<IndexProgress> {
    Json(state.indexer.progress())
}

async fn list_conversations(
    State(state): State<ServerState>,
    Query(query): Query<ConversationQuery>,
) -> AppResult<Json<crate::models::ConversationPage>> {
    let store = state.session()?.store.clone();
    let page = tokio::task::spawn_blocking(move || store.query_conversations(&query))
        .await
        .map_err(|_| AppError::Internal)??;
    Ok(Json(page))
}

#[derive(serde::Deserialize)]
struct DetailQuery {
    leaf: Option<String>,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
struct ConversationExportQuery {
    leaf: Option<String>,
    format: ConversationExportFormat,
}

#[derive(serde::Deserialize)]
#[serde(rename_all = "camelCase", deny_unknown_fields)]
struct ConversationSetExportRequest {
    ids: Vec<String>,
    format: ConversationExportFormat,
}

async fn get_conversation(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<DetailQuery>,
) -> AppResult<Json<crate::models::ConversationDetail>> {
    let store = state.session()?.store.clone();
    let detail = tokio::task::spawn_blocking(move || {
        store.conversation_detail(&id, query.leaf.as_deref())
    })
    .await
    .map_err(|_| AppError::Internal)??;
    Ok(Json(detail))
}

async fn conversation_export_estimate(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ConversationExportQuery>,
) -> AppResult<Json<ConversationExportEstimate>> {
    let (_, estimate) = build_conversation_export(&state, id, query.leaf, query.format).await?;
    Ok(Json(estimate))
}

async fn conversation_set_export_estimate(
    State(state): State<ServerState>,
    Json(request): Json<ConversationSetExportRequest>,
) -> AppResult<Json<ConversationExportEstimate>> {
    let (_, estimate) =
        build_conversation_set_export(&state, request.ids, request.format).await?;
    Ok(Json(estimate))
}

async fn save_conversation_export(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
    Query(query): Query<ConversationExportQuery>,
) -> AppResult<Json<SaveResult>> {
    let root = state.session()?.root.clone();
    let (bytes, estimate) =
        build_conversation_export(&state, id, query.leaf, query.format).await?;
    let file_name = estimate.file_name;
    let required_extension = query.format.extension().to_string();
    let dispatcher = state
        .main_thread_dispatcher
        .as_ref()
        .ok_or(AppError::Internal)?;
    let dialog_name = file_name.clone();
    let dialog_extension = required_extension.clone();
    let format_label = query.format.human_label();
    let selected = build_future_on_main_thread(dispatcher, move || {
        let selection = rfd::AsyncFileDialog::new()
            .set_title(format!("Save conversation as {format_label}"))
            .set_file_name(dialog_name)
            .add_filter(format_label, &[dialog_extension])
            .save_file();
        async move {
            selection
                .await
                .map(|selected| selected.path().to_path_buf())
        }
    })
    .await?;
    let Some(destination) = selected else {
        return Ok(Json(SaveResult {
            saved: false,
            file_name: None,
        }));
    };
    let destination = with_required_extension(destination, &required_extension);
    let saved_file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or(file_name);
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        if !root.write_destination_is_outside_root(&destination) {
            return Err(ErrorCode::PathRejected.into());
        }
        write_private_destination(&destination, |target| {
            target.write_all(&bytes).map_err(|_| ErrorCode::Internal)?;
            Ok(())
        })
    })
    .await
    .map_err(|_| AppError::Internal)??;
    Ok(Json(SaveResult {
        saved: true,
        file_name: Some(saved_file_name),
    }))
}

async fn save_conversation_set_export(
    State(state): State<ServerState>,
    Json(request): Json<ConversationSetExportRequest>,
) -> AppResult<Json<SaveResult>> {
    let root = state.session()?.root.clone();
    let format = request.format;
    let (bytes, estimate) = build_conversation_set_export(&state, request.ids, format).await?;
    let file_name = estimate.file_name;
    let required_extension = format.extension().to_string();
    let dispatcher = state
        .main_thread_dispatcher
        .as_ref()
        .ok_or(AppError::Internal)?;
    let dialog_name = file_name.clone();
    let dialog_extension = required_extension.clone();
    let format_label = format.human_label();
    let selected = build_future_on_main_thread(dispatcher, move || {
        let selection = rfd::AsyncFileDialog::new()
            .set_title(format!("Save selected conversations as {format_label}"))
            .set_file_name(dialog_name)
            .add_filter(format_label, &[dialog_extension])
            .save_file();
        async move {
            selection
                .await
                .map(|selected| selected.path().to_path_buf())
        }
    })
    .await?;
    let Some(destination) = selected else {
        return Ok(Json(SaveResult {
            saved: false,
            file_name: None,
        }));
    };
    let destination = with_required_extension(destination, &required_extension);
    let saved_file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or(file_name);
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        if !root.write_destination_is_outside_root(&destination) {
            return Err(ErrorCode::PathRejected.into());
        }
        write_private_destination(&destination, |target| {
            target.write_all(&bytes).map_err(|_| ErrorCode::Internal)?;
            Ok(())
        })
    })
    .await
    .map_err(|_| AppError::Internal)??;
    Ok(Json(SaveResult {
        saved: true,
        file_name: Some(saved_file_name),
    }))
}

async fn build_conversation_export(
    state: &ServerState,
    id: String,
    selected_leaf: Option<String>,
    format: ConversationExportFormat,
) -> AppResult<(Vec<u8>, ConversationExportEstimate)> {
    let store = state.session()?.store.clone();
    let pdf_permit = if format == ConversationExportFormat::Pdf {
        Some(
            state
                .pdf_export_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| AppError::Internal)?,
        )
    } else {
        None
    };
    tokio::task::spawn_blocking(move || {
        let _pdf_permit = pdf_permit;
        let detail = store.conversation_detail(&id, selected_leaf.as_deref())?;
        serialize_conversation_export(&detail, format)
    })
    .await
    .map_err(|_| AppError::Internal)?
}

async fn build_conversation_set_export(
    state: &ServerState,
    ids: Vec<String>,
    format: ConversationExportFormat,
) -> AppResult<(Vec<u8>, ConversationExportEstimate)> {
    let ids = validate_conversation_set_ids(ids)?;
    let store = state.session()?.store.clone();
    let pdf_permit = if format == ConversationExportFormat::Pdf {
        Some(
            state
                .pdf_export_slots
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| AppError::Internal)?,
        )
    } else {
        None
    };
    tokio::task::spawn_blocking(move || {
        let _pdf_permit = pdf_permit;
        let mut details = Vec::with_capacity(ids.len());
        let mut message_count = 0_usize;
        let mut attachment_count = 0_usize;
        let mut projected_text_bytes = 0_usize;
        for id in &ids {
            let detail = store.conversation_detail(id, None)?;
            message_count = message_count
                .checked_add(detail.messages.len())
                .ok_or(ErrorCode::ResourceLimit)?;
            projected_text_bytes = projected_text_bytes
                .checked_add(detail.title.len())
                .ok_or(ErrorCode::ResourceLimit)?;
            for message in &detail.messages {
                attachment_count = attachment_count
                    .checked_add(message.attachments.len())
                    .ok_or(ErrorCode::ResourceLimit)?;
                projected_text_bytes = projected_text_bytes
                    .checked_add(message.role.len())
                    .and_then(|total| total.checked_add(message.text.len()))
                    .ok_or(ErrorCode::ResourceLimit)?;
            }
            if message_count > MAX_CONVERSATION_SET_MESSAGES
                || attachment_count > MAX_CONVERSATION_SET_ATTACHMENTS
                || projected_text_bytes > MAX_CONVERSATION_EXPORT_BYTES
            {
                return Err(ErrorCode::ResourceLimit.into());
            }
            details.push(detail);
        }
        serialize_conversation_set_export(&details, format)
    })
    .await
    .map_err(|_| AppError::Internal)?
}

fn validate_conversation_set_ids(ids: Vec<String>) -> AppResult<Vec<String>> {
    if ids.is_empty() || ids.len() > MAX_CONVERSATION_SET_SIZE {
        return Err(ErrorCode::ResourceLimit.into());
    }
    let mut seen = HashSet::with_capacity(ids.len());
    for id in &ids {
        if id.len() != 32
            || !id.bytes().all(|byte| byte.is_ascii_hexdigit())
            || !seen.insert(id.clone())
        {
            return Err(ErrorCode::InvalidRequest.into());
        }
    }
    Ok(ids)
}

fn create_private_file(path: &Path) -> AppResult<std::fs::File> {
    let mut options = std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    #[cfg(unix)]
    {
        use std::os::unix::fs::OpenOptionsExt;
        options.mode(0o600);
    }
    options
        .open(path)
        .map_err(|_| ErrorCode::PathRejected.into())
}

fn write_private_destination(
    destination: &Path,
    write: impl FnOnce(&mut std::fs::File) -> AppResult<()>,
) -> AppResult<()> {
    if let Ok(metadata) = std::fs::symlink_metadata(destination)
        && (metadata.file_type().is_symlink() || !metadata.is_file())
    {
        return Err(ErrorCode::PathRejected.into());
    }
    let parent = destination
        .parent()
        .filter(|path| !path.as_os_str().is_empty())
        .ok_or(ErrorCode::PathRejected)?;
    let parent_metadata =
        std::fs::symlink_metadata(parent).map_err(|_| ErrorCode::PathRejected)?;
    if !parent_metadata.is_dir() {
        return Err(ErrorCode::PathRejected.into());
    }

    let mut random_bytes = [0_u8; 18];
    rand::rng().fill(&mut random_bytes);
    let temporary_name = format!(
        ".chatgpt-history-browser-{}.tmp",
        URL_SAFE_NO_PAD.encode(random_bytes)
    );
    let temporary_path = parent.join(temporary_name);
    let mut temporary = create_private_file(&temporary_path)?;
    let result = (|| {
        write(&mut temporary)?;
        temporary.sync_all().map_err(|_| ErrorCode::Internal)?;
        drop(temporary);
        std::fs::rename(&temporary_path, destination).map_err(|_| ErrorCode::PathRejected)?;
        Ok(())
    })();
    if result.is_err() {
        let _ = std::fs::remove_file(&temporary_path);
    }
    result
}

async fn attachment_content(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Response> {
    let session = state.session()?;
    let store = session.store.clone();
    let record = tokio::task::spawn_blocking(move || store.attachment_record(&id))
        .await
        .map_err(|_| AppError::Internal)??;
    if matches!(
        record.preview_kind,
        PreviewKind::Unsupported | PreviewKind::Missing | PreviewKind::Text
    ) {
        return Err(ErrorCode::UnsupportedPreview.into());
    }
    if record.byte_size > MAX_INLINE_MEDIA_BYTES {
        return Err(ErrorCode::ResourceLimit.into());
    }
    let root = session.root.clone();
    let source_name = record.source_name.clone();
    let validated =
        tokio::task::spawn_blocking(move || open_validated_preview(&root, &source_name))
            .await
            .map_err(|_| AppError::Internal)??;
    if validated.preview_kind != record.preview_kind
        || validated.byte_size != record.byte_size
        || matches!(
            validated.preview_kind,
            PreviewKind::Unsupported | PreviewKind::Missing | PreviewKind::Text
        )
    {
        return Err(ErrorCode::UnsupportedPreview.into());
    }
    let (slot_permit, byte_permit) = acquire_preview_budget(&state, validated.byte_size)?;
    let reader = GuardedPreviewReader {
        file: tokio::fs::File::from_std(validated.file),
        _slot_permit: slot_permit,
        _byte_permit: byte_permit,
    };
    let stream = ReaderStream::with_capacity(reader, 64 * 1024);
    let mut response = Response::new(Body::from_stream(stream));
    *response.status_mut() = StatusCode::OK;
    response.headers_mut().insert(
        CONTENT_TYPE,
        HeaderValue::from_str(
            validated
                .detected_mime
                .as_deref()
                .unwrap_or("application/octet-stream"),
        )
        .unwrap_or_else(|_| HeaderValue::from_static("application/octet-stream")),
    );
    response.headers_mut().insert(
        CONTENT_LENGTH,
        HeaderValue::from_str(&validated.byte_size.to_string())
            .map_err(|_| ErrorCode::Internal)?,
    );
    response.headers_mut().insert(
        CONTENT_DISPOSITION,
        HeaderValue::from_static("inline; filename=\"attachment-preview.bin\""),
    );
    Ok(response)
}

fn acquire_preview_budget(
    state: &ServerState,
    byte_size: u64,
) -> AppResult<(OwnedSemaphorePermit, OwnedSemaphorePermit)> {
    let slot_permit = state
        .preview_slots
        .clone()
        .try_acquire_owned()
        .map_err(|_| ErrorCode::ResourceLimit)?;
    let byte_units = byte_size.div_ceil(PREVIEW_BUDGET_UNIT).max(1);
    let byte_units = u32::try_from(byte_units).map_err(|_| ErrorCode::ResourceLimit)?;
    let byte_permit = state
        .preview_bytes
        .clone()
        .try_acquire_many_owned(byte_units)
        .map_err(|_| ErrorCode::ResourceLimit)?;
    Ok((slot_permit, byte_permit))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct TextPreview {
    text: String,
}

async fn attachment_text(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<TextPreview>> {
    let session = state.session()?;
    let store = session.store.clone();
    let record = tokio::task::spawn_blocking(move || store.attachment_record(&id))
        .await
        .map_err(|_| AppError::Internal)??;
    if record.preview_kind != PreviewKind::Text {
        return Err(ErrorCode::UnsupportedPreview.into());
    }
    let root = session.root.clone();
    let source_name = record.source_name;
    let text = tokio::task::spawn_blocking(move || read_text_preview(&root, &source_name))
        .await
        .map_err(|_| AppError::Internal)??;
    Ok(Json(TextPreview { text }))
}

#[derive(Serialize)]
#[serde(rename_all = "camelCase")]
struct SaveResult {
    saved: bool,
    file_name: Option<String>,
}

async fn save_attachment(
    State(state): State<ServerState>,
    AxumPath(id): AxumPath<String>,
) -> AppResult<Json<SaveResult>> {
    let session = state.session()?;
    let store = session.store.clone();
    let record = tokio::task::spawn_blocking(move || store.attachment_record(&id))
        .await
        .map_err(|_| AppError::Internal)??;
    let source_root = session.root.clone();
    let source_name = record.source_name;
    let validated =
        tokio::task::spawn_blocking(move || open_validated_preview(&source_root, &source_name))
            .await
            .map_err(|_| AppError::Internal)??;
    let dispatcher = state
        .main_thread_dispatcher
        .as_ref()
        .ok_or(AppError::Internal)?;
    let download_name = safe_download_name_for_detected_type(
        &record.display_name,
        validated.detected_mime.as_deref(),
        validated.detected_extension.as_deref(),
        validated.preview_kind,
    );
    let required_extension = Path::new(&download_name)
        .extension()
        .and_then(|extension| extension.to_str())
        .unwrap_or("bin")
        .to_string();
    let dialog_name = download_name.clone();
    let dialog_extension = required_extension.clone();
    let selected = build_future_on_main_thread(dispatcher, move || {
        let selection = rfd::AsyncFileDialog::new()
            .set_title("Save a copy of the attachment")
            .set_file_name(dialog_name)
            .add_filter("Detected file type", &[dialog_extension])
            .save_file();
        async move {
            selection
                .await
                .map(|selected| selected.path().to_path_buf())
        }
    })
    .await?;
    let Some(destination) = selected else {
        return Ok(Json(SaveResult {
            saved: false,
            file_name: None,
        }));
    };
    let destination = with_required_extension(destination, &required_extension);
    let saved_file_name = destination
        .file_name()
        .and_then(|name| name.to_str())
        .map(ToOwned::to_owned)
        .unwrap_or(download_name);
    let root = session.root.clone();
    tokio::task::spawn_blocking(move || -> AppResult<()> {
        if !root.write_destination_is_outside_root(&destination) {
            return Err(ErrorCode::PathRejected.into());
        }
        let mut source = validated.file;
        write_private_destination(&destination, |target| {
            std::io::copy(&mut source, target).map_err(|_| ErrorCode::Internal)?;
            Ok(())
        })
    })
    .await
    .map_err(|_| AppError::Internal)??;
    Ok(Json(SaveResult {
        saved: true,
        file_name: Some(saved_file_name),
    }))
}

fn with_required_extension(mut path: PathBuf, extension: &str) -> PathBuf {
    let current = path.extension().and_then(|value| value.to_str());
    if !current.is_some_and(|value| value.eq_ignore_ascii_case(extension)) {
        path.set_extension(extension);
    }
    path
}

fn validate_web_root(path: &Path) -> AppResult<()> {
    let metadata = std::fs::symlink_metadata(path).map_err(|_| ErrorCode::Internal)?;
    if metadata.file_type().is_symlink() || is_reparse_metadata(&metadata) || !metadata.is_dir()
    {
        return Err(ErrorCode::Internal.into());
    }
    let canonical_root = std::fs::canonicalize(path).map_err(|_| ErrorCode::Internal)?;
    let mut stack = vec![(canonical_root.clone(), 0_usize)];
    let mut file_count = 0_usize;
    let mut total_bytes = 0_u64;
    let mut has_index = false;

    while let Some((directory, depth)) = stack.pop() {
        if depth > MAX_WEB_ASSET_DEPTH {
            return Err(ErrorCode::Internal.into());
        }
        let entries = std::fs::read_dir(&directory).map_err(|_| ErrorCode::Internal)?;
        for entry in entries {
            let entry = entry.map_err(|_| ErrorCode::Internal)?;
            let candidate = entry.path();
            let metadata =
                std::fs::symlink_metadata(&candidate).map_err(|_| ErrorCode::Internal)?;
            if metadata.file_type().is_symlink() || is_reparse_metadata(&metadata) {
                return Err(ErrorCode::Internal.into());
            }
            if metadata.is_dir() {
                stack.push((candidate, depth.saturating_add(1)));
                continue;
            }
            if !metadata.is_file() || static_file_has_multiple_links(&metadata) {
                return Err(ErrorCode::Internal.into());
            }
            file_count = file_count.saturating_add(1);
            total_bytes = total_bytes
                .checked_add(metadata.len())
                .ok_or(ErrorCode::Internal)?;
            if file_count > MAX_WEB_ASSET_COUNT || total_bytes > MAX_WEB_ASSET_BYTES {
                return Err(ErrorCode::Internal.into());
            }
            if candidate == canonical_root.join("index.html") {
                has_index = true;
            }
        }
    }
    if !has_index {
        return Err(ErrorCode::Internal.into());
    }
    Ok(())
}

#[cfg(windows)]
fn is_reparse_metadata(metadata: &std::fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn is_reparse_metadata(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(unix)]
fn static_file_has_multiple_links(metadata: &std::fs::Metadata) -> bool {
    use std::os::unix::fs::MetadataExt;
    metadata.nlink() > 1
}

#[cfg(not(unix))]
fn static_file_has_multiple_links(_metadata: &std::fs::Metadata) -> bool {
    false
}

#[cfg(test)]
mod tests {
    use std::fs;

    use axum::body::to_bytes;
    use tempfile::TempDir;
    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use tower::ServiceExt;

    use super::*;

    fn test_router() -> (TempDir, Router, ServerState) {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("index.html"), "<!doctype html>").expect("write html");
        let state = ServerState::new(
            "synthetic-session-token".to_string(),
            "127.0.0.1:41000".to_string(),
            "http://127.0.0.1:41000".to_string(),
        );
        let router = build_router(state.clone(), directory.path().to_path_buf());
        (directory, router, state)
    }

    fn request(path: &str, state: &ServerState) -> Request<Body> {
        Request::builder()
            .uri(path)
            .header(HOST, state.expected_host.as_str())
            .header(AUTHORIZATION, format!("Bearer {}", state.token.as_str()))
            .body(Body::empty())
            .expect("request")
    }

    async fn loopback_request(address: std::net::SocketAddr, request: &[u8]) -> Vec<u8> {
        let mut stream = tokio::net::TcpStream::connect(address)
            .await
            .expect("connect to loopback server");
        stream.write_all(request).await.expect("write request");
        let mut response = Vec::new();
        stream
            .read_to_end(&mut response)
            .await
            .expect("read response");
        response
    }

    #[tokio::test]
    async fn native_dialog_future_is_built_on_the_dispatched_thread() {
        let dispatcher: MainThreadDispatcher = Arc::new(|task| {
            std::thread::Builder::new()
                .name("synthetic-main-thread".to_string())
                .spawn(task)
                .map_err(|_| AppError::Internal)?;
            Ok(())
        });

        let builder_thread = build_future_on_main_thread(&dispatcher, || {
            let builder_thread = std::thread::current()
                .name()
                .unwrap_or("unnamed")
                .to_string();
            std::future::ready(builder_thread)
        })
        .await
        .expect("build and await synthetic future");

        assert_eq!(builder_thread, "synthetic-main-thread");
    }

    #[tokio::test]
    async fn rejects_missing_or_wrong_host_and_token() {
        let (_directory, router, state) = test_router();
        let missing = router
            .clone()
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(missing.status(), StatusCode::UNAUTHORIZED);

        let wrong = router
            .oneshot(
                Request::builder()
                    .uri("/api/status")
                    .header(HOST, state.expected_host.as_str())
                    .header(AUTHORIZATION, "Bearer wrong")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(wrong.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn protected_status_has_security_headers_and_no_paths() {
        let (_directory, router, state) = test_router();
        let response = router
            .oneshot(request("/api/status", &state))
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::OK);
        assert_eq!(
            response
                .headers()
                .get("x-content-type-options")
                .and_then(|value| value.to_str().ok()),
            Some("nosniff")
        );
        let body = to_bytes(response.into_body(), 64 * 1024)
            .await
            .expect("body");
        let text = String::from_utf8(body.to_vec()).expect("utf8");
        let local_temp = std::env::temp_dir().to_string_lossy().into_owned();
        assert!(!text.contains(&local_temp));
    }

    #[tokio::test]
    async fn foreign_origin_cannot_mutate_state() {
        let (_directory, router, state) = test_router();
        let response = router
            .oneshot(
                Request::builder()
                    .method(Method::POST)
                    .uri("/api/index/start")
                    .header(HOST, state.expected_host.as_str())
                    .header(AUTHORIZATION, format!("Bearer {}", state.token.as_str()))
                    .header(ORIGIN, "https://example.invalid")
                    .body(Body::empty())
                    .expect("request"),
            )
            .await
            .expect("response");
        assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
    }

    #[tokio::test]
    async fn real_loopback_transport_enforces_capability_and_headers() {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("index.html"), "<!doctype html>").expect("write html");
        let (listener, state, bound) =
            bind_loopback(directory.path().to_path_buf()).expect("bind loopback");
        let address = listener.local_addr().expect("listener address");
        let listener = tokio::net::TcpListener::from_std(listener).expect("tokio listener");
        let shutdown = bound.shutdown.clone();
        let router = build_router(state, directory.path().to_path_buf());
        let server = tokio::spawn(async move {
            axum::serve(listener, router)
                .with_graceful_shutdown(shutdown.cancelled_owned())
                .await
                .expect("serve loopback");
        });

        let authorized = format!(
            "GET /api/status HTTP/1.1\r\nHost: {}\r\nAuthorization: Bearer {}\r\nConnection: close\r\n\r\n",
            address, bound.token
        );
        let authorized = loopback_request(address, authorized.as_bytes()).await;
        let authorized = String::from_utf8(authorized).expect("HTTP response is UTF-8");
        assert!(authorized.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(authorized.contains("content-security-policy: default-src 'none'"));
        assert!(authorized.contains("cache-control: no-store"));

        let unauthorized = format!(
            "GET /api/status HTTP/1.1\r\nHost: {}\r\nConnection: close\r\n\r\n",
            address
        );
        let unauthorized = loopback_request(address, unauthorized.as_bytes()).await;
        let unauthorized = String::from_utf8(unauthorized).expect("HTTP response is UTF-8");
        assert!(unauthorized.starts_with("HTTP/1.1 401 Unauthorized\r\n"));

        bound.shutdown.cancel();
        server.await.expect("server task");
    }

    #[test]
    fn loopback_binding_uses_distinct_ephemeral_ipv4_ports_and_tokens() {
        let first_directory = TempDir::new().expect("first temp directory");
        let second_directory = TempDir::new().expect("second temp directory");
        fs::write(first_directory.path().join("index.html"), "<!doctype html>")
            .expect("write first html");
        fs::write(
            second_directory.path().join("index.html"),
            "<!doctype html>",
        )
        .expect("write second html");
        let (first_listener, _, first_bound) =
            bind_loopback(first_directory.path().to_path_buf()).expect("first bind");
        let (second_listener, _, second_bound) =
            bind_loopback(second_directory.path().to_path_buf()).expect("second bind");
        let first_address = first_listener.local_addr().expect("first address");
        let second_address = second_listener.local_addr().expect("second address");
        assert_eq!(
            first_address.ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert_eq!(
            second_address.ip(),
            std::net::IpAddr::V4(Ipv4Addr::LOCALHOST)
        );
        assert_ne!(first_address.port(), 0);
        assert_ne!(second_address.port(), 0);
        assert_ne!(first_address.port(), second_address.port());
        assert_ne!(first_bound.token, second_bound.token);
        assert!(first_bound.origin.starts_with("http://127.0.0.1:"));
        assert!(second_bound.origin.starts_with("http://127.0.0.1:"));
        assert_eq!(first_bound.token.len(), 43);
        assert_eq!(second_bound.token.len(), 43);
    }

    #[test]
    fn preview_admission_caps_concurrency_and_aggregate_bytes() {
        let state = ServerState::new(
            "synthetic-session-token".to_string(),
            "127.0.0.1:41000".to_string(),
            "http://127.0.0.1:41000".to_string(),
        );
        let permits = (0..MAX_CONCURRENT_PREVIEWS)
            .map(|_| acquire_preview_budget(&state, PREVIEW_BUDGET_UNIT))
            .collect::<AppResult<Vec<_>>>()
            .expect("acquire bounded preview slots");
        assert!(acquire_preview_budget(&state, PREVIEW_BUDGET_UNIT).is_err());
        drop(permits);

        let full_budget =
            acquire_preview_budget(&state, MAX_INLINE_MEDIA_BYTES).expect("full byte budget");
        assert!(acquire_preview_budget(&state, 1).is_err());
        drop(full_budget);
    }

    #[test]
    fn selected_conversation_ids_are_opaque_unique_and_bounded() {
        let first = "0123456789abcdef0123456789abcdef".to_string();
        let second = "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa".to_string();
        assert_eq!(
            validate_conversation_set_ids(vec![first.clone(), second.clone()])
                .expect("valid selection"),
            vec![first.clone(), second]
        );

        assert_eq!(
            validate_conversation_set_ids(Vec::new())
                .expect_err("empty selection")
                .code(),
            ErrorCode::ResourceLimit
        );
        assert_eq!(
            validate_conversation_set_ids(vec![first.clone(), first])
                .expect_err("duplicate selection")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            validate_conversation_set_ids(vec!["../synthetic".to_string()])
                .expect_err("invalid identifier")
                .code(),
            ErrorCode::InvalidRequest
        );
        assert_eq!(
            validate_conversation_set_ids(vec![
                "bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb".to_string();
                MAX_CONVERSATION_SET_SIZE + 1
            ])
            .expect_err("oversized selection")
            .code(),
            ErrorCode::ResourceLimit
        );
    }

    #[test]
    fn save_destinations_keep_the_selected_export_extension() {
        assert_eq!(
            with_required_extension(PathBuf::from("/tmp/Synthetic recording"), "wav"),
            PathBuf::from("/tmp/Synthetic recording.wav")
        );
        assert_eq!(
            with_required_extension(PathBuf::from("/tmp/Synthetic notes.txt"), "pdf"),
            PathBuf::from("/tmp/Synthetic notes.pdf")
        );
        assert_eq!(
            with_required_extension(PathBuf::from("/tmp/Synthetic notes.MD"), "md"),
            PathBuf::from("/tmp/Synthetic notes.MD")
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_destination_atomically_replaces_a_confirmed_regular_file() {
        use std::os::unix::fs::PermissionsExt;

        let directory = TempDir::new().expect("temp directory");
        let destination = directory.path().join("Synthetic export.txt");
        fs::write(&destination, "old content").expect("write old destination");

        write_private_destination(&destination, |target| {
            target
                .write_all(b"replacement content")
                .map_err(|_| ErrorCode::Internal)?;
            Ok(())
        })
        .expect("replace destination");

        assert_eq!(
            fs::read_to_string(&destination).expect("read replacement"),
            "replacement content"
        );
        assert_eq!(
            fs::metadata(&destination)
                .expect("replacement metadata")
                .permissions()
                .mode()
                & 0o777,
            0o600
        );
        assert_eq!(
            fs::read_dir(directory.path())
                .expect("read temp directory")
                .count(),
            1
        );
    }

    #[cfg(unix)]
    #[test]
    fn private_destination_rejects_an_existing_symlink() {
        let directory = TempDir::new().expect("temp directory");
        let outside = directory.path().join("outside.txt");
        let destination = directory.path().join("destination.txt");
        fs::write(&outside, "untouched").expect("write outside file");
        std::os::unix::fs::symlink(&outside, &destination).expect("create symlink");

        let result = write_private_destination(&destination, |target| {
            target
                .write_all(b"replacement")
                .map_err(|_| ErrorCode::Internal)?;
            Ok(())
        });

        assert!(result.is_err());
        assert_eq!(
            fs::read_to_string(&outside).expect("read outside file"),
            "untouched"
        );
    }

    #[cfg(unix)]
    #[test]
    fn rejects_symlinked_static_assets() {
        let directory = TempDir::new().expect("temp directory");
        fs::write(directory.path().join("index.html"), "<!doctype html>").expect("write html");
        let outside = directory.path().with_extension("synthetic-outside");
        fs::write(&outside, "synthetic").expect("write outside asset");
        std::os::unix::fs::symlink(&outside, directory.path().join("asset.js"))
            .expect("create symlink");
        assert!(validate_web_root(directory.path()).is_err());
        fs::remove_file(outside).expect("remove outside asset");
    }
}
