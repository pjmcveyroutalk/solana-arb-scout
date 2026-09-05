use crate::pumpswap::{self, PumpSwapHydrationSnapshot};
use crate::raydium::{self, RaydiumHydrationSnapshot};
use crate::route::{RouteLeg, TwoLegRouteCandidate};
use orca_whirlpools_core::{
    swap_quote_by_input_token, ExactInSwapQuote, OracleFacade, TickArrayFacade, TickArrays,
    TransferFee, WhirlpoolFacade,
};
use scout_core::{
    AdapterCapabilities, AuxiliaryStateKind, CapabilityState, ContentionFootprintState,
    LiquidityModel, NormalizedPoolState, PoolTradingState, QuoteReserveState, Venue,
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
    Orca {
        trade_fee_raw: u64,
        trade_fee_rate_min: u32,
        trade_fee_rate_max: u32,
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
            Self::Orca {
                trade_fee_raw,
                trade_fee_rate_min,
                trade_fee_rate_max,
            } => format!(
                "trade_fee_raw={trade_fee_raw} trade_fee_rate_min={trade_fee_rate_min} \
                 trade_fee_rate_max={trade_fee_rate_max}"
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

#[allow(dead_code)]
pub enum VenueQuoteContext<'a> {
    Raydium {
        pool_id: String,
        snapshot: &'a RaydiumHydrationSnapshot,
    },
    PumpSwap {
        pool_id: String,
        snapshot: &'a PumpSwapHydrationSnapshot,
    },
    Orca {
        snapshot: &'a OrcaQuoteSnapshot,
    },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct QuoteReadiness {
    venue: Venue,
    pool_id: String,
    token_a_mint: String,
    token_b_mint: String,
    source_slot: u64,
    capabilities: AdapterCapabilities,
}

impl QuoteReadiness {
    pub(crate) fn validate_for_pool(&self, pool: &NormalizedPoolState) -> Result<(), String> {
        if pool.trading_state != PoolTradingState::Tradable {
            return Err(format!(
                "pool {} is not tradable: state={}",
                pool.pool_id,
                pool.trading_state.label()
            ));
        }

        if self.venue != pool.venue {
            return Err(format!(
                "quote readiness venue mismatch: readiness={} pool={}",
                self.venue.label(),
                pool.venue.label()
            ));
        }

        if self.pool_id != pool.pool_id {
            return Err(format!(
                "quote readiness pool mismatch: readiness={} pool={}",
                self.pool_id, pool.pool_id
            ));
        }

        if self.token_a_mint != pool.token_a.mint || self.token_b_mint != pool.token_b.mint {
            return Err(format!(
                "quote readiness token pair mismatch for pool {}",
                pool.pool_id
            ));
        }

        if self.source_slot < pool.source_slot {
            return Err(format!(
                "stale quote readiness: pool={} pool_slot={} readiness_slot={}",
                pool.pool_id, pool.source_slot, self.source_slot
            ));
        }

        if self.capabilities.exact_input_quote != CapabilityState::Supported {
            return Err(format!(
                "quote readiness for pool {} does not prove exact-input support",
                pool.pool_id
            ));
        }

        Ok(())
    }

    #[cfg(test)]
    pub(crate) fn synthetic_for_test(
        pool: &NormalizedPoolState,
        capabilities: AdapterCapabilities,
    ) -> Self {
        Self {
            venue: pool.venue,
            pool_id: pool.pool_id.clone(),
            token_a_mint: pool.token_a.mint.clone(),
            token_b_mint: pool.token_b.mint.clone(),
            source_slot: pool.source_slot,
            capabilities,
        }
    }
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaQuoteReadinessEvidence {
    pool_id: String,
    token_a_mint: String,
    token_b_mint: String,
    source_slot: u64,
    quote_a_to_b: ExactInSwapQuote,
    quote_b_to_a: ExactInSwapQuote,
}

#[allow(dead_code)]
impl OrcaQuoteReadinessEvidence {
    pub(crate) fn from_o2_quotes(
        pool_id: &str,
        token_a_mint: &str,
        token_b_mint: &str,
        source_slot: u64,
        quote_a_to_b: ExactInSwapQuote,
        quote_b_to_a: ExactInSwapQuote,
    ) -> Result<Self, String> {
        if pool_id.is_empty() {
            return Err("Orca O2 readiness evidence requires a pool id".to_owned());
        }

        if token_a_mint.is_empty() || token_b_mint.is_empty() {
            return Err("Orca O2 readiness evidence requires both token mints".to_owned());
        }

        if token_a_mint == token_b_mint {
            return Err("Orca O2 readiness evidence requires distinct token mints".to_owned());
        }

        if source_slot == 0 {
            return Err("Orca O2 readiness evidence requires a nonzero source slot".to_owned());
        }

        validate_orca_exact_input_quote("O2 A-to-B readiness", &quote_a_to_b)?;
        validate_orca_exact_input_quote("O2 B-to-A readiness", &quote_b_to_a)?;

        Ok(Self {
            pool_id: pool_id.to_owned(),
            token_a_mint: token_a_mint.to_owned(),
            token_b_mint: token_b_mint.to_owned(),
            source_slot,
            quote_a_to_b,
            quote_b_to_a,
        })
    }

    fn validate_for_pool(&self, pool: &NormalizedPoolState) -> Result<(), String> {
        if self.pool_id != pool.pool_id {
            return Err(format!(
                "Orca O2 readiness pool mismatch: evidence={} pool={}",
                self.pool_id, pool.pool_id
            ));
        }

        if self.token_a_mint != pool.token_a.mint || self.token_b_mint != pool.token_b.mint {
            return Err(format!(
                "Orca O2 readiness token pair mismatch for pool {}",
                pool.pool_id
            ));
        }

        if self.source_slot < pool.source_slot {
            return Err(format!(
                "stale Orca O2 readiness evidence: pool={} pool_slot={} evidence_slot={}",
                pool.pool_id, pool.source_slot, self.source_slot
            ));
        }

        validate_orca_exact_input_quote("O2 A-to-B readiness", &self.quote_a_to_b)?;
        validate_orca_exact_input_quote("O2 B-to-A readiness", &self.quote_b_to_a)?;

        Ok(())
    }
}

fn validate_orca_exact_input_quote(label: &str, quote: &ExactInSwapQuote) -> Result<(), String> {
    if quote.token_in == 0 {
        return Err(format!("Orca {label} quote has zero token input"));
    }

    if quote.token_est_out == 0 {
        return Err(format!("Orca {label} quote has zero estimated output"));
    }

    if quote.token_min_out > quote.token_est_out {
        return Err(format!(
            "Orca {label} quote minimum output exceeds estimated output"
        ));
    }

    if quote.trade_fee > quote.token_in {
        return Err(format!("Orca {label} quote trade fee exceeds token input"));
    }

    if quote.trade_fee_rate_min > quote.trade_fee_rate_max {
        return Err(format!("Orca {label} quote fee-rate bounds are inverted"));
    }

    Ok(())
}

fn orca_clmm_capabilities() -> AdapterCapabilities {
    AdapterCapabilities {
        liquidity_model: LiquidityModel::Clmm,
        exact_input_quote: CapabilityState::Supported,
        spl_token: CapabilityState::Supported,
        token_2022: CapabilityState::RequiresHydration,
        transfer_fee: CapabilityState::RequiresHydration,
        auxiliary_state: AuxiliaryStateKind::Ticks,
        contention_footprint: ContentionFootprintState::Incomplete,
    }
}

pub fn orca_quote_readiness_for_pool(
    pool: &NormalizedPoolState,
    evidence: &OrcaQuoteReadinessEvidence,
) -> Result<QuoteReadiness, String> {
    if pool.venue != Venue::Orca {
        return Err(format!(
            "Orca quote readiness requires venue=orca, got {}",
            pool.venue.label()
        ));
    }

    if pool.trading_state != PoolTradingState::Tradable {
        return Err(format!(
            "pool {} is not tradable: state={}",
            pool.pool_id,
            pool.trading_state.label()
        ));
    }

    if let QuoteReserveState::Available { .. } = &pool.quote_reserves {
        return Err(format!(
            "Orca CLMM pool {} must not fabricate CPMM quote reserves",
            pool.pool_id
        ));
    }

    evidence.validate_for_pool(pool)?;

    let readiness = QuoteReadiness {
        venue: Venue::Orca,
        pool_id: pool.pool_id.clone(),
        token_a_mint: pool.token_a.mint.clone(),
        token_b_mint: pool.token_b.mint.clone(),
        source_slot: evidence.source_slot,
        capabilities: orca_clmm_capabilities(),
    };

    readiness.validate_for_pool(pool)?;
    Ok(readiness)
}

#[allow(dead_code)]
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaQuoteSnapshot {
    pool_id: String,
    token_a_mint: String,
    token_b_mint: String,
    token_a_decimals: u8,
    token_b_decimals: u8,
    source_slot: u64,
    whirlpool: WhirlpoolFacade,
    tick_arrays: [TickArrayFacade; 5],
    timestamp: u64,
    oracle: Option<OracleFacade>,
    transfer_fee_a: Option<TransferFee>,
    transfer_fee_b: Option<TransferFee>,
}

#[allow(dead_code)]
impl OrcaQuoteSnapshot {
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn from_o2_hydration(
        pool: &NormalizedPoolState,
        evidence: &OrcaQuoteReadinessEvidence,
        source_slot: u64,
        token_a_decimals: u8,
        token_b_decimals: u8,
        whirlpool: WhirlpoolFacade,
        tick_arrays: [TickArrayFacade; 5],
        timestamp: u64,
        oracle: Option<OracleFacade>,
        transfer_fee_a: Option<TransferFee>,
        transfer_fee_b: Option<TransferFee>,
    ) -> Result<Self, String> {
        let readiness = orca_quote_readiness_for_pool(pool, evidence)?;

        if source_slot != readiness.source_slot {
            return Err(format!(
                "Orca quote snapshot slot mismatch: readiness={} snapshot={source_slot}",
                readiness.source_slot
            ));
        }

        if token_a_decimals != pool.token_a.decimals {
            return Err(format!(
                "Orca quote snapshot token A decimals mismatch: pool={} snapshot={token_a_decimals}",
                pool.token_a.decimals
            ));
        }

        if token_b_decimals != pool.token_b.decimals {
            return Err(format!(
                "Orca quote snapshot token B decimals mismatch: pool={} snapshot={token_b_decimals}",
                pool.token_b.decimals
            ));
        }

        if whirlpool.tick_spacing == 0 {
            return Err("Orca quote snapshot tick spacing must be greater than zero".to_owned());
        }

        match (whirlpool.is_initialized_with_adaptive_fee(), oracle) {
            (true, None) => {
                return Err("adaptive-fee Orca Whirlpool requires Oracle state".to_owned());
            }
            (false, Some(_)) => {
                return Err("non-adaptive Orca Whirlpool must not receive Oracle state".to_owned());
            }
            _ => {}
        }

        if let Some(oracle_state) = oracle {
            if oracle_state.trade_enable_timestamp > timestamp {
                return Err(format!(
                    concat!(
                        "adaptive-fee Orca Whirlpool trading is not enabled yet: ",
                        "trade_enable_timestamp={} quote_timestamp={}"
                    ),
                    oracle_state.trade_enable_timestamp, timestamp
                ));
            }
        }

        Ok(Self {
            pool_id: readiness.pool_id,
            token_a_mint: readiness.token_a_mint,
            token_b_mint: readiness.token_b_mint,
            token_a_decimals,
            token_b_decimals,
            source_slot,
            whirlpool,
            tick_arrays,
            timestamp,
            oracle,
            transfer_fee_a,
            transfer_fee_b,
        })
    }
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

struct OrcaQuoteAdapter<'a> {
    snapshot: &'a OrcaQuoteSnapshot,
}

impl<'a> OrcaQuoteAdapter<'a> {
    fn new(snapshot: &'a OrcaQuoteSnapshot) -> Self {
        Self { snapshot }
    }

    fn adapter_capabilities() -> AdapterCapabilities {
        orca_clmm_capabilities()
    }
}

impl ExactInputQuoteAdapter for OrcaQuoteAdapter<'_> {
    fn venue(&self) -> Venue {
        Venue::Orca
    }

    fn pool_id(&self) -> &str {
        self.snapshot.pool_id.as_str()
    }

    fn source_slot(&self) -> u64 {
        self.snapshot.source_slot
    }

    fn capabilities(&self) -> AdapterCapabilities {
        Self::adapter_capabilities()
    }

    fn contains_pair(&self, input_mint: &str, output_mint: &str) -> bool {
        let mint_a = self.snapshot.token_a_mint.as_str();
        let mint_b = self.snapshot.token_b_mint.as_str();

        (mint_a == input_mint && mint_b == output_mint)
            || (mint_b == input_mint && mint_a == output_mint)
    }

    #[cfg(test)]
    fn mint_decimals(&self, mint: &str) -> Result<u8, String> {
        if mint == self.snapshot.token_a_mint {
            Ok(self.snapshot.token_a_decimals)
        } else if mint == self.snapshot.token_b_mint {
            Ok(self.snapshot.token_b_decimals)
        } else {
            Err(format!("mint {mint} is not in Orca quote context"))
        }
    }

    fn quote_exact_input(
        &self,
        input_mint: &str,
        amount_in_raw: u64,
    ) -> Result<VenueLegQuote, String> {
        if amount_in_raw == 0 {
            return Err("Orca exact-input quote amount must be greater than zero".to_owned());
        }

        let specified_token_a = if input_mint == self.snapshot.token_a_mint {
            true
        } else if input_mint == self.snapshot.token_b_mint {
            false
        } else {
            return Err(format!(
                "input mint {input_mint} is not part of the Orca Whirlpool"
            ));
        };

        let quote = swap_quote_by_input_token(
            amount_in_raw,
            specified_token_a,
            0,
            self.snapshot.whirlpool,
            self.snapshot.oracle,
            TickArrays::Five(
                self.snapshot.tick_arrays[0],
                self.snapshot.tick_arrays[1],
                self.snapshot.tick_arrays[2],
                self.snapshot.tick_arrays[3],
                self.snapshot.tick_arrays[4],
            ),
            self.snapshot.timestamp,
            self.snapshot.transfer_fee_a,
            self.snapshot.transfer_fee_b,
        )
        .map_err(|error| {
            format!(
                concat!(
                    "Orca authoritative exact-input quote failed: pool={} ",
                    "input_mint={} amount_in_raw={} error={error:?}"
                ),
                self.snapshot.pool_id, input_mint, amount_in_raw
            )
        })?;

        orca_leg_quote_from_core(
            self.snapshot.pool_id.as_str(),
            self.snapshot.source_slot,
            amount_in_raw,
            quote,
        )
    }
}

fn orca_leg_quote_from_core(
    pool_id: &str,
    source_slot: u64,
    amount_in_requested_raw: u64,
    quote: ExactInSwapQuote,
) -> Result<VenueLegQuote, String> {
    validate_orca_exact_input_quote("universal-dispatch exact-input", &quote)?;

    if quote.token_in > amount_in_requested_raw {
        return Err(format!(
            concat!(
                "Orca authoritative quote consumed more input than requested: ",
                "pool={} requested={} consumed={}"
            ),
            pool_id, amount_in_requested_raw, quote.token_in
        ));
    }

    let amount_in_unspent_raw = amount_in_requested_raw
        .checked_sub(quote.token_in)
        .ok_or_else(|| "Orca universal quote input subtraction underflow".to_owned())?;

    Ok(VenueLegQuote {
        venue: Venue::Orca,
        pool_id: pool_id.to_owned(),
        amount_in_requested_raw,
        amount_in_consumed_raw: quote.token_in,
        amount_in_unspent_raw,
        amount_out_raw: quote.token_est_out,
        fees: VenueFeeComponents::Orca {
            trade_fee_raw: quote.trade_fee,
            trade_fee_rate_min: quote.trade_fee_rate_min,
            trade_fee_rate_max: quote.trade_fee_rate_max,
        },
        quote_source_slot: source_slot,
    })
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
        VenueQuoteContext::Orca { snapshot } => {
            let adapter = OrcaQuoteAdapter::new(snapshot);
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

pub fn quote_readiness_for_pool(
    pool: &NormalizedPoolState,
    context: &VenueQuoteContext<'_>,
) -> Result<QuoteReadiness, String> {
    with_quote_adapter(context, |adapter| {
        if pool.trading_state != PoolTradingState::Tradable {
            return Err(format!(
                "pool {} is not tradable: state={}",
                pool.pool_id,
                pool.trading_state.label()
            ));
        }

        if pool.venue != adapter.venue() {
            return Err(format!(
                "pool/context venue mismatch: pool={} context={}",
                pool.venue.label(),
                adapter.venue().label()
            ));
        }

        if pool.pool_id != adapter.pool_id() {
            return Err(format!(
                "pool/context pool mismatch: pool={} context={}",
                pool.pool_id,
                adapter.pool_id()
            ));
        }

        if adapter.source_slot() < pool.source_slot {
            return Err(format!(
                "stale quote context: pool_slot={} quote_slot={}",
                pool.source_slot,
                adapter.source_slot()
            ));
        }

        if !adapter.contains_pair(&pool.token_a.mint, &pool.token_b.mint) {
            return Err(format!(
                "quote context does not contain normalized pool pair {}/{}",
                pool.token_a.mint, pool.token_b.mint
            ));
        }

        ensure_exact_input_quote_supported(adapter)?;

        let capabilities = adapter.capabilities();

        match capabilities.liquidity_model {
            LiquidityModel::Cpmm => match &pool.quote_reserves {
                QuoteReserveState::Available {
                    token_a_raw,
                    token_b_raw,
                    ..
                } => {
                    if *token_a_raw == 0 || *token_b_raw == 0 {
                        return Err(format!(
                            "pool {} does not have positive CPMM quote reserves",
                            pool.pool_id
                        ));
                    }
                }
                QuoteReserveState::Unavailable => {
                    return Err(format!(
                        "pool {} does not have CPMM quote reserves",
                        pool.pool_id
                    ));
                }
            },
            LiquidityModel::Clmm | LiquidityModel::Dlmm => {
                return Err(format!(
                    "{} {} quote readiness is not enabled in D0 production",
                    adapter.venue().label(),
                    capabilities.liquidity_model.label()
                ));
            }
        }

        let readiness = QuoteReadiness {
            venue: pool.venue,
            pool_id: pool.pool_id.clone(),
            token_a_mint: pool.token_a.mint.clone(),
            token_b_mint: pool.token_b.mint.clone(),
            source_slot: adapter.source_slot(),
            capabilities,
        };

        readiness.validate_for_pool(pool)?;
        Ok(readiness)
    })
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
    use orca_whirlpools_core::{TickFacade, TICK_ARRAY_SIZE};
    use scout_core::NormalizedToken;

    const TEST_MINT: &str = "ApZuxdpzMrbEYTGEzeY9afh5pj9d6qPRJCTgQYiipbKg";
    const THIRD_MINT: &str = "9xQeWvG816bUx9EPjHmaT23yvVM8hGe3ucGWnZ6b9S4Y";
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

    fn orca_exact_input_quote(token_in: u64, token_est_out: u64) -> ExactInSwapQuote {
        ExactInSwapQuote {
            token_in,
            token_est_out,
            token_min_out: token_est_out,
            trade_fee: token_in / 1_000,
            trade_fee_rate_min: 3_000,
            trade_fee_rate_max: 3_000,
        }
    }

    fn orca_pool() -> NormalizedPoolState {
        let mut pool = normalized_pool(
            Venue::Orca,
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        pool.quote_reserves = QuoteReserveState::Unavailable;
        pool
    }

    fn orca_evidence() -> Result<OrcaQuoteReadinessEvidence, String> {
        OrcaQuoteReadinessEvidence::from_o2_quotes(
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            101,
            orca_exact_input_quote(1_000_000, 12_108_498),
            orca_exact_input_quote(1_000_000, 82_091),
        )
    }

    fn orca_tick_array(start_tick_index: i32) -> TickArrayFacade {
        TickArrayFacade {
            start_tick_index,
            ticks: [TickFacade::default(); TICK_ARRAY_SIZE],
        }
    }

    fn orca_tick_arrays() -> [TickArrayFacade; 5] {
        [
            orca_tick_array(0),
            orca_tick_array(5_632),
            orca_tick_array(11_264),
            orca_tick_array(-5_632),
            orca_tick_array(-11_264),
        ]
    }

    fn orca_whirlpool() -> WhirlpoolFacade {
        let mut whirlpool = WhirlpoolFacade::default();
        whirlpool.tick_spacing = 64;
        whirlpool.fee_tier_index_seed = 64u16.to_le_bytes();
        whirlpool
    }

    fn orca_snapshot() -> Result<OrcaQuoteSnapshot, String> {
        let pool = orca_pool();
        let evidence = orca_evidence()?;

        OrcaQuoteSnapshot::from_o2_hydration(
            &pool,
            &evidence,
            101,
            9,
            6,
            orca_whirlpool(),
            orca_tick_arrays(),
            1_700_000_000,
            None,
            None,
            None,
        )
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

        let ray_capabilities = with_quote_adapter(&ray_context, |adapter| adapter.capabilities());
        let pump_capabilities = with_quote_adapter(&pump_context, |adapter| adapter.capabilities());

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
    fn universal_dispatch_exposes_orca_clmm_capabilities() -> Result<(), String> {
        let snapshot = orca_snapshot()?;
        let context = VenueQuoteContext::Orca {
            snapshot: &snapshot,
        };

        let capabilities = with_quote_adapter(&context, |adapter| adapter.capabilities());

        assert_eq!(capabilities, orca_clmm_capabilities());
        assert_eq!(capabilities.liquidity_model, LiquidityModel::Clmm);
        assert_eq!(capabilities.auxiliary_state, AuxiliaryStateKind::Ticks);
        assert_eq!(
            capabilities.contention_footprint,
            ContentionFootprintState::Incomplete
        );
        Ok(())
    }

    #[test]
    fn orca_core_quote_mapping_preserves_partial_consumption_and_fee_evidence() -> Result<(), String>
    {
        let native = ExactInSwapQuote {
            token_in: 900_000,
            token_est_out: 12_108_498,
            token_min_out: 12_108_498,
            trade_fee: 2_700,
            trade_fee_rate_min: 3_000,
            trade_fee_rate_max: 3_000,
        };

        let adapted = orca_leg_quote_from_core("orca-pool", 101, 1_000_000, native)?;

        assert_eq!(
            adapted,
            VenueLegQuote {
                venue: Venue::Orca,
                pool_id: "orca-pool".to_owned(),
                amount_in_requested_raw: 1_000_000,
                amount_in_consumed_raw: 900_000,
                amount_in_unspent_raw: 100_000,
                amount_out_raw: 12_108_498,
                fees: VenueFeeComponents::Orca {
                    trade_fee_raw: 2_700,
                    trade_fee_rate_min: 3_000,
                    trade_fee_rate_max: 3_000,
                },
                quote_source_slot: 101,
            }
        );
        Ok(())
    }

    #[test]
    fn orca_core_quote_mapping_rejects_overconsumption() {
        let native = ExactInSwapQuote {
            token_in: 1_000_001,
            token_est_out: 12_108_498,
            token_min_out: 12_108_498,
            trade_fee: 3_000,
            trade_fee_rate_min: 3_000,
            trade_fee_rate_max: 3_000,
        };

        let result = orca_leg_quote_from_core("orca-pool", 101, 1_000_000, native);

        assert!(matches!(
            result,
            Err(error) if error.contains("consumed more input than requested")
        ));
    }

    #[test]
    fn orca_snapshot_rejects_adaptive_pool_without_oracle() -> Result<(), String> {
        let pool = orca_pool();
        let evidence = orca_evidence()?;
        let mut whirlpool = orca_whirlpool();
        whirlpool.fee_tier_index_seed = 65u16.to_le_bytes();

        let result = OrcaQuoteSnapshot::from_o2_hydration(
            &pool,
            &evidence,
            101,
            9,
            6,
            whirlpool,
            orca_tick_arrays(),
            1_700_000_000,
            None,
            None,
            None,
        );

        assert!(matches!(
            result,
            Err(error) if error.contains("requires Oracle state")
        ));
        Ok(())
    }

    #[test]
    fn raydium_quote_readiness_binds_live_context_and_positive_reserves() -> Result<(), String> {
        let pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let readiness = quote_readiness_for_pool(&pool, &context)?;

        assert_eq!(readiness.venue, Venue::RaydiumCpmm);
        assert_eq!(readiness.pool_id, "raydium-pool");
        assert_eq!(readiness.token_a_mint, WRAPPED_SOL_MINT);
        assert_eq!(readiness.token_b_mint, TEST_MINT);
        assert_eq!(readiness.source_slot, 101);
        assert_eq!(readiness.capabilities.liquidity_model, LiquidityModel::Cpmm);
        Ok(())
    }

    #[test]
    fn pumpswap_quote_readiness_binds_live_context_and_positive_reserves() -> Result<(), String> {
        let pool = normalized_pool(
            Venue::PumpSwap,
            "pumpswap-pool",
            TEST_MINT,
            WRAPPED_SOL_MINT,
            6,
            9,
            100,
        );
        let pump = pumpswap_snapshot();
        let context = VenueQuoteContext::PumpSwap {
            pool_id: "pumpswap-pool".to_owned(),
            snapshot: &pump,
        };

        let readiness = quote_readiness_for_pool(&pool, &context)?;

        assert_eq!(readiness.venue, Venue::PumpSwap);
        assert_eq!(readiness.pool_id, "pumpswap-pool");
        assert_eq!(readiness.source_slot, 102);
        assert_eq!(readiness.capabilities.liquidity_model, LiquidityModel::Cpmm);
        Ok(())
    }

    #[test]
    fn cpmm_readiness_rejects_unavailable_and_zero_reserves() {
        let base_pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let mut unavailable_pool = base_pool.clone();
        unavailable_pool.quote_reserves = QuoteReserveState::Unavailable;

        let result = quote_readiness_for_pool(&unavailable_pool, &context);
        assert!(matches!(
            result,
            Err(error) if error.contains("does not have CPMM quote reserves")
        ));

        let mut zero_reserve_pool = base_pool;
        zero_reserve_pool.quote_reserves = QuoteReserveState::Available {
            token_a_raw: 0,
            token_b_raw: 20_000_000_000,
            source_slot: 100,
        };

        let result = quote_readiness_for_pool(&zero_reserve_pool, &context);
        assert!(matches!(
            result,
            Err(error) if error.contains("positive CPMM quote reserves")
        ));
    }

    #[test]
    fn cpmm_readiness_preserves_reserve_source_slot_semantics() {
        let mut pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        pool.quote_reserves = QuoteReserveState::Available {
            token_a_raw: 10_000_000_000,
            token_b_raw: 20_000_000_000,
            source_slot: 99,
        };

        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        assert!(quote_readiness_for_pool(&pool, &context).is_ok());
    }

    #[test]
    fn quote_readiness_rejects_nontradable_pool() {
        let mut pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        pool.trading_state = PoolTradingState::SwapDisabled;
        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let result = quote_readiness_for_pool(&pool, &context);
        assert!(matches!(result, Err(error) if error.contains("is not tradable")));
    }

    #[test]
    fn quote_readiness_rejects_venue_and_pool_mismatch() {
        let pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        let pump = pumpswap_snapshot();
        let wrong_venue = VenueQuoteContext::PumpSwap {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &pump,
        };

        let result = quote_readiness_for_pool(&pool, &wrong_venue);
        assert!(matches!(result, Err(error) if error.contains("venue mismatch")));

        let ray = raydium_snapshot();
        let wrong_pool = VenueQuoteContext::Raydium {
            pool_id: "wrong-pool".to_owned(),
            snapshot: &ray,
        };

        let result = quote_readiness_for_pool(&pool, &wrong_pool);
        assert!(matches!(result, Err(error) if error.contains("pool mismatch")));
    }

    #[test]
    fn quote_readiness_rejects_token_pair_mismatch() {
        let pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            THIRD_MINT,
            9,
            6,
            100,
        );
        let ray = raydium_snapshot();
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let result = quote_readiness_for_pool(&pool, &context);
        assert!(matches!(
            result,
            Err(error) if error.contains("does not contain normalized pool pair")
        ));
    }

    #[test]
    fn quote_readiness_rejects_stale_context() {
        let pool = normalized_pool(
            Venue::RaydiumCpmm,
            "raydium-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        let mut ray = raydium_snapshot();
        ray.slot = 99;
        let context = VenueQuoteContext::Raydium {
            pool_id: "raydium-pool".to_owned(),
            snapshot: &ray,
        };

        let result = quote_readiness_for_pool(&pool, &context);
        assert!(matches!(
            result,
            Err(error) if error.contains("stale quote context")
        ));
    }

    #[test]
    fn synthetic_clmm_readiness_does_not_require_cpmm_reserves() {
        let mut pool = normalized_pool(
            Venue::Orca,
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
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

        assert!(readiness.validate_for_pool(&pool).is_ok());
        assert_eq!(readiness.capabilities.liquidity_model, LiquidityModel::Clmm);
        assert_eq!(
            readiness.capabilities.auxiliary_state,
            AuxiliaryStateKind::Ticks
        );
    }

    #[test]
    fn orca_quote_readiness_binds_bidirectional_o2_evidence() -> Result<(), String> {
        let mut pool = normalized_pool(
            Venue::Orca,
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );
        pool.quote_reserves = QuoteReserveState::Unavailable;

        let evidence = OrcaQuoteReadinessEvidence::from_o2_quotes(
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            101,
            orca_exact_input_quote(1_000_000, 12_108_498),
            orca_exact_input_quote(1_000_000, 82_091),
        )?;

        let readiness = orca_quote_readiness_for_pool(&pool, &evidence)?;

        assert_eq!(readiness.venue, Venue::Orca);
        assert_eq!(readiness.pool_id, "orca-pool");
        assert_eq!(readiness.token_a_mint, WRAPPED_SOL_MINT);
        assert_eq!(readiness.token_b_mint, TEST_MINT);
        assert_eq!(readiness.source_slot, 101);
        assert_eq!(readiness.capabilities, orca_clmm_capabilities());
        assert_eq!(readiness.capabilities.liquidity_model, LiquidityModel::Clmm);
        assert_eq!(
            readiness.capabilities.auxiliary_state,
            AuxiliaryStateKind::Ticks
        );

        Ok(())
    }

    #[test]
    fn orca_quote_readiness_rejects_fabricated_cpmm_reserves() -> Result<(), String> {
        let pool = normalized_pool(
            Venue::Orca,
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            100,
        );

        let evidence = OrcaQuoteReadinessEvidence::from_o2_quotes(
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            101,
            orca_exact_input_quote(1_000_000, 12_108_498),
            orca_exact_input_quote(1_000_000, 82_091),
        )?;

        let result = orca_quote_readiness_for_pool(&pool, &evidence);

        assert!(matches!(
            result,
            Err(error) if error.contains("must not fabricate CPMM quote reserves")
        ));

        Ok(())
    }

    #[test]
    fn orca_quote_readiness_rejects_stale_evidence() -> Result<(), String> {
        let mut pool = normalized_pool(
            Venue::Orca,
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            9,
            6,
            102,
        );
        pool.quote_reserves = QuoteReserveState::Unavailable;

        let evidence = OrcaQuoteReadinessEvidence::from_o2_quotes(
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            101,
            orca_exact_input_quote(1_000_000, 12_108_498),
            orca_exact_input_quote(1_000_000, 82_091),
        )?;

        let result = orca_quote_readiness_for_pool(&pool, &evidence);

        assert!(matches!(
            result,
            Err(error) if error.contains("stale Orca O2 readiness evidence")
        ));

        Ok(())
    }

    #[test]
    fn orca_o2_readiness_evidence_rejects_invalid_quote_proof() {
        let result = OrcaQuoteReadinessEvidence::from_o2_quotes(
            "orca-pool",
            WRAPPED_SOL_MINT,
            TEST_MINT,
            101,
            orca_exact_input_quote(1_000_000, 0),
            orca_exact_input_quote(1_000_000, 82_091),
        );

        assert!(matches!(
            result,
            Err(error) if error.contains("zero estimated output")
        ));
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
        assert!(matches!(
            result,
            Err(error) if error.contains("stale quote context")
        ));
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
