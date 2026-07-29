use std::sync::{
    Arc, Mutex,
    atomic::{AtomicBool, Ordering},
};

use crate::{
    error::{AppResult, ErrorCode},
    models::{IndexPhase, IndexProgress},
    safe_root::SafeExportRoot,
    store::Store,
};

pub struct ArchiveSession {
    pub root: Arc<SafeExportRoot>,
    pub store: Store,
    baseline: Vec<(String, u64, u128)>,
}

impl ArchiveSession {
    pub fn new(root: SafeExportRoot) -> AppResult<Self> {
        let baseline = root.source_fingerprint();
        let store = Store::for_export(&root)?;
        Ok(Self {
            root: Arc::new(root),
            store,
            baseline,
        })
    }

    pub fn new_with_cache_root(
        root: SafeExportRoot,
        cache_root: &std::path::Path,
    ) -> AppResult<Self> {
        let baseline = root.source_fingerprint();
        let store = Store::for_export_with_cache_root(&root, cache_root)?;
        Ok(Self {
            root: Arc::new(root),
            store,
            baseline,
        })
    }

    pub fn source_remains_unchanged(&self) -> bool {
        self.root.remains_unchanged(&self.baseline)
    }
}

#[derive(Clone)]
pub struct IndexCoordinator {
    inner: Arc<IndexCoordinatorInner>,
}

struct IndexCoordinatorInner {
    running: AtomicBool,
    cancellation: Mutex<Option<Arc<AtomicBool>>>,
    progress: Mutex<IndexProgress>,
}

impl Default for IndexCoordinator {
    fn default() -> Self {
        Self {
            inner: Arc::new(IndexCoordinatorInner {
                running: AtomicBool::new(false),
                cancellation: Mutex::new(None),
                progress: Mutex::new(IndexProgress::default()),
            }),
        }
    }
}

impl IndexCoordinator {
    pub fn start(&self, session: Arc<ArchiveSession>) -> AppResult<()> {
        if self
            .inner
            .running
            .compare_exchange(false, true, Ordering::AcqRel, Ordering::Acquire)
            .is_err()
        {
            return Err(ErrorCode::IndexBusy.into());
        }

        let cancellation = Arc::new(AtomicBool::new(false));
        {
            let mut slot = self
                .inner
                .cancellation
                .lock()
                .map_err(|_| ErrorCode::Internal)?;
            *slot = Some(cancellation.clone());
        }
        {
            let mut progress = self
                .inner
                .progress
                .lock()
                .map_err(|_| ErrorCode::Internal)?;
            *progress = IndexProgress {
                phase: IndexPhase::Discovering,
                failure_code: None,
                shards_total: session.root.shards().len(),
                shards_complete: 0,
                bytes_total: session.root.shards().iter().map(|shard| shard.size).sum(),
                bytes_processed: 0,
                conversations_indexed: 0,
                conversations_skipped: 0,
                diagnostics: 0,
            };
        }

        let coordinator = self.clone();
        let _task = tokio::task::spawn_blocking(move || {
            coordinator.run(session, cancellation);
        });
        Ok(())
    }

    pub fn cancel(&self) -> AppResult<IndexProgress> {
        let cancellation_guard = self
            .inner
            .cancellation
            .lock()
            .map_err(|_| ErrorCode::Internal)?;
        let Some(cancellation) = cancellation_guard.as_ref() else {
            return Ok(self.progress());
        };
        cancellation.store(true, Ordering::Release);
        drop(cancellation_guard);
        if let Ok(mut progress) = self.inner.progress.lock()
            && matches!(
                progress.phase,
                IndexPhase::Discovering | IndexPhase::Indexing
            )
        {
            progress.phase = IndexPhase::Cancelling;
        }
        Ok(self.progress())
    }

    pub fn progress(&self) -> IndexProgress {
        self.inner
            .progress
            .lock()
            .map_or_else(|_| IndexProgress::default(), |progress| progress.clone())
    }

    pub fn is_running(&self) -> bool {
        self.inner.running.load(Ordering::Acquire)
    }

    pub fn reset(&self) -> AppResult<IndexProgress> {
        if self.is_running() {
            return Err(ErrorCode::IndexBusy.into());
        }
        let mut progress = self
            .inner
            .progress
            .lock()
            .map_err(|_| ErrorCode::Internal)?;
        *progress = IndexProgress::default();
        Ok(progress.clone())
    }

    fn run(&self, session: Arc<ArchiveSession>, cancellation: Arc<AtomicBool>) {
        let result = self.run_inner(&session, &cancellation);
        if let Ok(mut progress) = self.inner.progress.lock() {
            progress.phase = match result {
                Ok(()) => {
                    progress.failure_code = None;
                    IndexPhase::Complete
                }
                Err(crate::error::AppError::Public(ErrorCode::IndexCancelled)) => {
                    progress.failure_code = None;
                    IndexPhase::Cancelled
                }
                Err(error) => {
                    progress.failure_code = Some(error.code());
                    progress.diagnostics = progress.diagnostics.saturating_add(1);
                    IndexPhase::Failed
                }
            };
        }
        if let Ok(mut slot) = self.inner.cancellation.lock() {
            *slot = None;
        }
        self.inner.running.store(false, Ordering::Release);
    }

    fn run_inner(&self, session: &ArchiveSession, cancellation: &AtomicBool) -> AppResult<()> {
        let _lock = session.store.acquire_index_lock()?;
        let attachment_fingerprint = session.root.attachment_inventory_fingerprint();
        let attachment_inventory_is_current = session
            .store
            .attachment_inventory_is_current(&attachment_fingerprint)?;
        self.set_phase(IndexPhase::Indexing);
        for shard in session.root.shards() {
            if cancellation.load(Ordering::Acquire) {
                return Err(ErrorCode::IndexCancelled.into());
            }
            if attachment_inventory_is_current
                && session.store.shard_is_current(&session.root, shard)?
            {
                self.update_progress(|progress| {
                    progress.shards_complete = progress.shards_complete.saturating_add(1);
                    progress.bytes_processed =
                        progress.bytes_processed.saturating_add(shard.size);
                });
                continue;
            }

            let stats = session.store.index_shard(
                &session.root,
                shard,
                cancellation,
                |byte_count| {
                    self.update_progress(|progress| {
                        progress.bytes_processed =
                            progress.bytes_processed.saturating_add(byte_count);
                    });
                },
            )?;
            self.update_progress(|progress| {
                progress.shards_complete = progress.shards_complete.saturating_add(1);
                progress.conversations_indexed = progress
                    .conversations_indexed
                    .saturating_add(stats.conversations_indexed);
                progress.conversations_skipped = progress
                    .conversations_skipped
                    .saturating_add(stats.conversations_skipped);
                progress.diagnostics = progress.diagnostics.saturating_add(stats.diagnostics);
            });
        }
        if !session.source_remains_unchanged() {
            return Err(ErrorCode::PathRejected.into());
        }
        session
            .store
            .finalize_archive_state(session.root.shards(), &attachment_fingerprint)?;
        Ok(())
    }

    fn set_phase(&self, phase: IndexPhase) {
        self.update_progress(|progress| progress.phase = phase);
    }

    fn update_progress<F>(&self, update: F)
    where
        F: FnOnce(&mut IndexProgress),
    {
        if let Ok(mut progress) = self.inner.progress.lock() {
            update(&mut progress);
        }
    }
}

#[cfg(test)]
mod tests {
    use std::fs;

    use tempfile::TempDir;

    use super::*;

    fn synthetic_session() -> (TempDir, Arc<ArchiveSession>) {
        let directory = TempDir::new().expect("temp directory");
        let export = directory.path().join("export");
        fs::create_dir(&export).expect("create export");
        fs::write(
            export.join("conversations-000.json"),
            r#"[{
              "title":"Synthetic",
              "current_node":"node-a",
              "mapping":{
                "node-a":{
                  "parent":null,
                  "children":[],
                  "message":{
                    "author":{"role":"user"},
                    "content":{"parts":["Synthetic message"]}
                  }
                }
              }
            }]"#,
        )
        .expect("write shard");
        let root = SafeExportRoot::select(&export).expect("select root");
        let cache = directory.path().join("cache");
        let session =
            Arc::new(ArchiveSession::new_with_cache_root(root, &cache).expect("session"));
        session.store.discard().expect("clean index");
        (directory, session)
    }

    #[test]
    fn synchronous_run_completes_and_is_resumable() {
        let (_directory, session) = synthetic_session();
        let coordinator = IndexCoordinator::default();
        coordinator
            .run_inner(&session, &AtomicBool::new(false))
            .expect("first run");
        let first = coordinator.progress();
        assert_eq!(first.shards_complete, 1);
        assert_eq!(first.conversations_indexed, 1);

        coordinator
            .run_inner(&session, &AtomicBool::new(false))
            .expect("resume");
        let resumed = coordinator.progress();
        assert_eq!(resumed.shards_complete, 2);
    }

    #[test]
    fn cancellation_is_checked_before_a_shard() {
        let (_directory, session) = synthetic_session();
        let coordinator = IndexCoordinator::default();
        let cancelled = AtomicBool::new(true);
        assert!(matches!(
            coordinator.run_inner(&session, &cancelled),
            Err(crate::error::AppError::Public(ErrorCode::IndexCancelled))
        ));
    }

    #[test]
    fn successful_refresh_prunes_a_removed_shard() {
        let (directory, seed_session) = synthetic_session();
        let export = directory.path().join("export");
        let cache = directory.path().join("cache");
        drop(seed_session);
        fs::copy(
            export.join("conversations-000.json"),
            export.join("conversations-001.json"),
        )
        .expect("copy second shard");

        let first_root = SafeExportRoot::select(&export).expect("select first root");
        let first_session = ArchiveSession::new_with_cache_root(first_root, &cache)
            .expect("create first session");
        let coordinator = IndexCoordinator::default();
        coordinator
            .run_inner(&first_session, &AtomicBool::new(false))
            .expect("index two shards");
        assert_eq!(conversation_total(&first_session.store), 2);
        drop(first_session);

        fs::remove_file(export.join("conversations-001.json")).expect("remove second shard");
        let refreshed_root = SafeExportRoot::select(&export).expect("select refreshed root");
        let refreshed_session = ArchiveSession::new_with_cache_root(refreshed_root, &cache)
            .expect("create refreshed session");
        coordinator
            .run_inner(&refreshed_session, &AtomicBool::new(false))
            .expect("refresh one shard");
        assert_eq!(conversation_total(&refreshed_session.store), 1);
    }

    fn conversation_total(store: &Store) -> u64 {
        store
            .query_conversations(&crate::models::ConversationQuery {
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
            .expect("query conversations")
            .total
    }
}
