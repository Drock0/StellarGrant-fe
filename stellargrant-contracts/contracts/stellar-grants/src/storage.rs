use crate::types::{Grant, Milestone};
use soroban_sdk::{contracttype, Env};

#[contracttype]
pub enum DataKey {
    Grant(u64),
    Milestone(u64, u32),
    GrantCounter,
    Contributor(soroban_sdk::Address),
    /// Reviewer stake amount for a grant: (grant_id, reviewer) -> i128
    ReviewerStake(u64, soroban_sdk::Address),
    /// Minimum stake required to review a grant
    MinReviewerStake,
    /// Treasury address for slashed stakes
    Treasury,
    /// Identity oracle contract address for KYC verification
    IdentityOracle,
    ReviewerReputation(soroban_sdk::Address),
}

pub struct Storage;

// Soroban TTL values are expressed in ledgers. At roughly 5 seconds per ledger,
// this refreshes entries when they have less than about 6 days left and extends
// them out to roughly 58 days, which keeps long-lived grants active without
// needing constant writes.
const PERSISTENT_TTL_THRESHOLD: u32 = 100_000;
const PERSISTENT_TTL_EXTEND_TO: u32 = 1_000_000;

impl Storage {
    fn bump_persistent_ttl(env: &Env, key: &DataKey) {
        env.storage().persistent().extend_ttl(
            key,
            PERSISTENT_TTL_THRESHOLD,
            PERSISTENT_TTL_EXTEND_TO,
        );
    }

    // --- Staking helpers ---

    pub fn get_reviewer_stake(env: &Env, grant_id: u64, reviewer: &soroban_sdk::Address) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::ReviewerStake(grant_id, reviewer.clone()))
            .unwrap_or(0)
    }

    pub fn set_reviewer_stake(
        env: &Env,
        grant_id: u64,
        reviewer: &soroban_sdk::Address,
        amount: i128,
    ) {
        env.storage()
            .persistent()
            .set(&DataKey::ReviewerStake(grant_id, reviewer.clone()), &amount);
    }

    pub fn get_min_reviewer_stake(env: &Env) -> i128 {
        env.storage()
            .persistent()
            .get(&DataKey::MinReviewerStake)
            .unwrap_or(0)
    }

    pub fn get_treasury(env: &Env) -> Option<soroban_sdk::Address> {
        env.storage().persistent().get(&DataKey::Treasury)
    }

    pub fn get_identity_oracle(env: &Env) -> Option<soroban_sdk::Address> {
        env.storage().persistent().get(&DataKey::IdentityOracle)
    }

    pub fn get_grant(env: &Env, grant_id: u64) -> Option<Grant> {
        let key = DataKey::Grant(grant_id);
        let grant = env.storage().persistent().get(&key);
        if grant.is_some() {
            Self::bump_persistent_ttl(env, &key);
        }
        grant
    }

    pub fn set_grant(env: &Env, grant_id: u64, grant: &Grant) {
        let key = DataKey::Grant(grant_id);
        env.storage().persistent().set(&key, grant);
        Self::bump_persistent_ttl(env, &key);
    }

    pub fn has_grant(env: &Env, grant_id: u64) -> bool {
        env.storage().persistent().has(&DataKey::Grant(grant_id))
    }

    pub fn get_milestone(env: &Env, grant_id: u64, milestone_idx: u32) -> Option<Milestone> {
        let key = DataKey::Milestone(grant_id, milestone_idx);
        let milestone = env.storage().persistent().get(&key);
        if milestone.is_some() {
            Self::bump_persistent_ttl(env, &key);
        }
        milestone
    }

    pub fn set_milestone(env: &Env, grant_id: u64, milestone_idx: u32, milestone: &Milestone) {
        let key = DataKey::Milestone(grant_id, milestone_idx);
        env.storage().persistent().set(&key, milestone);
        Self::bump_persistent_ttl(env, &key);
    }

    pub fn increment_grant_counter(env: &Env) -> u64 {
        let key = DataKey::GrantCounter;
        let mut counter: u64 = env.storage().persistent().get(&key).unwrap_or(0);
        counter += 1;
        env.storage().persistent().set(&key, &counter);
        Self::bump_persistent_ttl(env, &key);
        counter
    }

    pub fn get_contributor(
        env: &Env,
        contributor: soroban_sdk::Address,
    ) -> Option<crate::types::ContributorProfile> {
        let key = DataKey::Contributor(contributor);
        let profile = env.storage().persistent().get(&key);
        if profile.is_some() {
            Self::bump_persistent_ttl(env, &key);
        }
        profile
    }

    pub fn set_contributor(
        env: &Env,
        contributor: soroban_sdk::Address,
        profile: &crate::types::ContributorProfile,
    ) {
        let key = DataKey::Contributor(contributor);
        env.storage().persistent().set(&key, profile);
        Self::bump_persistent_ttl(env, &key);
    }

    pub fn get_reviewer_reputation(env: &Env, reviewer: soroban_sdk::Address) -> u32 {
        env.storage()
            .persistent()
            .get(&DataKey::ReviewerReputation(reviewer))
            .unwrap_or(1) // Default basic reputation
    }

    pub fn set_reviewer_reputation(env: &Env, reviewer: soroban_sdk::Address, score: u32) {
        env.storage()
            .persistent()
            .set(&DataKey::ReviewerReputation(reviewer), &score);
    }
}
