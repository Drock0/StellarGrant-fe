use soroban_sdk::{contractevent, token, Address, Env, String};

use crate::errors::ContractError;
use crate::storage::Storage;
use crate::types::{BondClaim, BondStatus, GrantStatus, PerformanceBond};

// ── Events ──────────────────────────────────────────────────────────────────

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondRequired {
    pub bond_id: u32,
    pub grant_id: u64,
    pub bond_amount: i128,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondPosted {
    pub bond_id: u32,
    pub grant_id: u64,
    pub guarantor: Address,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondReleased {
    pub bond_id: u32,
    pub grant_id: u64,
}

#[contractevent]
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct BondClaimed {
    pub bond_id: u32,
    pub grant_id: u64,
    pub payout_amount: i128,
}

// ── Public API ──────────────────────────────────────────────────────────────

/// Grant owner requires a bond for their grant. Sets the bond requirement.
pub fn require_bond(
    env: &Env,
    owner: &Address,
    grant_id: u64,
    token: &Address,
    amount: i128,
    ttl_ledgers: u32,
) -> Result<u32, ContractError> {
    owner.require_auth();

    let grant = Storage::get_grant(env, grant_id).ok_or(ContractError::GrantNotFound)?;
    if grant.owner != *owner {
        return Err(ContractError::Unauthorized);
    }
    if amount <= 0 {
        return Err(ContractError::InvalidInput);
    }
    if Storage::get_performance_bond(env, grant_id).is_some() {
        return Err(ContractError::AlreadyRegistered);
    }

    let bond_id = Storage::next_bond_id(env);
    let now = env.ledger().timestamp();
    let bond = PerformanceBond {
        id: bond_id,
        grant_id,
        principal: grant.owner.clone(),
        // Guarantor is finalised when the bond is posted; defaults to the principal.
        guarantor: grant.owner.clone(),
        bond_amount: amount,
        token: token.clone(),
        status: BondStatus::Pending,
        posted_at: None,
        expires_at: now + ttl_ledgers as u64,
    };
    Storage::set_performance_bond(env, &bond);
    Storage::set_bond_grant(env, bond_id, grant_id);

    BondRequired {
        bond_id,
        grant_id,
        bond_amount: amount,
    }
    .publish(env);

    Ok(bond_id)
}

/// Guarantor posts the bond by depositing the bond amount.
pub fn post_bond(env: &Env, guarantor: &Address, bond_id: u32) -> Result<(), ContractError> {
    guarantor.require_auth();

    let grant_id = Storage::get_bond_grant(env, bond_id).ok_or(ContractError::BondNotFound)?;
    let mut bond =
        Storage::get_performance_bond(env, grant_id).ok_or(ContractError::BondNotFound)?;

    if bond.status != BondStatus::Pending {
        return Err(ContractError::BondAlreadyPosted);
    }
    if env.ledger().timestamp() > bond.expires_at {
        return Err(ContractError::BondExpired);
    }

    token::Client::new(env, &bond.token).transfer(
        guarantor,
        &env.current_contract_address(),
        &bond.bond_amount,
    );

    bond.guarantor = guarantor.clone();
    bond.status = BondStatus::Active;
    bond.posted_at = Some(env.ledger().timestamp());
    Storage::set_performance_bond(env, &bond);

    BondPosted {
        bond_id,
        grant_id,
        guarantor: guarantor.clone(),
    }
    .publish(env);

    Ok(())
}

/// Release bond back to the guarantor after successful grant completion.
pub fn release_bond(env: &Env, grant_id: u64) -> Result<(), ContractError> {
    let mut bond = match Storage::get_performance_bond(env, grant_id) {
        Some(b) => b,
        None => return Ok(()), // no bond on this grant — nothing to release
    };

    if bond.status != BondStatus::Active {
        return Err(ContractError::BondNotActive);
    }

    token::Client::new(env, &bond.token).transfer(
        &env.current_contract_address(),
        &bond.guarantor,
        &bond.bond_amount,
    );

    bond.status = BondStatus::Released;
    Storage::set_performance_bond(env, &bond);

    BondReleased {
        bond_id: bond.id,
        grant_id,
    }
    .publish(env);

    Ok(())
}

/// Funder claims the bond payout after a contributor default.
pub fn claim_bond(
    env: &Env,
    funder: &Address,
    grant_id: u64,
    reason: String,
) -> Result<BondClaim, ContractError> {
    funder.require_auth();

    let grant = Storage::get_grant(env, grant_id).ok_or(ContractError::GrantNotFound)?;
    let is_funder = grant.funders.iter().any(|f| f.funder == *funder);
    if !is_funder {
        return Err(ContractError::Unauthorized);
    }

    let mut bond =
        Storage::get_performance_bond(env, grant_id).ok_or(ContractError::BondNotFound)?;
    if bond.status != BondStatus::Active {
        return Err(ContractError::BondNotActive);
    }

    // Claimable only when the grant was cancelled/abandoned or the bond expired.
    let expired = env.ledger().timestamp() > bond.expires_at;
    let cancelled = grant.status == GrantStatus::Cancelled;
    if !expired && !cancelled {
        return Err(ContractError::InvalidState);
    }

    let payout = bond.bond_amount;
    token::Client::new(env, &bond.token).transfer(&env.current_contract_address(), funder, &payout);

    bond.status = BondStatus::Claimed;
    Storage::set_performance_bond(env, &bond);

    let claim = BondClaim {
        bond_id: bond.id,
        claimed_by: funder.clone(),
        claim_reason: reason,
        payout_amount: payout,
        claimed_at: env.ledger().timestamp(),
    };
    Storage::set_bond_claim(env, &claim);

    BondClaimed {
        bond_id: bond.id,
        grant_id,
        payout_amount: payout,
    }
    .publish(env);

    Ok(claim)
}

/// Return the bond for a grant.
pub fn get_bond(env: &Env, grant_id: u64) -> Option<PerformanceBond> {
    Storage::get_performance_bond(env, grant_id)
}

/// Check if a grant has an active (posted) bond.
pub fn has_active_bond(env: &Env, grant_id: u64) -> bool {
    matches!(
        Storage::get_performance_bond(env, grant_id),
        Some(b) if b.status == BondStatus::Active
    )
}
