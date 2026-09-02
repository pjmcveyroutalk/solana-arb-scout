use crate::economics::{
    CommonEconomicsCosts, CostProvenanceKind, EconomicsCostModel, FlashFundingCosts, RequiredCost,
    TreasuryFundingCosts,
};
use crate::raydium::{RaydiumHydrationSnapshot, RAYDIUM_CPMM_PROGRAM_ID};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use crate::sizing::SolUsdPrice;
use scout_core::Venue;
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::collections::BTreeSet;
use std::str::FromStr;

pub const RUNG11_V0_BASIS_ID: &str = "rung11-v0-single-signer-600k-cu";
pub const PRIORITY_SELECTION_POLICY_ID: &str = "solana-local-p75-positive-v1";
pub const PROJECT0_FLASH_PROVIDER_BASIS_ID: &str = "project0-flashloan-zero-protocol-fee-v1";
pub const MODELED_SIGNATURE_COUNT: u64 = 1;
pub const MODELED_COMPUTE_UNIT_LIMIT: u64 = 600_000;
pub const BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;
pub const JITO_TIP_FLOOR_URL: &str = "https://bundles.jito.wtf/api/v1/bundles/tip_floor";
pub const PUMPSWAP_CONTENTION_POLICY_REASON: &str = concat!(
    "PumpSwap localized priority scope incomplete: deterministic recipient-selection ",
    "execution policy is not defined for the protocol/buyback fee recipient accounts ",
    "required by the current PumpSwap swap contract"
);

const MICRO_LAMPORTS_PER_LAMPORT: u64 = 1_000_000;
const PRIORITY_FEE_RPC_REQUEST_ID: u64 = 11;
const MAX_LOCALIZED_PRIORITY_ACCOUNTS: usize = 128;
const RAYDIUM_OBSERVATION_SEED: &[u8] = b"observation";
const LAMPORTS_PER_SOL_DECIMALS: i32 = 9;
const MAX_DECIMAL_POWER: u32 = 38;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DeterministicVenueContentionFootprint {
    accounts: Vec<String>,
    provenance: String,
}

impl DeterministicVenueContentionFootprint {
    fn new_with_provenance(
        accounts: impl IntoIterator<Item = String>,
        provenance: impl Into<String>,
    ) -> Result<Self, String> {
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
                format!(
                    "localized priority-fee contention account is invalid: account={account} error={error}"
                )
            })?;
        }

        let provenance = provenance.into();
        if provenance.trim().is_empty() {
            return Err(
                "localized priority-fee contention provenance must not be empty".to_owned(),
            );
        }

        Ok(Self {
            accounts: accounts.into_iter().collect(),
            provenance,
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
                concat!(
                    "sample_count=0 zero_count=0 role=observed scope_account_count={} ",
                    "scope_provenance={}"
                ),
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
                "max_micro_lamports_per_cu={} role=observed scope_account_count={} ",
                "scope_provenance={}"
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
pub struct PriorityFeeSelection {
    pub selected_micro_lamports_per_cu: u64,
    pub total_sample_count: usize,
    pub positive_sample_count: usize,
    pub min_slot: u64,
    pub max_slot: u64,
    pub policy_id: &'static str,
}

impl PriorityFeeSelection {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "policy={} selected_micro_lamports_per_cu={} total_sample_count={} ",
                "positive_sample_count={} slot_range={}..={} role=modeled_assumption"
            ),
            self.policy_id,
            self.selected_micro_lamports_per_cu,
            self.total_sample_count,
            self.positive_sample_count,
            self.min_slot,
            self.max_slot
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PriorityObservationState {
    Available(PriorityFeeObservation),
    Unavailable(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitoTipFloorObservation {
    pub time: String,
    pub landed_tips_25th_lamports: u64,
    pub landed_tips_50th_lamports: u64,
    pub landed_tips_75th_lamports: u64,
    pub landed_tips_95th_lamports: u64,
    pub landed_tips_99th_lamports: u64,
    pub ema_landed_tips_50th_lamports: u64,
}

impl JitoTipFloorObservation {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "time={} p25_lamports={} p50_lamports={} p75_lamports={} ",
                "p95_lamports={} p99_lamports={} ema_p50_lamports={} ",
                "source=Jito-public-tip-floor role=observed-market-telemetry-only"
            ),
            self.time,
            self.landed_tips_25th_lamports,
            self.landed_tips_50th_lamports,
            self.landed_tips_75th_lamports,
            self.landed_tips_95th_lamports,
            self.landed_tips_99th_lamports,
            self.ema_landed_tips_50th_lamports
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum JitoObservationState {
    Available(JitoTipFloorObservation),
    Unavailable(String),
}

#[derive(Debug, Clone, Copy)]
pub struct ExternalCostUsdPrices<'a> {
    sol_usd: &'a SolUsdPrice,
    usdc_usd: Option<&'a SolUsdPrice>,
    usdt_usd: Option<&'a SolUsdPrice>,
}

impl<'a> ExternalCostUsdPrices<'a> {
    pub fn new(
        sol_usd: &'a SolUsdPrice,
        usdc_usd: Option<&'a SolUsdPrice>,
        usdt_usd: Option<&'a SolUsdPrice>,
    ) -> Self {
        Self {
            sol_usd,
            usdc_usd,
            usdt_usd,
        }
    }
}

pub fn raydium_contention_footprint(
    pool_id: &str,
    snapshot: &RaydiumHydrationSnapshot,
) -> Result<DeterministicVenueContentionFootprint, String> {
    raydium_contention_footprint_from_fields(
        pool_id,
        &snapshot.pool_state.token_0_vault,
        &snapshot.pool_state.token_1_vault,
        &snapshot.pool_state.observation_key,
    )
}

fn raydium_contention_footprint_from_fields(
    pool_id: &str,
    token_0_vault: &str,
    token_1_vault: &str,
    stored_observation_key: &str,
) -> Result<DeterministicVenueContentionFootprint, String> {
    let pool = Pubkey::from_str(pool_id)
        .map_err(|error| format!("invalid Raydium pool id {pool_id}: {error}"))?;
    let program = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID).map_err(|error| {
        format!("invalid configured Raydium CPMM program id {RAYDIUM_CPMM_PROGRAM_ID}: {error}")
    })?;

    let (derived_observation, _) =
        Pubkey::find_program_address(&[RAYDIUM_OBSERVATION_SEED, pool.as_ref()], &program);
    let derived_observation = derived_observation.to_string();

    if stored_observation_key != derived_observation.as_str() {
        return Err(format!(
            concat!(
                "Raydium observation account verification failed: pool={} ",
                "stored_observation_key={} derived_observation_key={}"
            ),
            pool_id, stored_observation_key, derived_observation
        ));
    }

    DeterministicVenueContentionFootprint::new_with_provenance(
        [
            pool_id.to_owned(),
            token_0_vault.to_owned(),
            token_1_vault.to_owned(),
            derived_observation,
        ],
        concat!(
            "Raydium CPMM deterministic writable contention subset: pool_state, ",
            "token_0_vault, token_1_vault, verified observation_state; ",
            "not the complete future transaction writable set; executor-dependent ",
            "user token accounts are excluded"
        ),
    )
}

pub fn pumpswap_contention_footprint(
    pool_id: &str,
) -> Result<DeterministicVenueContentionFootprint, String> {
    Pubkey::from_str(pool_id)
        .map_err(|error| format!("invalid PumpSwap pool id {pool_id}: {error}"))?;

    Err(PUMPSWAP_CONTENTION_POLICY_REASON.to_owned())
}

#[derive(Debug, Clone, Copy)]
pub struct VenueContentionInput<'a> {
    pub venue: Venue,
    pub pool_id: &'a str,
    pub raydium_snapshot: Option<&'a RaydiumHydrationSnapshot>,
}

pub fn route_contention_footprint(
    leg_1_input: VenueContentionInput<'_>,
    leg_2_input: VenueContentionInput<'_>,
) -> Result<DeterministicVenueContentionFootprint, String> {
    let leg_1 = venue_contention_footprint(leg_1_input)?;
    let leg_2 = venue_contention_footprint(leg_2_input)?;

    DeterministicVenueContentionFootprint::new_with_provenance(
        leg_1
            .accounts()
            .iter()
            .chain(leg_2.accounts().iter())
            .cloned(),
        format!(
            concat!(
                "two-leg deterministic venue contention union; leg1=[{}]; leg2=[{}]; ",
                "not the complete future transaction writable set"
            ),
            leg_1.provenance(),
            leg_2.provenance()
        ),
    )
}

fn venue_contention_footprint(
    input: VenueContentionInput<'_>,
) -> Result<DeterministicVenueContentionFootprint, String> {
    match input.venue {
        Venue::RaydiumCpmm => raydium_contention_footprint(
            input.pool_id,
            input.raydium_snapshot.ok_or_else(|| {
                format!(
                    "missing Raydium hydration snapshot for localized priority scope: pool={}",
                    input.pool_id
                )
            })?,
        ),
        Venue::PumpSwap => pumpswap_contention_footprint(input.pool_id),
        Venue::Meteora | Venue::Orca => Err(format!(
            "unsupported venue for Rung 11C localized priority scope: venue={:?}",
            input.venue
        )),
    }
}

pub fn localized_priority_fee_request(footprint: &DeterministicVenueContentionFootprint) -> Value {
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
                format!("priority-fee observation at slot {slot} missing prioritizationFee")
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

pub fn select_priority_fee(
    observation: &PriorityFeeObservation,
) -> Result<Option<PriorityFeeSelection>, String> {
    if observation.samples.is_empty() {
        return Ok(None);
    }

    let min_slot = observation
        .samples
        .iter()
        .map(|sample| sample.slot)
        .min()
        .ok_or_else(|| "priority observation unexpectedly lacked a minimum slot".to_owned())?;
    let max_slot = observation
        .samples
        .iter()
        .map(|sample| sample.slot)
        .max()
        .ok_or_else(|| "priority observation unexpectedly lacked a maximum slot".to_owned())?;

    let mut positives = observation
        .samples
        .iter()
        .filter_map(|sample| {
            (sample.micro_lamports_per_cu > 0).then_some(sample.micro_lamports_per_cu)
        })
        .collect::<Vec<_>>();

    if positives.is_empty() {
        return Ok(None);
    }

    positives.sort_unstable();

    let rank_numerator = positives
        .len()
        .checked_mul(3)
        .ok_or_else(|| "priority p75 rank multiplication overflow".to_owned())?;
    let rank = rank_numerator.div_ceil(4);

    if rank == 0 || rank > positives.len() {
        return Err("priority p75 rank fell outside positive sample range".to_owned());
    }

    let selected_index = rank
        .checked_sub(1)
        .ok_or_else(|| "priority p75 selected index underflow".to_owned())?;
    let selected_micro_lamports_per_cu = *positives
        .get(selected_index)
        .ok_or_else(|| "priority p75 selected index fell outside positive samples".to_owned())?;

    Ok(Some(PriorityFeeSelection {
        selected_micro_lamports_per_cu,
        total_sample_count: observation.samples.len(),
        positive_sample_count: positives.len(),
        min_slot,
        max_slot,
        policy_id: PRIORITY_SELECTION_POLICY_ID,
    }))
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

pub fn parse_jito_tip_floor_response(payload: &Value) -> Result<JitoTipFloorObservation, String> {
    let rows = payload
        .as_array()
        .ok_or_else(|| "Jito tip-floor response must be a JSON array".to_owned())?;
    let row = rows
        .first()
        .ok_or_else(|| "Jito tip-floor response contained no rows".to_owned())?;

    let time = match row.get("time") {
        Some(Value::String(value)) if !value.trim().is_empty() => value.clone(),
        Some(Value::Number(value)) => value.to_string(),
        Some(_) => return Err("Jito tip-floor time has unsupported type".to_owned()),
        None => return Err("Jito tip-floor response missing time".to_owned()),
    };

    Ok(JitoTipFloorObservation {
        time,
        landed_tips_25th_lamports: jito_sol_field_to_lamports(row, "landed_tips_25th_percentile")?,
        landed_tips_50th_lamports: jito_sol_field_to_lamports(row, "landed_tips_50th_percentile")?,
        landed_tips_75th_lamports: jito_sol_field_to_lamports(row, "landed_tips_75th_percentile")?,
        landed_tips_95th_lamports: jito_sol_field_to_lamports(row, "landed_tips_95th_percentile")?,
        landed_tips_99th_lamports: jito_sol_field_to_lamports(row, "landed_tips_99th_percentile")?,
        ema_landed_tips_50th_lamports: jito_sol_field_to_lamports(
            row,
            "ema_landed_tips_50th_percentile",
        )?,
    })
}

fn jito_sol_field_to_lamports(row: &Value, field: &str) -> Result<u64, String> {
    let value = row
        .get(field)
        .ok_or_else(|| format!("Jito tip-floor response missing {field}"))?;

    let text = match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) if !text.trim().is_empty() => text.clone(),
        _ => {
            return Err(format!("Jito tip-floor field {field} must be numeric SOL"));
        }
    };

    decimal_sol_to_lamports(&text)
        .map_err(|error| format!("Jito tip-floor field {field} is invalid: {error}"))
}

fn decimal_sol_to_lamports(text: &str) -> Result<u64, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("SOL amount must not be empty".to_owned());
    }

    if text.starts_with('-') {
        return Err("SOL amount must not be negative".to_owned());
    }

    let unsigned = text.strip_prefix('+').unwrap_or(text);
    let (mantissa, exponent) = split_exponent(unsigned)?;

    let mut pieces = mantissa.split('.');
    let integer_part = pieces
        .next()
        .ok_or_else(|| "SOL amount missing mantissa".to_owned())?;
    let fractional_part = pieces.next().unwrap_or("");

    if pieces.next().is_some() {
        return Err("SOL amount contains multiple decimal points".to_owned());
    }

    if integer_part.is_empty() && fractional_part.is_empty() {
        return Err("SOL amount contains no digits".to_owned());
    }

    if !integer_part
        .chars()
        .all(|character| character.is_ascii_digit())
        || !fractional_part
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err("SOL amount contains non-decimal digits".to_owned());
    }

    let digits_text = format!("{integer_part}{fractional_part}");
    let digits = if digits_text.is_empty() {
        0u128
    } else {
        digits_text
            .parse::<u128>()
            .map_err(|error| format!("SOL amount digits exceed u128: {error}"))?
    };

    let fractional_digits = i32::try_from(fractional_part.len())
        .map_err(|_| "SOL fractional digit count exceeds i32".to_owned())?;
    let scale = exponent
        .checked_sub(fractional_digits)
        .and_then(|value| value.checked_add(LAMPORTS_PER_SOL_DECIMALS))
        .ok_or_else(|| "SOL-to-lamport decimal scale overflow".to_owned())?;

    let lamports_u128 = if scale >= 0 {
        let scale = u32::try_from(scale)
            .map_err(|_| "SOL positive decimal scale conversion failed".to_owned())?;
        digits
            .checked_mul(checked_pow10_u128(scale)?)
            .ok_or_else(|| "SOL-to-lamport multiplication overflow".to_owned())?
    } else {
        let divisor_scale = scale
            .checked_neg()
            .ok_or_else(|| "SOL negative decimal scale overflow".to_owned())?;
        let divisor_scale = u32::try_from(divisor_scale)
            .map_err(|_| "SOL negative decimal scale conversion failed".to_owned())?;
        let divisor = checked_pow10_u128(divisor_scale)?;
        let quotient = digits / divisor;
        let remainder = digits % divisor;

        let doubled_remainder = remainder
            .checked_mul(2)
            .ok_or_else(|| "SOL-to-lamport rounding overflow".to_owned())?;

        if doubled_remainder >= divisor {
            quotient
                .checked_add(1)
                .ok_or_else(|| "SOL-to-lamport rounding increment overflow".to_owned())?
        } else {
            quotient
        }
    };

    u64::try_from(lamports_u128).map_err(|_| "SOL amount exceeds u64 lamports".to_owned())
}

fn split_exponent(text: &str) -> Result<(&str, i32), String> {
    let mut split_index = None;

    for (index, character) in text.char_indices() {
        if character == 'e' || character == 'E' {
            if split_index.is_some() {
                return Err("SOL amount contains multiple exponents".to_owned());
            }
            split_index = Some(index);
        }
    }

    let Some(index) = split_index else {
        return Ok((text, 0));
    };

    let mantissa = &text[..index];
    let exponent_text = &text[index + 1..];

    if exponent_text.is_empty() {
        return Err("SOL amount exponent is empty".to_owned());
    }

    let exponent = exponent_text
        .parse::<i32>()
        .map_err(|error| format!("SOL amount exponent is invalid: {error}"))?;

    Ok((mantissa, exponent))
}

fn checked_pow10_u128(exponent: u32) -> Result<u128, String> {
    if exponent > MAX_DECIMAL_POWER {
        return Err(format!(
            "decimal power-of-ten exponent exceeds u128-safe bound: {exponent}"
        ));
    }

    let mut value = 1u128;

    for _ in 0..exponent {
        value = value
            .checked_mul(10)
            .ok_or_else(|| "decimal power-of-ten overflow".to_owned())?;
    }

    Ok(value)
}

#[cfg(test)]
pub fn economics_cost_model(
    anchor_mint: &str,
    anchor_decimals: u8,
    priority_observation: &PriorityObservationState,
    jito_observation: &JitoObservationState,
) -> Result<EconomicsCostModel, String> {
    economics_cost_model_with_usd_prices(
        anchor_mint,
        anchor_decimals,
        priority_observation,
        jito_observation,
        None,
    )
}

pub fn economics_cost_model_with_usd_prices(
    anchor_mint: &str,
    anchor_decimals: u8,
    priority_observation: &PriorityObservationState,
    jito_observation: &JitoObservationState,
    usd_prices: Option<&ExternalCostUsdPrices<'_>>,
) -> Result<EconomicsCostModel, String> {
    EconomicsCostModel::new(
        RUNG11_V0_BASIS_ID,
        CommonEconomicsCosts {
            base_fee: modeled_base_fee_cost(anchor_mint, anchor_decimals, usd_prices)?,
            priority_fee: modeled_priority_fee_cost(
                anchor_mint,
                anchor_decimals,
                priority_observation,
                usd_prices,
            )?,
            submission_cost: submission_cost_unknown(jito_observation)?,
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
            borrowing_cost: project0_flash_borrowing_cost()?,
        },
    )
}

fn modeled_base_fee_cost(
    anchor_mint: &str,
    anchor_decimals: u8,
    usd_prices: Option<&ExternalCostUsdPrices<'_>>,
) -> Result<RequiredCost, String> {
    let base_fee_lamports = modeled_base_fee_lamports()?;

    match lamports_to_anchor_raw(base_fee_lamports, anchor_mint, anchor_decimals, usd_prices) {
        Ok(amount_anchor_raw) => RequiredCost::known(
            amount_anchor_raw,
            CostProvenanceKind::ModeledAssumption,
            format!(
                concat!(
                    "basis={} current_base_fee_lamports_per_signature={} ",
                    "modeled_signature_count={} anchor_conversion={}"
                ),
                RUNG11_V0_BASIS_ID,
                BASE_FEE_LAMPORTS_PER_SIGNATURE,
                MODELED_SIGNATURE_COUNT,
                anchor_conversion_provenance(anchor_mint)
            ),
        ),
        Err(reason) => RequiredCost::unknown(
            CostProvenanceKind::ModeledAssumption,
            format!("base_fee unknown: {reason}"),
        ),
    }
}

fn modeled_priority_fee_cost(
    anchor_mint: &str,
    anchor_decimals: u8,
    priority_observation: &PriorityObservationState,
    usd_prices: Option<&ExternalCostUsdPrices<'_>>,
) -> Result<RequiredCost, String> {
    let observation = match priority_observation {
        PriorityObservationState::Available(observation) => observation,
        PriorityObservationState::Unavailable(reason) => {
            return RequiredCost::unknown(
                CostProvenanceKind::ModeledAssumption,
                format!(
                    "priority_fee unknown: localized contention observation unavailable: {reason}"
                ),
            );
        }
    };

    let Some(selection) = select_priority_fee(observation)? else {
        return RequiredCost::unknown(
            CostProvenanceKind::ModeledAssumption,
            concat!(
                "priority_fee unknown: localized contention observation contained no positive ",
                "priority-fee samples for the modeled selection policy"
            ),
        );
    };

    let priority_lamports = priority_fee_lamports_for_price(
        selection.selected_micro_lamports_per_cu,
        MODELED_COMPUTE_UNIT_LIMIT,
    )?;

    match lamports_to_anchor_raw(priority_lamports, anchor_mint, anchor_decimals, usd_prices) {
        Ok(amount_anchor_raw) => RequiredCost::known(
            amount_anchor_raw,
            CostProvenanceKind::ModeledAssumption,
            format!(
                concat!(
                    "basis={} policy={} selected_micro_lamports_per_cu={} modeled_cu_limit={} ",
                    "observed_total_samples={} observed_positive_samples={} ",
                    "observed_slot_range={}..={} ",
                    "scope_accounts={} scope_provenance={} anchor_conversion={}"
                ),
                RUNG11_V0_BASIS_ID,
                selection.policy_id,
                selection.selected_micro_lamports_per_cu,
                MODELED_COMPUTE_UNIT_LIMIT,
                selection.total_sample_count,
                selection.positive_sample_count,
                selection.min_slot,
                selection.max_slot,
                observation.scope_accounts.len(),
                observation.scope_provenance,
                anchor_conversion_provenance(anchor_mint)
            ),
        ),
        Err(reason) => RequiredCost::unknown(
            CostProvenanceKind::ModeledAssumption,
            format!("priority_fee unknown: {reason}"),
        ),
    }
}

fn anchor_conversion_provenance(anchor_mint: &str) -> &'static str {
    if anchor_mint == WRAPPED_SOL_MINT {
        "WSOL-lamports-1:1"
    } else {
        "Pyth-conservative-SOLUSD-cross-stableUSD-confidence-bounds-ceil"
    }
}

fn submission_cost_unknown(
    jito_observation: &JitoObservationState,
) -> Result<RequiredCost, String> {
    let reason = match jito_observation {
        JitoObservationState::Available(observation) => format!(
            concat!(
                "submission_cost unknown: Jito tip-floor telemetry is available at time={} ",
                "but no Rung 11C submission/tip bidding policy has been adopted"
            ),
            observation.time
        ),
        JitoObservationState::Unavailable(reason) => {
            format!("submission_cost unknown: Jito tip-floor telemetry unavailable: {reason}")
        }
    };

    RequiredCost::unknown(CostProvenanceKind::ModeledAssumption, reason)
}

fn project0_flash_borrowing_cost() -> Result<RequiredCost, String> {
    RequiredCost::known(
        0,
        CostProvenanceKind::ModeledAssumption,
        format!(
            concat!(
                "provider_basis={} provider=Project0 protocol_flashloan_fee_raw=0 ",
                "scope=provider-protocol-fee-only excludes=network-priority-submission-jito-",
                "failure-safety-reserve-execution-overhead source=Project0-official-documentation"
            ),
            PROJECT0_FLASH_PROVIDER_BASIS_ID
        ),
    )
}

fn lamports_to_anchor_raw(
    lamports: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    usd_prices: Option<&ExternalCostUsdPrices<'_>>,
) -> Result<u64, String> {
    if anchor_mint == WRAPPED_SOL_MINT {
        if anchor_decimals != 9 {
            return Err(format!(
                "WSOL anchor expected 9 decimals, got {anchor_decimals}"
            ));
        }

        return Ok(lamports);
    }

    if anchor_mint == USDC_MINT {
        if anchor_decimals != 6 {
            return Err(format!(
                "USDC anchor expected 6 decimals, got {anchor_decimals}"
            ));
        }

        let usd_prices = usd_prices.ok_or_else(|| {
            concat!(
                "stablecoin external-cost conversion is not active for USDC: ",
                "read-only SOL/USD and USDC/USD observations were not supplied"
            )
            .to_owned()
        })?;

        let stable_usd = usd_prices.usdc_usd.ok_or_else(|| {
            "stablecoin external-cost conversion is missing a USDC/USD observation".to_owned()
        })?;

        return conservative_lamports_to_stable_raw(
            lamports,
            anchor_decimals,
            usd_prices.sol_usd,
            stable_usd,
        );
    }

    if anchor_mint == USDT_MINT {
        if anchor_decimals != 6 {
            return Err(format!(
                "USDT anchor expected 6 decimals, got {anchor_decimals}"
            ));
        }

        let usd_prices = usd_prices.ok_or_else(|| {
            concat!(
                "stablecoin external-cost conversion is not active for USDT: ",
                "read-only SOL/USD and USDT/USD observations were not supplied"
            )
            .to_owned()
        })?;

        let stable_usd = usd_prices.usdt_usd.ok_or_else(|| {
            "stablecoin external-cost conversion is missing a USDT/USD observation".to_owned()
        })?;

        return conservative_lamports_to_stable_raw(
            lamports,
            anchor_decimals,
            usd_prices.sol_usd,
            stable_usd,
        );
    }

    Err(format!(
        "unsupported Rung 11 external-cost anchor mint {anchor_mint}"
    ))
}

fn conservative_lamports_to_stable_raw(
    lamports: u64,
    stable_decimals: u8,
    sol_usd: &SolUsdPrice,
    stable_usd: &SolUsdPrice,
) -> Result<u64, String> {
    if sol_usd.price == 0 {
        return Err("SOL/USD price must be greater than zero".to_owned());
    }

    if stable_usd.price == 0 {
        return Err("stablecoin/USD price must be greater than zero".to_owned());
    }

    let sol_upper = sol_usd
        .price
        .checked_add(sol_usd.confidence)
        .ok_or_else(|| "SOL/USD upper confidence bound overflow".to_owned())?;

    let stable_lower = stable_usd
        .price
        .checked_sub(stable_usd.confidence)
        .ok_or_else(|| "stablecoin/USD lower confidence bound underflow".to_owned())?;

    if stable_lower == 0 {
        return Err("stablecoin/USD lower confidence bound must be greater than zero".to_owned());
    }

    let token_scale = checked_pow10_u128(u32::from(stable_decimals))?;

    let mut numerator = u128::from(lamports)
        .checked_mul(u128::from(sol_upper))
        .and_then(|value| value.checked_mul(token_scale))
        .ok_or_else(|| "stablecoin external-cost numerator overflow".to_owned())?;

    let mut denominator = 1_000_000_000u128
        .checked_mul(u128::from(stable_lower))
        .ok_or_else(|| "stablecoin external-cost denominator overflow".to_owned())?;

    let exponent_delta = i64::from(sol_usd.exponent) - i64::from(stable_usd.exponent);

    match exponent_delta.cmp(&0) {
        std::cmp::Ordering::Greater => {
            let exponent = u32::try_from(exponent_delta)
                .map_err(|_| "positive USD exponent delta conversion failed".to_owned())?;
            numerator = numerator
                .checked_mul(checked_pow10_u128(exponent)?)
                .ok_or_else(|| "stablecoin external-cost exponent numerator overflow".to_owned())?;
        }
        std::cmp::Ordering::Less => {
            let magnitude = exponent_delta
                .checked_neg()
                .ok_or_else(|| "negative USD exponent delta overflow".to_owned())?;
            let exponent = u32::try_from(magnitude)
                .map_err(|_| "negative USD exponent delta conversion failed".to_owned())?;
            denominator = denominator
                .checked_mul(checked_pow10_u128(exponent)?)
                .ok_or_else(|| {
                    "stablecoin external-cost exponent denominator overflow".to_owned()
                })?;
        }
        std::cmp::Ordering::Equal => {}
    }

    let quotient = numerator / denominator;
    let remainder = numerator % denominator;

    let raw = if remainder == 0 {
        quotient
    } else {
        quotient
            .checked_add(1)
            .ok_or_else(|| "stablecoin external-cost ceiling division overflow".to_owned())?
    };

    u64::try_from(raw)
        .map_err(|_| "stablecoin external-cost conversion exceeded u64 raw units".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const TEST_POOL: &str = "11111111111111111111111111111111";
    const TEST_VAULT_A: &str = "SysvarC1ock11111111111111111111111111111111";
    const TEST_VAULT_B: &str = "SysvarRent111111111111111111111111111111111";

    fn usd_price(price: u64, confidence: u64, exponent: i32) -> SolUsdPrice {
        SolUsdPrice {
            price,
            confidence,
            exponent,
            publish_time: 1_700_000_000,
            posted_slot: 123_456,
            rpc_slot: 123_456,
        }
    }

    #[test]
    fn footprint_sorts_and_deduplicates_accounts() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new_with_provenance(
            [
                TEST_VAULT_B.to_owned(),
                TEST_POOL.to_owned(),
                TEST_VAULT_A.to_owned(),
                TEST_POOL.to_owned(),
            ],
            "fixture contention scope",
        )?;

        assert_eq!(footprint.accounts().len(), 3);
        assert!(footprint
            .accounts()
            .windows(2)
            .all(|pair| pair[0] < pair[1]));

        Ok(())
    }

    #[test]
    fn footprint_rejects_invalid_pubkey() {
        let result = DeterministicVenueContentionFootprint::new_with_provenance(
            ["not-a-solana-pubkey".to_owned()],
            "fixture contention scope",
        );
        assert!(result.is_err());
    }

    #[test]
    fn raydium_footprint_requires_stored_observation_key_to_match_pda() -> Result<(), String> {
        let pool = Pubkey::from_str(TEST_POOL)
            .map_err(|error| format!("invalid test pool pubkey: {error}"))?;
        let program = Pubkey::from_str(RAYDIUM_CPMM_PROGRAM_ID)
            .map_err(|error| format!("invalid Raydium program pubkey: {error}"))?;
        let (observation, _) =
            Pubkey::find_program_address(&[RAYDIUM_OBSERVATION_SEED, pool.as_ref()], &program);

        let observation_string = observation.to_string();
        let footprint = raydium_contention_footprint_from_fields(
            TEST_POOL,
            TEST_VAULT_A,
            TEST_VAULT_B,
            &observation_string,
        )?;

        assert_eq!(footprint.accounts().len(), 4);
        assert!(footprint
            .accounts()
            .iter()
            .any(|account| account == &observation_string));

        let mismatch = raydium_contention_footprint_from_fields(
            TEST_POOL,
            TEST_VAULT_A,
            TEST_VAULT_B,
            TEST_VAULT_A,
        );
        assert!(matches!(mismatch, Err(error) if error.contains("verification failed")));

        Ok(())
    }

    #[test]
    fn pumpswap_scope_fails_closed_with_exact_policy_reason() {
        let result = pumpswap_contention_footprint(TEST_POOL);

        assert_eq!(result, Err(PUMPSWAP_CONTENTION_POLICY_REASON.to_owned()));
    }

    #[test]
    fn localized_request_uses_deterministic_account_scope() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new_with_provenance(
            [TEST_VAULT_A.to_owned(), TEST_VAULT_B.to_owned()],
            "fixture contention scope",
        )?;

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
        let footprint = DeterministicVenueContentionFootprint::new_with_provenance(
            [TEST_VAULT_A.to_owned(), TEST_VAULT_B.to_owned()],
            "fixture contention scope",
        )?;

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
    fn p75_selection_uses_positive_samples_only_and_nearest_rank() -> Result<(), String> {
        let observation = PriorityFeeObservation {
            samples: vec![
                PriorityFeeObservationSample {
                    slot: 100,
                    micro_lamports_per_cu: 0,
                },
                PriorityFeeObservationSample {
                    slot: 101,
                    micro_lamports_per_cu: 10,
                },
                PriorityFeeObservationSample {
                    slot: 102,
                    micro_lamports_per_cu: 20,
                },
                PriorityFeeObservationSample {
                    slot: 103,
                    micro_lamports_per_cu: 30,
                },
                PriorityFeeObservationSample {
                    slot: 104,
                    micro_lamports_per_cu: 40,
                },
            ],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "fixture localized scope".to_owned(),
        };

        let selection = select_priority_fee(&observation)?
            .ok_or_else(|| "expected positive p75 selection".to_owned())?;

        assert_eq!(selection.selected_micro_lamports_per_cu, 30);
        assert_eq!(selection.total_sample_count, 5);
        assert_eq!(selection.positive_sample_count, 4);
        assert_eq!(selection.min_slot, 100);
        assert_eq!(selection.max_slot, 104);
        assert_eq!(selection.policy_id, PRIORITY_SELECTION_POLICY_ID);

        Ok(())
    }

    #[test]
    fn p75_selection_returns_none_when_all_samples_are_zero() -> Result<(), String> {
        let observation = PriorityFeeObservation {
            samples: vec![
                PriorityFeeObservationSample {
                    slot: 100,
                    micro_lamports_per_cu: 0,
                },
                PriorityFeeObservationSample {
                    slot: 101,
                    micro_lamports_per_cu: 0,
                },
            ],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "fixture localized scope".to_owned(),
        };

        assert_eq!(select_priority_fee(&observation)?, None);
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
    fn jito_parser_accepts_decimal_and_scientific_sol_values() -> Result<(), String> {
        let payload = json!([{
            "time": "2026-09-01T23:00:00Z",
            "landed_tips_25th_percentile": 0.000001,
            "landed_tips_50th_percentile": 2e-6,
            "landed_tips_75th_percentile": "0.000003",
            "landed_tips_95th_percentile": 0.000004,
            "landed_tips_99th_percentile": 5e-6,
            "ema_landed_tips_50th_percentile": 0.000006
        }]);

        let observation = parse_jito_tip_floor_response(&payload)?;

        assert_eq!(observation.landed_tips_25th_lamports, 1_000);
        assert_eq!(observation.landed_tips_50th_lamports, 2_000);
        assert_eq!(observation.landed_tips_75th_lamports, 3_000);
        assert_eq!(observation.landed_tips_95th_lamports, 4_000);
        assert_eq!(observation.landed_tips_99th_lamports, 5_000);
        assert_eq!(observation.ema_landed_tips_50th_lamports, 6_000);

        Ok(())
    }

    #[test]
    fn jito_parser_rejects_missing_field() {
        let payload = json!([{
            "time": "2026-09-01T23:00:00Z",
            "landed_tips_25th_percentile": 0.000001
        }]);

        assert!(parse_jito_tip_floor_response(&payload).is_err());
    }

    #[test]
    fn decimal_sol_conversion_rounds_to_nearest_lamport() -> Result<(), String> {
        assert_eq!(decimal_sol_to_lamports("0.0000000014")?, 1);
        assert_eq!(decimal_sol_to_lamports("0.0000000015")?, 2);
        assert_eq!(decimal_sol_to_lamports("6e-6")?, 6_000);
        Ok(())
    }

    #[test]
    fn decimal_sol_conversion_rejects_unbounded_exponents() {
        assert!(decimal_sol_to_lamports("1e100").is_err());
        assert!(decimal_sol_to_lamports("1e-100").is_err());
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
        assert_eq!(
            lamports_to_anchor_raw(5_000, WRAPPED_SOL_MINT, 9, None)?,
            5_000
        );
        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_conversion_fails_closed_without_prices() {
        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6, None).is_err());
        assert!(lamports_to_anchor_raw(5_000, USDT_MINT, 6, None).is_err());
    }

    #[test]
    fn usdc_external_cost_conversion_uses_observed_cross_price() -> Result<(), String> {
        let sol = usd_price(20_000_000_000, 0, -8);
        let usdc = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert_eq!(
            lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices))?,
            1_000
        );

        Ok(())
    }

    #[test]
    fn usdt_external_cost_conversion_respects_depeg() -> Result<(), String> {
        let sol = usd_price(20_000_000_000, 0, -8);
        let usdt = usd_price(80_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, None, Some(&usdt));

        assert_eq!(
            lamports_to_anchor_raw(5_000, USDT_MINT, 6, Some(&prices))?,
            1_250
        );

        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_conversion_uses_conservative_confidence_bounds(
    ) -> Result<(), String> {
        let sol = usd_price(20_000_000_000, 100_000_000, -8);
        let usdc = usd_price(100_000_000, 1_000_000, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert_eq!(
            lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices))?,
            1_016
        );

        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_conversion_normalizes_exponents() -> Result<(), String> {
        let sol = usd_price(20_000, 0, -2);
        let usdc = usd_price(1_000_000, 0, -6);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert_eq!(
            lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices))?,
            1_000
        );

        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_conversion_rejects_nonpositive_lower_bound() {
        let sol = usd_price(20_000_000_000, 0, -8);
        let usdc = usd_price(100_000_000, 100_000_000, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices)).is_err());
    }

    #[test]
    fn stablecoin_external_cost_conversion_requires_matching_feed() {
        let sol = usd_price(20_000_000_000, 0, -8);
        let usdt = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, None, Some(&usdt));

        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices)).is_err());
    }

    #[test]
    fn stablecoin_external_cost_conversion_rejects_wrong_decimals() {
        let sol = usd_price(20_000_000_000, 0, -8);
        let usdc = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 9, Some(&prices)).is_err());
        assert!(lamports_to_anchor_raw(5_000, USDT_MINT, 9, Some(&prices)).is_err());
    }

    #[test]
    fn stablecoin_external_cost_conversion_rejects_unbounded_exponent_delta() {
        let sol = usd_price(20_000_000_000, 0, 100);
        let usdc = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices)).is_err());
    }

    #[test]
    fn stablecoin_external_cost_conversion_rejects_sol_upper_bound_overflow() {
        let sol = usd_price(u64::MAX, 1, -8);
        let usdc = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        assert!(lamports_to_anchor_raw(5_000, USDC_MINT, 6, Some(&prices)).is_err());
    }

    #[test]
    fn localized_positive_observation_can_become_modeled_priority_cost() -> Result<(), String> {
        let observation = PriorityObservationState::Available(PriorityFeeObservation {
            samples: vec![
                PriorityFeeObservationSample {
                    slot: 100,
                    micro_lamports_per_cu: 0,
                },
                PriorityFeeObservationSample {
                    slot: 101,
                    micro_lamports_per_cu: 10_000,
                },
            ],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "fixture localized venue contention scope".to_owned(),
        });

        let model = economics_cost_model(
            WRAPPED_SOL_MINT,
            9,
            &observation,
            &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
        )?;

        match model.common.priority_fee {
            RequiredCost::Known(cost) => {
                assert_eq!(cost.amount_anchor_raw(), 6_000);
                assert_eq!(
                    cost.provenance_kind(),
                    CostProvenanceKind::ModeledAssumption
                );
                assert!(cost.provenance().contains(PRIORITY_SELECTION_POLICY_ID));
                assert!(cost.provenance().contains("WSOL-lamports-1:1"));
            }
            RequiredCost::Unknown { .. } => {
                return Err("expected modeled known priority fee".to_owned());
            }
        }

        Ok(())
    }

    #[test]
    fn stablecoin_cost_model_uses_observed_usd_conversion() -> Result<(), String> {
        let priority_observation = PriorityObservationState::Available(PriorityFeeObservation {
            samples: vec![PriorityFeeObservationSample {
                slot: 100,
                micro_lamports_per_cu: 10_000,
            }],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "fixture localized venue contention scope".to_owned(),
        });

        let sol = usd_price(20_000_000_000, 0, -8);
        let usdc = usd_price(100_000_000, 0, -8);
        let prices = ExternalCostUsdPrices::new(&sol, Some(&usdc), None);

        let model = economics_cost_model_with_usd_prices(
            USDC_MINT,
            6,
            &priority_observation,
            &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
            Some(&prices),
        )?;

        match model.common.base_fee {
            RequiredCost::Known(cost) => {
                assert_eq!(cost.amount_anchor_raw(), 1_000);
                assert!(cost
                    .provenance()
                    .contains("Pyth-conservative-SOLUSD-cross-stableUSD-confidence-bounds-ceil"));
            }
            RequiredCost::Unknown { .. } => {
                return Err("expected modeled known USDC base fee".to_owned());
            }
        }

        match model.common.priority_fee {
            RequiredCost::Known(cost) => {
                assert_eq!(cost.amount_anchor_raw(), 1_200);
                assert!(cost
                    .provenance()
                    .contains("Pyth-conservative-SOLUSD-cross-stableUSD-confidence-bounds-ceil"));
            }
            RequiredCost::Unknown { .. } => {
                return Err("expected modeled known USDC priority fee".to_owned());
            }
        }

        Ok(())
    }

    #[test]
    fn unavailable_scope_keeps_priority_cost_unknown() -> Result<(), String> {
        let model = economics_cost_model(
            WRAPPED_SOL_MINT,
            9,
            &PriorityObservationState::Unavailable(PUMPSWAP_CONTENTION_POLICY_REASON.to_owned()),
            &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
        )?;

        assert!(matches!(
            model.common.priority_fee,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn jito_observation_never_becomes_submission_cost_without_policy() -> Result<(), String> {
        let jito = JitoObservationState::Available(JitoTipFloorObservation {
            time: "fixture".to_owned(),
            landed_tips_25th_lamports: 1,
            landed_tips_50th_lamports: 2,
            landed_tips_75th_lamports: 3,
            landed_tips_95th_lamports: 4,
            landed_tips_99th_lamports: 5,
            ema_landed_tips_50th_lamports: 6,
        });

        let model = economics_cost_model(
            WRAPPED_SOL_MINT,
            9,
            &PriorityObservationState::Unavailable("fixture unavailable".to_owned()),
            &jito,
        )?;

        assert!(matches!(
            model.common.submission_cost,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn project0_flash_provider_cost_is_known_zero_and_anchor_independent() -> Result<(), String> {
        for (anchor_mint, anchor_decimals) in
            [(WRAPPED_SOL_MINT, 9), (USDC_MINT, 6), (USDT_MINT, 6)]
        {
            let model = economics_cost_model(
                anchor_mint,
                anchor_decimals,
                &PriorityObservationState::Unavailable("fixture unavailable".to_owned()),
                &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
            )?;

            match model.flash.borrowing_cost {
                RequiredCost::Known(cost) => {
                    assert_eq!(cost.amount_anchor_raw(), 0);
                    assert_eq!(
                        cost.provenance_kind(),
                        CostProvenanceKind::ModeledAssumption
                    );
                    assert!(cost.provenance().contains(PROJECT0_FLASH_PROVIDER_BASIS_ID));
                    assert!(cost
                        .provenance()
                        .contains("scope=provider-protocol-fee-only"));
                    assert!(cost
                        .provenance()
                        .contains("source=Project0-official-documentation"));
                }
                RequiredCost::Unknown { .. } => {
                    return Err(format!(
                        "expected known Project 0 flash provider fee for anchor {anchor_mint}"
                    ));
                }
            }
        }

        Ok(())
    }

    #[test]
    fn remaining_unresolved_external_costs_stay_explicitly_unknown() -> Result<(), String> {
        let model = economics_cost_model(
            WRAPPED_SOL_MINT,
            9,
            &PriorityObservationState::Unavailable("fixture unavailable".to_owned()),
            &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
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

        match model.flash.borrowing_cost {
            RequiredCost::Known(cost) => {
                assert_eq!(cost.amount_anchor_raw(), 0);
                assert!(cost.provenance().contains(PROJECT0_FLASH_PROVIDER_BASIS_ID));
            }
            RequiredCost::Unknown { .. } => {
                return Err("expected known Project 0 flash provider fee".to_owned());
            }
        }

        Ok(())
    }

    #[test]
    fn stablecoin_costs_fail_closed_without_conversion_model() -> Result<(), String> {
        let observation = PriorityObservationState::Available(PriorityFeeObservation {
            samples: vec![PriorityFeeObservationSample {
                slot: 100,
                micro_lamports_per_cu: 10_000,
            }],
            scope_accounts: vec![TEST_VAULT_A.to_owned()],
            scope_provenance: "fixture localized venue contention scope".to_owned(),
        });

        let model = economics_cost_model(
            USDC_MINT,
            6,
            &observation,
            &JitoObservationState::Unavailable("fixture unavailable".to_owned()),
        )?;

        assert!(matches!(
            model.common.base_fee,
            RequiredCost::Unknown { .. }
        ));
        assert!(matches!(
            model.common.priority_fee,
            RequiredCost::Unknown { .. }
        ));

        Ok(())
    }

    #[test]
    fn malformed_priority_response_is_rejected() -> Result<(), String> {
        let footprint = DeterministicVenueContentionFootprint::new_with_provenance(
            [TEST_VAULT_A.to_owned()],
            "fixture contention scope",
        )?;

        let payload = json!({
            "jsonrpc": "2.0",
            "id": PRIORITY_FEE_RPC_REQUEST_ID,
            "result": [{"slot": 100}]
        });

        assert!(parse_localized_priority_fee_response(&payload, &footprint).is_err());
        Ok(())
    }
}
