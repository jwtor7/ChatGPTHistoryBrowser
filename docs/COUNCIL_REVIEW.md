# Council Review: macOS MVP

Review date: 2026-07-28

Four independent specialists reviewed the repository from Security, UX, UI,
and Marketability perspectives. All four structured outputs passed the council
schema validator. The deduplication pass received 27 findings; overlapping
wording was consolidated during implementation review.

## Applied now

- Removed the renderer-controlled arbitrary path-selection endpoint. Export
  access now begins only with the native folder picker.
- Added content-hash verification before reusing indexed shards and revalidated
  attachment type from the exact file handle streamed for preview.
- Clears stale list and conversation content on failed or superseded requests,
  with bounded retry actions.
- Added sanitized indexing failure categories and recovery guidance.
- Added complete keyboard behavior for the destructive index-deletion dialog:
  safe initial focus, Tab containment, Escape, inert background, and focus
  restoration.
- Resolves alternate branch actions to a terminal descendant instead of
  truncating at the first alternate node.
- Distinguishes cancelled attachment saves from completed copies.
- Increased metadata and focus-indicator contrast.
- Tightened archive density, differentiated message roles, and stacked global
  alerts so they do not overlap.
- Reframed the README around the user problem, current source-build status,
  authentic synthetic proof, and verifiable privacy boundaries.
- Added reusable privacy-safe prompts for a hero, onboarding art, icon, social
  card, and authentic screenshot backdrop.

## Accepted but deferred

- Server-side pagination for exceptionally large individual conversation paths.
  The current implementation is bounded, but a polished continuation model
  needs a deliberate API and UX contract.
- Per-export availability metadata for optional archive/star filters.
- A real installed-WKWebView automation harness, Developer ID signing,
  notarization, SBOM, and license artifacts. These are public-distribution
  gates, not source-build MVP claims.
- Generated hero, splash, icon, and social artwork. Prompts are ready, but
  generated assets should not replace the authentic synthetic product capture.

## Declined or narrowed

- Displaying the selected export basename in the renderer was declined because
  it weakens the project’s path-minimization boundary. The onboarding copy now
  explains exactly which extracted folder to choose without exposing a path.
- A wholesale visual redesign was declined. Both UI and UX reviews agreed that
  the existing editorial direction is credible; targeted contrast, density,
  state, and keyboard fixes provide more value.
- Windows, Linux, and unreachable phone-width layouts remain outside the
  macOS-only MVP.
