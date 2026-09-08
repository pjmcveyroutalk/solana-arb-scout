# SCOUT V0 — CURRENT STATE

Last updated: 2026-09-08

## Repository state

- Repository: `pjmcveyroutalk/solana-arb-scout`
- Canonical integration branch: `scout-dev`
- Production branch: `main`
- Current `main` SHA: `bb805a34fdbeede6f0e43f5b754ca98ea86fc8df`
- Current `scout-dev` SHA: `bb805a34fdbeede6f0e43f5b754ca98ea86fc8df`
- Branches are intentionally realigned before the Meteora integration sequence begins.
- Operator workflow remains branch-first: implementation work lands on `scout-dev`, deterministic CI is required before merge, and `main` is not edited casually.

## H3 closeout

H3 — centralize RPC and WebSocket transport — is complete and merged.

- PR #7 merged to `main`.
- H3 deterministic Rust 1.80 CI completed successfully before merge.
- Shared RPC transport is centralized in `rpc_transport.rs`.
- Shared WebSocket decoding / subscription lifecycle primitives are centralized in `ws_transport.rs`.
- Orca runtime paths consume the shared transport primitives.
- Existing Scout, Orca O1, Orca O2, cross-venue route, and localized-priority paths remain read-only.

H3 established the transport and subscription-lifecycle foundation required by Meteora without forcing existing venues into an unnecessary redesign.

## Post-H3 control-plane cleanup

Control-plane cleanup is complete.

- Stale PR #3 was closed without merge.
- `scout-dev` was realigned to the merged H3 `main`.
- `.github/workflows/live-smoke.yml` was corrected so live smoke consumes the committed `Cargo.lock` with `cargo run --locked`.
- The workflow no longer regenerates or mutates dependency resolution before runtime certification.
- PR #8 merged that single workflow cleanup into `main`.
- `scout-dev` was realigned again after PR #8.

## Canonical Rust contract

Scout V0 is pinned to Rust 1.80.0 as a source-generation and delivery contract.

Every Rust handoff is expected to satisfy the repository's canonical Rust 1.80 preflight before merge. Formatting is part of source correctness, not post-CI cleanup.

The canonical preflight remains the authority for:

- Rust 1.80 formatting
- compilation
- Clippy
- tests
- dependency / lockfile checks
- Scout safety tripwires

No Rust source is considered Rust-1.80-format-certified until the actual repository preflight passes.

## Runtime certification

Merged `main` SHA:

`bb805a34fdbeede6f0e43f5b754ca98ea86fc8df`

Post-merge deterministic CI completed successfully.

The repaired live-smoke workflow also completed successfully on this exact merged SHA. Successful runtime checks included:

- Scout Existing Live Smoke
- Orca O1 Live Observation
- Orca O2 Live Deterministic Parity
- Orca Cross-Venue Live Route Proof
- Orca + Raydium Localized Priority Proof

This closes the H3 runtime certification gate.

## Safety posture

Scout remains read-only.

The repository continues to deny or tripwire unsafe execution behavior in the current development phase. No signing or transaction-submission authority is being introduced as part of the Meteora market-data / quote integration.

Meteora implementation must preserve this boundary until a later, explicitly authorized execution phase.

## Current venue architecture

Scout's venue-neutral core already supports the concepts required for Meteora integration, including:

- `Venue::Meteora`
- `LiquidityModel::Dlmm`
- `AuxiliaryStateKind::Bins`
- normalized pool state
- venue capability and trading state
- Token-2022 policy gating
- route-level requested / consumed / unspent input accounting
- source-slot tracking

Meteora is recognized by the runtime quote layer but remains fail-closed until its implementation is enabled.

Existing production-proven venue paths remain intact:

- Raydium
- PumpSwap
- Orca Whirlpools

Meteora is additive; this is not a venue-architecture rewrite.

## Meteora DLMM implementation authority

The canonical integration authority is:

`SOLANA ARB SCOUT V0 — METEORA DLMM INTEGRATION HANDOFF`

Status: Integration-Ready Research Package.

Research is closed for the initial integration scope. Do not restart broad research, redesign the architecture, import Anchor, or substitute the full Meteora SDK into Scout.

Official differential/reference baseline:

- repository: `MeteoraAg/dlmm-sdk`
- commit: `576919e3e4368e542c402f000b4264724f7f23ec`

Meteora DLMM program ID:

`LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo`

## Initial Meteora scope

Initial implementation is intentionally narrow:

- DLMM
- exact-input
- `Swap2`
- SPL Token
- currently allowed Token-2022 subset
- profile `0.12.0`
- BinArray v3
- native Rust implementation
- coherent hydration
- limit-order liquidity included in quote truth
- dynamic fees
- bitmap extension
- full-fill arbitrage admission

Deferred:

- exact-output
- unsupported Token-2022 transfer-fee / transfer-hook / scaled-UI behavior
- DAMM v2
- limit-order placement / cancellation
- generalized host-fee support
- ALT correctness as an execution dependency

Unknown or unsupported token behavior remains fail-closed.

## Meteora data-coherence contract

WebSocket activity is a dirty signal.

HTTP hydration is authoritative.

The integration must preserve:

- dirty-slot watermark
- `getMultipleAccounts` minimum-context-slot floor
- generation validation
- refresh-again-if-authoritative-slot-is-behind
- reconnect invalidation
- readiness only after authoritative bootstrap

Subscription lifecycle semantics are venue-neutral:

`requested != confirmed != bootstrapped != ready`

A confirmed subscription alone is never equivalent to quote readiness.

## Operational readiness tiers

Meteora pools progress through:

`COLD -> WARM -> HOT -> EXECUTION_CANDIDATE`

These tiers describe operational readiness and must not be collapsed into a single subscribed / not-subscribed flag.

## Canonical Meteora roadmap

M1 — constants / profile

M2 — strict decoders

M3 — bin / PDA primitives

M4 — bitmap traversal

M5 — immutable snapshot

M6 — core arithmetic

M7 — limit-order liquidity

M8 — exact-input traversal

M9 — hydration planner

M10 — native `Swap2` serializer

M11 — `ExecutionAccountPlan`

M12 — v0 compile + simulation

M13 — Scout integration

M14 — frozen / mainnet certification

This M1-M14 sequence is authoritative. Do not invent or resurrect unsupported H4-H9 historical milestone names.

## Immediate next step

Begin **M1 — constants / profile** on `scout-dev`.

M1 should establish only the smallest stable Meteora surface required by later phases: canonical program identity, supported profile/version identity, narrowly scoped constants, and fail-closed configuration contracts.

Do not advance arithmetic, account decoding, traversal, serializer, or execution-plan logic into M1 unless the repository or canonical Meteora handoff proves it belongs there.

## Operator handoff rules

For implementation work:

- research / audit first
- retrieve exact current source before modifying it
- full replacement files only
- no patches
- one file at a time when practical
- Rust 1.80 compatibility is mandatory
- GitHub CI is the certification authority
- do not ask the operator to format or debug generated Rust
- do not mutate `main` directly
- no tracking parameters in GitHub links
- no execution authority without an explicit later-phase decision

This file is the durable repo-owned current-state ledger for Scout V0.

