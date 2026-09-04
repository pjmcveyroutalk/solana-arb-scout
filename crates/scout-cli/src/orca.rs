use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const ORCA_WHIRLPOOL_PROGRAM_ID: &str = "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc";

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

const WHIRLPOOL_STATE_LEN: usize = 653;
const WHIRLPOOL_DISCRIMINATOR: [u8; 8] = [63, 149, 209, 12, 225, 128, 99, 9];

const WHIRLPOOLS_CONFIG_OFFSET: usize = 8;
const WHIRLPOOL_BUMP_OFFSET: usize = 40;
const TICK_SPACING_OFFSET: usize = 41;
const FEE_TIER_INDEX_SEED_OFFSET: usize = 43;
const FEE_RATE_OFFSET: usize = 45;
const PROTOCOL_FEE_RATE_OFFSET: usize = 47;
const LIQUIDITY_OFFSET: usize = 49;
const SQRT_PRICE_OFFSET: usize = 65;
const TICK_CURRENT_INDEX_OFFSET: usize = 81;
const TOKEN_MINT_A_OFFSET: usize = 101;
const TOKEN_VAULT_A_OFFSET: usize = 133;
const TOKEN_MINT_B_OFFSET: usize = 181;
const TOKEN_VAULT_B_OFFSET: usize = 213;

const MINT_BASE_LEN: usize = 82;
const MINT_DECIMALS_OFFSET: usize = 44;
const MINT_INITIALIZED_OFFSET: usize = 45;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaWhirlpoolState {
    pub whirlpools_config: String,
    pub whirlpool_bump: u8,
    pub tick_spacing: u16,
    pub fee_tier_index_seed: u16,
    pub fee_rate: u16,
    pub protocol_fee_rate: u16,
    pub liquidity: u128,
    pub sqrt_price: u128,
    pub tick_current_index: i32,
    pub token_mint_a: String,
    pub token_vault_a: String,
    pub token_mint_b: String,
    pub token_vault_b: String,
}

impl OrcaWhirlpoolState {
    pub fn is_adaptive_fee(&self) -> bool {
        self.fee_tier_index_seed != self.tick_spacing
    }

    pub fn summary(&self) -> String {
        format!(
            concat!(
                "config={} bump={} tick_spacing={} fee_tier_index={} ",
                "fee_rate={} protocol_fee_rate={} liquidity={} sqrt_price={} ",
                "tick_current_index={} mint_a={} mint_b={} vault_a={} vault_b={} ",
                "adaptive_fee={}"
            ),
            self.whirlpools_config,
            self.whirlpool_bump,
            self.tick_spacing,
            self.fee_tier_index_seed,
            self.fee_rate,
            self.protocol_fee_rate,
            self.liquidity,
            self.sqrt_price,
            self.tick_current_index,
            self.token_mint_a,
            self.token_mint_b,
            self.token_vault_a,
            self.token_vault_b,
            self.is_adaptive_fee(),
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaWhirlpoolAccountObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub encoded_data_len: usize,
    pub decoded_data_len: usize,
    pub pool_state: OrcaWhirlpoolState,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OrcaWhirlpoolHydrationSnapshot {
    pub slot: u64,
    pub pool_state: OrcaWhirlpoolState,
    pub token_a_program: String,
    pub token_b_program: String,
    pub token_a_decimals: u8,
    pub token_b_decimals: u8,
}

impl OrcaWhirlpoolHydrationSnapshot {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "snapshot_slot={} token_a_program={} token_b_program={} ",
                "token_a_decimals={} token_b_decimals={} adaptive_fee={}"
            ),
            self.slot,
            self.token_a_program,
            self.token_b_program,
            self.token_a_decimals,
            self.token_b_decimals,
            self.pool_state.is_adaptive_fee(),
        )
    }
}

pub fn program_subscribe_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 18,
        "method": "programSubscribe",
        "params": [
            ORCA_WHIRLPOOL_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": WHIRLPOOL_STATE_LEN
                    }
                ]
            }
        ]
    })
}

pub fn hydration_account_pubkeys(observation: &OrcaWhirlpoolAccountObservation) -> [String; 3] {
    [
        observation.pubkey.clone(),
        observation.pool_state.token_mint_a.clone(),
        observation.pool_state.token_mint_b.clone(),
    ]
}

pub fn parse_program_notification(
    payload: &Value,
) -> Result<Option<OrcaWhirlpoolAccountObservation>, String> {
    if payload.get("method").and_then(Value::as_str) != Some("programNotification") {
        return Ok(None);
    }

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "program notification missing account owner".to_owned())?;

    if owner != ORCA_WHIRLPOOL_PROGRAM_ID {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Orca notification missing slot".to_owned())?;

    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "Orca notification missing pubkey".to_owned())?
        .to_owned();

    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "Orca notification missing base64 account data".to_owned())?;

    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "Orca notification missing account-data encoding".to_owned())?;

    if encoding != "base64" {
        return Err(format!("unexpected Orca account-data encoding: {encoding}"));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Orca base64 account data: {error}"))?;

    let pool_state = decode_whirlpool_state(&decoded_data)?;

    Ok(Some(OrcaWhirlpoolAccountObservation {
        pubkey,
        slot,
        owner: owner.to_owned(),
        encoded_data_len: encoded_data.len(),
        decoded_data_len: decoded_data.len(),
        pool_state,
    }))
}

pub fn parse_hydration_response(
    observation: &OrcaWhirlpoolAccountObservation,
    payload: &Value,
) -> Result<OrcaWhirlpoolHydrationSnapshot, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Solana getMultipleAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Orca hydration response missing context slot".to_owned())?;

    if slot < observation.slot {
        return Err(format!(
            "stale Orca hydration snapshot: trigger_slot={} snapshot_slot={slot}",
            observation.slot
        ));
    }

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Orca hydration response missing account array".to_owned())?;

    if accounts.len() != 3 {
        return Err(format!(
            "Orca hydration expected exactly 3 accounts, got {}",
            accounts.len()
        ));
    }

    if accounts.iter().any(Value::is_null) {
        return Err("Orca hydration response contained a missing account".to_owned());
    }

    let pool_data = decode_rpc_account_data(
        &accounts[0],
        ORCA_WHIRLPOOL_PROGRAM_ID,
        "Orca Whirlpool snapshot",
    )?;
    let pool_state = decode_whirlpool_state(&pool_data)?;

    verify_pool_identity(&observation.pool_state, &pool_state)?;

    if pool_state.is_adaptive_fee() {
        return Err(format!(
            "Orca adaptive-fee Whirlpool {} requires Oracle hydration before admission",
            observation.pubkey
        ));
    }

    let (token_a_program, token_a_decimals) =
        parse_mint_account(&accounts[1], "Orca token A mint")?;
    let (token_b_program, token_b_decimals) =
        parse_mint_account(&accounts[2], "Orca token B mint")?;

    Ok(OrcaWhirlpoolHydrationSnapshot {
        slot,
        pool_state,
        token_a_program,
        token_b_program,
        token_a_decimals,
        token_b_decimals,
    })
}

pub fn hydrate_normalized_observation(
    observation: &OrcaWhirlpoolAccountObservation,
    snapshot: &OrcaWhirlpoolHydrationSnapshot,
    account_update_received_at_unix_ms: u64,
    hydrated_at_unix_ms: u64,
) -> Result<NormalizedPoolState, String> {
    if snapshot.slot < observation.slot {
        return Err(format!(
            "stale Orca hydration snapshot: trigger_slot={} snapshot_slot={}",
            observation.slot, snapshot.slot
        ));
    }

    verify_pool_identity(&observation.pool_state, &snapshot.pool_state)?;

    if snapshot.pool_state.is_adaptive_fee() {
        return Err(format!(
            "Orca adaptive-fee Whirlpool {} requires Oracle hydration before normalization",
            observation.pubkey
        ));
    }

    Ok(NormalizedPoolState {
        pool_id: observation.pubkey.clone(),
        venue: Venue::Orca,
        program_id: observation.owner.clone(),
        source_slot: observation.slot,
        token_a: NormalizedToken {
            mint: snapshot.pool_state.token_mint_a.clone(),
            vault: snapshot.pool_state.token_vault_a.clone(),
            decimals: snapshot.token_a_decimals,
        },
        token_b: NormalizedToken {
            mint: snapshot.pool_state.token_mint_b.clone(),
            vault: snapshot.pool_state.token_vault_b.clone(),
            decimals: snapshot.token_b_decimals,
        },
        trading_state: PoolTradingState::Tradable,
        quote_reserves: QuoteReserveState::Unavailable,
        account_update_received_at_unix_ms,
        normalized_at_unix_ms: hydrated_at_unix_ms,
    })
}

fn decode_whirlpool_state(data: &[u8]) -> Result<OrcaWhirlpoolState, String> {
    if data.len() != WHIRLPOOL_STATE_LEN {
        return Err(format!(
            "Orca Whirlpool account length mismatch: expected {WHIRLPOOL_STATE_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = read_array::<8>(data, 0, "discriminator")?;

    if discriminator != WHIRLPOOL_DISCRIMINATOR {
        return Err(format!(
            "Orca Whirlpool discriminator mismatch: expected {:?}, got {:?}",
            WHIRLPOOL_DISCRIMINATOR, discriminator
        ));
    }

    Ok(OrcaWhirlpoolState {
        whirlpools_config: read_pubkey(data, WHIRLPOOLS_CONFIG_OFFSET, "whirlpools_config")?,
        whirlpool_bump: *data
            .get(WHIRLPOOL_BUMP_OFFSET)
            .ok_or_else(|| "Orca Whirlpool missing bump".to_owned())?,
        tick_spacing: read_u16(data, TICK_SPACING_OFFSET, "tick_spacing")?,
        fee_tier_index_seed: read_u16(data, FEE_TIER_INDEX_SEED_OFFSET, "fee_tier_index_seed")?,
        fee_rate: read_u16(data, FEE_RATE_OFFSET, "fee_rate")?,
        protocol_fee_rate: read_u16(data, PROTOCOL_FEE_RATE_OFFSET, "protocol_fee_rate")?,
        liquidity: read_u128(data, LIQUIDITY_OFFSET, "liquidity")?,
        sqrt_price: read_u128(data, SQRT_PRICE_OFFSET, "sqrt_price")?,
        tick_current_index: read_i32(data, TICK_CURRENT_INDEX_OFFSET, "tick_current_index")?,
        token_mint_a: read_pubkey(data, TOKEN_MINT_A_OFFSET, "token_mint_a")?,
        token_vault_a: read_pubkey(data, TOKEN_VAULT_A_OFFSET, "token_vault_a")?,
        token_mint_b: read_pubkey(data, TOKEN_MINT_B_OFFSET, "token_mint_b")?,
        token_vault_b: read_pubkey(data, TOKEN_VAULT_B_OFFSET, "token_vault_b")?,
    })
}

fn verify_pool_identity(
    trigger: &OrcaWhirlpoolState,
    snapshot: &OrcaWhirlpoolState,
) -> Result<(), String> {
    if trigger.whirlpools_config != snapshot.whirlpools_config {
        return Err("Orca hydration whirlpools_config changed".to_owned());
    }

    if trigger.tick_spacing != snapshot.tick_spacing {
        return Err("Orca hydration tick_spacing changed".to_owned());
    }

    if trigger.fee_tier_index_seed != snapshot.fee_tier_index_seed {
        return Err("Orca hydration fee_tier_index_seed changed".to_owned());
    }

    if trigger.token_mint_a != snapshot.token_mint_a {
        return Err("Orca hydration token_mint_a changed".to_owned());
    }

    if trigger.token_vault_a != snapshot.token_vault_a {
        return Err("Orca hydration token_vault_a changed".to_owned());
    }

    if trigger.token_mint_b != snapshot.token_mint_b {
        return Err("Orca hydration token_mint_b changed".to_owned());
    }

    if trigger.token_vault_b != snapshot.token_vault_b {
        return Err("Orca hydration token_vault_b changed".to_owned());
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

    decode_account_data(account, label)
}

fn parse_mint_account(account: &Value, label: &str) -> Result<(String, u8), String> {
    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing owner"))?;

    if owner != SPL_TOKEN_PROGRAM_ID && owner != TOKEN_2022_PROGRAM_ID {
        return Err(format!("{label} uses unsupported token program {owner}"));
    }

    let data = decode_account_data(account, label)?;

    if data.len() < MINT_BASE_LEN {
        return Err(format!(
            "{label} shorter than Mint base layout: expected at least {MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    let initialized = *data
        .get(MINT_INITIALIZED_OFFSET)
        .ok_or_else(|| format!("{label} missing initialized flag"))?;

    if initialized != 1 {
        return Err(format!(
            "{label} is not initialized: initialized={initialized}"
        ));
    }

    let decimals = *data
        .get(MINT_DECIMALS_OFFSET)
        .ok_or_else(|| format!("{label} missing decimals"))?;

    Ok((owner.to_owned(), decimals))
}

fn decode_account_data(account: &Value, label: &str) -> Result<Vec<u8>, String> {
    let encoded = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing base64 account data"))?;

    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing account-data encoding"))?;

    if encoding != "base64" {
        return Err(format!(
            "{label} has unsupported account-data encoding {encoding}"
        ));
    }

    BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("{label} contains invalid base64: {error}"))
}

fn read_pubkey(data: &[u8], offset: usize, label: &str) -> Result<String, String> {
    let bytes = read_array::<32>(data, offset, label)?;
    Ok(bs58::encode(bytes).into_string())
}

fn read_u16(data: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    Ok(u16::from_le_bytes(read_array::<2>(data, offset, label)?))
}

fn read_i32(data: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    Ok(i32::from_le_bytes(read_array::<4>(data, offset, label)?))
}

fn read_u128(data: &[u8], offset: usize, label: &str) -> Result<u128, String> {
    Ok(u128::from_le_bytes(read_array::<16>(data, offset, label)?))
}

fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("Orca {label} offset overflow"))?;

    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("Orca Whirlpool {label} outside account data"))?;

    <[u8; N]>::try_from(bytes)
        .map_err(|_| format!("Orca Whirlpool {label} had invalid byte length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_whirlpool_data(adaptive_fee: bool) -> Vec<u8> {
        let mut data = vec![0u8; WHIRLPOOL_STATE_LEN];

        data[0..8].copy_from_slice(&WHIRLPOOL_DISCRIMINATOR);
        data[WHIRLPOOLS_CONFIG_OFFSET..WHIRLPOOLS_CONFIG_OFFSET + 32].copy_from_slice(&[1u8; 32]);
        data[WHIRLPOOL_BUMP_OFFSET] = 255;

        data[TICK_SPACING_OFFSET..TICK_SPACING_OFFSET + 2].copy_from_slice(&64u16.to_le_bytes());

        let fee_tier_index = if adaptive_fee { 32u16 } else { 64u16 };
        data[FEE_TIER_INDEX_SEED_OFFSET..FEE_TIER_INDEX_SEED_OFFSET + 2]
            .copy_from_slice(&fee_tier_index.to_le_bytes());

        data[FEE_RATE_OFFSET..FEE_RATE_OFFSET + 2].copy_from_slice(&3_000u16.to_le_bytes());
        data[PROTOCOL_FEE_RATE_OFFSET..PROTOCOL_FEE_RATE_OFFSET + 2]
            .copy_from_slice(&300u16.to_le_bytes());
        data[LIQUIDITY_OFFSET..LIQUIDITY_OFFSET + 16].copy_from_slice(&1_000_000u128.to_le_bytes());
        data[SQRT_PRICE_OFFSET..SQRT_PRICE_OFFSET + 16]
            .copy_from_slice(&18_446_744_073_709_551_616u128.to_le_bytes());
        data[TICK_CURRENT_INDEX_OFFSET..TICK_CURRENT_INDEX_OFFSET + 4]
            .copy_from_slice(&0i32.to_le_bytes());

        data[TOKEN_MINT_A_OFFSET..TOKEN_MINT_A_OFFSET + 32].copy_from_slice(&[2u8; 32]);
        data[TOKEN_VAULT_A_OFFSET..TOKEN_VAULT_A_OFFSET + 32].copy_from_slice(&[3u8; 32]);
        data[TOKEN_MINT_B_OFFSET..TOKEN_MINT_B_OFFSET + 32].copy_from_slice(&[4u8; 32]);
        data[TOKEN_VAULT_B_OFFSET..TOKEN_VAULT_B_OFFSET + 32].copy_from_slice(&[5u8; 32]);

        data
    }

    fn sample_mint_data(decimals: u8) -> Vec<u8> {
        let mut data = vec![0u8; MINT_BASE_LEN];
        data[MINT_DECIMALS_OFFSET] = decimals;
        data[MINT_INITIALIZED_OFFSET] = 1;
        data
    }

    fn rpc_account(owner: &str, data: &[u8]) -> Value {
        json!({
            "data": [
                BASE64_STANDARD.encode(data),
                "base64"
            ],
            "executable": false,
            "lamports": 1,
            "owner": owner,
            "rentEpoch": 0
        })
    }

    fn observation(
        adaptive_fee: bool,
        slot: u64,
    ) -> Result<OrcaWhirlpoolAccountObservation, String> {
        let data = sample_whirlpool_data(adaptive_fee);
        let pool_state = decode_whirlpool_state(&data)?;

        Ok(OrcaWhirlpoolAccountObservation {
            pubkey: bs58::encode([9u8; 32]).into_string(),
            slot,
            owner: ORCA_WHIRLPOOL_PROGRAM_ID.to_owned(),
            encoded_data_len: BASE64_STANDARD.encode(&data).len(),
            decoded_data_len: data.len(),
            pool_state,
        })
    }

    #[test]
    fn official_whirlpool_layout_decodes() -> Result<(), String> {
        let data = sample_whirlpool_data(false);
        let state = decode_whirlpool_state(&data)?;

        assert_eq!(state.tick_spacing, 64);
        assert_eq!(state.fee_tier_index_seed, 64);
        assert_eq!(state.fee_rate, 3_000);
        assert_eq!(state.protocol_fee_rate, 300);
        assert!(!state.is_adaptive_fee());

        Ok(())
    }

    #[test]
    fn adaptive_fee_pool_is_detected_from_fee_tier_index() -> Result<(), String> {
        let data = sample_whirlpool_data(true);
        let state = decode_whirlpool_state(&data)?;

        assert_eq!(state.tick_spacing, 64);
        assert_eq!(state.fee_tier_index_seed, 32);
        assert!(state.is_adaptive_fee());

        Ok(())
    }

    #[test]
    fn wrong_discriminator_is_rejected() -> Result<(), String> {
        let mut data = sample_whirlpool_data(false);
        data[0] ^= 1;

        match decode_whirlpool_state(&data) {
            Ok(_) => Err("invalid discriminator was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("discriminator mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn unrelated_program_notification_is_ignored() -> Result<(), String> {
        let payload = json!({
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": 123_456
                    },
                    "value": {
                        "pubkey": bs58::encode([8u8; 32]).into_string(),
                        "account": rpc_account(
                            "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C",
                            &sample_whirlpool_data(false)
                        )
                    }
                }
            }
        });

        let parsed = parse_program_notification(&payload)?;

        assert!(parsed.is_none());

        Ok(())
    }

    #[test]
    fn ordinary_pool_hydrates_without_fabricating_reserves() -> Result<(), String> {
        let observation = observation(false, 100)?;
        let pool_data = sample_whirlpool_data(false);
        let mint_a_data = sample_mint_data(9);
        let mint_b_data = sample_mint_data(6);

        let payload = json!({
            "result": {
                "context": {
                    "slot": 101
                },
                "value": [
                    rpc_account(ORCA_WHIRLPOOL_PROGRAM_ID, &pool_data),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &mint_a_data),
                    rpc_account(TOKEN_2022_PROGRAM_ID, &mint_b_data)
                ]
            }
        });

        let snapshot = parse_hydration_response(&observation, &payload)?;

        assert_eq!(snapshot.slot, 101);
        assert_eq!(snapshot.token_a_decimals, 9);
        assert_eq!(snapshot.token_b_decimals, 6);

        let normalized = hydrate_normalized_observation(&observation, &snapshot, 1_000, 1_001)?;

        assert_eq!(normalized.venue, Venue::Orca);
        assert_eq!(normalized.trading_state, PoolTradingState::Tradable);
        assert_eq!(normalized.token_a.decimals, 9);
        assert_eq!(normalized.token_b.decimals, 6);
        assert_eq!(normalized.quote_reserves, QuoteReserveState::Unavailable);

        Ok(())
    }

    #[test]
    fn adaptive_fee_pool_fails_closed_without_oracle_hydration() -> Result<(), String> {
        let observation = observation(true, 100)?;
        let pool_data = sample_whirlpool_data(true);

        let payload = json!({
            "result": {
                "context": {
                    "slot": 101
                },
                "value": [
                    rpc_account(ORCA_WHIRLPOOL_PROGRAM_ID, &pool_data),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(9)),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(6))
                ]
            }
        });

        match parse_hydration_response(&observation, &payload) {
            Ok(_) => Err("adaptive-fee Whirlpool was admitted without Oracle".to_owned()),
            Err(error) => {
                assert!(error.contains("requires Oracle hydration"));
                Ok(())
            }
        }
    }

    #[test]
    fn stale_hydration_is_rejected() -> Result<(), String> {
        let observation = observation(false, 100)?;

        let payload = json!({
            "result": {
                "context": {
                    "slot": 99
                },
                "value": [
                    rpc_account(
                        ORCA_WHIRLPOOL_PROGRAM_ID,
                        &sample_whirlpool_data(false)
                    ),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(9)),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(6))
                ]
            }
        });

        match parse_hydration_response(&observation, &payload) {
            Ok(_) => Err("stale hydration snapshot was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("stale Orca hydration snapshot"));
                Ok(())
            }
        }
    }

    #[test]
    fn hydration_identity_change_is_rejected() -> Result<(), String> {
        let observation = observation(false, 100)?;
        let mut changed_pool_data = sample_whirlpool_data(false);

        changed_pool_data[TOKEN_MINT_B_OFFSET..TOKEN_MINT_B_OFFSET + 32]
            .copy_from_slice(&[7u8; 32]);

        let payload = json!({
            "result": {
                "context": {
                    "slot": 101
                },
                "value": [
                    rpc_account(ORCA_WHIRLPOOL_PROGRAM_ID, &changed_pool_data),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(9)),
                    rpc_account(SPL_TOKEN_PROGRAM_ID, &sample_mint_data(6))
                ]
            }
        });

        match parse_hydration_response(&observation, &payload) {
            Ok(_) => Err("identity-changing hydration snapshot was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("token_mint_b changed"));
                Ok(())
            }
        }
    }
}
