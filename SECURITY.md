# Security Policy

## Protect your export

ChatGPT exports can contain years of private conversations, account metadata,
links, identifiers, and attachments. Never attach an export, conversation JSON,
an index database, an application cache, a screenshot of real content, or an
unreviewed diagnostic file to a GitHub issue or security report.

Use only newly generated synthetic data when demonstrating a problem. If a
minimal synthetic reproduction is not possible, describe the structural symptom
without quoting field values. No automated tool can guarantee that it found or
removed every piece of personally identifiable information.

## Reporting a vulnerability

Report vulnerabilities through a
[private GitHub security advisory](https://github.com/jwtor7/ChatGPTHistoryBrowser/security/advisories/new).
Do not open a public issue for a vulnerability that may expose private data.

A safe report may include:

- the application version or commit identifier;
- the operating system and runtime version;
- steps using the repository's synthetic export generator;
- expected and observed behavior;
- a structure-only diagnostic summary after manually reviewing it.

Do not include real titles, messages, prompts, responses, names, organizations,
email addresses, URLs, identifiers, timestamps, attachment names or contents,
filesystem paths, account information, logs, caches, or database files.

If you believe a credential or private export content entered this repository,
stop sharing it, rotate any affected credential, and report the incident
privately. Deleting a file in a later commit does not remove it from Git history.

## Supported code

Security fixes target the current default branch and the latest published
release. Older builds may not receive fixes. A release is not considered
security-reviewed merely because automated tests passed.

The supported MVP platform is macOS. Windows and Linux builds have not been
validated and are future portability work; they are not covered by the MVP
support or security-verification claim.

## Security design

The application is intended to:

- process data on the user's machine with no telemetry or normal-use network
  egress;
- treat the selected export as read-only;
- bind its web server only to the IPv4 loopback address on an operating-system
  assigned port;
- keep filesystem paths in the backend and expose only opaque resource handles
  to browser code;
- sanitize rendered content and never execute export-provided HTML, scripts, or
  shell commands;
- store derived indexes outside both the source export and the repository;
- avoid message bodies, attachment contents, filenames, and source paths in
  ordinary logs.

The complete threat model, privacy gates, cache model, and known limitations are
documented in [docs/PRIVACY_SECURITY.md](docs/PRIVACY_SECURITY.md).

## Disclosure expectations

Please allow maintainers a reasonable opportunity to investigate and prepare a
fix before public disclosure. Testing must remain within data and systems you
are authorized to use. Do not test with another person's export or attempt to
access services beyond the local application instance.
