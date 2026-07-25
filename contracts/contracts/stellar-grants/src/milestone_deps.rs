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

/// Attach a dependency DAG to a grant. Only the grant owner may call this.
pub fn attach_dag(
    env: &Env,
    owner: &Address,
    grant_id: u64,
    deps: Vec<MilestoneDependency>,
) -> Result<(), ContractError> {
    let grant = Storage::get_grant(env, grant_id).ok_or(ContractError::GrantNotFound)?;
    if grant.owner != *owner {
        return Err(ContractError::Unauthorized);
    }
    let dag = MilestoneDag {
        grant_id,
        dependencies: deps,
        is_valid: true,
    };
    Storage::set_milestone_dag(env, grant_id, &dag);
    Ok(())
}

/// Return milestone indices that have no unsatisfied dependencies.
pub fn unblocked_milestones(env: &Env, grant_id: u64) -> Vec<u32> {
    let dag = match Storage::get_milestone_dag(env, grant_id) {
        Some(d) => d,
        None => return soroban_sdk::Vec::new(env),
    };

    let grant = match Storage::get_grant(env, grant_id) {
        Some(g) => g,
        None => return soroban_sdk::Vec::new(env),
    };

    let mut result = soroban_sdk::Vec::new(env);
    for idx in 0..grant.total_milestones {
        let mut blocked = false;
        for dep in dag.dependencies.iter() {
            if dep.milestone_idx == idx {
                for depends_on in dep.depends_on.iter() {
                    if let Some(m) = Storage::get_milestone(env, grant_id, depends_on) {
                        if m.state != MilestoneState::Approved {
                            blocked = true;
                            break;
                        }
                    } else {
                        blocked = true;
                        break;
                    }
                }
            }
            if blocked {
                break;
            }
        }
        if !blocked {
            result.push_back(idx);
        }
    }
    result
}

/// Return the milestone indices that depend on the given index.
pub fn dependents_of(env: &Env, grant_id: u64, idx: u32) -> Vec<u32> {
    let dag = match Storage::get_milestone_dag(env, grant_id) {
        Some(d) => d,
        None => return soroban_sdk::Vec::new(env),
    };

    let mut result = soroban_sdk::Vec::new(env);
    for dep in dag.dependencies.iter() {
        if dep.depends_on.contains(idx) {
            result.push_back(dep.milestone_idx);
        }
    }
    result
}

/// Retrieve the DAG for a grant, if one has been attached.
pub fn get_dag(env: &Env, grant_id: u64) -> Option<MilestoneDag> {
    Storage::get_milestone_dag(env, grant_id)
}

/// Return a topologically sorted order of milestone indices.
/// Returns DependencyNotSatisfied if a cycle is detected.
pub fn topological_order(
    env: &Env,
    deps: &Vec<MilestoneDependency>,
    total: u32,
) -> Result<Vec<u32>, ContractError> {
    let mut visited = soroban_sdk::Vec::<bool>::new(env);
    for _ in 0..total {
        visited.push_back(false);
    }
    let mut in_stack = soroban_sdk::Vec::<bool>::new(env);
    for _ in 0..total {
        in_stack.push_back(false);
    }
    let mut order = soroban_sdk::Vec::<u32>::new(env);

    fn dfs(
        node: u32,
        deps: &Vec<MilestoneDependency>,
        visited: &mut Vec<bool>,
        in_stack: &mut Vec<bool>,
        order: &mut Vec<u32>,
        total: u32,
    ) -> Result<(), ContractError> {
        let v = visited.get(node).unwrap_or(true);
        if v {
            return Ok(());
        }
        let ins = in_stack.get(node).unwrap_or(false);
        if ins {
            return Err(ContractError::DependencyNotSatisfied);
        }

        in_stack.set(node, true);

        for dep in deps.iter() {
            if dep.milestone_idx == node {
                for depends_on in dep.depends_on.iter() {
                    if depends_on < total {
                        dfs(depends_on, deps, visited, in_stack, order, total)?;
                    }
                }
            }
        }

        in_stack.set(node, false);
        visited.set(node, true);
        order.push_back(node);
        Ok(())
    }

    for i in 0..total {
        dfs(i, deps, &mut visited, &mut in_stack, &mut order, total)?;
    }

    Ok(order)
}
