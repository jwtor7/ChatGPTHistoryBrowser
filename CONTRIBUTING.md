# Contributing

Thank you for helping improve ChatGPT History Browser. Privacy is a release gate,
not an optional review item.

## Never contribute private export data

Do not upload, attach, paste, commit, or quote any part of a real ChatGPT
export. This includes:

- conversation text, titles, IDs, timestamps, and account metadata;
- attachment contents, original filenames, hashes, and opaque `.dat` names;
- screenshots or screen recordings made with real data;
- generated SQLite indexes, WAL/SHM files, logs, crash dumps, and profiler
  output;
- absolute local paths, usernames, hostnames, email addresses, phone numbers,
  tokens, or other identifying values.

This rule applies to issues, pull requests, discussions, benchmarks, test
fixtures, CI artifacts, and communication with maintainers. A redaction is not
automatically safe: context, identifiers, lengths, and filenames can still be
identifying.

Use the repository's synthetic generator for every reproducible example. If a
real export exposes a compatibility problem, reduce the observation to
structure-only metadata using the process in
[Supported Exports](docs/SUPPORTED_EXPORTS.md#safe-compatibility-reports).
Never send the source export to a contributor.

## Development setup

Install the prerequisites listed in [README.md](README.md#prerequisites), then:

```sh
git clone <repository-url>
cd ChatGPTHistoryBrowser
npm ci
```

## Git workflow

`main` is protected and must stay releasable. Create a short-lived branch for
normal work:

- `feat/<short-name>` for features;
- `fix/<short-name>` for bug fixes;
- `docs/<short-name>` for documentation; or
- `chore/<short-name>` for maintenance.

Open a focused pull request, wait for required privacy, quality, security, and
E2E checks, then squash merge. Do not force-push or commit directly to `main`
except for an explicitly authorized repository bootstrap or emergency recovery.
Versioned releases are annotated `vX.Y.Z` tags created from verified `main`
commits.

See [Git and release process](docs/RELEASE_PROCESS.md) for version sources,
release gates, artifact naming, and signing status.

Start the desktop application:

```sh
npm run dev
```

The web-only command is useful for isolated styling work, but it cannot access
the authenticated Rust API:

```sh
npm run web:dev
```

## Synthetic fixtures

Create an empty temporary directory outside the repository:

```sh
npm run generate:synthetic -- --output /tmp/chatgpt-history-browser-fixture
```

Optional generator flags include `--count`, `--shard-size`, `--large`, and
`--large-messages`. Run the command with `--help` for current usage. The
generator refuses destinations inside the repository and destinations that
already contain files.

Synthetic data should be unmistakably fictional and must not be copied from a
real conversation. Do not add "anonymized" real exports as fixtures.

## Before opening a pull request

Format, lint, type-check, test, and build:

```sh
npm run format:check
npm run lint
npm run typecheck
npm test
npm run test:security
npm run build
```

For changes that affect the browser workflow:

```sh
npx playwright install chromium
npm run test:e2e
```

Run the privacy gates before inspecting or sharing the diff:

```sh
node scripts/privacy/audit-repo.mjs all
node scripts/privacy/audit-git-objects.mjs
```

If Gitleaks is installed, also scan every reachable commit:

```sh
gitleaks git . --redact --log-opts='--all'
```

Scanners are defense in depth. They cannot prove that a repository contains no
PII or private conversation data. Review every changed filename and line
yourself.

## Pull request expectations

A pull request should:

- explain the user-visible behavior and privacy impact;
- include synthetic tests for changed behavior;
- preserve read-only access to the selected export;
- preserve the no-normal-use-egress boundary;
- avoid new telemetry, remote assets, permissive CORS, or unrestricted file
  access;
- update documentation when supported export structures or limits change;
- contain no uploaded CI artifacts that could capture application data; and
- pass the privacy job before dependency installation or any other CI job.

Keep changes focused. Do not weaken a privacy, security, formatting, linting,
type, test, build, or E2E gate just to make CI green. If a gate identifies an
intentional exception, document the narrow reason and add a regression test.

## Reporting bugs safely

Before opening an issue, read
[Troubleshooting](docs/TROUBLESHOOTING.md#report-a-problem-safely). Describe
behavior with synthetic reproduction steps and structure-only counts. Do not
include logs unless you have manually reviewed every line and confirmed that it
contains no export-derived value or local path.

Security vulnerabilities should follow [SECURITY.md](SECURITY.md), not a public
issue.

## Contribution license

By submitting a contribution, you agree that it may be distributed under the
repository's [MIT License](LICENSE).
