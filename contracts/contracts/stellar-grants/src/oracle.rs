use soroban_sdk::{Address, Env};

use crate::errors::ContractError;
use crate::storage::Storage;
use crate::types::{OracleConfig, PriceQuote, ProtocolModule};

/// Seven-decimal fixed-point scale for oracle prices.
pub const PRICE_SCALE: i128 = 10_000_000;

mod math {
    use crate::errors::ContractError;

    pub fn proportional_share(
        amount: i128,
        numerator: i128,
        denominator: i128,
    ) -> Result<i128, ContractError> {
        if denominator == 0 {
            return Err(ContractError::InvalidInput);
        }
        amount
            .checked_mul(numerator)
            .ok_or(ContractError::InvalidInput)?
            .checked_div(denominator)
            .ok_or(ContractError::InvalidInput)
    }
}

fn require_global_admin(env: &Env, admin: &Address) -> Result<(), ContractError> {
    let global_admin = Storage::get_global_admin(env).ok_or(ContractError::Unauthorized)?;
    if global_admin != *admin {
        return Err(ContractError::Unauthorized);
    }
    Ok(())
}

/// Read the oracle price for `token` from `oracle` using the safe cross-contract
/// helper (#692). The previous implementation called `env.invoke_contract`
/// directly, which panics/aborts the caller's transaction if the configured
/// oracle is misbehaving (wrong return type, trap, missing function). Routing
/// through `cross_contract::read_oracle_price` returns a `ContractError` instead
/// — caller transactions fail cleanly rather than aborting.
fn fetch_oracle_price(
    env: &Env,
    oracle: &Address,
    token: &Address,
) -> Result<(i128, u64), ContractError> {
    crate::cross_contract::read_oracle_price(env, oracle, token)
}

/// Return the stored oracle config. Returns Err if not configured.
pub fn get_oracle_config(env: &Env) -> Result<OracleConfig, ContractError> {
    Storage::get_oracle_config(env).ok_or(ContractError::InvalidState)
}

/// Check if the oracle price for a token is within the staleness threshold.
pub fn is_price_fresh(env: &Env, quote: &PriceQuote) -> bool {
    let Ok(config) = get_oracle_config(env) else {
        return false;
    };
    let age = env.ledger().timestamp().saturating_sub(quote.fetched_at);
    age <= config.staleness_threshold
}

/// Set the oracle contract address and config. Admin only.
pub fn set_oracle(env: &Env, admin: &Address, config: OracleConfig) -> Result<(), ContractError> {
    admin.require_auth();
    require_global_admin(env, admin)?;
    crate::circuit_breaker::require_open(env, ProtocolModule::Oracle)?;

    if config.staleness_threshold == 0 {
        return Err(ContractError::InvalidInput);
    }

    Storage::set_oracle_config(env, &config);
    Ok(())
}

/// Fetch the current price of `token` from the oracle contract.
/// Returns Err if oracle is not configured or price is stale.
pub fn get_price(env: &Env, token: &Address) -> Result<PriceQuote, ContractError> {
    crate::circuit_breaker::require_open(env, ProtocolModule::Oracle)?;
    let config = get_oracle_config(env)?;
    let (price_in_base, fetched_at) = fetch_oracle_price(env, &config.oracle_address, token)?;

    if price_in_base <= 0 {
        return Err(ContractError::InvalidInput);
    }

    let quote = PriceQuote {
        token: token.clone(),
        price_in_base,
        fetched_at,
        is_stale: false,
    };

    let fresh = is_price_fresh(env, &quote);
    if !fresh {
        return Err(ContractError::InvalidInput);
    }

    Ok(PriceQuote {
        is_stale: false,
        ..quote
    })
}

/// Convert `amount` of `from_token` to equivalent units of `to_token`.
/// Uses oracle prices. Returns Err if either token is not supported.
pub fn convert_amount(
    env: &Env,
    amount: i128,
    from_token: &Address,
    to_token: &Address,
) -> Result<i128, ContractError> {
    crate::circuit_breaker::require_open(env, ProtocolModule::Oracle)?;
    if amount < 0 {
        return Err(ContractError::InvalidInput);
    }
    if from_token == to_token {
        return Ok(amount);
    }

    let from_quote = get_price(env, from_token)?;
    let to_quote = get_price(env, to_token)?;

    let base_amount = math::proportional_share(amount, from_quote.price_in_base, PRICE_SCALE)?;
    math::proportional_share(base_amount, PRICE_SCALE, to_quote.price_in_base)
}

#[cfg(test)]
mod tests {
    use super::*;
    use soroban_sdk::{
        contract, contractimpl, contracttype,
        testutils::{Address as _, Ledger},
        Address, Env,
    };

    #[contracttype]
    #[derive(Clone)]
    enum MockDataKey {
        Price(Address),
    }

    #[contract]
    struct MockOracle;

    #[contractimpl]
    impl MockOracle {
        pub fn set_price(env: Env, token: Address, price: i128, timestamp: u64) {
            env.storage()
                .instance()
                .set(&MockDataKey::Price(token), &(price, timestamp));
        }

        pub fn price(env: Env, token: Address) -> Option<(i128, u64)> {
            env.storage().instance().get(&MockDataKey::Price(token))
        }
    }

    /// A type-mismatched oracle — returns the wrong SCVal shape (#692).
    /// The safe path must surface this as `InvalidInput` instead of aborting.
    #[contract]
    struct WrongTypeOracle;

    #[contractimpl]
    impl WrongTypeOracle {
        pub fn price(_env: Env, _token: Address) -> i128 {
            42
        }
    }

    fn setup_oracle(env: &Env) -> (Address, Address, Address) {
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let admin = Address::generate(env);

        env.as_contract(&contract_id, || {
            Storage::set_global_admin(env, &admin);

            let oracle_id = env.register(MockOracle, ());
            let oracle_addr = oracle_id.clone();
            let base_token = Address::generate(env);
            let xlm_token = Address::generate(env);

            set_oracle(
                env,
                &admin,
                OracleConfig {
                    oracle_address: oracle_addr.clone(),
                    base_token: base_token.clone(),
                    staleness_threshold: 3_600,
                },
            )
            .unwrap();

            (oracle_addr, base_token, xlm_token)
        })
    }

    #[test]
    fn test_is_price_fresh_within_threshold() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let (_, base_token, _) = setup_oracle(&env);

        env.as_contract(&contract_id, || {
            env.ledger().set_timestamp(1_000);
            let quote = PriceQuote {
                token: base_token,
                price_in_base: PRICE_SCALE,
                fetched_at: 800,
                is_stale: false,
            };
            assert!(is_price_fresh(&env, &quote));
        });
    }

    #[test]
    fn test_is_price_fresh_outside_threshold() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let (_, base_token, _) = setup_oracle(&env);

        env.as_contract(&contract_id, || {
            env.ledger().set_timestamp(5_000);
            let quote = PriceQuote {
                token: base_token,
                price_in_base: PRICE_SCALE,
                fetched_at: 1_000,
                is_stale: true,
            };
            assert!(!is_price_fresh(&env, &quote));
        });
    }

    #[test]
    fn test_stale_price_returns_err_from_get_price() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let (oracle_addr, base_token, _) = setup_oracle(&env);

        let oracle_client = MockOracleClient::new(&env, &oracle_addr);
        oracle_client.set_price(&base_token, &PRICE_SCALE, &100);

        env.as_contract(&contract_id, || {
            env.ledger().set_timestamp(10_000);
            assert_eq!(
                get_price(&env, &base_token),
                Err(ContractError::InvalidInput)
            );
        });
    }

    #[test]
    fn test_convert_amount_xlm_usdc_roundtrip() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let (oracle_addr, usdc_token, xlm_token) = setup_oracle(&env);
        let oracle_client = MockOracleClient::new(&env, &oracle_addr);

        let now = 2_000u64;
        env.ledger().set_timestamp(now);
        oracle_client.set_price(&xlm_token, &12_000_000i128, &now);
        oracle_client.set_price(&usdc_token, &PRICE_SCALE, &now);

        env.as_contract(&contract_id, || {
            let xlm_amount = 100_000_000i128; // 10 XLM
            let usdc_amount = convert_amount(&env, xlm_amount, &xlm_token, &usdc_token).unwrap();
            assert_eq!(usdc_amount, 120_000_000); // 12 USDC

            let xlm_back = convert_amount(&env, usdc_amount, &usdc_token, &xlm_token).unwrap();
            assert_eq!(xlm_back, xlm_amount);
        });
    }

    /// #692: a type-mismatched oracle must return Err, not abort.
    /// The previous `env.invoke_contract` path would panic the entire caller
    /// transaction; the safe `cross_contract::read_oracle_price` path returns
    /// a `ContractError` instead.
    #[test]
    fn test_get_price_returns_err_for_type_mismatched_oracle() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let admin = Address::generate(&env);

        env.as_contract(&contract_id, || {
            Storage::set_global_admin(&env, &admin);

            let wrong_id = env.register(WrongTypeOracle, ());
            let base_token = Address::generate(&env);
            let xlm_token = Address::generate(&env);

            set_oracle(
                &env,
                &admin,
                OracleConfig {
                    oracle_address: wrong_id.clone(),
                    base_token: base_token.clone(),
                    staleness_threshold: 3_600,
                },
            )
            .unwrap();

            // Type-mismatched oracle must produce a clean Err — caller doesn't abort.
            assert_eq!(
                get_price(&env, &xlm_token),
                Err(ContractError::InvalidInput)
            );
        });
    }

    /// #692: a not-yet-configured oracle address must produce a clean Err
    /// (the safe-call helper has no target contract to invoke) — caller
    /// doesn't abort, the call returns `InvalidState`.
    #[test]
    fn test_get_price_returns_err_when_oracle_unconfigured() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let admin = Address::generate(&env);

        env.as_contract(&contract_id, || {
            Storage::set_global_admin(&env, &admin);
        });

        let token = Address::generate(&env);
        env.as_contract(&contract_id, || {
            assert_eq!(get_price(&env, &token), Err(ContractError::InvalidState));
        });
    }

    /// #693: tripping the Oracle circuit breaker must block both get_price
    /// and convert_amount, and resetting it must restore them.
    #[test]
    fn test_oracle_circuit_breaker_blocks_get_price_and_convert() {
        let env = Env::default();
        env.mock_all_auths();
        let contract_id = env.register(crate::StellarGrantsContract, ());
        let admin = Address::generate(&env);

        let (oracle_addr, base_token, xlm_token) =
            env.as_contract(&contract_id, || setup_oracle(&env));
        let oracle_client = MockOracleClient::new(&env, &oracle_addr);

        let now = 2_000u64;
        env.ledger().set_timestamp(now);
        oracle_client.set_price(&xlm_token, &12_000_000i128, &now);
        oracle_client.set_price(&base_token.clone(), &PRICE_SCALE, &now);

        env.as_contract(&contract_id, || {
            // Pre-breaker the functions work.
            assert!(get_price(&env, &base_token).is_ok());

            // Trip the Oracle breaker.
            crate::circuit_breaker::trip(
                &env,
                &admin,
                crate::types::ProtocolModule::Oracle,
                soroban_sdk::String::from_str(&env, "compromised feed"),
                None,
            )
            .unwrap();

            // Both refuse with ModuleTripped.
            assert_eq!(
                get_price(&env, &base_token),
                Err(ContractError::ModuleTripped)
            );
            assert_eq!(
                convert_amount(&env, 1_000_000, &xlm_token, &base_token),
                Err(ContractError::ModuleTripped)
            );

            // Reset and confirm the functions work again.
            crate::circuit_breaker::reset(&env, &admin, crate::types::ProtocolModule::Oracle)
                .unwrap();
            assert!(get_price(&env, &base_token).is_ok());
        });
    }
}
