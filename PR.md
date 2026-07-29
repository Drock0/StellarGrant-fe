# Enforce MAX_RUBRIC_WEIGHTS in validate_rubric (#812)

## Summary
This PR enforces the `MAX_RUBRIC_WEIGHTS` constant (cap of 6 scoring weights) within `scoring::validate_rubric` to prevent unbounded iteration costs during scoring operations.

Previously, `constants::MAX_RUBRIC_WEIGHTS` (defined as 6) was never checked in `scoring.rs`. An admin could define a rubric with an arbitrary number of `ScoringWeight` entries as long as their total basis points summed to 10,000 BPS. Every subsequent call to `score_contributor` and `rank_contributors` would iterate over all entries in the rubric's weight vector, turning scoring into an unbounded-cost operation.

## Changes
- **`contracts/contracts/stellar-grants/src/scoring.rs`**:
  - Imported `MAX_RUBRIC_WEIGHTS` from `crate::constants`.
  - Added `weights.len() > MAX_RUBRIC_WEIGHTS` check in `validate_rubric`, returning `ContractError::InvalidWeights` if exceeded.
  - Added unit test `test_max_rubric_weights_boundary` confirming that a rubric with `MAX_RUBRIC_WEIGHTS` entries (6) is accepted while `MAX_RUBRIC_WEIGHTS + 1` entries (7) is rejected.

## Verification
- Added boundary unit tests for `MAX_RUBRIC_WEIGHTS` (6 entries accepted, 7 entries rejected with `ContractError::InvalidWeights`).
