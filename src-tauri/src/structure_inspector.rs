use std::{
    collections::HashMap,
    ffi::OsString,
    io::{Read, Write},
    path::Path,
};

use serde::Serialize;
use serde_json::{Map, Value};

use crate::{
    json_stream::{JsonStreamLimits, stream_json_array},
    safe_root::SafeExportRoot,
};

pub const EXIT_COMPLETE: i32 = 0;
pub const EXIT_REJECTED: i32 = 2;
pub const EXIT_PARTIAL: i32 = 3;
pub const EXIT_USAGE: i32 = 64;
pub const EXIT_OUTPUT_ERROR: i32 = 74;

const PATH_STDIN_FLAG: &str = "--path-stdin";
const MAX_STDIN_PATH_BYTES: u64 = 32 * 1024;

const KNOWN_CONVERSATION_KEYS: &[&str] = &[
    "id",
    "conversation_id",
    "title",
    "create_time",
    "update_time",
    "mapping",
    "current_node",
    "is_archived",
    "archived",
    "is_starred",
    "starred",
];
const KNOWN_NODE_KEYS: &[&str] = &["id", "message", "parent", "children"];
const KNOWN_MESSAGE_KEYS: &[&str] = &[
    "id",
    "author",
    "create_time",
    "update_time",
    "content",
    "status",
    "end_turn",
    "weight",
    "metadata",
    "recipient",
    "channel",
];
const KNOWN_AUTHOR_KEYS: &[&str] = &["role", "name", "metadata"];
const KNOWN_CONTENT_KEYS: &[&str] = &["content_type", "parts", "text"];

const REPORT_KEYS: &[&str] = &[
    "shardCount",
    "parsedShardCount",
    "malformedShardCount",
    "conversationRecordCount",
    "objectRecordCount",
    "nonObjectRecordCount",
    "knownFieldPresence",
    "mappingNodeCount",
    "missingReferenceCount",
    "cycleCount",
    "oversizedRecordCount",
    "unknownKeyCount",
    "sourceUnchanged",
];
const KNOWN_FIELD_REPORT_KEYS: &[&str] = &[
    "id",
    "conversationId",
    "title",
    "createTime",
    "updateTime",
    "mapping",
    "currentNode",
    "isArchived",
    "archived",
    "isStarred",
    "starred",
];

const EMPTY_REPORT_JSON: &[u8] = br#"{"shardCount":0,"parsedShardCount":0,"malformedShardCount":0,"conversationRecordCount":0,"objectRecordCount":0,"nonObjectRecordCount":0,"knownFieldPresence":{"id":0,"conversationId":0,"title":0,"createTime":0,"updateTime":0,"mapping":0,"currentNode":0,"isArchived":0,"archived":0,"isStarred":0,"starred":0},"mappingNodeCount":0,"missingReferenceCount":0,"cycleCount":0,"oversizedRecordCount":0,"unknownKeyCount":0,"sourceUnchanged":false}"#;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct KnownFieldPresenceCounts {
    pub id: u64,
    pub conversation_id: u64,
    pub title: u64,
    pub create_time: u64,
    pub update_time: u64,
    pub mapping: u64,
    pub current_node: u64,
    pub is_archived: u64,
    pub archived: u64,
    pub is_starred: u64,
    pub starred: u64,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Serialize)]
#[serde(rename_all = "camelCase")]
pub struct StructureReport {
    pub shard_count: u64,
    pub parsed_shard_count: u64,
    pub malformed_shard_count: u64,
    pub conversation_record_count: u64,
    pub object_record_count: u64,
    pub non_object_record_count: u64,
    pub known_field_presence: KnownFieldPresenceCounts,
    pub mapping_node_count: u64,
    pub missing_reference_count: u64,
    pub cycle_count: u64,
    pub oversized_record_count: u64,
    pub unknown_key_count: u64,
    pub source_unchanged: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum InspectionCompletion {
    Complete,
    Partial,
    Rejected,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InspectionOutcome {
    pub report: StructureReport,
    pub completion: InspectionCompletion,
}

pub fn inspect_structure(root_path: &Path) -> InspectionOutcome {
    inspect_structure_with_limits(root_path, JsonStreamLimits::default())
}

fn inspect_structure_with_limits(
    root_path: &Path,
    limits: JsonStreamLimits,
) -> InspectionOutcome {
    let Ok(root) = SafeExportRoot::select(root_path) else {
        return InspectionOutcome {
            report: StructureReport::default(),
            completion: InspectionCompletion::Rejected,
        };
    };

    let mut baseline = root.source_fingerprint();
    sort_fingerprint(&mut baseline);

    let mut report = StructureReport {
        shard_count: usize_to_u64(root.shards().len()),
        ..StructureReport::default()
    };

    for shard in root.shards() {
        let Ok(file) = root.open_entry(shard) else {
            increment(&mut report.malformed_shard_count);
            continue;
        };

        let stream_result = stream_json_array(file, limits, |record, _ordinal| {
            increment(&mut report.conversation_record_count);
            let value: Value = serde_json::from_slice(record)?;
            inspect_record(&value, &mut report);
            Ok(())
        });

        match stream_result {
            Ok(stats) => {
                increment(&mut report.parsed_shard_count);
                add(&mut report.oversized_record_count, stats.records_too_large);
                add(
                    &mut report.conversation_record_count,
                    stats.records_too_large,
                );
            }
            Err(_) => increment(&mut report.malformed_shard_count),
        }
    }

    report.source_unchanged = source_is_unchanged(root_path, &baseline);
    let partial = report.malformed_shard_count > 0
        || report.oversized_record_count > 0
        || !report.source_unchanged;

    InspectionOutcome {
        report,
        completion: if partial {
            InspectionCompletion::Partial
        } else {
            InspectionCompletion::Complete
        },
    }
}

pub fn run_cli<I, R, W>(arguments: I, input: &mut R, output: &mut W) -> i32
where
    I: IntoIterator<Item = OsString>,
    R: Read,
    W: Write,
{
    let mut arguments = arguments.into_iter();
    let first = arguments.next();
    let has_extra = arguments.next().is_some();

    let (report, intended_exit) = match (first, has_extra) {
        (Some(argument), false) if argument == PATH_STDIN_FLAG => {
            match read_stdin_path(input) {
                Some(root) => report_for_root(Path::new(&root)),
                None => (StructureReport::default(), EXIT_USAGE),
            }
        }
        _ => (StructureReport::default(), EXIT_USAGE),
    };

    let mut serialized = serialize_validated_report(&report);
    serialized.push(b'\n');
    if output.write_all(&serialized).is_err() {
        return EXIT_OUTPUT_ERROR;
    }
    intended_exit
}

fn read_stdin_path<R: Read>(input: &mut R) -> Option<OsString> {
    let mut bytes = Vec::new();
    let mut bounded = input.take(MAX_STDIN_PATH_BYTES.saturating_add(1));
    bounded.read_to_end(&mut bytes).ok()?;
    if usize_to_u64(bytes.len()) > MAX_STDIN_PATH_BYTES {
        return None;
    }

    if bytes.last() == Some(&b'\n') {
        bytes.pop();
        if bytes.last() == Some(&b'\r') {
            bytes.pop();
        }
    }
    if bytes.is_empty()
        || bytes
            .iter()
            .any(|byte| matches!(*byte, b'\0' | b'\r' | b'\n'))
    {
        return None;
    }

    String::from_utf8(bytes).ok().map(OsString::from)
}

fn report_for_root(root: &Path) -> (StructureReport, i32) {
    let outcome = inspect_structure(root);
    let exit = match outcome.completion {
        InspectionCompletion::Complete => EXIT_COMPLETE,
        InspectionCompletion::Partial => EXIT_PARTIAL,
        InspectionCompletion::Rejected => EXIT_REJECTED,
    };
    (outcome.report, exit)
}

fn inspect_record(value: &Value, report: &mut StructureReport) {
    let Some(conversation) = value.as_object() else {
        increment(&mut report.non_object_record_count);
        return;
    };

    increment(&mut report.object_record_count);
    count_unknown_keys(
        conversation,
        KNOWN_CONVERSATION_KEYS,
        &mut report.unknown_key_count,
    );
    count_known_fields(conversation, &mut report.known_field_presence);

    let Some(mapping) = conversation.get("mapping").and_then(Value::as_object) else {
        return;
    };
    add(&mut report.mapping_node_count, usize_to_u64(mapping.len()));
    inspect_mapping(mapping, conversation.get("current_node"), report);
}

fn count_known_fields(
    conversation: &Map<String, Value>,
    counts: &mut KnownFieldPresenceCounts,
) {
    if conversation.contains_key("id") {
        increment(&mut counts.id);
    }
    if conversation.contains_key("conversation_id") {
        increment(&mut counts.conversation_id);
    }
    if conversation.contains_key("title") {
        increment(&mut counts.title);
    }
    if conversation.contains_key("create_time") {
        increment(&mut counts.create_time);
    }
    if conversation.contains_key("update_time") {
        increment(&mut counts.update_time);
    }
    if conversation.contains_key("mapping") {
        increment(&mut counts.mapping);
    }
    if conversation.contains_key("current_node") {
        increment(&mut counts.current_node);
    }
    if conversation.contains_key("is_archived") {
        increment(&mut counts.is_archived);
    }
    if conversation.contains_key("archived") {
        increment(&mut counts.archived);
    }
    if conversation.contains_key("is_starred") {
        increment(&mut counts.is_starred);
    }
    if conversation.contains_key("starred") {
        increment(&mut counts.starred);
    }
}

fn inspect_mapping(
    mapping: &Map<String, Value>,
    current_node: Option<&Value>,
    report: &mut StructureReport,
) {
    let mut aliases = HashMap::with_capacity(mapping.len().saturating_mul(2));
    for (index, (mapping_key, node_value)) in mapping.iter().enumerate() {
        aliases.entry(mapping_key.as_str()).or_insert(index);
        if let Some(node_id) = node_value
            .as_object()
            .and_then(|node| node.get("id"))
            .and_then(Value::as_str)
        {
            aliases.entry(node_id).or_insert(index);
        }
    }

    let mut edges = vec![Vec::new(); mapping.len()];
    for (index, node_value) in mapping.values().enumerate() {
        let Some(node) = node_value.as_object() else {
            continue;
        };
        count_unknown_keys(node, KNOWN_NODE_KEYS, &mut report.unknown_key_count);

        if let Some(parent) = node.get("parent").and_then(Value::as_str) {
            if let Some(parent_index) = aliases.get(parent).copied() {
                edges[parent_index].push(index);
            } else {
                increment(&mut report.missing_reference_count);
            }
        }

        if let Some(children) = node.get("children").and_then(Value::as_array) {
            for child in children {
                let Some(child) = child.as_str() else {
                    continue;
                };
                if let Some(child_index) = aliases.get(child).copied() {
                    edges[index].push(child_index);
                } else {
                    increment(&mut report.missing_reference_count);
                }
            }
        }

        inspect_message_shape(node.get("message"), report);
    }

    if let Some(current_node) = current_node.and_then(Value::as_str)
        && !aliases.contains_key(current_node)
    {
        increment(&mut report.missing_reference_count);
    }

    for neighbors in &mut edges {
        neighbors.sort_unstable();
        neighbors.dedup();
    }
    add(&mut report.cycle_count, count_cycle_back_edges(&edges));
}

fn inspect_message_shape(message: Option<&Value>, report: &mut StructureReport) {
    let Some(message) = message.and_then(Value::as_object) else {
        return;
    };
    count_unknown_keys(message, KNOWN_MESSAGE_KEYS, &mut report.unknown_key_count);

    if let Some(author) = message.get("author").and_then(Value::as_object) {
        count_unknown_keys(author, KNOWN_AUTHOR_KEYS, &mut report.unknown_key_count);
    }
    if let Some(content) = message.get("content").and_then(Value::as_object) {
        count_unknown_keys(content, KNOWN_CONTENT_KEYS, &mut report.unknown_key_count);
    }
}

fn count_unknown_keys(object: &Map<String, Value>, known: &[&str], destination: &mut u64) {
    for key in object.keys() {
        if !known.contains(&key.as_str()) {
            increment(destination);
        }
    }
}

fn count_cycle_back_edges(edges: &[Vec<usize>]) -> u64 {
    let mut states = vec![0_u8; edges.len()];
    let mut cycles = 0_u64;

    for root in 0..edges.len() {
        if states[root] != 0 {
            continue;
        }
        states[root] = 1;
        let mut stack = vec![(root, 0_usize)];

        while let Some((node, next_neighbor)) = stack.last_mut() {
            if *next_neighbor >= edges[*node].len() {
                states[*node] = 2;
                stack.pop();
                continue;
            }

            let neighbor = edges[*node][*next_neighbor];
            *next_neighbor += 1;
            match states[neighbor] {
                0 => {
                    states[neighbor] = 1;
                    stack.push((neighbor, 0));
                }
                1 => increment(&mut cycles),
                _ => {}
            }
        }
    }
    cycles
}

fn source_is_unchanged(root_path: &Path, baseline: &[(String, u64, u128)]) -> bool {
    let Ok(current_root) = SafeExportRoot::select(root_path) else {
        return false;
    };
    let mut current = current_root.source_fingerprint();
    sort_fingerprint(&mut current);
    current == baseline
}

fn sort_fingerprint(fingerprint: &mut [(String, u64, u128)]) {
    fingerprint.sort_unstable_by(|left, right| {
        left.0
            .cmp(&right.0)
            .then_with(|| left.1.cmp(&right.1))
            .then_with(|| left.2.cmp(&right.2))
    });
}

fn serialize_validated_report(report: &StructureReport) -> Vec<u8> {
    match serde_json::to_vec(report) {
        Ok(serialized) if is_fixed_report_json(&serialized) => serialized,
        _ => EMPTY_REPORT_JSON.to_vec(),
    }
}

fn is_fixed_report_json(serialized: &[u8]) -> bool {
    let Ok(Value::Object(report)) = serde_json::from_slice(serialized) else {
        return false;
    };
    if !has_exact_keys(&report, REPORT_KEYS) {
        return false;
    }

    for key in REPORT_KEYS {
        match *key {
            "knownFieldPresence" | "sourceUnchanged" => {}
            _ if report.get(*key).and_then(Value::as_u64).is_some() => {}
            _ => return false,
        }
    }
    if report
        .get("sourceUnchanged")
        .and_then(Value::as_bool)
        .is_none()
    {
        return false;
    }

    let Some(Value::Object(known_fields)) = report.get("knownFieldPresence") else {
        return false;
    };
    has_exact_keys(known_fields, KNOWN_FIELD_REPORT_KEYS)
        && KNOWN_FIELD_REPORT_KEYS
            .iter()
            .all(|key| known_fields.get(*key).and_then(Value::as_u64).is_some())
}

fn has_exact_keys(object: &Map<String, Value>, expected: &[&str]) -> bool {
    object.len() == expected.len() && expected.iter().all(|key| object.contains_key(*key))
}

fn increment(value: &mut u64) {
    *value = value.saturating_add(1);
}

fn add(value: &mut u64, amount: u64) {
    *value = value.saturating_add(amount);
}

fn usize_to_u64(value: usize) -> u64 {
    u64::try_from(value).unwrap_or(u64::MAX)
}

#[cfg(test)]
mod tests {
    use std::{
        fs,
        io::{self, Write},
    };

    use serde_json::json;
    use tempfile::TempDir;

    use super::*;

    fn write_shard(directory: &TempDir, ordinal: u32, value: &Value) {
        let name = format!("conversations-{ordinal:03}.json");
        fs::write(
            directory.path().join(name),
            serde_json::to_vec(value).expect("serialize synthetic fixture"),
        )
        .expect("write synthetic shard");
    }

    fn private_canaries() -> Vec<String> {
        vec![
            ["canary", "@", "example.com"].concat(),
            ["/", "Users", "/", "SyntheticCanary", "/", "private"].concat(),
            ["permission", " denied: synthetic canary"].concat(),
            ["dynamic", "-private-key-", "synthetic"].concat(),
            ["private", "-identifier-", "synthetic"].concat(),
        ]
    }

    fn canary_fixture(canaries: &[String]) -> Value {
        let email = &canaries[0];
        let private_path = &canaries[1];
        let error_fragment = &canaries[2];
        let unknown_key = &canaries[3];
        let private_identifier = &canaries[4];

        let mut first_node = Map::new();
        first_node.insert("id".to_string(), json!(email));
        first_node.insert("parent".to_string(), json!(private_path));
        first_node.insert("children".to_string(), json!([private_identifier]));
        first_node.insert(
            "message".to_string(),
            json!({
                "id": private_identifier,
                "author": {
                    "role": "synthetic-private-role",
                    "private_author_key": private_path
                },
                "content": {
                    "content_type": "text",
                    "parts": [error_fragment],
                    "private_content_key": email
                },
                "private_message_key": error_fragment
            }),
        );
        first_node.insert(unknown_key.clone(), json!(private_path));

        let mut second_node = Map::new();
        second_node.insert("id".to_string(), json!(private_identifier));
        second_node.insert("parent".to_string(), json!(email));
        second_node.insert("children".to_string(), json!([email]));

        let mut mapping = Map::new();
        mapping.insert(email.clone(), Value::Object(first_node));
        mapping.insert(private_identifier.clone(), Value::Object(second_node));

        let mut conversation = Map::new();
        conversation.insert("id".to_string(), json!(private_identifier));
        conversation.insert("conversation_id".to_string(), json!(private_identifier));
        conversation.insert("title".to_string(), json!(error_fragment));
        conversation.insert("create_time".to_string(), json!(private_path));
        conversation.insert("update_time".to_string(), json!(email));
        conversation.insert("mapping".to_string(), Value::Object(mapping));
        conversation.insert("current_node".to_string(), json!(email));
        conversation.insert("is_archived".to_string(), json!(false));
        conversation.insert("archived".to_string(), json!(false));
        conversation.insert("is_starred".to_string(), json!(true));
        conversation.insert("starred".to_string(), json!(true));
        conversation.insert(unknown_key.clone(), json!(error_fragment));

        json!([Value::Object(conversation), error_fragment, 7, null])
    }

    #[test]
    fn emits_only_the_fixed_counter_schema_and_suppresses_canaries() {
        let directory = TempDir::new().expect("temp directory");
        let canaries = private_canaries();
        write_shard(&directory, 0, &canary_fixture(&canaries));

        let outcome = inspect_structure(directory.path());
        assert_eq!(outcome.completion, InspectionCompletion::Complete);
        assert_eq!(outcome.report.shard_count, 1);
        assert_eq!(outcome.report.parsed_shard_count, 1);
        assert_eq!(outcome.report.conversation_record_count, 4);
        assert_eq!(outcome.report.object_record_count, 1);
        assert_eq!(outcome.report.non_object_record_count, 3);
        assert_eq!(outcome.report.mapping_node_count, 2);
        assert_eq!(outcome.report.missing_reference_count, 1);
        assert_eq!(outcome.report.cycle_count, 1);
        assert!(outcome.report.unknown_key_count >= 4);
        assert!(outcome.report.source_unchanged);

        let serialized = serialize_validated_report(&outcome.report);
        assert!(is_fixed_report_json(&serialized));
        let output = String::from_utf8(serialized).expect("fixed report is utf8");
        for canary in canaries {
            assert!(!output.contains(&canary));
        }
        assert!(!output.contains(&directory.path().to_string_lossy().to_string()));
        assert!(!output.contains("synthetic-private-role"));
    }

    #[test]
    fn counts_known_top_level_field_presence_without_emitting_values() {
        let directory = TempDir::new().expect("temp directory");
        let canaries = private_canaries();
        write_shard(&directory, 0, &canary_fixture(&canaries));

        let report = inspect_structure(directory.path()).report;
        assert_eq!(
            report.known_field_presence,
            KnownFieldPresenceCounts {
                id: 1,
                conversation_id: 1,
                title: 1,
                create_time: 1,
                update_time: 1,
                mapping: 1,
                current_node: 1,
                is_archived: 1,
                archived: 1,
                is_starred: 1,
                starred: 1,
            }
        );
    }

    #[test]
    fn counts_missing_references_and_cycles_without_serializing_identifiers() {
        let directory = TempDir::new().expect("temp directory");
        let fixture = json!([
            {
                "mapping": {
                    "synthetic-a": {
                        "id": "synthetic-a",
                        "parent": "synthetic-b",
                        "children": ["synthetic-b", "synthetic-missing-child"]
                    },
                    "synthetic-b": {
                        "id": "synthetic-b",
                        "parent": "synthetic-a",
                        "children": ["synthetic-a"]
                    }
                },
                "current_node": "synthetic-missing-current"
            }
        ]);
        write_shard(&directory, 0, &fixture);

        let report = inspect_structure(directory.path()).report;
        assert_eq!(report.mapping_node_count, 2);
        assert_eq!(report.missing_reference_count, 2);
        assert_eq!(report.cycle_count, 1);
        let output =
            String::from_utf8(serialize_validated_report(&report)).expect("report utf8");
        assert!(!output.contains("synthetic-a"));
        assert!(!output.contains("synthetic-missing"));
    }

    #[test]
    fn malformed_shards_produce_partial_fixed_output() {
        let directory = TempDir::new().expect("temp directory");
        write_shard(&directory, 0, &json!([{"mapping": {}}]));
        fs::write(
            directory.path().join("conversations-001.json"),
            br#"[{"title":"canary@example.com"}"#,
        )
        .expect("write malformed synthetic shard");

        let outcome = inspect_structure(directory.path());
        assert_eq!(outcome.completion, InspectionCompletion::Partial);
        assert_eq!(outcome.report.shard_count, 2);
        assert_eq!(outcome.report.parsed_shard_count, 1);
        assert_eq!(outcome.report.malformed_shard_count, 1);
        assert!(outcome.report.source_unchanged);
        let output = String::from_utf8(serialize_validated_report(&outcome.report))
            .expect("report utf8");
        assert!(!output.contains("canary@example.com"));
    }

    #[test]
    fn oversized_records_are_counted_without_entering_the_report() {
        let directory = TempDir::new().expect("temp directory");
        write_shard(
            &directory,
            0,
            &json!([
                {"title": "x".repeat(256)},
                {"mapping": {}}
            ]),
        );

        let outcome = inspect_structure_with_limits(
            directory.path(),
            JsonStreamLimits {
                max_record_bytes: 64,
                max_nesting_depth: 32,
            },
        );
        assert_eq!(outcome.completion, InspectionCompletion::Partial);
        assert_eq!(outcome.report.parsed_shard_count, 1);
        assert_eq!(outcome.report.conversation_record_count, 2);
        assert_eq!(outcome.report.object_record_count, 1);
        assert_eq!(outcome.report.oversized_record_count, 1);
    }

    #[test]
    fn rejected_roots_return_only_an_empty_fixed_report() {
        let directory = TempDir::new().expect("temp directory");
        let rejected = directory.path().join("not-an-export");
        fs::create_dir(&rejected).expect("create rejected directory");

        let outcome = inspect_structure(&rejected);
        assert_eq!(outcome.completion, InspectionCompletion::Rejected);
        assert_eq!(outcome.report, StructureReport::default());
        assert!(is_fixed_report_json(&serialize_validated_report(
            &outcome.report
        )));
    }

    #[test]
    fn cli_stdout_is_fixed_json_for_success_partial_rejection_and_usage() {
        let valid = TempDir::new().expect("valid temp directory");
        write_shard(&valid, 0, &json!([]));

        let malformed = TempDir::new().expect("partial temp directory");
        fs::write(
            malformed.path().join("conversations-000.json"),
            br#"[{"private":"canary@example.com"}"#,
        )
        .expect("write malformed shard");

        let rejected = TempDir::new().expect("rejected temp directory");
        let cases = [
            (
                vec![OsString::from(PATH_STDIN_FLAG)],
                format!("{}\n", valid.path().to_string_lossy()).into_bytes(),
                EXIT_COMPLETE,
            ),
            (
                vec![OsString::from(PATH_STDIN_FLAG)],
                format!("{}\n", malformed.path().to_string_lossy()).into_bytes(),
                EXIT_PARTIAL,
            ),
            (
                vec![OsString::from(PATH_STDIN_FLAG)],
                format!("{}\n", rejected.path().to_string_lossy()).into_bytes(),
                EXIT_REJECTED,
            ),
            (
                vec![valid.path().as_os_str().to_os_string()],
                Vec::new(),
                EXIT_USAGE,
            ),
            (Vec::new(), Vec::new(), EXIT_USAGE),
            (
                vec![
                    OsString::from("synthetic-first"),
                    OsString::from("synthetic-second"),
                ],
                Vec::new(),
                EXIT_USAGE,
            ),
        ];

        for (arguments, input_bytes, expected_exit) in cases {
            let mut output = Vec::new();
            let mut input = io::Cursor::new(input_bytes);
            assert_eq!(run_cli(arguments, &mut input, &mut output), expected_exit);
            assert_eq!(output.last(), Some(&b'\n'));
            assert!(is_fixed_report_json(
                output.strip_suffix(b"\n").expect("newline suffix")
            ));
            let text = String::from_utf8(output).expect("output utf8");
            assert!(!text.contains("canary@example.com"));
            assert!(!text.contains("synthetic-first"));
            assert!(!text.contains("not-an-export"));
        }
    }

    #[test]
    fn cli_accepts_the_root_via_bounded_stdin_without_echoing_it() {
        let valid = TempDir::new().expect("valid temp directory");
        write_shard(&valid, 0, &canary_fixture(&private_canaries()));

        let private_input = format!("{}\n", valid.path().to_string_lossy()).into_bytes();
        let mut input = io::Cursor::new(private_input);
        let mut output = Vec::new();
        assert_eq!(
            run_cli([OsString::from(PATH_STDIN_FLAG)], &mut input, &mut output,),
            EXIT_COMPLETE
        );
        assert!(is_fixed_report_json(
            output.strip_suffix(b"\n").expect("newline suffix")
        ));
        let text = String::from_utf8(output).expect("output utf8");
        assert!(!text.contains(&valid.path().to_string_lossy().to_string()));
        for canary in private_canaries() {
            assert!(!text.contains(&canary));
        }
    }

    #[test]
    fn cli_rejects_ambiguous_or_oversized_stdin_as_fixed_usage_output() {
        let cases = [
            Vec::new(),
            b"synthetic-first\nsynthetic-second\n".to_vec(),
            vec![b'x'; (MAX_STDIN_PATH_BYTES + 1) as usize],
        ];

        for bytes in cases {
            let mut input = io::Cursor::new(bytes);
            let mut output = Vec::new();
            assert_eq!(
                run_cli([OsString::from(PATH_STDIN_FLAG)], &mut input, &mut output,),
                EXIT_USAGE
            );
            assert!(is_fixed_report_json(
                output.strip_suffix(b"\n").expect("newline suffix")
            ));
            let text = String::from_utf8(output).expect("output utf8");
            assert!(!text.contains("synthetic-first"));
            assert!(!text.contains("synthetic-second"));
        }
    }

    #[test]
    fn output_failures_use_a_fixed_exit_code_without_panicking() {
        struct RejectingWriter;

        impl Write for RejectingWriter {
            fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
                Err(io::Error::other("synthetic writer rejection"))
            }

            fn flush(&mut self) -> io::Result<()> {
                Ok(())
            }
        }

        let mut output = RejectingWriter;
        let mut input = io::empty();
        assert_eq!(
            run_cli(Vec::new(), &mut input, &mut output),
            EXIT_OUTPUT_ERROR
        );
    }

    #[test]
    fn schema_validator_rejects_strings_arrays_and_dynamic_keys() {
        assert!(is_fixed_report_json(EMPTY_REPORT_JSON));
        assert!(!is_fixed_report_json(br#"{"shardCount":"private-value"}"#));
        assert!(!is_fixed_report_json(br#"{"dynamic-private-key":1}"#));
        let mut report = serialize_validated_report(&StructureReport::default());
        report.extend_from_slice(br#"{"private":[]}"#);
        assert!(!is_fixed_report_json(&report));
    }
}
