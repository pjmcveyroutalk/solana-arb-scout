use crate::economics::{CostProvenanceKind, RequiredCost};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use crate::sizing::SolUsdPrice;
use serde_json::{json, Value};

pub const TRANSACTION_SHAPE_BASIS_ID: &str = "rung11-v0-single-signer-600k-cu";
pub const MODELED_SIGNATURE_COUNT: u64 = 1;
pub const MODELED_COMPUTE_UNIT_LIMIT: u64 = 600_000;
pub const BASE_FEE_LAMPORTS_PER_SIGNATURE: u64 = 5_000;

const MICRO_LAMPORTS_PER_LAMPORT: u128 = 1_000_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PriorityObservationScope {
    Global,
}

impl PriorityObservationScope {
    pub fn label(self) -> &'static str {
        match self {
            Self::Global => "global-empty-writable-account-set",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PriorityFeeObservation {
    pub slot: u64,
    pub micro_lamports_per_cu: u64,
    pub scope: PriorityObservationScope,
}

impl PriorityFeeObservation {
    pub fn summary(&self) -> String {
        format!(
            "slot={} micro_lamports_per_cu={} scope={}",
            self.slot,
            self.micro_lamports_per_cu,
            self.scope.label()
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct JitoTipFloorObservation {
    pub time: String,
    pub landed_25th_lamports: u64,
    pub landed_50th_lamports: u64,
    pub landed_75th_lamports: u64,
    pub landed_95th_lamports: u64,
    pub landed_99th_lamports: u64,
    pub ema_landed_50th_lamports: u64,
}

impl JitoTipFloorObservation {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "time={} p25_lamports={} p50_lamports={} p75_lamports={} ",
                "p95_lamports={} p99_lamports={} ema_p50_lamports={}"
            ),
            self.time,
            self.landed_25th_lamports,
            self.landed_50th_lamports,
            self.landed_75th_lamports,
            self.landed_95th_lamports,
            self.landed_99th_lamports,
            self.ema_landed_50th_lamports,
        )
    }
}

pub fn priority_fee_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 16,
        "method": "getRecentPrioritizationFees",
        "params": [[]]
    })
}

pub fn parse_priority_fee_observations(
    payload: &Value,
) -> Result<Vec<PriorityFeeObservation>, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "getRecentPrioritizationFees returned an RPC error: {error}"
        ));
    }

    let observations = payload
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| "priority-fee response missing result array".to_owned())?;

    if observations.is_empty() {
        return Err("priority-fee response contained no observations".to_owned());
    }

    observations
        .iter()
        .map(|observation| {
            let slot = observation
                .get("slot")
                .and_then(Value::as_u64)
                .ok_or_else(|| "priority-fee observation missing slot".to_owned())?;

            let micro_lamports_per_cu = observation
                .get("prioritizationFee")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    format!(
                        "priority-fee observation at slot {slot} missing prioritizationFee"
                    )
                })?;

            Ok(PriorityFeeObservation {
                slot,
                micro_lamports_per_cu,
                scope: PriorityObservationScope::Global,
            })
        })
        .collect()
}

pub fn modeled_base_fee_lamports() -> Result<u64, String> {
    BASE_FEE_LAMPORTS_PER_SIGNATURE
        .checked_mul(MODELED_SIGNATURE_COUNT)
        .ok_or_else(|| "modeled base-fee multiplication overflow".to_owned())
}

pub fn modeled_base_fee_cost(
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: &SolUsdPrice,
) -> Result<RequiredCost, String> {
    let base_fee_lamports = modeled_base_fee_lamports()?;

    let amount_anchor_raw = lamports_to_anchor_raw(
        base_fee_lamports,
        anchor_mint,
        anchor_decimals,
        sol_usd_price,
    )?;

    RequiredCost::known(
        amount_anchor_raw,
        CostProvenanceKind::ModeledAssumption,
        format!(
            concat!(
                "basis={} base_fee_rate_lamports_per_signature={} ",
                "modeled_signature_count={} source=Solana-documented-rate"
            ),
            TRANSACTION_SHAPE_BASIS_ID,
            BASE_FEE_LAMPORTS_PER_SIGNATURE,
            MODELED_SIGNATURE_COUNT,
        ),
    )
}

pub fn priority_fee_lamports(
    micro_lamports_per_cu: u64,
    compute_unit_limit: u64,
) -> Result<u64, String> {
    if compute_unit_limit == 0 {
        return Err("priority-fee compute-unit limit must be greater than zero".to_owned());
    }

    let numerator = u128::from(micro_lamports_per_cu)
        .checked_mul(u128::from(compute_unit_limit))
        .ok_or_else(|| "priority-fee multiplication overflow".to_owned())?;

    let lamports = checked_ceil_div(numerator, MICRO_LAMPORTS_PER_LAMPORT)?;

    u64::try_from(lamports).map_err(|_| "priority fee exceeded u64 lamports".to_owned())
}

pub fn modeled_priority_fee_cost(
    selected_observation: &PriorityFeeObservation,
    selection_basis: &str,
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: &SolUsdPrice,
) -> Result<RequiredCost, String> {
    if selection_basis.trim().is_empty() {
        return Err("priority-fee selection basis must not be empty".to_owned());
    }

    let lamports = priority_fee_lamports(
        selected_observation.micro_lamports_per_cu,
        MODELED_COMPUTE_UNIT_LIMIT,
    )?;

    let amount_anchor_raw =
        lamports_to_anchor_raw(lamports, anchor_mint, anchor_decimals, sol_usd_price)?;

    RequiredCost::known(
        amount_anchor_raw,
        CostProvenanceKind::ModeledAssumption,
        format!(
            concat!(
                "basis={} observed_slot={} observed_micro_lamports_per_cu={} ",
                "observation_scope={} modeled_compute_unit_limit={} selection_basis={}"
            ),
            TRANSACTION_SHAPE_BASIS_ID,
            selected_observation.slot,
            selected_observation.micro_lamports_per_cu,
            selected_observation.scope.label(),
            MODELED_COMPUTE_UNIT_LIMIT,
            selection_basis,
        ),
    )
}

pub fn parse_jito_tip_floor(payload: &Value) -> Result<JitoTipFloorObservation, String> {
    let rows = payload
        .as_array()
        .ok_or_else(|| "Jito tip-floor response must be a JSON array".to_owned())?;

    if rows.len() != 1 {
        return Err(format!(
            "Jito tip-floor response expected exactly one row, got {}",
            rows.len()
        ));
    }

    let row = rows
        .first()
        .ok_or_else(|| "Jito tip-floor response was empty".to_owned())?;

    let time = row
        .get("time")
        .and_then(Value::as_str)
        .ok_or_else(|| "Jito tip-floor response missing time".to_owned())?
        .trim()
        .to_owned();

    if time.is_empty() {
        return Err("Jito tip-floor response time must not be empty".to_owned());
    }

    Ok(JitoTipFloorObservation {
        time,
        landed_25th_lamports: jito_sol_field_to_lamports(
            row,
            "landed_tips_25th_percentile",
        )?,
        landed_50th_lamports: jito_sol_field_to_lamports(
            row,
            "landed_tips_50th_percentile",
        )?,
        landed_75th_lamports: jito_sol_field_to_lamports(
            row,
            "landed_tips_75th_percentile",
        )?,
        landed_95th_lamports: jito_sol_field_to_lamports(
            row,
            "landed_tips_95th_percentile",
        )?,
        landed_99th_lamports: jito_sol_field_to_lamports(
            row,
            "landed_tips_99th_percentile",
        )?,
        ema_landed_50th_lamports: jito_sol_field_to_lamports(
            row,
            "ema_landed_tips_50th_percentile",
        )?,
    })
}

pub fn jito_tip_lamports_to_cost(
    tip_lamports: u64,
    observation_time: &str,
    selection_basis: &str,
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: &SolUsdPrice,
) -> Result<RequiredCost, String> {
    if observation_time.trim().is_empty() {
        return Err("Jito observation time must not be empty".to_owned());
    }

    if selection_basis.trim().is_empty() {
        return Err("Jito tip selection basis must not be empty".to_owned());
    }

    let amount_anchor_raw =
        lamports_to_anchor_raw(tip_lamports, anchor_mint, anchor_decimals, sol_usd_price)?;

    RequiredCost::known(
        amount_anchor_raw,
        CostProvenanceKind::ModeledAssumption,
        format!(
            concat!(
                "source_observation=Jito-public-tip-floor ",
                "observation_time={} selected_tip_lamports={} ",
                "selection_basis={} interpretation=hypothetical-submission-cost"
            ),
            observation_time, tip_lamports, selection_basis
        ),
    )
}

pub fn lamports_to_anchor_raw(
    lamports: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    _sol_usd_price: &SolUsdPrice,
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
                "Rung 11 stablecoin external-cost conversion is not yet authorized for anchor {}; ",
                "SOL/USD alone does not prove stablecoin/USD parity and no explicit parity ",
                "observation/model basis has been supplied"
            ),
            anchor_mint
        ));
    }

    Err(format!(
        "unsupported Rung 11 external-cost anchor mint {anchor_mint}"
    ))
}

fn jito_sol_field_to_lamports(row: &Value, field: &str) -> Result<u64, String> {
    let value = row
        .get(field)
        .ok_or_else(|| format!("Jito tip-floor response missing {field}"))?;

    let raw = match value {
        Value::Number(number) => number.to_string(),
        Value::String(text) => text.clone(),
        _ => return Err(format!("Jito tip-floor field {field} must be numeric")),
    };

    decimal_sol_to_lamports(&raw)
        .map_err(|error| format!("Jito tip-floor field {field} invalid: {error}"))
}

fn decimal_sol_to_lamports(raw: &str) -> Result<u64, String> {
    let trimmed = raw.trim();

    if trimmed.is_empty() {
        return Err("SOL amount must not be empty".to_owned());
    }

    if trimmed.starts_with('-') {
        return Err("SOL amount must not be negative".to_owned());
    }

    let unsigned = trimmed.strip_prefix('+').unwrap_or(trimmed);
    let (mantissa, exponent) = split_decimal_exponent(unsigned)?;

    let mut pieces = mantissa.split('.');
    let whole = pieces
        .next()
        .ok_or_else(|| "SOL amount missing mantissa".to_owned())?;
    let fractional = pieces.next().unwrap_or("");

    if pieces.next().is_some() {
        return Err("SOL amount contains multiple decimal points".to_owned());
    }

    if whole.is_empty() && fractional.is_empty() {
        return Err("SOL amount missing digits".to_owned());
    }

    if !whole.chars().all(|character| character.is_ascii_digit())
        || !fractional
            .chars()
            .all(|character| character.is_ascii_digit())
    {
        return Err("SOL amount contains non-decimal digits".to_owned());
    }

    let digits = format!("{whole}{fractional}");

    let mantissa_value = if digits.is_empty() {
        0u128
    } else {
        digits
            .parse::<u128>()
            .map_err(|error| format!("SOL amount mantissa overflow: {error}"))?
    };

    let fractional_digits = i32::try_from(fractional.len())
        .map_err(|_| "SOL amount fractional precision exceeded i32".to_owned())?;

    let lamport_exponent = 9i32
        .checked_add(exponent)
        .and_then(|value| value.checked_sub(fractional_digits))
        .ok_or_else(|| "SOL-to-lamport exponent overflow".to_owned())?;

    let lamports = if lamport_exponent >= 0 {
        let scale_exponent = u32::try_from(lamport_exponent)
            .map_err(|_| "SOL-to-lamport positive exponent conversion failed".to_owned())?;

        mantissa_value
            .checked_mul(checked_pow10(scale_exponent)?)
            .ok_or_else(|| "SOL-to-lamport multiplication overflow".to_owned())?
    } else {
        let divisor_exponent = lamport_exponent
            .checked_abs()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| "SOL-to-lamport divisor exponent overflow".to_owned())?;

        let divisor = checked_pow10(divisor_exponent)?;
        let quotient = mantissa_value / divisor;
        let remainder = mantissa_value % divisor;

        let doubled_remainder = remainder
            .checked_mul(2)
            .ok_or_else(|| "SOL-to-lamport rounding overflow".to_owned())?;

        if doubled_remainder >= divisor {
            quotient
                .checked_add(1)
                .ok_or_else(|| "SOL-to-lamport rounded result overflow".to_owned())?
        } else {
            quotient
        }
    };

    u64::try_from(lamports).map_err(|_| "SOL-to-lamport result exceeded u64".to_owned())
}

fn split_decimal_exponent(raw: &str) -> Result<(&str, i32), String> {
    let exponent_index = raw
        .char_indices()
        .find_map(|(index, character)| matches!(character, 'e' | 'E').then_some(index));

    let Some(index) = exponent_index else {
        return Ok((raw, 0));
    };

    let mantissa = raw
        .get(..index)
        .ok_or_else(|| "SOL amount mantissa slicing failed".to_owned())?;

    let exponent_text = raw
        .get(index + 1..)
        .ok_or_else(|| "SOL amount exponent slicing failed".to_owned())?;

    if exponent_text.is_empty() {
        return Err("SOL amount exponent is empty".to_owned());
    }

    if exponent_text.contains('e') || exponent_text.contains('E') {
        return Err("SOL amount contains multiple exponents".to_owned());
    }

    let exponent = exponent_text
        .parse::<i32>()
        .map_err(|error| format!("SOL amount exponent invalid: {error}"))?;

    Ok((mantissa, exponent))
}

fn checked_pow10(exponent: u32) -> Result<u128, String> {
    10u128
        .checked_pow(exponent)
        .ok_or_else(|| format!("decimal scale 10^{exponent} exceeded u128"))
}

fn checked_ceil_div(numerator: u128, denominator: u128) -> Result<u128, String> {
    if denominator == 0 {
        return Err("checked ceil division denominator must not be zero".to_owned());
    }

    let quotient = numerator / denominator;
    let remainder = numerator % denominator;

    if remainder == 0 {
        Ok(quotient)
    } else {
        quotient
            .checked_add(1)
            .ok_or_else(|| "checked ceil division overflow".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const SOL_USD_PRICE: SolUsdPrice = SolUsdPrice {
        price: 20_000_000_000,
        confidence: 25_000,
        exponent: -8,
        publish_time: 1_700_000_000,
        posted_slot: 123_456,
        rpc_slot: 123_456,
    };

    #[test]
    fn locked_transaction_shape_is_single_signer_and_600k_cu() -> Result<(), String> {
        assert_eq!(
            TRANSACTION_SHAPE_BASIS_ID,
            "rung11-v0-single-signer-600k-cu"
        );
        assert_eq!(MODELED_SIGNATURE_COUNT, 1);
        assert_eq!(MODELED_COMPUTE_UNIT_LIMIT, 600_000);
        assert_eq!(modeled_base_fee_lamports()?, 5_000);

        Ok(())
    }

    #[test]
    fn priority_request_is_global_and_read_only() {
        let request = priority_fee_request();

        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("getRecentPrioritizationFees")
        );

        assert_eq!(
            request
                .pointer("/params/0")
                .and_then(Value::as_array)
                .map(Vec::len),
            Some(0)
        );
    }

    #[test]
    fn priority_response_preserves_raw_observation_units_and_scope() -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "result": [
                {
                    "slot": 100,
                    "prioritizationFee": 50_000
                },
                {
                    "slot": 101,
                    "prioritizationFee": 75_000
                }
            ],
            "id": 16
        });

        let observations = parse_priority_fee_observations(&payload)?;

        assert_eq!(observations.len(), 2);
        assert_eq!(observations[0].slot, 100);
        assert_eq!(observations[0].micro_lamports_per_cu, 50_000);
        assert_eq!(observations[0].scope, PriorityObservationScope::Global);

        Ok(())
    }

    #[test]
    fn priority_fee_uses_ceiling_integer_math() -> Result<(), String> {
        assert_eq!(priority_fee_lamports(50_000, 600_000)?, 30_000);
        assert_eq!(priority_fee_lamports(1, 1)?, 1);

        Ok(())
    }

    #[test]
    fn jito_parser_converts_sol_values_to_lamports_without_unit_confusion() -> Result<(), String> {
        let payload = json!([
            {
                "time": "2026-09-01T12:00:00Z",
                "landed_tips_25th_percentile": 0.000001,
                "landed_tips_50th_percentile": 0.000002,
                "landed_tips_75th_percentile": 0.000003,
                "landed_tips_95th_percentile": 0.000004,
                "landed_tips_99th_percentile": 0.000005,
                "ema_landed_tips_50th_percentile": 0.0000025
            }
        ]);

        let observation = parse_jito_tip_floor(&payload)?;

        assert_eq!(observation.landed_25th_lamports, 1_000);
        assert_eq!(observation.landed_50th_lamports, 2_000);
        assert_eq!(observation.landed_75th_lamports, 3_000);
        assert_eq!(observation.landed_95th_lamports, 4_000);
        assert_eq!(observation.landed_99th_lamports, 5_000);
        assert_eq!(observation.ema_landed_50th_lamports, 2_500);

        Ok(())
    }

    #[test]
    fn jito_parser_accepts_scientific_notation() -> Result<(), String> {
        assert_eq!(decimal_sol_to_lamports("1e-6")?, 1_000);
        assert_eq!(decimal_sol_to_lamports("2.5e-6")?, 2_500);
        assert_eq!(decimal_sol_to_lamports("0.000001")?, 1_000);

        Ok(())
    }

    #[test]
    fn wsol_external_cost_preserves_lamport_raw_units() -> Result<(), String> {
        let raw =
            lamports_to_anchor_raw(12_345, WRAPPED_SOL_MINT, 9, &SOL_USD_PRICE)?;

        assert_eq!(raw, 12_345);

        Ok(())
    }

    #[test]
    fn stablecoin_external_cost_fails_closed_without_parity_basis() {
        assert!(
            lamports_to_anchor_raw(5_000, USDC_MINT, 6, &SOL_USD_PRICE).is_err()
        );

        assert!(
            lamports_to_anchor_raw(5_000, USDT_MINT, 6, &SOL_USD_PRICE).is_err()
        );
    }

    #[test]
    fn unsupported_anchor_fails_closed() {
        assert!(
            lamports_to_anchor_raw(
                5_000,
                "unsupported-mint",
                6,
                &SOL_USD_PRICE
            )
            .is_err()
        );
    }

    #[test]
    fn priority_parser_rejects_rpc_errors_and_empty_results() {
        let rpc_error = json!({
            "jsonrpc": "2.0",
            "error": {
                "code": -32000,
                "message": "fixture error"
            },
            "id": 16
        });

        assert!(parse_priority_fee_observations(&rpc_error).is_err());

        let empty = json!({
            "jsonrpc": "2.0",
            "result": [],
            "id": 16
        });

        assert!(parse_priority_fee_observations(&empty).is_err());
    }

    #[test]
    fn modeled_costs_
