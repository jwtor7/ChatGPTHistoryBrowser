use std::{
    collections::HashSet,
    fs::{self, File, OpenOptions},
    io::{Read, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicBool, Ordering},
    time::{Duration, SystemTime, UNIX_EPOCH},
};

use directories::ProjectDirs;
use fs2::FileExt;
use rusqlite::{
    Connection, OpenFlags, OptionalExtension, Transaction, params, params_from_iter,
    types::Value as SqlValue,
};
use serde_json::Value;

use crate::{
    attachments::{ResolvedAttachment, resolve_attachment},
    conversation::project_conversation,
    error::{AppResult, ErrorCode},
    json_stream::{JsonStreamLimits, stream_json_array},
    models::{
        AttachmentKindFilter, AttachmentStatus, AttachmentView, BranchView, ConversationDetail,
        ConversationListItem, ConversationPage, ConversationQuery, DiagnosticView, MessageView,
        PreviewKind, ProjectedConversation,
    },
    safe_root::{SafeExportRoot, SafeFileEntry},
};

const INDEX_MARKER: &str = "chatgpt-history-browser-index-v1\n";
const INDEX_PROJECTION_VERSION: &str = "5";
const MAX_PAGE_SIZE: u32 = 100;
const MAX_PAGE_NUMBER: u32 = 100_000;
const MAX_SEARCH_BYTES: usize = 512;
const MAX_SEARCH_TERMS: usize = 16;
const MAX_DETAIL_PATH: usize = 50_000;
const MAX_INDEX_BYTES: u64 = 16 * 1024 * 1024 * 1024;
const MIN_FREE_SPACE_BYTES: u64 = 512 * 1024 * 1024;
const SOURCE_TO_INDEX_ESTIMATE: u64 = 4;
const QUOTA_CHECK_INTERVAL: u64 = 16;

#[derive(Clone)]
pub struct Store {
    directory: PathBuf,
    database_path: PathBuf,
}

pub struct StoreLock {
    file: File,
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = FileExt::unlock(&self.file);
    }
}

#[derive(Debug, Clone, Copy, Default)]
pub struct ShardIndexStats {
    pub conversations_indexed: u64,
    pub conversations_skipped: u64,
    pub diagnostics: u64,
    pub bytes_processed: u64,
}

#[derive(Clone)]
pub struct AttachmentRecord {
    pub id: String,
    pub display_name: String,
    pub source_name: String,
    pub detected_mime: Option<String>,
    pub byte_size: u64,
    pub preview_kind: PreviewKind,
}

impl Store {
    pub fn for_export(root: &SafeExportRoot) -> AppResult<Self> {
        let project_dirs = ProjectDirs::from(
            "app",
            "ChatGPT History Browser Contributors",
            "ChatGPT History Browser",
        )
        .ok_or(ErrorCode::IndexUnavailable)?;
        Self::for_export_with_cache_root(root, project_dirs.cache_dir())
    }

    pub fn for_export_with_cache_root(
        root: &SafeExportRoot,
        cache_root: &Path,
    ) -> AppResult<Self> {
        let canonical_cache_root = ensure_private_cache_root(cache_root)?;
        let indexes = canonical_cache_root.join("indexes");
        ensure_private_directory(&indexes, &canonical_cache_root)?;
        let canonical_indexes =
            fs::canonicalize(&indexes).map_err(|_| ErrorCode::IndexUnavailable)?;
        let directory = canonical_indexes.join(root.opaque_cache_key());
        ensure_private_directory(&directory, &canonical_indexes)?;
        if !root.cache_is_outside_root(&directory) {
            return Err(ErrorCode::PathRejected.into());
        }
        initialize_index_marker(&directory)?;

        let store = Self {
            database_path: directory.join("index.sqlite3"),
            directory,
        };
        store.initialize()?;
        Ok(store)
    }

    pub fn initialize(&self) -> AppResult<()> {
        let connection = self.open_connection()?;
        connection.execute_batch(
            r#"
            CREATE TABLE IF NOT EXISTS shards (
                name TEXT PRIMARY KEY,
                byte_size INTEGER NOT NULL,
                modified_nanos TEXT NOT NULL,
                content_hash TEXT NOT NULL,
                conversation_count INTEGER NOT NULL,
                indexed_at REAL NOT NULL
            );

            CREATE TABLE IF NOT EXISTS archive_state (
                key TEXT PRIMARY KEY,
                value TEXT NOT NULL
            );

            CREATE TABLE IF NOT EXISTS conversations (
                conversation_key TEXT PRIMARY KEY,
                shard_name TEXT NOT NULL,
                shard_ordinal INTEGER NOT NULL,
                title TEXT NOT NULL,
                created_at REAL,
                updated_at REAL,
                archived INTEGER,
                starred INTEGER,
                current_node TEXT,
                message_count INTEGER NOT NULL,
                has_attachments INTEGER NOT NULL,
                FOREIGN KEY (shard_name) REFERENCES shards(name) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS conversations_sort
                ON conversations(updated_at DESC, created_at DESC, conversation_key);
            CREATE INDEX IF NOT EXISTS conversations_shard
                ON conversations(shard_name);

            CREATE TABLE IF NOT EXISTS nodes (
                conversation_key TEXT NOT NULL,
                node_id TEXT NOT NULL,
                parent_node_id TEXT,
                message_id TEXT,
                role TEXT NOT NULL,
                created_at REAL,
                content_type TEXT NOT NULL,
                message_text TEXT NOT NULL,
                has_attachments INTEGER NOT NULL,
                PRIMARY KEY (conversation_key, node_id),
                FOREIGN KEY (conversation_key) REFERENCES conversations(conversation_key)
                    ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS nodes_parent
                ON nodes(conversation_key, parent_node_id);
            CREATE INDEX IF NOT EXISTS nodes_role
                ON nodes(conversation_key, role);

            CREATE TABLE IF NOT EXISTS attachments (
                attachment_key TEXT PRIMARY KEY,
                conversation_key TEXT NOT NULL,
                node_id TEXT NOT NULL,
                ordinal INTEGER NOT NULL,
                display_name TEXT NOT NULL,
                source_name TEXT,
                claimed_mime TEXT,
                detected_mime TEXT,
                byte_size INTEGER,
                status TEXT NOT NULL,
                preview_kind TEXT NOT NULL,
                FOREIGN KEY (conversation_key, node_id)
                    REFERENCES nodes(conversation_key, node_id) ON DELETE CASCADE
            );
            CREATE INDEX IF NOT EXISTS attachments_message
                ON attachments(conversation_key, node_id, ordinal);
            CREATE INDEX IF NOT EXISTS attachments_kind
                ON attachments(preview_kind, conversation_key);
            CREATE INDEX IF NOT EXISTS attachments_detected_kind
                ON attachments(detected_mime, status, conversation_key);

            CREATE TABLE IF NOT EXISTS diagnostics (
                conversation_key TEXT NOT NULL,
                code TEXT NOT NULL,
                occurrence_count INTEGER NOT NULL,
                PRIMARY KEY (conversation_key, code),
                FOREIGN KEY (conversation_key) REFERENCES conversations(conversation_key)
                    ON DELETE CASCADE
            );

            CREATE VIRTUAL TABLE IF NOT EXISTS conversation_fts USING fts5(
                conversation_key UNINDEXED,
                title,
                body,
                tokenize = 'unicode61 remove_diacritics 2'
            );
            "#,
        )?;
        tighten_database_files(&self.database_path)?;
        Ok(())
    }

    pub fn acquire_index_lock(&self) -> AppResult<StoreLock> {
        self.validate_directory()?;
        let path = self.directory.join(".index-lock");
        let file = OpenOptions::new()
            .create(true)
            .read(true)
            .write(true)
            .truncate(false)
            .open(&path)
            .map_err(|_| ErrorCode::IndexUnavailable)?;
        set_private_file_permissions(&path)?;
        set_private_file_permissions_from_handle(&file)?;
        file.try_lock_exclusive()
            .map_err(|_| ErrorCode::IndexBusy)?;
        Ok(StoreLock { file })
    }

    pub fn shard_is_current(
        &self,
        root: &SafeExportRoot,
        shard: &SafeFileEntry,
    ) -> AppResult<bool> {
        let connection = self.open_connection()?;
        let projection_version = connection
            .query_row(
                "SELECT value FROM archive_state WHERE key = 'projection_version'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        if projection_version.as_deref() != Some(INDEX_PROJECTION_VERSION) {
            return Ok(false);
        }
        let current = connection
            .query_row(
                "SELECT byte_size, modified_nanos, content_hash
                 FROM shards WHERE name = ?1",
                [shard.name.as_str()],
                |row| {
                    Ok((
                        row.get::<_, i64>(0)?,
                        row.get::<_, String>(1)?,
                        row.get::<_, String>(2)?,
                    ))
                },
            )
            .optional()?;
        let Some((size, modified, expected_hash)) = current else {
            return Ok(false);
        };
        if u64::try_from(size).ok() != Some(shard.size)
            || modified != shard.modified_nanos.to_string()
            || expected_hash.is_empty()
        {
            return Ok(false);
        }

        let mut file = root.open_entry(shard)?;
        let mut hasher = blake3::Hasher::new();
        let mut buffer = [0_u8; 64 * 1024];
        loop {
            let count = file.read(&mut buffer)?;
            if count == 0 {
                break;
            }
            hasher.update(&buffer[..count]);
        }
        Ok(hasher.finalize().to_hex().as_str() == expected_hash)
    }

    pub fn attachment_inventory_is_current(&self, fingerprint: &str) -> AppResult<bool> {
        let connection = self.open_connection_read_only()?;
        let current = connection
            .query_row(
                "SELECT value FROM archive_state WHERE key = 'attachment_inventory'",
                [],
                |row| row.get::<_, String>(0),
            )
            .optional()?;
        Ok(current.as_deref() == Some(fingerprint))
    }

    pub fn finalize_archive_state(
        &self,
        shards: &[SafeFileEntry],
        attachment_fingerprint: &str,
    ) -> AppResult<()> {
        let current_shards = shards
            .iter()
            .map(|shard| shard.name.as_str())
            .collect::<HashSet<_>>();
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let mut statement = transaction.prepare("SELECT name FROM shards")?;
        let existing = statement
            .query_map([], |row| row.get::<_, String>(0))?
            .collect::<Result<Vec<_>, _>>()?;
        drop(statement);

        for stale in existing
            .iter()
            .filter(|name| !current_shards.contains(name.as_str()))
        {
            transaction.execute(
                "DELETE FROM conversation_fts
                 WHERE conversation_key IN (
                     SELECT conversation_key FROM conversations WHERE shard_name = ?1
                 )",
                [stale],
            )?;
            transaction.execute("DELETE FROM conversations WHERE shard_name = ?1", [stale])?;
            transaction.execute("DELETE FROM shards WHERE name = ?1", [stale])?;
        }
        transaction.execute(
            "INSERT INTO archive_state (key, value)
             VALUES ('attachment_inventory', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [attachment_fingerprint],
        )?;
        transaction.execute(
            "INSERT INTO archive_state (key, value)
             VALUES ('projection_version', ?1)
             ON CONFLICT(key) DO UPDATE SET value = excluded.value",
            [INDEX_PROJECTION_VERSION],
        )?;
        transaction.commit()?;
        Ok(())
    }

    pub fn index_shard<F>(
        &self,
        root: &SafeExportRoot,
        shard: &SafeFileEntry,
        cancelled: &AtomicBool,
        mut on_bytes: F,
    ) -> AppResult<ShardIndexStats>
    where
        F: FnMut(u64),
    {
        if cancelled.load(Ordering::Relaxed) {
            return Err(ErrorCode::IndexCancelled.into());
        }
        self.ensure_capacity_before_index(shard.size)?;
        let source = root.open_entry(shard)?;
        let mut reader = HashingReader::new(source, &mut on_bytes);
        let mut connection = self.open_connection()?;
        let transaction = connection.transaction()?;
        let shard_size = sql_i64_from_u64(shard.size)?;

        transaction.execute(
            "DELETE FROM conversation_fts
             WHERE conversation_key IN (
                 SELECT conversation_key FROM conversations WHERE shard_name = ?1
             )",
            [shard.name.as_str()],
        )?;
        transaction.execute(
            "DELETE FROM conversations WHERE shard_name = ?1",
            [shard.name.as_str()],
        )?;
        transaction.execute("DELETE FROM shards WHERE name = ?1", [shard.name.as_str()])?;
        transaction.execute(
            "INSERT INTO shards
             (name, byte_size, modified_nanos, content_hash, conversation_count, indexed_at)
             VALUES (?1, ?2, ?3, '', 0, ?4)",
            params![
                shard.name,
                shard_size,
                shard.modified_nanos.to_string(),
                unix_time_seconds()
            ],
        )?;

        let mut stats = ShardIndexStats::default();
        let stream_stats = stream_json_array(
            &mut reader,
            JsonStreamLimits::default(),
            |record, ordinal| {
                if cancelled.load(Ordering::Relaxed) {
                    return Err(ErrorCode::IndexCancelled.into());
                }
                let raw: Value = match serde_json::from_slice(record) {
                    Ok(value) => value,
                    Err(_) => {
                        stats.conversations_skipped += 1;
                        stats.diagnostics += 1;
                        return Ok(());
                    }
                };
                let projected = match project_conversation(&raw, &shard.name, ordinal) {
                    Ok(value) => value,
                    Err(crate::error::AppError::Public(
                        ErrorCode::UnsupportedRecord
                        | ErrorCode::ResourceLimit
                        | ErrorCode::RecordTooLarge,
                    )) => {
                        stats.conversations_skipped += 1;
                        stats.diagnostics += 1;
                        return Ok(());
                    }
                    Err(error) => return Err(error),
                };
                insert_conversation(&transaction, root, shard, ordinal, &projected)?;
                stats.conversations_indexed += 1;
                stats.diagnostics = stats.diagnostics.saturating_add(
                    projected
                        .diagnostics
                        .iter()
                        .map(|diagnostic| u64::from(diagnostic.count))
                        .sum(),
                );
                if stats.conversations_indexed % QUOTA_CHECK_INTERVAL == 0 {
                    self.ensure_capacity_during_index()?;
                }
                Ok(())
            },
        )?;
        stats.conversations_skipped = stats
            .conversations_skipped
            .saturating_add(stream_stats.records_too_large);
        stats.diagnostics = stats
            .diagnostics
            .saturating_add(stream_stats.records_too_large);
        stats.bytes_processed = stream_stats.bytes_read;

        if cancelled.load(Ordering::Relaxed) {
            return Err(ErrorCode::IndexCancelled.into());
        }
        self.ensure_capacity_during_index()?;
        transaction.execute(
            "UPDATE shards
             SET content_hash = ?2, conversation_count = ?3, indexed_at = ?4
             WHERE name = ?1",
            params![
                shard.name,
                reader.hash_hex(),
                sql_i64_from_u64(stats.conversations_indexed)?,
                unix_time_seconds()
            ],
        )?;
        transaction.commit()?;
        tighten_database_files(&self.database_path)?;
        Ok(stats)
    }

    pub fn query_conversations(
        &self,
        query: &ConversationQuery,
    ) -> AppResult<ConversationPage> {
        let page = query.page.unwrap_or(0).min(MAX_PAGE_NUMBER);
        let page_size = query.page_size.unwrap_or(50).clamp(1, MAX_PAGE_SIZE);
        let search = query
            .search
            .as_deref()
            .map(str::trim)
            .filter(|value| !value.is_empty())
            .map(build_fts_query)
            .transpose()?;

        let mut conditions = Vec::<String>::new();
        let mut values = Vec::<SqlValue>::new();
        let join = if search.is_some() {
            " JOIN conversation_fts f ON f.conversation_key = c.conversation_key "
        } else {
            ""
        };
        if let Some(search) = search {
            conditions.push("conversation_fts MATCH ?".to_string());
            values.push(SqlValue::Text(search));
        }
        if let Some(date_from) = query.date_from.filter(|value| value.is_finite()) {
            conditions.push("COALESCE(c.updated_at, c.created_at, 0) >= ?".to_string());
            values.push(SqlValue::Real(date_from));
        }
        if let Some(date_to) = query.date_to.filter(|value| value.is_finite()) {
            conditions.push("COALESCE(c.updated_at, c.created_at, 0) <= ?".to_string());
            values.push(SqlValue::Real(date_to));
        }
        if let Some(archived) = query.archived {
            conditions.push(if archived {
                "c.archived = 1".to_string()
            } else {
                "COALESCE(c.archived, 0) = 0".to_string()
            });
        }
        if let Some(starred) = query.starred {
            conditions.push(if starred {
                "c.starred = 1".to_string()
            } else {
                "COALESCE(c.starred, 0) = 0".to_string()
            });
        }
        if let Some(has_attachments) = query.has_attachments {
            conditions.push(format!(
                "c.has_attachments = {}",
                i32::from(has_attachments)
            ));
        }
        if let Some(attachment_kind) = query.attachment_kind {
            let kind_condition = match attachment_kind {
                AttachmentKindFilter::Image => "kind_attachment.detected_mime LIKE 'image/%'",
                AttachmentKindFilter::Audio => "kind_attachment.detected_mime LIKE 'audio/%'",
                AttachmentKindFilter::Video => "kind_attachment.detected_mime LIKE 'video/%'",
                AttachmentKindFilter::Pdf => {
                    "kind_attachment.detected_mime = 'application/pdf'"
                }
                AttachmentKindFilter::Text => "kind_attachment.detected_mime LIKE 'text/%'",
                AttachmentKindFilter::Missing => "kind_attachment.status = 'missing'",
                AttachmentKindFilter::Other => {
                    "kind_attachment.status <> 'missing'
                    AND (
                        kind_attachment.detected_mime IS NULL
                        OR (
                            kind_attachment.detected_mime NOT LIKE 'image/%'
                            AND kind_attachment.detected_mime NOT LIKE 'audio/%'
                            AND kind_attachment.detected_mime NOT LIKE 'video/%'
                            AND kind_attachment.detected_mime NOT LIKE 'text/%'
                            AND kind_attachment.detected_mime <> 'application/pdf'
                        )
                    )"
                }
            };
            conditions.push(format!(
                "EXISTS (
                    SELECT 1 FROM attachments kind_attachment
                    WHERE kind_attachment.conversation_key = c.conversation_key
                    AND ({kind_condition})
                )"
            ));
        }
        if let Some(role) = query.role.as_deref().filter(|role| is_supported_role(role)) {
            conditions.push(
                "EXISTS (
                    SELECT 1 FROM nodes role_node
                    WHERE role_node.conversation_key = c.conversation_key
                    AND role_node.role = ?
                )"
                .to_string(),
            );
            values.push(SqlValue::Text(role.to_string()));
        }
        let where_sql = if conditions.is_empty() {
            String::new()
        } else {
            format!(" WHERE {}", conditions.join(" AND "))
        };

        let connection = self.open_connection_read_only()?;
        let count_sql = format!("SELECT COUNT(*) FROM conversations c {join}{where_sql}");
        let total = sql_u64_from_i64(connection.query_row(
            &count_sql,
            params_from_iter(values.clone()),
            |row| row.get::<_, i64>(0),
        )?)?;

        let preview_sql = if join.is_empty() {
            "NULL"
        } else {
            "snippet(conversation_fts, 2, '', '', ' … ', 18)"
        };
        let data_sql = format!(
            "SELECT c.conversation_key, c.title, c.created_at, c.updated_at,
                    c.archived, c.starred, c.has_attachments, c.message_count,
                    {preview_sql}
             FROM conversations c {join}{where_sql}
             ORDER BY COALESCE(c.updated_at, c.created_at, 0) DESC,
                      c.conversation_key ASC
             LIMIT ? OFFSET ?"
        );
        let offset = u64::from(page) * u64::from(page_size);
        values.push(SqlValue::Integer(i64::from(page_size)));
        values.push(SqlValue::Integer(offset.min(i64::MAX as u64) as i64));
        let mut statement = connection.prepare(&data_sql)?;
        let items = statement
            .query_map(params_from_iter(values), |row| {
                Ok(ConversationListItem {
                    id: row.get(0)?,
                    title: row.get(1)?,
                    created_at: row.get(2)?,
                    updated_at: row.get(3)?,
                    archived: optional_bool(row.get(4)?),
                    starred: optional_bool(row.get(5)?),
                    has_attachments: row.get::<_, i64>(6)? != 0,
                    message_count: row.get(7)?,
                    match_preview: row.get(8)?,
                })
            })?
            .collect::<Result<Vec<_>, _>>()?;
        let has_more = offset.saturating_add(items.len() as u64) < total;
        Ok(ConversationPage {
            items,
            page,
            page_size,
            total,
            has_more,
        })
    }

    pub fn conversation_detail(
        &self,
        conversation_key: &str,
        selected_leaf: Option<&str>,
    ) -> AppResult<ConversationDetail> {
        if !is_opaque_id(conversation_key) || selected_leaf.is_some_and(|id| !is_opaque_id(id))
        {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let connection = self.open_connection_read_only()?;
        let header = connection
            .query_row(
                "SELECT title, created_at, updated_at, archived, starred, current_node
                 FROM conversations WHERE conversation_key = ?1",
                [conversation_key],
                |row| {
                    Ok((
                        row.get::<_, String>(0)?,
                        row.get::<_, Option<f64>>(1)?,
                        row.get::<_, Option<f64>>(2)?,
                        row.get::<_, Option<i64>>(3)?,
                        row.get::<_, Option<i64>>(4)?,
                        row.get::<_, Option<String>>(5)?,
                    ))
                },
            )
            .optional()?
            .ok_or(ErrorCode::ConversationNotFound)?;
        let leaf = selected_leaf.map(ToOwned::to_owned).or(header.5.clone());
        let path = load_path(&connection, conversation_key, leaf.as_deref())?;
        let mut messages = Vec::new();
        for (index, node) in path.iter().enumerate() {
            let next_id = path.get(index + 1).map(|next| next.node_id.as_str());
            let branches =
                load_branches(&connection, conversation_key, &node.node_id, next_id)?;
            let attachments =
                load_message_attachments(&connection, conversation_key, &node.node_id)?;
            if !is_renderable_message(
                node.message_id.as_deref(),
                &node.text,
                !attachments.is_empty(),
                !branches.is_empty(),
            ) {
                continue;
            }
            messages.push(MessageView {
                node_id: node.node_id.clone(),
                role: node.role.clone(),
                created_at: node.created_at,
                content_type: node.content_type.clone(),
                text: node.text.clone(),
                attachments,
                alternate_branches: branches,
            });
        }
        let diagnostics = load_diagnostics(&connection, conversation_key)?;
        Ok(ConversationDetail {
            id: conversation_key.to_string(),
            title: header.0,
            created_at: header.1,
            updated_at: header.2,
            archived: optional_bool(header.3),
            starred: optional_bool(header.4),
            selected_leaf: leaf,
            messages,
            diagnostics,
        })
    }

    pub fn attachment_record(&self, attachment_key: &str) -> AppResult<AttachmentRecord> {
        if !is_opaque_id(attachment_key) {
            return Err(ErrorCode::InvalidRequest.into());
        }
        let connection = self.open_connection_read_only()?;
        connection
            .query_row(
                "SELECT attachment_key, display_name, source_name, detected_mime,
                        byte_size, preview_kind, status
                 FROM attachments WHERE attachment_key = ?1",
                [attachment_key],
                |row| {
                    let source_name: Option<String> = row.get(2)?;
                    let byte_size = row
                        .get::<_, Option<i64>>(4)?
                        .and_then(|value| u64::try_from(value).ok());
                    let status: String = row.get(6)?;
                    if source_name.is_none() || byte_size.is_none() || status != "available" {
                        return Err(rusqlite::Error::QueryReturnedNoRows);
                    }
                    Ok(AttachmentRecord {
                        id: row.get(0)?,
                        display_name: row.get(1)?,
                        source_name: source_name.unwrap_or_default(),
                        detected_mime: row.get(3)?,
                        byte_size: byte_size.unwrap_or_default(),
                        preview_kind: preview_kind_from_db(&row.get::<_, String>(5)?),
                    })
                },
            )
            .optional()?
            .ok_or_else(|| ErrorCode::AttachmentNotFound.into())
    }

    pub fn discard(&self) -> AppResult<()> {
        self.validate_directory()?;
        if !index_marker_is_valid(&self.directory)?
            || self.database_path.parent() != Some(self.directory.as_path())
            || self.directory.file_name().is_none()
        {
            return Err(ErrorCode::PathRejected.into());
        }
        for path in [
            self.database_path.clone(),
            database_sidecar(&self.database_path, "-wal"),
            database_sidecar(&self.database_path, "-shm"),
            database_sidecar(&self.database_path, "-journal"),
        ] {
            remove_private_file(&path)?;
        }
        self.initialize()?;
        Ok(())
    }

    fn open_connection(&self) -> AppResult<Connection> {
        self.validate_directory()?;
        let connection = Connection::open_with_flags(
            &self.database_path,
            OpenFlags::SQLITE_OPEN_READ_WRITE
                | OpenFlags::SQLITE_OPEN_CREATE
                | OpenFlags::SQLITE_OPEN_NO_MUTEX
                | OpenFlags::SQLITE_OPEN_NOFOLLOW,
        )?;
        connection.busy_timeout(Duration::from_secs(5))?;
        connection.execute_batch(
            "PRAGMA foreign_keys = ON;
             PRAGMA journal_mode = WAL;
             PRAGMA synchronous = NORMAL;
             PRAGMA temp_store = MEMORY;
             PRAGMA trusted_schema = OFF;",
        )?;
        set_private_file_permissions(&self.database_path)?;
        Ok(connection)
    }

    fn open_connection_read_only(&self) -> AppResult<Connection> {
        let connection = self.open_connection()?;
        connection.execute_batch("PRAGMA query_only = ON;")?;
        Ok(connection)
    }

    fn ensure_capacity_before_index(&self, source_bytes: u64) -> AppResult<()> {
        self.validate_directory()?;
        let current_bytes = derivative_size(&self.database_path)?;
        let available_bytes =
            fs2::available_space(&self.directory).map_err(|_| ErrorCode::IndexUnavailable)?;
        if !capacity_budget_allows(available_bytes, source_bytes, current_bytes) {
            return Err(ErrorCode::ResourceLimit.into());
        }
        Ok(())
    }

    fn ensure_capacity_during_index(&self) -> AppResult<()> {
        self.validate_directory()?;
        let current_bytes = derivative_size(&self.database_path)?;
        let available_bytes =
            fs2::available_space(&self.directory).map_err(|_| ErrorCode::IndexUnavailable)?;
        if current_bytes > MAX_INDEX_BYTES || available_bytes < MIN_FREE_SPACE_BYTES {
            return Err(ErrorCode::ResourceLimit.into());
        }
        Ok(())
    }

    fn validate_directory(&self) -> AppResult<()> {
        let metadata =
            fs::symlink_metadata(&self.directory).map_err(|_| ErrorCode::IndexUnavailable)?;
        if metadata.file_type().is_symlink()
            || cache_metadata_is_reparse(&metadata)
            || !metadata.is_dir()
            || fs::canonicalize(&self.directory).map_err(|_| ErrorCode::IndexUnavailable)?
                != self.directory
            || !index_marker_is_valid(&self.directory)?
        {
            return Err(ErrorCode::IndexUnavailable.into());
        }
        Ok(())
    }
}

fn insert_conversation(
    transaction: &Transaction<'_>,
    root: &SafeExportRoot,
    shard: &SafeFileEntry,
    ordinal: u64,
    conversation: &ProjectedConversation,
) -> AppResult<()> {
    let message_count = conversation
        .nodes
        .iter()
        .filter(|node| {
            is_renderable_message(
                node.message_id.as_deref(),
                &node.text,
                !node.attachments.is_empty(),
                node.child_node_ids.len() > 1,
            )
        })
        .count();
    let has_attachments = conversation
        .nodes
        .iter()
        .any(|node| !node.attachments.is_empty());
    let ordinal = sql_i64_from_u64(ordinal)?;
    let message_count = i64::try_from(message_count).map_err(|_| ErrorCode::ResourceLimit)?;
    transaction.execute(
        "INSERT INTO conversations
         (conversation_key, shard_name, shard_ordinal, title, created_at, updated_at,
          archived, starred, current_node, message_count, has_attachments)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            conversation.key,
            shard.name,
            ordinal,
            conversation.title,
            conversation.created_at,
            conversation.updated_at,
            conversation.archived.map(i32::from),
            conversation.starred.map(i32::from),
            conversation.current_node,
            message_count,
            i32::from(has_attachments)
        ],
    )?;

    let mut search_body = String::new();
    for node in &conversation.nodes {
        transaction.execute(
            "INSERT INTO nodes
             (conversation_key, node_id, parent_node_id, message_id, role, created_at,
              content_type, message_text, has_attachments)
             VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9)",
            params![
                conversation.key,
                node.node_id,
                node.parent_node_id,
                node.message_id,
                node.role,
                node.created_at,
                node.content_type,
                node.text,
                i32::from(!node.attachments.is_empty())
            ],
        )?;
        if !node.text.trim().is_empty() {
            if !search_body.is_empty() {
                search_body.push('\n');
            }
            search_body.push_str(&node.text);
        }
        for (attachment_ordinal, attachment) in node.attachments.iter().enumerate() {
            let resolved = resolve_attachment(
                root,
                &conversation.key,
                &node.node_id,
                attachment_ordinal,
                attachment,
            )?;
            insert_attachment(
                transaction,
                &conversation.key,
                &node.node_id,
                attachment_ordinal,
                &resolved,
            )?;
        }
    }
    transaction.execute(
        "INSERT INTO conversation_fts (conversation_key, title, body)
         VALUES (?1, ?2, ?3)",
        params![conversation.key, conversation.title, search_body],
    )?;
    for diagnostic in &conversation.diagnostics {
        transaction.execute(
            "INSERT INTO diagnostics (conversation_key, code, occurrence_count)
             VALUES (?1, ?2, ?3)",
            params![conversation.key, diagnostic.code, diagnostic.count],
        )?;
    }
    Ok(())
}

fn insert_attachment(
    transaction: &Transaction<'_>,
    conversation_key: &str,
    node_id: &str,
    ordinal: usize,
    attachment: &ResolvedAttachment,
) -> AppResult<()> {
    let ordinal = i64::try_from(ordinal).map_err(|_| ErrorCode::ResourceLimit)?;
    let byte_size = attachment.byte_size.map(sql_i64_from_u64).transpose()?;
    transaction.execute(
        "INSERT INTO attachments
         (attachment_key, conversation_key, node_id, ordinal, display_name, source_name,
          claimed_mime, detected_mime, byte_size, status, preview_kind)
         VALUES (?1, ?2, ?3, ?4, ?5, ?6, ?7, ?8, ?9, ?10, ?11)",
        params![
            attachment.key,
            conversation_key,
            node_id,
            ordinal,
            attachment.display_name,
            attachment.source_name,
            attachment.claimed_mime,
            attachment.detected_mime,
            byte_size,
            attachment_status_to_db(attachment.status),
            preview_kind_to_db(attachment.preview_kind)
        ],
    )?;
    Ok(())
}

fn is_renderable_message(
    message_id: Option<&str>,
    text: &str,
    has_attachments: bool,
    has_alternate_branches: bool,
) -> bool {
    !text.trim().is_empty()
        || has_attachments
        || (message_id.is_some() && has_alternate_branches)
}

struct PathNode {
    node_id: String,
    parent_node_id: Option<String>,
    message_id: Option<String>,
    role: String,
    created_at: Option<f64>,
    content_type: String,
    text: String,
}

fn load_path(
    connection: &Connection,
    conversation_key: &str,
    leaf: Option<&str>,
) -> AppResult<Vec<PathNode>> {
    let Some(mut cursor) = leaf.map(ToOwned::to_owned) else {
        return Ok(Vec::new());
    };
    let mut reversed = Vec::new();
    let mut seen = HashSet::new();
    while reversed.len() < MAX_DETAIL_PATH {
        if !seen.insert(cursor.clone()) {
            return Err(ErrorCode::ResourceLimit.into());
        }
        let node = connection
            .query_row(
                "SELECT node_id, parent_node_id, message_id, role, created_at,
                        content_type, message_text
                 FROM nodes
                 WHERE conversation_key = ?1 AND node_id = ?2",
                params![conversation_key, cursor],
                |row| {
                    Ok(PathNode {
                        node_id: row.get(0)?,
                        parent_node_id: row.get(1)?,
                        message_id: row.get(2)?,
                        role: row.get(3)?,
                        created_at: row.get(4)?,
                        content_type: row.get(5)?,
                        text: row.get(6)?,
                    })
                },
            )
            .optional()?
            .ok_or(ErrorCode::ConversationNotFound)?;
        let parent = node.parent_node_id.clone();
        reversed.push(node);
        let Some(parent) = parent else {
            break;
        };
        cursor = parent;
    }
    if reversed.len() == MAX_DETAIL_PATH
        && reversed
            .last()
            .is_some_and(|node| node.parent_node_id.is_some())
    {
        return Err(ErrorCode::ResourceLimit.into());
    }
    reversed.reverse();
    Ok(reversed)
}

fn load_branches(
    connection: &Connection,
    conversation_key: &str,
    parent_node_id: &str,
    active_child: Option<&str>,
) -> AppResult<Vec<BranchView>> {
    let mut statement = connection.prepare(
        "SELECT node_id, role, substr(message_text, 1, 180)
         FROM nodes
         WHERE conversation_key = ?1 AND parent_node_id = ?2
         ORDER BY COALESCE(created_at, 0), node_id
         LIMIT 100",
    )?;
    let immediate_branches = statement
        .query_map(params![conversation_key, parent_node_id], |row| {
            Ok((
                row.get::<_, String>(0)?,
                row.get::<_, String>(1)?,
                row.get::<_, String>(2)?,
            ))
        })?
        .filter_map(|result| match result {
            Ok(branch) if Some(branch.0.as_str()) != active_child => Some(Ok(branch)),
            Ok(_) => None,
            Err(error) => Some(Err(error)),
        })
        .collect::<Result<Vec<_>, _>>()?;
    immediate_branches
        .into_iter()
        .map(|(child_node_id, role, preview)| {
            Ok(BranchView {
                leaf_node_id: terminal_branch_leaf(
                    connection,
                    conversation_key,
                    &child_node_id,
                )?,
                role,
                preview,
            })
        })
        .collect()
}

fn terminal_branch_leaf(
    connection: &Connection,
    conversation_key: &str,
    branch_root: &str,
) -> AppResult<String> {
    connection
        .query_row(
            "WITH RECURSIVE branch(node_id) AS (
                 SELECT ?2
                 UNION
                 SELECT child.node_id
                 FROM nodes child
                 JOIN branch parent ON child.parent_node_id = parent.node_id
                 WHERE child.conversation_key = ?1
             )
             SELECT branch.node_id
             FROM branch
             JOIN nodes candidate
               ON candidate.conversation_key = ?1
              AND candidate.node_id = branch.node_id
             WHERE NOT EXISTS (
                 SELECT 1
                 FROM nodes child
                 WHERE child.conversation_key = ?1
                   AND child.parent_node_id = branch.node_id
             )
             ORDER BY COALESCE(candidate.created_at, 0) DESC, branch.node_id DESC
             LIMIT 1",
            params![conversation_key, branch_root],
            |row| row.get(0),
        )
        .optional()?
        .ok_or_else(|| ErrorCode::ResourceLimit.into())
}

fn load_message_attachments(
    connection: &Connection,
    conversation_key: &str,
    node_id: &str,
) -> AppResult<Vec<AttachmentView>> {
    let mut statement = connection.prepare(
        "SELECT attachment_key, display_name, claimed_mime, detected_mime, byte_size,
                status, preview_kind
         FROM attachments
         WHERE conversation_key = ?1 AND node_id = ?2
         ORDER BY ordinal
         LIMIT 2000",
    )?;
    Ok(statement
        .query_map(params![conversation_key, node_id], |row| {
            Ok(AttachmentView {
                id: row.get(0)?,
                display_name: row.get(1)?,
                claimed_mime: row.get(2)?,
                detected_mime: row.get(3)?,
                byte_size: row
                    .get::<_, Option<i64>>(4)?
                    .and_then(|value| u64::try_from(value).ok()),
                status: attachment_status_from_db(&row.get::<_, String>(5)?),
                preview_kind: preview_kind_from_db(&row.get::<_, String>(6)?),
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn load_diagnostics(
    connection: &Connection,
    conversation_key: &str,
) -> AppResult<Vec<DiagnosticView>> {
    let mut statement = connection.prepare(
        "SELECT code, occurrence_count FROM diagnostics
         WHERE conversation_key = ?1 ORDER BY code",
    )?;
    Ok(statement
        .query_map([conversation_key], |row| {
            Ok(DiagnosticView {
                code: row.get(0)?,
                count: row.get(1)?,
            })
        })?
        .collect::<Result<Vec<_>, _>>()?)
}

fn build_fts_query(input: &str) -> AppResult<String> {
    if input.len() > MAX_SEARCH_BYTES {
        return Err(ErrorCode::InvalidRequest.into());
    }
    let terms = input
        .split_whitespace()
        .filter(|term| !term.is_empty())
        .take(MAX_SEARCH_TERMS)
        .map(|term| format!("\"{}\"*", term.replace('"', "\"\"")))
        .collect::<Vec<_>>();
    if terms.is_empty() {
        return Err(ErrorCode::InvalidRequest.into());
    }
    Ok(terms.join(" AND "))
}

fn is_supported_role(role: &str) -> bool {
    matches!(role, "user" | "assistant" | "system" | "tool" | "other")
}

fn is_opaque_id(value: &str) -> bool {
    value.len() == 32 && value.bytes().all(|byte| byte.is_ascii_hexdigit())
}

fn optional_bool(value: Option<i64>) -> Option<bool> {
    value.map(|number| number != 0)
}

fn sql_i64_from_u64(value: u64) -> AppResult<i64> {
    i64::try_from(value).map_err(|_| ErrorCode::ResourceLimit.into())
}

fn sql_u64_from_i64(value: i64) -> AppResult<u64> {
    u64::try_from(value).map_err(|_| ErrorCode::IndexUnavailable.into())
}

fn attachment_status_to_db(status: AttachmentStatus) -> &'static str {
    match status {
        AttachmentStatus::Available => "available",
        AttachmentStatus::Missing => "missing",
        AttachmentStatus::Rejected => "rejected",
    }
}

fn attachment_status_from_db(value: &str) -> AttachmentStatus {
    match value {
        "available" => AttachmentStatus::Available,
        "rejected" => AttachmentStatus::Rejected,
        _ => AttachmentStatus::Missing,
    }
}

fn preview_kind_to_db(kind: PreviewKind) -> &'static str {
    match kind {
        PreviewKind::Image => "image",
        PreviewKind::Audio => "audio",
        PreviewKind::Video => "video",
        PreviewKind::Pdf => "pdf",
        PreviewKind::Text => "text",
        PreviewKind::Unsupported => "unsupported",
        PreviewKind::Missing => "missing",
    }
}

fn preview_kind_from_db(value: &str) -> PreviewKind {
    match value {
        "image" => PreviewKind::Image,
        "audio" => PreviewKind::Audio,
        "video" => PreviewKind::Video,
        "pdf" => PreviewKind::Pdf,
        "text" => PreviewKind::Text,
        "missing" => PreviewKind::Missing,
        _ => PreviewKind::Unsupported,
    }
}

fn unix_time_seconds() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0.0, |duration| duration.as_secs_f64())
}

fn capacity_budget_allows(available_bytes: u64, source_bytes: u64, current_bytes: u64) -> bool {
    let estimated_additional = source_bytes.saturating_mul(SOURCE_TO_INDEX_ESTIMATE);
    current_bytes
        .checked_add(estimated_additional)
        .is_some_and(|estimated_total| estimated_total <= MAX_INDEX_BYTES)
        && available_bytes >= estimated_additional.saturating_add(MIN_FREE_SPACE_BYTES)
}

fn database_sidecar(database_path: &Path, suffix: &str) -> PathBuf {
    let mut value = database_path.as_os_str().to_os_string();
    value.push(suffix);
    PathBuf::from(value)
}

fn derivative_size(database_path: &Path) -> AppResult<u64> {
    [
        database_path.to_path_buf(),
        database_sidecar(database_path, "-wal"),
        database_sidecar(database_path, "-shm"),
        database_sidecar(database_path, "-journal"),
    ]
    .into_iter()
    .try_fold(0_u64, |total, path| {
        let metadata = match fs::symlink_metadata(&path) {
            Ok(metadata) => metadata,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                return Ok(total);
            }
            Err(_) => return Err(ErrorCode::IndexUnavailable.into()),
        };
        if metadata.file_type().is_symlink()
            || cache_metadata_is_reparse(&metadata)
            || private_path_has_multiple_links(&path, &metadata)?
            || !metadata.is_file()
        {
            return Err(ErrorCode::IndexUnavailable.into());
        }
        total
            .checked_add(metadata.len())
            .ok_or_else(|| ErrorCode::ResourceLimit.into())
    })
}

fn remove_private_file(path: &Path) -> AppResult<()> {
    let metadata = match fs::symlink_metadata(path) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(_) => return Err(ErrorCode::IndexUnavailable.into()),
    };
    if metadata.file_type().is_symlink()
        || cache_metadata_is_reparse(&metadata)
        || private_path_has_multiple_links(path, &metadata)?
        || !metadata.is_file()
    {
        return Err(ErrorCode::PathRejected.into());
    }
    fs::remove_file(path).map_err(|_| ErrorCode::IndexUnavailable.into())
}

fn set_private_directory_permissions(path: &Path) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        fs::set_permissions(path, fs::Permissions::from_mode(0o700))
            .map_err(|_| ErrorCode::IndexUnavailable)?;
    }
    #[cfg(windows)]
    set_private_windows_acl(path, true)?;
    Ok(())
}

fn ensure_private_cache_root(path: &Path) -> AppResult<PathBuf> {
    if !path.is_absolute() {
        return Err(ErrorCode::IndexUnavailable.into());
    }

    let mut cursor = path;
    let mut missing_components = Vec::new();
    let canonical_existing_parent = loop {
        match fs::symlink_metadata(cursor) {
            Ok(metadata) => {
                if metadata.file_type().is_symlink()
                    || cache_metadata_is_reparse(&metadata)
                    || !metadata.is_dir()
                {
                    return Err(ErrorCode::IndexUnavailable.into());
                }
                break fs::canonicalize(cursor).map_err(|_| ErrorCode::IndexUnavailable)?;
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                let component = cursor
                    .file_name()
                    .ok_or(ErrorCode::IndexUnavailable)?
                    .to_os_string();
                missing_components.push(component);
                cursor = cursor.parent().ok_or(ErrorCode::IndexUnavailable)?;
            }
            Err(_) => return Err(ErrorCode::IndexUnavailable.into()),
        }
    };

    let mut canonical = canonical_existing_parent;
    for component in missing_components.iter().rev() {
        let next = canonical.join(component);
        ensure_private_directory(&next, &canonical)?;
        canonical = fs::canonicalize(&next).map_err(|_| ErrorCode::IndexUnavailable)?;
    }
    set_private_directory_permissions(&canonical)?;
    Ok(canonical)
}

fn ensure_private_directory(path: &Path, canonical_parent: &Path) -> AppResult<()> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            if metadata.file_type().is_symlink()
                || cache_metadata_is_reparse(&metadata)
                || !metadata.is_dir()
            {
                return Err(ErrorCode::IndexUnavailable.into());
            }
        }
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            fs::create_dir(path).map_err(|_| ErrorCode::IndexUnavailable)?;
        }
        Err(_) => return Err(ErrorCode::IndexUnavailable.into()),
    }
    let canonical = fs::canonicalize(path).map_err(|_| ErrorCode::IndexUnavailable)?;
    if canonical.parent() != Some(canonical_parent) {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    set_private_directory_permissions(&canonical)
}

fn initialize_index_marker(directory: &Path) -> AppResult<()> {
    if index_marker_is_valid(directory)? {
        return Ok(());
    }
    let root = cap_std::fs::Dir::open_ambient_dir(directory, cap_std::ambient_authority())
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.write(true).create_new(true);
    let mut marker = root
        .open_with(".index-owner", &options)
        .map(cap_std::fs::File::into_std)
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    marker
        .write_all(INDEX_MARKER.as_bytes())
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    marker.sync_all().map_err(|_| ErrorCode::IndexUnavailable)?;
    set_private_file_permissions_from_handle(&marker)?;
    set_private_file_permissions(&directory.join(".index-owner"))?;
    if !index_marker_is_valid(directory)? {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    Ok(())
}

fn index_marker_is_valid(directory: &Path) -> AppResult<bool> {
    let root = cap_std::fs::Dir::open_ambient_dir(directory, cap_std::ambient_authority())
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    let metadata = match root.symlink_metadata(".index-owner") {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(false),
        Err(_) => return Err(ErrorCode::IndexUnavailable.into()),
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    let marker = root
        .open(".index-owner")
        .map(cap_std::fs::File::into_std)
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    let opened_metadata = marker.metadata().map_err(|_| ErrorCode::IndexUnavailable)?;
    if opened_metadata.len() > 128
        || private_opened_file_has_multiple_links(&marker)?
        || cache_metadata_is_reparse(&opened_metadata)
    {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    let mut content = String::new();
    marker
        .take(128)
        .read_to_string(&mut content)
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    Ok(content == INDEX_MARKER)
}

#[cfg(unix)]
fn private_path_has_multiple_links(_path: &Path, metadata: &fs::Metadata) -> AppResult<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(metadata.nlink() > 1)
}

#[cfg(unix)]
fn private_opened_file_has_multiple_links(file: &File) -> AppResult<bool> {
    use std::os::unix::fs::MetadataExt;
    Ok(file
        .metadata()
        .map_err(|_| ErrorCode::IndexUnavailable)?
        .nlink()
        > 1)
}

#[cfg(windows)]
fn private_path_has_multiple_links(path: &Path, _metadata: &fs::Metadata) -> AppResult<bool> {
    use std::os::windows::fs::OpenOptionsExt;
    use windows_sys::Win32::Storage::FileSystem::FILE_FLAG_OPEN_REPARSE_POINT;

    let file = OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|_| ErrorCode::IndexUnavailable)?;
    private_opened_file_has_multiple_links(&file)
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn private_opened_file_has_multiple_links(file: &File) -> AppResult<bool> {
    use std::{mem::MaybeUninit, os::windows::io::AsRawHandle};
    use windows_sys::Win32::Storage::FileSystem::{
        BY_HANDLE_FILE_INFORMATION, GetFileInformationByHandle,
    };

    let mut information = MaybeUninit::<BY_HANDLE_FILE_INFORMATION>::uninit();
    // SAFETY: `file` owns a valid handle and the API initializes the output
    // structure only when it reports success.
    let succeeded = unsafe {
        GetFileInformationByHandle(file.as_raw_handle() as _, information.as_mut_ptr())
    };
    if succeeded == 0 {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    // SAFETY: the Windows API reported successful initialization above.
    Ok(unsafe { information.assume_init() }.nNumberOfLinks > 1)
}

#[cfg(not(any(unix, windows)))]
fn private_path_has_multiple_links(_path: &Path, _metadata: &fs::Metadata) -> AppResult<bool> {
    Ok(false)
}

#[cfg(not(any(unix, windows)))]
fn private_opened_file_has_multiple_links(_file: &File) -> AppResult<bool> {
    Ok(false)
}

#[cfg(windows)]
fn cache_metadata_is_reparse(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;
    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x0000_0400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn cache_metadata_is_reparse(_metadata: &fs::Metadata) -> bool {
    false
}

fn set_private_file_permissions(path: &Path) -> AppResult<()> {
    if path.exists() {
        #[cfg(unix)]
        {
            use std::os::unix::fs::PermissionsExt;
            fs::set_permissions(path, fs::Permissions::from_mode(0o600))
                .map_err(|_| ErrorCode::IndexUnavailable)?;
        }
        #[cfg(windows)]
        set_private_windows_acl(path, false)?;
    }
    Ok(())
}

fn set_private_file_permissions_from_handle(file: &File) -> AppResult<()> {
    #[cfg(unix)]
    {
        use std::os::unix::fs::PermissionsExt;
        file.set_permissions(fs::Permissions::from_mode(0o600))
            .map_err(|_| ErrorCode::IndexUnavailable)?;
    }
    Ok(())
}

#[cfg(windows)]
#[allow(unsafe_code)]
fn set_private_windows_acl(path: &Path, directory: bool) -> AppResult<()> {
    use std::{os::windows::ffi::OsStrExt, ptr::null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::{
            Authorization::ConvertStringSecurityDescriptorToSecurityDescriptorW,
            DACL_SECURITY_INFORMATION, PROTECTED_DACL_SECURITY_INFORMATION,
            PSECURITY_DESCRIPTOR, SetFileSecurityW,
        },
    };

    let sddl = if directory {
        "D:P(A;OICI;FA;;;OW)(A;OICI;FA;;;SY)"
    } else {
        "D:P(A;;FA;;;OW)(A;;FA;;;SY)"
    };
    let sddl_wide = sddl
        .encode_utf16()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let path_wide = path
        .as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect::<Vec<_>>();
    let mut descriptor: PSECURITY_DESCRIPTOR = null_mut();
    // SAFETY: both UTF-16 inputs are NUL-terminated, the out-pointer is valid,
    // and the returned descriptor is released with LocalFree below.
    let converted = unsafe {
        ConvertStringSecurityDescriptorToSecurityDescriptorW(
            sddl_wide.as_ptr(),
            1,
            &mut descriptor,
            null_mut(),
        )
    };
    if converted == 0 || descriptor.is_null() {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    // SAFETY: `descriptor` came from the conversion API and `path_wide`
    // remains valid for the duration of this call.
    let applied = unsafe {
        SetFileSecurityW(
            path_wide.as_ptr(),
            DACL_SECURITY_INFORMATION | PROTECTED_DACL_SECURITY_INFORMATION,
            descriptor,
        )
    };
    // SAFETY: this pointer is owned by the local allocator after successful
    // conversion and is released exactly once.
    let _ = unsafe { LocalFree(descriptor.cast()) };
    if applied == 0 {
        return Err(ErrorCode::IndexUnavailable.into());
    }
    Ok(())
}

fn tighten_database_files(database_path: &Path) -> AppResult<()> {
    set_private_file_permissions(database_path)?;
    set_private_file_permissions(&database_sidecar(database_path, "-wal"))?;
    set_private_file_permissions(&database_sidecar(database_path, "-shm"))?;
    set_private_file_permissions(&database_sidecar(database_path, "-journal"))?;
    Ok(())
}

struct HashingReader<'a, R, F>
where
    R: Read,
    F: FnMut(u64),
{
    inner: R,
    hasher: blake3::Hasher,
    on_bytes: &'a mut F,
}

impl<'a, R, F> HashingReader<'a, R, F>
where
    R: Read,
    F: FnMut(u64),
{
    fn new(inner: R, on_bytes: &'a mut F) -> Self {
        Self {
            inner,
            hasher: blake3::Hasher::new(),
            on_bytes,
        }
    }

    fn hash_hex(&self) -> String {
        self.hasher.finalize().to_hex().to_string()
    }
}

impl<R, F> Read for HashingReader<'_, R, F>
where
    R: Read,
    F: FnMut(u64),
{
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        let count = self.inner.read(buffer)?;
        if count > 0 {
            self.hasher.update(&buffer[..count]);
            (self.on_bytes)(count as u64);
        }
        Ok(count)
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn synthetic_export() -> (TempDir, SafeExportRoot) {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        let shard = r#"[
          {
            "id": "synthetic-conversation",
            "title": "Synthetic searchable title",
            "create_time": 1000,
            "update_time": 1100,
            "is_archived": false,
            "is_starred": true,
            "current_node": "node-b",
            "mapping": {
              "node-a": {
                "parent": null,
                "children": ["node-b", "node-c"],
                "message": {
                  "id": "message-a",
                  "author": {"role": "user"},
                  "content": {"content_type": "text", "parts": ["Synthetic prompt token"]}
                }
              },
              "node-b": {
                "parent": "node-a",
                "children": [],
                "message": {
                  "id": "message-b",
                  "author": {"role": "assistant"},
                  "content": {"content_type": "text", "parts": ["Synthetic primary answer"]}
                }
              },
              "node-c": {
                "parent": "node-a",
                "children": ["node-d"],
                "message": {
                  "id": "message-c",
                  "author": {"role": "assistant"},
                  "content": {"content_type": "text", "parts": ["Synthetic alternate answer"]}
                }
              },
              "node-d": {
                "parent": "node-c",
                "children": [],
                "message": {
                  "id": "message-d",
                  "author": {"role": "assistant"},
                  "content": {"content_type": "text", "parts": ["Synthetic alternate continuation"]}
                }
              }
            }
          }
        ]"#;
        fs::write(export.join("conversations-000.json"), shard).expect("write shard");
        let root = SafeExportRoot::select(&export).expect("select root");
        (directory, root)
    }

    fn synthetic_export_from_value(value: &Value) -> (TempDir, SafeExportRoot) {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        fs::write(
            export.join("conversations-000.json"),
            serde_json::to_vec(value).expect("serialize synthetic shard"),
        )
        .expect("write shard");
        let root = SafeExportRoot::select(&export).expect("select root");
        (directory, root)
    }

    #[test]
    fn indexes_searches_and_loads_a_branch() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store.discard().expect("clean store");
        let shard = &root.shards()[0];
        let stats = store
            .index_shard(&root, shard, &AtomicBool::new(false), |_| {})
            .expect("index");
        assert_eq!(stats.conversations_indexed, 1);

        let page = store
            .query_conversations(&ConversationQuery {
                page: Some(0),
                page_size: Some(20),
                search: Some("primary".to_string()),
                date_from: None,
                date_to: None,
                role: Some("assistant".to_string()),
                archived: Some(false),
                starred: Some(true),
                has_attachments: Some(false),
                attachment_kind: None,
            })
            .expect("query");
        assert_eq!(page.total, 1);

        let detail = store
            .conversation_detail(&page.items[0].id, None)
            .expect("detail");
        assert_eq!(detail.messages.len(), 2);
        assert_eq!(detail.messages[0].alternate_branches.len(), 1);
        let branch_leaf = detail.messages[0].alternate_branches[0]
            .leaf_node_id
            .clone();
        let branch = store
            .conversation_detail(&page.items[0].id, Some(&branch_leaf))
            .expect("branch");
        assert_eq!(branch.messages.len(), 3);
        assert!(branch.messages[1].text.contains("alternate"));
        assert!(branch.messages[2].text.contains("continuation"));
    }

    #[test]
    fn transcript_parts_do_not_set_attachment_flags_or_detail_cards() {
        let (directory, root) = synthetic_export_from_value(&json!([{
            "title": "Synthetic transcript",
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "id": "message-a",
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
        }]));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let page = store
            .query_conversations(&ConversationQuery::default())
            .expect("query");
        assert!(!page.items[0].has_attachments);
        let detail = store
            .conversation_detail(&page.items[0].id, None)
            .expect("detail");
        assert_eq!(detail.messages[0].text, "Synthetic spoken transcript");
        assert!(detail.messages[0].attachments.is_empty());
    }

    #[test]
    fn genuine_missing_file_references_remain_accessible() {
        let (directory, root) = synthetic_export_from_value(&json!([{
            "title": "Synthetic missing attachment",
            "current_node": "node-a",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "id": "message-a",
                        "author": {"role": "user"},
                        "content": {
                            "parts": [{
                                "asset_pointer": "file-service://synthetic-missing",
                                "content_type": "application/pdf"
                            }]
                        }
                    }
                }
            }
        }]));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let page = store
            .query_conversations(&ConversationQuery::default())
            .expect("query");
        assert!(page.items[0].has_attachments);
        let detail = store
            .conversation_detail(&page.items[0].id, None)
            .expect("detail");
        assert_eq!(detail.messages[0].attachments.len(), 1);
        assert!(matches!(
            detail.messages[0].attachments[0].status,
            AttachmentStatus::Missing
        ));
    }

    #[test]
    fn attachment_kind_filter_uses_detected_file_categories() {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        let shard = json!([
            {
                "title": "Synthetic audio conversation",
                "current_node": "audio-node",
                "mapping": {
                    "audio-node": {
                        "parent": null,
                        "children": [],
                        "message": {
                            "id": "audio-message",
                            "author": {"role": "user"},
                            "content": {
                                "parts": [{
                                    "asset_pointer": "file-service://file-synthetic-audio",
                                    "name": "Attachment",
                                    "mime_type": "application/octet-stream"
                                }]
                            }
                        }
                    }
                }
            },
            {
                "title": "Synthetic missing-file conversation",
                "current_node": "missing-node",
                "mapping": {
                    "missing-node": {
                        "parent": null,
                        "children": [],
                        "message": {
                            "id": "missing-message",
                            "author": {"role": "user"},
                            "content": {
                                "parts": [{
                                    "asset_pointer": "file-service://file-synthetic-missing",
                                    "name": "Missing document",
                                    "mime_type": "application/pdf"
                                }]
                            }
                        }
                    }
                }
            }
        ]);
        fs::write(
            export.join("conversations-000.json"),
            serde_json::to_vec(&shard).expect("serialize shard"),
        )
        .expect("write shard");
        let mut wav = vec![0_u8; 44 + 64];
        wav[0..4].copy_from_slice(b"RIFF");
        wav[4..8].copy_from_slice(&(100_u32).to_le_bytes());
        wav[8..12].copy_from_slice(b"WAVE");
        wav[12..16].copy_from_slice(b"fmt ");
        wav[16..20].copy_from_slice(&(16_u32).to_le_bytes());
        wav[20..22].copy_from_slice(&(1_u16).to_le_bytes());
        wav[22..24].copy_from_slice(&(1_u16).to_le_bytes());
        wav[24..28].copy_from_slice(&(8_000_u32).to_le_bytes());
        wav[28..32].copy_from_slice(&(8_000_u32).to_le_bytes());
        wav[32..34].copy_from_slice(&(1_u16).to_le_bytes());
        wav[34..36].copy_from_slice(&(8_u16).to_le_bytes());
        wav[36..40].copy_from_slice(b"data");
        wav[40..44].copy_from_slice(&(64_u32).to_le_bytes());
        fs::write(export.join("file-synthetic-audio.dat"), wav).expect("write audio");

        let root = SafeExportRoot::select(&export).expect("select root");
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");
        store
            .open_connection()
            .expect("open store")
            .execute(
                "UPDATE attachments
                 SET preview_kind = 'unsupported'
                 WHERE detected_mime LIKE 'audio/%'",
                [],
            )
            .expect("simulate a detected audio file that is save-only");

        let audio = store
            .query_conversations(&ConversationQuery {
                attachment_kind: Some(AttachmentKindFilter::Audio),
                ..ConversationQuery::default()
            })
            .expect("audio query");
        assert_eq!(audio.total, 1);
        assert_eq!(audio.items[0].title, "Synthetic audio conversation");

        let other = store
            .query_conversations(&ConversationQuery {
                attachment_kind: Some(AttachmentKindFilter::Other),
                ..ConversationQuery::default()
            })
            .expect("other query");
        assert_eq!(other.total, 0);

        let missing = store
            .query_conversations(&ConversationQuery {
                attachment_kind: Some(AttachmentKindFilter::Missing),
                ..ConversationQuery::default()
            })
            .expect("missing query");
        assert_eq!(missing.total, 1);
        assert_eq!(
            missing.items[0].title,
            "Synthetic missing-file conversation"
        );
    }

    #[test]
    fn normalized_markers_do_not_reach_details_search_snippets_or_branch_previews() {
        let (directory, root) = synthetic_export_from_value(&json!([{
            "title": "Synthetic marker test",
            "current_node": "node-b",
            "mapping": {
                "node-a": {
                    "parent": null,
                    "children": ["node-b", "node-c"],
                    "message": {
                        "id": "message-a",
                        "author": {"role": "user"},
                        "content": {
                            "parts": ["Synthetic prefix \u{e200}cite\u{e202}turn0search0\u{e201} suffix"]
                        }
                    }
                },
                "node-b": {
                    "parent": "node-a",
                    "children": [],
                    "message": {
                        "id": "message-b",
                        "author": {"role": "assistant"},
                        "content": {"parts": ["Synthetic main branch"]}
                    }
                },
                "node-c": {
                    "parent": "node-a",
                    "children": [],
                    "message": {
                        "id": "message-c",
                        "author": {"role": "assistant"},
                        "content": {
                            "parts": ["Alternate \u{e200}navlist\u{e202}turn0news0\u{e201} preview"]
                        }
                    }
                }
            }
        }]));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let page = store
            .query_conversations(&ConversationQuery {
                search: Some("Synthetic".to_string()),
                ..ConversationQuery::default()
            })
            .expect("search");
        assert!(
            page.items[0]
                .match_preview
                .as_deref()
                .is_some_and(|preview| !preview.contains('\u{e200}'))
        );

        let detail = store
            .conversation_detail(&page.items[0].id, None)
            .expect("detail");
        assert!(
            detail
                .messages
                .iter()
                .all(|message| !message.text.contains('\u{e200}'))
        );
        assert!(
            detail.messages[0]
                .alternate_branches
                .iter()
                .all(|branch| !branch.preview.contains('\u{e200}'))
        );
    }

    #[test]
    fn message_counts_and_details_share_the_renderability_predicate() {
        let (directory, root) = synthetic_export_from_value(&json!([{
            "title": "Synthetic renderability test",
            "current_node": "node-visible",
            "mapping": {
                "node-null": {
                    "parent": null,
                    "children": ["node-missing"],
                    "message": null
                },
                "node-missing": {
                    "parent": "node-null",
                    "children": ["node-empty"],
                    "message": {
                        "id": "message-missing",
                        "author": {"role": "assistant"}
                    }
                },
                "node-empty": {
                    "parent": "node-missing",
                    "children": ["node-whitespace"],
                    "message": {
                        "id": "message-empty",
                        "author": {"role": "assistant"},
                        "content": {"parts": []}
                    }
                },
                "node-whitespace": {
                    "parent": "node-empty",
                    "children": ["node-marker"],
                    "message": {
                        "id": "message-whitespace",
                        "author": {"role": "assistant"},
                        "content": {"parts": [" \n\t "]}
                    }
                },
                "node-marker": {
                    "parent": "node-whitespace",
                    "children": ["node-attachment"],
                    "message": {
                        "id": "message-marker",
                        "author": {"role": "assistant"},
                        "content": {
                            "parts": ["\u{e200}cite\u{e202}turn0search0\u{e201}"]
                        }
                    }
                },
                "node-attachment": {
                    "parent": "node-marker",
                    "children": ["node-visible"],
                    "message": {
                        "id": "message-attachment",
                        "author": {"role": "user"},
                        "content": {
                            "parts": [{
                                "file_id": "synthetic-file",
                                "mime_type": "text/plain"
                            }]
                        }
                    }
                },
                "node-visible": {
                    "parent": "node-attachment",
                    "children": [],
                    "message": {
                        "id": "message-visible",
                        "author": {"role": "assistant"},
                        "content": {"parts": ["Synthetic visible response"]}
                    }
                }
            }
        }]));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let page = store
            .query_conversations(&ConversationQuery::default())
            .expect("query");
        assert_eq!(page.items[0].message_count, 2);

        let detail = store
            .conversation_detail(&page.items[0].id, None)
            .expect("detail");
        assert_eq!(detail.messages.len(), 2);
        assert!(detail.messages[0].text.is_empty());
        assert_eq!(detail.messages[0].attachments.len(), 1);
        assert_eq!(detail.messages[1].text, "Synthetic visible response");
    }

    #[test]
    fn branch_bearing_messages_are_renderable_but_null_structural_nodes_are_not() {
        assert!(is_renderable_message(
            Some("synthetic-message"),
            "",
            false,
            true
        ));
        assert!(!is_renderable_message(None, "", false, true));
        assert!(!is_renderable_message(
            Some("synthetic-message"),
            " \n\t ",
            false,
            false
        ));
    }

    #[test]
    fn date_filters_use_the_same_effective_timestamp_as_display_and_sort() {
        let (directory, root) = synthetic_export_from_value(&json!([
            {
                "title": "Synthetic recently updated",
                "create_time": 100,
                "update_time": 900,
                "current_node": "node-a",
                "mapping": {
                    "node-a": {
                        "parent": null,
                        "children": [],
                        "message": {
                            "id": "message-a",
                            "author": {"role": "user"},
                            "content": {"parts": ["Synthetic alpha"]}
                        }
                    }
                }
            },
            {
                "title": "Synthetic older update",
                "create_time": 950,
                "update_time": 200,
                "current_node": "node-b",
                "mapping": {
                    "node-b": {
                        "parent": null,
                        "children": [],
                        "message": {
                            "id": "message-b",
                            "author": {"role": "assistant"},
                            "content": {"parts": ["Synthetic beta"]}
                        }
                    }
                }
            }
        ]));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let page = store
            .query_conversations(&ConversationQuery {
                date_from: Some(500.0),
                ..ConversationQuery::default()
            })
            .expect("query");
        assert_eq!(page.total, 1);
        assert_eq!(page.items[0].title, "Synthetic recently updated");
        assert_eq!(page.items[0].updated_at, Some(900.0));
    }

    #[test]
    fn zero_based_pagination_covers_boundary_sizes_without_gaps() {
        let mut conversations = Vec::new();
        for (group, count) in [
            ("belowpage", 3_u32),
            ("equalpage", 50_u32),
            ("abovepage", 51_u32),
        ] {
            for index in 0..count {
                let node_id = format!("{group}-node-{index}");
                conversations.push(json!({
                    "title": format!("Synthetic {group} {index}"),
                    "update_time": f64::from(index),
                    "current_node": node_id,
                    "mapping": {
                        (node_id.clone()): {
                            "parent": null,
                            "children": [],
                            "message": {
                                "id": format!("{group}-message-{index}"),
                                "author": {"role": "user"},
                                "content": {"parts": [format!("Synthetic {group} body")]}
                            }
                        }
                    }
                }));
            }
        }

        let (directory, root) = synthetic_export_from_value(&Value::Array(conversations));
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store
            .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
            .expect("index");

        let query = |search: &str, page: u32| {
            store
                .query_conversations(&ConversationQuery {
                    page: Some(page),
                    page_size: Some(50),
                    search: Some(search.to_string()),
                    ..ConversationQuery::default()
                })
                .expect("query")
        };

        let below = query("belowpage", 0);
        assert_eq!((below.page, below.total, below.items.len()), (0, 3, 3));
        assert!(!below.has_more);

        let equal = query("equalpage", 0);
        assert_eq!((equal.page, equal.total, equal.items.len()), (0, 50, 50));
        assert!(!equal.has_more);

        let above_first = query("abovepage", 0);
        let above_last = query("abovepage", 1);
        assert_eq!(
            (above_first.page, above_first.total, above_first.items.len()),
            (0, 51, 50)
        );
        assert!(above_first.has_more);
        assert_eq!(
            (above_last.page, above_last.total, above_last.items.len()),
            (1, 51, 1)
        );
        assert!(!above_last.has_more);
        let first_ids = above_first
            .items
            .iter()
            .map(|item| item.id.as_str())
            .collect::<HashSet<_>>();
        assert!(
            above_last
                .items
                .iter()
                .all(|item| !first_ids.contains(item.id.as_str()))
        );

        let empty = query("no-synthetic-match", 0);
        assert_eq!((empty.page, empty.total, empty.items.len()), (0, 0, 0));
        assert!(!empty.has_more);
    }

    #[test]
    fn cancellation_preserves_the_previous_committed_shard() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store.discard().expect("clean store");
        let shard = &root.shards()[0];
        store
            .index_shard(&root, shard, &AtomicBool::new(false), |_| {})
            .expect("first index");
        let cancelled = AtomicBool::new(true);
        assert!(store.index_shard(&root, shard, &cancelled, |_| {}).is_err());
        let page = store
            .query_conversations(&ConversationQuery {
                page: None,
                page_size: None,
                search: None,
                date_from: None,
                date_to: None,
                role: None,
                archived: None,
                starred: None,
                has_attachments: None,
                attachment_kind: None,
            })
            .expect("query");
        assert_eq!(page.total, 1);
    }

    #[test]
    fn unchanged_metadata_cannot_hide_replaced_shard_content() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store.discard().expect("clean store");
        let shard = &root.shards()[0];
        store
            .index_shard(&root, shard, &AtomicBool::new(false), |_| {})
            .expect("index");
        store
            .finalize_archive_state(root.shards(), &root.attachment_inventory_fingerprint())
            .expect("finalize archive");
        assert!(store.shard_is_current(&root, shard).expect("current shard"));

        let shard_path = directory
            .path()
            .join("export")
            .join("conversations-000.json");
        let original_metadata = fs::metadata(&shard_path).expect("metadata");
        let original_modified = original_metadata.modified().expect("modified");
        let original = fs::read_to_string(&shard_path).expect("read shard");
        let replaced = original.replace("primary", "changed");
        assert_eq!(replaced.len(), original.len());
        fs::write(&shard_path, replaced).expect("replace shard");
        OpenOptions::new()
            .write(true)
            .open(&shard_path)
            .expect("open shard")
            .set_times(std::fs::FileTimes::new().set_modified(original_modified))
            .expect("restore timestamp");

        assert!(
            !store
                .shard_is_current(&root, shard)
                .expect("compare content hash")
        );
    }

    #[test]
    fn projection_version_change_invalidates_unchanged_shards() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        store.discard().expect("clean store");
        let shard = &root.shards()[0];
        store
            .index_shard(&root, shard, &AtomicBool::new(false), |_| {})
            .expect("index");
        store
            .finalize_archive_state(root.shards(), &root.attachment_inventory_fingerprint())
            .expect("finalize archive");
        assert!(store.shard_is_current(&root, shard).expect("current shard"));

        store
            .open_connection()
            .expect("connection")
            .execute(
                "UPDATE archive_state SET value = 'legacy' WHERE key = 'projection_version'",
                [],
            )
            .expect("replace projection version");

        assert!(
            !store
                .shard_is_current(&root, shard)
                .expect("projection version")
        );
    }

    #[test]
    fn fts_input_is_parameterized_and_operator_text_is_quoted() {
        let query = build_fts_query("synthetic OR \"quoted\"").expect("query");
        assert_eq!(query, "\"synthetic\"* AND \"OR\"* AND \"\"\"quoted\"\"\"*");
    }

    #[test]
    fn capacity_budget_preserves_a_free_space_reserve() {
        assert!(capacity_budget_allows(
            MIN_FREE_SPACE_BYTES + 4_096,
            1_024,
            0,
        ));
        assert!(!capacity_budget_allows(MIN_FREE_SPACE_BYTES, 1_024, 0,));
        assert!(!capacity_budget_allows(u64::MAX, MAX_INDEX_BYTES, 0,));
    }

    #[test]
    fn discard_removes_an_allowlisted_rollback_journal() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        let journal = database_sidecar(&store.database_path, "-journal");
        fs::write(&journal, b"synthetic journal").expect("write journal");
        store.discard().expect("discard index");
        assert!(!journal.exists());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_preseeded_symlink_cache_root() {
        let (directory, root) = synthetic_export();
        let outside = directory.path().join("outside-cache");
        fs::create_dir(&outside).expect("create outside cache");
        let cache = directory.path().join("cache-link");
        std::os::unix::fs::symlink(&outside, &cache).expect("create cache symlink");

        assert!(Store::for_export_with_cache_root(&root, &cache).is_err());
    }

    #[cfg(unix)]
    #[test]
    fn rejects_a_replaced_store_directory_before_opening_sqlite() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        let moved = directory.path().join("moved-index");
        fs::rename(&store.directory, &moved).expect("move index directory");
        let replacement = directory.path().join("replacement-index");
        fs::create_dir(&replacement).expect("create replacement directory");
        fs::write(replacement.join(".index-owner"), INDEX_MARKER).expect("write marker");
        std::os::unix::fs::symlink(&replacement, &store.directory)
            .expect("replace index directory with symlink");

        assert!(store.open_connection().is_err());
    }

    #[cfg(unix)]
    #[test]
    fn security_rejects_a_preseeded_database_symlink() {
        let (directory, root) = synthetic_export();
        let cache = directory.path().join("cache");
        let store = Store::for_export_with_cache_root(&root, &cache).expect("store");
        fs::remove_file(&store.database_path).expect("remove app database");

        let outside = directory.path().join("outside.sqlite3");
        let outside_connection = Connection::open(&outside).expect("create outside database");
        outside_connection
            .execute_batch("CREATE TABLE sentinel (value TEXT NOT NULL);")
            .expect("initialize outside database");
        drop(outside_connection);
        std::os::unix::fs::symlink(&outside, &store.database_path)
            .expect("preseed database symlink");

        assert!(store.open_connection().is_err());
        let outside_connection = Connection::open(&outside).expect("reopen outside database");
        assert!(
            outside_connection
                .query_row(
                    "SELECT 1 FROM sqlite_schema WHERE type = 'table' AND name = 'sentinel'",
                    [],
                    |row| row.get::<_, i64>(0),
                )
                .is_ok()
        );
    }
}
