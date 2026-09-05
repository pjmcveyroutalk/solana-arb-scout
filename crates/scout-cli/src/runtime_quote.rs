use crate::orca_live;
use crate::pumpswap;
use crate::quote::{
    quote_readiness_for_pool, quote_two_leg_exact_input, ExactInputQuoteAdapter, QuoteReadiness,
    TwoLegRouteQuote, VenueQuoteContext,
};
use crate::raydium;
use crate::route::{RouteLeg, TwoLegRouteCandidate};
use scout_core::{NormalizedPoolState, Venue};
use std::collections::BTreeMap;

pub fn readiness_for_pool(
    pool: &NormalizedPoolState,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
) -> Option<QuoteReadiness> {
    let result = match pool.venue {
        Venue::RaydiumCpmm => {
            let snapshot = match raydium_quote_contexts.get(&pool.pool_id) {
                Some(snapshot) => snapshot,
                None => {
                    log_missing(pool, "missing quote context");
                    return None;
                }
            };

            let context = VenueQuoteContext::Raydium {
                pool_id: pool.pool_id.clone(),
                snapshot,
            };

            quote_readiness_for_pool(pool, &context)
        }
        Venue::PumpSwap => {
            let snapshot = match pumpswap_quote_contexts.get(&pool.pool_id) {
                Some(snapshot) => snapshot,
                None => {
                    log_missing(pool, "missing quote context");
                    return None;
                }
            };

            let context = VenueQuoteContext::PumpSwap {
                pool_id: pool.pool_id.clone(),
                snapshot,
            };

            quote_readiness_for_pool(pool, &context)
        }
        Venue::Orca => {
            let prepared = match orca_prepared.get(&pool.pool_id) {
                Some(prepared) => prepared,
                None => {
                    log_missing(pool, "missing prepared Orca snapshot");
                    return None;
                }
            };

            return match prepared.readiness.validate_for_pool(pool) {
                Ok(()) => Some(prepared.readiness.clone()),
                Err(error) => {
                    log_missing(pool, &error);
                    None
                }
            };
        }
        Venue::Meteora => {
            log_missing(pool, "Meteora runtime quote path is not enabled");
            return None;
        }
    };

    match result {
        Ok(readiness) => Some(readiness),
        Err(error) => {
            log_missing(pool, &error);
            None
        }
    }
}

pub fn quote_route_exact_input(
    route: &TwoLegRouteCandidate,
    amount_in_raw: u64,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
) -> Result<TwoLegRouteQuote, String> {
    with_leg_adapter(
        route.leg_1(),
        raydium_quote_contexts,
        pumpswap_quote_contexts,
        orca_prepared,
        |leg_1_adapter| {
            with_leg_adapter(
                route.leg_2(),
                raydium_quote_contexts,
                pumpswap_quote_contexts,
                orca_prepared,
                |leg_2_adapter| {
                    quote_two_leg_exact_input(route, amount_in_raw, leg_1_adapter, leg_2_adapter)
                },
            )
        },
    )
}

pub fn normalized_mint_decimals_for_leg(
    leg: &RouteLeg,
    mint: &str,
    eligible_pools: &[NormalizedPoolState],
) -> Result<u8, String> {
    let pool = eligible_pools
        .iter()
        .find(|pool| pool.venue == leg.venue() && pool.pool_id == leg.pool_id())
        .ok_or_else(|| {
            format!(
                "missing normalized pool for route venue={} pool={}",
                leg.venue().label(),
                leg.pool_id()
            )
        })?;

    if pool.token_a.mint == mint {
        Ok(pool.token_a.decimals)
    } else if pool.token_b.mint == mint {
        Ok(pool.token_b.decimals)
    } else {
        Err(format!(
            "mint {mint} is not in normalized route pool {}",
            leg.pool_id()
        ))
    }
}

fn with_leg_adapter<T>(
    leg: &RouteLeg,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
    operation: impl FnOnce(&dyn ExactInputQuoteAdapter) -> Result<T, String>,
) -> Result<T, String> {
    match leg.venue() {
        Venue::RaydiumCpmm => {
            let snapshot = raydium_quote_contexts.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing Raydium quote context for route pool {}",
                    leg.pool_id()
                )
            })?;
            let context = VenueQuoteContext::Raydium {
                pool_id: leg.pool_id().to_owned(),
                snapshot,
            };
            operation(&context)
        }
        Venue::PumpSwap => {
            let snapshot = pumpswap_quote_contexts.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing PumpSwap quote context for route pool {}",
                    leg.pool_id()
                )
            })?;
            let context = VenueQuoteContext::PumpSwap {
                pool_id: leg.pool_id().to_owned(),
                snapshot,
            };
            operation(&context)
        }
        Venue::Orca => {
            let prepared = orca_prepared.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing prepared Orca quote snapshot for route pool {}",
                    leg.pool_id()
                )
            })?;
            operation(&prepared.quote_snapshot)
        }
        Venue::Meteora => Err(format!(
            "Meteora runtime quote path is not enabled for route pool {}",
            leg.pool_id()
        )),
    }
}

fn log_missing(pool: &NormalizedPoolState, reason: &str) {
    println!(
        "quote_readiness_unavailable: venue={} pool={} reason={reason}",
        pool.venue.label(),
        pool.pool_id
    );
}

