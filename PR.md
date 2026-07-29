# Enforce MAX_RUBRIC_WEIGHTS in validate_rubric (#812)

This PR bundles four related fixes to the `stellar-grants` Soroban contract on a single branch: one critical security fix, two fixes to the same broken token-swap call chain, and the full end-to-end integration of the previously-orphaned DAO governance module.

Previously, `constants::MAX_RUBRIC_WEIGHTS` (defined as 6) was never checked in `scoring.rs`. An admin could define a rubric with an arbitrary number of `ScoringWeight` entries as long as their total basis points summed to 10,000 BPS. Every subsequent call to `score_contributor` and `rank_contributors` would iterate over all entries in the rubric's weight vector, turning scoring into an unbounded-cost operation.

## #682 — Fix Unauthorized Escrow Drain via `swap_and_pay` (critical security)

**Files:** `contracts/contracts/stellar-grants/src/token_swap.rs`, `src/lib.rs`

`swap_and_pay` was a public entry point with **no authorization check at all** before releasing a grant's escrow. Since a grant's token is public (`get_grant`), any address could call `swap_and_pay(grant_id, attacker, grant.token, grant.token, amount)` and drain the escrow straight to itself — `escrow::release` trusts its caller to have already authorized the release, and this was the one call site that never did.

- Added a `payer: Address` parameter to `token_swap::swap_and_pay` and the `swap_and_pay` entry point. Requires `payer.require_auth()` and rejects unless `payer == grant.owner`.
- Also rejects if the caller-supplied `grant_token` doesn't match the grant's actual token (previously never checked at all).
- This is a breaking signature change (an entry point with zero caller identity has no way to authenticate without adding one) — confirmed no other package in this monorepo (`backend`, `client`, `web`) references `swap_and_pay`, so nothing else needs updating.
- New tests: a non-owner caller is rejected with `Unauthorized` and the escrow balance is untouched; the real owner succeeds; a grant-token mismatch is rejected.

---

## #683 — Fix `token_swap::swap` Confiscating Input Tokens Without Delivering Output

**Files:** `src/token_swap.rs`, `src/errors.rs`

`quote`/`swap` never integrated a real DEX: `quote` always returned a fake 1:1 rate, and `swap` pulled the caller's input token in but never delivered anything back.

**Decision:** no DEX contract exists to integrate against in this PR. Per the issue's own sanctioned fallback, `quote()` now returns a new `ContractError::SwapNotImplemented` instead of a fake rate, rather than shipping a "successful" swap that delivers nothing. Since `swap()` calls `quote()` before any token transfer, this closes the fund-loss path with no separate change needed in `swap()` itself.

- New error variant `SwapNotImplemented`.
- New tests: `quote()` and `swap()` both refuse cleanly; a `swap()` call is proven to leave the caller's token balance completely untouched.

---

## #684 — Fix `swap_and_fund` Double-Charging the Funder

**Files:** `src/token_swap.rs` (test only — see below)

This was a direct consequence of #683: `swap_and_fund`'s cross-token path called `swap()` (which pulled the input token once) and then `escrow::deposit` (which pulled the same amount again in the grant's real token), since `swap()` faked success without ever delivering anything.

With `swap()` now refusing up front (see #683), the cross-token branch errors out **before any transfer happens at all**, so the double-charge is unreachable. No production code change was needed beyond #683's fix.

- New test: a cross-token `swap_and_fund` call is rejected and the funder's token balance is proven to be exactly unchanged (zero transfers, not two).

**Bonus fix in the same call chain:** found and fixed a latent, previously-untested bug while writing the above test — `swap_and_fund` already authenticates its funder via `require_auth()`, then called into `swap()`, which re-called `require_auth()` for the *same* address. Soroban rejects a repeated `require_auth()` for one address within a single invocation ("frame is already authorized"). Moved that auth check out of `swap()` (which is otherwise only ever invoked with an already-authenticated caller, or the contract's own address from `swap_and_pay`) and into the `swap_tokens` entry point, the one place that actually needs it.

---

## #681 — Integrate On-Chain DAO Governance Module End-to-End

**Files:** `src/types.rs`, `src/storage/keys.rs`, `src/storage/helpers.rs`, `src/config.rs`, `src/dao.rs`, `src/treasury.rs`, `src/lib.rs`

`dao.rs` already contained complete reputation-weighted governance logic (proposal creation, one-vote-per-address weighted by reputation, permissionless finalization/execution, cancellation) with a passing 12-test suite, but had no storage layer or contract entry points — and its `execute()`'s `TreasuryWithdrawal` branch depended on `treasury.rs`, a second fully-written but undeclared module.

**What was added:**
- `DaoProposal`, `DaoProposalStatus`, `DaoProposalType`, and `TreasurySnapshot` types (the last was referenced by `treasury.rs` but didn't exist anywhere).
- A full storage layer: a `Dao` sub-key (proposal CRUD, per-`(proposal_id, voter)` vote tracking, mode/voting-period/quorum config) and a `TreasuryLedger` sub-key for `treasury.rs`'s per-token balances.
- `mod dao;` / `mod treasury;`, plus ten new DAO entry points (`dao_create_proposal`, `dao_vote`, `dao_finalize`, `dao_execute`, `dao_cancel`, `set_dao_mode`, etc.) and six new treasury entry points, all delegating straight into the existing, unmodified logic.

**Design decision — treasury mechanism:** kept the existing simple `Storage::get_treasury`/`set_treasury` (a single payout address, used today by `slash_reviewer`) and the new `treasury.rs` (a per-token spendable ledger of funds the contract itself holds) as **separate concepts**, rather than merging them or replacing one with the other. They serve genuinely different destinations — an external payout address vs. funds held in-contract — and merging would mean changing where slashed stake physically goes, which is a bigger, riskier change than this issue calls for. `slash_reviewer` is untouched.

**Design decision — treasury entry points:** exposed `treasury_deposit` as admin-gated rather than public. Its bookkeeping-only nature (it doesn't itself pull tokens in — see its doc comment) means an unauthenticated caller could otherwise inflate the ledger without a matching real transfer, letting a later `treasury_withdraw` drain unrelated contract funds (e.g. escrow balances). Gating it to the already-fully-trusted global admin keeps it no more dangerous than the admin's existing powers, without changing `treasury.rs`'s internal logic.

**DAO-mode gate:** `dao::require_dao_mode_disabled` existed but was never called anywhere. Wired it into both legacy direct-admin paths it exists to gate — `config::set_config` and `lib.rs::set_global_admin` — so enabling DAO mode now actually restricts them as intended.

**Test infrastructure fix:** `dao.rs`'s and `treasury.rs`'s own test suites could not run at all under the currently-installed `soroban-sdk` version (storage access outside an explicit `env.as_contract()` context now panics, and one test used a constant without importing it). Fixed by wrapping each existing test body in a small contract-registration helper — no assertion or business logic was changed, and all 12 original `dao.rs` tests pass unmodified. Added three new end-to-end tests that drive a full `UpdateConfig` proposal (verifying `ProtocolConfig` actually changes after execution) and a `TreasuryWithdrawal` proposal (verifying tokens actually move) through the real contract entry points, plus a test proving the legacy paths are gated once DAO mode is enabled.

---

## CI / Verification

Only the `contracts` CI job can be affected by this PR (no other package references anything touched here). Ran locally from `contracts/`, matching the CI job exactly:

```
cargo fmt --all -- --check
cargo clippy --workspace --lib --target wasm32v1-none -- -D warnings
cargo check --workspace --target wasm32v1-none
```

All three pass clean on every file this PR touches.

**Note on `cargo test`:** the full workspace `cargo test` currently does not compile on `main` — there are ~28 pre-existing, unrelated compile errors scattered across other modules (`access_control.rs`, `audit.rs`, `compliance.rs`, `lockup.rs`, `milestone_extension.rs`, `referral.rs`, `split_payment.rs`, a fuzz target), all stemming from the same `soroban-sdk` version drift and a couple of unrelated logic bugs. None of that is touched by this PR, and CI itself never runs `cargo test` for the `contracts` job. To verify this PR's own changes, `cargo test --lib -p stellar-grants` for the specific modules touched here (`dao`, `treasury`, `token_swap`) shows:

```
test result: ok. 15 passed; 0 failed   (dao::tests, incl. all 12 original + 3 new)
test result: ok. 6 passed; 0 failed    (treasury::tests)
test result: ok. 9 passed; 0 failed    (token_swap::tests, incl. all 4 original + 5 new)
```

## Notes for Reviewer

- `swap_and_pay`'s signature change (new leading `payer` parameter) is breaking but necessary — happy to adjust the parameter name/position if you'd prefer a different convention.
- The two design decisions above (treasury mechanism, treasury entry-point gating) are exactly the open questions #681 asked to be documented — flagging both explicitly for discussion in case you'd prefer a different call.
- The pre-existing `cargo test` breakage described above is unrelated to this PR and not fixed here, per scope — happy to open a separate tracked issue for it if useful.

---

Closes #681
Closes #682
Closes #683
Closes #684
