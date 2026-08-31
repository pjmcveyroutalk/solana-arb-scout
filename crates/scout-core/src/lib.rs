use serde::{Deserialize, Serialize};
use std::time::Duration;
use thiserror::Error;

pub const MIN_NET_PROFIT_USD: f64 = 0.05;

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

    #[test]
    fn treasury_model_calculates_positive_net() {
        let result = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Treasury,
            None,
        );

        assert!(result.is_ok());

        let economics = match result {
            Ok(value) => value,
            Err(error) => panic!("unexpected economics error: {error}"),
        };

        assert!(economics.expected_net_usd > 0.0);
        assert!(economics.passes_minimum);
    }

    #[test]
    fn flash_model_includes_flash_costs() {
        let flash = FlashCosts {
            borrow_fee_usd: 0.01,
            added_execution_cost_usd: 0.02,
        };

        let treasury = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Treasury,
            None,
        );

        let flash_result = calculate_economics(
            &sample_quote(),
            &sample_execution(),
            FundingMode::Flash,
            Some(&flash),
        );

        assert!(treasury.is_ok());
        assert!(flash_result.is_ok());

        let treasury = match treasury {
            Ok(value) => value,
            Err(error) => panic!("unexpected treasury error: {error}"),
        };

        let flash_result = match flash_result {
            Ok(value) => value,
            Err(error) => panic!("unexpected flash error: {error}"),
        };

        assert!(flash_result.expected_net_usd < treasury.expected_net_usd);
    }

    #[test]
    fn rejects_zero_input() {
        let mut quote = sample_quote();
        quote.input_usd = 0.0;

        let result = calculate_economics(
            &quote,
            &sample_execution(),
            FundingMode::Treasury,
            None,
        );

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
