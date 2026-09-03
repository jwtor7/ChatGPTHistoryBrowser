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
8. Run `npm run tauri:build:release` with the Apple signing and notarization
   environment configured. Tauri notarizes and staples the `.app` only.
9. Run `npm run notarize:dmg` against the signed DMG. This submits that DMG
   with `xcrun notarytool submit`, waits until status is `Accepted`, and
   staples the ticket onto the same file. The `.app` ticket cannot be reused
   because notarization tickets are per-cdhash.
10. Run `npm run privacy:package`.
11. Verify the app and DMG signatures with `codesign --verify`.
12. Validate the stapled notarization tickets with `xcrun stapler validate`.
13. Assess the app and DMG with `spctl --assess`.
14. Verify the DMG filesystem with `hdiutil verify`.
15. Merge the release preparation through the normal pull-request path.
16. Create and push an annotated `vX.Y.Z` tag on the verified `main` commit.

The tag-triggered release workflow repeats the release gates, creates
`ChatGPT-History-Browser-macOS-arm64.dmg`, generates `SHA256SUMS.txt`, and
publishes both to GitHub Releases.

## Apple release credentials

The tag workflow fails closed unless these GitHub Actions repository secrets
are configured:

- `APPLE_CERTIFICATE`: base64-encoded Developer ID Application `.p12`;
- `APPLE_CERTIFICATE_PASSWORD`: password used when exporting the `.p12`;
- `APPLE_SIGNING_IDENTITY`: full `Developer ID Application: …` identity shown
  by `security find-identity -v -p codesigning`;
- `APPLE_API_ISSUER`: App Store Connect API issuer ID;
- `APPLE_API_KEY`: App Store Connect API key ID; and
- `APPLE_API_KEY_P8`: complete private key contents downloaded from App Store
  Connect.

`npm run tauri:build` remains ad-hoc signed for local development.
`npm run tauri:build:release` deliberately has no ad-hoc fallback. Tauri imports
the certificate, enables hardened runtime, submits the `.app` to Apple, and
staples that accepted ticket. Tauri 2.11.5 then builds a signed DMG that is
not submitted. The release workflow and `npm run notarize:dmg` submit that
DMG with the App Store Connect API key, wait until notarization is
`Accepted`, and staple the ticket onto the same DMG. The workflow publishes
nothing unless signature, ticket, Gatekeeper, privacy, and DMG-integrity
checks all pass.
