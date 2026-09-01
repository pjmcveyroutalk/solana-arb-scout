use crate::pumpswap::{self, PumpSwapHydrationSnapshot};
use crate::raydium::{self, RaydiumHydrationSnapshot};
use crate::route::{RouteLeg, TwoLegRouteCandidate};
use scout_core::Venue;

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum VenueFeeComponents {
    RaydiumCpmm {
        trade_fee_raw: u64,
        protocol_fee_raw: u64,
        fund_fee_raw: u64,
        creator_fee_raw: u64,
    },
    PumpSwap {
        lp_fee_raw: u64,
        protocol_fee_raw: u64,
        creator_fee_raw: u64,
    },
}

impl VenueFeeComponents {
    fn summary(&self) -> String {
        match self {
            Self::RaydiumCpmm {
                trade_fee_raw,
                protocol_fee_raw,
                fund_fee_raw,
                creator_fee_raw,
            } => format!(
                "trade_fee_raw={trade_fee_raw} protocol_fee_raw={protocol_fee_raw} \
                 fund_fee_raw={fund_fee_raw} creator_fee_raw={creator_fee_raw}"
            ),
            Self::PumpSwap {
                lp_fee_raw,
                protocol_fee_raw,
                creator_fee_raw,
            } => format!(
                "lp_fee_raw={lp_fee_raw} protocol_fee_raw={protocol_fee_raw} \
                 creator_fee_raw={creator_fee_raw}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct VenueLegQuote {
    pub venue: Venue,
    pub pool_id: String,
    pub amount_in_requested_raw: u64,
    pub amount_in_consumed_raw: u64,
    pub amount_in_unspent_raw: u64,
    pub amount_out_raw: u64,
    pub fees: VenueFeeComponents,
    pub quote_source_slot: u64,
}

impl VenueLegQuote {
    fn summary(&self) -> String {
        format!(
            concat!(
                "venue={} pool={} requested_in_raw={} consumed_in_raw={} unspent_in_raw={} ",
                "out_raw={} fees=[{}] quote_slot={}"
            ),
            self.venue.label(),
            self.pool_id,
            self.amount_in_requested_raw,
            self.amount_in_consumed_raw,
            self.amount_in_unspent_raw,
            self.amount_out_raw,
            self.fees.summary(),
            self.quote_source_slot,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TwoLegRouteQuote {
    pub anchor_mint: String,
    pub intermediate_mint: String,
    pub anchor_input_requested_raw: u64,
    pub anchor_input_consumed_raw: u64,
    pub anchor_input_unspent_raw: u64,
    pub anchor_output_raw: u64,
    pub leg_1: VenueLegQuote,
    pub leg_2: VenueLegQuote,
}

impl TwoLegRouteQuote {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "anchor={} intermediate={} requested_anchor_in_raw={} ",
                "consumed_anchor_in_raw={} unspent_anchor_in_raw={} anchor_out_raw={} ",
                "leg1=[{}] leg2=[{}]"
            ),
            self.anchor_mint,
            self.intermediate_mint,
            self.anchor_input_requested_raw,
            self.anchor_input_consumed_raw,
            self.anchor_input_unspent_raw,
            self.anchor_output_raw,
            self.leg_1.summary(),
            self.leg_2.summary(),
        )
    }
}

pub enum VenueQuoteContext<'a> {
    Raydium {
        pool_id: String,
        snapshot: &'a RaydiumHydrationSnapshot,
    },
    PumpSwap {
        pool_id: String,
        snapshot: &'a PumpSwapHydrationSnapshot,
    },
}

pub fn one_whole_anchor_input_raw(
    route: &TwoLegRouteCandidate,
    leg_1_context: &VenueQuoteContext<'_>,
) -> Result<u64, String> {
    validate_context(route.leg_1(), leg_1_context)?;
    let decimals = context_mint_decimals(leg_1_context, route.anchor_mint())?;

    10u64
        .checked_pow(u32::from(decimals))
        .ok_or_else(|| format!("anchor decimals {decimals} exceed u64 whole-token sizing"))
}

pub fn quote_two_leg_exact_input(
    route: &TwoLegRouteCandidate,
    amount_in_raw: u64,
    leg_1_context: &VenueQuoteContext<'_>,
    leg_2_context: &VenueQuoteContext<'_>,
) -> Result<TwoLegRouteQuote, String> {
    if amount_in_raw == 0 {
        return Err("route quote input must be greater than zero".to_owned());
    }

    validate_context(route.leg_1(), leg_1_context)?;
    validate_context(route.leg_2(), leg_2_context)?;

    let leg_1 = quote_leg(route.leg_1(), amount_in_raw, leg_1_context)?;
    let leg_2 = quote_leg(route.leg_2(), leg_1.amount_out_raw, leg_2_context)?;

    if leg_2.amount_in_unspent_raw != 0 {
        return Err(format!(
            concat!(
                "second leg left non-anchor intermediate input unspent; ",
                "pool={} unspent_raw={}"
            ),
            leg_2.pool_id, leg_2.amount_in_unspent_raw
        ));
    }

    let anchor_output_raw = leg_2
        .amount_out_raw
        .checked_add(leg_1.amount_in_unspent_raw)
        .ok_or_else(|| "route anchor output overflow while restoring unspent input".to_owned())?;

    Ok(TwoLegRouteQuote {
        anchor_mint: route.anchor_mint().to_owned(),
        intermediate_mint: route.intermediate_mint().to_owned(),
        anchor_input_requested_raw: amount_in_raw,
        anchor_input_consumed_raw: leg_1.amount_in_consumed_raw,
        anchor_input_unspent_raw: leg_1.amount_in_unspent_raw,
        anchor_output_raw,
        leg_1,
        leg_2,
    })
}

fn quote_leg(
    leg: &RouteLeg,
    amount_in_raw: u64,
    context: &VenueQuoteContext<'_>,
) -> Result<VenueLegQuote, String> {
    match context {
        VenueQuoteContext::Raydium { pool_id, snapshot } => {
            let quote = raydium::quote_exact_input(snapshot, leg.input_mint(), amount_in_raw)?;

            Ok(VenueLegQuote {
                venue: Venue::RaydiumCpmm,
                pool_id: pool_id.clone(),
                amount_in_requested_raw: quote.amount_in_raw,
                amount_in_consumed_raw: quote.amount_in_raw,
                amount_in_unspent_raw: 0,
                amount_out_raw: quote.amount_out_raw,
                fees: VenueFeeComponents::RaydiumCpmm {
                    trade_fee_raw: quote.trade_fee_raw,
                    protocol_fee_raw: quote.protocol_fee_raw,
                    fund_fee_raw: quote.fund_fee_raw,
                    creator_fee_raw: quote.creator_fee_raw,
                },
                quote_source_slot: quote.source_slot,
            })
        }
        VenueQuoteContext::PumpSwap { pool_id, snapshot } => {
            let quote = pumpswap::quote_exact_input(snapshot, leg.input_mint(), amount_in_raw)?;

            Ok(VenueLegQuote {
                venue: Venue::PumpSwap,
                pool_id: pool_id.clone(),
                amount_in_requested_raw: quote.amount_in_requested_raw,
                amount_in_consumed_raw: quote.amount_in_consumed_raw,
                amount_in_unspent_raw: quote.amount_in_unspent_raw,
                amount_out_raw: quote.amount_out_raw,
                fees: VenueFeeComponents::PumpSwap {
                    lp_fee_raw: quote.lp_fee_raw,
                    protocol_fee_raw: quote.protocol_fee_raw,
                    creator_fee_raw: quote.creator_fee_raw,
                },
                quote_source_slot: quote.source_slot,
            })
        }
    }
}

fn validate_context(leg: &RouteLeg, context: &VenueQuoteContext<'_>) -> Result<(), String> {
    if leg.venue() != context_venue(context) {
        return Err(format!(
            "route/context venue mismatch: route={} context={}",
            leg.venue().label(),
            context_venue(context).label()
        ));
    }

    if leg.pool_id() != context_pool_id(context) {
        return Err(format!(
            "route/context pool mismatch: route={} context={}",
            leg.pool_id(),
            context_pool_id(context)
        ));
    }

    let context_slot = context_source_slot(context);
    if context_slot < leg.source_slot() {
        return Err(format!(
            "stale quote context: route_slot={} quote_slot={context_slot}",
            leg.source_slot()
        ));
    }

    if !context_contains_pair(context, leg.input_mint(), leg.output_mint()) {
        return Err(format!(
            "quote context does not contain route pair {}/{}",
            leg.input_mint(),
            leg.output_mint()
        ));
    }

    Ok(())
}

fn context_venue(context: &VenueQuoteContext<'_>) -> Venue {
    match context {
        VenueQuoteContext::Raydium { .. } => Venue::RaydiumCpmm,
        VenueQuoteContext::PumpSwap { .. } => Venue::PumpSwap,
    }
}

fn context_pool_id<'a>(context: &'a VenueQuoteContext<'_>) -> &'a str {
    match context {
        VenueQuoteContext::Raydium { pool_id, .. }
        | VenueQuoteContext::PumpSwap { pool_id, .. } => pool_id.as_str(),
    }
}

fn context_source_slot(context: &VenueQuoteContext<'_>) -> u64 {
    match context {
        VenueQuoteContext::Raydium { snapshot, .. } => snapshot.slot,
        VenueQuoteContext::PumpSwap { snapshot, .. } => snapshot.slot,
    }
}

fn context_contains_pair(
    context: &VenueQuoteContext<'_>,
    input_mint: &str,
    output_mint: &str,
) -> bool {
    let (mint_a, mint_b) = match context {
        VenueQuoteContext::Raydium { snapshot, .. } => (
            snapshot.pool_state.token_0_mint.as_str(),
            snapshot.pool_state.token_1_mint.as_str(),
        ),
        VenueQuoteContext::PumpSwap { snapshot, .. } => (
            snapshot.pool_state.base_mint.as_str(),
            snapshot.pool_state.quote_mint.as_str(),
        ),
    };

    (mint_a == input_mint && mint_b == output_mint)
        || (mint_b == input_mint && mint_a == output_mint)
}

fn context_mint_decimals(context: &VenueQuoteContext<'_>, mint: &str) -> Result<u8, String> {
    match context {
        VenueQuoteContext::Raydium { snapshot, .. } => {
            if mint == snapshot.pool_state.token_0_mint {
                Ok(snapshot.pool_state.mint_0_decimals)
            } else if mint == snapshot.pool_state.token_1_mint {
                Ok(snapshot.pool_state.mint_1_decimals)
            } else {
                Err(format!("mint {mint} is not in Raydium quote context"))
            }
        }
        VenueQuoteContext::PumpSwap { snapshot, .. } => {
            if mint == snapshot.pool_state.base_mint {
                Ok(snapshot.base_decimals)
            } else if mint == snapshot.pool_state.quote_mint {
                Ok(snapshot.quote_decimals)
            } else {
                Err(format!("mint {mint} is not in PumpSwap quote context"))
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::pumpswap::{PumpSwapFeeConfig, PumpSwapFeeTier, PumpSwapFees, PumpSwapPoolState};
    use crate::raydium::{RaydiumAmmConfig, RaydiumCpmmPoolState};
    use crate::route::{generate_two_leg_routes, WRAPPED_SOL_MINT};
    use scout_core::{NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState};

    const TEST_MINT: &str = "ApZuxdpzMrbEYTGEzeY9afh5pj9d6qPRJCTgQYiipbKg";
    const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";

    fn normalized_pool(
        venue: Venue,
        pool_id: &str,
        mint_a: &str,
        mint_b: &str,
        decimals_a: u8,
        decimals_b: u8,
        slot: u64,
    ) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-program", venue.label()),
            source_slot: slot,
            token_a: NormalizedToken {
                mint: mint_a.to_owned(),
                vault: format!("{pool_id}-vault-a"),
                decimals: decimals_a,
            },
            token_b: NormalizedToken {
                mint: mint_b.to_owned(),
                vault: format!("{pool_id}-vault-b"),
                decimals: decimals_b,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Available {
                token_a_raw: 10_000_000_000,
                token_b_raw: 20_000_000_000,
                source_slot: slot,
            },
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    fn raydium_snapshot() -> RaydiumHydrationSnapshot {
        RaydiumHydrationSnapshot {
            slot: 101,
            pool_state: RaydiumCpmmPoolState {
                amm_config: "amm-config".to_owned(),
                token_0_vault: "ray-vault-0".to_owned(),
                token_1_vault: "ray-vault-1".to_owned(),
                token_0_mint: WRAPPED_SOL_MINT.to_owned(),
                token_1_mint: TEST_MINT.to_owned(),
                token_0_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
                token_1_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
                status: 0,
                lp_mint_decimals: 6,
                mint_0_decimals: 9,
                mint_1_decimals: 6,
                lp_supply: 1,
                protocol_fees_token_0: 0,
                protocol_fees_token_1: 0,
                fund_fees_token_0: 0,
                fund_fees_token_1: 0,
                open_time: 0,
                recent_epoch: 0,
                creator_fee_on: 0,
                enable_creator_fee: false,
                creator_fees_token_0: 0,
                creator_fees_token_1: 0,
            },
            amm_config: RaydiumAmmConfig {
                bump: 1,
                disable_create_pool: false,
                index: 1,
                trade_fee_rate: 2_500,
                protocol_fee_rate: 120_000,
                fund_fee_rate: 40_000,
                create_pool_fee: 0,
                protocol_owner: "protocol".to_owned(),
                fund_owner: "fund".to_owned(),
                creator_fee_rate: 500,
            },
            token_0_vault_raw: 10_000_000_000,
            token_1_vault_raw: 20_000_000_000,
            token_0_accrued_fees_raw: 0,
            token_1_accrued_fees_raw: 0,
            token_0_effective_raw: 10_000_000_000,
            token_1_effective_raw: 20_000_000_000,
        }
    }

    fn pumpswap_snapshot() -> PumpSwapHydrationSnapshot {
        PumpSwapHydrationSnapshot {
            slot: 102,
            pool_state: PumpSwapPoolState {
                pool_bump: 1,
                index: 1,
                creator: "11111111111111111111111111111111".to_owned(),
                base_mint: TEST_MINT.to_owned(),
                quote_mint: WRAPPED_SOL_MINT.to_owned(),
                lp_mint: "11111111111111111111111111111111".to_owned(),
                pool_base_token_account: "pump-base-vault".to_owned(),
                pool_quote_token_account: "pump-quote-vault".to_owned(),
                lp_supply: 1,
                coin_creator: Some("11111111111111111111111111111111".to_owned()),
                is_mayhem_mode: Some(false),
                is_cashback_coin: Some(false),
                virtual_quote_reserves: 0,
            },
            base_vault_raw: 20_000_000_000,
            quote_vault_raw: 10_000_000_000,
            effective_quote_raw: 10_000_000_000,
            base_mint_supply_raw: 1_000_000_000_000,
            base_token_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
            quote_token_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
            base_decimals: 6,
            quote_decimals: 9,
            disable_flags: 0,
            trading_state: PoolTradingState::Tradable,
            fee_config: PumpSwapFeeConfig {
                bump: 255,
                admin: "11111111111111111111111111111111".to_owned(),
                flat_fees: PumpSwapFees {
                    lp_fee_bps: 20,
                    protocol_fee_bps: 5,
                    creator_fee_bps: 5,
                },
                fee_tiers: vec![PumpSwapFeeTier {
                    market_cap_lamports_threshold: 0,
                    fees: PumpSwapFees {
                        lp_fee_bps: 20,
                        protocol_fee_bps: 5,
                        creator_fee_bps: 5,
                    },
                }],
                stable_fee_tiers: Vec::new(),
            },
        }
    }

    fn route_candidates() -> Vec<TwoLegRouteCandidate> {
        generate_two_leg_routes(&[
            normalized_pool(
                Venue::RaydiumCpmm,
                "raydium-pool",
                WRAPPED_SOL_MINT,
                TEST_MINT,
                9,
                6,
                100,
            ),
            normalized_pool(
                Venue::PumpSwap,
                "pumpswap-pool",
                TEST_MINT,
                WRAPPED_SOL_MINT,
                6,
                9,
                100,
            ),
        ])
    }

    #[test]
    fn whole_anchor_probe_uses_live_context_decimals() -> Result<(), String> {
        let routes = route_candidates();
        let route = routes
            .iter()
            .find(|candidate| candidate.leg_1().venue() == Venue::PumpSwap)
            .ok_or_else(|| "missing PumpSwap-first route fixture".to_owned())?;
        let snapshot = pumpswap_snapshot();
        let context = VenueQuoteContext::PumpSwap {
            pool_id: "pumpswap-pool".to_owned(),
            snapshot: &snapshot,
        };

        assert_eq!(one_whole_anchor_input_raw(route, &context)?, 1_000_000_000);
        Ok(())
    }

    #[test]
    fn two_leg_quote_chains_raw_output_and_restores_anchor_dust() -> Result<(), String> {
        let routes = route_candidates();
        let route = routes
            .iter()
            .find(|candidate| candidate.leg_1().venue() == Venue::PumpSwap)
            .ok_or_else(|| "missing PumpSwap-first route fixture".to_owned())?;
        let pump = pumpswap_snapshot();
        let ray = raydium_snapshot();
        let leg_1_context = VenueQuoteContext::PumpSwap {
            pool_id: "pumpswap-pool".to_owned(),
            snapshot: &pump,
        };
        let leg_2_context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let quote =
            quote_two_leg_exact_input(route, 1_000_000_000, &leg_1_context, &leg_2_context)?;

        assert_eq!(
            quote.leg_2.amount_in_requested_raw,
            quote.leg_1.amount_out_raw
        );
        assert_eq!(quote.leg_2.amount_in_unspent_raw, 0);
        assert_eq!(
            quote.anchor_output_raw,
            quote.leg_2.amount_out_raw + quote.anchor_input_unspent_raw
        );
        assert!(quote.anchor_output_raw > 0);

        Ok(())
    }

    #[test]
    fn stale_quote_context_is_rejected() -> Result<(), String> {
        let routes = route_candidates();
        let route = routes
            .iter()
            .find(|candidate| candidate.leg_1().venue() == Venue::RaydiumCpmm)
            .ok_or_else(|| "missing Raydium-first route fixture".to_owned())?;
        let mut ray = raydium_snapshot();
        ray.slot = 99;
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let result = one_whole_anchor_input_raw(route, &context);

        assert!(matches!(result, Err(error) if error.contains("stale quote context")));
        Ok(())
    }

    #[test]
    fn mismatched_pool_context_is_rejected() -> Result<(), String> {
        let routes = route_candidates();
        let route = routes
            .iter()
            .find(|candidate| candidate.leg_1().venue() == Venue::RaydiumCpmm)
            .ok_or_else(|| "missing Raydium-first route fixture".to_owned())?;
        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "wrong-pool".to_owned(),
            snapshot: &ray,
        };

        let result = one_whole_anchor_input_raw(route, &context);

        assert!(matches!(result, Err(error) if error.contains("pool mismatch")));
        Ok(())
    }
}
