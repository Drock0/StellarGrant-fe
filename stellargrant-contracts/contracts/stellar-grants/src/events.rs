use soroban_sdk::{contracttype, symbol_short, Address, Env};

#[contracttype]
#[derive(Clone, Debug, PartialEq)]
pub struct MilestoneVoted {
    pub reviewer: Address,
    pub approve: bool,
    pub timestamp: u64,
}

pub fn milestone_voted(
    env: &Env,
    grant_id: u64,
    milestone_idx: u32,
    reviewer: Address,
    approve: bool,
) {
    let topics = (symbol_short!("voted"), grant_id, milestone_idx);
    let data = (reviewer, approve, env.ledger().timestamp());
    #[allow(deprecated)]
    env.events().publish(topics, data);
}
