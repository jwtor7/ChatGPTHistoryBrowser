# Changelog

All notable user-facing changes are documented here. This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

### Added

- Provider-neutral active-path export with a versioned JSON manifest, embedded
  Markdown, reusable import prompt, exact pre-save estimates, and a native
  macOS save dialog.
- Round-trip coverage for message order, roles, timestamps, and branch
  provenance, including regression checks that attachment names and branch
  previews are excluded.

### Security

- Portable packages are created offline with opaque filenames and restrictive
  file permissions. They exclude attachments, local paths, index metadata,
  diagnostics, and session capabilities, and warn before private data crosses
  into another provider.

## [0.1.1] - 2026-07-28

### Added

- A local macOS browser for extracted official ChatGPT exports, with full-text
  search, filters, branch recovery, and bounded attachment previews.
- A disposable SQLite index that can be cancelled, resumed, or removed without
  changing the source export.
- Independently synthetic correctness, security, privacy, browser, packaging,
  and 10,000-conversation performance tests.
- A tag-driven release workflow that packages an Apple silicon DMG, verifies
  its signature and checksum, and publishes it with SHA-256 checksums.

### Changed

- The README now leads with a direct macOS download, clear installation
  guidance, the supplied product screenshot, and concise privacy disclosures.
- Native macOS folder and save dialogs are created on the application main
  thread while their results remain asynchronously awaited.

### Fixed

- Corrected zero-based page handling so page numbers no longer skip or repeat
  conversations.
- Stopped structured transcript parts from being misclassified as
  attachments, including reliable invalidation of older local indexes.
- Removed known internal citation markers consistently from messages, search
  snippets, and branch previews.
- Unified message rendering and counting so structural nodes are not counted as
  visible messages.
- Prevented Clear Filters from leaving an already-clear list loading forever.
- Made date filters use the same effective timestamp as display and sorting.
- Prevented a macOS crash when opening the export folder picker.
- Prevented import crashes when a long malformed internal marker crosses a
  multi-byte UTF-8 boundary.

### Security

- Source exports remain read-only, application traffic remains authenticated
  and loopback-only, and packaged artifacts are scanned for prohibited private
  data before release.

[0.1.1]: https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/tag/v0.1.1
