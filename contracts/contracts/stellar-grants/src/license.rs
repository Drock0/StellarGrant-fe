use crate::storage::Storage;
use crate::types::{ContractError, IpRights, LicenseRecord, LicenseType};
use soroban_sdk::{Address, Env, String};

/// Attach a license record to an approved milestone deliverable.
/// Only the grant owner may call this. The milestone must already exist.
pub fn attach_license(
    env: &Env,
    caller: &Address,
    grant_id: u64,
    milestone_idx: u32,
    spdx_id: String,
    license_type: LicenseType,
    rights: IpRights,
    restrictions: String,
) -> Result<LicenseRecord, ContractError> {
    caller.require_auth();

    let grant = Storage::get_grant(env, grant_id).ok_or(ContractError::GrantNotFound)?;
    if grant.owner != *caller {
        return Err(ContractError::Unauthorized);
    }
    if milestone_idx >= grant.total_milestones {
        return Err(ContractError::MilestoneIndexOutOfBounds);
    }
    Storage::get_milestone(env, grant_id, milestone_idx).ok_or(ContractError::MilestoneNotFound)?;

    let record = LicenseRecord {
        grant_id,
        milestone_idx,
        spdx_id,
        license_type,
        rights,
        restrictions,
        attached_by: caller.clone(),
        attached_at: env.ledger().timestamp(),
    };
    Storage::set_license_record(env, grant_id, milestone_idx, &record);
    Ok(record)
}

pub fn get_license(env: &Env, grant_id: u64, milestone_idx: u32) -> Option<LicenseRecord> {
    Storage::get_license_record(env, grant_id, milestone_idx)
}
