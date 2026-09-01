use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use serde_json::{json, Value};

pub const USD_SIZE_GRID: [u64; 9] = [1, 5, 10, 25, 50, 100, 250, 500, 1_000];
pub const PYTH_SOL_USD_ACCOUNT: &str = "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE";

const PYTH_RECEIVER_PROGRAM_ID: &str = "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ";
const PRICE_UPDATE_V2_LEN: usize = 134;
const PRICE_UPDATE_V2_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];
const SOL_USD_FEED_ID: [u8; 32] = [
    239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47,
    142, 208, 198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109,
];
const MAX_SOL_USD_AGE_SECONDS: u64 = 90;
const MAX_FUTURE_SKEW_SECONDS: i64 = 5;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SolUsdPrice {
    pub price: u64,
    pub confidence: u64,
    pub exponent: i32,
    pub publish_time: i64,
    pub posted_slot: u64,
    pub rpc_slot: u64,
}

impl SolUsdPrice {
    pub fn summary(&self) -> String {
        format!(
            "price={} confidence={} exponent={} publish_time={} posted_slot={} rpc_slot={}",
            self.price,
            self.confidence,
            self.exponent,
            self.publish_time,
            self.posted_slot,
            self.rpc_slot
        )
    }
}

pub fn sol_usd_price_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 15,
        "method": "getAccountInfo",
        "params": [
            PYTH_SOL_USD_ACCOUNT,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    })
}

pub fn parse_sol_usd_price(payload: &Value, now_unix_seconds: i64) -> Result<SolUsdPrice, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!("Pyth SOL/USD getAccountInfo returned an RPC error: {error}"));
    }

    let rpc_slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Pyth SOL/USD response missing context slot".to_owned())?;

    let account = payload
        .pointer("/result/value")
        .ok_or_else(|| "Pyth SOL/USD response missing account value".to_owned())?;

    if account.is_null() {
        return Err("Pyth SOL/USD account was not found".to_owned());
    }

    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "Pyth SOL/USD account missing owner".to_owned())?;

    if owner != PYTH_RECEIVER_PROGRAM_ID {
        return Err(format!(
            "Pyth SOL/USD owner mismatch: expected {PYTH_RECEIVER_PROGRAM_ID}, got {owner}"
        ));
    }

    let executable = account
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(|| "Pyth SOL/USD account missing executable flag".to_owned())?;

    if executable {
        return Err("Pyth SOL/USD account unexpectedly executable".to_owned());
    }

    let encoded_data = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "Pyth SOL/USD account missing base64 data".to_owned())?;
    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "Pyth SOL/USD account missing data encoding".to_owned())?;

    if encoding != "base64" {
        return Err(format!("unexpected Pyth SOL/USD account encoding: {encoding}"));
    }

    let data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Pyth SOL/USD base64 data: {error}"))?;

    if data.len() != PRICE_UPDATE_V2_LEN {
        return Err(format!(
            "unexpected Pyth PriceUpdateV2 length: expected {PRICE_UPDATE_V2_LEN}, got {}",
            data.len()
        ));
    }

    if data.get(0..8) != Some(PRICE_UPDATE_V2_DISCRIMINATOR.as_slice()) {
        return Err("unexpected Pyth PriceUpdateV2 discriminator".to_owned());
    }

    let verification_level = *data
        .get(40)
        .ok_or_else(|| "Pyth PriceUpdateV2 missing verification level".to_owned())?;

    if verification_level != 1 {
        return Err("Pyth SOL/USD price update is not fully verified".to_owned());
    }

    let feed_id = data
        .get(41..73)
        .ok_or_else(|| "Pyth PriceUpdateV2 missing feed id".to_owned())?;

    if feed_id != SOL_USD_FEED_ID.as_slice() {
        return Err("Pyth price update feed id is not SOL/USD".to_owned());
    }

    let price_signed = read_i64(&data, 73)?;
    let price = u64::try_from(price_signed)
        .map_err(|_| "Pyth SOL/USD price must be positive".to_owned())?;

    if price == 0 {
        return Err("Pyth SOL/USD price must be greater than zero".to_owned());
    }

    let confidence = read_u64(&data, 81)?;
    let exponent = read_i32(&data, 89)?;
    let publish_time = read_i64(&data, 93)?;
    let posted_slot = read_u64(&data, 125)?;

    if posted_slot > rpc_slot {
        return Err(format!(
            "Pyth SOL/USD posted slot exceeds RPC context: posted_slot={posted_slot} rpc_slot={rpc_slot}"
        ));
    }

    let maximum_publish_time = now_unix_seconds
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .ok_or_else(|| "Pyth SOL/USD future-skew calculation overflow".to_owned())?;

    if publish_time > maximum_publish_time {
        return Err(format!(
            "Pyth SOL/USD publish time is too far in the future: publish_time={publish_time} now={now_unix_seconds}"
        ));
    }

    let age_seconds = if publish_time > now_unix_seconds {
        0
    } else {
        now_unix_seconds
            .checked_sub(publish_time)
            .and_then(|age| u64::try_from(age).ok())
            .ok_or_else(|| "Pyth SOL/USD age calculation failed".to_owned())?
    };

    if age_seconds > MAX_SOL_USD_AGE_SECONDS {
        return Err(format!(
            "Pyth SOL/USD price is stale: age_seconds={age_seconds} max_age_seconds={MAX_SOL_USD_AGE_SECONDS}"
        ));
    }

    Ok(SolUsdPrice {
        price,
        confidence,
        exponent,
        publish_time,
        posted_slot,
        rpc_slot,
    })
}

pub fn usd_dollars_to_anchor_raw(
    dollars: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: Option<&SolUsdPrice>,
) -> Result<u64, String> {
    if dollars == 0 {
        return Err("USD size must be greater than zero".to_owned());
    }

    let token_scale = checked_pow10(u32::from(anchor_decimals))?;

    let raw = if anchor_mint == USDC_MINT || anchor_mint == USDT_MINT {
        u128::from(dollars)
            .checked_mul(token_scale)
            .ok_or_else(|| "stablecoin USD sizing overflow".to_owned())?
    } else if anchor_mint == WRAPPED_SOL_MINT {
        let price = sol_usd_price
            .ok_or_else(|| "SOL/USD price context is required for WSOL sizing".to_owned())?;
        let price_raw = u128::from(price.price);

        if price.exponent < 0 {
            let exponent_magnitude = price
                .exponent
                .checked_abs()
                .and_then(|value| u32::try_from(value).ok())
                .ok_or_else(|| "Pyth SOL/USD exponent magnitude overflow".to_owned())?;
            let price_scale = checked_pow10(exponent_magnitude)?;

            u128::from(dollars)
                .checked_mul(token_scale)
                .and_then(|value| value.checked_mul(price_scale))
                .ok_or_else(|| "WSOL USD sizing numerator overflow".to_owned())?
                / price_raw
        } else {
            let exponent = u32::try_from(price.exponent)
                .map_err(|_| "Pyth SOL/USD exponent conversion failed".to_owned())?;
            let price_scale = checked_pow10(exponent)?;
            let denominator = price_raw
                .checked_mul(price_scale)
                .ok_or_else(|| "WSOL USD sizing denominator overflow".to_owned())?;

            u128::from(dollars)
                .checked_mul(token_scale)
                .ok_or_else(|| "WSOL USD sizing numerator overflow".to_owned())?
                / denominator
        }
    } else {
        return Err(format!("unsupported Rung 10 USD anchor mint {anchor_mint}"));
    };

    if raw == 0 {
        return Err(format!(
            "USD size ${dollars} rounded to zero raw units for anchor {anchor_mint}"
        ));
    }

    u64::try_from(raw).map_err(|_| "USD-sized anchor input exceeded u64".to_owned())
}

fn checked_pow10(exponent: u32) -> Result<u128, String> {
    10u128
        .checked_pow(exponent)
        .ok_or_else(|| format!("decimal scale 10^{exponent} exceeded u128"))
}

fn read_i64(data: &[u8], offset: usize) -> Result<i64, String> {
    let bytes = take::<8>(data, offset)?;
    Ok(i64::from_le_bytes(bytes))
}

fn read_u64(data: &[u8], offset: usize) -> Result<u64, String> {
    let bytes = take::<8>(data, offset)?;
    Ok(u64::from_le_bytes(bytes))
}

fn read_i32(data: &[u8], offset: usize) -> Result<i32, String> {
    let bytes = take::<4>(data, offset)?;
    Ok(i32::from_le_bytes(bytes))
}

fn take<const N: usize>(data: &[u8], offset: usize) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| "Pyth account offset overflow".to_owned())?;
    let slice = data
        .get(offset..end)
        .ok_or_else(|| "Pyth account ended unexpectedly".to_owned())?;

    <[u8; N]>::try_from(slice).map_err(|_| "Pyth account field had unexpected size".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;

    const NOW: i64 = 1_700_000_000;

    fn price_update_bytes(price: i64, exponent: i32, publish_time: i64) -> Vec<u8> {
        let mut data = vec![0u8; PRICE_UPDATE_V2_LEN];
        data[0..8].copy_from_slice(&PRICE_UPDATE_V2_DISCRIMINATOR);
        data[40] = 1;
        data[41..73].copy_from_slice(&SOL_USD_FEED_ID);
        data[73..81].copy_from_slice(&price.to_le_bytes());
        data[81..89].copy_from_slice(&25_000u64.to_le_bytes());
        data[89..93].copy_from_slice(&exponent.to_le_bytes());
        data[93..101].copy_from_slice(&publish_time.to_le_bytes());
        data[125..133].copy_from_slice(&123_456u64.to_le_bytes());
        data
    }

    fn price_payload(data: &[u8]) -> Value {
        json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 123_456 },
                "value": {
                    "data": [BASE64_STANDARD.encode(data), "base64"],
                    "executable": false,
                    "lamports": 1,
                    "owner": PYTH_RECEIVER_PROGRAM_ID,
                    "rentEpoch": 0,
                    "space": PRICE_UPDATE_V2_LEN
                }
            },
            "id": 15
        })
    }

    #[test]
    fn price_request_targets_sponsored_sol_usd_account() {
        let request = sol_usd_price_request();

        assert_eq!(request.get("method").and_then(Value::as_str), Some("getAccountInfo"));
        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(PYTH_SOL_USD_ACCOUNT)
        );
    }

    #[test]
    fn parses_fully_verified_fresh_sol_usd_price() -> Result<(), String> {
        let payload = price_payload(&price_update_bytes(20_000_000_000, -8, NOW - 30));
        let price = parse_sol_usd_price(&payload, NOW)?;

        assert_eq!(price.price, 20_000_000_000);
        assert_eq!(price.exponent, -8);
        assert_eq!(price.publish_time, NOW - 30);
        assert_eq!(price.posted_slot, 123_456);
        assert_eq!(price.rpc_slot, 123_456);

        Ok(())
    }

    #[test]
    fn rejects_wrong_owner_feed_partial_stale_and_nonpositive_prices() {
        let bytes = price_update_bytes(20_000_000_000, -8, NOW - 30);

        let mut wrong_owner = price_payload(&bytes);
        wrong_owner["result"]["value"]["owner"] =
            Value::from("11111111111111111111111111111111");
        assert!(parse_sol_usd_price(&wrong_owner, NOW).is_err());

        let mut wrong_feed_bytes = bytes.clone();
        wrong_feed_bytes[41] ^= 1;
        assert!(parse_sol_usd_price(&price_payload(&wrong_feed_bytes), NOW).is_err());

        let mut partial_bytes = bytes.clone();
        partial_bytes[40] = 0;
        partial_bytes[41] = 5;
        assert!(parse_sol_usd_price(&price_payload(&partial_bytes), NOW).is_err());

        let stale = price_update_bytes(20_000_000_000, -8, NOW - 91);
        assert!(parse_sol_usd_price(&price_payload(&stale), NOW).is_err());

        let zero = price_update_bytes(0, -8, NOW - 30);
        assert!(parse_sol_usd_price(&price_payload(&zero), NOW).is_err());

        let negative = price_update_bytes(-1, -8, NOW - 30);
        assert!(parse_sol_usd_price(&price_payload(&negative), NOW).is_err());
    }

    #[test]
    fn stablecoin_grid_maps_exactly_to_raw_units() -> Result<(), String> {
        for dollars in USD_SIZE_GRID {
            let expected = dollars
                .checked_mul(1_000_000)
                .ok_or_else(|| "test stablecoin multiplication overflow".to_owned())?;
            assert_eq!(
                usd_dollars_to_anchor_raw(dollars, USDC_MINT, 6, None)?,
                expected
            );
            assert_eq!(
                usd_dollars_to_anchor_raw(dollars, USDT_MINT, 6, None)?,
                expected
            );
        }

        Ok(())
    }

    #[test]
    fn wsol_grid_uses_integer_price_scaling_and_floors_raw_units() -> Result<(), String> {
        let price = SolUsdPrice {
            price: 20_000_000_000,
            confidence: 1,
            exponent: -8,
            publish_time: NOW,
            posted_slot: 1,
            rpc_slot: 1,
        };

        assert_eq!(
            usd_dollars_to_anchor_raw(1, WRAPPED_SOL_MINT, 9, Some(&price))?,
            5_000_000
        );
        assert_eq!(
            usd_dollars_to_anchor_raw(1_000, WRAPPED_SOL_MINT, 9, Some(&price))?,
            5_000_000_000
        );

        let positive_exponent_price = SolUsdPrice {
            price: 2,
            confidence: 0,
            exponent: 2,
            publish_time: NOW,
            posted_slot: 1,
            rpc_slot: 1,
        };
        assert_eq!(
            usd_dollars_to_anchor_raw(
                1,
                WRAPPED_SOL_MINT,
                9,
                Some(&positive_exponent_price)
            )?,
            5_000_000
        );

        Ok(())
    }
}
