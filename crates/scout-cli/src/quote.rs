use crate::pumpswap::{self, PumpSwapHydrationSnapshot};
use crate::raydium::{self, RaydiumHydrationSnapshot};
use crate::route::{RouteLeg, TwoLegRouteCandidate};
use scout_core::{
    AdapterCapabilities, AuxiliaryStateKind, CapabilityState, ContentionFootprintState,
    LiquidityModel, Venue,
};

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

trait ExactInputQuoteAdapter {
    fn venue(&self) -> Venue;

    fn pool_id(&self) -> &str;

    fn source_slot(&self) -> u64;

    fn capabilities(&self) -> AdapterCapabilities;

    fn contains_pair(&self, input_mint: &str, output_mint: &str) -> bool;

    #[cfg(test)]
    fn mint_decimals(&self, mint: &str) -> Result<u8, String>;

    fn quote_exact_input(
        &self,
        input_mint: &str,
        amount_in_raw: u64,
    ) -> Result<VenueLegQuote, String>;
}

struct RaydiumCpmmQuoteAdapter<'a> {
    pool_id: &'a str,
    snapshot: &'a RaydiumHydrationSnapshot,
}

impl<'a> RaydiumCpmmQuoteAdapter<'a> {
    fn new(pool_id: &'a str, snapshot: &'a RaydiumHydrationSnapshot) -> Self {
        Self { pool_id, snapshot }
    }

    fn adapter_capabilities() -> AdapterCapabilities {
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
}

impl ExactInputQuoteAdapter for RaydiumCpmmQuoteAdapter<'_> {
    fn venue(&self) -> Venue {
        Venue::RaydiumCpmm
    }

    fn pool_id(&self) -> &str {
        self.pool_id
    }

    fn source_slot(&self) -> u64 {
        self.snapshot.slot
    }

    fn capabilities(&self) -> AdapterCapabilities {
        Self::adapter_capabilities()
    }

    fn contains_pair(&self, input_mint: &str, output_mint: &str) -> bool {
        let mint_a = self.snapshot.pool_state.token_0_mint.as_str();
        let mint_b = self.snapshot.pool_state.token_1_mint.as_str();

        (mint_a == input_mint && mint_b == output_mint)
            || (mint_b == input_mint && mint_a == output_mint)
    }

    #[cfg(test)]
    fn mint_decimals(&self, mint: &str) -> Result<u8, String> {
        if mint == self.snapshot.pool_state.token_0_mint {
            Ok(self.snapshot.pool_state.mint_0_decimals)
        } else if mint == self.snapshot.pool_state.token_1_mint {
            Ok(self.snapshot.pool_state.mint_1_decimals)
        } else {
            Err(format!("mint {mint} is not in Raydium quote context"))
        }
    }

    fn quote_exact_input(
        &self,
        input_mint: &str,
        amount_in_raw: u64,
    ) -> Result<VenueLegQuote, String> {
        let quote = raydium::quote_exact_input(self.snapshot, input_mint, amount_in_raw)?;

        Ok(VenueLegQuote {
            venue: Venue::RaydiumCpmm,
            pool_id: self.pool_id.to_owned(),
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
}

struct PumpSwapQuoteAdapter<'a> {
    pool_id: &'a str,
    snapshot: &'a PumpSwapHydrationSnapshot,
}

impl<'a> PumpSwapQuoteAdapter<'a> {
    fn new(pool_id: &'a str, snapshot: &'a PumpSwapHydrationSnapshot) -> Self {
        Self { pool_id, snapshot }
    }

    fn adapter_capabilities() -> AdapterCapabilities {
        AdapterCapabilities {
            liquidity_model: LiquidityModel::Cpmm,
            exact_input_quote: CapabilityState::Supported,
            spl_token: CapabilityState::Supported,
            token_2022: CapabilityState::RequiresHydration,
            transfer_fee: CapabilityState::RequiresHydration,
            auxiliary_state: AuxiliaryStateKind::None,
            contention_footprint: ContentionFootprintState::Incomplete,
        }
    }
}

impl ExactInputQuoteAdapter for PumpSwapQuoteAdapter<'_> {
    fn venue(&self) -> Venue {
        Venue::PumpSwap
    }

    fn pool_id(&self) -> &str {
        self.pool_id
    }

    fn source_slot(&self) -> u64 {
        self.snapshot.slot
    }

    fn capabilities(&self) -> AdapterCapabilities {
        Self::adapter_capabilities()
    }

    fn contains_pair(&self, input_mint: &str, output_mint: &str) -> bool {
        let mint_a = self.snapshot.pool_state.base_mint.as_str();
        let mint_b = self.snapshot.pool_state.quote_mint.as_str();

        (mint_a == input_mint && mint_b == output_mint)
            || (mint_b == input_mint && mint_a == output_mint)
    }

    #[cfg(test)]
    fn mint_decimals(&self, mint: &str) -> Result<u8, String> {
        if mint == self.snapshot.pool_state.base_mint {
            Ok(self.snapshot.base_decimals)
        } else if mint == self.snapshot.pool_state.quote_mint {
            Ok(self.snapshot.quote_decimals)
        } else {
            Err(format!("mint {mint} is not in PumpSwap quote context"))
        }
    }

    fn quote_exact_input(
        &self,
        input_mint: &str,
        amount_in_raw: u64,
    ) -> Result<VenueLegQuote, String> {
        let quote = pumpswap::quote_exact_input(self.snapshot, input_mint, amount_in_raw)?;

        Ok(VenueLegQuote {
            venue: Venue::PumpSwap,
            pool_id: self.pool_id.to_owned(),
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

fn with_quote_adapter<T>(
    context: &VenueQuoteContext<'_>,
    operation: impl FnOnce(&dyn ExactInputQuoteAdapter) -> T,
) -> T {
    match context {
        VenueQuoteContext::Raydium { pool_id, snapshot } => {
            let adapter = RaydiumCpmmQuoteAdapter::new(pool_id.as_str(), snapshot);
            operation(&adapter)
        }
        VenueQuoteContext::PumpSwap { pool_id, snapshot } => {
            let adapter = PumpSwapQuoteAdapter::new(pool_id.as_str(), snapshot);
            operation(&adapter)
        }
    }
}

fn ensure_exact_input_quote_supported(adapter: &dyn ExactInputQuoteAdapter) -> Result<(), String> {
    match adapter.capabilities().exact_input_quote {
        CapabilityState::Supported => Ok(()),
        CapabilityState::Unsupported => Err(format!(
            "{} adapter does not support exact-input quoting",
            adapter.venue().label()
        )),
        CapabilityState::RequiresHydration => Err(format!(
            "{} adapter requires additional hydration before exact-input quoting",
            adapter.venue().label()
        )),
    }
}

#[cfg(test)]
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
    with_quote_adapter(context, |adapter| {
        ensure_exact_input_quote_supported(adapter)?;
        adapter.quote_exact_input(leg.input_mint(), amount_in_raw)
    })
}

fn validate_context(leg: &RouteLeg, context: &VenueQuoteContext<'_>) -> Result<(), String> {
    with_quote_adapter(context, |adapter| {
        if leg.venue() != adapter.venue() {
            return Err(format!(
                "route/context venue mismatch: route={} context={}",
                leg.venue().label(),
                adapter.venue().label()
            ));
        }

        if leg.pool_id() != adapter.pool_id() {
            return Err(format!(
                "route/context pool mismatch: route={} context={}",
                leg.pool_id(),
                adapter.pool_id()
            ));
        }

        let context_slot = adapter.source_slot();
        if context_slot < leg.source_slot() {
            return Err(format!(
                "stale quote context: route_slot={} quote_slot={context_slot}",
                leg.source_slot()
            ));
        }

        if !adapter.contains_pair(leg.input_mint(), leg.output_mint()) {
            return Err(format!(
                "quote context does not contain route pair {}/{}",
                leg.input_mint(),
                leg.output_mint()
            ));
        }

        Ok(())
    })
}

#[cfg(test)]
fn context_mint_decimals(context: &VenueQuoteContext<'_>, mint: &str) -> Result<u8, String> {
    with_quote_adapter(context, |adapter| adapter.mint_decimals(mint))
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
                observation_key: "11111111111111111111111111111111".to_owned(),
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
    fn raydium_adapter_preserves_native_quote_and_capabilities() -> Result<(), String> {
        let snapshot = raydium_snapshot();
        let amount_in_raw = 1_000_000_000;
        let native = raydium::quote_exact_input(&snapshot, WRAPPED_SOL_MINT, amount_in_raw)?;
        let adapter = RaydiumCpmmQuoteAdapter::new("raydium-pool", &snapshot);
        let adapted = adapter.quote_exact_input(WRAPPED_SOL_MINT, amount_in_raw)?;

        let expected = VenueLegQuote {
            venue: Venue::RaydiumCpmm,
            pool_id: "raydium-pool".to_owned(),
            amount_in_requested_raw: native.amount_in_raw,
            amount_in_consumed_raw: native.amount_in_raw,
            amount_in_unspent_raw: 0,
            amount_out_raw: native.amount_out_raw,
            fees: VenueFeeComponents::RaydiumCpmm {
                trade_fee_raw: native.trade_fee_raw,
                protocol_fee_raw: native.protocol_fee_raw,
                fund_fee_raw: native.fund_fee_raw,
                creator_fee_raw: native.creator_fee_raw,
            },
            quote_source_slot: native.source_slot,
        };

        assert_eq!(adapted, expected);

        assert_eq!(
            RaydiumCpmmQuoteAdapter::adapter_capabilities(),
            AdapterCapabilities {
                liquidity_model: LiquidityModel::Cpmm,
                exact_input_quote: CapabilityState::Supported,
                spl_token: CapabilityState::Supported,
                token_2022: CapabilityState::RequiresHydration,
                transfer_fee: CapabilityState::RequiresHydration,
                auxiliary_state: AuxiliaryStateKind::None,
                contention_footprint: ContentionFootprintState::Complete,
            }
        );

        Ok(())
    }

    #[test]
    fn pumpswap_adapter_preserves_native_quotes_and_capabilities() -> Result<(), String> {
        let snapshot = pumpswap_snapshot();
        let adapter = PumpSwapQuoteAdapter::new("pumpswap-pool", &snapshot);

        for (input_mint, amount_in_raw) in [
            (TEST_MINT, 1_000_000_u64),
            (WRAPPED_SOL_MINT, 1_000_000_000_u64),
        ] {
            let native = pumpswap::quote_exact_input(&snapshot, input_mint, amount_in_raw)?;
            let adapted = adapter.quote_exact_input(input_mint, amount_in_raw)?;

            let expected = VenueLegQuote {
                venue: Venue::PumpSwap,
                pool_id: "pumpswap-pool".to_owned(),
                amount_in_requested_raw: native.amount_in_requested_raw,
                amount_in_consumed_raw: native.amount_in_consumed_raw,
                amount_in_unspent_raw: native.amount_in_unspent_raw,
                amount_out_raw: native.amount_out_raw,
                fees: VenueFeeComponents::PumpSwap {
                    lp_fee_raw: native.lp_fee_raw,
                    protocol_fee_raw: native.protocol_fee_raw,
                    creator_fee_raw: native.creator_fee_raw,
                },
                quote_source_slot: native.source_slot,
            };

            assert_eq!(adapted, expected);
        }

        assert_eq!(
            PumpSwapQuoteAdapter::adapter_capabilities(),
            AdapterCapabilities {
                liquidity_model: LiquidityModel::Cpmm,
                exact_input_quote: CapabilityState::Supported,
                spl_token: CapabilityState::Supported,
                token_2022: CapabilityState::RequiresHydration,
                transfer_fee: CapabilityState::RequiresHydration,
                auxiliary_state: AuxiliaryStateKind::None,
                contention_footprint: ContentionFootprintState::Incomplete,
            }
        );

        Ok(())
    }

    #[test]
    fn universal_dispatch_exposes_both_adapter_capability_contracts() {
        let ray = raydium_snapshot();
        let pump = pumpswap_snapshot();

        let ray_context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };
        let pump_context = VenueQuoteContext::PumpSwap {
            pool_id: "pumpswap-pool".to_owned(),
            snapshot: &pump,
        };

        let ray_capabilities =
            with_quote_adapter(&ray_context, |adapter| adapter.capabilities());
        let pump_capabilities =
            with_quote_adapter(&pump_context, |adapter| adapter.capabilities());

        assert_eq!(
            ray_capabilities,
            RaydiumCpmmQuoteAdapter::adapter_capabilities()
        );
        assert_eq!(
            pump_capabilities,
            PumpSwapQuoteAdapter::adapter_capabilities()
        );
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
    fn universal_route_dispatch_quotes_both_venue_directions() -> Result<(), String> {
        let routes = route_candidates();
        let pump = pumpswap_snapshot();
        let ray = raydium_snapshot();
        let mut raydium_first_seen = false;
        let mut pumpswap_first_seen = false;

        for route in routes {
            let quote = match route.leg_1().venue() {
                Venue::RaydiumCpmm => {
                    raydium_first_seen = true;
                    let leg_1_context = VenueQuoteContext::Raydium {
                        pool_id: "raydium-pool".to_owned(),
                        snapshot: &ray,
                    };
                    let leg_2_context = VenueQuoteContext::PumpSwap {
                        pool_id: "pumpswap-pool".to_owned(),
                        snapshot: &pump,
                    };

                    quote_two_leg_exact_input(
                        &route,
                        1_000_000_000,
                        &leg_1_context,
                        &leg_2_context,
                    )?
                }
                Venue::PumpSwap => {
                    pumpswap_first_seen = true;
                    let leg_1_context = VenueQuoteContext::PumpSwap {
                        pool_id: "pumpswap-pool".to_owned(),
                        snapshot: &pump,
                    };
                    let leg_2_context = VenueQuoteContext::Raydium {
                        pool_id: "raydium-pool".to_owned(),
                        snapshot: &ray,
                    };

                    quote_two_leg_exact_input(
                        &route,
                        1_000_000_000,
                        &leg_1_context,
                        &leg_2_context,
                    )?
                }
                venue => {
                    return Err(format!(
                        "unexpected venue {} in universal dispatch fixture",
                        venue.label()
                    ));
                }
            };

            assert_eq!(quote.leg_1.venue, route.leg_1().venue());
            assert_eq!(quote.leg_2.venue, route.leg_2().venue());
            assert_eq!(
                quote.leg_2.amount_in_requested_raw,
                quote.leg_1.amount_out_raw
            );
        }

        assert!(raydium_first_seen);
        assert!(pumpswap_first_seen);

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
