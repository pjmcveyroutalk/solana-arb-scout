use crate::orca_live;
use crate::pumpswap;
use crate::quote::{
    quote_readiness_for_pool, quote_two_leg_exact_input, ExactInputQuoteAdapter, QuoteReadiness,
    TwoLegRouteQuote, VenueQuoteContext,
};
use crate::raydium;
use crate::route::{RouteLeg, TwoLegRouteCandidate};
use scout_cli::meteora::MeteoraDlmmSnapshot;
use scout_cli::meteora_m13::MeteoraM13PreparedQuote;
use scout_core::{NormalizedPoolState, Venue};
use std::collections::BTreeMap;

#[derive(Debug)]
pub struct MeteoraRuntimeQuoteState {
    pub normalized: NormalizedPoolState,
    pub snapshot: MeteoraDlmmSnapshot,
}

pub fn readiness_for_pool(
    pool: &NormalizedPoolState,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
) -> Option<QuoteReadiness> {
    let meteora_runtime = BTreeMap::new();

    readiness_for_pool_with_meteora(
        pool,
        raydium_quote_contexts,
        pumpswap_quote_contexts,
        orca_prepared,
        &meteora_runtime,
    )
}

pub fn readiness_for_pool_with_meteora(
    pool: &NormalizedPoolState,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
    meteora_runtime: &BTreeMap<String, MeteoraRuntimeQuoteState>,
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
            let runtime = match meteora_runtime.get(&pool.pool_id) {
                Some(runtime) => runtime,
                None => {
                    log_missing(pool, "missing prepared Meteora runtime state");
                    return None;
                }
            };

            let prepared = match MeteoraM13PreparedQuote::from_snapshot(pool, &runtime.snapshot) {
                Ok(prepared) => prepared,
                Err(error) => {
                    log_missing(pool, &error);
                    return None;
                }
            };

            return match QuoteReadiness::from_validated_source(
                pool,
                prepared.source_slot(),
                prepared.capabilities(),
            ) {
                Ok(readiness) => Some(readiness),
                Err(error) => {
                    log_missing(pool, &error);
                    None
                }
            };
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
    let meteora_runtime = BTreeMap::new();

    quote_route_exact_input_with_meteora(
        route,
        amount_in_raw,
        raydium_quote_contexts,
        pumpswap_quote_contexts,
        orca_prepared,
        &meteora_runtime,
    )
}

pub fn quote_route_exact_input_with_meteora(
    route: &TwoLegRouteCandidate,
    amount_in_raw: u64,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, orca_live::PreparedOrca>,
    meteora_runtime: &BTreeMap<String, MeteoraRuntimeQuoteState>,
) -> Result<TwoLegRouteQuote, String> {
    with_leg_adapter(
        route.leg_1(),
        raydium_quote_contexts,
        pumpswap_quote_contexts,
        orca_prepared,
        meteora_runtime,
        |leg_1_adapter| {
            with_leg_adapter(
                route.leg_2(),
                raydium_quote_contexts,
                pumpswap_quote_contexts,
                orca_prepared,
                meteora_runtime,
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
    meteora_runtime: &BTreeMap<String, MeteoraRuntimeQuoteState>,
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
        Venue::Meteora => {
            let runtime = meteora_runtime.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing prepared Meteora runtime state for route pool {}",
                    leg.pool_id()
                )
            })?;
            let prepared =
                MeteoraM13PreparedQuote::from_snapshot(&runtime.normalized, &runtime.snapshot)?;
            operation(&prepared)
        }
    }
}

fn log_missing(pool: &NormalizedPoolState, reason: &str) {
    println!(
        "quote_readiness_unavailable: venue={} pool={} reason={reason}",
        pool.venue.label(),
        pool.pool_id
    );
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{generate_two_leg_routes, WRAPPED_SOL_MINT};
    use scout_cli::meteora::{
        DlmmProtocolProfile, MeteoraClockSnapshot, MeteoraLbPairState, MeteoraSnapshotSource,
    };
    use scout_core::{
        AuxiliaryStateKind, CapabilityState, LiquidityModel, NormalizedToken, PoolTradingState,
        QuoteReserveState,
    };

    const SOURCE_SLOT: u64 = 100;
    const GENERATION_ID: u64 = 7;

    fn sample_meteora_runtime(
    ) -> Result<(NormalizedPoolState, MeteoraRuntimeQuoteState, String), String> {
        let lb_pair_pubkey = [9_u8; 32];
        let mint_x_vec = bs58::decode(WRAPPED_SOL_MINT)
            .into_vec()
            .map_err(|error| format!("could not decode WSOL mint for test: {error}"))?;
        let mint_x: [u8; 32] = mint_x_vec
            .try_into()
            .map_err(|_| "WSOL mint did not decode to 32 bytes".to_owned())?;
        let mint_y = [7_u8; 32];
        let intermediate_mint = bs58::encode(mint_y).into_string();

        let lb_pair = MeteoraLbPairState {
            base_factor: 1,
            filter_period: 1,
            decay_period: 2,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: 0,
            last_update_timestamp: 0,
            active_id: 0,
            bin_step: 1,
            mint_x,
            mint_y,
            bin_array_bitmap: [0; 16],
        };

        let snapshot = MeteoraDlmmSnapshot::new(
            lb_pair_pubkey,
            lb_pair,
            Vec::new(),
            None,
            MeteoraClockSnapshot {
                slot: SOURCE_SLOT,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            DlmmProtocolProfile::V0_12,
            MeteoraSnapshotSource {
                source_slot: SOURCE_SLOT,
                generation_id: GENERATION_ID,
            },
        )
        .map_err(|error| format!("could not build Meteora test snapshot: {error:?}"))?;

        let pool_id = bs58::encode(lb_pair_pubkey).into_string();
        let normalized = NormalizedPoolState {
            pool_id,
            venue: Venue::Meteora,
            program_id: "meteora-test-program".to_owned(),
            source_slot: SOURCE_SLOT,
            token_a: NormalizedToken {
                mint: WRAPPED_SOL_MINT.to_owned(),
                vault: "meteora-vault-a".to_owned(),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: intermediate_mint.clone(),
                vault: "meteora-vault-b".to_owned(),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Unavailable,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        };

        let runtime = MeteoraRuntimeQuoteState {
            normalized: normalized.clone(),
            snapshot,
        };

        Ok((normalized, runtime, intermediate_mint))
    }

    fn sample_counterpart(
        venue: Venue,
        pool_id: &str,
        intermediate_mint: &str,
    ) -> NormalizedPoolState {
        let quote_reserves = match venue {
            Venue::RaydiumCpmm | Venue::PumpSwap => QuoteReserveState::Available {
                token_a_raw: 1_000,
                token_b_raw: 2_000,
                source_slot: SOURCE_SLOT,
            },
            Venue::Orca | Venue::Meteora => QuoteReserveState::Unavailable,
        };

        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-test-program", venue.label()),
            source_slot: SOURCE_SLOT,
            token_a: NormalizedToken {
                mint: WRAPPED_SOL_MINT.to_owned(),
                vault: format!("{pool_id}-vault-a"),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: intermediate_mint.to_owned(),
                vault: format!("{pool_id}-vault-b"),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    #[test]
    fn meteora_readiness_accepts_valid_prepared_runtime_state() -> Result<(), String> {
        let (pool, runtime, _) = sample_meteora_runtime()?;
        let raydium = BTreeMap::new();
        let pumpswap = BTreeMap::new();
        let orca = BTreeMap::new();
        let meteora = BTreeMap::from([(pool.pool_id.clone(), runtime)]);

        let readiness =
            readiness_for_pool_with_meteora(&pool, &raydium, &pumpswap, &orca, &meteora)
                .ok_or_else(|| "Meteora readiness unexpectedly unavailable".to_owned())?;

        readiness.validate_for_pool(&pool)
    }

    #[test]
    fn meteora_dispatch_is_pairing_independent_across_all_supported_counterparts(
    ) -> Result<(), String> {
        let counterparts = [
            (Venue::RaydiumCpmm, "raydium-pool"),
            (Venue::PumpSwap, "pumpswap-pool"),
            (Venue::Orca, "orca-pool"),
        ];

        for (counterpart_venue, counterpart_pool_id) in counterparts {
            let (meteora_pool, runtime, intermediate_mint) = sample_meteora_runtime()?;
            let counterpart =
                sample_counterpart(counterpart_venue, counterpart_pool_id, &intermediate_mint);
            let routes = generate_two_leg_routes(&[meteora_pool.clone(), counterpart]);

            assert_eq!(routes.len(), 2);

            let raydium = BTreeMap::new();
            let pumpswap = BTreeMap::new();
            let orca = BTreeMap::new();
            let meteora = BTreeMap::from([(meteora_pool.pool_id.clone(), runtime)]);

            for route in &routes {
                let meteora_leg = if route.leg_1().venue() == Venue::Meteora {
                    route.leg_1()
                } else {
                    route.leg_2()
                };

                with_leg_adapter(
                    meteora_leg,
                    &raydium,
                    &pumpswap,
                    &orca,
                    &meteora,
                    |adapter| {
                        assert_eq!(adapter.venue(), Venue::Meteora);
                        assert_eq!(adapter.pool_id(), meteora_pool.pool_id.as_str());
                        assert_eq!(adapter.source_slot(), SOURCE_SLOT);
                        assert!(adapter.contains_pair(WRAPPED_SOL_MINT, intermediate_mint.as_str()));

                        let capabilities = adapter.capabilities();
                        assert_eq!(capabilities.liquidity_model, LiquidityModel::Dlmm);
                        assert_eq!(capabilities.exact_input_quote, CapabilityState::Supported);
                        assert_eq!(capabilities.auxiliary_state, AuxiliaryStateKind::Bins);

                        Ok(())
                    },
                )?;
            }
        }

        Ok(())
    }

    #[test]
    fn meteora_dispatch_without_runtime_state_fails_closed() -> Result<(), String> {
        let (meteora_pool, _, intermediate_mint) = sample_meteora_runtime()?;
        let counterpart =
            sample_counterpart(Venue::RaydiumCpmm, "raydium-pool", &intermediate_mint);
        let routes = generate_two_leg_routes(&[meteora_pool, counterpart]);
        let meteora_leg = routes
            .iter()
            .flat_map(|route| [route.leg_1(), route.leg_2()])
            .find(|leg| leg.venue() == Venue::Meteora)
            .ok_or_else(|| "Meteora leg missing from deterministic route".to_owned())?;

        let result = with_leg_adapter(
            meteora_leg,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            |_| Ok(()),
        );

        assert!(matches!(
            result,
            Err(error) if error.contains("missing prepared Meteora runtime state")
        ));

        Ok(())
    }
}

