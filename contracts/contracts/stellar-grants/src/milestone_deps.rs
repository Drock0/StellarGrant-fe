use crate::storage::Storage;
use crate::types::{ContractError, MilestoneState};
use soroban_sdk::Env;

/// Check if a milestone can be submitted by verifying all dependencies are satisfied.
/// A milestone can be submitted if all previous milestones have been approved.
pub fn can_submit(env: &Env, grant_id: u64, milestone_idx: u32) -> Result<(), ContractError> {
    if milestone_idx == 0 {
        return Ok(());
    }

    for prev_idx in 0..milestone_idx {
        if let Some(milestone) = Storage::get_milestone(env, grant_id, prev_idx) {
            if milestone.state != MilestoneState::Approved {
                return Err(ContractError::DependencyNotSatisfied);
            }
        } else {
            return Err(ContractError::DependencyNotSatisfied);
        }
    }

    Ok(())
}
