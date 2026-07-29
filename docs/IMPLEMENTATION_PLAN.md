# Implementation Plan

## Objective

Deliver a privacy-first, open-source desktop application that can browse, search, and recover content from an extracted ChatGPT export without loading the generated HTML archive.

The first release is complete only when it provides a working, packaged vertical slice; supports the required synthetic scale; passes privacy, security, correctness, and production-build tests; and is ready for a manual pre-push audit. Scaffolding alone is not a deliverable.

Checked work items mean the corresponding source or verification evidence
exists in the working tree. Unchecked items remain future hardening or public
distribution work. Windows and Linux are future portability work, not MVP
release gates.

Architecture is defined by [ADR-0001](adr/0001-local-desktop-architecture.md):

- Tauri 2 desktop shell;
- Rust modular monolith;
- React and TypeScript frontend;
- bundled SPA and API served by Axum on `127.0.0.1:0`;
- SQLite with FTS5 in the per-user macOS cache directory; and
- a read-only capability boundary around the selected export.

## Delivery principles

1. **Privacy controls precede compatibility inspection.** No private export may be examined until ignore rules, privacy scanning, safe diagnostics, and independently generated synthetic fixtures exist.
2. **The source export is immutable.** No feature may require a write, lock, rename, thumbnail, sidecar, or index inside the export directory.
3. **Private values do not cross unnecessary boundaries.** Rust owns paths and raw files after selection. A manually entered path is transient request input; responses use opaque IDs and the minimum bounded content required for display.
4. **Derived data is disposable.** Every index and cache can be rebuilt and deleted without affecting the source.
5. **Partial work remains valid.** Indexing replaces each shard in one transaction so cancellation or failure before commit preserves the last-known-good shard.
6. **Synthetic data is the default development environment.** Tests, fixtures, screenshots, examples, benchmarks, and bug reports use generated fictional content only.
7. **Production behavior is tested.** Development-mode success is insufficient.
8. **No push occurs without explicit approval.** Implementation, tests, performance checks, and complete privacy audits must finish first.

## Proposed repository layout

```text
.
├── .github/
│   └── workflows/
├── docs/
│   ├── adr/
│   ├── privacy-security-model.md
│   ├── supported-exports.md
│   └── troubleshooting.md
├── scripts/
│   ├── generate-synthetic-export.*
│   └── privacy-gate.*
├── specs/
│   └── chatgpt-export-browser.spec.md
├── src/
│   ├── api/
│   ├── components/
│   ├── features/
│   ├── routes/
│   └── styles/
├── src-tauri/
│   ├── capabilities/
│   ├── migrations/
│   └── src/
│       ├── app/
│       ├── attachment/
│       ├── export/
│       ├── index/
│       ├── ingest/
│       ├── model/
│       ├── privacy/
│       ├── query/
│       ├── server/
│       └── session/
└── tests/
    ├── e2e/
    ├── fixtures/
    ├── integration/
    ├── performance/
    └── security/
```

The exact filenames may change during scaffolding. The module boundaries and privacy responsibilities must not.

## Dependency baseline

Pin exact versions in lockfiles after validating the current stable releases.

### Rust

- `tauri`
- `tokio`
- `axum`
- `tower-http`
- `rusqlite` with bundled SQLite
- `serde` and `serde_json`
- `cap-std`
- `infer`
- `blake3`
- `directories`
- `tokio-util`
- `thiserror`
- `subtle`

Do not add a shell plugin, auto-updater, telemetry SDK, crash uploader, remote HTTP client, or frontend filesystem/SQL plugin.

### Frontend

- React
- TypeScript in strict mode
- Vite
- TanStack Query
- TanStack Virtual
- `react-markdown`
- `remark-gfm`
- `rehype-sanitize`
- locally bundled `pdfjs-dist`
- a renderer that converts code tokens to React text nodes

Do not add remote fonts, CDN assets, raw-HTML Markdown plugins, runtime analytics, or third-party embeds.

## Phase 0: Repository and privacy foundation

### Work

- [x] Ignore operating-system metadata, archives, export artifacts, `.dat` files, databases, indexes, caches, logs, environment files, screenshots, and temporary test output.
- [x] Remove accidental operating-system metadata from the working tree.
- [x] Add a permissive open-source license.
- [x] Add `SECURITY.md` with a private compatibility-reporting process.
- [x] Add `CONTRIBUTING.md` with synthetic-data-only rules.
- [x] Add a local and CI privacy gate.
- [ ] Add established secret scanning with a pinned configuration.
- [x] Add repository-specific checks for forbidden export artifacts, absolute home paths, private-path fragments, and suspicious personal-data patterns.
- [x] Document the required manual diff, untracked-file, and history review.
- [x] Create a synthetic-export generator that is independent of any private export.
- [x] Create a privacy-safe diagnostic contract that allows field names, value types, counts, and structural relationships but never values.

### Exit criteria

- The privacy gate fails on intentionally planted prohibited synthetic cases.
- The gate passes on the clean repository and synthetic fixtures.
- Generated fixtures contain only obviously fictional labels and reserved domains.
- No private-export inspection has occurred.

## Phase 1: Desktop and local-transport foundation

### Work

- [x] Scaffold Tauri 2, React, TypeScript, and Vite.
- [x] Start Axum from a pre-bound `127.0.0.1:0` listener.
- [x] Verify the effective socket address before opening the WebView.
- [x] Serve the production SPA and `/api` from the same origin.
- [x] Generate a per-launch bootstrap secret with at least 256 bits of entropy.
- [x] Transfer the secret in the initial URL fragment, remove it immediately, and keep it out of persistent Web storage.
- [x] Require bearer authorization on every private API route.
- [x] Reject wrong `Host` and `Origin` values, unsupported route methods, invalid content on JSON routes, and oversized request bodies.
- [x] Configure CSP and defensive response headers.
- [ ] Block navigation, new windows, permission prompts, and external requests.
- [ ] Add graceful shutdown for the listener, index task, and database workers.
- [x] Add an export-selection boundary that supports a native picker or an explicitly entered path and returns an opaque archive handle, never a path.
- [x] Resolve the per-user cache directory in Rust.

### Tests

- [x] Use tested 256-bit token generation and fragment parsing with constant-time bearer comparison.
- [x] Integration-test unauthenticated, invalid-token, wrong-origin, and wrong-host failures.
- [x] Verify that the bound address is exactly IPv4 loopback and the port differs across launches.
- [ ] Verify that production assets contain no remote URLs.
- [x] Add a synthetic browser regression that fails on any non-loopback request.
- [x] Add end-to-end smoke coverage for the production web build, not only the Vite development server.

### Exit criteria

The packaged application opens its bundled UI, can select a synthetic directory without echoing or retaining its path in renderer state, rejects unauthorized API calls, and makes no external network request.

## Phase 2: Synthetic schema and streaming ingestion

### Synthetic generator coverage

- [x] Multiple conversation shards.
- [x] Mapping trees with `current_node`.
- [x] Alternate branches.
- [x] Deleted and missing nodes.
- [x] User, assistant, system, tool, and unknown roles.
- [x] Markdown, fenced code, and malicious HTML/script payloads.
- [x] Archived and starred records.
- [ ] Missing and empty titles.
- [ ] Image, audio, video, PDF, text, JSON, CSV, code, and unsupported attachments.
- [x] Both supported `.dat` naming styles.
- [x] Missing attachments.
- [x] Misleading extensions and MIME values.
- [ ] Path traversal and symlink attempts.
- [x] Malformed JSON.
- [ ] Deep graphs, cycles, and large conversations.
- [ ] Interrupted and resumed indexing.
- [x] At least 10,000 lightweight conversations.
- [x] A deterministic 500 MiB benchmark profile.

Every generated artifact must carry a synthetic marker in its manifest. No generated content may be derived from a private export.

### Ingestion work

- [x] Validate that a selected directory contains at least one supported shard.
- [x] Discover shards without assuming a fixed count.
- [x] Open every source through the read-only root capability.
- [x] Build a streaming top-level-array deserializer.
- [x] Implement schema-tolerant typed adapters for known fields and safe unknown-field skipping.
- [ ] Bound recursion, queued work, diagnostic size, and any per-preview buffer.
- [x] Emit progress as byte, record, and aggregate-diagnostic counts only.
- [x] Implement cooperative cancellation.
- [x] Produce sanitized diagnostics for malformed shards and records.
- [x] Hash each parsed shard while reading it.

### Exit criteria

A multi-shard synthetic export can be indexed without reading an entire shard or archive into memory. Malformed records produce bounded diagnostics and do not reveal their content.

## Phase 3: Database, resumability, and search

### Schema and migrations

- [ ] Add versioned migrations.
- [x] Store shard fingerprints.
- [x] Normalize conversations, nodes, parent relationships, messages, and attachments.
- [ ] Add indexes for dates, roles, archive state, star state, and attachment presence.
- [x] Add an FTS5 table for titles and message text.
- [x] Verify FTS5 availability while initializing the index.
- [x] Use parameterized SQL and a constrained search-query parser.

### Incremental behavior

- [ ] Stage changed shards under a new generation.
- [ ] Commit bounded batches.
- [x] Keep the previous committed shard queryable until its transactional replacement finishes.
- [x] Atomically commit a completed shard replacement.
- [ ] Clean abandoned staging generations after a crash or cancellation.
- [x] Skip unchanged shards using recorded source metadata.
- [ ] Support a complete rebuild.
- [ ] Support complete deletion of the database, WAL, shared-memory, and staging files.

### Tests

- [ ] Migration tests from every supported schema version.
- [x] Verify that cancellation preserves the previous committed shard.
- [ ] FTS synchronization and rebuild tests.
- [ ] Search syntax fuzzing.
- [x] Filter-combination tests.
- [ ] Concurrent browsing-while-indexing tests.

### Exit criteria

The UI can query completed shards while another shard is indexing. Cancellation or a forced process termination never promotes partial shard data.

## Phase 4: Conversation browser vertical slice

### Work

- [x] Add onboarding and privacy disclosure.
- [x] Show validation and indexing status.
- [x] Add a virtualized, paginated conversation list.
- [x] Browse by title and date.
- [x] Search titles and message text.
- [x] Filter by date, role, archived state, starred state, and attachment presence.
- [x] Reconstruct the active path from `current_node`.
- [x] Detect cycles, missing parents, and invalid graphs.
- [ ] Preserve source child-edge order.
- [x] Expose alternate responses from stored parent relationships.
- [x] Render normalized roles distinctly.
- [x] Render Markdown through a strict sanitizer.
- [x] Render code without raw HTML.
- [x] Show clear empty, partial, malformed, and unsupported states.
- [x] Add keyboard navigation, focus states, semantic landmarks, and screen-reader labels.

### Vertical-slice fixture

Use a small generated export containing:

- two shards;
- one branched conversation;
- one missing node;
- one malicious Markdown payload;
- one safe image attachment;
- one safe text attachment; and
- one missing attachment.

### Exit criteria

A user can select the generated export, watch indexing, search, open a conversation, switch a branch, read safely rendered content, preview the two supported attachments, understand the missing attachment, and delete the index.

## Phase 5: Attachment gateway and previews

### Work

- [x] Extract attachment identifiers and display metadata during indexing.
- [x] Match identifiers to local `.dat` candidates without renaming source files.
- [x] Resolve every request through the root capability.
- [x] Detect file type from a bounded signature prefix.
- [x] Apply an explicit inline-preview allowlist.
- [x] Add dimension- and pixel-bounded image preview for PNG and JPEG.
- [x] Require an explicit preview action and render attachment cards in batches of 24.
- [ ] Add HTTP Range support for browser-supported audio and video.
- [x] Bundle PDF.js and render a bounded first page to canvas with automatic external access disabled.
- [x] Add bounded escaped previews for UTF-8 text.
- [x] Treat SVG, HTML, scripts, unknown types, and mismatches as save-only.
- [x] Fetch preview content through the bearer-authenticated API and use process-local blob URLs.
- [x] Add an explicit user-approved save-copy operation.
- [ ] Add an explicit external-open operation.
- [x] Never invoke a shell or construct a command string.

### Tests

- [x] Magic-signature and misleading-metadata tests.
- [ ] Range-request tests, including invalid and oversized ranges.
- [ ] Path traversal, symlink escape, device path, and Unicode path tests on macOS.
- [ ] Stored-XSS tests across filenames, MIME values, Markdown, PDF metadata, and error messages.
- [ ] Large-media tests that prove no complete file is buffered in Rust or JavaScript.
- [x] Preview opt-in, attachment batching, image-dimension, and aggregate preview-budget tests.

### Exit criteria

Every required format either previews through the allowlisted safe path or produces a clear fallback. No request can read a file outside the selected root.

## Phase 6: Scale, reliability, and platform hardening

### Work

- [x] Enforce a 16 GiB index-derivative ceiling, a 512 MiB free-space reserve, and a four-times-source-size preflight estimate.
- [x] Apply macOS `0700`/`0600` cache permissions.
- [x] Stream media through four preview slots and a 64 MiB aggregate in-flight byte budget.
- [x] Benchmark the 10,000-conversation profile.
- [x] Benchmark the 500 MB shard profile.
- [x] Exercise at least 10,000 attachment metadata records.
- [x] Profile backend process memory during the synthetic scale runs.
- [ ] Tune SQLite batch sizes and indexes based on measured evidence.
- [ ] Verify progress frequency and cancellation latency.
- [ ] Verify search, list, and conversation-open latency budgets.
- [ ] Add malformed-input fuzzing and property tests.
- [ ] Add macOS-specific path and filesystem tests.
- [ ] Add a macOS WKWebView production smoke test.
- [ ] Test audio/video fallback behavior when codecs are unavailable.
- [ ] Test cleanup after interrupted indexing and abnormal termination.

### Exit criteria

All measurable nonfunctional requirements in the product specification pass on the documented reference environment, or a deviation is documented and accepted before release.

### Recorded macOS arm64 MVP evidence

Release-mode synthetic benchmarks completed on the local Apple arm64 reference
machine:

- 10,000 conversations: 269 ms indexing, 39 ms search, 17,629,184-byte peak RSS.
- 10,000 conversations with 10,000 attachment records: 520 ms indexing,
  21,184,512-byte peak RSS.
- 535,758,301-byte shard stream: 4,577 ms indexing,
  20,217,856-byte peak RSS.

These are deterministic synthetic measurements, not claims about every Mac or
every future export shape.

## Phase 7: Read-only compatibility check

This phase is permitted only after Phases 0 through 6 and their privacy gates pass.

### Work

- [x] Run a structure-only diagnostic against the private export.
- [x] Emit only fixed-schema counts and structural relationships.
- [x] Suppress every scalar value, filename, identifier, timestamp, path, title, message, and attachment value.
- [x] Keep diagnostic output outside the repository.
- [x] Convert the discovered empty-object compatibility case into a newly authored synthetic fixture.
- [x] Delete temporary diagnostics when no longer required.
- [x] Rerun the repository privacy gate after compatibility work.

### Exit criteria

The supported schema is compatible with the private export's structure, no value was printed or persisted, and every resulting test case is independently synthetic.

## Phase 8: Documentation, packaging, and release audit

### Documentation

- [x] Complete `README.md` with synthetic screenshots only.
- [x] State that the application is local-only and does not upload conversations.
- [x] State that the source export is read-only.
- [x] Explain that indexes contain local plaintext copies of searchable content.
- [x] Explain `.dat` attachments.
- [x] Document supported exports and known limitations.
- [x] Document cache locations and deletion.
- [x] Document oversized generated-HTML troubleshooting.
- [x] Publish the privacy and security model.
- [x] Explain safe compatibility reporting without uploading an export.
- [x] Warn users never to attach an export to a public issue.
- [x] State that no software can guarantee detection or removal of every piece of personal information.
- [x] Tell users to review the source and security model before trusting the tool.

### Packaging

- [x] Build the supported macOS arm64 `.app` and DMG artifacts.
- [ ] Build a signed and notarized macOS release candidate.
- [x] Ad-hoc sign and smoke-test the packaged macOS `.app`.
- [ ] Generate checksums, an SBOM, and a third-party license inventory.
- [x] Confirm that no updater is enabled and the launched app listens only on IPv4 loopback.

### Required verification

- [x] Format Rust and frontend sources.
- [x] Lint with warnings treated as errors.
- [x] Run TypeScript type checking.
- [x] Run Rust unit and integration tests.
- [x] Run frontend unit and component tests.
- [x] Run security tests.
- [ ] Run end-to-end tests in development mode.
- [x] Run end-to-end tests against the production build.
- [x] Run performance benchmarks.
- [x] Run the production Tauri build on the supported macOS arm64 target.
- [x] Review dependency vulnerabilities with current RustSec and npm advisories.
- [ ] Generate and review a third-party license inventory.

### Privacy audit before the first commit

- [x] Confirm there are no tracked diffs in the unborn repository.
- [x] Confirm there are no staged files.
- [x] Inspect every nonignored untracked file with the repository privacy scanner.
- [x] Run the privacy gate and secret scanner.
- [x] Search for absolute home paths, private directory fragments, prohibited artifacts, and suspicious personal-data patterns.
- [x] Confirm that screenshots, snapshots, reports, logs, and benchmark output are synthetic.

### Additional privacy audit before the first push

- [x] Re-run the working-tree and staged audits.
- [x] Confirm the tracked-file set is empty before the first commit.
- [x] Scan the complete local Git object database and history.
- [x] Confirm there are no commits intended for publication yet.
- [x] Confirm that no private export path or content appears in Git metadata.
- [x] Record exact audit commands and summarized results in the handoff.
- [x] Record the exact nonignored first-commit file manifest in the handoff.
- [x] Stop and request explicit approval before staging, committing, or pushing.

No push is authorized by this plan.

## Test command contract

The final scripts may use different internal tools, but contributors should have one documented command for each gate:

| Script contract       | Purpose                                                 |
| --------------------- | ------------------------------------------------------- |
| `format:check`        | Rust and frontend formatting                            |
| `lint`                | Rust and frontend linting with warnings as errors       |
| `typecheck`           | TypeScript validation                                   |
| `test:unit`           | Rust and frontend unit tests                            |
| `test:integration`    | Parser, database, API, and attachment integration tests |
| `test:security`       | Path, transport, XSS, CSP, and privacy regression tests |
| `test:e2e`            | Development-mode user flows                             |
| `test:e2e:production` | Packaged or production-server user flows                |
| `test:performance`    | Deterministic 10,000-conversation and 500 MB benchmarks |
| `privacy:check`       | Working-tree and tracked-content privacy gate           |
| `build`               | Frontend and Rust production build                      |
| `package`             | Native desktop artifacts                                |

MVP CI must run the applicable contracts on macOS. Privacy and unit gates run
on every change. Production end-to-end, packaging, and performance jobs may use
dedicated workflows but remain release-blocking. Windows and Linux CI may be
added later as portability work; it is not part of MVP completion.

## Definition of done

The implementation is done when:

- all mandatory requirements in the specification are implemented;
- every acceptance criterion passes against independently generated synthetic exports;
- performance targets pass and results are recorded without private data;
- production packages have been installed and smoke-tested;
- the source export remained unmodified;
- the compatibility check emitted structure only;
- all automated and manual privacy audits pass;
- known limitations are documented; and
- the user has received a concise evidence report and has been asked for approval before any push.
