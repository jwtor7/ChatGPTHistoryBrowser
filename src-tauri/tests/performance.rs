use std::{
    fs::File,
    io::{BufWriter, Write},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
    },
    thread::JoinHandle,
    time::{Duration, Instant},
};

use chatgpt_history_browser::{
    models::ConversationQuery, safe_root::SafeExportRoot, store::Store,
};
use serde_json::json;
use tempfile::TempDir;

const INDEX_10K_LIMIT: Duration = Duration::from_secs(120);
const INDEX_10K_ATTACHMENTS_LIMIT: Duration = Duration::from_secs(120);
const SEARCH_10K_LIMIT: Duration = Duration::from_secs(5);
const INDEX_500_MIB_LIMIT: Duration = Duration::from_secs(600);
const PEAK_RSS_10K_LIMIT_BYTES: u64 = 1024 * 1024 * 1024;
const PEAK_RSS_500_MIB_LIMIT_BYTES: u64 = 1536 * 1024 * 1024;

#[test]
#[ignore = "performance test"]
fn performance_10000() {
    let directory = TempDir::new().unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    let export = directory.path().join("export");
    std::fs::create_dir(&export).unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    write_synthetic_shard(&export, 10_000, 48);
    let root =
        SafeExportRoot::select(&export).unwrap_or_else(|_| panic!("fixture validation failed"));
    let store = Store::for_export_with_cache_root(&root, &directory.path().join("cache"))
        .unwrap_or_else(|_| panic!("temporary index setup failed"));
    store
        .discard()
        .unwrap_or_else(|_| panic!("temporary index reset failed"));

    let rss_sampler = PeakRssSampler::start();
    let index_started = Instant::now();
    let stats = store
        .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
        .unwrap_or_else(|_| panic!("synthetic indexing failed"));
    let index_elapsed = index_started.elapsed();
    assert_eq!(stats.conversations_indexed, 10_000);
    assert!(
        index_elapsed < INDEX_10K_LIMIT,
        "10k indexing exceeded the release performance budget"
    );

    let query_started = Instant::now();
    let page = store
        .query_conversations(&ConversationQuery {
            page: Some(0),
            page_size: Some(100),
            search: Some("Synthetic searchable token".to_string()),
            date_from: None,
            date_to: None,
            role: Some("assistant".to_string()),
            archived: None,
            starred: None,
            has_attachments: None,
        })
        .unwrap_or_else(|_| panic!("synthetic search failed"));
    let query_elapsed = query_started.elapsed();
    let peak_rss_bytes = rss_sampler.finish();
    assert_eq!(page.total, 10_000);
    assert!(
        query_elapsed < SEARCH_10K_LIMIT,
        "10k full-text query exceeded the release performance budget"
    );
    assert_peak_rss_within(peak_rss_bytes, PEAK_RSS_10K_LIMIT_BYTES, "10k");

    print_metrics(
        "PERF_10K",
        stats.conversations_indexed,
        root.validation().total_json_bytes,
        index_elapsed,
        Some(query_elapsed),
        page.total,
        peak_rss_bytes,
        PEAK_RSS_10K_LIMIT_BYTES,
    );
}

#[test]
#[ignore = "performance test"]
fn performance_10000_attachments() {
    let directory = TempDir::new().unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    let export = directory.path().join("export");
    std::fs::create_dir(&export).unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    write_synthetic_attachment_export(&export, 10_000);
    let root =
        SafeExportRoot::select(&export).unwrap_or_else(|_| panic!("fixture validation failed"));
    assert_eq!(root.attachment_count(), 10_000);
    let store = Store::for_export_with_cache_root(&root, &directory.path().join("cache"))
        .unwrap_or_else(|_| panic!("temporary index setup failed"));
    store
        .discard()
        .unwrap_or_else(|_| panic!("temporary index reset failed"));

    let rss_sampler = PeakRssSampler::start();
    let index_started = Instant::now();
    let stats = store
        .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
        .unwrap_or_else(|_| panic!("synthetic attachment indexing failed"));
    let index_elapsed = index_started.elapsed();
    let peak_rss_bytes = rss_sampler.finish();
    assert_eq!(stats.conversations_indexed, 10_000);
    assert!(
        index_elapsed < INDEX_10K_ATTACHMENTS_LIMIT,
        "10k attachment indexing exceeded the release performance budget"
    );
    assert_peak_rss_within(peak_rss_bytes, PEAK_RSS_10K_LIMIT_BYTES, "10k attachments");

    print_metrics(
        "PERF_10K_ATTACHMENTS",
        stats.conversations_indexed,
        root.validation().total_json_bytes,
        index_elapsed,
        None,
        0,
        peak_rss_bytes,
        PEAK_RSS_10K_LIMIT_BYTES,
    );
}

#[test]
#[ignore = "release-scale performance test"]
fn performance_stream_500mb() {
    let directory = TempDir::new().unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    let export = directory.path().join("export");
    std::fs::create_dir(&export).unwrap_or_else(|_| panic!("temporary fixture setup failed"));
    let text_bytes_per_record = 64 * 1024;
    let record_count = 8_100;
    write_synthetic_shard(&export, record_count, text_bytes_per_record);
    let root =
        SafeExportRoot::select(&export).unwrap_or_else(|_| panic!("fixture validation failed"));
    assert!(
        root.validation().total_json_bytes >= 500 * 1024 * 1024,
        "release-scale fixture must contain at least 500 MiB of JSON"
    );
    let store = Store::for_export_with_cache_root(&root, &directory.path().join("cache"))
        .unwrap_or_else(|_| panic!("temporary index setup failed"));
    store
        .discard()
        .unwrap_or_else(|_| panic!("temporary index reset failed"));

    let rss_sampler = PeakRssSampler::start();
    let index_started = Instant::now();
    let stats = store
        .index_shard(&root, &root.shards()[0], &AtomicBool::new(false), |_| {})
        .unwrap_or_else(|_| panic!("release-scale synthetic indexing failed"));
    let index_elapsed = index_started.elapsed();
    let peak_rss_bytes = rss_sampler.finish();
    assert_eq!(stats.conversations_indexed, record_count as u64);
    assert!(
        index_elapsed < INDEX_500_MIB_LIMIT,
        "500 MiB indexing exceeded the release performance budget"
    );
    assert_peak_rss_within(peak_rss_bytes, PEAK_RSS_500_MIB_LIMIT_BYTES, "500 MiB");

    print_metrics(
        "PERF_500_MIB",
        stats.conversations_indexed,
        root.validation().total_json_bytes,
        index_elapsed,
        None,
        0,
        peak_rss_bytes,
        PEAK_RSS_500_MIB_LIMIT_BYTES,
    );
}

fn write_synthetic_shard(directory: &std::path::Path, count: usize, text_bytes: usize) {
    let file = File::create(directory.join("conversations-000.json"))
        .unwrap_or_else(|_| panic!("fixture creation failed"));
    let mut writer = BufWriter::new(file);
    writer
        .write_all(b"[")
        .unwrap_or_else(|_| panic!("fixture generation failed"));
    let synthetic_text = format!(
        "Synthetic searchable token {}",
        "s".repeat(text_bytes.saturating_sub(27))
    );
    for index in 0..count {
        if index > 0 {
            writer
                .write_all(b",")
                .unwrap_or_else(|_| panic!("fixture generation failed"));
        }
        let conversation = json!({
            "id": format!("synthetic-conversation-{index:06}"),
            "title": format!("Synthetic conversation {index:06}"),
            "create_time": 4_102_444_800_f64 + index as f64,
            "update_time": 4_102_444_900_f64 + index as f64,
            "current_node": "synthetic-node-assistant",
            "mapping": {
                "synthetic-node-user": {
                    "parent": null,
                    "children": ["synthetic-node-assistant"],
                    "message": {
                        "id": format!("synthetic-message-user-{index:06}"),
                        "author": {"role": "user"},
                        "content": {
                            "content_type": "text",
                            "parts": ["Synthetic prompt"]
                        }
                    }
                },
                "synthetic-node-assistant": {
                    "parent": "synthetic-node-user",
                    "children": [],
                    "message": {
                        "id": format!("synthetic-message-assistant-{index:06}"),
                        "author": {"role": "assistant"},
                        "content": {
                            "content_type": "text",
                            "parts": [&synthetic_text]
                        }
                    }
                }
            }
        });
        serde_json::to_writer(&mut writer, &conversation)
            .unwrap_or_else(|_| panic!("fixture generation failed"));
    }
    writer
        .write_all(b"]")
        .unwrap_or_else(|_| panic!("fixture generation failed"));
    writer
        .flush()
        .unwrap_or_else(|_| panic!("fixture generation failed"));
}

fn write_synthetic_attachment_export(directory: &std::path::Path, count: usize) {
    const SYNTHETIC_PNG_HEADER: &[u8] = b"\x89PNG\r\n\x1a\n\0\0\0\rIHDR\0\0\0\x01\0\0\0\x01";
    let file = File::create(directory.join("conversations-000.json"))
        .unwrap_or_else(|_| panic!("fixture creation failed"));
    let mut writer = BufWriter::new(file);
    writer
        .write_all(b"[")
        .unwrap_or_else(|_| panic!("fixture generation failed"));
    for index in 0..count {
        if index > 0 {
            writer
                .write_all(b",")
                .unwrap_or_else(|_| panic!("fixture generation failed"));
        }
        let reference = format!("file-{index:06}");
        let conversation = json!({
            "id": format!("synthetic-attachment-conversation-{index:06}"),
            "title": format!("Synthetic attachment conversation {index:06}"),
            "current_node": "synthetic-node",
            "mapping": {
                "synthetic-node": {
                    "parent": null,
                    "children": [],
                    "message": {
                        "id": format!("synthetic-attachment-message-{index:06}"),
                        "author": {"role": "user"},
                        "content": {
                            "content_type": "text",
                            "parts": ["Synthetic attachment prompt"]
                        },
                        "metadata": {
                            "attachments": [{
                                "file_id": reference,
                                "name": format!("Synthetic image {index:06}.png"),
                                "mime_type": "image/png"
                            }]
                        }
                    }
                }
            }
        });
        serde_json::to_writer(&mut writer, &conversation)
            .unwrap_or_else(|_| panic!("fixture generation failed"));
        std::fs::write(
            directory.join(format!("file-{index:06}.dat")),
            SYNTHETIC_PNG_HEADER,
        )
        .unwrap_or_else(|_| panic!("attachment fixture generation failed"));
    }
    writer
        .write_all(b"]")
        .unwrap_or_else(|_| panic!("fixture generation failed"));
    writer
        .flush()
        .unwrap_or_else(|_| panic!("fixture generation failed"));
}

fn assert_peak_rss_within(peak_rss_bytes: Option<u64>, limit_bytes: u64, scale: &str) {
    if let Some(peak_rss_bytes) = peak_rss_bytes {
        assert!(
            peak_rss_bytes <= limit_bytes,
            "{scale} peak RSS exceeded the release performance budget"
        );
    }
}

#[allow(clippy::too_many_arguments)]
fn print_metrics(
    label: &str,
    conversations: u64,
    source_bytes: u64,
    index_elapsed: Duration,
    search_elapsed: Option<Duration>,
    search_results: u64,
    peak_rss_bytes: Option<u64>,
    peak_rss_limit_bytes: u64,
) {
    let search_ms = search_elapsed
        .map(|duration| duration.as_millis().to_string())
        .unwrap_or_else(|| "not_run".to_string());
    let peak_rss = peak_rss_bytes
        .map(|bytes| bytes.to_string())
        .unwrap_or_else(|| "unavailable".to_string());
    println!(
        "{label} conversations={conversations} source_bytes={source_bytes} \
         index_ms={} search_ms={search_ms} search_results={search_results} \
         peak_rss_bytes={peak_rss} peak_rss_limit_bytes={peak_rss_limit_bytes}",
        index_elapsed.as_millis()
    );
}

struct PeakRssSampler {
    stop: Arc<AtomicBool>,
    worker: Option<JoinHandle<Option<u64>>>,
}

impl PeakRssSampler {
    fn start() -> Self {
        let stop = Arc::new(AtomicBool::new(false));
        let worker_stop = Arc::clone(&stop);
        let worker = std::thread::spawn(move || {
            let mut peak = current_rss_bytes();
            while !worker_stop.load(Ordering::Relaxed) {
                if let Some(current) = current_rss_bytes() {
                    peak = Some(peak.map_or(current, |previous| previous.max(current)));
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            if let Some(current) = current_rss_bytes() {
                peak = Some(peak.map_or(current, |previous| previous.max(current)));
            }
            peak
        });
        Self {
            stop,
            worker: Some(worker),
        }
    }

    fn finish(mut self) -> Option<u64> {
        self.stop.store(true, Ordering::Relaxed);
        self.worker
            .take()
            .and_then(|worker| worker.join().unwrap_or(None))
    }
}

impl Drop for PeakRssSampler {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Relaxed);
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }
}

#[cfg(target_os = "linux")]
fn current_rss_bytes() -> Option<u64> {
    let status = std::fs::read_to_string("/proc/self/status").ok()?;
    let kibibytes = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))?
        .split_whitespace()
        .next()?
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(target_os = "macos")]
fn current_rss_bytes() -> Option<u64> {
    let output = std::process::Command::new("/bin/ps")
        .args(["-o", "rss=", "-p", &std::process::id().to_string()])
        .output()
        .ok()?;
    if !output.status.success() {
        return None;
    }
    let kibibytes = std::str::from_utf8(&output.stdout)
        .ok()?
        .trim()
        .parse::<u64>()
        .ok()?;
    kibibytes.checked_mul(1024)
}

#[cfg(not(any(target_os = "linux", target_os = "macos")))]
fn current_rss_bytes() -> Option<u64> {
    None
}
