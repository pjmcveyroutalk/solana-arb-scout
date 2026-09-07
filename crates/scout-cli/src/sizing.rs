use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};

pub const USD_SIZE_GRID: [u64; 9] = [1, 5, 10, 25, 50, 100, 250, 500, 1_000];

pub const PYTH_SOL_USD_ACCOUNT: &str = "7UVimffxr9ow1uXYxsr4LHAcV58mLzhmwaeKvJ1pjLiE";
pub const PYTH_USDC_USD_ACCOUNT: &str = "Dpw1EAVrSB1ibxiDQyTAW6Zip3J4Btk2x4SgApQCeFbX";
pub const PYTH_USDT_USD_ACCOUNT: &str = "HT2PLQBcG5EiCcNSaMHAjSgd9F98ecpATbk4Sk5oYuM";

const PYTH_RECEIVER_PROGRAM_ID: &str = "rec5EKMGg6MxZYaMdyBfgwp4d5rB9T1VQH5pJv5LtFJ";
const PRICE_UPDATE_V2_LEN: usize = 134;
const PRICE_UPDATE_V2_DISCRIMINATOR: [u8; 8] = [34, 241, 35, 99, 157, 126, 244, 205];

const SOL_USD_FEED_ID: [u8; 32] = [
    239, 13, 139, 111, 218, 44, 235, 164, 29, 161, 93, 64, 149, 209, 218, 57, 42, 13, 47, 142, 208,
    198, 199, 188, 15, 76, 250, 200, 194, 128, 181, 109,
];

const USDC_USD_FEED_ID: [u8; 32] = [
    234, 160, 32, 198, 28, 196, 121, 113, 40, 19, 70, 28, 225, 83, 137, 74, 150, 166, 192, 11, 33,
    237, 12, 252, 39, 152, 209, 249, 169, 233, 201, 74,
];

const USDT_USD_FEED_ID: [u8; 32] = [
    43, 137, 185, 220, 143, 223, 159, 52, 112, 154, 91, 16, 107, 71, 47, 15, 57, 187, 108, 169,
    206, 4, 176, 253, 127, 46, 151, 22, 136, 226, 229, 59,
];

const MAX_PYTH_USD_AGE_SECONDS: u64 = 90;
const MAX_FUTURE_SKEW_SECONDS: i64 = 5;
const MAX_PYTH_CONFIDENCE_BPS: u64 = 100;
const BPS_DENOMINATOR: u64 = 10_000;
const MAX_PYTH_ABS_EXPONENT: u32 = 38;
const PYTH_USD_ACCEPTANCE_POLICY_ID: &str = "pyth-usd-full-verified-fresh-confidence-1pct-v1";
const PYTH_VERIFICATION_REQUIREMENT: &str = "fully-verified";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum PythUsdFeed {
    Sol,
    Usdc,
    Usdt,
}

#[cfg(test)]
const PYTH_USD_FEEDS: [PythUsdFeed; 3] = [PythUsdFeed::Sol, PythUsdFeed::Usdc, PythUsdFeed::Usdt];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PythUsdProvenance {
    account: &'static str,
    feed_id: &'static [u8; 32],
    receiver_program_id: &'static str,
    verification_requirement: &'static str,
    acceptance_policy_id: &'static str,
}

impl PythUsdFeed {
    pub fn label(self) -> &'static str {
        match self {
            Self::Sol => "SOL/USD",
            Self::Usdc => "USDC/USD",
            Self::Usdt => "USDT/USD",
        }
    }

    pub fn account(self) -> &'static str {
        self.provenance().account
    }

    pub fn request_id(self) -> u64 {
        match self {
            Self::Sol => 15,
            Self::Usdc => 16,
            Self::Usdt => 17,
        }
    }

    #[cfg(test)]
    fn feed_id(self) -> &'static [u8; 32] {
        self.provenance().feed_id
    }

    fn provenance(self) -> PythUsdProvenance {
        let (account, feed_id) = match self {
            Self::Sol => (PYTH_SOL_USD_ACCOUNT, &SOL_USD_FEED_ID),
            Self::Usdc => (PYTH_USDC_USD_ACCOUNT, &USDC_USD_FEED_ID),
            Self::Usdt => (PYTH_USDT_USD_ACCOUNT, &USDT_USD_FEED_ID),
        };

        PythUsdProvenance {
            account,
            feed_id,
            receiver_program_id: PYTH_RECEIVER_PROGRAM_ID,
            verification_requirement: PYTH_VERIFICATION_REQUIREMENT,
            acceptance_policy_id: PYTH_USD_ACCEPTANCE_POLICY_ID,
        }
    }
}

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

pub fn pyth_usd_price_request(feed: PythUsdFeed) -> Value {
    let provenance = feed.provenance();

    json!({
        "jsonrpc": "2.0",
        "id": feed.request_id(),
        "method": "getAccountInfo",
        "params": [
            provenance.account,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    })
}

#[cfg(test)]
pub fn sol_usd_price_request() -> Value {
    pyth_usd_price_request(PYTH_USD_FEEDS[0])
}

pub fn parse_pyth_usd_price(
    payload: &Value,
    now_unix_seconds: i64,
    feed: PythUsdFeed,
) -> Result<SolUsdPrice, String> {
    let provenance = feed.provenance();

    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Pyth {} getAccountInfo returned an RPC error: {error}",
            feed.label()
        ));
    }

    let jsonrpc = payload
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Pyth {} response missing jsonrpc version", feed.label()))?;

    if jsonrpc != "2.0" {
        return Err(format!(
            "Pyth {} response has unexpected jsonrpc version: {jsonrpc}",
            feed.label()
        ));
    }

    let response_id = payload
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Pyth {} response missing numeric id", feed.label()))?;

    if response_id != feed.request_id() {
        return Err(format!(
            "Pyth {} response id mismatch: expected={} actual={response_id}",
            feed.label(),
            feed.request_id()
        ));
    }

    let rpc_slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Pyth {} response missing context slot", feed.label()))?;

    let account = payload
        .pointer("/result/value")
        .ok_or_else(|| format!("Pyth {} response missing account value", feed.label()))?;

    if account.is_null() {
        return Err(format!("Pyth {} account was not found", feed.label()));
    }

    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Pyth {} account missing owner", feed.label()))?;

    if owner != provenance.receiver_program_id {
        return Err(format!(
            "Pyth {} owner mismatch: expected {}, got {owner}",
            feed.label(),
            provenance.receiver_program_id
        ));
    }

    let executable = account
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("Pyth {} account missing executable flag", feed.label()))?;

    if executable {
        return Err(format!(
            "Pyth {} account unexpectedly executable",
            feed.label()
        ));
    }

    let encoded_data = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Pyth {} account missing base64 data", feed.label()))?;

    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Pyth {} account missing data encoding", feed.label()))?;

    if encoding != "base64" {
        return Err(format!(
            "unexpected Pyth {} account encoding: {encoding}",
            feed.label()
        ));
    }

    let data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Pyth {} base64 data: {error}", feed.label()))?;

    if data.len() != PRICE_UPDATE_V2_LEN {
        return Err(format!(
            "unexpected Pyth {} PriceUpdateV2 length: expected {PRICE_UPDATE_V2_LEN}, got {}",
            feed.label(),
            data.len()
        ));
    }

    if data.get(0..8) != Some(PRICE_UPDATE_V2_DISCRIMINATOR.as_slice()) {
        return Err(format!(
            "unexpected Pyth {} PriceUpdateV2 discriminator",
            feed.label()
        ));
    }

    let verification_level = *data.get(40).ok_or_else(|| {
        format!(
            "Pyth {} PriceUpdateV2 missing verification level",
            feed.label()
        )
    })?;

    if verification_level != 1 {
        return Err(format!(
            "Pyth {} price update does not satisfy verification requirement {}",
            feed.label(),
            provenance.verification_requirement
        ));
    }

    let feed_id = data
        .get(41..73)
        .ok_or_else(|| format!("Pyth {} PriceUpdateV2 missing feed id", feed.label()))?;

    if feed_id != provenance.feed_id.as_slice() {
        return Err(format!("Pyth price update feed id is not {}", feed.label()));
    }

    let price_signed = read_i64(&data, 73)?;
    let price = u64::try_from(price_signed)
        .map_err(|_| format!("Pyth {} price must be positive", feed.label()))?;

    if price == 0 {
        return Err(format!(
            "Pyth {} price must be greater than zero",
            feed.label()
        ));
    }

    let confidence = read_u64(&data, 81)?;
    let exponent = read_i32(&data, 89)?;
    let publish_time = read_i64(&data, 93)?;
    let posted_slot = read_u64(&data, 125)?;

    validate_pyth_usd_confidence(feed, price, confidence, provenance.acceptance_policy_id)?;
    validate_pyth_usd_exponent(feed, exponent, provenance.acceptance_policy_id)?;

    if posted_slot > rpc_slot {
        return Err(format!(
            "Pyth {} posted slot exceeds RPC context: posted_slot={posted_slot} rpc_slot={rpc_slot}",
            feed.label()
        ));
    }

    let maximum_publish_time = now_unix_seconds
        .checked_add(MAX_FUTURE_SKEW_SECONDS)
        .ok_or_else(|| format!("Pyth {} future-skew calculation overflow", feed.label()))?;

    if publish_time > maximum_publish_time {
        return Err(format!(
            "Pyth {} publish time is too far in the future: publish_time={publish_time} now={now_unix_seconds}",
            feed.label()
        ));
    }

    let age_seconds = if publish_time > now_unix_seconds {
        0
    } else {
        now_unix_seconds
            .checked_sub(publish_time)
            .and_then(|age| u64::try_from(age).ok())
            .ok_or_else(|| format!("Pyth {} age calculation failed", feed.label()))?
    };

    if age_seconds > MAX_PYTH_USD_AGE_SECONDS {
        return Err(format!(
            "Pyth {} price is stale: age_seconds={age_seconds} max_age_seconds={MAX_PYTH_USD_AGE_SECONDS}",
            feed.label()
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

fn validate_pyth_usd_confidence(
    feed: PythUsdFeed,
    price: u64,
    confidence: u64,
    policy_id: &str,
) -> Result<(), String> {
    if confidence >= price {
        return Err(format!(
            "Pyth {} confidence interval has no positive lower price bound: price={} confidence={} policy={}",
            feed.label(),
            price,
            confidence,
            policy_id
        ));
    }

    let confidence_bps_numerator = u128::from(confidence)
        .checked_mul(u128::from(BPS_DENOMINATOR))
        .ok_or_else(|| format!("Pyth {} confidence-ratio overflow", feed.label()))?;

    let confidence_bps_limit = u128::from(price)
        .checked_mul(u128::from(MAX_PYTH_CONFIDENCE_BPS))
        .ok_or_else(|| format!("Pyth {} confidence-limit overflow", feed.label()))?;

    if confidence_bps_numerator > confidence_bps_limit {
        return Err(format!(
            "Pyth {} confidence interval exceeds Scout acceptance policy: price={} confidence={} max_confidence_bps={} policy={}",
            feed.label(),
            price,
            confidence,
            MAX_PYTH_CONFIDENCE_BPS,
            policy_id
        ));
    }

    Ok(())
}

fn validate_pyth_usd_exponent(
    feed: PythUsdFeed,
    exponent: i32,
    policy_id: &str,
) -> Result<(), String> {
    let magnitude = exponent
        .checked_abs()
        .and_then(|value| u32::try_from(value).ok())
        .ok_or_else(|| {
            format!(
                "Pyth {} exponent magnitude conversion failed: exponent={exponent}",
                feed.label()
            )
        })?;

    if magnitude > MAX_PYTH_ABS_EXPONENT {
        return Err(format!(
            "Pyth {} exponent exceeds Scout arithmetic policy: exponent={} max_abs_exponent={} policy={}",
            feed.label(),
            exponent,
            MAX_PYTH_ABS_EXPONENT,
            policy_id
        ));
    }

    Ok(())
}

#[cfg(test)]
pub fn parse_sol_usd_price(payload: &Value, now_unix_seconds: i64) -> Result<SolUsdPrice, String> {
    parse_pyth_usd_price(payload, now_unix_seconds, PythUsdFeed::Sol)
}

pub fn usd_dollars_to_anchor_raw(
    dollars: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: Option<&SolUsdPrice>,
) -> Result<u64, String> {
    if anchor_mint == USDC_MINT || anchor_mint == USDT_MINT {
        return Err(format!(
            "stablecoin USD sizing requires its accepted Pyth USD feed for anchor {anchor_mint}"
        ));
    }

    if anchor_mint != WRAPPED_SOL_MINT {
        return Err(format!("unsupported Rung 10 USD anchor mint {anchor_mint}"));
    }

    let sol_usd_price = sol_usd_price
        .ok_or_else(|| "Pyth SOL/USD price context is required for WSOL sizing".to_owned())?;

    usd_dollars_to_anchor_raw_with_prices(
        dollars,
        anchor_mint,
        anchor_decimals,
        sol_usd_price,
        None,
        None,
    )
}

pub fn usd_dollars_to_anchor_raw_with_prices(
    dollars: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    sol_usd_price: &SolUsdPrice,
    usdc_usd_price: Option<&SolUsdPrice>,
    usdt_usd_price: Option<&SolUsdPrice>,
) -> Result<u64, String> {
    let price = if anchor_mint == WRAPPED_SOL_MINT {
        sol_usd_price
    } else if anchor_mint == USDC_MINT {
        usdc_usd_price
            .ok_or_else(|| "Pyth USDC/USD price context is required for USDC sizing".to_owned())?
    } else if anchor_mint == USDT_MINT {
        usdt_usd_price
            .ok_or_else(|| "Pyth USDT/USD price context is required for USDT sizing".to_owned())?
    } else {
        return Err(format!("unsupported Rung 10 USD anchor mint {anchor_mint}"));
    };

    usd_dollars_to_raw_at_upper_usd_bound(dollars, anchor_mint, anchor_decimals, price)
}

fn usd_dollars_to_raw_at_upper_usd_bound(
    dollars: u64,
    anchor_mint: &str,
    anchor_decimals: u8,
    price: &SolUsdPrice,
) -> Result<u64, String> {
    if dollars == 0 {
        return Err("USD size must be greater than zero".to_owned());
    }

    let token_scale = checked_pow10(u32::from(anchor_decimals))?;
    let upper_price = price.price.checked_add(price.confidence).ok_or_else(|| {
        format!("Pyth USD upper confidence bound overflow for anchor {anchor_mint}")
    })?;

    if upper_price == 0 {
        return Err(format!(
            "Pyth USD upper confidence bound must be positive for anchor {anchor_mint}"
        ));
    }

    let upper_price_raw = u128::from(upper_price);

    let raw = if price.exponent < 0 {
        let exponent_magnitude = price
            .exponent
            .checked_abs()
            .and_then(|value| u32::try_from(value).ok())
            .ok_or_else(|| {
                format!("Pyth USD exponent magnitude overflow for anchor {anchor_mint}")
            })?;
        let price_scale = checked_pow10(exponent_magnitude)?;

        u128::from(dollars)
            .checked_mul(token_scale)
            .and_then(|value| value.checked_mul(price_scale))
            .ok_or_else(|| format!("USD sizing numerator overflow for anchor {anchor_mint}"))?
            / upper_price_raw
    } else {
        let exponent = u32::try_from(price.exponent)
            .map_err(|_| format!("Pyth USD exponent conversion failed for anchor {anchor_mint}"))?;
        let price_scale = checked_pow10(exponent)?;
        let denominator = upper_price_raw
            .checked_mul(price_scale)
            .ok_or_else(|| format!("USD sizing denominator overflow for anchor {anchor_mint}"))?;

        u128::from(dollars)
            .checked_mul(token_scale)
            .ok_or_else(|| format!("USD sizing numerator overflow for anchor {anchor_mint}"))?
            / denominator
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

    fn price_update_bytes(
        feed: PythUsdFeed,
        price: i64,
        confidence: u64,
        exponent: i32,
        publish_time: i64,
    ) -> Vec<u8> {
        let mut data = vec![0u8; PRICE_UPDATE_V2_LEN];
        data[0..8].copy_from_slice(&PRICE_UPDATE_V2_DISCRIMINATOR);
        data[40] = 1;
        data[41..73].copy_from_slice(feed.feed_id());
        data[73..81].copy_from_slice(&price.to_le_bytes());
        data[81..89].copy_from_slice(&confidence.to_le_bytes());
        data[89..93].copy_from_slice(&exponent.to_le_bytes());
        data[93..101].copy_from_slice(&publish_time.to_le_bytes());
        data[125..133].copy_from_slice(&123_456u64.to_le_bytes());
        data
    }

    fn price_payload(feed: PythUsdFeed, data: &[u8]) -> Value {
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
            "id": feed.request_id()
        })
    }

    fn test_price(price: u64, confidence: u64, exponent: i32) -> SolUsdPrice {
        SolUsdPrice {
            price,
            confidence,
            exponent,
            publish_time: NOW,
            posted_slot: 1,
            rpc_slot: 1,
        }
    }

    #[test]
    fn price_requests_target_all_sponsored_usd_accounts() {
        for feed in PYTH_USD_FEEDS {
            let request = pyth_usd_price_request(feed);

            assert_eq!(
                request.get("method").and_then(Value::as_str),
                Some("getAccountInfo")
            );
            assert_eq!(
                request.pointer("/params/0").and_then(Valu
