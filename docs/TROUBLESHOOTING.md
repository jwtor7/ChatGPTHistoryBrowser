# Troubleshooting

> **Keep the export private.** Do not attach conversations, screenshots,
> indexes, logs, filenames, identifiers, or local paths to a public issue.
> Troubleshooting should use synthetic data and the fixed-field compatibility
> report only.

## `chat.html` freezes or will not open

Do not open `chat.html`. Large exports can produce an HTML file too large for a
normal browser to parse reliably, and ChatGPT History Browser does not use it.

1. Extract the downloaded archive.
2. Start the desktop application.
3. Choose the extracted directory containing `conversations.json` or numbered
   `conversations-<number>.json` shards.

The JSON shards are the supported conversation source.

## “The selected folder is not a supported export”

Confirm that:

- you selected the extracted directory, not the ZIP file;
- at least one recognized conversation JSON file is directly inside it;
- the shard is a regular file containing a top-level JSON array;
- the directory and shard are not symlinks or hard-link aliases; and
- the files have not moved or changed since the directory was selected.

Nested shards and differently named JSON files are intentionally ignored. See
[Supported Exports](SUPPORTED_EXPORTS.md#required-directory-layout).

## “A safe processing limit was reached”

The input exceeded a defensive shard, file, record, mapping, message, preview,
index-size, or free-space limit. Indexing stops if the SQLite derivative would
exceed 16 GiB, if less than 512 MiB would remain free, or if available space
cannot cover the four-times-source-size preflight estimate. Do not remove or
raise a limit merely to process a private export. Instead:

1. Reproduce the same structural shape with synthetic data.
2. Run the fixed-field structure inspector locally.
3. Report only reviewed numeric/Boolean counts and the synthetic fixture.

The current limits are documented in
[Supported Exports](SUPPORTED_EXPORTS.md#resource-limits).

## Indexing is slow

Large JSON shards and SQLite full-text indexing are disk- and CPU-intensive.
Keep the application open, ensure the platform cache volume has adequate free
space, and avoid changing the export while indexing.

The index is resumable at completed-shard boundaries. If you cancel, wait for
the interface to show the cancelled state before restarting. Cancellation does
not edit the export.

Near-limit exports may take substantial time even when structurally valid.
Never upload the export for performance investigation; generate a large
synthetic fixture instead:

```sh
npm run generate:synthetic -- \
  --output /tmp/chatgpt-history-browser-large-fixture --large
```

## “An indexing job is already running”

Wait for the active job to finish or press **Cancel indexing** and wait for its
state to settle. If the desktop process stopped unexpectedly, close every
remaining application window and start it again before retrying.

Do not manually delete cache files while the application is running.

## “The local index is unavailable”

Common causes are insufficient disk space, an unwritable platform cache
directory, another running application process holding the index lock, an
unexpected marker or link in the selected index directory, or a failure to
apply owner-only permissions. On the supported macOS target, cache directories
require mode `0700` and cache files require mode `0600`.

1. Close all application windows.
2. Check free space and normal user access to the platform cache directory.
3. Restart the application and select the export again.
4. If the error persists, remove only the application index as described below
   and rebuild it.

The source export should not need write permission.

## Remove a local index

The preferred method is **Delete local index** (also described as **Discard
local index**) in the application. It removes the plaintext SQLite database and
its WAL/SHM sidecars, then leaves an empty initialized index. The source export
is unchanged.

For manual cleanup:

1. Close the application completely.
2. Open the current user's macOS `Library/Caches` area.
3. Find the ChatGPT History Browser project directory, then its `indexes`
   directory.
4. Remove only the relevant opaque-key index directory—or the application's
   cache directory if you intentionally want to discard every local index.
5. Leave the extracted export directory untouched.

Exact macOS naming is supplied by the Rust `directories` crate and can vary.
An index directory contains an `.index-owner` marker and can contain
`index.sqlite3`, `index.sqlite3-wal`, `index.sqlite3-shm`,
`index.sqlite3-journal`, and `.index-lock`.

These files contain plaintext projected conversation content. Filesystem
deletion does not guarantee physical erasure from SSD storage, snapshots,
backups, or endpoint-security systems.

Never use a recursive deletion command against a home directory, an
unvalidated environment variable, or the export.

## An attachment is missing

The conversation can refer to an attachment that is absent from the downloaded
export, has a variant reference, or does not match the recognized root-level
`file-….dat` / `file_….dat` layout. Missing attachments do not prevent
conversation browsing.

Do not rename, move, or rewrite files in the source export to force a match.
Create a synthetic fixture when testing resolver changes.

## “This attachment cannot be previewed safely”

The type may be outside the preview allowlist, the signature may be unknown,
the content may not be valid UTF-8 text, or the file may exceed a preview
limit. The `.dat` extension alone is not enough to determine type. Raster-image
preview is limited to bounded PNG and JPEG files; GIF and WebP are intentionally
not rendered inline.

SVG and HTML are deliberately not rendered inline. Audio and video playback
also depends on the local WebView and installed codecs. PDF rendering may vary
with available memory and graphics support.

Preview bytes are not loaded automatically. Select the attachment's
**Preview image**, **Preview audio**, **Preview video**, **Preview PDF**, or
**Preview text** action to load it locally.

See [Supported Exports](SUPPORTED_EXPORTS.md#attachment-layout-and-previews) for
the current allowlist and size bounds.

## “Restart the application to continue”

The desktop window did not receive a valid per-launch session capability.
Close the window and start ChatGPT History Browser from the desktop application
again. A manually opened loopback URL, restored browser tab, or web-only Vite
window cannot substitute for the Tauri-created session.

## The desktop window does not start

For a source checkout:

1. Confirm Node.js 20.19 or newer, npm, Rust 1.97.1 or newer, `rustfmt`, and
   Clippy are installed.
2. Install the current Tauri 2 macOS prerequisites.
3. Run `npm ci`.
4. Run `npm run typecheck`, `npm test`, and `npm run build` to isolate frontend
   and Rust failures.
5. Start the full app with `npm run dev`.

`npm run web:dev` starts the frontend only. The functional API exists only
while the Rust/Tauri process is running.

Verify Xcode Command Line Tools and the system WebKit runtime. Windows and
Linux are not supported MVP targets.

## Local firewall or loopback warning

The application binds an ephemeral port on `127.0.0.1`, not on a LAN address.
A firewall or endpoint-security product may still prompt on first launch.
Confirm that the process and build are the expected local application before
allowing loopback access. Do not expose, proxy, or forward the local port.

The application should make no normal-use external request. Package
installation, dependency audit, and source updates are separate development
activities that may access registries.

## Report a problem safely

**Never include the real export or anything derived from its values.**

A safe report contains:

- the exact application commit;
- operating system name, version, and CPU architecture;
- the phase and public error code/message shown by the app;
- reviewed, structure-only numeric/Boolean counts from the inspector;
- a minimal synthetic fixture reproducing the same structure; and
- expected versus observed behavior using only fictional values.

Follow
[Safe compatibility reports](SUPPORTED_EXPORTS.md#safe-compatibility-reports).
Review the inspector source and its entire output before sharing. If it contains
any path, string value, filename, title, identifier, or unexpected key, do not
share it.

Do not post:

- screenshots or recordings made with the export open;
- browser developer-tools output, request dumps, or copied DOM;
- SQLite files, WAL/SHM files, crash reports, or profiler captures;
- attachment names, hashes, sizes tied to a unique real file, or message
  excerpts; or
- absolute paths, usernames, hostnames, account details, or timestamps that can
  identify a person.

Automated scanners and source review are useful but cannot guarantee perfect
PII detection. If the behavior may be a vulnerability, use the private process
in [SECURITY.md](../SECURITY.md) instead of opening a public issue.
