use crate::quote::QuoteReadiness;
use scout_core::{NormalizedPoolState, Venue};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ActiveMint {
    mint: String,
    venues: Vec<Venue>,
    pool_ids: Vec<String>,
}

impl ActiveMint {
    pub fn summary(&self) -> String {
        let venue_labels = self
            .venues
            .iter()
            .map(|venue| venue.label())
            .collect::<Vec<_>>()
            .join(",");

        format!(
            "mint={} venue_count={} pool_count={} venues={}",
            self.mint,
            self.venues.len(),
            self.pool_ids.len(),
            venue_labels,
        )
    }
}

#[derive(Debug, Default)]
struct MintTracker {
    venues: Vec<Venue>,
    pool_ids: BTreeSet<String>,
}

impl MintTracker {
    fn observe(&mut self, venue: Venue, pool_id: &str) {
        if !self.venues.contains(&venue) {
            self.venues.push(venue);
        }

        self.pool_ids.insert(pool_id.to_owned());
    }
}

#[derive(Debug)]
struct RegisteredPool {
    pool: NormalizedPoolState,
    readiness: Option<QuoteReadiness>,
}

#[derive(Debug, Default)]
pub struct ActiveMintRegistry {
    latest_pools: BTreeMap<String, RegisteredPool>,
}

impl ActiveMintRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(
        &mut self,
        pool: NormalizedPoolState,
        readiness: Option<QuoteReadiness>,
    ) -> Result<bool, String> {
        let key = pool_key(&pool);

        if let Some(existing) = self.latest_pools.get(&key) {
            if pool.source_slot < existing.pool.source_slot {
                return Ok(false);
            }
        }

        if let Some(readiness) = readiness.as_ref() {
            readiness.validate_for_pool(&pool)?;
        }

        self.latest_pools
            .insert(key, RegisteredPool { pool, readiness });

        Ok(true)
    }

    pub fn current_pool_count(&self) -> usize {
        self.latest_pools.len()
    }

    pub fn current_eligible_pools(&self) -> Vec<NormalizedPoolState> {
        self.latest_pools
            .values()
            .filter(|registered| registered.readiness.is_some())
            .map(|registered| registered.pool.clone())
            .collect()
    }

    pub fn active_mints(&self) -> Vec<ActiveMint> {
        let mut trackers = BTreeMap::<String, MintTracker>::new();

        for registered in self
            .latest_pools
            .values()
            .filter(|registered| registered.readiness.is_some())
        {
            let pool = &registered.pool;

            observe_mint(&mut trackers, &pool.token_a.mint, pool);

            if pool.token_b.mint != pool.token_a.mint {
                observe_mint(&mut trackers, &pool.token_b.mint, pool);
            }
        }

        trackers
            .into_iter()
            .filter_map(|(mint, mut tracker)| {
                if tracker.venues.len() < 2 {
                    return None;
                }

                tracker.venues.sort_by_key(|venue| venue.label());

                Some(ActiveMint {
                    mint,
                    venues: tracker.venues,
                    pool_ids: tracker.pool_ids.into_iter().collect(),
                })
            })
            .collect()
    }
}

fn pool_key(pool: &NormalizedPoolState) -> String {
    format!("{}:{}", pool.venue.label(), pool.pool_id)
}

fn observe_mint(
    trackers: &mut BTreeMap<String, MintTracker>,
    mint: &str,
    pool: &NormalizedPoolState,
) {
    trackers
        .entry(mint.to_owned())
        .or_default()
        .observe(pool.venue, &pool.pool_id);
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_core::{
        AdapterCapabilities, AuxiliaryStateKind, CapabilityState, ContentionFootprintState,
        LiquidityModel, NormalizedToken, PoolTradingState, QuoteReserveState,
    };

    const SHARED_MINT: &str = "SharedMint111111111111111111111111111111";
    const OTHER_MINT: &str = "OtherMint1111111111111111111111111111111";
    const THIRD_MINT: &str = "ThirdMint1111111111111111111111111111111";

    fn sample_pool(
        venue: Venue,
        pool_id: &str,
        source_slot: u64,
        trading_state: PoolTradingState,
    ) -> NormalizedPoolState {
        sample_pool_with_pair(
            venue,
            pool_id,
            source_slot,
            trading_state,
            SHARED_MINT,
            OTHER_MINT,
        )
    }

    fn sample_pool_with_pair(
        venue: Venue,
        pool_id: &str,
        source_slot: u64,
        trading_state: PoolTradingState,
        token_a_mint: &str,
        token_b_mint: &str,
    ) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-program", venue.label()),
            source_slot,
            token_a: NormalizedToken {
                mint: token_a_mint.to_owned(),
                vault: format!("{pool_id}-vault-a"),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: token_b_mint.to_owned(),
                vault: format!("{pool_id}-vault-b"),
                decimals: 6,
            },
            trading_state,
            quote_reserves: QuoteReserveState::Available {
                token_a_raw: 100,
                token_b_raw: 200,
                source_slot,
            },
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    fn cpmm_capabilities() -> AdapterCapabilities {
        AdapterCapabilities {
            liquidity_model: LiquidityModel::Cpmm,
            exact_input_quote: CapabilityState::Supported,
            spl_token: CapabilityState::Supported,
            token_2022: CapabilityState::RequiresHydration,
            transfer_fee: CapabilityState::RequiresHydration,
            auxiliary_state: AuxiliaryStateKind::None,
            contention_footprint: ContentionFootprintState::Complete,
        }
    }

    fn readiness(pool: &NormalizedPoolState) -> QuoteReadiness {
        QuoteReadiness::synthetic_for_test(pool, cpmm_capabilities())
    }

    #[test]
    fn two_independent_venues_activate_same_mint() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let raydium = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
        );
        let pumpswap = sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::Tradable,
        );

        registry.upsert(raydium.clone(), Some(readiness(&raydium)))?;
        registry.upsert(pumpswap.clone(), Some(readiness(&pumpswap)))?;

        let active = registry.active_mints();

        assert!(active.iter().any(|mint| mint.mint == SHARED_MINT));
        Ok(())
    }

    #[test]
    fn multiple_pools_from_one_venue_do_not_activate_mint() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let pool_a = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool-a",
            10,
            PoolTradingState::Tradable,
        );
        let pool_b = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool-b",
            11,
            PoolTradingState::Tradable,
        );

        registry.upsert(pool_a.clone(), Some(readiness(&pool_a)))?;
        registry.upsert(pool_b.clone(), Some(readiness(&pool_b)))?;

        assert!(registry.active_mints().is_empty());
        Ok(())
    }

    #[test]
    fn explicit_readiness_is_required_for_eligibility() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let raydium = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
        );
        let pumpswap = sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::Tradable,
        );

        registry.upsert(raydium.clone(), Some(readiness(&raydium)))?;
        registry.upsert(pumpswap, None)?;

        assert!(registry.active_mints().is_empty());

        let eligible = registry.current_eligible_pools();
        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].pool_id, "raydium-pool");

        Ok(())
    }

    #[test]
    fn stale_update_cannot_regress_current_pool_state() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let raydium = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
        );
        let pumpswap = sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            21,
            PoolTradingState::Tradable,
        );

        registry.upsert(raydium.clone(), Some(readiness(&raydium)))?;
        registry.upsert(pumpswap.clone(), Some(readiness(&pumpswap)))?;

        let stale = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            19,
            PoolTradingState::SwapDisabled,
        );

        assert!(!registry.upsert(stale, None)?);
        assert!(!registry.active_mints().is_empty());

        Ok(())
    }

    #[test]
    fn newer_state_without_readiness_removes_previous_eligibility() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let raydium = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
        );
        let pumpswap = sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            21,
            PoolTradingState::Tradable,
        );

        registry.upsert(raydium.clone(), Some(readiness(&raydium)))?;
        registry.upsert(pumpswap.clone(), Some(readiness(&pumpswap)))?;

        assert!(!registry.active_mints().is_empty());

        let newer_raydium = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            22,
            PoolTradingState::Tradable,
        );

        registry.upsert(newer_raydium, None)?;

        assert!(registry.active_mints().is_empty());
        assert_eq!(registry.current_eligible_pools().len(), 1);

        Ok(())
    }

    #[test]
    fn readiness_venue_mismatch_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let target = sample_pool(
            Venue::RaydiumCpmm,
            "shared-pool",
            20,
            PoolTradingState::Tradable,
        );
        let source = sample_pool(
            Venue::PumpSwap,
            "shared-pool",
            20,
            PoolTradingState::Tradable,
        );
        let mismatched = readiness(&source);

        let result = registry.upsert(target, Some(mismatched));

        assert!(matches!(
            result,
            Err(error) if error.contains("quote readiness venue mismatch")
        ));
    }

    #[test]
    fn readiness_pool_mismatch_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let target = sample_pool(
            Venue::RaydiumCpmm,
            "target-pool",
            20,
            PoolTradingState::Tradable,
        );
        let source = sample_pool(
            Venue::RaydiumCpmm,
            "other-pool",
            20,
            PoolTradingState::Tradable,
        );
        let mismatched = readiness(&source);

        let result = registry.upsert(target, Some(mismatched));

        assert!(matches!(
            result,
            Err(error) if error.contains("quote readiness pool mismatch")
        ));
    }

    #[test]
    fn readiness_token_pair_mismatch_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let target = sample_pool_with_pair(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
            SHARED_MINT,
            OTHER_MINT,
        );
        let source = sample_pool_with_pair(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
            SHARED_MINT,
            THIRD_MINT,
        );
        let mismatched = readiness(&source);

        let result = registry.upsert(target, Some(mismatched));

        assert!(matches!(
            result,
            Err(error) if error.contains("quote readiness token pair mismatch")
        ));
    }

    #[test]
    fn stale_readiness_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let target = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
        );
        let stale_source = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            19,
            PoolTradingState::Tradable,
        );
        let stale = readiness(&stale_source);

        let result = registry.upsert(target, Some(stale));

        assert!(matches!(
            result,
            Err(error) if error.contains("stale quote readiness")
        ));
    }

    #[test]
    fn unsupported_exact_input_readiness_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let pool = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
        );
        let unsupported = QuoteReadiness::synthetic_for_test(
            &pool,
            AdapterCapabilities {
                liquidity_model: LiquidityModel::Cpmm,
                exact_input_quote: CapabilityState::Unsupported,
                spl_token: CapabilityState::Supported,
                token_2022: CapabilityState::RequiresHydration,
                transfer_fee: CapabilityState::RequiresHydration,
                auxiliary_state: AuxiliaryStateKind::None,
                contention_footprint: ContentionFootprintState::Complete,
            },
        );

        let result = registry.upsert(pool, Some(unsupported));

        assert!(matches!(
            result,
            Err(error) if error.contains("does not prove exact-input support")
        ));
    }

    #[test]
    fn nontradable_pool_with_readiness_is_rejected() {
        let mut registry = ActiveMintRegistry::new();

        let pool = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::SwapDisabled,
        );
        let synthetic = readiness(&pool);

        let result = registry.upsert(pool, Some(synthetic));

        assert!(matches!(
            result,
            Err(error) if error.contains("is not tradable")
        ));
    }

    #[test]
    fn synthetic_clmm_ticks_readiness_is_eligible_without_cpmm_reserves() -> Result<(), String> {
        let mut registry = ActiveMintRegistry::new();

        let mut pool = sample_pool(
            Venue::Orca,
            "orca-pool",
            20,
            PoolTradingState::Tradable,
        );
        pool.quote_reserves = QuoteReserveState::Unavailable;

        let readiness = QuoteReadiness::synthetic_for_test(
            &pool,
            AdapterCapabilities {
                liquidity_model: LiquidityModel::Clmm,
                exact_input_quote: CapabilityState::Supported,
                spl_token: CapabilityState::Supported,
                token_2022: CapabilityState::RequiresHydration,
                transfer_fee: CapabilityState::RequiresHydration,
                auxiliary_state: AuxiliaryStateKind::Ticks,
                contention_footprint: ContentionFootprintState::Incomplete,
            },
        );

        registry.upsert(pool, Some(readiness))?;

        let eligible = registry.current_eligible_pools();

        assert_eq!(eligible.len(), 1);
        assert_eq!(eligible[0].venue, Venue::Orca);
        assert_eq!(eligible[0].pool_id, "orca-pool");

        Ok(())
    }
}
