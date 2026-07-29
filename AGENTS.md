# Repository Guidelines

## Project Structure & Module Organization

`src/` contains the React and TypeScript interface. The privacy-sensitive desktop backend, local API, indexing, attachment handling, and export logic live in `src-tauri/src/`. Frontend, security, synthetic-data, privacy, browser, and Rust tests are under `tests/` and `src-tauri/`. Keep reusable artwork in `assets/`, generated platform icons in `src-tauri/icons/`, maintenance utilities in `scripts/`, requirements in `specs/`, and supporting guidance in `docs/`.

Start with the [OpenWiki documentation](openwiki/quickstart.md) for the source-grounded project overview and module map.

## Build, Test, and Development Commands

- `npm ci` installs the locked dependencies.
- `npm run dev` starts the complete Tauri desktop application.
- `npm run web:dev` runs the interface only; the authenticated Rust API is unavailable.
- `npm run check` runs privacy, formatting, production web build, lint, type checking, and standard tests.
- `npm run test:e2e` exercises the production web build at desktop and mobile viewports with mocked APIs.
- `npm run test:security` runs focused frontend and Rust security regressions.
- `npm run build` creates optimized web and Rust builds; `npm run tauri:build` produces distributable bundles.

## Coding Style & Naming Conventions

Prettier owns Markdown, JSON, CSS, and TypeScript formatting; ESLint enforces frontend rules. Use two-space indentation, `PascalCase` React components, `camelCase` TypeScript identifiers, and descriptive filenames such as `AttachmentCard.tsx`. Rust uses `rustfmt`, Clippy with warnings denied, and conventional `snake_case` modules and functions. Run `npm run format` only for intentional formatting changes.

## Testing Guidelines

Use Vitest for frontend tests (`*.test.tsx`), Playwright for browser scenarios (`*.spec.ts`), Node’s test runner for privacy and synthetic fixtures, and Rust `#[test]` modules for backend behavior. Every bug fix needs a focused regression. Use only unmistakably fictional generated data; never copy or anonymize a real export.

## Security & Configuration

Preserve read-only source access, authenticated loopback APIs, restrictive file handling, and no normal-use network egress. Never commit exports, indexes, logs, absolute personal paths, credentials, or private screenshots. Run `npm run privacy` before staging and `npm run privacy:staged` afterward.

## Commit & Pull Request Guidelines

Follow the repository’s concise prefixes: `feat:`, `fix:`, `docs:`, `ci:`, `chore:`, or `release:`. Use short-lived `feat/…`, `fix/…`, `docs/…`, or `chore/…` branches. Pull requests must explain user-visible and privacy impact, link relevant issues, include synthetic regression evidence, update documentation, and provide screenshots for interface changes. Keep `main` releasable and never weaken a gate to make CI pass. Derive release dates from local machine time.
