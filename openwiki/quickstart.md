# ChatGPT History Browser Quickstart

ChatGPT History Browser is a privacy-first macOS desktop application for
searching an extracted official ChatGPT export. The source export is opened
read-only, while a disposable SQLite full-text index is stored separately in
the current user's cache directory.

## Documentation map

- [Architecture](architecture.md) explains the desktop shell, local API,
  indexing pipeline, and module boundaries.
- [Development](development.md) covers setup, commands, tests, and contribution
  workflow.
- [Privacy and security](privacy-and-security.md) summarizes the trust
  boundaries and safe-development rules.
- [Supported exports](../docs/SUPPORTED_EXPORTS.md) defines accepted archive
  layouts and structure-only compatibility reports.
- [Troubleshooting](../docs/TROUBLESHOOTING.md) covers user-facing recovery and
  safe bug reports.

## Run from source

The supported release target is an Apple silicon Mac. Install Node.js 20.19 or
newer, npm, Rust 1.97.1 with Cargo, rustfmt, and Clippy, and the Xcode Command
Line Tools.

```sh
npm ci
npm run dev
```

`npm run dev` builds the bundled web interface and launches the Tauri
application. `npm run web:dev` runs only the React interface; it cannot use the
authenticated Rust API without mocked responses.

## Use the application

1. Request an official ChatGPT export and extract the downloaded ZIP.
2. Choose the extracted folder containing `conversations.json` or numbered
   `conversations-*.json` shards. Do not choose the ZIP or `chat.html`.
3. Review the detected shard and attachment counts, then build the local index.
   Indexing can be cancelled and resumed at shard boundaries.
4. Search or filter by date, role, archive state, starred state, attachment
   presence, or detected file type.
5. Open a conversation to inspect its active path and alternate branches.
6. Preview supported local attachments or save a copy. Saved copies receive a
   sanitized, meaningful filename and a content-derived passive extension.
7. Use **Export current path…** to save the selected path as Markdown, PDF, or
   plain text. Attachments and alternate branches are not included.
8. Choose **Discard local index** when the derived search data is no longer
   needed. This never changes the source export.

## Verify a change

```sh
npm run check
npm run test:security
npx playwright install chromium
npm run test:e2e
```

Use only independently generated synthetic fixtures. Never put a real export,
index, attachment, filename, screenshot, log, or absolute personal path in the
repository, an issue, or CI output. See [Development](development.md) for the
complete verification matrix.
