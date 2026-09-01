use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const RAYDIUM_CPMM_PROGRAM_ID: &str =
    "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const SPL_TOKEN_PROGRAM_ID: &str =
    "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str =
    "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const FEE_RATE_DENOMINATOR: u64 = 1_000_000;

const POOL_STATE_LEN: usize = 637;
const POOL_STATE_DISCRIMINATOR: [u8; 8] =
    [247, 237, 227, 245, 215, 195, 222, 70];

const AMM_CONFIG_LEN: usize = 236;
const AMM_CONFIG_DISCRIMINATOR: [u8; 8] =
    [218, 244, 33, 104, 203, 203, 43, 111];

const TOKEN_ACCOUNT_BASE_LEN: usize = 165;
const TOKEN_ACCOUNT_MINT_OFFSET: usize = 0;
const TOKEN_ACCOUNT_AMOUNT_OFFSET: usize = 64;
const SWAP_DISABLED_BIT: u8 = 2;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmPoolState {
    pub amm_config: String,
    pub token_0_vault: String,
    pub token_1_vault: String,
    pub token_0_mint: String,
    pub token_1_mint: String,
    pub token_0_program: String,
    pub token_1_program: String,
    pub observation_key: String,
    pub status: u8,
    pub lp_mint_decimals: u8,
    pub mint_0_decimals: u8,
    pub mint_1_decimals: u8,
    pub lp_supply: u64,
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub open_time: u64,
    pub recent_epoch: u64,
    pub creator_fee_on: u8,
    pub enable_creator_fee: bool,
    pub creator_fees_token_0: u64,
    pub creator_fees_token_1: u64,
}

impl RaydiumCpmmPoolState {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "amm_config={} ",
                "mint0={} mint1={} ",
                "vault0={} vault1={} ",
                "token0_program={} token1_program={} ",
                "status={} ",
                "lp_decimals={} mint0_decimals={} mint1_decimals={} ",
                "lp_supply={} ",
                "protocol_fees0={} protocol_fees1={} ",
                "fund_fees0={} fund_fees1={} ",
                "creator_fee_on={} creator_fee_enabled={} ",
                "creator_fees0={} creator_fees1={} ",
                "open_time={} recent_epoch={}"
            ),
            self.amm_config,
            self.token_0_mint,
            self.token_1_mint,
            self.token_0_vault,
            self.token_1_vault,
            self.token_0_program,
            self.token_1_program,
            self.status,
            self.lp_mint_decimals,
            self.mint_0_decimals,
            self.mint_1_decimals,
            self.lp_supply,
            self.protocol_fees_token_0,
            self.protocol_fees_token_1,
            self.fund_fees_token_0,
            self.fund_fees_token_1,
            self.creator_fee_on,
            self.enable_creator_fee,
            self.creator_fees_token_0,
            self.creator_fees_token_1,
            self.open_time,
            self.recent_epoch,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumAmmConfig {
    pub bump: u8,
    pub disable_create_pool: bool,
    pub index: u16,
    pub trade_fee_rate: u64,
    pub protocol_fee_rate: u64,
    pub fund_fee_rate: u64,
    pub create_pool_fee: u64,
    pub protocol_owner: String,
    pub fund_owner: String,
    pub creator_fee_rate: u64,
}

impl RaydiumAmmConfig {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "config_index={} trade_fee_rate={} protocol_fee_rate={} ",
                "fund_fee_rate={} creator_fee_rate={} create_pool_fee={} ",
                "disable_create_pool={}"
            ),
            self.index,
            self.trade_fee_rate,
            self.protocol_fee_rate,
            self.fund_fee_rate,
            self.creator_fee_rate,
            self.create_pool_fee,
            self.disable_create_pool,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmAccountObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub encoded_data_len: usize,
    pub decoded_data_len: usize,
    pub pool_state: RaydiumCpmmPoolState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumHydrationSnapshot {
    pub slot: u64,
    pub pool_state: RaydiumCpmmPoolState,
    pub amm_config: RaydiumAmmConfig,
    pub token_0_vault_raw: u64,
    pub token_1_vault_raw: u64,
    pub token_0_accrued_fees_raw: u64,
    pub token_1_accrued_fees_raw: u64,
    pub token_0_effective_raw: u64,
    pub token_1_effective_raw: u64,
}

impl RaydiumHydrationSnapshot {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "reserve_slot={} ",
                "vault0_raw={} vault1_raw={} ",
                "fees0_raw={} fees1_raw={} ",
                "effective0_raw={} effective1_raw={} ",
                "trade_fee_rate={} protocol_fee_rate={} ",
                "fund_fee_rate={} creator_fee_rate={}"
            ),
            self.slot,
            self.token_0_vault_raw,
            self.token_1_vault_raw,
            self.token_0_accrued_fees_raw,
            self.token_1_accrued_fees_raw,
            self.token_0_effective_raw,
            self.token_1_effective_raw,
            self.amm_config.trade_fee_rate,
            self.amm_config.protocol_fee_rate,
            self.amm_config.fund_fee_rate,
            self.amm_config.creator_fee_rate,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumExactInputQuote {
    pub amount_in_raw: u64,
    pub amount_out_raw: u64,
    pub trade_fee_raw: u64,
    pub protocol_fee_raw: u64,
    pub fund_fee_raw: u64,
    pub creator_fee_raw: u64,
    pub source_slot: u64,
}

pub fn quote_exact_input(
    snapshot: &RaydiumHydrationSnapshot,
    input_mint: &str,
    amount_in_raw: u64,
) -> Result<RaydiumExactInputQuote, String> {
    if amount_in_raw == 0 {
        return Err("Raydium quote input must be greater than zero".to_owned());
    }

    ensure_legacy_token_program(
        &snapshot.pool_state.token_0_program,
        "Raydium token_0",
    )?;
    ensure_legacy_token_program(
        &snapshot.pool_state.token_1_program,
        "Raydium token_1",
    )?;

    let zero_for_one = if input_mint == snapshot.pool_state.token_0_mint {
        true
    } else if input_mint == snapshot.pool_state.token_1_mint {
        false
    } else {
        return Err(format!(
            "Raydium quote input mint {input_mint} is not in pool"
        ));
    };

    let (input_reserve, output_reserve) = if zero_for_one {
        (
            snapshot.token_0_effective_raw,
            snapshot.token_1_effective_raw,
        )
    } else {
        (
            snapshot.token_1_effective_raw,
            snapshot.token_0_effective_raw,
        )
    };

    if input_reserve == 0 || output_reserve == 0 {
        return Err(
            "Raydium quote reserves must be greater than zero".to_owned()
        );
    }

    validate_fee_rate(
        snapshot.amm_config.trade_fee_rate,
        "trade_fee_rate",
    )?;
    validate_fee_rate(
        snapshot.amm_config.protocol_fee_rate,
        "protocol_fee_rate",
    )?;
    validate_fee_rate(
        snapshot.amm_config.fund_fee_rate,
        "fund_fee_rate",
    )?;
    validate_fee_rate(
        snapshot.amm_config.creator_fee_rate,
        "creator_fee_rate",
    )?;

    let trade_fee_share_rate = snapshot
        .amm_config
        .protocol_fee_rate
        .checked_add(snapshot.amm_config.fund_fee_rate)
        .ok_or_else(|| {
            "Raydium protocol/fund fee-share rate overflow".to_owned()
        })?;

    if trade_fee_share_rate > FEE_RATE_DENOMINATOR {
        return Err(format!(
            "Raydium protocol/fund fee-share total exceeds \
             {FEE_RATE_DENOMINATOR}: {trade_fee_share_rate}"
        ));
    }

    let creator_fee_rate = if snapshot.pool_state.enable_creator_fee {
        snapshot.amm_config.creator_fee_rate
    } else {
        0
    };

    let creator_fee_on_input = creator_fee_on_input(
        snapshot.pool_state.creator_fee_on,
        zero_for_one,
    )?;

    let (trade_fee_raw, creator_fee_raw, curve_input_raw) =
        if creator_fee_on_input {
            let combined_rate = snapshot
                .amm_config
                .trade_fee_rate
                .checked_add(creator_fee_rate)
                .ok_or_else(|| {
                    "Raydium combined input fee rate overflow".to_owned()
                })?;

            if combined_rate >= FEE_RATE_DENOMINATOR {
                return Err(format!(
                    "Raydium combined input fee rate must be below \
                     {FEE_RATE_DENOMINATOR}"
                ));
            }

            let total_fee = fee_ceil(
                amount_in_raw,
                combined_rate,
                FEE_RATE_DENOMINATOR,
            )?;

            let creator_fee = if combined_rate == 0 {
                0
            } else {
                fee_floor(
                    total_fee,
                    creator_fee_rate,
                    combined_rate,
                )?
            };

            let trade_fee = total_fee
                .checked_sub(creator_fee)
                .ok_or_else(|| {
                    "Raydium trade-fee split underflow".to_owned()
                })?;

            let curve_input = amount_in_raw
                .checked_sub(total_fee)
                .ok_or_else(|| {
                    "Raydium input fee exceeds quote input".to_owned()
                })?;

            (trade_fee, creator_fee, curve_input)
        } else {
            let trade_fee = fee_ceil(
                amount_in_raw,
                snapshot.amm_config.trade_fee_rate,
                FEE_RATE_DENOMINATOR,
            )?;

            let curve_input = amount_in_raw
                .checked_sub(trade_fee)
                .ok_or_else(|| {
                    "Raydium trade fee exceeds quote input".to_owned()
                })?;

            (trade_fee, 0, curve_input)
        };

    if curve_input_raw == 0 {
        return Err(
            "Raydium quote input is fully consumed by fees".to_owned()
        );
    }

    let protocol_fee_raw = fee_floor(
        trade_fee_raw,
        snapshot.amm_config.protocol_fee_rate,
        FEE_RATE_DENOMINATOR,
    )?;

    let fund_fee_raw = fee_floor(
        trade_fee_raw,
        snapshot.amm_config.fund_fee_rate,
        FEE_RATE_DENOMINATOR,
    )?;

    let curve_output_raw = constant_product_output(
        curve_input_raw,
        input_reserve,
        output_reserve,
    )?;

    let (amount_out_raw, creator_fee_raw) =
        if creator_fee_on_input {
            (curve_output_raw, creator_fee_raw)
        } else {
            let creator_fee = fee_ceil(
                curve_output_raw,
                creator_fee_rate,
                FEE_RATE_DENOMINATOR,
            )?;

            let amount_out = curve_output_raw
                .checked_sub(creator_fee)
                .ok_or_else(|| {
                    "Raydium creator fee exceeds quote output".to_owned()
                })?;

            (amount_out, creator_fee)
        };

    if amount_out_raw == 0 {
        return Err("Raydium quote output rounded to zero".to_owned());
    }

    Ok(RaydiumExactInputQuote {
        amount_in_raw,
        amount_out_raw,
        trade_fee_raw,
        protocol_fee_raw,
        fund_fee_raw,
        creator_fee_raw,
        source_slot: snapshot.slot,
    })
}

fn ensure_legacy_token_program(
    program: &str,
    label: &str,
) -> Result<(), String> {
    if program == SPL_TOKEN_PROGRAM_ID {
        return Ok(());
    }

    if program == TOKEN_2022_PROGRAM_ID {
        return Err(format!(
            "{label} uses Token-2022; transfer-fee extension state is \
             not hydrated, so quoting fails closed"
        ));
    }

    Err(format!(
        "{label} uses unsupported token program {program}"
    ))
}

fn creator_fee_on_input(
    creator_fee_on: u8,
    zero_for_one: bool,
) -> Result<bool, String> {
    match creator_fee_on {
        0 => Ok(true),
        1 => Ok(zero_for_one),
        2 => Ok(!zero_for_one),
        _ => Err(format!(
            "invalid Raydium creator_fee_on value {creator_fee_on}"
        )),
    }
}

fn validate_fee_rate(rate: u64, label: &str) -> Result<(), String> {
    if rate > FEE_RATE_DENOMINATOR {
        return Err(format!(
            "Raydium {label} exceeds denominator \
             {FEE_RATE_DENOMINATOR}: {rate}"
        ));
    }

    Ok(())
}

fn fee_ceil(
    amount: u64,
    rate: u64,
    denominator: u64,
) -> Result<u64, String> {
    if rate == 0 || amount == 0 {
        return Ok(0);
    }

    let numerator = u128::from(amount)
        .checked_mul(u128::from(rate))
        .ok_or_else(|| {
            "Raydium fee multiplication overflow".to_owned()
        })?;

    let value =
        ceil_div_u128(numerator, u128::from(denominator))?;

    u64::try_from(value)
        .map_err(|_| "Raydium fee exceeded u64".to_owned())
}

fn fee_floor(
    amount: u64,
    rate: u64,
    denominator: u64,
) -> Result<u64, String> {
    if denominator == 0 {
        return Err(
            "Raydium fee denominator cannot be zero".to_owned()
        );
    }

    let numerator = u128::from(amount)
        .checked_mul(u128::from(rate))
        .ok_or_else(|| {
            "Raydium fee multiplication overflow".to_owned()
        })?;

    let value = numerator / u128::from(denominator);

    u64::try_from(value)
        .map_err(|_| "Raydium fee exceeded u64".to_owned())
}

fn constant_product_output(
    amount_in_raw: u64,
    input_reserve_raw: u64,
    output_reserve_raw: u64,
) -> Result<u64, String> {
    let denominator = u128::from(input_reserve_raw)
        .checked_add(u128::from(amount_in_raw))
        .ok_or_else(|| {
            "Raydium constant-product denominator overflow".to_owned()
        })?;

    if denominator == 0 {
        return Err(
            "Raydium constant-product denominator cannot be zero"
                .to_owned(),
        );
    }

    let numerator = u128::from(amount_in_raw)
        .checked_mul(u128::from(output_reserve_raw))
        .ok_or_else(|| {
            "Raydium constant-product multiplication overflow"
                .to_owned()
        })?;

    let output = numerator / denominator;

    u64::try_from(output)
        .map_err(|_| "Raydium quote output exceeded u64".to_owned())
}

fn ceil_div_u128(
    numerator: u128,
    denominator: u128,
) -> Result<u128, String> {
    if denominator == 0 {
        return Err("Raydium division by zero".to_owned());
    }

    if numerator == 0 {
        return Ok(0);
    }

    numerator
        .checked_sub(1)
        .and_then(|value| value.checked_div(denominator))
        .and_then(|value| value.checked_add(1))
        .ok_or_else(|| {
            "Raydium ceiling division overflow".to_owned()
        })
}

pub fn program_subscribe_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "programSubscribe",
        "params": [
            RAYDIUM_CPMM_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": POOL_STATE_LEN
                    }
                ]
            }
        ]
    })
}

pub fn hydration_account_pubkeys(
    observation: &RaydiumCpmmAccountObservation,
) -> [String; 4] {
    [
        observation.pubkey.clone(),
        observation.pool_state.amm_config.clone(),
        observation.pool_state.token_0_vault.clone(),
        observation.pool_state.token_1_vault.clone(),
    ]
}

pub fn parse_program_notification(
    payload: &Value,
) -> Result<Option<RaydiumCpmmAccountObservation>, String> {
    if payload.get("method").and_then(Value::as_str)
        != Some("programNotification")
    {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "Raydium notification missing slot".to_owned()
        })?;

    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Raydium notification missing pubkey".to_owned()
        })?
        .to_owned();

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Raydium notification missing owner".to_owned()
        })?
        .to_owned();

    if owner != RAYDIUM_CPMM_PROGRAM_ID {
        return Err(format!(
            "unexpected Raydium account owner: {owner}"
        ));
    }

    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Raydium notification missing base64 account data"
                .to_owned()
        })?;

    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| {
            "Raydium notification missing account-data encoding"
                .to_owned()
        })?;

    if encoding != "base64" {
        return Err(format!(
            "unexpected Raydium account-data encoding: {encoding}"
        ));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| {
            format!(
                "invalid Raydium base64 account data: {error}"
            )
        })?;

    let pool_state = decode_pool_state(&decoded_data)?;

    Ok(Some(RaydiumCpmmAccountObservation {
        pubkey,
        slot,
        owner,
        encoded_data_len: encoded_data.len(),
        decoded_data_len: decoded_data.len(),
        pool_state,
    }))
}

pub fn parse_hydration_response(
    observation: &RaydiumCpmmAccountObservation,
    payload: &Value,
) -> Result<RaydiumHydrationSnapshot, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Solana getMultipleAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "Solana getMultipleAccounts response missing context slot"
                .to_owned()
        })?;

    if slot < observation.slot {
        return Err(format!(
            "stale Raydium hydration snapshot: \
             trigger_slot={} reserve_slot={slot}",
            observation.slot
        ));
    }

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Solana getMultipleAccounts response missing account array"
                .to_owned()
        })?;

    if accounts.len() != 4 {
        return Err(format!(
            "Raydium hydration expected exactly 4 accounts, got {}",
            accounts.len()
        ));
    }

    if accounts.iter().any(Value::is_null) {
        return Err(
            "Raydium hydration response contained a missing account"
                .to_owned(),
        );
    }

    let pool_data = decode_rpc_account_data(
        &accounts[0],
        RAYDIUM_CPMM_PROGRAM_ID,
        "Raydium pool snapshot",
    )?;

    let pool_state = decode_pool_state(&pool_data)?;

    verify_pool_identity(
        &observation.pool_state,
        &pool_state,
    )?;

    let amm_config_data = decode_rpc_account_data(
        &accounts[1],
        RAYDIUM_CPMM_PROGRAM_ID,
        "Raydium AmmConfig snapshot",
    )?;

    let amm_config = decode_amm_config(&amm_config_data)?;

    let token_0_vault_raw = parse_token_
