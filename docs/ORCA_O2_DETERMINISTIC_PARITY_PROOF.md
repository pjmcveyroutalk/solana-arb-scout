# Orca O2 Deterministic Parity Proof

Status: PROVEN  
Scope: Read-only quote construction  
Branch: `r14d-orca-o2-parity`  
Baseline main commit: `4e4099fb7e4a22e915cb04d1fc5c030f7e923339`  
Pinned Orca upstream commit: `408c945fef4c49ab70def4303377cfaf8f0f3c99`  
Pinned quote core: `orca_whirlpools_core = 2.1.1`  
Rust contract: `1.80.0`

## Purpose

This checkpoint establishes deterministic parity between Scout's Orca O2 quote-input assembly and the authoritative Orca Whirlpool quote core before any live RPC transport is added.

This proof does not claim an independent reimplementation of Orca CLMM mathematics.

Scout intentionally delegates authoritative swap mathematics to Orca's pinned `swap_quote_by_input_token` implementation.

The parity burden is therefore to prove that Scout supplies the authoritative function with the correct deterministic state, direction, time, fee schedule, tick arrays, and adaptive-fee Oracle state.

## Authoritative quote boundary

Scout's O2 exact-input quote path terminates directly at:

`orca_whirlpools_core::swap_quote_by_input_token`

The returned `ExactInSwapQuote` is returned by Scout without post-processing or replacement arithmetic.

Therefore a second test that calls `swap_quote_by_input_token` with the same already-assembled arguments and compares the result to Scout's wrapper would be tautological.

The meaningful proof surface is the input-assembly boundary.

## Deterministic state proved

### Whirlpool state

Scout validates and preserves the quote-relevant Whirlpool fields:

- tick spacing
- fee tier index seed
- fee rate
- protocol fee rate
- liquidity
- square-root price
- current tick index
- global fee-growth values
- reward timing and growth state

The decoded facade is checked against the normalized O1 pool identity/state before quoting.

### Tick arrays

Scout deterministically constructs the bounded five-array quote window:

1. current
2. current + 1
3. current + 2
4. current - 1
5. current - 2

Both supported Orca account layouts are handled:

- fixed TickArray
- DynamicTickArray

The decoder fails closed on:

- wrong owner
- wrong discriminator
- wrong Whirlpool identity
- wrong start tick index
- malformed dynamic bitmap
- bitmap/tag disagreement
- invalid dynamic length
- initialized bits outside the 88-tick range

Decoded fixed and dynamic layouts converge on the same `TickArrayFacade` representation consumed by Orca core.

### Direction mapping

Scout maps the requested input mint deterministically:

- mint A input -> `specified_token_a = true`
- mint B input -> `specified_token_a = false`

An input mint outside the Whirlpool fails closed.

Both directions are covered by deterministic quote tests.

### Transfer fees

Scout does not invent Token-2022 transfer-fee arithmetic.

Mint extension state is decoded from deterministic snapshot bytes and converted into Orca core `TransferFee` inputs.

The active schedule is selected from the same snapshot Clock epoch.

Activation is exact:

`current_epoch >= newer_fee.epoch`

Existing deterministic tests cover:

- input transfer fee
- output transfer fee
- both swap directions
- fee rounding
- maximum-fee caps
- inverse/gross-up input behavior
- exact activation-epoch transition

Independent integer arithmetic is used in the tests to verify transfer-fee expectations rather than copying the observed Orca result.

### Clock authority

The Clock sysvar is authoritative for both time-sensitive O2 inputs:

- `Clock.epoch` -> Token-2022 transfer-fee schedule
- `Clock.unix_timestamp` -> Orca adaptive-fee timing

These values must originate from the same deterministic snapshot as the quote accounts.

`getEpochInfo` is not authoritative for quote construction and must never override snapshot Clock state.

### Adaptive fee Oracle

Adaptive-fee pools require Oracle state.

Non-adaptive pools must not receive adaptive Oracle state.

Scout validates:

- Oracle owner
- Oracle discriminator
- embedded Whirlpool identity
- trade-enable timestamp
- adaptive fee constants
- adaptive fee variables

A quote before `trade_enable_timestamp` fails closed.

The adaptive timing gate uses the same snapshot Clock timestamp supplied to the quote.

## Output parity

The pinned Orca implementation constructs `ExactInSwapQuote` with:

- `token_in`
- `token_est_out`
- `token_min_out`
- `trade_fee`
- `trade_fee_rate_min`
- `trade_fee_rate_max`

Scout returns the authoritative Orca result directly.

There is no Scout-side recalculation of these output fields after the Orca core call.

Consequently, once deterministic input parity is established, output parity follows directly from the pinned authoritative function boundary.

## CI evidence

The integrated O2 package passed Scout V0 CI on pull request head:

`4f52c13f7220a63d677a4e50c8b722afe4af9348`

It then merged to main as:

`4e4099fb7e4a22e915cb04d1fc5c030f7e923339`

Post-merge Scout V0 CI run #277 passed the canonical Rust 1.80 preflight, compilation, formatting, Clippy, deterministic tests, and safety checks on that exact main commit.

## Explicit non-goals

This checkpoint does not authorize or implement:

- live RPC hydration
- routing admission
- transaction construction
- signing
- submission
- Jito or bundle submission
- borrowing
- flash-loan execution
- treasury mutation
- live trading

R15 remains blocked.

## Deterministic parity verdict

PASS.

The Scout Orca O2 deterministic quote path has sufficient evidence to advance to live read-only parity.

No additional duplicate core-vs-core deterministic test is required.

## Next gate

O2 live parity.

The live implementation must preserve the deterministic proof by hydrating all authoritative quote state in one ordered `getMultipleAccounts` snapshot with `minContextSlot` tied to the O1 observation trigger.

Ordinary pool snapshot:

1. Whirlpool
2. mint A
3. mint B
4. tick array current
5. tick array +1
6. tick array +2
7. tick array -1
8. tick array -2
9. Clock

Adaptive pool snapshot adds:

10. Oracle

The response context slot must be greater than or equal to the observation trigger slot.

After hydration Scout must:

- re-decode the snapshot Whirlpool
- verify stable pool identity against the O1 observation
- recompute the five-array window from snapshot Whirlpool state
- reject a trigger/snapshot tick-window boundary crossing
- validate every non-null account
- apply the proven missing/uninitialized tick-array rules
- derive transfer fees from snapshot mints and snapshot Clock epoch
- derive adaptive timing from snapshot Clock timestamp
- quote both directions through the pinned Orca core
- fail closed on every state or parity mismatch

Only after live parity is proven may Orca advance to QuoteReadiness and registry/route integration.
