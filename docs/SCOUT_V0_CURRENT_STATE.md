# SCOUT V0 — CURRENT STATE

Last updated: 2026-09-10

## Repository state

- Repository: `pjmcveyroutalk/solana-arb-scout`
- Canonical integration branch: `scout-dev`
- Production branch: `main`
- Certified `main` SHA: `e8f2f35724ad4bbcabf7cbfca6a8c022ee5a9b45`
- Certified `main` tree: `9fe29a1b46e356efe5f0e27ef78799236db6c050`
- Current `scout-dev` SHA: `524dd30b4ba677565de0331b70a985e840e5e049`
- Current `scout-dev` tree: `9fe29a1b46e356efe5f0e27ef78799236db6c050`
- PR #35 is merged by a standard two-parent merge commit.
- The certified rollback checkpoint is the exact `main` SHA/tree above, backed by post-merge CI and post-merge live-smoke evidence.

Operator workflow remains branch-first: implementation work lands on `scout-dev`, evidence is independently verified, canonical Rust 1.80 CI certifies source changes, and `main` is not edited casually.

## Current milestone state

- M13 — Scout Meteora integration: **SEALED**
- M14 — Meteora frozen/mainnet differential certification: **SEALED**
- Stage B — normal-runtime Meteora participation + R12 compatibility: **CLOSED**
- Pre-merge history/integrity audit: **CLOSED**
- PR #35 merge certification: **CLOSED**
- New post-merge rollback checkpoint: **CERTIFIED**
- Phase 4 — documentation/source-of-truth synchronization: **ACTIVE**
- Stage C — four-venue route activation: **NEXT AFTER PHASE 4**
- R15 — execution-architecture authorization gate: **HARD BLOCKED**

Do not reopen M13 or M14 without verified regression evidence.

## Certified post-merge checkpoint

Certified `main`:

`e8f2f35724ad4bbcabf7cbfca6a8c022ee5a9b45`

Certified tree:

`9fe29a1b46e356efe5f0e27ef78799236db6c050`

Post-merge deterministic CI:

- Scout V0 CI #518
- Run ID: `34545369583`
- Result: **GREEN**
- Exact `main`: `e8f2f35724ad4bbcabf7cbfca6a8c022ee5a9b45`
- Canonical Rust 1.80 preflight: **PASS**

Post-merge runtime certification:

- Scout V0 Live Smoke #83
- Run ID: `34547165417`
- Event: `workflow_dispatch`
- Branch: `main`
- Exact head: `e8f2f35724ad4bbcabf7cbfca6a8c022ee5a9b45`
- Result: **GREEN**
- All 7 workflow jobs passed.

Retained Live Smoke #83 evidence independently verifies:

- R12 completed with 54/54 unique candidate identities.
- 36 R12 candidates contained truthful Meteora fee serialization.
- R12 completed normally with a `run_end`.
- R13 carried all 54 candidates forward.
- R13 reached maturity.
- All 6 R13 route-history searches completed with `history_complete=true`.
- All 6 completed as `no_atomic_match_complete`.
- R13 ended with `search_incomplete_count=0`.
- Meteora M14 frozen/mainnet differential evidence remained internally consistent.
- The M14 frozen payload was passed to Scout unchanged.
- No native version-byte rewrite occurred.
- Bilateral M14 quotes remained full-fill with zero unspent input.
- Meteora localized priority remains truthfully unavailable/unknown until later contention work supplies a supported scope.

Live Smoke #82 previously failed closed when two high-activity route-history searches saturated the bounded history-pagination window. The succeeding #83 run on unchanged certified `main` demonstrates that #82 was not evidence of a Stage B source regression.

## Four-venue runtime state

Scout's current read-only runtime supports evidence across four venues:

- Raydium
- PumpSwap
- Orca Whirlpools
- Meteora DLMM

Meteora is no longer merely a dormant enum/architecture placeholder. Stage B proved that supported Meteora pools can be observed, hydrated/prepared, admitted to the normal runtime registry, routed cross-venue, quoted, serialized into R12 evidence, replayed, and carried into R13 forensics.

The Stage B merge introduced only the approved effective three-file package:

- `crates/scout-cli/src/main.rs`
- `crates/scout-cli/src/meteora_runtime.rs`
- `crates/scout-cli/src/recorder.rs`

Sealed M13/M14 source was not reopened by the merge.

## Meteora certification authority

The official differential/reference baseline remains:

- Repository: `MeteoraAg/dlmm-sdk`
- Commit: `576919e3e4368e542c402f000b4264724f7f23ec`
- Reference quote function: `commons::quote::quote_exact_in`
- Meteora DLMM program ID: `LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`

M14 remains a read-only frozen/mainnet differential certification surface. It does not grant transaction construction, signing, submission, borrowing, Treasury mutation, or live-trading authority.

## Canonical Rust contract

Scout V0 is pinned to Rust 1.80.0 as a source-generation and delivery contract.

Every Rust handoff is expected to satisfy the repository's canonical Rust 1.80 preflight before merge. Formatting is part of source correctness, not a post-CI cleanup step.

The canonical preflight remains the authority for:

- Rust 1.80 formatting
- compilation
- Clippy with warnings denied
- workspace tests
- committed dependency/lockfile checks
- Scout read-only capability tripwires

No Rust source is considered certified until the actual repository preflight passes.

## Safety posture

Scout V0 remains read-only.

Current work does **not** authorize:

- private keys
- signing
- transaction submission or broadcast
- bundles or Jito searcher execution
- TPU execution
- Treasury mutation
- borrowing execution
- flash execution
- live trading

Public on-chain observation, deterministic quote reconstruction, evidence recording, replay, forensics, and read-only certification remain allowed.

R15 remains **HARD BLOCKED** unless a renewed evidence-based R14 decision explicitly returns `SELECT_NICHE`. Even a future `SELECT_NICHE` result would not itself authorize keys, signing, submission, Treasury mutation, flash execution, or live trading.

## Current active phase — Phase 4

Phase 4 exists to make GitHub and the canonical Drive documents tell the same current story before Stage C begins.

Required synchronization:

1. Update this repo-owned `docs/SCOUT_V0_CURRENT_STATE.md` ledger.
2. Update the canonical post-M14 Working Blueprint with the exact final Stage B, merge, CI, and Live Smoke evidence.
3. Update the canonical rolling Scout Master Handoff continuation marker.
4. Verify no competing current-state document still presents an older milestone as current.
5. Preserve archive/recovery chronology intact.

Stage C must not begin until Phase 4 is verified complete.

## Next build phase — Stage C

After Phase 4 closes, the next active build phase is:

**Stage C — Four-Venue Route Activation**

Stage C is intended to move from Stage B proof that Meteora can participate in normal runtime routes to deliberate, deterministic four-venue route activation coverage.

The Stage C sequence is:

- C01 — audit exact current route/dispatch/readiness interfaces after the Stage B merge
- C02 — define the supported venue-pair matrix for Raydium, PumpSwap, Orca, and Meteora
- C03 — prove existing structural route-validity rules remain venue-neutral
- C04 — add deterministic dispatch/readiness coverage for Meteora combinations supported by evidence
- C05 — preserve full-fill exact-input admission semantics
- C06 — preserve source-slot/freshness provenance across both legs
- C07 — prove no duplicate/invalid same-venue or mismatched-pair routes
- C08 — canonical Rust 1.80 CI
- C09 — dedicated live Stage C evidence gate
- C10 — close Stage C only after live multi-venue route evidence is inspectable and reproducible

Do not skip Stage C gates or advance directly to later contention, priority, repeated-observation, or execution work.

## Later evidence sequence

After Stage C, the canonical post-M14 sequence remains:

- Stage D — Meteora contention footprint
- Stage E — localized priority / competition
- Stage F — repeated observation / temporal evidence
- Stages G/H/I/J — bounded dataset, captureability forensics, cohort reconstruction, and renewed R14 decision
- R15 — blocked authorization gate, reachable only if renewed R14 explicitly returns `SELECT_NICHE`

Meteora localized priority being unavailable/unknown before Stage D/E is intentional and must remain truthful.

## Operator handoff rules

For implementation work:

- research and audit first
- verify exact live repository state before proposing mutation
- complete-file replacement only
- no patches or line-edit instructions
- one operator action at a time
- Rust 1.80 compatibility is mandatory
- GitHub CI certifies source changes
- do not make the operator format or debug generated Rust
- do not mutate `main` casually
- preserve M13/M14 seals absent verified regression
- keep V0 read-only
- keep Cross-Chain/RWA and Pulse Field Lab isolated from this build
- do not advance R15 without the explicit renewed-R14 gate

This file is the durable repo-owned current-state ledger for Scout V0.


