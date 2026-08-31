use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};

pub const PUMPSWAP_PROGRAM_ID: &str = "pAMMBay6oceH9fJKBRHGP5D4bD4sWpmSwMn52FMfXEA";

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
                "base_decimals={} quote_decimals={} ",
                "disable_flags={} trading_state={:?} ",
                "fee_config=[{}]"
            ),
            self.slot,
            self.base_vault_raw,
            self.quote_vault_raw,
            self.effective_quote_raw,
            self.base_mint_supply_raw,
            self.base_decimals,
            self.quote_decimals,
            self.disable_flags,
            self.trading_state,
            self.fee_config.summary(),
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
        return Err("
