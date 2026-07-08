use soroban_sdk::{contractevent, token, Address, Bytes, Env};

use crate::config;
use crate::constants::BASIS_POINTS_SCALE;
use crate::errors::ContractError;
use crate::storage::Storage;
use crate::types::{ReferralCode, ReferralRecord};

// ── Events ──────────────────────────────────────────────────────────────────

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralCodeCreated {
    pub referrer: Address,
    pub code_hash: Bytes,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralApplied {
    pub referred: Address,
    pub referrer: Address,
    pub code_hash: Bytes,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralRewardEarned {
    pub referrer: Address,
    pub referred: Address,
    pub token: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralRewardsClaimed {
    pub referrer: Address,
    pub token: Address,
    pub amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct ReferralCodeDeactivated {
    pub referrer: Address,
    pub code_hash: Bytes,
}

// ── Helpers ─────────────────────────────────────────────────────────────────

fn is_registered(env: &Env, addr: &Address) -> bool {
    Storage::get_contributor(env, addr.clone()).is_some()
        || Storage::get_reviewer_profile(env, addr).is_some()
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Create a referral code. Any registered contributor or reviewer.
/// Returns the SHA-256 code hash that the referrer shares with new joiners.
pub fn create_code(
    env: &Env,
    referrer: &Address,
    expires_at: Option<u64>,
    max_uses: Option<u32>,
) -> Result<Bytes, ContractError> {
    referrer.require_auth();

    if !is_registered(env, referrer) {
        return Err(ContractError::Unauthorized);
    }

    let now = env.ledger().timestamp();
    if let Some(exp) = expires_at {
        if exp <= now {
            return Err(ContractError::InvalidInput);
        }
    }
    if let Some(mu) = max_uses {
        if mu == 0 {
            return Err(ContractError::InvalidInput);
        }
    }

    // Derive a unique, unguessable code hash from PRNG salt + ledger context.
    let salt: u64 = env.prng().gen();
    let mut plain = Bytes::new(env);
    plain.extend_from_array(&salt.to_be_bytes());
    plain.extend_from_array(&now.to_be_bytes());
    plain.extend_from_array(&(env.ledger().sequence() as u64).to_be_bytes());
    let code_hash: Bytes = env.crypto().sha256(&plain).into();

    let code = ReferralCode {
        code_hash: code_hash.clone(),
        referrer: referrer.clone(),
        created_at: now,
        expires_at,
        max_uses,
        uses: 0,
        is_active: true,
    };
    Storage::set_referral_code(env, &code);

    ReferralCodeCreated {
        referrer: referrer.clone(),
        code_hash: code_hash.clone(),
    }
    .publish(env);

    Ok(code_hash)
}

/// Apply a referral code on registration. Called once per referred address.
pub fn apply_code(env: &Env, referred: &Address, code_hash: &Bytes) -> Result<(), ContractError> {
    referred.require_auth();

    let mut code =
        Storage::get_referral_code(env, code_hash).ok_or(ContractError::ReferralCodeNotFound)?;

    if !code.is_active {
        return Err(ContractError::ReferralCodeInactive);
    }

    let now = env.ledger().timestamp();
    if let Some(exp) = code.expires_at {
        if now > exp {
            return Err(ContractError::ReferralCodeExpired);
        }
    }
    if let Some(mu) = code.max_uses {
        if code.uses >= mu {
            return Err(ContractError::ReferralCodeExhausted);
        }
    }

    // Self-referral and double-referral are rejected.
    if code.referrer == *referred {
        return Err(ContractError::InvalidInput);
    }
    if Storage::get_referral_record(env, referred).is_some() {
        return Err(ContractError::AlreadyReferred);
    }

    code.uses = code.uses.saturating_add(1);
    Storage::set_referral_code(env, &code);

    let record = ReferralRecord {
        referred: referred.clone(),
        referrer: code.referrer.clone(),
        code_hash: code_hash.clone(),
        referred_at: now,
        first_action_at: None,
        reward_paid: false,
    };
    Storage::set_referral_record(env, &record);

    ReferralApplied {
        referred: referred.clone(),
        referrer: code.referrer.clone(),
        code_hash: code_hash.clone(),
    }
    .publish(env);

    Ok(())
}

/// Trigger reward after referred address completes its first qualifying action.
/// No-op (Ok) if there is no referral record or the reward was already paid.
pub fn trigger_reward(
    env: &Env,
    referred: &Address,
    token: &Address,
    fee_amount: i128,
) -> Result<(), ContractError> {
    let mut record = match Storage::get_referral_record(env, referred) {
        Some(r) => r,
        None => return Ok(()),
    };

    if record.reward_paid {
        return Ok(());
    }

    let referral_bps = config::get_config(env).referral_fee_bps;
    let reward = if fee_amount > 0 && referral_bps > 0 {
        fee_amount
            .checked_mul(referral_bps as i128)
            .ok_or(ContractError::InvalidInput)?
            .checked_div(BASIS_POINTS_SCALE as i128)
            .ok_or(ContractError::InvalidInput)?
    } else {
        0
    };

    let now = env.ledger().timestamp();
    record.first_action_at = Some(now);
    record.reward_paid = true;
    Storage::set_referral_record(env, &record);

    if reward > 0 {
        let pending = Storage::get_referral_rewards(env, &record.referrer, token);
        Storage::set_referral_rewards(
            env,
            &record.referrer,
            token,
            pending
                .checked_add(reward)
                .ok_or(ContractError::InvalidInput)?,
        );

        ReferralRewardEarned {
            referrer: record.referrer.clone(),
            referred: referred.clone(),
            token: token.clone(),
            amount: reward,
        }
        .publish(env);
    }

    Ok(())
}

/// Referrer claims accumulated rewards for a token.
pub fn claim_rewards(
    env: &Env,
    referrer: &Address,
    token: &Address,
) -> Result<i128, ContractError> {
    referrer.require_auth();

    let pending = Storage::get_referral_rewards(env, referrer, token);
    if pending <= 0 {
        return Err(ContractError::NoRewardsToClaim);
    }

    Storage::set_referral_rewards(env, referrer, token, 0);

    token::Client::new(env, token).transfer(&env.current_contract_address(), referrer, &pending);

    ReferralRewardsClaimed {
        referrer: referrer.clone(),
        token: token.clone(),
        amount: pending,
    }
    .publish(env);

    Ok(pending)
}

/// Return total unclaimed rewards for a referrer and token.
pub fn pending_rewards(env: &Env, referrer: &Address, token: &Address) -> i128 {
    Storage::get_referral_rewards(env, referrer, token)
}

/// Return the referral record for a referred address.
pub fn get_record(env: &Env, referred: &Address) -> Option<ReferralRecord> {
    Storage::get_referral_record(env, referred)
}

/// Deactivate a referral code. Creator or admin only.
pub fn deactivate_code(
    env: &Env,
    caller: &Address,
    code_hash: &Bytes,
) -> Result<(), ContractError> {
    caller.require_auth();

    let mut code =
        Storage::get_referral_code(env, code_hash).ok_or(ContractError::ReferralCodeNotFound)?;

    let is_creator = code.referrer == *caller;
    let is_admin = Storage::get_global_admin(env) == Some(caller.clone());
    if !is_creator && !is_admin {
        return Err(ContractError::Unauthorized);
    }

    code.is_active = false;
    Storage::set_referral_code(env, &code);

    ReferralCodeDeactivated {
        referrer: code.referrer.clone(),
        code_hash: code_hash.clone(),
    }
    .publish(env);

    Ok(())
}
