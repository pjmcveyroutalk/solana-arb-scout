use crate::quote::TwoLegRouteQuote;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FundingMode {
    Treasury,
    Flash,
}

impl FundingMode {
    pub fn label(self) -> &'static str {
        match self {
            Self::Treasury => "treasury",
            Self::Flash => "flash",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CostProvenanceKind {
    Observed,
    ModeledAssumption,
}

impl CostProvenanceKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::Observed => "observed",
            Self::ModeledAssumption => "modeled_assumption",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExplicitCost {
    amount_anchor_raw: u64,
    provenance_kind: CostProvenanceKind,
    provenance: String,
}

impl ExplicitCost {
    pub fn new(
        amount_anchor_raw: u64,
        provenance_kind: CostProvenanceKind,
        provenance: impl Into<String>,
    ) -> Result<Self, String> {
        let provenance = provenance.into();

        if provenance.trim().is_empty() {
            return Err("economics cost provenance must not be empty".to_owned());
        }

        Ok(Self {
            amount_anchor_raw,
            provenance_kind,
            provenance,
        })
    }

    pub fn amount_anchor_raw(&self) -> u64 {
        self.amount_anchor_raw
    }

    pub fn provenance_kind(&self) -> CostProvenanceKind {
        self.provenance_kind
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequiredCost {
    Known(ExplicitCost),
    Unknown {
        provenance_kind: CostProvenanceKind,
        reason: String,
    },
}

impl RequiredCost {
    pub fn known(
        amount_anchor_raw: u64,
        provenance_kind: CostProvenanceKind,
        provenance: impl Into<String>,
    ) -> Result<Self, String> {
        Ok(Self::Known(ExplicitCost::new(
            amount_anchor_raw,
            provenance_kind,
            provenance,
        )?))
    }

    pub fn unknown(
        provenance_kind: CostProvenanceKind,
        reason: impl Into<String>,
    ) -> Result<Self, String> {
        let reason = reason.into();

        if reason.trim().is_empty() {
            return Err("economics unknown-cost reason must not be empty".to_owned());
        }

        Ok(Self::Unknown {
            provenance_kind,
            reason,
        })
    }

    pub fn resolve(&self, cost_name: &str) -> Result<&ExplicitCost, String> {
        match self {
            Self::Known(cost) => Ok(cost),
            Self::Unknown {
                provenance_kind,
                reason,
            } => Err(format!(
                "economics required cost unknown: cost={cost_name} kind={} reason={reason}",
                provenance_kind.label()
            )),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommonEconomicsCosts {
    pub base_fee: RequiredCost,
    pub priority_fee: RequiredCost,
    pub submission_cost: RequiredCost,
    pub expected_failure_cost: RequiredCost,
    pub safety_reserve: RequiredCost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TreasuryFundingCosts {
    pub capital_cost: RequiredCost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FlashFundingCosts {
    pub borrowing_cost: RequiredCost,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EconomicsCostModel {
    basis_id: String,
    pub common: CommonEconomicsCosts,
    pub treasury: TreasuryFundingCosts,
    pub flash: FlashFundingCosts,
}

impl EconomicsCostModel {
    pub fn new(
        basis_id: impl Into<String>,
        common: CommonEconomicsCosts,
        treasury: TreasuryFundingCosts,
        flash: FlashFundingCosts,
    ) -> Result<Self, String> {
        let basis_id = basis_id.into();

        if basis_id.trim().is_empty() {
            return Err("economics cost basis id must not be empty".to_owned());
        }

        Ok(Self {
            basis_id,
            common,
            treasury,
            flash,
        })
    }

    pub fn basis_id(&self) -> &str {
        &self.basis_id
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ExpectedNetEconomics {
    pub funding_mode: FundingMode,
    pub cost_basis_id: String,
    pub anchor_mint: String,
    pub anchor_input_requested_raw: u64,
    pub anchor_output_raw: u64,
    pub gross_delta_raw: i128,
    pub common_cost_raw: u64,
    pub funding_cost_raw: u64,
    pub total_external_cost_raw: u64,
    pub expected_net_raw: i128,
}

impl ExpectedNetEconomics {
    pub fn is_positive(&self) -> bool {
        self.expected_net_raw > 0
    }

    pub fn summary(&self) -> String {
        format!(
            concat!(
                "funding={} basis={} anchor={} requested_in_raw={} anchor_out_raw={} ",
                "gross_delta_raw={} common_cost_raw={} funding_cost_raw={} ",
                "total_external_cost_raw={} expected_net_raw={} positive={}"
            ),
            self.funding_mode.label(),
            self.cost_basis_id,
            self.anchor_mint,
            self.anchor_input_requested_raw,
            self.anchor_output_raw,
            self.gross_delta_raw,
            self.common_cost_raw,
            self.funding_cost_raw,
            self.total_external_cost_raw,
            self.expected_net_raw,
            self.is_positive(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEconomics {
    pub treasury: ExpectedNetEconomics,
    pub flash: ExpectedNetEconomics,
}

pub fn evaluate_expected_net(
    quote: &TwoLegRouteQuote,
    costs: &EconomicsCostModel,
) -> Result<RouteEconomics, String> {
    let treasury = evaluate_expected_net_for_mode(quote, costs, FundingMode::Treasury)?;
    let flash = evaluate_expected_net_for_mode(quote, costs, FundingMode::Flash)?;

    Ok(RouteEconomics { treasury, flash })
}

pub fn evaluate_expected_net_for_mode(
    quote: &TwoLegRouteQuote,
    costs: &EconomicsCostModel,
    funding_mode: FundingMode,
) -> Result<ExpectedNetEconomics, String> {
    if quote.anchor_input_requested_raw == 0 {
        return Err("economics requires non-zero anchor input".to_owned());
    }

    let common_cost_raw = resolved_common_cost_raw(costs)?;
    let gross_delta_raw =
        i128::from(quote.anchor_output_raw) - i128::from(quote.anchor_input_requested_raw);

    let funding_cost_raw = match funding_mode {
        FundingMode::Treasury => costs
            .treasury
            .capital_cost
            .resolve("treasury_capital_cost")?
            .amount_anchor_raw(),
        FundingMode::Flash => costs
            .flash
            .borrowing_cost
            .resolve("flash_borrowing_cost")?
            .amount_anchor_raw(),
    };

    evaluate_funding_mode(
        quote,
        costs,
        funding_mode,
        gross_delta_raw,
        common_cost_raw,
        funding_cost_raw,
    )
}

fn resolved_common_cost_raw(costs: &EconomicsCostModel) -> Result<u64, String> {
    let base_fee = costs.common.base_fee.resolve("base_fee")?;
    let priority_fee = costs.common.priority_fee.resolve("priority_fee")?;
    let submission_cost = costs.common.submission_cost.resolve("submission_cost")?;
    let expected_failure_cost = costs
        .common
        .expected_failure_cost
        .resolve("expected_failure_cost")?;
    let safety_reserve = costs.common.safety_reserve.resolve("safety_reserve")?;

    checked_sum_costs([
        base_fee,
        priority_fee,
        submission_cost,
        expected_failure_cost,
        safety_reserve,
    ])
}

fn evaluate_funding_mode(
    quote: &TwoLegRouteQuote,
    costs: &EconomicsCostModel,
    funding_mode: FundingMode,
    gross_delta_raw: i128,
    common_cost_raw: u64,
    funding_cost_raw: u64,
) -> Result<ExpectedNetEconomics, String> {
    let total_external_cost_raw = common_cost_raw
        .checked_add(funding_cost_raw)
        .ok_or_else(|| "economics total external cost overflow".to_owned())?;

    let expected_net_raw = gross_delta_raw
        .checked_sub(i128::from(total_external_cost_raw))
        .ok_or_else(|| "economics expected-net underflow".to_owned())?;

    Ok(ExpectedNetEconomics {
        funding_mode,
        cost_basis_id: costs.basis_id().to_owned(),
        anchor_mint: quote.anchor_mint.clone(),
        anchor_input_requested_raw: quote.anchor_input_requested_raw,
        anchor_output_raw: quote.anchor_output_raw,
        gross_delta_raw,
        common_cost_raw,
        funding_cost_raw,
        total_external_cost_raw,
        expected_net_raw,
    })
}

fn checked_sum_costs<const N: usize>(costs: [&ExplicitCost; N]) -> Result<u64, String> {
    costs.into_iter().try_fold(0u64, |total, cost| {
        total
            .checked_add(cost.amount_anchor_raw())
            .ok_or_else(|| "economics common cost overflow".to_owned())
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::quote::{TwoLegRouteQuote, VenueFeeComponents, VenueLegQuote};
    use scout_core::Venue;

    const ANCHOR_MINT: &str = "So11111111111111111111111111111111111111112";
    const INTERMEDIATE_MINT: &str = "Intermediate1111111111111111111111111111111";

    fn observed_cost(amount_anchor_raw: u64, label: &str) -> Result<RequiredCost, String> {
        RequiredCost::known(amount_anchor_raw, CostProvenanceKind::Observed, label)
    }

    fn modeled_cost(amount_anchor_raw: u64, label: &str) -> Result<RequiredCost, String> {
        RequiredCost::known(
            amount_anchor_raw,
            CostProvenanceKind::ModeledAssumption,
            label,
        )
    }

    fn cost_model() -> Result<EconomicsCostModel, String> {
        EconomicsCostModel::new(
            "test-cost-basis-v2",
            CommonEconomicsCosts {
                base_fee: observed_cost(5, "fixture observed base fee")?,
                priority_fee: observed_cost(7, "fixture observed priority fee")?,
                submission_cost: observed_cost(11, "fixture observed submission cost")?,
                expected_failure_cost: modeled_cost(13, "fixture modeled failure cost")?,
                safety_reserve: modeled_cost(17, "fixture modeled safety reserve")?,
            },
            TreasuryFundingCosts {
                capital_cost: modeled_cost(19, "fixture modeled treasury capital cost")?,
            },
            FlashFundingCosts {
                borrowing_cost: modeled_cost(23, "fixture modeled flash borrowing cost")?,
            },
        )
    }

    fn leg_quote(
        venue: Venue,
        pool_id: &str,
        amount_in_raw: u64,
        amount_out_raw: u64,
        slot: u64,
    ) -> Result<VenueLegQuote, String> {
        let fees = match venue {
            Venue::RaydiumCpmm => VenueFeeComponents::RaydiumCpmm {
                trade_fee_raw: 10,
                protocol_fee_raw: 2,
                fund_fee_raw: 1,
                creator_fee_raw: 0,
            },
            Venue::PumpSwap => VenueFeeComponents::PumpSwap {
                lp_fee_raw: 10,
                protocol_fee_raw: 2,
                creator_fee_raw: 0,
            },
            Venue::Meteora | Venue::Orca => {
                return Err("unsupported venue in economics test fixture".to_owned());
            }
        };

        Ok(VenueLegQuote {
            venue,
            pool_id: pool_id.to_owned(),
            amount_in_requested_raw: amount_in_raw,
            amount_in_consumed_raw: amount_in_raw,
            amount_in_unspent_raw: 0,
            amount_out_raw,
            fees,
            quote_source_slot: slot,
        })
    }

    fn route_quote(
        anchor_input_raw: u64,
        anchor_output_raw: u64,
    ) -> Result<TwoLegRouteQuote, String> {
        Ok(TwoLegRouteQuote {
            anchor_mint: ANCHOR_MINT.to_owned(),
            intermediate_mint: INTERMEDIATE_MINT.to_owned(),
            anchor_input_requested_raw: anchor_input_raw,
            anchor_input_consumed_raw: anchor_input_raw,
            anchor_input_unspent_raw: 0,
            anchor_output_raw,
            leg_1: leg_quote(
                Venue::RaydiumCpmm,
                "raydium-pool",
                anchor_input_raw,
                2_000,
                100,
            )?,
            leg_2: leg_quote(
                Venue::PumpSwap,
                "pumpswap-pool",
                2_000,
                anchor_output_raw,
                101,
            )?,
        })
    }

    #[test]
    fn treasury_and_flash_models_are_evaluated_independently() -> Result<(), String> {
        let quote = route_quote(1_000, 1_200)?;
        let costs = cost_model()?;
        let result = evaluate_expected_net(&quote, &costs)?;

        assert_eq!(result.treasury.gross_delta_raw, 200);
        assert_eq!(result.flash.gross_delta_raw, 200);

        assert_eq!(result.treasury.common_cost_raw, 53);
        assert_eq!(result.flash.common_cost_raw, 53);

        assert_eq!(result.treasury.funding_cost_raw, 19);
        assert_eq!(result.flash.funding_cost_raw, 23);

        assert_eq!(result.treasury.total_external_cost_raw, 72);
        assert_eq!(result.flash.total_external_cost_raw, 76);

        assert_eq!(result.treasury.expected_net_raw, 128);
        assert_eq!(result.flash.expected_net_raw, 124);

        assert!(result.treasury.is_positive());
        assert!(result.flash.is_positive());

        Ok(())
    }

    #[test]
    fn one_funding_mode_can_evaluate_when_the_other_is_unknown() -> Result<(), String> {
        let quote = route_quote(1_000, 1_200)?;
        let mut costs = cost_model()?;
        costs.treasury.capital_cost = RequiredCost::unknown(
            CostProvenanceKind::ModeledAssumption,
            "treasury capital policy not defined",
        )?;

        let treasury = evaluate_expected_net_for_mode(&quote, &costs, FundingMode::Treasury);
        let flash = evaluate_expected_net_for_mode(&quote, &costs, FundingMode::Flash)?;

        assert_eq!(
            treasury,
            Err(
                "economics required cost unknown: cost=treasury_capital_cost kind=modeled_assumption reason=treasury capital policy not defined"
                    .to_owned()
            )
        );
        assert_eq!(flash.expected_net_raw, 124);

        Ok(())
    }

    #[test]
    fn observed_and_modeled_costs_preserve_distinct_provenance() -> Result<(), String> {
        let costs = cost_model()?;

        let base_fee = costs.common.base_fee.resolve("base_fee")?;
        let safety_reserve = costs.common.safety_reserve.resolve("safety_reserve")?;

        assert_eq!(base_fee.provenance_kind(), CostProvenanceKind::Observed);
        assert_eq!(
            safety_reserve.provenance_kind(),
            CostProvenanceKind::ModeledAssumption
        );
        assert_eq!(base_fee.provenance(), "fixture observed base fee");
        assert_eq!(
            safety_reserve.provenance(),
            "fixture modeled safety reserve"
        );

        Ok(())
    }

    #[test]
    fn unknown_common_cost_fails_closed() -> Result<(), String> {
        let quote = route_quote(1_000, 1_200)?;
        let mut costs = cost_model()?;
        costs.common.priority_fee = RequiredCost::unknown(
            CostProvenanceKind::Observed,
            "priority fee observation unavailable",
        )?;

        let result = evaluate_expected_net_for_mode(&quote, &costs, FundingMode::Flash);

        assert_eq!(
            result,
            Err(
                "economics required cost unknown: cost=priority_fee kind=observed reason=priority fee observation unavailable"
                    .to_owned()
            )
        );

        Ok(())
    }

    #[test]
    fn negative_gross_route_remains_negative_after_costs() -> Result<(), String> {
        let quote = route_quote(1_000, 900)?;
        let costs = cost_model()?;
        let result = evaluate_expected_net(&quote, &costs)?;

        assert_eq!(result.treasury.gross_delta_raw, -100);
        assert_eq!(result.treasury.expected_net_raw, -172);
        assert_eq!(result.flash.expected_net_raw, -176);

        assert!(!result.treasury.is_positive());
        assert!(!result.flash.is_positive());

        Ok(())
    }

    #[test]
    fn route_quote_output_is_the_gross_basis_without_dex_fee_double_counting() -> Result<(), String>
    {
        let quote = route_quote(1_000, 1_050)?;
        let costs = cost_model()?;
        let result = evaluate_expected_net(&quote, &costs)?;

        assert_eq!(result.treasury.gross_delta_raw, 50);
        assert_eq!(result.flash.gross_delta_raw, 50);

        Ok(())
    }

    #[test]
    fn zero_anchor_input_is_rejected() -> Result<(), String> {
        let quote = route_quote(0, 100)?;
        let costs = cost_model()?;
        let result = evaluate_expected_net(&quote, &costs);

        assert_eq!(
            result,
            Err("economics requires non-zero anchor input".to_owned())
        );

        Ok(())
    }

    #[test]
    fn empty_cost_provenance_is_rejected() {
        let result = ExplicitCost::new(1, CostProvenanceKind::Observed, "   ");

        assert_eq!(
            result,
            Err("economics cost provenance must not be empty".to_owned())
        );
    }

    #[test]
    fn empty_unknown_cost_reason_is_rejected() {
        let result = RequiredCost::unknown(CostProvenanceKind::Observed, "   ");

        assert_eq!(
            result,
            Err("economics unknown-cost reason must not be empty".to_owned())
        );
    }

    #[test]
    fn empty_cost_basis_id_is_rejected() -> Result<(), String> {
        let result = EconomicsCostModel::new(
            "",
            CommonEconomicsCosts {
                base_fee: observed_cost(1, "base")?,
                priority_fee: observed_cost(1, "priority")?,
                submission_cost: observed_cost(1, "submission")?,
                expected_failure_cost: modeled_cost(1, "failure")?,
                safety_reserve: modeled_cost(1, "reserve")?,
            },
            TreasuryFundingCosts {
                capital_cost: modeled_cost(1, "treasury")?,
            },
            FlashFundingCosts {
                borrowing_cost: modeled_cost(1, "flash")?,
            },
        );

        assert_eq!(
            result,
            Err("economics cost basis id must not be empty".to_owned())
        );

        Ok(())
    }

    #[test]
    fn common_cost_overflow_is_rejected() -> Result<(), String> {
        let quote = route_quote(1_000, 2_000)?;

        let costs = EconomicsCostModel::new(
            "overflow-test",
            CommonEconomicsCosts {
                base_fee: observed_cost(u64::MAX, "base")?,
                priority_fee: observed_cost(1, "priority")?,
                submission_cost: observed_cost(0, "submission")?,
                expected_failure_cost: modeled_cost(0, "failure")?,
                safety_reserve: modeled_cost(0, "reserve")?,
            },
            TreasuryFundingCosts {
                capital_cost: modeled_cost(0, "treasury")?,
            },
            FlashFundingCosts {
                borrowing_cost: modeled_cost(0, "flash")?,
            },
        )?;

        let result = evaluate_expected_net(&quote, &costs);

        assert_eq!(result, Err("economics common cost overflow".to_owned()));

        Ok(())
    }

    #[test]
    fn funding_cost_overflow_is_rejected() -> Result<(), String> {
        let quote = route_quote(1_000, 2_000)?;

        let costs = EconomicsCostModel::new(
            "funding-overflow-test",
            CommonEconomicsCosts {
                base_fee: observed_cost(u64::MAX, "base")?,
                priority_fee: observed_cost(0, "priority")?,
                submission_cost: observed_cost(0, "submission")?,
                expected_failure_cost: modeled_cost(0, "failure")?,
                safety_reserve: modeled_cost(0, "reserve")?,
            },
            TreasuryFundingCosts {
                capital_cost: modeled_cost(1, "treasury")?,
            },
            FlashFundingCosts {
                borrowing_cost: modeled_cost(0, "flash")?,
            },
        )?;

        let result = evaluate_expected_net(&quote, &costs);

        assert_eq!(
            result,
            Err("economics total external cost overflow".to_owned())
        );

        Ok(())
    }
}
