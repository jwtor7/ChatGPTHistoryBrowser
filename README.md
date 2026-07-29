<p align="center">
  <img src="assets/app-icon.svg" width="88" alt="History Browser app icon">
</p>

<h1 align="center">ChatGPT History Browser</h1>

<p align="center">
  Browse and search your official ChatGPT export on your Mac—locally,
  read-only, and without uploading it.
</p>

<p align="center">
  <strong>macOS · Apple silicon · Local-only · Read-only source · MIT</strong>
</p>

<p align="center">
  <a href="https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/latest/download/ChatGPT-History-Browser-macOS-arm64.dmg"><strong>Download the latest macOS DMG</strong></a>
  ·
  <a href="https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/latest">Release notes and checksums</a>
</p>

![History Browser onboarding screen](docs/images/history-browser-onboarding.png)

## Your archive, readable again

Official exports can contain years of useful context, but a large `chat.html`
is cumbersome to browse and easy to freeze. History Browser reads the
authoritative JSON files, reconstructs branches, and builds a fast disposable
search index on your device.

| Private by design                                              | Useful at scale                                                       | Source stays safe                                                           |
| -------------------------------------------------------------- | --------------------------------------------------------------------- | --------------------------------------------------------------------------- |
| No cloud sync, analytics, remote search, or normal-use egress. | Full-text search, filters, branches, and bounded attachment previews. | The selected export is opened read-only and is never edited or reorganized. |

### What you can do

- Search across years of conversations with a fast local full-text index.
- Filter by date, role, archive state, starred state, attachment presence, and
  detected file type: images, audio, video, PDFs, text, other files, or
  missing.
- Read the active conversation path and recover alternate branches.
- Export the active path as Markdown, PDF, or plain text with a readable
  title-based filename.
- Select conversations across pages or select the complete current filtered
  result, then export the bounded set as one ordered document.
- Preview supported local text, PDF, image, audio, and video attachments
  through a constrained loopback server.
- Cancel and resume indexing.
- Delete the disposable local index without touching the export.

## Install on macOS

The current release supports **Apple silicon Macs**.

1. [Download the latest DMG](https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/latest/download/ChatGPT-History-Browser-macOS-arm64.dmg).
2. Open the DMG and drag **ChatGPT History Browser** into **Applications**.
3. Open the app, choose the folder containing your extracted official export,
   review the detected summary, and start indexing.

### macOS trust status

Release `v0.2.0` was ad-hoc signed and may be blocked by Gatekeeper. Starting
with `v0.2.1`, public releases must be Developer ID signed, Apple-notarized, and
stapled before the release workflow will publish them. Replace `v0.2.0` with
`v0.2.1` or later rather than disabling Gatekeeper globally.

Only download builds from this repository’s
[GitHub Releases](https://github.com/jwtor7/ChatGPTHistoryBrowser/releases).
Compare the DMG against the published `SHA256SUMS.txt` when artifact integrity
matters to your threat model.

## Choose the right folder

Extract the official export ZIP first, then select the extracted directory
containing:

```text
conversations.json
```

or numbered shards such as:

```text
conversations-000.json
conversations-001.json
```

Do not select the ZIP or `chat.html`. The application never needs write
permission to the export directory.

Read [Supported Exports](docs/SUPPORTED_EXPORTS.md) for precise compatibility
rules and safe structure-only reporting.

## Privacy model

History Browser is local-first, not magically risk-free.

- **Source export:** opened read-only.
- **Search index:** stored separately in the current user’s macOS cache area.
- **Index contents:** plaintext and potentially sensitive.
- **Application traffic:** authenticated `127.0.0.1` only.
- **Frontend:** bundled with the app; no remote fonts or scripts.
- **Telemetry:** none.
- **Normal-use internet access:** none.

Anyone or any process that can read your operating-system account, caches, or
backups may be able to read the generated index. Use **Discard local index** in
the app when you no longer need it.

Never upload a real export, index, private screenshot, filename, path, or log to
a public issue. Use independently synthetic fixtures and the repository’s
structure-only compatibility process.

Read the full [Privacy and Security Model](docs/PRIVACY_SECURITY.md), the
[Security Policy](SECURITY.md), and
[Troubleshooting](docs/TROUBLESHOOTING.md).

## How it works

```text
Official extracted export (read-only)
                  │
                  ▼
       bounded streaming parser
                  │
                  ▼
       disposable SQLite index
                  │
                  ▼
 authenticated loopback API (127.0.0.1)
                  │
                  ▼
          bundled React UI
```

Rust owns filesystem access, validation, parsing, indexing, and the local Axum
server. The Tauri webview receives only projected conversation data through a
per-launch capability token. Attachment paths and file signatures are
revalidated before preview or save operations.

When an attachment copy is saved, its suggested filename is sanitized and its
extension is derived from the detected file type. Generic source labels become
readable names such as `Audio attachment 1.wav`; source files are never
renamed. Unknown content uses `.bin`, while active text-like source suffixes
are replaced with the passive `.txt` extension.

## Conversation export

Open a conversation and choose **Export current path…**, then select Markdown,
PDF, or plain text. The app shows the exact filename before saving. A title
such as `**Exceptional Opportunity Scan` becomes
`Exceptional-Opportunity-Scan.md`, `.pdf`, or `.txt`.

To collect several conversations, use the selection checkboxes in the
conversation list and choose **Export selected…**. A manual selection persists
across pages and filters and can contain up to 100 conversations. The combined
document uses each conversation's default active path in selection order and a
filename such as `Selected-conversations-12.md`.

To collect the complete current result, choose **Select all matching**. This
mode is visibly distinct from manual selection, ignores pagination, and is
available when the submitted search and filters match 100 conversations or
fewer. Changing a search or filter clears the all-matching selection. The
backend re-evaluates the query before saving and refuses to open the native
save dialog if the query, ordered result set, format, or serialized document
changed after the estimate.

Every document contains only the chosen active path for each conversation,
with message order, roles, and timestamps; alternate branches are not
included. The confirmation shows the exact serialized size and attachment
count before the native macOS save dialog opens. Attachment bytes and names,
local paths, internal identifiers, and session capabilities are excluded. The
chosen extension is enforced even if the destination name is edited.

Export runs entirely on the device and makes no outbound request. The saved
document contains private data. If you import or share it, the selected
conversation becomes subject to the destination provider’s policies.

## Build from source

Prerequisites:

- Node.js 20.19 or newer;
- npm;
- Rust 1.97.1 with Cargo, rustfmt, and Clippy;
- Xcode Command Line Tools; and
- the [Tauri 2 macOS prerequisites](https://v2.tauri.app/start/prerequisites/).

```sh
git clone https://github.com/jwtor7/ChatGPTHistoryBrowser.git
cd ChatGPTHistoryBrowser
npm ci
npm run dev
```

Build an ad-hoc signed app and DMG for local development:

```sh
npm run tauri:build
```

Public distribution uses `npm run tauri:build:release` in the tag workflow and
requires Apple signing and notarization credentials.

## Verify changes

```sh
npm run check
npm run test:security
npx playwright install chromium
npm run test:e2e
npm run test:performance
npm audit --audit-level=high
cargo install cargo-audit --locked --version 0.22.2
cargo audit --file src-tauri/Cargo.lock
node scripts/privacy/audit-repo.mjs all
node scripts/privacy/audit-git-objects.mjs
```

Every public fixture, benchmark, screenshot, and browser test must be
independently synthetic. The synthetic generator refuses repository-contained
and non-empty destinations:

```sh
npm run generate:synthetic -- --output /tmp/chatgpt-history-browser-fixture
```

## Releases and contributions

- [OpenWiki documentation](openwiki/quickstart.md)
- [Latest release](https://github.com/jwtor7/ChatGPTHistoryBrowser/releases/latest)
- [Changelog](CHANGELOG.md)
- [Contributing](CONTRIBUTING.md)
- [Git and release process](docs/RELEASE_PROCESS.md)
- [Architecture decision](docs/adr/0001-local-desktop-architecture.md)

`main` is protected and releasable. Normal work uses short-lived `feat/`,
`fix/`, `docs/`, or `chore/` branches and squash-merged pull requests after all
required checks pass. Versioned tags automatically run the complete release
workflow and publish the macOS DMG with checksums.

Windows, Linux, and Intel macOS are future release work.

## Independent project

ChatGPT History Browser is not affiliated with or endorsed by OpenAI.
“ChatGPT” is used descriptively to identify the supported export format.

MIT licensed. See [LICENSE](LICENSE).
