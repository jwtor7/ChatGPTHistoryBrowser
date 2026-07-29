# Supported Exports

> **Do not attach an export to an issue or pull request.** Never share
> conversation text, titles, identifiers, filenames, screenshots, local paths,
> generated indexes, or raw parser output. Use only the fixed-field,
> structure-only compatibility report described below, and review it before
> sharing.

ChatGPT History Browser currently supports the JSON conversation data and
selected opaque attachment files from a standard extracted ChatGPT export. The
application intentionally recognizes a narrow root-level layout.

The supported MVP platform is macOS. Windows and Linux compatibility has not
been validated and remains future portability work.

## Required directory layout

Select a directory containing at least one of:

- `conversations.json`
- `conversations-<number>.json`, where `<number>` contains ASCII digits only

Examples of recognized shard names are `conversations-0.json` and
`conversations-001.json`. Shards are read in numeric order, with the unnumbered
file first when it exists.

Conversation shards must be root-level regular files containing a JSON array.
The selected directory itself and every file opened by the application must not
be a symbolic link, reparse-point escape, or multiply linked file. Nested
conversation files are ignored.

The export is treated as read-only. The application captures source-file
metadata during selection and rejects unsafe changes or path substitutions
while files are being used.

## Conversation structure

Current projection supports the common export fields:

- conversation identity and title;
- create and update timestamps;
- `mapping` nodes, parent/child relationships, and `current_node`;
- message authors and roles;
- text represented through common `content.parts`, `content.text`, or message
  text forms;
- archive and star flags; and
- common attachment references and display metadata.

Unknown object fields are not copied wholesale into the index. The parser
projects the fields it understands and applies strict resource bounds.
Malformed records or unsupported structures may be skipped or rejected rather
than interpreted heuristically.

Branch reconstruction follows the active path when one is available while
preserving enough projected relationships for the browser's branch behavior.
Do not assume that every historical or future ChatGPT export variation is
already supported.

## Attachment layout and previews

Root-level regular files are attachment candidates only when their names:

- begin with `file-` or `file_` (case-insensitive); and
- end with `.dat` (case-insensitive).

The `.dat` suffix does not identify the underlying media type. The application
reads at most a small signature prefix and uses a conservative allowlist.
Current inline preview categories are:

- PNG and JPEG images whose encoded dimensions fit the image budget;
- recognized audio formats;
- MP4, WebM, QuickTime, and MPEG video;
- PDF; and
- UTF-8 text that passes conservative control-character checks.

GIF, WebP, SVG, HTML, executable content, unknown binary formats, and
unrecognized signatures are not rendered inline. Browser and operating-system
codec support can still affect whether recognized audio or video plays.

Opening a conversation does not fetch attachment bodies. Every preview starts
only after the user selects its **Preview** action, and the interface reveals
attachment cards in batches of 24. This bounds initial rendering for the
supported maximum of 2,000 attachment references per conversation.

Preview and transfer bounds are:

- up to 64 KiB read for signature detection;
- up to 2 MiB for inline text;
- up to 20 MiB for a PDF first-page canvas preview;
- up to 64 MiB for inline image, audio, and video;
- up to 8,192 pixels in either raster-image dimension and 16,777,216 decoded
  raster-image pixels; and
- up to 16 GiB for a bounded local download response.

A missing or unsupported attachment does not invalidate otherwise readable
conversation data.

## Resource limits

The current defensive limits include:

- at most 10,000 conversation shards;
- at most 1,000,000 attachment candidates;
- at most 64 GiB combined conversation JSON;
- at most 50,000 mapping nodes per conversation;
- at most 10,000 children per mapping node;
- at most 2,000 attachments per conversation;
- at most 4 MiB projected text for one message; and
- bounded identifiers, titles, filenames, MIME values, JSON nesting, and record
  sizes.

The disposable index has separate disk safeguards:

- at most 16 GiB across the SQLite database and its WAL, shared-memory, and
  rollback-journal sidecars;
- at least 512 MiB of free space must remain; and
- preflight available space must cover a conservative estimate of four times
  the source shard size.

These are safety limits, not target performance guarantees. A structurally
valid export near a limit can still require substantial time and disk space.

## Not supported

The current application does not directly support:

- ZIP or other archive files—extract them first;
- `chat.html` as the conversation source;
- nested conversation shards or nested attachment directories;
- arbitrarily named `.dat` files;
- symbolic links, hard-link aliases, or path traversal;
- encrypted, compressed, or executable attachment payloads; or
- export formats that do not provide a root-level JSON array of conversation
  records.

`chat.html` can be extremely large and may freeze a normal browser. You do not
need to open it. Select the extracted directory that contains the JSON shards.

## Safe compatibility reports

The repository includes a structure inspector whose output schema contains
only fixed field names with numeric counts and one Boolean source-integrity
result. It does not emit source paths, values, titles, IDs, filenames, or
conversation text.

Build and run it locally from a reviewed macOS checkout. This interactive form
keeps the selected path out of the literal command:

```sh
read -r EXPORT_DIRECTORY
printf '%s\n' "$EXPORT_DIRECTORY" |
  cargo run --quiet --manifest-path src-tauri/Cargo.toml \
    --bin inspect_structure -- --path-stdin
unset EXPORT_DIRECTORY
```

`--path-stdin` is the only accepted input mode. The inspector reads a single
path of at most 32 KiB from standard input and rejects positional path
arguments, extra arguments, embedded line breaks, NUL bytes, and oversized
input with fixed usage output.

The report can contain only:

- shard, parsed-shard, and malformed-shard counts;
- conversation, object, and non-object record counts;
- counts indicating the presence of known top-level fields;
- mapping-node, missing-reference, cycle, oversized-record, and unknown-key
  counts; and
- `sourceUnchanged`.

Before sharing even this report:

1. Review the inspector source and the complete JSON output.
2. Confirm that every key is in the fixed list above.
3. Confirm that every value is a number or Boolean.
4. Share the smallest useful subset of counts and synthetic reproduction steps.

If any path, filename, string value, conversation content, or unexpected key
appears, do not share the output. Open no public issue with the private data.
Source review and automated PII scanning reduce risk but cannot provide a
perfect guarantee for all future code or data shapes.

When reporting a compatibility problem, include the application commit,
operating system and architecture, safe counts, whether indexing completed,
and a fully synthetic fixture that reproduces the same structure. See
[Troubleshooting](TROUBLESHOOTING.md#report-a-problem-safely).
