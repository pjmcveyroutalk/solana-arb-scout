use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::str::FromStr;

pub const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

const PUMP_PROGRAM_ID: &str = "6EF8rrecthR5Dkzon8Nwu78hRvfCKubJ14M5uBEwF6P";
const DEFAULT_PUBKEY: &str = "11111111111111111111111111111111";
const FEE_BPS_DENOMINATOR: u64 = 10_000;

const PUMPSWAP_GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";

const PUMP_FEES_PROGRAM_ID: &str = "pfeeUxB6jkeY1Hxd7CsFCAjcbHA9rWtchMGdZ6VojVZ";
const PUMPSWAP_FEE_CONFIG: &str = "5PHirr8joyTMp9JMm6nW7hNDVyEYdkzDqazxPD7RaTjx";
const PUMPSWAP_FEE_CONFIG_BUMP: u8 = 255;

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];
const FEE_CONFIG_DISCRIMINATOR: [u8; 8] = [143, 52, 146, 187, 219, 123, 76, 155];

const POOL_BASE_LEN: usize = 211;
const POOL_BASE_MINT_OFFSET: usize = 43;
const POOL_QUOTE_MINT_OFFSET: usize = 75;
const POOL_COIN_CREATOR_END: usize = 243;
const POOL_MAYHEM_END: usize = 244;
const POOL_CASHBACK_END: usize = 245;
const POOL_VIRTUAL_QUOTE_END: usize = 261;

const GLOBAL_CONFIG_DISABLE_FLAGS_OFFSET: usize = 56;
const GLOBAL_CONFIG_MIN_LEN: usize = GLOBAL_CONFIG_DISABLE_FLAGS_OFFSET + 1;

const DISABLE_BUY_MASK: u8 = 1 << 3;
const DISABLE_SELL_MASK: u8 = 1 << 4;

const TOKEN_ACCOUNT_BASE_LEN: usize = 165;
const TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;

const MINT_ACCOUNT_BASE_LEN: usize = 82;
const MINT_SUPPLY_OFFSET: usize = 36;
const MINT_DECIMALS_OFFSET: usize = 44;
const MINT_INITIALIZED_OFFSET: usize = 45;

const FEE_CONFIG_FIXED_PREFIX_LEN: usize = 8 + 1 + 32 + 24;
const FEE_TIER_SERIALIZED_LEN: usize = 16 + 24;
const MAX_FEE_BPS: u64 = 10_000;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapPoolState {
    pub pool_bump: u8,
    pub index: u16,
    pub creator: String,
    pub base_mint: String,
    pub quote_mint: String,
    pub lp_mint: String,
    pub pool_base_token_account: String,
    pub pool_quote_token_account: String,
    pub lp_supply: u64,
    pub coin_creator: Option<String>,
    pub is_mayhem_mode: Option<bool>,
    pub is_cashback_coin: Option<bool>,
    pub virtual_quote_reserves: i128,
}

impl PumpSwapPoolState {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "pool_bump={} index={} creator={} ",
                "base_mint={} quote_mint={} ",
                "base_vault={} quote_vault={} ",
                "lp_mint={} lp_supply={} ",
                "coin_creator={} mayhem={} cashback={} ",
                "virtual_quote_reserves={}"
            ),
            self.pool_bump,
            self.index,
            self.creator,
            self.base_mint,
            self.quote_mint,
            self.pool_base_token_account,
            self.pool_quote_token_account,
            self.lp_mint,
            self.lp_supply,
            self.coin_creator.as_deref().unwrap_or("unavailable"),
            optional_bool_label(self.is_mayhem_mode),
            optional_bool_label(self.is_cashback_coin),
            self.virtual_quote_reserves,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapFees {
    pub lp_fee_bps: u64,
    pub protocol_fee_bps: u64,
    pub creator_fee_bps: u64,
}

impl PumpSwapFees {
    fn summary(&self) -> String {
        format!(
            "lp_fee_bps={} protocol_fee_bps={} creator_fee_bps={}",
            self.lp_fee_bps, self.protocol_fee_bps, self.creator_fee_bps
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapFeeTier {
    pub market_cap_lamports_threshold: u128,
    pub fees: PumpSwapFees,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapFeeConfig {
    pub bump: u8,
    pub admin: String,
    pub flat_fees: PumpSwapFees,
    pub fee_tiers: Vec<PumpSwapFeeTier>,
    pub stable_fee_tiers: Vec<PumpSwapFeeTier>,
}

impl PumpSwapFeeConfig {
    fn summary(&self) -> String {
        format!(
            concat!(
                "bump={} admin={} flat_fees=({}) ",
                "fee_tier_count={} stable_fee_tier_count={}"
            ),
            self.bump,
            self.admin,
            self.flat_fees.summary(),
            self.fee_tiers.len(),
            self.stable_fee_tiers.len(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapAccountObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub encoded_data_len: usize,
    pub decoded_data_len: usize,
    pub pool_state: PumpSwapPoolState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapHydrationSnapshot {
    pub slot: u64,
    pub pool_state: PumpSwapPoolState,
    pub base_vault_raw: u64,
    pub quote_vault_raw: u64,
    pub effective_quote_raw: u64,
    pub base_mint_supply_raw: u64,
    pub base_token_program: String,
    pub quote_token_program: String,
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub disable_flags: u8,
    pub trading_state: PoolTradingState,
    pub fee_config: PumpSwapFeeConfig,
}

impl PumpSwapHydrationSnapshot {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "reserve_slot={} ",
                "base_vault_raw={} quote_vault_raw={} ",
                "effective_quote_raw={} ",
                "base_mint_supply_raw={} ",
                "base_token_program={} quote_token_program={} ",
                "base_decimals={} quote_decimals={} ",
                "disable_flags={} trading_state={:?} ",
                "fee_config=[{}]"
            ),
            self.slot,
            self.base_vault_raw,
            self.quote_vault_raw,
            self.effective_quote_raw,
            self.base_mint_supply_raw,
            self.base_token_program,
            self.quote_token_program,
            self.base_decimals,
            self.quote_decimals,
            self.disable_flags,
            self.trading_state,
            self.fee_config.summary(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PumpSwapExactInputQuote {
    pub amount_in_requested_raw: u64,
    pub amount_in_consumed_raw: u64,
    pub amount_in_unspent_raw: u64,
    pub amount_out_raw: u64,
    pub lp_fee_raw: u64,
    pub protocol_fee_raw: u64,
    pub creator_fee_raw: u64,
    pub source_slot: u64,
}

pub fn quote_exact_input(
    snapshot: &PumpSwapHydrationSnapshot,
    input_mint: &str,
    amount_in_raw: u64,
) -> Result<PumpSwapExactInputQuote, String> {
    if amount_in_raw == 0 {
        return Err("PumpSwap quote input must be greater than zero".to_owned());
    }

    ensure_quote_token_program(&snapshot.base_token_program, "PumpSwap base mint")?;
    ensure_quote_token_program(&snapshot.quote_token_program, "PumpSwap quote mint")?;

    if snapshot.base_vault_raw == 0 || snapshot.effective_quote_raw == 0 {
        return Err("PumpSwap quote reserves must be greater than zero".to_owned());
    }

    if snapshot.trading_state != PoolTradingState::Tradable {
        return Err("PumpSwap pool is not currently tradable".to_owned());
    }

    let canonical_pump_pool = is_canonical_pump_pool(&snapshot.pool_state)?;
    let market_cap_lamports = pool_market_cap(snapshot)?;
    let selected_fees = select_pumpswap_fees(snapshot, canonical_pump_pool, market_cap_lamports)?;
    let creator_fee_active = coin_creator_fee_is_active(&snapshot.pool_state)?;
    let creator_fee_bps = if creator_fee_active {
        selected_fees.creator_fee_bps
    } else {
        0
    };
    let total_fee_bps = selected_fees
        .lp_fee_bps
        .checked_add(selected_fees.protocol_fee_bps)
        .and_then(|fees| fees.checked_add(creator_fee_bps))
        .ok_or_else(|| "PumpSwap total fee-bps overflow".to_owned())?;

    if total_fee_bps > FEE_BPS_DENOMINATOR {
        return Err(format!(
            "PumpSwap total fee bps exceeds {FEE_BPS_DENOMINATOR}: {total_fee_bps}"
        ));
    }

    if input_mint == snapshot.pool_state.base_mint {
        quote_base_input(snapshot, amount_in_raw, selected_fees, creator_fee_bps)
    } else if input_mint == snapshot.pool_state.quote_mint {
        quote_quote_input(snapshot, amount_in_raw, selected_fees, creator_fee_bps)
    } else {
        Err(format!(
            "PumpSwap quote input mint {input_mint} is not in pool"
        ))
    }
}

fn ensure_quote_token_program(program: &str, label: &str) -> Result<(), String> {
    if program == SPL_TOKEN_PROGRAM_ID {
        return Ok(());
    }

    if program == TOKEN_2022_PROGRAM_ID {
        return Err(format!(
            "{label} uses Token-2022; transfer-fee extension state is not hydrated, so quoting fails closed"
        ));
    }

    Err(format!("{label} uses unsupported token program {program}"))
}

fn quote_base_input(
    snapshot: &PumpSwapHydrationSnapshot,
    amount_in_raw: u64,
    selected_fees: PumpSwapFees,
    creator_fee_bps: u64,
) -> Result<PumpSwapExactInputQuote, String> {
    let denominator = u128::from(snapshot.base_vault_raw)
        .checked_add(u128::from(amount_in_raw))
        .ok_or_else(|| "PumpSwap sell denominator overflow".to_owned())?;
    let numerator = u128::from(snapshot.effective_quote_raw)
        .checked_mul(u128::from(amount_in_raw))
        .ok_or_else(|| "PumpSwap sell multiplication overflow".to_owned())?;
    let raw_quote_out = u64::try_from(numerator / denominator)
        .map_err(|_| "PumpSwap sell output exceeded u64".to_owned())?;

    if raw_quote_out == 0 {
        return Err("PumpSwap sell output rounded to zero".to_owned());
    }

    let lp_fee_raw = fee_bps_ceil(raw_quote_out, selected_fees.lp_fee_bps)?;
    let protocol_fee_raw = fee_bps_ceil(raw_quote_out, selected_fees.protocol_fee_bps)?;
    let creator_fee_raw = fee_bps_ceil(raw_quote_out, creator_fee_bps)?;
    let total_fee_raw = lp_fee_raw
        .checked_add(protocol_fee_raw)
        .and_then(|fees| fees.checked_add(creator_fee_raw))
        .ok_or_else(|| "PumpSwap sell fee overflow".to_owned())?;
    let amount_out_raw = raw_quote_out
        .checked_sub(total_fee_raw)
        .ok_or_else(|| "PumpSwap sell fees exceed raw quote output".to_owned())?;

    if amount_out_raw == 0 {
        return Err("PumpSwap sell output is fully consumed by fees".to_owned());
    }

    Ok(PumpSwapExactInputQuote {
        amount_in_requested_raw: amount_in_raw,
        amount_in_consumed_raw: amount_in_raw,
        amount_in_unspent_raw: 0,
        amount_out_raw,
        lp_fee_raw,
        protocol_fee_raw,
        creator_fee_raw,
        source_slot: snapshot.slot,
    })
}

fn quote_quote_input(
    snapshot: &PumpSwapHydrationSnapshot,
    amount_in_raw: u64,
    selected_fees: PumpSwapFees,
    creator_fee_bps: u64,
) -> Result<PumpSwapExactInputQuote, String> {
    let effective_quote_raw = max_curve_input_for_budget(
        amount_in_raw,
        selected_fees.lp_fee_bps,
        selected_fees.protocol_fee_bps,
        creator_fee_bps,
    )?;

    if effective_quote_raw == 0 {
        return Err("PumpSwap buy effective quote rounded to zero".to_owned());
    }

    let curve_denominator = u128::from(snapshot.effective_quote_raw)
        .checked_add(u128::from(effective_quote_raw))
        .ok_or_else(|| "PumpSwap buy curve denominator overflow".to_owned())?;
    let base_out_raw = u64::try_from(
        u128::from(snapshot.base_vault_raw)
            .checked_mul(u128::from(effective_quote_raw))
            .ok_or_else(|| "PumpSwap buy multiplication overflow".to_owned())?
            / curve_denominator,
    )
    .map_err(|_| "PumpSwap buy output exceeded u64".to_owned())?;

    if base_out_raw == 0 || base_out_raw >= snapshot.base_vault_raw {
        return Err("PumpSwap buy output is zero or depletes the base reserve".to_owned());
    }

    let raw_quote_required = u64::try_from(ceil_div_u128(
        u128::from(snapshot.effective_quote_raw)
            .checked_mul(u128::from(base_out_raw))
            .ok_or_else(|| "PumpSwap buy reverse-quote multiplication overflow".to_owned())?,
        u128::from(snapshot.base_vault_raw - base_out_raw),
    )?)
    .map_err(|_| "PumpSwap buy raw quote exceeded u64".to_owned())?;

    let lp_fee_raw = fee_bps_ceil(raw_quote_required, selected_fees.lp_fee_bps)?;
    let protocol_fee_raw = fee_bps_ceil(raw_quote_required, selected_fees.protocol_fee_bps)?;
    let creator_fee_raw = fee_bps_ceil(raw_quote_required, creator_fee_bps)?;
    let amount_in_consumed_raw = raw_quote_required
        .checked_add(lp_fee_raw)
        .and_then(|amount| amount.checked_add(protocol_fee_raw))
        .and_then(|amount| amount.checked_add(creator_fee_raw))
        .ok_or_else(|| "PumpSwap buy consumed-input overflow".to_owned())?;

    if amount_in_consumed_raw > amount_in_raw {
        return Err(format!(
            "PumpSwap buy quote exceeded requested input: requested={amount_in_raw} consumed={amount_in_consumed_raw}"
        ));
    }

    Ok(PumpSwapExactInputQuote {
        amount_in_requested_raw: amount_in_raw,
        amount_in_consumed_raw,
        amount_in_unspent_raw: amount_in_raw - amount_in_consumed_raw,
        amount_out_raw: base_out_raw,
        lp_fee_raw,
        protocol_fee_raw,
        creator_fee_raw,
        source_slot: snapshot.slot,
    })
}

fn select_pumpswap_fees(
    snapshot: &PumpSwapHydrationSnapshot,
    canonical_pump_pool: bool,
    market_cap_lamports: u128,
) -> Result<PumpSwapFees, String> {
    if !canonical_pump_pool {
        return Ok(snapshot.fee_config.flat_fees.clone());
    }

    let first_tier = snapshot
        .fee_config
        .fee_tiers
        .first()
        .ok_or_else(|| "PumpSwap dynamic FeeConfig has no fee tiers".to_owned())?;

    if market_cap_lamports < first_tier.market_cap_lamports_threshold {
        return Ok(first_tier.fees.clone());
    }

    for tier in snapshot.fee_config.fee_tiers.iter().rev() {
        if market_cap_lamports >= tier.market_cap_lamports_threshold {
            return Ok(tier.fees.clone());
        }
    }

    Ok(first_tier.fees.clone())
}

fn pool_market_cap(snapshot: &PumpSwapHydrationSnapshot) -> Result<u128, String> {
    if snapshot.base_vault_raw == 0 {
        return Err("PumpSwap market-cap base reserve cannot be zero".to_owned());
    }

    u128::from(snapshot.effective_quote_raw)
        .checked_mul(u128::from(snapshot.base_mint_supply_raw))
        .map(|value| value / u128::from(snapshot.base_vault_raw))
        .ok_or_else(|| "PumpSwap market-cap multiplication overflow".to_owned())
}

fn is_canonical_pump_pool(pool: &PumpSwapPoolState) -> Result<bool, String> {
    let base_mint = Pubkey::from_str(&pool.base_mint)
        .map_err(|error| format!("invalid PumpSwap base mint for canonicality: {error}"))?;
    let pool_creator = Pubkey::from_str(&pool.creator)
        .map_err(|error| format!("invalid PumpSwap creator for canonicality: {error}"))?;
    let pump_program = Pubkey::from_str(PUMP_PROGRAM_ID)
        .map_err(|error| format!("invalid Pump program id: {error}"))?;
    let (expected_creator, _) =
        Pubkey::try_find_program_address(&[b"pool-authority", base_mint.as_ref()], &pump_program)
            .ok_or_else(|| "could not derive canonical PumpSwap pool authority PDA".to_owned())?;

    Ok(pool_creator == expected_creator)
}

fn coin_creator_fee_is_active(pool: &PumpSwapPoolState) -> Result<bool, String> {
    let coin_creator = pool.coin_creator.as_deref().ok_or_else(|| {
        "PumpSwap coin_creator is unavailable; creator fee cannot be quoted".to_owned()
    })?;

    Ok(coin_creator != DEFAULT_PUBKEY)
}

fn max_curve_input_for_budget(
    amount_in_raw: u64,
    lp_fee_bps: u64,
    protocol_fee_bps: u64,
    creator_fee_bps: u64,
) -> Result<u64, String> {
    let mut low = 0u64;
    let mut high = amount_in_raw;

    while low < high {
        let distance = high
            .checked_sub(low)
            .ok_or_else(|| "PumpSwap buy budget-search underflow".to_owned())?;
        let midpoint = low
            .checked_add(distance / 2)
            .and_then(|value| value.checked_add(distance % 2))
            .ok_or_else(|| "PumpSwap buy budget-search midpoint overflow".to_owned())?;

        let lp_fee_raw = fee_bps_ceil(midpoint, lp_fee_bps)?;
        let protocol_fee_raw = fee_bps_ceil(midpoint, protocol_fee_bps)?;
        let creator_fee_raw = fee_bps_ceil(midpoint, creator_fee_bps)?;
        let consumed = midpoint
            .checked_add(lp_fee_raw)
            .and_then(|amount| amount.checked_add(protocol_fee_raw))
            .and_then(|amount| amount.checked_add(creator_fee_raw));

        if matches!(consumed, Some(value) if value <= amount_in_raw) {
            low = midpoint;
        } else {
            high = midpoint
                .checked_sub(1)
                .ok_or_else(|| "PumpSwap buy budget-search underflow".to_owned())?;
        }
    }

    Ok(low)
}

fn fee_bps_ceil(amount: u64, basis_points: u64) -> Result<u64, String> {
    if amount == 0 || basis_points == 0 {
        return Ok(0);
    }

    let numerator = u128::from(amount)
        .checked_mul(u128::from(basis_points))
        .ok_or_else(|| "PumpSwap fee multiplication overflow".to_owned())?;
    let value = ceil_div_u128(numerator, u128::from(FEE_BPS_DENOMINATOR))?;

    u64::try_from(value).map_err(|_| "PumpSwap fee exceeded u64".to_owned())
}

fn ceil_div_u128(numerator: u128, denominator: u128) -> Result<u128, String> {
    if denominator == 0 {
        return Err("PumpSwap division by zero".to_owned());
    }

    if numerator == 0 {
        return Ok(0);
    }

    numerator
        .checked_sub(1)
        .and_then(|value| value.checked_div(denominator))
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| "PumpSwap ceiling division overflow".to_owned())
}

pub fn program_subscribe_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 4,
        "method": "programSubscribe",
        "params": [
            PUMPSWAP_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "filters": [
                    {
                        "memcmp": {
                            "offset": 0,
                            "bytes": "hQrXeCntzbV"
                        }
                    }
                ]
            }
        ]
    })
}

pub fn pair_lookup_requests(anchor_mint: &str, intermediate_mint: &str) -> [Value; 2] {
    [
        pair_lookup_request(7, anchor_mint, intermediate_mint),
        pair_lookup_request(8, intermediate_mint, anchor_mint),
    ]
}

fn pair_lookup_request(request_id: u64, base_mint: &str, quote_mint: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getProgramAccounts",
        "params": [
            PUMPSWAP_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "withContext": true,
                "filters": [
                    {
                        "memcmp": {
                            "offset": 0,
                            "bytes": "hQrXeCntzbV"
                        }
                    },
                    {
                        "memcmp": {
                            "offset": POOL_BASE_MINT_OFFSET,
                            "bytes": base_mint
                        }
                    },
                    {
                        "memcmp": {
                            "offset": POOL_QUOTE_MINT_OFFSET,
                            "bytes": quote_mint
                        }
                    }
                ]
            }
        ]
    })
}

pub fn parse_pair_lookup_response(
    payload: &Value,
) -> Result<Vec<PumpSwapAccountObservation>, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "PumpSwap getProgramAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "PumpSwap getProgramAccounts response missing context slot".to_owned())?;

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "PumpSwap getProgramAccounts response missing account array".to_owned())?;

    let mut observations = Vec::with_capacity(accounts.len());

    for entry in accounts {
        let pubkey = entry
            .get("pubkey")
            .and_then(Value::as_str)
            .ok_or_else(|| "PumpSwap getProgramAccounts entry missing pubkey".to_owned())?;

        let account = entry
            .get("account")
            .ok_or_else(|| "PumpSwap getProgramAccounts entry missing account".to_owned())?;

        let notification = json!({
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": slot
                    },
                    "value": {
                        "pubkey": pubkey,
                        "account": account
                    }
                }
            }
        });

        let observation = parse_program_notification(&notification)?
            .ok_or_else(|| "PumpSwap pair lookup account did not decode".to_owned())?;

        if observations
            .iter()
            .any(|existing: &PumpSwapAccountObservation| existing.pubkey == observation.pubkey)
        {
            continue;
        }

        observations.push(observation);
    }

    Ok(observations)
}

pub fn hydration_account_pubkeys(observation: &PumpSwapAccountObservation) -> [String; 7] {
    [
        observation.pubkey.clone(),
        observation.pool_state.pool_base_token_account.clone(),
        observation.pool_state.pool_quote_token_account.clone(),
        observation.pool_state.base_mint.clone(),
        observation.pool_state.quote_mint.clone(),
        PUMPSWAP_GLOBAL_CONFIG.to_owned(),
        PUMPSWAP_FEE_CONFIG.to_owned(),
    ]
}

pub fn parse_program_notification(
    payload: &Value,
) -> Result<Option<PumpSwapAccountObservation>, String> {
    if payload.get("method").and_then(Value::as_str) != Some("programNotification") {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "PumpSwap notification missing slot".to_owned())?;

    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "PumpSwap notification missing pubkey".to_owned())?
        .to_owned();

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "PumpSwap notification missing owner".to_owned())?
        .to_owned();

    if owner != PUMPSWAP_PROGRAM_ID {
        return Err(format!("unexpected PumpSwap account owner: {owner}"));
    }

    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "PumpSwap notification missing base64 account data".to_owned())?;

    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "PumpSwap notification missing account-data encoding".to_owned())?;

    if encoding != "base64" {
        return Err(format!(
            "unexpected PumpSwap account-data encoding: {encoding}"
        ));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid PumpSwap base64 account data: {error}"))?;

    let pool_state = decode_pool_state(&decoded_data)?;

    Ok(Some(PumpSwapAccountObservation {
        pubkey,
        slot,
        owner,
        encoded_data_len: encoded_data.len(),
        decoded_data_len: decoded_data.len(),
        pool_state,
    }))
}

pub fn parse_hydration_response(
    observation: &PumpSwapAccountObservation,
    payload: &Value,
) -> Result<PumpSwapHydrationSnapshot, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Solana getMultipleAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Solana getMultipleAccounts response missing context slot".to_owned())?;

    if slot < observation.slot {
        return Err(format!(
            "stale PumpSwap hydration snapshot: trigger_slot={} reserve_slot={slot}",
            observation.slot
        ));
    }

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Solana getMultipleAccounts response missing account array".to_owned())?;

    if accounts.len() != 7 {
        return Err(format!(
            "PumpSwap hydration expected exactly 7 accounts, got {}",
            accounts.len()
        ));
    }

    if accounts.iter().any(Value::is_null) {
        return Err("PumpSwap hydration response contained a missing account".to_owned());
    }

    let pool_data = decode_rpc_account_data(&accounts[0], PUMPSWAP_PROGRAM_ID, "PumpSwap pool")?;
    let pool_state = decode_pool_state(&pool_data)?;

    verify_pool_identity(&observation.pool_state, &pool_state)?;

    let (base_mint_owner, base_mint_data) =
        decode_supported_mint_account(&accounts[3], "PumpSwap base mint")?;

    let (quote_mint_owner, quote_mint_data) =
        decode_supported_mint_account(&accounts[4], "PumpSwap quote mint")?;

    let base_decimals = parse_mint_decimals(&base_mint_data, "PumpSwap base mint")?;
    let quote_decimals = parse_mint_decimals(&quote_mint_data, "PumpSwap quote mint")?;
    let base_mint_supply_raw = parse_mint_supply(&base_mint_data, "PumpSwap base mint")?;

    let base_vault_raw = parse_token_vault_account(
        &accounts[1],
        &base_mint_owner,
        &pool_state.base_mint,
        "PumpSwap base vault",
    )?;

    let quote_vault_raw = parse_token_vault_account(
        &accounts[2],
        &quote_mint_owner,
        &pool_state.quote_mint,
        "PumpSwap quote vault",
    )?;

    let disable_flags = parse_global_config(&accounts[5])?;
    let trading_state = trading_state_from_disable_flags(disable_flags);
    let fee_config = parse_fee_config(&accounts[6])?;

    let effective_quote = i128::from(quote_vault_raw)
        .checked_add(pool_state.virtual_quote_reserves)
        .ok_or_else(|| "PumpSwap effective quote reserve overflow".to_owned())?;

    let effective_quote_raw = u64::try_from(effective_quote).map_err(|_| {
        format!(
            "PumpSwap effective quote reserve outside u64 range: raw={} virtual={} effective={}",
            quote_vault_raw, pool_state.virtual_quote_reserves, effective_quote
        )
    })?;

    Ok(PumpSwapHydrationSnapshot {
        slot,
        pool_state,
        base_vault_raw,
        quote_vault_raw,
        effective_quote_raw,
        base_mint_supply_raw,
        base_token_program: base_mint_owner,
        quote_token_program: quote_mint_owner,
        base_decimals,
        quote_decimals,
        disable_flags,
        trading_state,
        fee_config,
    })
}

pub fn hydrate_normalized_observation(
    observation: &PumpSwapAccountObservation,
    snapshot: &PumpSwapHydrationSnapshot,
    account_update_received_at_unix_ms: u64,
    hydrated_at_unix_ms: u64,
) -> Result<NormalizedPoolState, String> {
    if snapshot.slot < observation.slot {
        return Err(format!(
            concat!(
                "stale PumpSwap hydration snapshot: ",
                "trigger_slot={} reserve_slot={}"
            ),
            observation.slot, snapshot.slot
        ));
    }

    verify_pool_identity(&observation.pool_state, &snapshot.pool_state)?;

    Ok(NormalizedPoolState {
        pool_id: observation.pubkey.clone(),
        venue: Venue::PumpSwap,
        program_id: observation.owner.clone(),
        source_slot: observation.slot,
        token_a: NormalizedToken {
            mint: snapshot.pool_state.base_mint.clone(),
            vault: snapshot.pool_state.pool_base_token_account.clone(),
            decimals: snapshot.base_decimals,
        },
        token_b: NormalizedToken {
            mint: snapshot.pool_state.quote_mint.clone(),
            vault: snapshot.pool_state.pool_quote_token_account.clone(),
            decimals: snapshot.quote_decimals,
        },
        trading_state: snapshot.trading_state,
        quote_reserves: QuoteReserveState::Available {
            token_a_raw: snapshot.base_vault_raw,
            token_b_raw: snapshot.effective_quote_raw,
            source_slot: snapshot.slot,
        },
        account_update_received_at_unix_ms,
        normalized_at_unix_ms: hydrated_at_unix_ms,
    })
}

fn decode_pool_state(data: &[u8]) -> Result<PumpSwapPoolState, String> {
    if data.len() < POOL_BASE_LEN {
        return Err(format!(
            "unexpected PumpSwap Pool account length: expected at least {}, got {}",
            POOL_BASE_LEN,
            data.len()
        ));
    }

    if data.get(..8) != Some(POOL_DISCRIMINATOR.as_slice()) {
        return Err("unexpected PumpSwap Pool discriminator".to_owned());
    }

    let pool_bump = data[8];
    let index = u16::from_le_bytes(read_array::<2>(data, 9, "index")?);
    let creator = pubkey_at(data, 11, "creator")?;
    let base_mint = pubkey_at(data, POOL_BASE_MINT_OFFSET, "base_mint")?;
    let quote_mint = pubkey_at(data, POOL_QUOTE_MINT_OFFSET, "quote_mint")?;
    let lp_mint = pubkey_at(data, 107, "lp_mint")?;
    let pool_base_token_account = pubkey_at(data, 139, "pool_base_token_account")?;
    let pool_quote_token_account = pubkey_at(data, 171, "pool_quote_token_account")?;
    let lp_supply = u64::from_le_bytes(read_array::<8>(data, 203, "lp_supply")?);

    let valid_extension_boundary = matches!(
        data.len(),
        POOL_BASE_LEN | POOL_COIN_CREATOR_END | POOL_MAYHEM_END | POOL_CASHBACK_END
    ) || data.len() >= POOL_VIRTUAL_QUOTE_END;

    if !valid_extension_boundary {
        return Err(format!(
            concat!(
                "PumpSwap Pool account ended inside ",
                "an appended field: length={}"
            ),
            data.len()
        ));
    }

    let coin_creator = if data.len() >= POOL_COIN_CREATOR_END {
        Some(pubkey_at(data, 211, "coin_creator")?)
    } else {
        None
    };

    let is_mayhem_mode = if data.len() >= POOL_MAYHEM_END {
        Some(parse_bool(data[243], "is_mayhem_mode")?)
    } else {
        None
    };

    let is_cashback_coin = if data.len() >= POOL_CASHBACK_END {
        Some(parse_bool(data[244], "is_cashback_coin")?)
    } else {
        None
    };

    let virtual_quote_reserves = if data.len() >= POOL_VIRTUAL_QUOTE_END {
        i128::from_le_bytes(read_array::<16>(data, 245, "virtual_quote_reserves")?)
    } else {
        0
    };

    Ok(PumpSwapPoolState {
        pool_bump,
        index,
        creator,
        base_mint,
        quote_mint,
        lp_mint,
        pool_base_token_account,
        pool_quote_token_account,
        lp_supply,
        coin_creator,
        is_mayhem_mode,
        is_cashback_coin,
        virtual_quote_reserves,
    })
}

fn verify_pool_identity(
    trigger: &PumpSwapPoolState,
    snapshot: &PumpSwapPoolState,
) -> Result<(), String> {
    if trigger.pool_bump != snapshot.pool_bump {
        return Err("PumpSwap hydration pool_bump changed".to_owned());
    }

    if trigger.index != snapshot.index {
        return Err("PumpSwap hydration pool index changed".to_owned());
    }

    if trigger.creator != snapshot.creator {
        return Err("PumpSwap hydration creator changed".to_owned());
    }

    if trigger.lp_mint != snapshot.lp_mint {
        return Err("PumpSwap hydration lp_mint changed".to_owned());
    }

    if trigger.base_mint != snapshot.base_mint {
        return Err("PumpSwap hydration base_mint changed".to_owned());
    }

    if trigger.quote_mint != snapshot.quote_mint {
        return Err("PumpSwap hydration quote_mint changed".to_owned());
    }

    if trigger.pool_base_token_account != snapshot.pool_base_token_account {
        return Err("PumpSwap hydration base vault changed".to_owned());
    }

    if trigger.pool_quote_token_account != snapshot.pool_quote_token_account {
        return Err("PumpSwap hydration quote vault changed".to_owned());
    }

    Ok(())
}

fn parse_fee_config(account: &Value) -> Result<PumpSwapFeeConfig, String> {
    let data = decode_rpc_account_data(account, PUMP_FEES_PROGRAM_ID, "PumpSwap FeeConfig")?;

    decode_fee_config(&data)
}

fn decode_fee_config(data: &[u8]) -> Result<PumpSwapFeeConfig, String> {
    let minimum_len = FEE_CONFIG_FIXED_PREFIX_LEN
        .checked_add(8)
        .ok_or_else(|| "PumpSwap FeeConfig minimum length overflow".to_owned())?;

    if data.len() < minimum_len {
        return Err(format!(
            "PumpSwap FeeConfig shorter than required prefix: expected at least {}, got {}",
            minimum_len,
            data.len()
        ));
    }

    if data.get(..8) != Some(FEE_CONFIG_DISCRIMINATOR.as_slice()) {
        return Err("unexpected PumpSwap FeeConfig discriminator".to_owned());
    }

    let bump = data[8];
    let admin = pubkey_at(data, 9, "FeeConfig admin")?;
    let flat_fees = fees_at(data, 41, "FeeConfig flat_fees")?;

    let mut offset = FEE_CONFIG_FIXED_PREFIX_LEN;
    let fee_tiers = fee_tier_vec(data, &mut offset, "FeeConfig fee_tiers")?;
    let stable_fee_tiers = fee_tier_vec(data, &mut offset, "FeeConfig stable_fee_tiers")?;

    let fee_config = PumpSwapFeeConfig {
        bump,
        admin,
        flat_fees,
        fee_tiers,
        stable_fee_tiers,
    };

    validate_fee_config(&fee_config)?;

    Ok(fee_config)
}

fn fee_tier_vec(
    data: &[u8],
    offset: &mut usize,
    label: &str,
) -> Result<Vec<PumpSwapFeeTier>, String> {
    let count_bytes = read_array::<4>(data, *offset, label)?;
    let count = u32::from_le_bytes(count_bytes) as usize;

    *offset = (*offset)
        .checked_add(4)
        .ok_or_else(|| format!("PumpSwap {label} vector-length offset overflow"))?;

    let serialized_len = count
        .checked_mul(FEE_TIER_SERIALIZED_LEN)
        .ok_or_else(|| format!("PumpSwap {label} serialized length overflow"))?;

    let end = (*offset)
        .checked_add(serialized_len)
        .ok_or_else(|| format!("PumpSwap {label} end offset overflow"))?;

    if end > data.len() {
        return Err(format!(
            "PumpSwap {label} exceeds account data: count={count} end={end} len={}",
            data.len()
        ));
    }

    let mut tiers = Vec::with_capacity(count);

    for index in 0..count {
        let threshold = u128::from_le_bytes(read_array::<16>(
            data,
            *offset,
            &format!("{label}[{index}] market_cap_lamports_threshold"),
        )?);

        *offset = (*offset)
            .checked_add(16)
            .ok_or_else(|| format!("PumpSwap {label}[{index}] threshold offset overflow"))?;

        let fees = fees_at(data, *offset, &format!("{label}[{index}] fees"))?;

        *offset = (*offset)
            .checked_add(24)
            .ok_or_else(|| format!("PumpSwap {label}[{index}] fees offset overflow"))?;

        tiers.push(PumpSwapFeeTier {
            market_cap_lamports_threshold: threshold,
            fees,
        });
    }

    Ok(tiers)
}

fn fees_at(data: &[u8], offset: usize, label: &str) -> Result<PumpSwapFees, String> {
    let protocol_offset = offset
        .checked_add(8)
        .ok_or_else(|| format!("PumpSwap {label} protocol fee offset overflow"))?;
    let creator_offset = offset
        .checked_add(16)
        .ok_or_else(|| format!("PumpSwap {label} creator fee offset overflow"))?;

    Ok(PumpSwapFees {
        lp_fee_bps: u64::from_le_bytes(read_array::<8>(data, offset, label)?),
        protocol_fee_bps: u64::from_le_bytes(read_array::<8>(data, protocol_offset, label)?),
        creator_fee_bps: u64::from_le_bytes(read_array::<8>(data, creator_offset, label)?),
    })
}

fn validate_fee_config(fee_config: &PumpSwapFeeConfig) -> Result<(), String> {
    if fee_config.bump != PUMPSWAP_FEE_CONFIG_BUMP {
        return Err(format!(
            "PumpSwap FeeConfig bump mismatch: expected {}, got {}",
            PUMPSWAP_FEE_CONFIG_BUMP, fee_config.bump
        ));
    }

    validate_fees(&fee_config.flat_fees, "FeeConfig flat_fees")?;

    if fee_config.fee_tiers.is_empty() {
        return Err("PumpSwap FeeConfig has no dynamic fee tiers".to_owned());
    }

    validate_fee_tiers(&fee_config.fee_tiers, "FeeConfig fee_tiers")?;
    validate_fee_tiers(&fee_config.stable_fee_tiers, "FeeConfig stable_fee_tiers")?;

    Ok(())
}

fn validate_fee_tiers(tiers: &[PumpSwapFeeTier], label: &str) -> Result<(), String> {
    let mut previous_threshold = None;

    for (index, tier) in tiers.iter().enumerate() {
        validate_fees(&tier.fees, &format!("{label}[{index}] fees"))?;

        if let Some(previous) = previous_threshold {
            if tier.market_cap_lamports_threshold < previous {
                return Err(format!(
                    "PumpSwap {label} thresholds are not ascending at index {index}"
                ));
            }
        }

        previous_threshold = Some(tier.market_cap_lamports_threshold);
    }

    Ok(())
}

fn validate_fees(fees: &PumpSwapFees, label: &str) -> Result<(), String> {
    let total = fees
        .lp_fee_bps
        .checked_add(fees.protocol_fee_bps)
        .and_then(|value| value.checked_add(fees.creator_fee_bps))
        .ok_or_else(|| format!("PumpSwap {label} total fee overflow"))?;

    if total > MAX_FEE_BPS {
        return Err(format!(
            "PumpSwap {label} total fee exceeds {MAX_FEE_BPS} bps: {total}"
        ));
    }

    Ok(())
}

fn parse_global_config(account: &Value) -> Result<u8, String> {
    let data = decode_rpc_account_data(account, PUMPSWAP_PROGRAM_ID, "PumpSwap GlobalConfig")?;

    if data.len() < GLOBAL_CONFIG_MIN_LEN {
        return Err(format!(
            "PumpSwap GlobalConfig shorter than required prefix: expected at least {}, got {}",
            GLOBAL_CONFIG_MIN_LEN,
            data.len()
        ));
    }

    if data.get(..8) != Some(GLOBAL_CONFIG_DISCRIMINATOR.as_slice()) {
        return Err("unexpected PumpSwap GlobalConfig discriminator".to_owned());
    }

    Ok(data[GLOBAL_CONFIG_DISABLE_FLAGS_OFFSET])
}

fn trading_state_from_disable_flags(disable_flags: u8) -> PoolTradingState {
    if disable_flags & (DISABLE_BUY_MASK | DISABLE_SELL_MASK) != 0 {
        PoolTradingState::SwapDisabled
    } else {
        PoolTradingState::Tradable
    }
}

fn decode_supported_mint_account(
    account: &Value,
    label: &str,
) -> Result<(String, Vec<u8>), String> {
    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing owner"))?;

    if !is_supported_token_program(owner) {
        return Err(format!(
            "{label} has unsupported token program owner: {owner}"
        ));
    }

    let data = decode_rpc_account_data(account, owner, label)?;

    Ok((owner.to_owned(), data))
}

fn parse_mint_supply(data: &[u8], label: &str) -> Result<u64, String> {
    if data.len() < MINT_ACCOUNT_BASE_LEN {
        return Err(format!(
            "{label} shorter than SPL Mint base layout: expected at least {}, got {}",
            MINT_ACCOUNT_BASE_LEN,
            data.len()
        ));
    }

    Ok(u64::from_le_bytes(read_array::<8>(
        data,
        MINT_SUPPLY_OFFSET,
        "mint supply",
    )?))
}

fn parse_mint_decimals(data: &[u8], label: &str) -> Result<u8, String> {
    if data.len() < MINT_ACCOUNT_BASE_LEN {
        return Err(format!(
            "{label} shorter than SPL Mint base layout: expected at least {}, got {}",
            MINT_ACCOUNT_BASE_LEN,
            data.len()
        ));
    }

    match data[MINT_INITIALIZED_OFFSET] {
        1 => Ok(data[MINT_DECIMALS_OFFSET]),
        0 => Err(format!("{label} is not initialized")),
        value => Err(format!("{label} has invalid is_initialized value: {value}")),
    }
}

fn parse_token_vault_account(
    account: &Value,
    expected_owner: &str,
    expected_mint: &str,
    label: &str,
) -> Result<u64, String> {
    let data = decode_rpc_account_data(account, expected_owner, label)?;

    if data.len() < TOKEN_ACCOUNT_BASE_LEN {
        return Err(format!(
            "{label} shorter than SPL Token account base layout: expected at least {}, got {}",
            TOKEN_ACCOUNT_BASE_LEN,
            data.len()
        ));
    }

    let mint = bs58::encode(read_array::<32>(
        &data,
        TOKEN_ACCOUNT_MINT_OFFSET,
        "vault mint",
    )?)
    .into_string();

    if mint != expected_mint {
        return Err(format!(
            "{label} mint mismatch: expected {expected_mint}, got {mint}"
        ));
    }

    Ok(u64::from_le_bytes(read_array::<8>(
        &data,
        TOKEN_ACCOUNT_AMOUNT_OFFSET,
        "vault amount",
    )?))
}

fn decode_rpc_account_data(
    account: &Value,
    expected_owner: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing owner"))?;

    if owner != expected_owner {
        return Err(format!(
            "{label} owner mismatch: expected {expected_owner}, got {owner}"
        ));
    }

    let encoded = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing base64 data"))?;

    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing data encoding"))?;

    if encoding != "base64" {
        return Err(format!("{label} unexpected encoding: {encoding}"));
    }

    BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("{label} invalid base64 data: {error}"))
}

fn is_supported_token_program(owner: &str) -> bool {
    owner == SPL_TOKEN_PROGRAM_ID || owner == TOKEN_2022_PROGRAM_ID
}

fn pubkey_at(data: &[u8], offset: usize, label: &str) -> Result<String, String> {
    Ok(bs58::encode(read_array::<32>(data, offset, label)?).into_string())
}

fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("PumpSwap {label} offset overflow"))?;

    let slice = data
        .get(offset..end)
        .ok_or_else(|| format!("PumpSwap {label} outside account data"))?;

    slice
        .try_into()
        .map_err(|_| format!("PumpSwap {label} length mismatch"))
}

fn parse_bool(value: u8, label: &str) -> Result<bool, String> {
    match value {
        0 => Ok(false),
        1 => Ok(true),
        _ => Err(format!("PumpSwap {label} invalid bool value: {value}")),
    }
}

fn optional_bool_label(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "true",
        Some(false) => "false",
        None => "unavailable",
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pool_data() -> Vec<u8> {
        let mut data = vec![0u8; POOL_VIRTUAL_QUOTE_END];

        data[..8].copy_from_slice(&POOL_DISCRIMINATOR);
        data[8] = 254;
        data[9..11].copy_from_slice(&0u16.to_le_bytes());

        for (offset, seed) in [
            (11, 1u8),
            (43, 2),
            (75, 3),
            (107, 4),
            (139, 5),
            (171, 6),
            (211, 7),
        ] {
            data[offset..offset + 32].fill(seed);
        }

        data[203..211].copy_from_slice(&123u64.to_le_bytes());
        data[243] = 1;
        data[244] = 0;
        data[245..261].copy_from_slice(&25i128.to_le_bytes());

        data
    }

    fn sample_global_config_data(disable_flags: u8) -> Vec<u8> {
        let mut data = vec![0u8; GLOBAL_CONFIG_MIN_LEN];
        data[..8].copy_from_slice(&GLOBAL_CONFIG_DISCRIMINATOR);
        data[GLOBAL_CONFIG_DISABLE_FLAGS_OFFSET] = disable_flags;
        data
    }

    fn append_fees(data: &mut Vec<u8>, lp: u64, protocol: u64, creator: u64) {
        data.extend_from_slice(&lp.to_le_bytes());
        data.extend_from_slice(&protocol.to_le_bytes());
        data.extend_from_slice(&creator.to_le_bytes());
    }

    fn append_tier(data: &mut Vec<u8>, threshold: u128, lp: u64, protocol: u64, creator: u64) {
        data.extend_from_slice(&threshold.to_le_bytes());
        append_fees(data, lp, protocol, creator);
    }

    fn sample_fee_config_data() -> Vec<u8> {
        let mut data = Vec::new();
        data.extend_from_slice(&FEE_CONFIG_DISCRIMINATOR);
        data.push(PUMPSWAP_FEE_CONFIG_BUMP);
        data.extend_from_slice(&[9u8; 32]);
        append_fees(&mut data, 20, 5, 5);

        data.extend_from_slice(&2u32.to_le_bytes());
        append_tier(&mut data, 0, 30, 10, 5);
        append_tier(&mut data, 1_000_000, 20, 5, 5);

        data.extend_from_slice(&1u32.to_le_bytes());
        append_tier(&mut data, 0, 10, 5, 0);

        data
    }

    fn rpc_account(owner: &str, data: &[u8]) -> Value {
        json!({
            "owner": owner,
            "data": [
                BASE64_STANDARD.encode(data),
                "base64"
            ]
        })
    }

    fn initialized_mint(decimals: u8) -> Vec<u8> {
        let mut data = vec![0u8; MINT_ACCOUNT_BASE_LEN];
        data[MINT_SUPPLY_OFFSET..MINT_SUPPLY_OFFSET + 8].copy_from_slice(&123_456u64.to_le_bytes());
        data[MINT_DECIMALS_OFFSET] = decimals;
        data[MINT_INITIALIZED_OFFSET] = 1;
        data
    }

    #[test]
    fn decodes_current_pool_layout() -> Result<(), String> {
        let state = decode_pool_state(&sample_pool_data())?;

        assert_eq!(state.pool_bump, 254);
        assert_eq!(state.lp_supply, 123);
        assert_eq!(state.is_mayhem_mode, Some(true));
        assert_eq!(state.is_cashback_coin, Some(false));
        assert_eq!(state.virtual_quote_reserves, 25);

        Ok(())
    }

    #[test]
    fn accepts_legacy_pool_prefix_without_appended_fields() -> Result<(), String> {
        let data = sample_pool_data()[..POOL_BASE_LEN].to_vec();

        let state = decode_pool_state(&data)?;

        assert_eq!(state.coin_creator, None);
        assert_eq!(state.is_mayhem_mode, None);
        assert_eq!(state.is_cashback_coin, None);
        assert_eq!(state.virtual_quote_reserves, 0);

        Ok(())
    }

    #[test]
    fn accepts_future_extension_bytes() -> Result<(), String> {
        let mut data = sample_pool_data();
        data.extend_from_slice(&[9u8; 24]);

        let state = decode_pool_state(&data)?;

        assert_eq!(state.virtual_quote_reserves, 25);

        Ok(())
    }

    #[test]
    fn rejects_partial_appended_field() -> Result<(), String> {
        let data = sample_pool_data()[..220].to_vec();

        let error = match decode_pool_state(&data) {
            Err(error) => error,
            Ok(_) => return Err("partial extension unexpectedly decoded".to_owned()),
        };

        assert!(error.contains("ended inside an appended field"));

        Ok(())
    }

    #[test]
    fn rejects_wrong_discriminator() -> Result<(), String> {
        let mut data = sample_pool_data();
        data[0] ^= 1;

        let error = match decode_pool_state(&data) {
            Err(error) => error,
            Ok(_) => return Err("wrong discriminator unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "unexpected PumpSwap Pool discriminator");

        Ok(())
    }

    #[test]
    fn rejects_invalid_appended_bool() -> Result<(), String> {
        let mut data = sample_pool_data();
        data[243] = 2;

        let error = match decode_pool_state(&data) {
            Err(error) => error,
            Ok(_) => return Err("invalid bool unexpectedly decoded".to_owned()),
        };

        assert!(error.contains("is_mayhem_mode invalid bool"));

        Ok(())
    }

    #[test]
    fn hydration_includes_global_and_fee_config() -> Result<(), String> {
        let state = decode_pool_state(&sample_pool_data())?;

        let observation = PumpSwapAccountObservation {
            pubkey: "pool".to_owned(),
            slot: 1,
            owner: PUMPSWAP_PROGRAM_ID.to_owned(),
            encoded_data_len: 0,
            decoded_data_len: POOL_VIRTUAL_QUOTE_END,
            pool_state: state,
        };

        let accounts = hydration_account_pubkeys(&observation);

        assert_eq!(accounts.len(), 7);
        assert_eq!(accounts[5], PUMPSWAP_GLOBAL_CONFIG);
        assert_eq!(accounts[6], PUMPSWAP_FEE_CONFIG);

        Ok(())
    }

    #[test]
    fn decodes_fee_config() -> Result<(), String> {
        let fee_config = decode_fee_config(&sample_fee_config_data())?;

        assert_eq!(fee_config.bump, PUMPSWAP_FEE_CONFIG_BUMP);
        assert_eq!(fee_config.flat_fees.lp_fee_bps, 20);
        assert_eq!(fee_config.flat_fees.protocol_fee_bps, 5);
        assert_eq!(fee_config.flat_fees.creator_fee_bps, 5);
        assert_eq!(fee_config.fee_tiers.len(), 2);
        assert_eq!(
            fee_config.fee_tiers[1].market_cap_lamports_threshold,
            1_000_000
        );
        assert_eq!(fee_config.stable_fee_tiers.len(), 1);

        Ok(())
    }

    #[test]
    fn rejects_wrong_fee_config_discriminator() -> Result<(), String> {
        let mut data = sample_fee_config_data();
        data[0] ^= 1;

        let error = match decode_fee_config(&data) {
            Err(error) => error,
            Ok(_) => return Err("wrong FeeConfig discriminator unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "unexpected PumpSwap FeeConfig discriminator");

        Ok(())
    }

    #[test]
    fn rejects_wrong_fee_config_bump() -> Result<(), String> {
        let mut data = sample_fee_config_data();
        data[8] = PUMPSWAP_FEE_CONFIG_BUMP.saturating_sub(1);

        let error = match decode_fee_config(&data) {
            Err(error) => error,
            Ok(_) => return Err("wrong FeeConfig bump unexpectedly decoded".to_owned()),
        };

        assert!(error.contains("FeeConfig bump mismatch"));

        Ok(())
    }

    #[test]
    fn rejects_empty_dynamic_fee_tiers() -> Result<(), String> {
        let mut data = Vec::new();
        data.extend_from_slice(&FEE_CONFIG_DISCRIMINATOR);
        data.push(PUMPSWAP_FEE_CONFIG_BUMP);
        data.extend_from_slice(&[9u8; 32]);
        append_fees(&mut data, 20, 5, 5);
        data.extend_from_slice(&0u32.to_le_bytes());
        data.extend_from_slice(&0u32.to_le_bytes());

        let error = match decode_fee_config(&data) {
            Err(error) => error,
            Ok(_) => return Err("empty FeeConfig tiers unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "PumpSwap FeeConfig has no dynamic fee tiers");

        Ok(())
    }

    #[test]
    fn rejects_unsorted_dynamic_fee_tiers() -> Result<(), String> {
        let mut data = Vec::new();
        data.extend_from_slice(&FEE_CONFIG_DISCRIMINATOR);
        data.push(PUMPSWAP_FEE_CONFIG_BUMP);
        data.extend_from_slice(&[9u8; 32]);
        append_fees(&mut data, 20, 5, 5);
        data.extend_from_slice(&2u32.to_le_bytes());
        append_tier(&mut data, 100, 20, 5, 5);
        append_tier(&mut data, 99, 20, 5, 5);
        data.extend_from_slice(&0u32.to_le_bytes());

        let error = match decode_fee_config(&data) {
            Err(error) => error,
            Ok(_) => return Err("unsorted FeeConfig tiers unexpectedly decoded".to_owned()),
        };

        assert!(error.contains("thresholds are not ascending"));

        Ok(())
    }

    #[test]
    fn rejects_fee_total_above_basis_point_denominator() -> Result<(), String> {
        let mut data = Vec::new();
        data.extend_from_slice(&FEE_CONFIG_DISCRIMINATOR);
        data.push(PUMPSWAP_FEE_CONFIG_BUMP);
        data.extend_from_slice(&[9u8; 32]);
        append_fees(&mut data, 9_999, 2, 0);
        data.extend_from_slice(&1u32.to_le_bytes());
        append_tier(&mut data, 0, 20, 5, 5);
        data.extend_from_slice(&0u32.to_le_bytes());

        let error = match decode_fee_config(&data) {
            Err(error) => error,
            Ok(_) => return Err("invalid FeeConfig fee total unexpectedly decoded".to_owned()),
        };

        assert!(error.contains("total fee exceeds 10000 bps"));

        Ok(())
    }

    #[test]
    fn parses_global_config_disable_flags() -> Result<(), String> {
        let account = rpc_account(
            PUMPSWAP_PROGRAM_ID,
            &sample_global_config_data(DISABLE_BUY_MASK | DISABLE_SELL_MASK),
        );

        let flags = parse_global_config(&account)?;

        assert_eq!(flags, DISABLE_BUY_MASK | DISABLE_SELL_MASK);

        Ok(())
    }

    #[test]
    fn rejects_wrong_global_config_discriminator() -> Result<(), String> {
        let mut data = sample_global_config_data(0);
        data[0] ^= 1;

        let account = rpc_account(PUMPSWAP_PROGRAM_ID, &data);

        let error = match parse_global_config(&account) {
            Err(error) => error,
            Ok(_) => return Err("wrong GlobalConfig discriminator unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "unexpected PumpSwap GlobalConfig discriminator");

        Ok(())
    }

    #[test]
    fn buy_disable_flag_disables_swaps() {
        assert_eq!(
            trading_state_from_disable_flags(DISABLE_BUY_MASK),
            PoolTradingState::SwapDisabled
        );
    }

    #[test]
    fn sell_disable_flag_disables_swaps() {
        assert_eq!(
            trading_state_from_disable_flags(DISABLE_SELL_MASK),
            PoolTradingState::SwapDisabled
        );
    }

    #[test]
    fn unrelated_disable_flags_remain_tradable() {
        assert_eq!(
            trading_state_from_disable_flags(1 << 1),
            PoolTradingState::Tradable
        );
    }

    #[test]
    fn initialized_mint_returns_supply() -> Result<(), String> {
        let data = initialized_mint(9);
        let supply = parse_mint_supply(&data, "test mint")?;

        assert_eq!(supply, 123_456);

        Ok(())
    }

    #[test]
    fn initialized_mint_returns_decimals() -> Result<(), String> {
        let data = initialized_mint(9);
        let decimals = parse_mint_decimals(&data, "test mint")?;

        assert_eq!(decimals, 9);

        Ok(())
    }

    #[test]
    fn rejects_uninitialized_mint() -> Result<(), String> {
        let mut data = initialized_mint(9);
        data[MINT_INITIALIZED_OFFSET] = 0;

        let error = match parse_mint_decimals(&data, "test mint") {
            Err(error) => error,
            Ok(_) => return Err("uninitialized mint unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "test mint is not initialized");

        Ok(())
    }

    #[test]
    fn rejects_invalid_mint_initialized_value() -> Result<(), String> {
        let mut data = initialized_mint(9);
        data[MINT_INITIALIZED_OFFSET] = 2;

        let error = match parse_mint_decimals(&data, "test mint") {
            Err(error) => error,
            Ok(_) => return Err("invalid initialized value unexpectedly decoded".to_owned()),
        };

        assert_eq!(error, "test mint has invalid is_initialized value: 2");

        Ok(())
    }

    #[test]
    fn subscription_uses_pool_discriminator_without_fixed_size() {
        let request = program_subscribe_request();

        assert_eq!(request["method"], "programSubscribe");
        assert_eq!(request["params"][0], PUMPSWAP_PROGRAM_ID);
        assert_eq!(request["params"][1]["filters"][0]["memcmp"]["offset"], 0);
        assert_eq!(
            request["params"][1]["filters"][0]["memcmp"]["bytes"],
            "hQrXeCntzbV"
        );
        assert!(request["params"][1]["filters"][0].get("dataSize").is_none());
    }

    #[test]
    fn pair_lookup_requests_cover_both_orientations() {
        let requests = pair_lookup_requests("anchor", "intermediate");

        assert_eq!(requests[0]["method"], "getProgramAccounts");
        assert_eq!(requests[0]["params"][0], PUMPSWAP_PROGRAM_ID);
        assert_eq!(
            requests[0]["params"][1]["filters"][1]["memcmp"]["offset"],
            POOL_BASE_MINT_OFFSET
        );
        assert_eq!(
            requests[0]["params"][1]["filters"][1]["memcmp"]["bytes"],
            "anchor"
        );
        assert_eq!(
            requests[0]["params"][1]["filters"][2]["memcmp"]["bytes"],
            "intermediate"
        );
        assert_eq!(
            requests[1]["params"][1]["filters"][1]["memcmp"]["bytes"],
            "intermediate"
        );
        assert_eq!(
            requests[1]["params"][1]["filters"][2]["memcmp"]["bytes"],
            "anchor"
        );
    }

    #[test]
    fn pair_lookup_response_reuses_pool_decoder() -> Result<(), String> {
        let payload = json!({
            "result": {
                "context": {
                    "slot": 42
                },
                "value": [
                    {
                        "pubkey": "pool",
                        "account": rpc_account(PUMPSWAP_PROGRAM_ID, &sample_pool_data())
                    }
                ]
            }
        });

        let observations = parse_pair_lookup_response(&payload)?;

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].pubkey, "pool");
        assert_eq!(observations[0].slot, 42);

        Ok(())
    }

    fn quote_snapshot(
        canonical: bool,
        creator_fee_active: bool,
    ) -> Result<PumpSwapHydrationSnapshot, String> {
        let mut pool_state = decode_pool_state(&sample_pool_data())?;
        let base_mint = Pubkey::from_str(&pool_state.base_mint)
            .map_err(|error| format!("test base mint parse failed: {error}"))?;
        let pump_program = Pubkey::from_str(PUMP_PROGRAM_ID)
            .map_err(|error| format!("test Pump program parse failed: {error}"))?;
        let (pump_pool_authority, _) = Pubkey::try_find_program_address(
            &[b"pool-authority", base_mint.as_ref()],
            &pump_program,
        )
        .ok_or_else(|| "test canonical authority derivation failed".to_owned())?;

        pool_state.creator = if canonical {
            pump_pool_authority.to_string()
        } else {
            Pubkey::new_from_array([42u8; 32]).to_string()
        };
        pool_state.coin_creator = Some(if creator_fee_active {
            Pubkey::new_from_array([43u8; 32]).to_string()
        } else {
            DEFAULT_PUBKEY.to_owned()
        });

        let mut fee_config = decode_fee_config(&sample_fee_config_data())?;
        fee_config.flat_fees = PumpSwapFees {
            lp_fee_bps: 100,
            protocol_fee_bps: 50,
            creator_fee_bps: 25,
        };

        Ok(PumpSwapHydrationSnapshot {
            slot: 901,
            pool_state,
            base_vault_raw: 1_000_000,
            quote_vault_raw: 2_000_000,
            effective_quote_raw: 2_000_000,
            base_mint_supply_raw: 1_000_000,
            base_token_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
            quote_token_program: SPL_TOKEN_PROGRAM_ID.to_owned(),
            base_decimals: 6,
            quote_decimals: 6,
            disable_flags: 0,
            trading_state: PoolTradingState::Tradable,
            fee_config,
        })
    }

    #[test]
    fn canonical_pool_selects_dynamic_market_cap_tier() -> Result<(), String> {
        let snapshot = quote_snapshot(true, true)?;
        let quote = quote_exact_input(&snapshot, &snapshot.pool_state.base_mint, 100_000)?;

        let canonical = is_canonical_pump_pool(&snapshot.pool_state)?;
        let market_cap = pool_market_cap(&snapshot)?;
        let fees = select_pumpswap_fees(&snapshot, canonical, market_cap)?;

        assert!(canonical);
        assert_eq!(market_cap, 2_000_000);
        assert_eq!(fees.lp_fee_bps, 20);
        assert_eq!(fees.protocol_fee_bps, 5);
        assert_eq!(fees.creator_fee_bps, 5);
        assert_eq!(quote.lp_fee_raw, 364);
        assert_eq!(quote.protocol_fee_raw, 91);
        assert_eq!(quote.creator_fee_raw, 91);
        assert_eq!(quote.amount_out_raw, 181_272);

        Ok(())
    }

    #[test]
    fn noncanonical_pool_uses_flat_fees() -> Result<(), String> {
        let snapshot = quote_snapshot(false, true)?;
        let _quote = quote_exact_input(&snapshot, &snapshot.pool_state.base_mint, 100_000)?;

        let canonical = is_canonical_pump_pool(&snapshot.pool_state)?;
        let market_cap = pool_market_cap(&snapshot)?;
        let fees = select_pumpswap_fees(&snapshot, canonical, market_cap)?;

        assert!(!canonical);
        assert_eq!(fees.lp_fee_bps, 100);
        assert_eq!(fees.protocol_fee_bps, 50);
        assert_eq!(fees.creator_fee_bps, 25);

        Ok(())
    }

    #[test]
    fn buy_budget_search_never_exceeds_requested_input() -> Result<(), String> {
        let budget = 1_000_000_000;
        let curve_input = max_curve_input_for_budget(budget, 20, 5, 0)?;
        let lp_fee = fee_bps_ceil(curve_input, 20)?;
        let protocol_fee = fee_bps_ceil(curve_input, 5)?;
        let consumed = curve_input
            .checked_add(lp_fee)
            .and_then(|amount| amount.checked_add(protocol_fee))
            .ok_or_else(|| "test consumed-input overflow".to_owned())?;

        assert_eq!(curve_input, 997_506_233);
        assert_eq!(consumed, budget);

        Ok(())
    }

    #[test]
    fn quote_input_buy_matches_official_integer_rounding() -> Result<(), String> {
        let snapshot = quote_snapshot(true, true)?;
        let quote = quote_exact_input(&snapshot, &snapshot.pool_state.quote_mint, 100_000)?;

        assert_eq!(quote.amount_in_requested_raw, 100_000);
        assert_eq!(quote.amount_in_consumed_raw, 99_998);
        assert_eq!(quote.amount_in_unspent_raw, 2);
        assert_eq!(quote.amount_out_raw, 47_482);
        assert_eq!(quote.lp_fee_raw, 200);
        assert_eq!(quote.protocol_fee_raw, 50);
        assert_eq!(quote.creator_fee_raw, 50);

        Ok(())
    }

    #[test]
    fn default_coin_creator_suppresses_creator_fee() -> Result<(), String> {
        let snapshot = quote_snapshot(true, false)?;
        let quote = quote_exact_input(&snapshot, &snapshot.pool_state.quote_mint, 100_000)?;

        assert_eq!(quote.creator_fee_raw, 0);
        assert_eq!(quote.amount_in_consumed_raw, 99_999);
        assert_eq!(quote.amount_in_unspent_raw, 1);
        assert_eq!(quote.amount_out_raw, 47_505);

        Ok(())
    }

    #[test]
    fn quote_fails_closed_for_token_2022() -> Result<(), String> {
        let mut snapshot = quote_snapshot(true, true)?;
        snapshot.base_token_program = TOKEN_2022_PROGRAM_ID.to_owned();

        let result = quote_exact_input(&snapshot, &snapshot.pool_state.base_mint, 100_000);

        assert!(matches!(result, Err(error) if error.contains("Token-2022")));
        Ok(())
    }

    #[test]
    fn quote_fails_closed_without_coin_creator() -> Result<(), String> {
        let mut snapshot = quote_snapshot(true, true)?;
        snapshot.pool_state.coin_creator = None;

        let result = quote_exact_input(&snapshot, &snapshot.pool_state.base_mint, 100_000);

        assert!(matches!(result, Err(error) if error.contains("coin_creator")));
        Ok(())
    }
}

