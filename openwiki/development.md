# Development

## Repository layout

| Path                 | Responsibility                                                                           |
| -------------------- | ---------------------------------------------------------------------------------------- |
| `src/`               | React components, local API client, types, token handling, and styles                    |
| `src-tauri/src/`     | Tauri lifecycle, loopback server, validation, indexing, storage, attachments, and export |
| `tests/frontend/`    | Vitest component behavior                                                                |
| `tests/security/`    | Focused Vitest security regressions                                                      |
| `tests/e2e/`         | Playwright checks against the production web build with mocked same-origin APIs          |
| `src-tauri/tests/`   | Release-scale Rust performance tests                                                     |
| `scripts/privacy/`   | Worktree, history, build, and package privacy gates                                      |
| `scripts/release/`   | Post-bundle DMG notarization and stapling for GitHub Releases                            |
| `scripts/synthetic/` | Safe fixture generator                                                                   |
| `docs/` and `specs/` | Product, security, release, compatibility, and requirement guidance                      |

## Prerequisites and setup

- Node.js 20.19 or newer and npm;
- Rust 1.97.1 with Cargo, rustfmt, and Clippy;
- Xcode Command Line Tools; and
- the Tauri 2 macOS prerequisites.

```sh
npm ci
npm run dev
```

Use `npm run web:dev` only for frontend work that does not require the Rust
backend. `npm run tauri:build` creates an ad-hoc signed app and DMG for local
development. Public releases use `npm run tauri:build:release` inside the tag
workflow with Apple signing and notarization credentials. Tauri notarizes the
`.app`; the workflow then submits and staples the signed DMG before verify.

## Command reference

| Command                    | Purpose                                                                     |
| -------------------------- | --------------------------------------------------------------------------- |
| `npm run check`            | Privacy scan, format check, web build, lint, type check, and standard tests |
| `npm run format:check`     | Prettier plus `cargo fmt --check`                                           |
| `npm run lint`             | ESLint plus Clippy with warnings denied                                     |
| `npm test`                 | Vitest, synthetic-generator, privacy-audit, release, and Rust tests         |
| `npm run notarize:dmg`     | Submit the signed release DMG to Apple and staple the accepted ticket       |
| `npm run test:security`    | Focused frontend and Rust security regressions                              |
| `npm run test:e2e`         | Headless Chromium checks at desktop and mobile viewports                    |
| `npm run test:performance` | Ignored release-scale Rust performance suite                                |
| `npm run privacy`          | Scan the working tree before staging                                        |
| `npm run privacy:staged`   | Scan the staged change set                                                  |
| `npm run tauri:build`      | Create the ad-hoc signed local app and DMG                                  |

Install Chromium once before Playwright:

```sh
npx playwright install chromium
```

Generate fixtures only into a new, empty directory outside the repository:

```sh
npm run generate:synthetic -- --output /tmp/chatgpt-history-browser-fixture
```

The generator refuses repository-contained or non-empty destinations.

## Style and tests

Prettier formats Markdown, JSON, CSS, and TypeScript. ESLint uses typed
TypeScript rules and forbids dynamic evaluation. React components use
`PascalCase`; TypeScript values use `camelCase`. Rust follows `rustfmt`,
`snake_case`, and a crate-wide unsafe-code denial.

Add a focused synthetic regression for every bug fix. Frontend unit tests use
`*.test.tsx`, browser scenarios use `*.spec.ts`, Node tests use `*.test.mjs`,
and Rust tests use `#[test]` or `#[tokio::test]`. The Playwright suite mocks the
API, so backend or desktop behavior also requires Rust coverage.

## Contribution and release flow

Create short-lived `feat/`, `fix/`, `docs/`, or `chore/` branches. Pull requests
must describe user-visible and privacy impact, list exact verification
commands, link relevant issues, and include synthetic evidence. Screenshots are
required for visible interface changes.

Do not weaken a privacy, security, formatting, lint, type, test, build, or E2E
gate. Keep `main` releasable and use the tag-driven process documented in
[Release Process](../docs/RELEASE_PROCESS.md).
