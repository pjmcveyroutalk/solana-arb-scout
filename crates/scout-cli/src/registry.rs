use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState, Venue};
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

#[derive(Debug, Default)]
pub struct ActiveMintRegistry {
    latest_pools: BTreeMap<String, NormalizedPoolState>,
}

impl ActiveMintRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn upsert(&mut self, pool: NormalizedPoolState) -> bool {
        let key = pool_key(&pool);

        if let Some(existing) = self.latest_pools.get(&key) {
            if pool.source_slot < existing.source_slot {
                return false;
            }
        }

        self.latest_pools.insert(key, pool);
        true
    }

    pub fn current_pool_count(&self) -> usize {
        self.latest_pools.len()
    }

    pub fn active_mints(&self) -> Vec<ActiveMint> {
        let mut trackers = BTreeMap::<String, MintTracker>::new();

        for pool in self.latest_pools.values().filter(|pool| is_eligible(pool)) {
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

fn is_eligible(pool: &NormalizedPoolState) -> bool {
    if pool.trading_state != PoolTradingState::Tradable {
        return false;
    }

    matches!(
        &pool.quote_reserves,
        QuoteReserveState::Available {
            token_a_raw,
            token_b_raw,
            ..
        } if *token_a_raw > 0 && *token_b_raw > 0
    )
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
    use scout_core::NormalizedToken;

    const SHARED_MINT: &str = "SharedMint111111111111111111111111111111";
    const OTHER_MINT: &str = "OtherMint1111111111111111111111111111111";

    fn sample_pool(
        venue: Venue,
        pool_id: &str,
        source_slot: u64,
        trading_state: PoolTradingState,
        token_a_raw: u64,
        token_b_raw: u64,
    ) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-program", venue.label()),
            source_slot,
            token_a: NormalizedToken {
                mint: SHARED_MINT.to_owned(),
                vault: format!("{pool_id}-vault-a"),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: OTHER_MINT.to_owned(),
                vault: format!("{pool_id}-vault-b"),
                decimals: 6,
            },
            trading_state,
            quote_reserves: QuoteReserveState::Available {
                token_a_raw,
                token_b_raw,
                source_slot,
            },
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    #[test]
    fn two_independent_venues_activate_same_mint() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::Tradable,
            300,
            400,
        ));

        let active = registry.active_mints();

        assert!(active.iter().any(|mint| mint.mint == SHARED_MINT));
    }

    #[test]
    fn multiple_pools_from_one_venue_do_not_activate_mint() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool-a",
            10,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool-b",
            11,
            PoolTradingState::Tradable,
            300,
            400,
        ));

        assert!(registry.active_mints().is_empty());
    }

    #[test]
    fn unavailable_reserves_do_not_qualify() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        let mut pumpswap = sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::Tradable,
            300,
            400,
        );
        pumpswap.quote_reserves = QuoteReserveState::Unavailable;

        registry.upsert(pumpswap);

        assert!(registry.active_mints().is_empty());
    }

    #[test]
    fn non_tradable_pool_does_not_qualify() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::SwapDisabled,
            300,
            400,
        ));

        assert!(registry.active_mints().is_empty());
    }

    #[test]
    fn zero_reserve_pool_does_not_qualify() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            10,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            11,
            PoolTradingState::Tradable,
            0,
            400,
        ));

        assert!(registry.active_mints().is_empty());
    }

    #[test]
    fn stale_update_cannot_regress_current_pool_state() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            21,
            PoolTradingState::Tradable,
            300,
            400,
        ));

        let stale = sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            19,
            PoolTradingState::SwapDisabled,
            100,
            200,
        );

        assert!(!registry.upsert(stale));
        assert!(!registry.active_mints().is_empty());
    }

    #[test]
    fn newer_state_can_remove_previous_eligibility() {
        let mut registry = ActiveMintRegistry::new();

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            20,
            PoolTradingState::Tradable,
            100,
            200,
        ));

        registry.upsert(sample_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            21,
            PoolTradingState::Tradable,
            300,
            400,
        ));

        assert!(!registry.active_mints().is_empty());

        registry.upsert(sample_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            22,
            PoolTradingState::SwapDisabled,
            100,
            200,
        ));

        assert!(registry.active_mints().is_empty());
    }
}
