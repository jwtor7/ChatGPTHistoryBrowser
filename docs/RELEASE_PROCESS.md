# Git and release process

This repository uses protected-trunk development with short-lived branches and
Semantic Versioning.

## Branches

- `main` is protected and must remain releasable.
- Use `feat/<short-name>` for backward-compatible features.
- Use `fix/<short-name>` for bug fixes.
- Use `docs/<short-name>` for documentation-only changes.
- Use `chore/<short-name>` for maintenance that does not change product
  behavior.

Normal changes reach `main` through a pull request. Keep each pull request
focused, include independently synthetic regression coverage, and use squash
merge after required checks pass. Force-pushing or deleting `main` is
prohibited.

## Commit and pull-request expectations

Write imperative commit subjects with a clear area and outcome, for example:

```text
fix: preserve UTF-8 boundaries during marker normalization
docs: clarify macOS installation
```

Pull requests must explain:

1. the user-visible result;
2. privacy and security impact;
3. synthetic evidence added or updated; and
4. the exact verification commands that passed.

Real export content, private screenshots, local indexes, caches, logs, and
absolute personal paths are never acceptable test or review artifacts.

## Versions

The release version must match in:

- `package.json` and the root package in `package-lock.json`;
- `src-tauri/Cargo.toml` and the application package in
  `src-tauri/Cargo.lock`; and
- `src-tauri/tauri.conf.json`.

Use Semantic Versioning:

- patch for backward-compatible bug fixes;
- minor for backward-compatible product features; and
- major for incompatible behavior or data-contract changes.

Every release also needs a dated `CHANGELOG.md` entry derived from machine-local
time.

## Release checklist

From a clean checkout of `main`:

1. Confirm the version sources and changelog agree.
2. Run `npm run check`.
3. Run `npm run test:security`.
4. Run `npm run test:e2e`.
5. Run `npm run test:performance`.
6. Run `npm audit --audit-level=high` and
   `cargo audit --file src-tauri/Cargo.lock`.
7. Run `node scripts/privacy/audit-repo.mjs all` and
   `node scripts/privacy/audit-git-objects.mjs`.
8. Run `npm run tauri:build` and `npm run privacy:package`.
9. Verify the app with `codesign --verify --deep --strict`.
10. Verify the DMG with `hdiutil verify`.
11. Merge the release preparation through the normal pull-request path.
12. Create and push an annotated `vX.Y.Z` tag on the verified `main` commit.

The tag-triggered release workflow repeats the release gates, creates
`ChatGPT-History-Browser-macOS-arm64.dmg`, generates `SHA256SUMS.txt`, and
publishes both to GitHub Releases.

## Signing status

Current public builds are ad-hoc signed for internal bundle integrity. They are
not Developer ID signed or Apple-notarized. Release notes and installation
instructions must state this plainly until notarization is implemented.
