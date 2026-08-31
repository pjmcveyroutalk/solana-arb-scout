use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const POOL_STATE_LEN: usize = 637;
const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

const AMM_CONFIG_LEN: usize = 236;
const AMM_CONFIG_DISCRIMINATOR: [u8; 8] = [218, 244, 33, 104, 203, 203, 43, 111];

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

pub fn hydration_account_pubkeys(observation: &RaydiumCpmmAccountObservation) -> [String; 4] {
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
    if payload.get("method").and_then(Value::as_str) != Some("programNotification") {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Raydium notification missing slot".to_owned())?;

    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing pubkey".to_owned())?
        .to_owned();

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing owner".to_owned())?
        .to_owned();

    if owner != RAYDIUM_CPMM_PROGRAM_ID {
        return Err(format!("unexpected Raydium account owner: {owner}"));
    }

    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing base64 account data".to_owned())?;

    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing account-data encoding".to_owned())?;

    if encoding != "base64" {
        return Err(format!(
            "unexpected Raydium account-data encoding: {encoding}"
        ));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Raydium base64 account data: {error}"))?;

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
        .ok_or_else(|| "Solana getMultipleAccounts response missing context slot".to_owned())?;

    if slot < observation.slot {
        return Err(format!(
            "stale Raydium hydration snapshot: trigger_slot={} reserve_slot={slot}",
            observation.slot
        ));
    }

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Solana getMultipleAccounts response missing account array".to_owned())?;

    if accounts.len() != 4 {
        return Err(format!(
            "Raydium hydration expected exactly 4 accounts, got {}",
            accounts.len()
        ));
    }

    if accounts.iter().any(Value::is_null) {
        return Err("Raydium hydration response contained a missing account".to_owned());
    }

    let pool_data = decode_rpc_account_data(
        &accounts[0],
        RAYDIUM_CPMM_PROGRAM_ID,
        "Raydium pool snapshot",
    )?;
    let pool_state = decode_pool_state(&pool_data)?;

    verify_pool_identity(&observation.pool_state, &pool_state)?;

    let amm_config_data = decode_rpc_account_data(
        &accounts[1],
        RAYDIUM_CPMM_PROGRAM_ID,
        "Raydium AmmConfig snapshot",
    )?;
    let amm_config = decode_amm_config(&amm_config_data)?;

    let token_0_vault_raw = parse_token_vault_account(
        &accounts[2],
        &pool_state.token_0_program,
        &pool_state.token_0_mint,
        "token_0_vault",
    )?;

    let token_1_vault_raw = parse_token_vault_account(
        &accounts[3],
        &pool_state.token_1_program,
        &pool_state.token_1_mint,
        "token_1_vault",
    )?;

    let token_0_accrued_fees_raw = checked_accrued_fees(
        pool_state.protocol_fees_token_0,
        pool_state.fund_fees_token_0,
        pool_state.creator_fees_token_0,
        "token_0",
    )?;

    let token_1_accrued_fees_raw = checked_accrued_fees(
        pool_state.protocol_fees_token_1,
        pool_state.fund_fees_token_1,
        pool_state.creator_fees_token_1,
        "token_1",
    )?;

    let token_0_effective_raw = token_0_vault_raw
        .checked_sub(token_0_accrued_fees_raw)
        .ok_or_else(|| {
            format!(
                "Raydium token_0 effective reserve underflow: vault={} accrued_fees={}",
                token_0_vault_raw, token_0_accrued_fees_raw
            )
        })?;

    let token_1_effective_raw = token_1_vault_raw
        .checked_sub(token_1_accrued_fees_raw)
        .ok_or_else(|| {
            format!(
                "Raydium token_1 effective reserve underflow: vault={} accrued_fees={}",
                token_1_vault_raw, token_1_accrued_fees_raw
            )
        })?;

    Ok(RaydiumHydrationSnapshot {
        slot,
        pool_state,
        amm_config,
        token_0_vault_raw,
        token_1_vault_raw,
        token_0_accrued_fees_raw,
        token_1_accrued_fees_raw,
        token_0_effective_raw,
        token_1_effective_raw,
    })
}

pub fn hydrate_normalized_observation(
    observation: &RaydiumCpmmAccountObservation,
    snapshot: &RaydiumHydrationSnapshot,
    account_update_received_at_unix_ms: u64,
    hydrated_at_unix_ms: u64,
) -> Result<NormalizedPoolState, String> {
    if snapshot.slot < observation.slot {
        return Err(format!(
            "stale Raydium hydration snapshot: trigger_slot={} reserve_slot={}",
            observation.slot, snapshot.slot
        ));
    }

    verify_pool_identity(&observation.pool_state, &snapshot.pool_state)?;

    let mut normalized = normalize_observation(
        observation,
        account_update_received_at_unix_ms,
        hydrated_at_unix_ms,
    );

    normalized.trading_state = trading_state(
        snapshot.pool_state.status,
        snapshot.pool_state.open_time,
        hydrated_at_unix_ms,
    );

    normalized.quote_reserves = QuoteReserveState::Available {
        token_a_raw: snapshot.token_0_effective_raw,
        token_b_raw: snapshot.token_1_effective_raw,
        source_slot: snapshot.slot,
    };

    Ok(normalized)
}

pub fn normalize_observation(
    observation: &RaydiumCpmmAccountObservation,
    account_update_received_at_unix_ms: u64,
    normalized_at_unix_ms: u64,
) -> NormalizedPoolState {
    let trading_state = trading_state(
        observation.pool_state.status,
        observation.pool_state.open_time,
        account_update_received_at_unix_ms,
    );

    NormalizedPoolState {
        pool_id: observation.pubkey.clone(),
        venue: Venue::RaydiumCpmm,
        program_id: observation.owner.clone(),
        source_slot: observation.slot,
        token_a: NormalizedToken {
            mint: observation.pool_state.token_0_mint.clone(),
            vault: observation.pool_state.token_0_vault.clone(),
            decimals: observation.pool_state.mint_0_decimals,
        },
        token_b: NormalizedToken {
            mint: observation.pool_state.token_1_mint.clone(),
            vault: observation.pool_state.token_1_vault.clone(),
            decimals: observation.pool_state.mint_1_decimals,
        },
        trading_state,
        quote_reserves: QuoteReserveState::Unavailable,
        account_update_received_at_unix_ms,
        normalized_at_unix_ms,
    }
}

fn verify_pool_identity(
    trigger: &RaydiumCpmmPoolState,
    snapshot: &RaydiumCpmmPoolState,
) -> Result<(), String> {
    if trigger.amm_config != snapshot.amm_config {
        return Err("Raydium hydration pool amm_config changed".to_owned());
    }

    if trigger.token_0_vault != snapshot.token_0_vault {
        return Err("Raydium hydration token_0_vault changed".to_owned());
    }

    if trigger.token_1_vault != snapshot.token_1_vault {
        return Err("Raydium hydration token_1_vault changed".to_owned());
    }

    if trigger.token_0_mint != snapshot.token_0_mint {
        return Err("Raydium hydration token_0_mint changed".to_owned());
    }

    if trigger.token_1_mint != snapshot.token_1_mint {
        return Err("Raydium hydration token_1_mint changed".to_owned());
    }

    if trigger.token_0_program != snapshot.token_0_program {
        return Err("Raydium hydration token_0_program changed".to_owned());
    }

    if trigger.token_1_program != snapshot.token_1_program {
        return Err("Raydium hydration token_1_program changed".to_owned());
    }

    if trigger.mint_0_decimals != snapshot.mint_0_decimals {
        return Err("Raydium hydration mint_0_decimals changed".to_owned());
    }

    if trigger.mint_1_decimals != snapshot.mint_1_decimals {
        return Err("Raydium hydration mint_1_decimals changed".to_owned());
    }

    Ok(())
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

    let executable = account
        .get("executable")
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("{label} missing executable flag"))?;

    if executable {
        return Err(format!("{label} unexpectedly executable"));
    }

    let encoded_data = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing base64 account data"))?;

    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing account-data encoding"))?;

    if encoding != "base64" {
        return Err(format!(
            "{label} had unexpected account-data encoding: {encoding}"
        ));
    }

    BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("{label} contained invalid base64 data: {error}"))
}

fn parse_token_vault_account(
    account: &Value,
    expected_program: &str,
    expected_mint: &str,
    label: &str,
) -> Result<u64, String> {
    let data = decode_rpc_account_data(account, expected_program, label)?;

    if data.len() < TOKEN_ACCOUNT_BASE_LEN {
        return Err(format!(
            "{label} account data too short: expected at least {TOKEN_ACCOUNT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    let mint_end = TOKEN_ACCOUNT_MINT_OFFSET + 32;
    let mint_bytes = data
        .get(TOKEN_ACCOUNT_MINT_OFFSET..mint_end)
        .ok_or_else(|| format!("{label} missing token mint bytes"))?;
    let mint = bs58::encode(mint_bytes).into_string();

    if mint != expected_mint {
        return Err(format!(
            "{label} mint mismatch: expected {expected_mint}, got {mint}"
        ));
    }

    let amount_end = TOKEN_ACCOUNT_AMOUNT_OFFSET + 8;
    let amount_bytes = data
        .get(TOKEN_ACCOUNT_AMOUNT_OFFSET..amount_end)
        .ok_or_else(|| format!("{label} missing token amount bytes"))?;
    let amount_array = <[u8; 8]>::try_from(amount_bytes)
        .map_err(|_| format!("{label} token amount had unexpected size"))?;

    Ok(u64::from_le_bytes(amount_array))
}

fn checked_accrued_fees(
    protocol_fees: u64,
    fund_fees: u64,
    creator_fees: u64,
    label: &str,
) -> Result<u64, String> {
    protocol_fees
        .checked_add(fund_fees)
        .and_then(|fees| fees.checked_add(creator_fees))
        .ok_or_else(|| format!("Raydium {label} accrued-fee overflow"))
}

fn trading_state(
    status: u8,
    open_time_unix_seconds: u64,
    observed_at_unix_ms: u64,
) -> PoolTradingState {
    let swap_disabled_mask = 1u8 << SWAP_DISABLED_BIT;

    if status & swap_disabled_mask != 0 {
        return PoolTradingState::SwapDisabled;
    }

    let observed_at_unix_seconds = observed_at_unix_ms / 1_000;

    if open_time_unix_seconds > observed_at_unix_seconds {
        return PoolTradingState::NotYetOpen;
    }

    PoolTradingState::Tradable
}

fn decode_amm_config(data: &[u8]) -> Result<RaydiumAmmConfig, String> {
    if data.len() != AMM_CONFIG_LEN {
        return Err(format!(
            "unexpected Raydium AmmConfig length: expected {AMM_CONFIG_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = data
        .get(0..AMM_CONFIG_DISCRIMINATOR.len())
        .ok_or_else(|| "Raydium AmmConfig missing discriminator".to_owned())?;

    if discriminator != AMM_CONFIG_DISCRIMINATOR {
        return Err("unexpected Raydium AmmConfig discriminator".to_owned());
    }

    let mut offset = AMM_CONFIG_DISCRIMINATOR.len();

    let bump = read_u8(data, &mut offset)?;
    let disable_create_pool_raw = read_u8(data, &mut offset)?;
    let disable_create_pool = match disable_create_pool_raw {
        0 => false,
        1 => true,
        other => {
            return Err(format!(
                "invalid Raydium disable_create_pool boolean value: {other}"
            ));
        }
    };

    let index = read_u16(data, &mut offset)?;
    let trade_fee_rate = read_u64(data, &mut offset)?;
    let protocol_fee_rate = read_u64(data, &mut offset)?;
    let fund_fee_rate = read_u64(data, &mut offset)?;
    let create_pool_fee = read_u64(data, &mut offset)?;
    let protocol_owner = read_pubkey(data, &mut offset)?;
    let fund_owner = read_pubkey(data, &mut offset)?;
    let creator_fee_rate = read_u64(data, &mut offset)?;

    skip(data, &mut offset, 15 * 8)?;

    if offset != data.len() {
        return Err(format!(
            "Raydium AmmConfig decoder ended at {offset}, account length is {}
