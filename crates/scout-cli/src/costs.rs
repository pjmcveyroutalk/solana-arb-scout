use crate::economics::{
    CommonEconomicsCosts, CostProvenanceKind, EconomicsCostModel, FlashFundingCosts,
    RequiredCost, TreasuryFundingCosts,
};
use crate::pumpswap::PumpSwapHydrationSnapshot;
use crate::raydium::{RaydiumHydrationSnapshot, RAYDIUM_CPMM_PROGRAM_ID};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use scout_core::Venue;
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::collections::BTreeSet;
use std::str::FromStr;

pub const RUNG11_V0_BASIS_ID: &str = "rung11-v0-single-signer-600k-cu";
pub const MODELED_SIGNATURE_COUNT: u64 = 1;
pub const MODELED_COMPUTE_UNIT_LIMIT: u64 = 600_000;
pub const BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

const MICRO_LAMPORTS_PER_LAMPORT: u64 = 1_000_000;
const PRIORITY_FEE_RPC_REQUEST_ID: u64 = 11;
const MAX_LOCALIZED_PRIORITY_ACCOUNTS: usize = 128;
const RAYDIUM_OBSERVATION_SEED: &[u8] = b"observation";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterministicVenueContentionFootprint {
    accounts: Vec<String>,
    provenance: String,
}

impl DeterministicVenueContentionFootprint {
    fn new(accounts: impl IntoIterator<Item = String>) -> Result<Self, String> {
        let accounts = accounts.into_iter().collect::<BTreeSet<_>>();

        if accounts.is_empty() {
            return Err("localized priority-fee contention footprint must not be empty".to_owned());
        }

        if accounts.len() > MAX_LOCALIZED_PRIORITY_ACCOUNTS {
            return Err(format!(
                "localized priority-fee contention footprint exceeds RPC maximum: count={} max={MAX_LOCALIZED_PRIORITY_ACCOUNTS}",
                accounts.len()
            ));
        }

        for account in &accounts {
            Pubkey::from_str(account).map_err(|error| {
                format!("localized priority-fee contention account is invalid: account={account} error={error}")
            })?;
        }

        Ok(Self {
            accounts: accounts.into_iter().collect(),
            provenance: concat!(
                "deterministic venue contention observation scope only; ",
                "not a complete future transaction writable-account footprint; ",
                "executor-dependent user accounts and execution-policy accounts are excluded"
            )
            .to_owned(),
        })
    }

    pub fn accounts(&self) -> &[String] {
        &self.accounts
    }

    pub fn provenance(&self) -> &str {
        &self.provenance
    }

    pub fn summary(&self) -> String {
        format!(
            "account_count={} accounts=[{}] provenance={}",
            self.accounts.len(),
            self.accounts.join(","),
            self.provenance
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityFeeObservationSample {
    pub slot: u64,
    pub micro_lamports_per_cu: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityFeeObservation {
    pub samples: Vec<PriorityFeeObservationSample>,
    pub scope_accounts: Vec<String>,
    pub scope_provenance: String,
}

impl PriorityFeeObservation {
    pub fn summary(&self) -> String {
        if self.samples.is_empty() {
            return format!(
                "sample_count=0 zero_count=0 scope_account_count={} scope_provenance={}",
                self.scope_accounts.len(),
                self.scope_provenance
            );
        }

        let zero_count = self
            .samples
            .iter()
            .filter(|sample| sample.micro_lamports_per_cu == 0)
            .count();

        let min = self
            .samples
            .iter()
            .map(|sample| sample.micro_lamports_per_cu)
            .min()
            .unwrap_or(0);

        let max = self
            .samples
            .iter()
            .map(|sample| sample.micro_lamports_per_cu)
            .max()
            .unwrap_or(0);

        format!(
            concat!(
                "sample_count={} zero_count={} min_micro_lamports_per_cu={} ",
                "max_micro_lamports_per_cu={} scope_account_count={} scope_provenance={}"
            ),
            self.samples.len(),
            zero_count,
            min,
            max,
            self.scope_accounts.len(),
            self.scope_provenance
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityObservationState {
    Available(PriorityFeeObservation),
    Unavailable(String),
}

pub fn raydium_contention_footprint(
    pool_id: &str,
    snapshot: &RaydiumHydrationSnapshot,
) -> Result<DeterministicVenueContentionFootprint, String> {
    let pool = Pubkey::from_str(pool_id)
        .map_err(|error| format!("invalid Raydium pool id {pool_id}: {error}"))?;
    let program = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).map_err(|error| {
        format!(
            "invalid configured Raydium CPMM program id {RAYDIUM_CPMM_PROGRAM_ID}: {error}"
        )
    })?;

    let (observation, _) =
        Pubkey::find_program_address(&[RAYDIUM_OBSERVATION_SEED, pool.as_ref()], &program);

    DeterministicVenueContentionFootprint::new([
        pool_id.to_owned(),
        snapshot.pool_state.token_0_vault.clone(),
        snapshot.pool_state.token_1_vault.clone(),
        observation.to_string(),
    ])
}

pub fn pumpswap_contention_footprint(
    pool_id: &str,
    snapshot: &PumpSwapHydrationSnapshot,
) -> Result<DeterministicVenueContentionFootprint, String> {
    DeterministicVenueContentionFootprint::new([
        pool_id.to_owned(),
        snapshot.pool_state.pool_base_token_account.clone(),
        snapshot.pool_state.pool_quote_token_account.clone(),
    ])
}

pub fn route_contention_footprint(
    leg_1_venue: Venue,
    leg_1_pool_id: &str,
    leg_1_raydium: Option<&RaydiumHydrationSnapshot>,
    leg_1_pumpswap: Option<&PumpSwapHydrationSnapshot>,
    leg_2_venue: Venue,
    leg_2_pool_id: &str,
    leg_2_raydium: Option<&RaydiumHydrationSnapshot>,
    leg_2_pumpswap: Option<&PumpSwapHydrationSnapshot>,
) -> Result<DeterministicVenueContentionFootprint, String> {
    let leg_1 = venue_contention_footprint(
        leg_1_venue,
        leg_1_pool_id,
        leg_1_raydium,
        leg_1_pumpswap,
    )?;
    let leg_2 = venue_contention_footprint(
        leg_2_venue,
        leg_2_pool_id,
        leg_2_raydium,
        leg_2_pumpswap,
    )?;

    DeterministicVenueContentionFootprint::new(
        leg_1
            .accounts()
            .iter()
            .chain(leg_2.accounts().iter())
            .cloned(),
    )
}

fn venue_contention_footprint(
    venue: Venue,
    pool_id: &str,
    raydium_snapshot: Option<&RaydiumHydrationSnapshot>,
    pumpswap_snapshot: Option<&PumpSwapHydrationSnapshot>,
) -> Result<DeterministicVenueContentionFootprint, String> {
    match venue {
        Venue::RaydiumCpmm => raydium_contention_footprint(
            pool_id,
            raydium_snapshot.ok_or_else(|| {
                format!(
                    "missing Raydium hydration snapshot for localized priority scope: pool={pool_id}"
                )
            })?,
        ),
        Venue::PumpSwap => pumpswap_contention_footprint(
            pool_id,
            pumpswap_snapshot.ok_or_else(|| {
                format!(
                    "missing PumpSwap hydration snapshot for localized priority scope: pool={pool_id}"
                )
            })?,
        ),
        Venue::Meteora | Venue::Orca => Err(format!(
            "unsupported venue for Rung 11C localized priority scope: venue={venue:?}"
        )),
    }
}

pub fn localized_priority_fee_request(
    footprint: &DeterministicVenueContentionFootprint,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": PRIORITY_FEE_RPC_REQUEST_ID,
        "method": "getRecentPrioritizationFees",
        "params": [footprint.accounts()]
    })
}

pub fn parse_localized_priority_fee_response(
    payload: &Value,
    footprint: &DeterministicVenueContentionFootprint,
) -> Result<PriorityFeeObservation, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "getRecentPrioritizationFees returned an RPC error: {error}"
        ));
    }

    let jsonrpc = payload
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| "priority-fee response missing jsonrpc version".to_owned())?;

    if jsonrpc != "2.0" {
        return Err(format!(
            "priority-fee response has unexpected jsonrpc version: {jsonrpc}"
        ));
    }

    let response_id = payload
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "priority-fee response missing numeric id".to_owned())?;

    if response_id != PRIORITY_FEE_RPC_REQUEST_ID {
        return Err(format!(
            "priority-fee response id mismatch: expected={PRIORITY_FEE_RPC_REQUEST_ID} actual={response_id}"
        ));
    }

    let result = payload
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| "priority-fee response missing result array".to_owned())?;

    let mut samples = Vec::with_capacity(result.len());

    for row in result {
        let slot = row
            .get("slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| "priority-fee observation missing slot".to_owned())?;

        let micro_lamports_per_cu = row
            .get("prioritizationFee")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!(
                    "priority-fee observation at slot {slot} missing prioritizationFee"
                )
            })?;

        samples.push(PriorityFeeObservationSample {
            slot,
            micro_lamports_per_cu,
        });
    }

    Ok(PriorityFeeObservation {
        samples,
        scope_accounts: footprint.accounts().to_vec(),
        scope_provenance: footprint.provenance().to_owned(),
    })
}

pub fn priority_fee_lamports_for_price(
    micro_lamports_per_cu: u64,
    compute_unit_limit: u64,
) -> Result<u64, String> {
    if compute_unit_limit == 0 {
        return Err("priority-fee compute-unit limit must be greater than zero".to_owned());
    }

    let numerator = micro_lamports_per_cu
        .checked_mul(compute_unit_limit)
        .ok_or_else(|| "priority-fee multiplication overflow".to_owned())?;

    let quotient = numerator / MICRO_LAMPORTS_PER_LAMPORT;
    let remainder = numerator % MICRO_LAMPORTS_PER_LAMPORT;

    if remainder == 0 {
        Ok(quotient)
    } else {
        quotient
            .checked_add(1)
            .ok_or_else(|| "priority-fee ceiling division overflow".to_owned())
    }
}

pub fn modeled_base_fee_lamports() -> Result<u64, String> {
    BASE_FEE_LAMPORTS_PER_SIGNATURE
        .checked_mul(MODELED_SIGNATURE_COUNT)
        .ok_or_else(|| "modeled base-fee multiplication overflow".to_owned())
}

pub fn economics_cost_model(
    anchor_mint: &str,
    anchor_decimals: u8,
    priority_observation: &PriorityObservationState,
) -> Result<EconomicsCostModel, String> {
    EconomicsCostModel::new(
        RUNG11_V0_BASIS_ID,
        CommonEconomicsCosts {
            base_fee: modeled_base_fee_cost(anchor_mint, anchor_decimals)?,
            priority_fee: priority_fee_unknown(priority_observation)?,
            submission_cost: RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                "submission_cost unknown: no submission/Jito policy has been adopted",
            )?,
            expected_failure_cost: RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                "expected_failure_cost unknown: no execution failure-rate model has been adopted",
            )?,
            safety_reserve: RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                "safety_reserve unknown: no execution safety-reserve policy has been adopted",
            )?,
        },
        TreasuryFundingCosts {
            capital_cost: RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                "treasury_capital_cost unknown: no treasury capital-cost policy has been adopted",
            )?,
        },
        FlashFundingCosts {
            borrowing_cost: RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                "flash_borrowing_cost unknown: no flash borrowing-cost model has been adopted",
            )?,
        },
    )
}

fn modeled_base_fee_cost(anchor_mint: &str, anchor_decimals: u8) -> Result<RequiredCost, String> {
    let base_fee_lamports = modeled_base_fee_lamports()?;

    match lamports_to_anchor_raw(base_fee_lamports, anchor_mint, anchor_decimals) {
        Ok(amount_anchor_raw) => RequiredCost::known(
            amount_anchor_raw,
            CostProvenanceKind::ModeledAssumption,
            format!(
                concat!(
                    "basis={} current_base_fee_lamports_per_signature={} ",
                    "modeled_signature_count={} anchor_conversion=WSOL-lamports-1:1"
                ),
                RUNG11_V0_BASIS_ID,
                BASE_FEE_LAMPORTS_PER_SIGNATURE,
                MODELED_SIGNATURE_COUNT
            ),
        ),
        Err(reason) => RequiredCost::unknown(
            CostProvenanceKind::ModeledAssumption,
            format!("base_fee unknown: {reason}"),
        ),
    }
}

fn priority_fee_unknown(
    priority_observation: &PriorityObservationState,
) -> Result<RequiredCost, String> {
    let reason = match priority_observation {
        PriorityObservationState::Available(observation) if observation.samples.is_empty() => {
            concat!(
                "priority_fee unknown: localized contention observation returned no matching ",
                "samples and no execution priority-price policy has been adopted"
            )
            .to_owned()
        }
        PriorityObservationState::Available(_) => concat!(
            "priority_fee unknown: localized contention observations available, ",
            "but no execution priority-price policy has been adopted"
        )
        .to_owned(),
        PriorityObservationState::Unavailable(reason) => format!(
            "priority_fee unknown: localized contention observation unavailable: {reason}"
        ),
    };

    RequiredCost::unknown(CostProvenanceKind::ModeledAssumption, reason)
}

fn lamports_to_anchor_raw(
    lamports: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
) -> Result<u64, String> {
    if anchor_mint == WRAPPED_SOL_MINT {
        if anchor_decimals != 9 {
            return Err(format!(
                "WSOL anchor expected 9 decimals, got {anchor_decimals}"
            ));
        }

        return Ok(lamports);
    }

    if anchor_mint == USDC_MINT || anchor_mint == USDT_MINT {
        return Err(format!(
            concat!(
                "stablecoin external-cost conversion is not authorized for anchor {}; ",
                "SOL-denominated network costs cannot be converted to stablecoin raw units ",
                "without an explicit stablecoin/USD conversion or parity model"
            ),
            anchor_mint
        ));
    }

    Err(format!(
        "unsupported Rung 11 external-cost anchor mint {anchor_mint}"
    ))
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_POOL: &str = "11111111111111111111111111111111";
    const TEST_VAULT_A: &str = "SysvarC1ock11111111111111111111111111111111";
    const TEST_VAULT_B: &str = "SysvarRent111111111111111111111111111111111";
    const TEST_VAULT_C: &str = "Vote111111111111111111111111111111111111111";
    const TEST_VAULT_D: &str = "Stake11111111111111111111111111111111111111";

    #[test]
    fn footprint_sorts_and_deduplicates_accounts() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new([
            TEST_VAULT_B.to_owned(),
            TEST_POOL.to_owned(),
            TEST_VAULT_A.to_owned(),
            TEST_POOL.to_owned(),
        ])?;

        assert_eq!(footprint.accounts().len(), 3);
        assert!(footprint
            .accounts()
            .windows(2)
            .all(|pair| pair[0] < pair[1]));

        Ok(())
    }

    #[test]
    fn footprint_rejects_invalid_pubkey() {
        let result =
            DeterministicVenueContentionFootprint::new(["not-a-solana-pubkey".to_owned()]);
        assert!(result.is_err());
    }

    #[test]
    fn localized_request_uses_deterministic_account_scope() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new([
            TEST_VAULT_A.to_owned(),
            TEST_VAULT_B.to_owned(),
        ])?;

        let request = localized_priority_fee_request(&footprint);

        assert_eq!(request["jsonrpc"], "2.0");
        assert_eq!(request["id"], PRIORITY_FEE_RPC_REQUEST_ID);
        assert_eq!(request["method"], "getRecentPrioritizationFees");
        assert_eq!(
            request["params"][0],
            serde_json::to_value(footprint.accounts()).map_err(|error| error.to_string())?
        );

        Ok(())
    }

    #[test]
    fn parser_preserves_zero_and_nonzero_samples() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new([
            TEST_VAULT_A.to_owned(),
            TEST_VAULT_B.to_owned(),
        ])?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID,
            "result": [
                {"slot": 100, "prioritizationFee": 0},
                {"slot": 101, "prioritizationFee": 2500}
            ]
        });

        let observation = parse_localized_priority_fee_response(&payload, &footprint)?;

        assert_eq!(observation.samples.len(), 2);
        assert_eq!(observation.samples[0].micro_lamports_per_cu, 0);
        assert_eq!(observation.samples[1].micro_lamports_per_cu, 2_500);

        Ok(())
    }

    #[test]
    fn parser_accepts_empty_result_as_valid_observation() -> Result<(), String> {
        let footprint =
            DeterministicVenueContentionFootprint::new([TEST_VAULT_A.to_owned()])?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID,
            "result": []
        });

        let observation = parse_localized_priority_fee_response(&payload, &footprint)?;
        assert!(observation.samples.is_empty());

        Ok(())
    }

    #[test]
    fn parser_rejects_rpc_error() -> Result<(), String> {
        let footprint =
            DeterministicVenueContentionFootprint::new([TEST_VAULT_A.to_owned()])?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID,
            "error": {"code": -32602, "message": "invalid params"}
        });

        assert!(parse_localized_priority_fee_response(&payload, &footprint).is_err());
        Ok(())
    }

    #[test]
    fn parser_rejects_wrong_response_id() -> Result<(), String> {
        let footprint =
            DeterministicVenueContentionFootprint::new([TEST_VAULT_A.to_owned()])?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID + 1,
            "result": []
        });

        assert!(parse_localized_priority_fee_response(&payload, &footprint).is_err());
        Ok(())
    }

    #[test]
    fn parser_rejects_malformed_sample() -> Result<(), String> {
        let footprint =
            DeterministicVenueContentionFootprint::new([TEST_VAULT_A.to_owned()])?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID,
            "result": [{"slot": 100}]
        });

        assert!(parse_localized_priority_fee_response(&payload, &footprint).is_err());
        Ok(())
    }

    #[test]
    fn priority_fee_arithmetic_uses_ceiling_division() -> Result<(), String> {
        assert_eq!(priority_fee_lamports_for_price(1, 600_000)?, 1);
        assert_eq!(
            priority_fee_lamports_for_price(1_000_000, 600_000)?,
            600_000
        );
        assert_eq!(priority_fee_lamports_for_price(2_500, 600_000)?, 1_500);

        Ok(())
    }

    #[test]
    fn priority_fee_arithmetic_rejects_zero_compute_limit() {
        assert!(priority_fee_lamports_for_price(1, 0).is_err());
    }

    #[test]
    fn priority_fee_arithmetic_detects_overflow() {
        assert!(priority_fee_lamports_for_price(u64::MAX, u64::MAX).is_err());
    }

    #[test]
    fn modeled_transaction_shape_is_explicit() -> Result<(), String> {
        assert_eq!(RUNG11_V0_BASIS_ID, "rung11-v0-single-signer-600k-cu");
        assert_eq!(MODELED_SIGNATURE_COUNT, 1);
        assert_eq!(MODELED_COMPUTE_UNIT_LIMIT, 600_000);
        assert_eq!(modeled_base_fee_lamports()?, 5_000);

        Ok(())
    }

    #[test]
    fn wsol_external_cost_conversion_is_one_to_one() -> Result<(), String> {
        assert_eq!(lamports_to_anchor_raw(5_000, WRAPPED_SOL_MINT, 9)?, 5_000);
        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_conversion_fails_closed() {
        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6).is_err());
        assert!(lamports_to_anchor_raw(5_000, USDT_MINT, 6).is_err());
    }

    #[test]
    fn localized_observation_does_not_become_known_priority_cost() -> Result<(), String> {
        let observation = PriorityObservationState::Available(PriorityFeeObservation {
            samples: vec![PriorityFeeObservationSample {
                slot: 100,
                micro_lamports_per_cu: 10_000,
            }],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "test localized venue contention scope".to_owned(),
        });

        let model = economics_cost_model(WRAPPED_SOL_MINT, 9, &observation)?;

        assert!(matches!(
            model.common.priority_fee,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn empty_observation_does_not_become_zero_priority_cost() -> Result<(), String> {
        let observation = PriorityObservationState::Available(PriorityFeeObservation {
            samples: Vec::new(),
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "test localized venue contention scope".to_owned(),
        });

        let model = economics_cost_model(WRAPPED_SOL_MINT, 9, &observation)?;

        assert!(matches!(
            model.common.priority_fee,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn unresolved_external_costs_remain_explicitly_unknown() -> Result<(), String> {
        let model = economics_cost_model(
            WRAPPED_SOL_MINT,
            9,
            &PriorityObservationState::Unavailable("fixture unavailable".to_owned()),
        )?;

        assert!(matches!(
            model.common.priority_fee,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.common.submission_cost,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.common.expected_failure_cost,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.common.safety_reserve,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.treasury.capital_cost,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.flash.borrowing_cost,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn stablecoin_base_fee_is_unknown_instead_of_assuming_parity() -> Result<(), String> {
        let model = economics_cost_model(
            USDC_MINT,
            6,
            &PriorityObservationState::Unavailable("fixture unavailable".to_owned()),
        )?;

        assert!(matches!(model.common.base_fee, RequiredCost::Unknown { .. }));
        Ok(())
    }

    #[test]
    fn test_fixture_pubkeys_are_valid() -> Result<(), String> {
        for key in [
            TEST_POOL,
            TEST_VAULT_A,
            TEST_VAULT_B,
            TEST_VAULT_C,
            TEST_VAULT_D,
        ] {
            Pubkey::from_str(key)
                .map_err(|error| format!("invalid test fixture pubkey {key}: {error}"))?;
        }

        Ok(())
    }
}

