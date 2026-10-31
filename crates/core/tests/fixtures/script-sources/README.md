# P0 script fixtures

These P0 fixtures now exercise the shared core Lua executor (moved from the
experimental test host in P2). P3 wires this executor into publisher update and
detail tasks. P6 migrated all built-in adapters to bundled Lua.

`recording.json` records three public Kagura EN responses fetched on 2026-10-30
(HTTP 200), their URLs, full decoded-response SHA-256, retained-section counts,
and fixture SHA-256. Full responses remain outside Git under
`.artifacts/script-source-adapters/p0/`. Fixtures retain the first two product
cards and relevant headings/navigation, or the detail heading and patch tab.
Scripts, unrelated page furniture and images are not needed as test inputs.
Selecting two cards is fixture reduction, not a runtime entry limit.

`kagura-en-prototype.lua` is a non-table API experiment for list roots,
pagination and detail parsing. It is not the complete maintained adapter and is
not bundled or registered as the production Kagura source. List and detail facts
were compared with the legacy Rust parser on the same recorded DOM fragments.
P6 froze those outputs as `kagura-en-{index,detail}-baseline.json` before
removing the Rust parser; current tests compare with those fixed expectations.

The real detail has four mirrors and no Steam app link. The Steam ID test
explicitly injects a synthetic Steam link; it must not be described as a live
identity verification. The multi-root traversal test reuses recorded cards at a
synthetic terminal URL; it does not assert that the live catalog ends at page 2.
The next-page error test verifies the actual recorded pagination link and
explicit error propagation, not a successful full-site crawl.

The full migration requirements are tracked in
[migration-matrix.md](../../../../../docs/features/script-source-adapters/migration-matrix.md).
