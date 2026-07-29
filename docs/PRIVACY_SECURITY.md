# Privacy and Security Model

## Non-negotiable privacy rule

Real ChatGPT export content must never enter this public repository, its Git
object database, CI output, release bundles, screenshots, fixtures, examples,
logs, or bug reports. This includes derived values such as indexes and
thumbnails as well as titles, messages, identifiers, filenames, metadata, URLs,
timestamps, account information, and attachments.

All committed examples and tests must be independently generated synthetic
data. If there is uncertainty about a value, file, or artifact, the release
fails closed.

The repository scanners reduce risk; they cannot prove that all personally
identifiable information has been detected. A complete manual review remains a
release requirement.

The supported MVP target is macOS. Security behavior described as implemented
and verified in this document applies to macOS unless stated otherwise.
Windows and Linux remain future portability work and are not supported MVP
targets.

## Data flow and trust boundaries

The application has four sensitive boundaries:

1. **Export to backend.** JSON, metadata, filenames, and attachment bytes are
   attacker-controlled input even when they came from an official export.
2. **Backend to index.** The search index is a private derivative of the export
   and requires the same confidentiality assumptions.
3. **Backend to browser.** Rendered Markdown and previewed attachments must
   remain inert data and must not gain script or network capabilities.
4. **Worktree to public Git.** Source control and CI are public disclosure
   boundaries. Ignore rules alone do not protect them.

Protected assets include the source export, its derived index, local filesystem
contents outside the selected root, the local server session capability,
browser content, developer worktrees, Git history, CI logs, and release
packages.

## Threat model

| Threat                                                                 | Impact                                           | Required controls                                                                               |
| ---------------------------------------------------------------------- | ------------------------------------------------ | ----------------------------------------------------------------------------------------------- |
| Export data is copied or committed accidentally                        | Permanent public disclosure                      | Ignore rules, worktree/index/history scanners, secret scanning, manual review                   |
| Export metadata traverses outside the selected root                    | Arbitrary local file disclosure                  | Canonical root, opaque handles, component checks, no-follow open, fail-closed containment       |
| A symlink, junction, hard link, or filesystem race escapes containment | Local file disclosure                            | Reject links and non-regular files, revalidate before open, stream from the validated handle    |
| Malicious JSON exhausts memory, disk, or CPU                           | Denial of service or corrupt index               | Streaming parse, depth/size/count limits, cancellation, transactional rebuild                   |
| Conversation content executes in the browser                           | Stored XSS and local data exposure               | Raw HTML disabled, allowlist sanitization, strict CSP, inert links, no dangerous DOM sinks      |
| A hostile attachment executes active content                           | Script execution, network access, decoder attack | Signature detection, safe type allowlist, sandboxing, download-only active formats, size limits |
| A website targets the local server                                     | Cross-site data access or state changes          | Exact loopback bind, random port, per-process capability, Host/Origin checks, no CORS           |
| Markdown, PDF, or media triggers an external request                   | Privacy leak through URL or metadata             | Default-deny network policy, bundled assets, blocked remote resources, egress tests             |
| Logs or caches disclose conversation content                           | Secondary private-data exposure                  | Per-user storage, restrictive permissions, redacted errors, persistent logs disabled by default |
| A dependency or CI action is compromised                               | Build or source disclosure                       | Lockfile, pinned actions, read-only CI permissions, dependency and build-output audits          |
| A user uploads an export to an issue                                   | Public disclosure                                | Prominent warnings and synthetic-only reproduction guidance                                     |

A malicious process already running as the same operating-system user may be
able to read the original export directly. The application does not claim to
protect against a fully compromised user account. It must still defend against
malicious archive content, websites, LAN clients, accidental disclosure, and
filesystem paths supplied by the export.

## Read-only source and path containment

The selected export is a read-only input. The application must not normalize,
rename, move, delete, or write files beneath `<export-root>`.

The backend should create a `SafeExportRoot` capability after verifying that the
selected root is a real directory rather than a symlink or reparse point. It
canonicalizes that root once and performs discovery itself. Browser APIs receive
random opaque handles, not filenames or paths.

Before opening a discovered file, the backend must:

- reject absolute paths, parent traversal, separators supplied through
  metadata, NULs, control characters, drive paths, network paths, and device
  names;
- inspect every path component and reject symlinks and junctions;
- compare the canonical candidate with the canonical root using a
  component-aware relative-path check, never a string prefix;
- reject hard links and anything other than a regular file;
- open read-only with no-follow semantics on macOS;
- verify the opened handle and stream from that handle instead of reopening the
  pathname;
- return a fixed error code without returning or logging the source path.

macOS tests must include prefix collisions, encoded traversal, case
differences, Unicode separator lookalikes, link escapes, symbolic links, hard
links, non-regular files, and a link-swap race.

## JSON and index safety

Conversation shards are parsed incrementally. Untrusted objects are projected
into typed application records; they are never merged into configuration or
used as object prototypes. Dynamic mapping identifiers belong in maps or
null-prototype objects.

Limits must be checked before allocation and cover JSON nesting, string and
content-part sizes, nodes per conversation, attachment counts, graph traversal,
preview bytes, pending work, and database growth. Graph traversal must detect
cycles, missing nodes, and invalid parent/child relationships.

Database operations use parameterized statements, including full-text search.
Indexing writes to a transaction or partial database and promotes it only after
successful completion. Cancellation or a malformed shard must not corrupt the
last usable index. Parser and database errors shown to users contain fixed codes
and shard ordinals, never snippets or raw exception messages.

## Browser, Markdown, and attachment isolation

Raw HTML in Markdown is disabled. Text and code are rendered as text nodes.
Dangerous sinks and execution primitives such as direct HTML assignment,
dynamic code evaluation, shell interpolation, and shell-enabled child
processes are prohibited. If an HTML-producing library is unavoidable, a narrow
allowlist sanitizer must be the final transformation; Trusted Types may be
added as defense in depth.

Markdown images and other remote resources are inert. External links must not
be fetched automatically. Any user-initiated external navigation requires an
explicit confirmation and must not disclose source data through a referrer.

Attachment type is determined from a bounded signature read. Export metadata
and the `.dat` extension are advisory only. Raster-image preview is limited to
PNG and JPEG files whose encoded dimensions fit the pixel budget. Recognized
audio and video, bounded plain text, and bounded PDF files can also be
previewed. GIF, WebP, SVG, HTML, XML, office files, archives, and other active or
unsupported formats are save-only. JSON, CSV, source code, and filenames are
always rendered as text. PDF previewing uses a bundled, sandboxed renderer with
document JavaScript, actions, and external links disabled.

Attachment cards are rendered in batches of 24. Image, audio, video, PDF, and
text preview bytes are fetched only after an explicit preview action. Media
responses use a fixed type allowlist, `nosniff`, a sanitized
`Content-Disposition`, a 64 MiB per-file ceiling, four concurrent preview
slots, and a 64 MiB aggregate in-flight byte budget. Rust streams media from the
already validated file handle instead of buffering the complete file. No
response includes an absolute filesystem path.

## Local server and network model

The server binds exactly to `127.0.0.1` and asks the operating system for an
available port. It must never bind to a wildcard address or silently substitute
another hostname.

Each process creates a high-entropy session capability. The bootstrap secret is
placed in the initial URL fragment, removed immediately from the visible URL,
and retained only in frontend module memory. Every private API request carries
bearer authorization. Cookies are not used for authorization because they are
not scoped by port. An explicit preview action performs an authenticated API
fetch, then gives the media element a process-local blob URL. The server
validates the exact Host and Origin, rejects foreign state-changing requests,
and does not enable CORS.

Normal use makes no outbound requests. Application code, fonts, scripts,
styles, renderers, and icons are bundled locally. Browser end-to-end tests fail
any request whose destination is not the active loopback origin.

The UI uses a default-deny Content Security Policy. `script-src` remains
self-only, with no `unsafe-inline` or dynamic evaluation. `style-src-elem` is
self-only; `style-src-attr` permits `unsafe-inline` solely for the dynamic
progress widths and virtualization geometry calculated by React and TanStack
Virtual. Connections and fonts are self-only, local blob URLs are allowed only
where previews require them, and objects, framing parents, base URL changes,
and form submission are blocked. Responses also set `nosniff`, a no-referrer
policy, same-origin resource and opener policies, and a restrictive permissions
policy.

## Cache and log model

The index contains private conversation text and must be treated as sensitively
as the export. It is stored under the operating system's per-user application
cache location, never in the repository, source export, current directory, or a
shared temporary directory. Each index-directory name is an opaque digest and
does not contain a literal source path, title, filename, or conversation
identifier.

On macOS, cache directories are set to mode `0700` and cache files to `0600`.
Cache roots, index directories, marker files, databases, and sidecars are
rejected if their type or containment does not match the expected
application-owned layout. The application does not retain a recent-export path
list.

The SQLite database, WAL, shared-memory, and rollback-journal files have a
combined 16 GiB ceiling. Before indexing a shard, the application reserves a
conservative four-times-source-size estimate and requires at least 512 MiB of
free space to remain. It checks the actual derivative size and free-space
reserve during indexing and fails closed with a fixed resource-limit error.

Persistent application logging is disabled by default. If diagnostic logging
is explicitly enabled, permitted fields are limited to application-generated
event codes, aggregate counts, durations, and application times. Message text,
titles, metadata values, attachment data or names, source timestamps,
identifiers, URLs, search terms, and filesystem paths are never logged. Raw
parser, database, and operating-system errors are mapped to fixed safe codes
before reaching a log or the browser.

Discarding an index validates an application-owned marker, closes active work,
removes only the allowlisted database, journal, WAL, and SHM files, then creates
a fresh empty database. It does not recursively delete the cache directory. The
source export is never touched. Filesystem deletion cannot guarantee physical
erasure from SSD wear-leveling, snapshots, backups, or cloud-synchronized
storage; this limitation must be disclosed to users.

## Structure-only compatibility diagnostics

Compatibility checks against a private export run only after privacy controls
and synthetic canary tests pass. The root is supplied interactively or through
private process IPC and is never placed in command arguments, configuration,
logs, or output.

The current command-line inspector accepts a root only with `--path-stdin`. It
reads at most 32 KiB, rejects embedded line breaks and NUL bytes, and rejects
positional path arguments with fixed usage output.

The diagnostic worker:

- recognizes only hard-coded, documented schema field names;
- reports unknown keys as a count because object keys can contain private data;
- emits only fixed output keys, numbers, booleans, and fixed status values;
- reports type histograms, field-presence counts, size buckets, node and edge
  counts, missing relationships, cycle counts, and fixed parse-error codes;
- never emits values, dynamic keys, IDs, hashes, roles, timestamps, filenames,
  URLs, snippets, exception messages, or paths;
- runs with cache and persistent logging disabled.

Before writing, the inspector serializes the report and validates it against
its exact fixed schema. Usage, rejection, partial-parse, and output failures use
fixed exit codes, and the tool does not write diagnostics to standard error. A
caller should still capture both output streams and reject anything outside the
documented schema. Real diagnostic output is reviewed locally and is never
committed or uploaded.

Synthetic leak tests place distinct canaries in every string, dynamic key, node
identifier, filename, malformed fragment, and path. Neither output stream may
contain a canary, and the complete output must conform to the allowlisted
schema.

## Repository privacy gates

`scripts/privacy/audit-repo.mjs` scans repository-relative paths and contents
without printing matched bytes. Its scopes are:

- `worktree`: regular files present in the repository, including ignored files
  except dependency stores;
- `staged`: the exact blobs selected for the next commit;
- `tracked`: every blob and mode currently in the Git index;
- `build`: common production output directories, including ignored output;
- `all`: all of the above.

It rejects export-shaped artifacts, archives, databases, logs, environment
files, export/cache directories, absolute user-home paths, non-reserved email
addresses, phone- and government-ID-like strings, high-confidence secrets,
symlinks, submodules, hard links, non-regular files, source maps in production
output, and files over the conservative size limit. Findings contain only a
rule ID and a 16-character hexadecimal digest of the repository-relative path.
They never contain a path, matched bytes, or a content excerpt. The digest is
an opaque correlation label for a single audit result, not proof that the
underlying file is safe.

`scripts/privacy/audit-git-objects.mjs` enumerates every loose and packed object
in the local object database, including unreachable objects. It scans blobs,
commit and tag metadata, tree entry names, refs, and reflog subjects. Findings
contain only a rule ID and object identifier.

Gitleaks runs independently with redaction and complete history:

```sh
gitleaks git . --redact --log-opts='--all'
```

The custom scanners and Gitleaks are complementary. Neither replaces manual
review, image inspection, dependency review, or incident response.

## Manual commit and release review

Run the content scanners before displaying a diff so an accidental private
value is not echoed to a terminal:

```sh
node scripts/privacy/audit-repo.mjs --scope all
node scripts/privacy/audit-git-objects.mjs
gitleaks git . --redact --log-opts='--all'
```

After all scanners pass:

1. Inspect `git status --short --untracked-files=all`.
2. Inspect the complete staged path list and staged diff.
3. Confirm there are no Git symlinks or submodules.
4. Manually review every fixture, documentation example, screenshot, binary,
   generated asset, and dependency change.
5. Run formatting, linting, type checking, unit, integration, security,
   end-to-end, production-build, and performance tests.
6. Scan the production build and inspect the package file inventory.
7. Run the structure-only compatibility check locally, then confirm the
   repository remains unchanged.
8. Repeat the repository, full-object, and Gitleaks scans at the exact committed
   revision intended for release.
9. Record the exact files, commands, pass/fail results, known limitations, and
   manual-review acknowledgement.
10. Obtain explicit approval before the first public push.

Any finding, unexpected file, raw diagnostic value, open high-impact security
issue, or uncertainty about provenance blocks the release. Scanner output and
CI success are evidence of checks performed, not proof that the repository is
free of every possible private value.

## Reporting compatibility problems safely

Never upload an export or a subset of one. Do not paste conversation content or
attachment metadata into an issue. Reproduce the structure with the synthetic
generator, or provide only a manually reviewed structure-only summary. When in
doubt, omit the data and describe the behavior.
