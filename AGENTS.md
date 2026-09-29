# Repository Guidelines

This file is the shared instruction source for every coding agent working in this repository. Claude Code imports it from `CLAUDE.md`; other agents read it directly or through their own thin pointer file. Put shared guidance here and keep agent-specific files as pointers so the agents never drift apart.

## Project Structure & Module Organization

`src/` contains the React and TypeScript interface. The privacy-sensitive desktop backend, local API, indexing, attachment handling, and export logic live in `src-tauri/src/`. Frontend, security, synthetic-data, privacy, browser, and Rust tests are under `tests/` and `src-tauri/`. Keep reusable artwork in `assets/`, generated platform icons in `src-tauri/icons/`, maintenance utilities in `scripts/`, requirements in `specs/`, and supporting guidance in `docs/`.

Start with the [OpenWiki documentation](openwiki/quickstart.md) for the source-grounded project overview and module map.

## Architecture Notes

The [Architecture](openwiki/architecture.md) page and [ADR-0001](docs/adr/0001-local-desktop-architecture.md) are normative. Facts that span files and are easy to miss:

- There is no Tauri IPC. `src-tauri/tauri.conf.json` declares zero windows and zero capabilities; `run_desktop` in `src-tauri/src/lib.rs` binds an Axum listener on `127.0.0.1:0`, spawns it, and opens one webview at `{origin}/#token={random}`. The SPA and `/api/*` share that origin, and the webview has no filesystem, shell, dialog, or invoke access.
- Auth flow: `src/token.ts` strips the fragment with `history.replaceState` and keeps the token in module memory; `LocalApi` in `src/api.ts` sends it as a bearer header and accepts only relative `/api/` paths. `src-tauri/src/server.rs` enforces Host, Origin, method, content type, a 16 KiB body limit, and security headers, and splits routes into `bounded_api` (30 s timeout) and `interactive_api` (native pick/save dialogs, no timeout).
- Indexing pipeline: `safe_root` (validates the export dir, read-only cap-std root, shard and attachment discovery, source fingerprint) → `json_stream` (incremental top-level array) → `conversation` (projects untrusted records into bounded models, validates the graph) → `indexer` (background work, progress, cancel, `ArchiveSession`) → `store` (SQLite + FTS5 in the per-user `ProjectDirs` cache dir, never beside the export). Bump `INDEX_PROJECTION_VERSION` in `store.rs` whenever projected output changes; a mismatch forces a reindex.
- `attachments.rs` re-validates opaque handles and sniffs signatures before preview or save; `portable_export.rs` serializes the active path only (PDF via CoreGraphics/CoreText on macOS); `structure_inspector.rs` emits value-suppressing diagnostics.
- `src/types.ts` mirrors the serde output of `src-tauri/src/models.rs`; change both together. Error codes are fixed strings in `error.rs` mapped to UI copy in `src/App.tsx`.

## Build, Test, and Development Commands

- `npm ci` installs the locked dependencies.
- `npm run dev` starts the complete Tauri desktop application. It builds `dist/` first, and there is no HMR against the Rust API, so rebuild to see Rust changes.
- `npm run web:dev` runs the interface only; the authenticated Rust API is unavailable.
- Every `cargo` invocation (clippy, test, build) needs `dist/index.html` to exist because Tauri's build script validates `frontendDist`. In a fresh checkout run `npm run build:web` first.
- `npm run check` runs privacy, formatting, production web build, lint, type checking, and standard tests.
- `npm run test:e2e` exercises the production web build at desktop and mobile viewports with mocked APIs. Install the browser once with `npx playwright install chromium`.
- `npm run test:security` runs focused frontend and Rust security regressions. The Rust half is `cargo test security`, a substring filter: a new Rust security regression must contain `security` in its test name or it will not run there.
- `npm run test:performance` runs the `#[ignore]` release-scale suite in `src-tauri/tests/performance.rs`.
- Single tests: Vitest `npx vitest run tests/frontend/App.test.tsx -t "name"`; Rust `cargo test --manifest-path src-tauri/Cargo.toml store::tests::name`; Node `node --test tests/privacy-audit.test.mjs`; Playwright `npx playwright test -g "name"`.
- `npm run generate:synthetic -- --output /tmp/fixture` writes fixtures into an empty directory outside the repo. For a structure-only compatibility report, pipe a path into `cargo run --manifest-path src-tauri/Cargo.toml --bin inspect_structure -- --path-stdin`.
- `npm run build` creates optimized web and Rust builds. `npm run tauri:build` produces ad-hoc signed distributable bundles; `npm run tauri:build:release` fails closed without Apple credentials.

## Coding Style & Naming Conventions

Prettier owns Markdown, JSON, CSS, and TypeScript formatting; ESLint enforces frontend rules. Use two-space indentation, `PascalCase` React components, `camelCase` TypeScript identifiers, and descriptive filenames such as `AttachmentCard.tsx`. Rust uses `rustfmt`, Clippy with warnings denied, and conventional `snake_case` modules and functions. Run `npm run format` only for intentional formatting changes.

## Testing Guidelines

Use Vitest for frontend tests (`*.test.tsx`), Playwright for browser scenarios (`*.spec.ts`), Node’s test runner for privacy and synthetic fixtures, and Rust `#[test]` modules for backend behavior. Every bug fix needs a focused regression. Use only unmistakably fictional generated data; never copy or anonymize a real export.

Rust tests are inline `#[cfg(test)]` modules; `server.rs` drives `build_router` through `tower::ServiceExt::oneshot`. Vitest runs in jsdom with `tests/setup.ts` and stubs global `fetch`. Playwright mocks `/api/*` against `vite preview` on port 4173. None of the JS suites reach Rust, so backend behavior needs a Rust test.

## Security & Configuration

Preserve read-only source access, authenticated loopback APIs, restrictive file handling, and no normal-use network egress. Never commit exports, indexes, logs, absolute personal paths, credentials, or private screenshots. Run `npm run privacy` before staging and `npm run privacy:staged` afterward.

The privacy audit (`scripts/privacy/audit-repo.mjs`) fails on content matching `/Users/<x>/`, `/home/<x>/`, non-reserved email addresses, phone- or SSN-shaped numbers, bearer-token-shaped strings, and credential assignments, and on paths such as `conversations*.json`, `chat.html`, `*.dat`, `*.zip`, `*.db`, `*.log`, `.env`, and `.DS_Store`. Use repo-relative paths in every tracked file, including this one. The CI privacy job runs with bare Node before `npm ci`, so `scripts/privacy/*.mjs` must stay dependency-free.

## Commit & Pull Request Guidelines

Follow the repository’s concise prefixes: `feat:`, `fix:`, `docs:`, `ci:`, `chore:`, or `release:`. Use short-lived `feat/…`, `fix/…`, `docs/…`, or `chore/…` branches. Pull requests must explain user-visible and privacy impact, link relevant issues, include synthetic regression evidence, update documentation, and provide screenshots for interface changes. Keep `main` releasable and never weaken a gate to make CI pass. Derive release dates from local machine time.

A release version must match in `package.json`, the root entry of `package-lock.json`, `src-tauri/Cargo.toml`, `src-tauri/Cargo.lock`, and `src-tauri/tauri.conf.json`, plus a dated `CHANGELOG.md` entry. Annotated `vX.Y.Z` tags drive the release workflow.
