use crate::storage::Storage;
use crate::types::{ContractError, MilestoneDag, MilestoneDependency, MilestoneState};
use soroban_sdk::{Address, Env, Vec};

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

/// Attach a dependency DAG to a grant, recording which milestones depend on which others.
pub fn attach_dag(
    env: &Env,
    _owner: &Address,
    grant_id: u64,
    dependencies: Vec<MilestoneDependency>,
) -> Result<(), ContractError> {
    let dag = MilestoneDag {
        grant_id,
        dependencies,
        is_valid: true,
    };
    Storage::set_milestone_dag(env, grant_id, &dag);
    Ok(())
}

/// Return the indices of milestones whose dependencies are all satisfied
/// (every `depends_on` entry is approved) and which are not yet approved themselves.
pub fn unblocked_milestones(env: &Env, grant_id: u64) -> Vec<u32> {
    let mut result = Vec::new(env);

    let dag = match Storage::get_milestone_dag(env, grant_id) {
        Some(d) if d.is_valid => d,
        _ => return result,
    };

    for dep in dag.dependencies.iter() {
        // Skip already-approved milestones
        if let Some(milestone) = Storage::get_milestone(env, grant_id, dep.milestone_idx) {
            if milestone.state == MilestoneState::Approved {
                continue;
            }
        }

        // Check that every dependency is satisfied
        let mut all_met = true;
        for required_idx in dep.depends_on.iter() {
            match Storage::get_milestone(env, grant_id, required_idx) {
                Some(req_ms) if req_ms.state == MilestoneState::Approved => {}
                _ => {
                    all_met = false;
                    break;
                }
            }
        }

        if all_met {
            result.push_back(dep.milestone_idx);
        }
    }

    result
}

/// Return the indices of milestones that directly depend on the given milestone index.
pub fn dependents_of(env: &Env, grant_id: u64, idx: u32) -> Vec<u32> {
    let mut result = Vec::new(env);

    let dag = match Storage::get_milestone_dag(env, grant_id) {
        Some(d) if d.is_valid => d,
        _ => return result,
    };

    for dep in dag.dependencies.iter() {
        for required_idx in dep.depends_on.iter() {
            if required_idx == idx {
                result.push_back(dep.milestone_idx);
                break;
            }
        }
    }

    result
}

/// Retrieve the full DAG for a grant, if one has been attached.
pub fn get_dag(env: &Env, grant_id: u64) -> Option<MilestoneDag> {
    Storage::get_milestone_dag(env, grant_id)
}

/// Compute a topological ordering of the dependency graph.
/// Returns indices in an order where every milestone appears after its dependencies.
pub fn topological_order(
    env: &Env,
    dependencies: &Vec<MilestoneDependency>,
    total: u32,
) -> Result<Vec<u32>, ContractError> {
    let mut in_degree = soroban_sdk::Map::<u32, u32>::new(env);
    for i in 0..total {
        in_degree.set(i, 0);
    }
    for dep in dependencies.iter() {
        let count = dep.depends_on.len() as u32;
        in_degree.set(
            dep.milestone_idx,
            in_degree.get(dep.milestone_idx).unwrap_or(0) + count,
        );
    }

    let mut queue = Vec::new(env);
    for i in 0..total {
        if in_degree.get(i).unwrap_or(0) == 0 {
            queue.push_back(i);
        }
    }

    let mut sorted = Vec::new(env);
    let mut q_idx = 0;
    while q_idx < queue.len() {
        let node = queue.get(q_idx).unwrap();
        q_idx += 1;
        sorted.push_back(node);

        for dep in dependencies.iter() {
            for req in dep.depends_on.iter() {
                if req == node {
                    let deg = in_degree.get(dep.milestone_idx).unwrap_or(0);
                    in_degree.set(dep.milestone_idx, deg - 1);
                    if deg == 1 {
                        queue.push_back(dep.milestone_idx);
                    }
                    break;
                }
            }
        }
    }

    if sorted.len() != total {
        return Err(ContractError::DependencyNotSatisfied);
    }

    Ok(sorted)
}
