use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

const PUMPSWAP_GLOBAL_CONFIG: &str = "ADyA8hdefvWN2dbGGWFotbzWxrAvLW83WG6QCVXvJKqw";

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";

const POOL_DISCRIMINATOR: [u8; 8] = [241, 154, 109, 4, 17, 177, 109, 188];
const GLOBAL_CONFIG_DISCRIMINATOR: [u8; 8] = [149, 8, 156, 202, 160, 252, 176, 217];

const POOL_BASE_LEN: usize = 211;
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
const MINT_DECIMALS_OFFSET: usize = 44;
const MINT_INITIALIZED_OFFSET: usize = 45;

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
    pub base_decimals: u8,
    pub quote_decimals: u8,
    pub disable_flags: u8,
    pub trading_state: PoolTradingState,
}

impl PumpSwapHydrationSnapshot {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "reserve_slot={} ",
                "base_vault_raw={} quote_vault_raw={} ",
                "effective_quote_raw={} ",
                "base_decimals={} quote_decimals={} ",
                "disable_flags={} trading_state={:?}"
            ),
            self.slot,
            self.base_vault_raw,
            self.quote_vault_raw,
            self.effective_quote_raw,
            self.base_decimals,
            self.quote_decimals,
            self.disable_flags,
            self.trading_state,
        )
    }
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

pub fn hydration_account_pubkeys(observation: &PumpSwapAccountObservation) -> [String; 6] {
    [
        observation.pubkey.clone(),
        observation.pool_state.pool_base_token_account.clone(),
        observation.pool_state.pool_quote_token_account.clone(),
        observation.pool_state.base_mint.clone(),
        observation.pool_state.quote_mint.clone(),
        PUMPSWAP_GLOBAL_CONFIG.to_owned(),
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

    if accounts.len() != 6 {
        return Err(format!(
            "PumpSwap hydration expected exactly 6 accounts, got {}",
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
        base_decimals,
        quote_decimals,
        disable_flags,
        trading_state,
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
        trading_state: snapshot.trading_state.clone(),
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
    let base_mint = pubkey_at(data, 43, "base_mint")?;
    let quote_mint = pubkey_at(data, 75, "quote_mint")?;
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
        data[MINT_DECIMALS_OFFSET] = decimals;
        data[MINT_INITIALIZED_OFFSET] = 1;
        data
    }

    #[test]
    fn decodes_current_pool_layout() {
        let state = decode_pool_state(&sample_pool_data()).expect("pool decode");

        assert_eq!(state.pool_bump, 254);
        assert_eq!(state.lp_supply, 123);
        assert_eq!(state.is_mayhem_mode, Some(true));
        assert_eq!(state.is_cashback_coin, Some(false));
        assert_eq!(state.virtual_quote_reserves, 25);
    }

    #[test]
    fn accepts_legacy_pool_prefix_without_appended_fields() {
        let data = sample_pool_data()[..POOL_BASE_LEN].to_vec();

        let state = decode_pool_state(&data).expect("legacy pool decode");

        assert_eq!(state.coin_creator, None);
        assert_eq!(state.is_mayhem_mode, None);
        assert_eq!(state.is_cashback_coin, None);
        assert_eq!(state.virtual_quote_reserves, 0);
    }

    #[test]
    fn accepts_future_extension_bytes() {
        let mut data = sample_pool_data();
        data.extend_from_slice(&[9u8; 24]);

        let state = decode_pool_state(&data).expect("future extension decode");

        assert_eq!(state.virtual_quote_reserves, 25);
    }

    #[test]
    fn rejects_partial_appended_field() {
        let data = sample_pool_data()[..220].to_vec();

        let error = decode_pool_state(&data).expect_err("partial extension must fail");

        assert!(error.contains("ended inside an appended field"));
    }

    #[test]
    fn rejects_wrong_discriminator() {
        let mut data = sample_pool_data();
        data[0] ^= 1;

        let error = decode_pool_state(&data).expect_err("wrong discriminator must fail");

        assert_eq!(error, "unexpected PumpSwap Pool discriminator");
    }

    #[test]
    fn rejects_invalid_appended_bool() {
        let mut data = sample_pool_data();
        data[243] = 2;

        let error = decode_pool_state(&data).expect_err("invalid bool must fail");

        assert!(error.contains("is_mayhem_mode invalid bool"));
    }

    #[test]
    fn hydration_includes_global_config() {
        let state = decode_pool_state(&sample_pool_data()).expect("pool decode");

        let observation = PumpSwapAccountObservation {
            pubkey: "pool".to_owned(),
            slot: 1,
            owner: PUMPSWAP_PROGRAM_ID.to_owned(),
            encoded_data_len: 0,
            decoded_data_len: POOL_VIRTUAL_QUOTE_END,
            pool_state: state,
        };

        let accounts = hydration_account_pubkeys(&observation);

        assert_eq!(accounts.len(), 6);
        assert_eq!(accounts[5], PUMPSWAP_GLOBAL_CONFIG);
    }

    #[test]
    fn parses_global_config_disable_flags() {
        let account = rpc_account(
            PUMPSWAP_PROGRAM_ID,
            &sample_global_config_data(DISABLE_BUY_MASK | DISABLE_SELL_MASK),
        );

        let flags = parse_global_config(&account).expect("global config decode");

        assert_eq!(flags, DISABLE_BUY_MASK | DISABLE_SELL_MASK);
    }

    #[test]
    fn rejects_wrong_global_config_discriminator() {
        let mut data = sample_global_config_data(0);
        data[0] ^= 1;

        let account = rpc_account(PUMPSWAP_PROGRAM_ID, &data);

        let error = parse_global_config(&account).expect_err("wrong discriminator must fail");

        assert_eq!(error, "unexpected PumpSwap GlobalConfig discriminator");
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
    fn initialized_mint_returns_decimals() {
        let data = initialized_mint(9);

        assert_eq!(
            parse_mint_decimals(&data, "test mint").expect("mint decode"),
            9
        );
    }

    #[test]
    fn rejects_uninitialized_mint() {
        let mut data = initialized_mint(9);
        data[MINT_INITIALIZED_OFFSET] = 0;

        let error = parse_mint_decimals(&data, "test mint").expect_err("mint must fail");

        assert_eq!(error, "test mint is not initialized");
    }

    #[test]
    fn rejects_invalid_mint_initialized_value() {
        let mut data = initialized_mint(9);
        data[MINT_INITIALIZED_OFFSET] = 2;

        let error = parse_mint_decimals(&data, "test mint").expect_err("mint must fail");

        assert_eq!(error, "test mint has invalid is_initialized value: 2");
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
}
