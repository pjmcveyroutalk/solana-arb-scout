use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const POOL_STATE_LEN: usize = 637;
const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];
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
                "effective0_raw={} effective1_raw={}"
            ),
            self.slot,
            self.token_0_vault_raw,
            self.token_1_vault_raw,
            self.token_0_accrued_fees_raw,
            self.token_1_accrued_fees_raw,
            self.token_0_effective_raw,
            self.token_1_effective_raw,
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

pub fn hydration_account_pubkeys(observation: &RaydiumCpmmAccountObservation) -> [String; 3] {
    [
        observation.pubkey.clone(),
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

    if accounts.len() != 3 {
        return Err(format!(
            "Raydium hydration expected exactly 3 accounts, got {}",
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

    let token_0_vault_raw = parse_token_vault_account(
        &accounts[1],
        &pool_state.token_0_program,
        &pool_state.token_0_mint,
        "token_0_vault",
    )?;

    let token_1_vault_raw = parse_token_vault_account(
        &accounts[2],
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

fn decode_pool_state(data: &[u8]) -> Result<RaydiumCpmmPoolState, String> {
    if data.len() != POOL_STATE_LEN {
        return Err(format!(
            "unexpected Raydium PoolState length: expected {POOL_STATE_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = data
        .get(0..POOL_STATE_DISCRIMINATOR.len())
        .ok_or_else(|| "Raydium PoolState missing discriminator".to_owned())?;

    if discriminator != POOL_STATE_DISCRIMINATOR {
        return Err("unexpected Raydium PoolState discriminator".to_owned());
    }

    let mut offset = POOL_STATE_DISCRIMINATOR.len();

    let amm_config = read_pubkey(data, &mut offset)?;
    let _pool_creator = read_pubkey(data, &mut offset)?;
    let token_0_vault = read_pubkey(data, &mut offset)?;
    let token_1_vault = read_pubkey(data, &mut offset)?;
    let _lp_mint = read_pubkey(data, &mut offset)?;
    let token_0_mint = read_pubkey(data, &mut offset)?;
    let token_1_mint = read_pubkey(data, &mut offset)?;
    let token_0_program = read_pubkey(data, &mut offset)?;
    let token_1_program = read_pubkey(data, &mut offset)?;
    let _observation_key = read_pubkey(data, &mut offset)?;

    let _auth_bump = read_u8(data, &mut offset)?;
    let status = read_u8(data, &mut offset)?;
    let lp_mint_decimals = read_u8(data, &mut offset)?;
    let mint_0_decimals = read_u8(data, &mut offset)?;
    let mint_1_decimals = read_u8(data, &mut offset)?;

    let lp_supply = read_u64(data, &mut offset)?;
    let protocol_fees_token_0 = read_u64(data, &mut offset)?;
    let protocol_fees_token_1 = read_u64(data, &mut offset)?;
    let fund_fees_token_0 = read_u64(data, &mut offset)?;
    let fund_fees_token_1 = read_u64(data, &mut offset)?;
    let open_time = read_u64(data, &mut offset)?;
    let recent_epoch = read_u64(data, &mut offset)?;

    let creator_fee_on = read_u8(data, &mut offset)?;
    let enable_creator_fee_raw = read_u8(data, &mut offset)?;

    let enable_creator_fee = match enable_creator_fee_raw {
        0 => false,
        1 => true,
        other => {
            return Err(format!(
                "invalid Raydium creator-fee boolean value: {other}"
            ));
        }
    };

    skip(data, &mut offset, 6)?;

    let creator_fees_token_0 = read_u64(data, &mut offset)?;
    let creator_fees_token_1 = read_u64(data, &mut offset)?;

    skip(data, &mut offset, 28 * 8)?;

    if offset != data.len() {
        return Err(format!(
            "Raydium PoolState decoder ended at {offset}, account length is {}",
            data.len()
        ));
    }

    Ok(RaydiumCpmmPoolState {
        amm_config,
        token_0_vault,
        token_1_vault,
        token_0_mint,
        token_1_mint,
        token_0_program,
        token_1_program,
        status,
        lp_mint_decimals,
        mint_0_decimals,
        mint_1_decimals,
        lp_supply,
        protocol_fees_token_0,
        protocol_fees_token_1,
        fund_fees_token_0,
        fund_fees_token_1,
        open_time,
        recent_epoch,
        creator_fee_on,
        enable_creator_fee,
        creator_fees_token_0,
        creator_fees_token_1,
    })
}

fn read_pubkey(data: &[u8], offset: &mut usize) -> Result<String, String> {
    let bytes = take::<32>(data, offset)?;
    Ok(bs58::encode(bytes).into_string())
}

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, String> {
    let bytes = take::<1>(data, offset)?;
    Ok(bytes[0])
}

fn read_u64(data: &[u8], offset: &mut usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(take::<8>(data, offset)?))
}

fn skip(data: &[u8], offset: &mut usize, len: usize) -> Result<(), String> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| "Raydium PoolState offset overflow".to_owned())?;

    if end > data.len() {
        return Err("Raydium PoolState ended unexpectedly".to_owned());
    }

    *offset = end;
    Ok(())
}

fn take<const N: usize>(data: &[u8], offset: &mut usize) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| "Raydium PoolState offset overflow".to_owned())?;

    let slice = data
        .get(*offset..end)
        .ok_or_else(|| "Raydium PoolState ended unexpectedly".to_owned())?;

    let bytes = <[u8; N]>::try_from(slice)
        .map_err(|_| "Raydium PoolState field had unexpected size".to_owned())?;

    *offset = end;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_targets_raydium_cpmm_pool_state_size() {
        let request = program_subscribe_request();

        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(RAYDIUM_CPMM_PROGRAM_ID)
        );
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("programSubscribe")
        );
        assert_eq!(
            request
                .pointer("/params/1/filters/0/dataSize")
                .and_then(Value::as_u64),
            Some(POOL_STATE_LEN as u64)
        );
    }

    #[test]
    fn ignores_non_program_notifications() -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "slotNotification"
        });

        assert_eq!(parse_program_notification(&payload)?, None);

        Ok(())
    }

    #[test]
    fn decodes_deterministic_pool_state_fixture() -> Result<(), String> {
        let data = fixture_pool_state(0, 1_234_567);
        let state = decode_pool_state(&data)?;

        assert_eq!(data.len(), POOL_STATE_LEN);
        assert_eq!(state.amm_config, bs58::encode([1u8; 32]).into_string());
        assert_eq!(state.token_0_vault, bs58::encode([3u8; 32]).into_string());
        assert_eq!(state.token_1_vault, bs58::encode([4u8; 32]).into_string());
        assert_eq!(state.token_0_mint, bs58::encode([6u8; 32]).into_string());
        assert_eq!(state.token_1_mint, bs58::encode([7u8; 32]).into_string());
        assert_eq!(state.token_0_program, bs58::encode([8u8; 32]).into_string());
        assert_eq!(state.token_1_program, bs58::encode([9u8; 32]).into_string());
        assert_eq!(state.status, 0);
        assert_eq!(state.lp_mint_decimals, 9);
        assert_eq!(state.mint_0_decimals, 6);
        assert_eq!(state.mint_1_decimals, 6);
        assert_eq!(state.lp_supply, 1_000);
        assert_eq!(state.protocol_fees_token_0, 10);
        assert_eq!(state.protocol_fees_token_1, 11);
        assert_eq!(state.fund_fees_token_0, 12);
        assert_eq!(state.fund_fees_token_1, 13);
        assert_eq!(state.open_time, 1_234_567);
        assert_eq!(state.recent_epoch, 500);
        assert_eq!(state.creator_fee_on, 0);
        assert!(state.enable_creator_fee);
        assert_eq!(state.creator_fees_token_0, 14);
        assert_eq!(state.creator_fees_token_1, 15);

        Ok(())
    }

    #[test]
    fn rejects_wrong_pool_state_discriminator() {
        let mut data = fixture_pool_state(0, 1_234_567);
        data[0] ^= 0xff;

        assert!(decode_pool_state(&data).is_err());
    }

    #[test]
    fn rejects_wrong_pool_state_length() {
        let mut data = fixture_pool_state(0, 1_234_567);
        let _ = data.pop();

        assert!(decode_pool_state(&data).is_err());
    }

    #[test]
    fn parses_and_decodes_read_only_raydium_observation() -> Result<(), String> {
        let data = fixture_pool_state(0, 1_234_567);
        let encoded_data = BASE64_STANDARD.encode(&data);

        let payload = json!({
            "jsonrpc": "2.0",
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": 123456
                    },
                    "value": {
                        "pubkey": "ExamplePool111111111111111111111111111111111",
                        "account": {
                            "data": [
                                encoded_data,
                                "base64"
                            ],
                            "executable": false,
                            "lamports": 1,
                            "owner": RAYDIUM_CPMM_PROGRAM_ID,
                            "rentEpoch": 0,
                            "space": POOL_STATE_LEN
                        }
                    }
                },
                "subscription": 99
            }
        });

        let observation = parse_program_notification(&payload)?
            .ok_or_else(|| "expected Raydium observation".to_owned())?;

        assert_eq!(observation.slot, 123456);
        assert_eq!(observation.owner, RAYDIUM_CPMM_PROGRAM_ID);
        assert_eq!(observation.decoded_data_len, POOL_STATE_LEN);
        assert_eq!(observation.pool_state.lp_supply, 1_000);
        assert_eq!(
            observation.pool_state.token_0_program,
            bs58::encode([8u8; 32]).into_string()
        );
        assert_eq!(
            observation.pool_state.token_1_program,
            bs58::encode([9u8; 32]).into_string()
        );

        Ok(())
    }

    #[test]
    fn normalizes_raydium_pool_without_inventing_reserves() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;

        let normalized = normalize_observation(&observation, 1_500_000_000, 1_500_000_001);

        assert_eq!(normalized.pool_id, observation.pubkey);
        assert_eq!(normalized.venue, Venue::RaydiumCpmm);
        assert_eq!(normalized.program_id, RAYDIUM_CPMM_PROGRAM_ID);
        assert_eq!(normalized.source_slot, 123_456);
        assert_eq!(normalized.token_a.mint, observation.pool_state.token_0_mint);
        assert_eq!(normalized.token_b.mint, observation.pool_state.token_1_mint);
        assert_eq!(
            normalized.token_a.vault,
            observation.pool_state.token_0_vault
        );
        assert_eq!(
            normalized.token_b.vault,
            observation.pool_state.token_1_vault
        );
        assert_eq!(normalized.token_a.decimals, 6);
        assert_eq!(normalized.token_b.decimals, 6);
        assert_eq!(normalized.trading_state, PoolTradingState::Tradable);
        assert_eq!(normalized.quote_reserves, QuoteReserveState::Unavailable);
        assert_eq!(normalized.account_update_received_at_unix_ms, 1_500_000_000);
        assert_eq!(normalized.normalized_at_unix_ms, 1_500_000_001);

        Ok(())
    }

    #[test]
    fn normalization_marks_future_open_time() -> Result<(), String> {
        let observation = fixture_observation(0, 2_000_000)?;

        let normalized = normalize_observation(&observation, 1_500_000_000, 1_500_000_001);

        assert_eq!(normalized.trading_state, PoolTradingState::NotYetOpen);

        Ok(())
    }

    #[test]
    fn normalization_marks_swap_disabled_status() -> Result<(), String> {
        let swap_disabled_status = 1u8 << SWAP_DISABLED_BIT;
        let observation = fixture_observation(swap_disabled_status, 1_234_567)?;

        let normalized = normalize_observation(&observation, 1_500_000_000, 1_500_000_001);

        assert_eq!(normalized.trading_state, PoolTradingState::SwapDisabled);

        Ok(())
    }

    #[test]
    fn hydration_account_order_is_pool_then_two_vaults() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;

        let account_pubkeys = hydration_account_pubkeys(&observation);

        assert_eq!(account_pubkeys[0], observation.pubkey);
        assert_eq!(account_pubkeys[1], observation.pool_state.token_0_vault);
        assert_eq!(account_pubkeys[2], observation.pool_state.token_1_vault);

        Ok(())
    }

    #[test]
    fn valid_hydration_snapshot_produces_effective_reserves() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let payload = fixture_hydration_payload(
            123_500,
            &observation.pool_state.token_0_program,
            6,
            10_000,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        let snapshot = parse_hydration_response(&observation, &payload)?;

        assert_eq!(snapshot.slot, 123_500);
        assert_eq!(snapshot.token_0_vault_raw, 10_000);
        assert_eq!(snapshot.token_1_vault_raw, 20_000);
        assert_eq!(snapshot.token_0_accrued_fees_raw, 36);
        assert_eq!(snapshot.token_1_accrued_fees_raw, 39);
        assert_eq!(snapshot.token_0_effective_raw, 9_964);
        assert_eq!(snapshot.token_1_effective_raw, 19_961);

        let normalized =
            hydrate_normalized_observation(&observation, &snapshot, 1_500_000_000, 1_500_000_100)?;

        assert_eq!(normalized.source_slot, 123_456);
        assert_eq!(
            normalized.quote_reserves,
            QuoteReserveState::Available {
                token_a_raw: 9_964,
                token_b_raw: 19_961,
                source_slot: 123_500,
            }
        );
        assert_eq!(normalized.normalized_at_unix_ms, 1_500_000_100);

        Ok(())
    }

    #[test]
    fn hydration_rejects_stale_context_slot() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let payload = fixture_hydration_payload(
            123_455,
            &observation.pool_state.token_0_program,
            6,
            10_000,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        assert!(parse_hydration_response(&observation, &payload).is_err());

        Ok(())
    }

    #[test]
    fn hydration_rejects_wrong_vault_owner_program() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let payload = fixture_hydration_payload(
            123_500,
            "WrongTokenProgram111111111111111111111111111",
            6,
            10_000,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        assert!(parse_hydration_response(&observation, &payload).is_err());

        Ok(())
    }

    #[test]
    fn hydration_rejects_wrong_vault_mint() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let payload = fixture_hydration_payload(
            123_500,
            &observation.pool_state.token_0_program,
            42,
            10_000,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        assert!(parse_hydration_response(&observation, &payload).is_err());

        Ok(())
    }

    #[test]
    fn hydration_rejects_effective_reserve_underflow() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let payload = fixture_hydration_payload(
            123_500,
            &observation.pool_state.token_0_program,
            6,
            35,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        assert!(parse_hydration_response(&observation, &payload).is_err());

        Ok(())
    }

    #[test]
    fn hydration_rejects_missing_account() -> Result<(), String> {
        let observation = fixture_observation(0, 1_234_567)?;
        let mut payload = fixture_hydration_payload(
            123_500,
            &observation.pool_state.token_0_program,
            6,
            10_000,
            &observation.pool_state.token_1_program,
            7,
            20_000,
        );

        payload["result"]["value"][2] = Value::Null;

        assert!(parse_hydration_response(&observation, &payload).is_err());

        Ok(())
    }

    fn fixture_observation(
        status: u8,
        open_time: u64,
    ) -> Result<RaydiumCpmmAccountObservation, String> {
        let pool_state = decode_pool_state(&fixture_pool_state(status, open_time))?;

        Ok(RaydiumCpmmAccountObservation {
            pubkey: "ExamplePool111111111111111111111111111111111".to_owned(),
            slot: 123_456,
            owner: RAYDIUM_CPMM_PROGRAM_ID.to_owned(),
            encoded_data_len: 852,
            decoded_data_len: POOL_STATE_LEN,
            pool_state,
        })
    }

    fn fixture_hydration_payload(
        context_slot: u64,
        token_0_owner: &str,
        token_0_mint_seed: u8,
        token_0_amount: u64,
        token_1_owner: &str,
        token_1_mint_seed: u8,
        token_1_amount: u64,
    ) -> Value {
        let pool_data = fixture_pool_state(0, 1_234_567);
        let token_0_data = fixture_token_account(token_0_mint_seed, token_0_amount);
        let token_1_data = fixture_token_account(token_1_mint_seed, token_1_amount);

        json!({
            "jsonrpc": "2.0",
            "id": 3,
            "result": {
                "context": {
                    "slot": context_slot
                },
                "value": [
                    fixture_rpc_account(RAYDIUM_CPMM_PROGRAM_ID, &pool_data),
                    fixture_rpc_account(token_0_owner, &token_0_data),
                    fixture_rpc_account(token_1_owner, &token_1_data)
                ]
            }
        })
    }

    fn fixture_rpc_account(owner: &str, data: &[u8]) -> Value {
        json!({
            "data": [
                BASE64_STANDARD.encode(data),
                "base64"
            ],
            "executable": false,
            "lamports": 1,
            "owner": owner,
            "rentEpoch": 0,
            "space": data.len()
        })
    }

    fn fixture_token_account(mint_seed: u8, amount: u64) -> Vec<u8> {
        let mut data = vec![0u8; TOKEN_ACCOUNT_BASE_LEN];

        data[0..32].fill(mint_seed);
        data[TOKEN_ACCOUNT_AMOUNT_OFFSET..TOKEN_ACCOUNT_AMOUNT_OFFSET + 8]
            .copy_from_slice(&amount.to_le_bytes());

        data
    }

    fn fixture_pool_state(status: u8, open_time: u64) -> Vec<u8> {
        let mut data = Vec::with_capacity(POOL_STATE_LEN);

        data.extend_from_slice(&POOL_STATE_DISCRIMINATOR);

        for seed in 1u8..=10 {
            data.extend(std::iter::repeat(seed).take(32));
        }

        data.extend_from_slice(&[
            250, // auth_bump
            status, 9, // lp_mint_decimals
            6, // mint_0_decimals
            6, // mint_1_decimals
        ]);

        for value in [1_000u64, 10, 11, 12, 13, open_time, 500] {
            data.extend_from_slice(&value.to_le_bytes());
        }

        data.push(0);
        data.push(1);
        data.extend_from_slice(&[0u8; 6]);

        data.extend_from_slice(&14u64.to_le_bytes());
        data.extend_from_slice(&15u64.to_le_bytes());

        data.extend_from_slice(&[0u8; 28 * 8]);

        data
    }
}
