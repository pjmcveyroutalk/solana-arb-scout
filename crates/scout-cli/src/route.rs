use scout_core::{NormalizedPoolState, Venue};
use std::collections::BTreeSet;

pub const WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
pub const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
pub const USDT_MINT: &str = "Es9vMFrzaCERmJfrF4H2FYD4KCoNkY11McCe8BenwNYB";

const ROUTE_ANCHORS: [&str; 3] = [WRAPPED_SOL_MINT, USDC_MINT, USDT_MINT];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteLeg {
    venue: Venue,
    pool_id: String,
    input_mint: String,
    output_mint: String,
    source_slot: u64,
}

impl RouteLeg {
    fn new(
        pool: &NormalizedPoolState,
        input_mint: &str,
        output_mint: &str,
    ) -> Self {
        Self {
            venue: pool.venue,
            pool_id: pool.pool_id.clone(),
            input_mint: input_mint.to_owned(),
            output_mint: output_mint.to_owned(),
            source_slot: pool.source_slot,
        }
    }

    fn summary(&self) -> String {
        format!(
            "venue={} pool={} input={} output={} slot={}",
            self.venue.label(),
            self.pool_id,
            self.input_mint,
            self.output_mint,
            self.source_slot,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoLegRouteCandidate {
    anchor_mint: String,
    intermediate_mint: String,
    leg_1: RouteLeg,
    leg_2: RouteLeg,
}

impl TwoLegRouteCandidate {
    pub fn summary(&self) -> String {
        format!(
            "anchor={} intermediate={} leg1=[{}] leg2=[{}]",
            self.anchor_mint,
            self.intermediate_mint,
            self.leg_1.summary(),
            self.leg_2.summary(),
        )
    }

    fn key(&self) -> String {
        format!(
            "{}:{}:{}:{}:{}:{}",
            self.anchor_mint,
            self.intermediate_mint,
            self.leg_1.venue.label(),
            self.leg_1.pool_id,
            self.leg_2.venue.label(),
            self.leg_2.pool_id,
        )
    }
}

pub fn generate_two_leg_routes(
    pools: &[NormalizedPoolState],
) -> Vec<TwoLegRouteCandidate> {
    let mut routes = Vec::new();
    let mut seen = BTreeSet::new();

    for left_index in 0..pools.len() {
        for right_index in (left_index + 1)..pools.len() {
            let left = &pools[left_index];
            let right = &pools[right_index];

            if left.venue == right.venue {
                continue;
            }

            if left.pool_id == right.pool_id {
                continue;
            }

            for anchor_mint in ROUTE_ANCHORS {
                let Some(left_intermediate) = counter_mint(left, anchor_mint) else {
                    continue;
                };

                let Some(right_intermediate) = counter_mint(right, anchor_mint) else {
                    continue;
                };

                if left_intermediate != right_intermediate {
                    continue;
                }

                add_candidate(
                    &mut routes,
                    &mut seen,
                    anchor_mint,
                    left_intermediate,
                    left,
                    right,
                );

                add_candidate(
                    &mut routes,
                    &mut seen,
                    anchor_mint,
                    left_intermediate,
                    right,
                    left,
                );
            }
        }
    }

    routes
}

fn add_candidate(
    routes: &mut Vec<TwoLegRouteCandidate>,
    seen: &mut BTreeSet<String>,
    anchor_mint: &str,
    intermediate_mint: &str,
    first_pool: &NormalizedPoolState,
    second_pool: &NormalizedPoolState,
) {
    let candidate = TwoLegRouteCandidate {
        anchor_mint: anchor_mint.to_owned(),
        intermediate_mint: intermediate_mint.to_owned(),
        leg_1: RouteLeg::new(first_pool, anchor_mint, intermediate_mint),
        leg_2: RouteLeg::new(second_pool, intermediate_mint, anchor_mint),
    };

    if seen.insert(candidate.key()) {
        routes.push(candidate);
    }
}

fn counter_mint<'a>(
    pool: &'a NormalizedPoolState,
    anchor_mint: &str,
) -> Option<&'a str> {
    let token_a_is_anchor = pool.token_a.mint == anchor_mint;
    let token_b_is_anchor = pool.token_b.mint == anchor_mint;

    match (token_a_is_anchor, token_b_is_anchor) {
        (true, false) => Some(pool.token_b.mint.as_str()),
        (false, true) => Some(pool.token_a.mint.as_str()),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_core::{
        NormalizedToken, PoolTradingState, QuoteReserveState,
    };

    const TEST_TOKEN: &str = "Token1111111111111111111111111111111111111";
    const OTHER_TOKEN: &str = "Other1111111111111111111111111111111111111";

    fn sample_pool(
        venue: Venue,
        pool_id: &str,
        token_a_mint: &str,
        token_b_mint: &str,
    ) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-program", venue.label()),
            source_slot: 100,
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
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Available {
                token_a_raw: 1_000,
                token_b_raw: 2_000,
                source_slot: 100,
            },
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    #[test]
    fn valid_cross_venue_pair_generates_both_directions() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
        ];

        let routes = generate_two_leg_routes(&pools);

        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].anchor_mint, WRAPPED_SOL_MINT);
        assert_eq!(routes[0].intermediate_mint, TEST_TOKEN);
    }

    #[test]
    fn same_venue_pools_do_not_form_route() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-a",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-b",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
        ];

        assert!(generate_two_leg_routes(&pools).is_empty());
    }

    #[test]
    fn shared_anchor_with_different_counter_mints_is_not_route() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                WRAPPED_SOL_MINT,
                OTHER_TOKEN,
            ),
        ];

        assert!(generate_two_leg_routes(&pools).is_empty());
    }

    #[test]
    fn unsupported_anchor_does_not_form_route() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                OTHER_TOKEN,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                OTHER_TOKEN,
                TEST_TOKEN,
            ),
        ];

        assert!(generate_two_leg_routes(&pools).is_empty());
    }

    #[test]
    fn reversed_pool_orientation_still_matches() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                TEST_TOKEN,
                WRAPPED_SOL_MINT,
            ),
        ];

        let routes = generate_two_leg_routes(&pools);

        assert_eq!(routes.len(), 2);
    }

    #[test]
    fn usdc_anchor_is_permitted() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                USDC_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                TEST_TOKEN,
                USDC_MINT,
            ),
        ];

        let routes = generate_two_leg_routes(&pools);

        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].anchor_mint, USDC_MINT);
    }

    #[test]
    fn usdt_anchor_is_permitted() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                USDT_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                TEST_TOKEN,
                USDT_MINT,
            ),
        ];

        let routes = generate_two_leg_routes(&pools);

        assert_eq!(routes.len(), 2);
        assert_eq!(routes[0].anchor_mint, USDT_MINT);
    }

    #[test]
    fn generated_routes_have_no_duplicate_structural_keys() {
        let pools = vec![
            sample_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
            sample_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                WRAPPED_SOL_MINT,
                TEST_TOKEN,
            ),
        ];

        let routes = generate_two_leg_routes(&pools);
        let keys = routes
            .iter()
            .map(TwoLegRouteCandidate::key)
            .collect::<BTreeSet<_>>();

        assert_eq!(keys.len(), routes.len());
    }
}
