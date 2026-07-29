# Feature: Multi-conversation document export

## Overview

Users can collect conversations discovered across paginated search and filter
results and save their default active paths as one readable Markdown, PDF, or
plain-text document. The feature extends the existing single-conversation
export boundary without adding uploads, provider credentials, attachment
copies, or writes to the selected source export.

Users can choose an explicit selection of at most 100 conversations across
pages or the complete submitted search and filter result when it contains at
most 100 conversations.

## Functional requirements

### FR-001: Persistent manual selection

While an archive is open, when the user selects or deselects a conversation,
the system shall preserve the explicit selection while the user changes pages,
searches, or filters.

### FR-002: Visible-page selection

While a result page contains conversations, when the user chooses **Select this
page**, the system shall add every visible conversation to the explicit
selection without duplicating an existing selection.

### FR-003: Selection bound

When an explicit selection would exceed 100 conversations, the system shall
leave the previous selection unchanged and explain the safe limit.

### FR-004: Exact estimate

While one or more conversations are selected, when the user requests export,
the system shall serialize the selected default active paths and show the exact
conversation, message, omitted-attachment, and byte counts before opening the
native save dialog.

### FR-005: Supported formats

When the user changes the selected-set export format, the system shall
recalculate the exact estimate and filename for Markdown, PDF, or plain text.

### FR-006: Deterministic ordering

When a selected-set document is generated, the system shall preserve selection
order, conversation message order, normalized roles, and finite timestamps.

### FR-007: Safe content projection

The system shall exclude attachment bytes and names, alternate branches,
filesystem paths, source and branch identifiers, index metadata, diagnostics,
session capabilities, logs, and provider credentials from selected-set
documents.

### FR-008: Local save

While an estimate is valid, when the user confirms export, the system shall
open the native macOS save dialog and write the selected document outside the
selected read-only source export with restrictive permissions and the required
format extension.

### FR-009: Cancellation

When the user cancels the in-app confirmation or native save dialog, the system
shall create no file and report the operation as cancelled.

### FR-010: Filtered result set

When the user chooses **Select all matching**, the system shall select the
complete submitted search and filter result independent of pagination and
shall explicitly invalidate that selection when its query changes.

### FR-011: Result snapshot

While an all-matching estimate is visible, when the user confirms save, the
system shall re-evaluate the query and serialized document and shall stop
before the native save dialog if the canonical query, ordered result IDs,
format, or serialized bytes differ from the estimate snapshot.

## Non-functional requirements

### Performance and bounds

- A manual selected set shall contain 1 to 100 unique opaque conversation IDs.
- An all-matching set shall contain 1 to 100 conversations after the backend
  resolves the submitted query independently of pagination.
- A selected set shall contain at most 100,000 projected messages and 100,000
  omitted attachment records.
- A generated document shall not exceed 128 MiB.
- Text supplied to native PDF generation shall not exceed 8 MiB or 2,000 pages.
- Estimate requests shall remain subject to the authenticated API's 30-second
  timeout.
- Only one PDF export shall use native rendering at a time.

### Security and privacy

- Every request shall use the authenticated loopback API and same-origin
  mutation protection.
- The backend shall validate every identifier, reject duplicates, and enforce
  bounds independently of the renderer.
- A result snapshot shall be an opaque digest and shall not contain query text,
  conversation IDs, document content, or filesystem metadata.
- Markdown shall neutralize active HTML and remote Markdown resources outside
  literal code.
- All formats shall visibly encode terminal control characters.
- Export shall make no outbound network request and request no provider
  credentials.
- Automated fixtures and screenshots shall be independently synthetic.

### Accessibility

- Every row-selection checkbox shall have a conversation-specific accessible
  name.
- The visible-page selector and selected count shall be keyboard and
  screen-reader accessible.
- The export confirmation shall retain focus trapping, Escape cancellation,
  format radio semantics, and focus return to the initiating button.

## Acceptance criteria

### AC-001: Select across pages

Given a synthetic result set spanning multiple pages,
when the user selects conversations on page one and page two,
then all selections remain selected,
and the displayed count equals the unique selected conversations.

### AC-002: Export an ordered selected set

Given two selected synthetic conversations,
when the user exports them as Markdown,
then the confirmation reports two conversations and the exact serialized size,
and the saved document contains both titles in selection order,
and each conversation preserves its message order, roles, and timestamps.

### AC-003: Format change

Given a valid selected-set estimate,
when the user changes from Markdown to plain text or PDF,
then the old estimate is discarded,
and the filename, exact byte count, and save action match the new format.

### AC-004: Privacy exclusions

Given selected conversations with attachments, branches, internal identifiers,
and diagnostics,
when every supported format is serialized,
then none of those excluded values appears in the generated document,
and the attachment count is reported as omitted.

### AC-005: Safe limit

Given 100 conversations are already selected,
when the user attempts to add another conversation,
then the selection remains at 100,
and the UI explains the limit,
and a direct backend request with 101 IDs returns `RESOURCE_LIMIT`.

### AC-006: Cancellation

Given a selected-set export confirmation,
when the user cancels the confirmation or native save dialog,
then no file is created,
and the UI reports cancellation without claiming completion.

### AC-007: No network egress

Given the production web build with synthetic same-origin API mocks,
when a selected-set export is estimated,
then every application request remains same-origin,
and no off-origin request is attempted.

### AC-008: Select all matching

Given a synthetic filtered result that spans multiple pages and contains no
more than 100 conversations,
when the user chooses **Select all matching**,
then the backend resolves every matching conversation independent of
pagination,
and the UI identifies the mode as all matching rather than manual selection,
and the estimate contains the complete result count.

### AC-009: Query or result invalidation

Given an active all-matching selection,
when the submitted search or filters change,
then the UI clears that selection and explains why,
and when the backend result or serialized document changes after estimate,
then save returns `RESULT_SET_CHANGED` before opening the native save dialog.

## Error handling

| Error condition                              | HTTP status | Public behavior                                      |
| -------------------------------------------- | ----------: | ---------------------------------------------------- |
| Empty or oversized selection                 |         400 | `RESOURCE_LIMIT`: explain the safe processing limit  |
| Malformed or duplicate identifier            |         400 | `INVALID_REQUEST`: reject the complete request       |
| Missing selected conversation                |         404 | `CONVERSATION_NOT_FOUND`: create no estimate or file |
| Destination inside the source export         |         400 | `PATH_REJECTED`: create no file                      |
| Generated output exceeds a byte or PDF bound |         400 | `RESOURCE_LIMIT`: fail before the save dialog        |
| Matching query, result, or document changed  |         409 | `RESULT_SET_CHANGED`: clear and require review       |
| Expired loopback capability                  |         401 | `UNAUTHORIZED`: request application restart          |
| Native dialog cancellation                   |         200 | Return `saved: false` without a completion claim     |

## Implementation TODO

### Backend

- [x] Add bounded selected-set serialization for Markdown, PDF, and plain text.
- [x] Validate 1 to 100 unique opaque conversation IDs.
- [x] Add authenticated estimate and save endpoints.
- [x] Reuse restrictive destination, extension, containment, and PDF controls.
- [x] Add query-snapshot support for **Select all matching**.

### Frontend

- [x] Add accessible per-row and visible-page selection controls.
- [x] Preserve manual selections across pages and filters.
- [x] Reuse the exact-estimate export dialog for selected sets.
- [x] Add clear-selection and safe-limit feedback.
- [x] Add **Select all matching** with explicit query invalidation.

### Testing

- [x] Add Rust ordering, bound, privacy, estimate, filename, and PDF tests.
- [x] Add frontend request, format-change, save-cancellation, and accessibility
      coverage.
- [x] Add production browser coverage with same-origin-only mocked APIs.
- [x] Add filtered-result query snapshot and invalidation coverage.

## Out of scope

- Attachment packaging, tracked in issue #20.
- Provider-specific adapters or compatibility claims, tracked in issue #21.
- Direct provider login, upload, or credentials.
- Automatic AI-generated summaries.
