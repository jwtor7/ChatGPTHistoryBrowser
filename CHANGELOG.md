# Changelog

All notable user-facing changes are documented here. This project follows
[Semantic Versioning](https://semver.org/spec/v2.0.0.html).

## [Unreleased]

## [0.2.0] - 2026-07-29

### Added

- Conversation export in Markdown, PDF, and plain text with title-based
  filenames, an exact filename and size preview, and a native macOS save
  dialog.
- Detected file-type filters for images, audio, video, PDFs, text,
  other/unsupported files, and missing attachments.
- Regression coverage for export formatting, filename sanitization, attachment
  extension repair, file-type queries, keyboard focus, estimate cancellation,
  and save cancellation.
- A concise `AGENTS.md` contributor guide covering repository structure,
  development commands, conventions, tests, and pull requests.

### Changed

- Refreshed the macOS app, onboarding, indexing, and browser surfaces around an
  independent archive-search icon, with updated platform assets and
  privacy-safe product screenshots.
- Attachment cards now show a useful detected type and a visible **Save copy**
  action.
- Export UI now identifies the selected message path explicitly, remains
  dismissible while estimating a document, and wraps long exact filenames
  instead of truncating them.

### Fixed

- Attachment copies now receive a meaningful sanitized filename and an
  extension derived from detected content, including generic WAV attachments
  that previously saved as extensionless `Attachment`, plus GIF, WebP, and
  FLAC files.
- Conversation exports no longer use opaque
  `context-….portable.json` filenames.
- PDF export now uses native Unicode text shaping so non-Latin scripts and
  emoji are preserved instead of being replaced with question marks.
- Markdown export preserves literal content inside inline and fenced code while
  continuing to neutralize active HTML and remote resources outside code.
- Confirmed file replacement now uses a private temporary file and atomic
  rename rather than failing when the destination already exists.

### Security

- Conversation documents are created offline with restrictive file
  permissions. They exclude attachments, attachment names, local paths,
  internal identifiers, index metadata, diagnostics, and session capabilities,
  and warn before private data is shared.
- Exported Markdown neutralizes active HTML and remote Markdown resources, and
  all text formats visibly encode terminal control characters.
- Attachment copies use a passive detected extension; unknown content uses
  `.bin` instead of restoring an executable source suffix.
- PDF generation is size-bounded and serialized to prevent concurrent exports
  from exhausting native rendering resources.

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

[Unreleased]: https://github.com/jwtor7/ChatGPTHistoryBrowser/compare/v0.2.0...HEAD
[0.2.0]: https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/tag/v0.2.0
[0.1.1]: https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/tag/v0.1.1
