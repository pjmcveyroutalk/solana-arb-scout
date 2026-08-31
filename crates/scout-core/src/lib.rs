use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

pub const MIN_NET_PROFIT_USD: f64 = 0.05;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Venue {
    RaydiumCpmm,
    Meteora,
    Orca,
    PumpSwap,
}

impl Venue {
    pub fn label(self) -> &'static str {
        match self {
            Self::RaydiumCpmm => "raydium_cpmm",
            Self::Meteora => "meteora",
            Self::Orca => "orca",
            Self::PumpSwap => "pumpswap",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolTradingState {
    Tradable,
    NotYetOpen,
    SwapDisabled,
}

impl PoolTradingState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tradable => "tradable",
            Self::NotYetOpen => "not_yet_open",
            Self::SwapDisabled => "swap_disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuoteReserveState {
    Unavailable,
    Available { token_a_raw: u64, token_b_raw: u64 },
}

impl QuoteReserveState {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Available { .. } => "available",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedToken {
    pub mint: String,
    pub vault: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedPoolState {
    pub pool_id: String,
    pub venue: Venue,
    pub program_id: String,
    pub source_slot: u64,
    pub token_a: NormalizedToken,
    pub token_b: NormalizedToken,
    pub trading_state: PoolTradingState,
    pub quote_reserves: QuoteReserveState,
    pub account_update_received_at_unix_ms: u64,
    pub normalized_at_unix_ms: u64,
}

impl NormalizedPoolState {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "venue={} ",
                "pool={} ",
                "program={} ",
                "slot={} ",
                "mint_a={} mint_b={} ",
                "vault_a={} vault_b={} ",
                "decimals_a={} decimals_b={} ",
                "trading_state={} ",
                "quote_reserves={} ",
                "received_at_ms={} normalized_at_ms={}"
            ),
            self.venue.label(),
            self.pool_id,
            self.program_id,
            self.source_slot,
            self.token_a.mint,
            self.token_b.mint,
            self.token_a.vault,
            self.token_b.vault,
            self.token_a.decimals,
            self.token_b.decimals,
            self.trading_state.label(),
            self.quote_reserves.label(),
            self.account_update_received_at_unix_ms,
            self.normalized_at_unix_ms,
        )
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum FundingMode {
    Treasury,
    Flash,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RouteQuote {
    pub input_usd: f64,
    pub output_usd: f64,
    pub dex_fees_usd: f64,
    pub price_impact_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ExecutionCosts {
    pub base_fee_usd: f64,
    pub priority_fee_usd: f64,
    pub submission_cost_usd: f64,
    pub expected_failure_cost_usd: f64,
    pub safety_reserve_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FlashCosts {
    pub borrow_fee_usd: f64,
    pub added_execution_cost_usd: f64,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct OpportunityEconomics {
    pub funding_mode: FundingMode,
    pub gross_profit_usd: f64,
    pub total_cost_usd: f64,
    pub expected_net_usd: f64,
    pub passes_minimum: bool,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyObservation {
    pub account_update_to_state: Duration,
    pub state_to_candidate: Duration,
    pub candidate_to_quotes: Duration,
    pub quotes_to_profit: Duration,
    pub profit_to_hypothetical_ready: Duration,
}

impl LatencyObservation {
    pub fn total_time_to_ready(&self) -> Duration {
        self.account_update_to_state
            + self.state_to_candidate
            + self.candidate_to_quotes
            + self.quotes_to_profit
            + self.profit_to_hypothetical_ready
    }
}

#[derive(Debug, Error)]
pub enum EconomicsError {
    #[error("route input must be greater than zero")]
    InvalidInput,

    #[error("economic values must be finite and non-negative")]
    InvalidCost,
}

fn valid_non_negative(value: f64) -> bool {
    value.is_finite() && value >= 0.0
}

pub fn calculate_economics(
    quote: &RouteQuote,
    execution: &ExecutionCosts,
    funding_mode: FundingMode,
    flash: Option<&FlashCosts>,
) -> Result<OpportunityEconomics, EconomicsError> {
    if !quote.input_usd.is_finite() || quote.input_usd <= 0.0 {
        return Err(EconomicsError::InvalidInput);
    }

    let values = [
        quote.output_usd,
        quote.dex_fees_usd,
        quote.price_impact_usd,
        execution.base_fee_usd,
        execution.priority_fee_usd,
        execution.submission_cost_usd,
        execution.expected_failure_cost_usd,
        execution.safety_reserve_usd,
    ];

    if !values.into_iter().all(valid_non_negative) {
        return Err(EconomicsError::InvalidCost);
    }

    let gross_profit_usd = quote.output_usd - quote.input_usd;

    let mut total_cost_usd = quote.dex_fees_usd
        + quote.price_impact_usd
        + execution.base_fee_usd
        + execution.priority_fee_usd
        + execution.submission_cost_usd
        + execution.expected_failure_cost_usd
        + execution.safety_reserve_usd;

    if funding_mode == FundingMode::Flash {
        let flash = flash.ok_or(EconomicsError::InvalidCost)?;

        if !valid_non_negative(flash.borrow_fee_usd)
            || !valid_non_negative(flash.added_execution_cost_usd)
        {
            return Err(EconomicsError::InvalidCost);
        }

        total_cost_usd += flash.borrow_fee_usd + flash.added_execution_cost_usd;
    }

    let expected_net_usd = gross_profit_usd - total_cost_usd;

    Ok(OpportunityEconomics {
        funding_mode,
        gross_profit_usd,
        total_cost_usd,
        expected_net_usd,
        passes_minimum: expected_net_usd >= MIN_NET_PROFIT_USD,
    })
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_quote() -> RouteQuote {
        RouteQuote {
            input_usd: 100.0,
            output_usd: 100.50,
            dex_fees_usd: 0.10,
            price_impact_usd: 0.05,
        }
    }

    fn sample_execution() -> ExecutionCosts {
        ExecutionCosts {
            base_fee_usd: 0.001,
            priority_fee_usd: 0.01,
            submission_cost_usd: 0.01,
            expected_failure_cost_usd: 0.02,
            safety_reserve_usd: 0.05,
        }
    }

    fn sample_normalized_pool() -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: "Pool111111111111111111111111111111111111".to_owned(),
            venue: Venue::RaydiumCpmm,
            program_id: "Program111111111111111111111111111111111".to_owned(),
            source_slot: 123_456,
            token_a: NormalizedToken {
                mint: "MintA111111111111111111111111111111111111".to_owned(),
                vault: "VaultA11111111111111111111111111111111111".to_owned(),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: "MintB111111111111111111111111111111111111".to_owned(),
                vault: "VaultB11111111111111111111111111111111111".to_owned(),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Unavailable,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    #[test]
    fn normalized_pool_preserves_venue_independent_identity() {
        let pool = sample_normalized_pool();

        assert_eq!(pool.venue, Venue::RaydiumCpmm);
        assert_eq!(pool.venue.label(), "raydium_cpmm");
        assert_eq!(pool.source_slot, 123_456);
        assert_eq!(pool.token_a.decimals, 9);
        assert_eq!(pool.token_b.decimals, 6);
        assert_eq!(pool.trading_state, PoolTradingState::Tradable);
    }

    #[test]
    fn normalized_pool_marks_missing_quote_reserves_explicitly() {
        let pool = sample_normalized_pool();

        assert_eq!(pool.quote_reserves, QuoteReserveState::Unavailable);
        assert!(!pool.quote_reserves.is_available());
        assert_eq!(pool.quote_reserves.label(), "unavailable");
    }

    #[test]
    fn normalized_pool_summary_exposes_provenance_and_timing() {
        let pool = sample_normalized_pool();
        let summary = pool.summary();

        assert!(summary.contains("venue=raydium_cpmm"));
        assert!(summary.contains("slot=123456"));
        assert!(summary.contains("trading_state=tradable"));
        assert!(summary.contains("quote_reserves=unavailable"));
        assert!(summary.contains("received_at_ms=1000"));
        assert!(summary.contains("normalized_at_ms=1001"));
    }

    #[test]
    fn treasury_model_calculates_positive_net() -> Result<(), EconomicsError> {
        let economics = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Treasury,
            None,
        )?;

        assert!(economics.expected_net_usd > 0.0);
        assert!(economics.passes_minimum);

        Ok(())
    }

    #[test]
    fn flash_model_includes_flash_costs() -> Result<(), EconomicsError> {
        let flash = FlashCosts {
            borrow_fee_usd: 0.01,
            added_execution_cost_usd: 0.02,
        };

        let treasury = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Treasury,
            None,
        )?;

        let flash_result = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Flash,
            Some(&flash),
        )?;

        assert!(flash_result.expected_net_usd < treasury.expected_net_usd);

        Ok(())
    }

    #[test]
    fn rejects_zero_input() {
        let mut quote = sample_quote();
        quote.input_usd = 0.0;

        let result = calculate_economics(&quote, &sample_execution(), FundingMode::Treasury, None);

        assert!(matches!(result, Err(EconomicsError::InvalidInput)));
    }

    #[test]
    fn flash_requires_flash_cost_model() {
        let result = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Flash,
            None,
        );

        assert!(matches!(result, Err(EconomicsError::InvalidCost)));
    }
}
