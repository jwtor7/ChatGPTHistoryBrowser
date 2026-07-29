# Privacy and Security

This page is a contributor summary. The normative and more detailed source is
the [Privacy and Security Model](../docs/PRIVACY_SECURITY.md).

## Non-negotiable data rule

Never use a real ChatGPT export in source control, tests, screenshots, issues,
pull requests, CI, logs, or release artifacts. Prohibited material includes
conversation text and metadata, attachment contents and filenames, generated
indexes, absolute personal paths, account details, tokens, and derived values.

Use independently generated synthetic fixtures. Redacting or anonymizing real
data is not sufficient. For compatibility reports, use only the fixed-schema,
value-suppressing process in
[Supported Exports](../docs/SUPPORTED_EXPORTS.md#safe-compatibility-reports).

## Runtime boundaries

- **Source archive:** opened read-only through a validated root capability. The
  app does not rename, normalize, move, delete, or write source files.
- **Derived index:** stored separately in the current user's cache directory
  with restrictive permissions. It contains private plaintext and should be
  discarded when no longer needed.
- **Local API:** bound only to `127.0.0.1` on an ephemeral port. Private routes
  require a random per-launch bearer capability and exact host/origin checks.
- **Webview:** receives bounded projections and opaque identifiers, not
  filesystem paths or database handles. Assets and renderers are bundled.
- **Network:** normal operation has no telemetry, analytics, remote fonts,
  cloud search, automatic update check, or outbound request.

## Untrusted content

Treat every archive value and attachment as hostile input. Rust performs
bounded streaming JSON parsing, graph validation, parameterized SQLite access,
path containment checks, signature-based attachment detection, and fixed-code
errors that do not expose source content.

Markdown raw HTML is disabled and sanitized. Remote resources are inert.
Attachment previews require explicit action and use allowlisted content types,
size and concurrency limits, `nosniff`, and process-local blob URLs. Unsupported
or active formats are save-only.

## Explicit export boundary

Saving a conversation creates a new private copy. The confirmation identifies
the selected path, proposed filename, format, message count, attachment count,
and estimated size. Markdown, PDF, and text exports exclude attachments,
alternate branches, local paths, opaque identifiers, capabilities, diagnostics,
and provider credentials.

Attachment copies use a sanitized display name and an extension derived from
detected content. Unknown or rejected content uses `.bin`; active text-like
suffixes are replaced with `.txt`.

## Required checks

Run the privacy audit before examining or sharing a change:

```sh
npm run privacy
node scripts/privacy/audit-repo.mjs all
node scripts/privacy/audit-git-objects.mjs
```

Before release, the repository also audits dependencies, generated builds,
packages, signatures, and the complete tagged workflow. Scanner success is
defense in depth, not proof that private data is absent; manually review every
changed filename and line.
