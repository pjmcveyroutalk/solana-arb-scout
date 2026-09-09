use crate::meteora::{
    decode_bin_array, decode_bitmap_extension, decode_lb_pair, DlmmProtocolProfile,
    MeteoraBinArraySnapshotInput, MeteoraBitmapExtensionState, MeteoraClockSnapshot,
    MeteoraDlmmSnapshot, MeteoraLbPairState, MeteoraSnapshotSource, BITMAP_EXTENSION_ACCOUNT_LEN,
    LB_PAIR_ACCOUNT_LEN, LB_PAIR_DISCRIMINATOR, METEORA_DLMM_PROGRAM_ID,
};
use crate::meteora_m9::meteora_initial_hydration_plan;
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use scout_core::{
    ensure_supported_token_2022_mint_extensions, NormalizedPoolState, NormalizedToken,
    PoolTradingState, QuoteReserveState, Venue,
};
use serde_json::{json, Value};
use solana_pubkey::{pubkey, Pubkey};

const METEORA_DLMM_PROGRAM_PUBKEY: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const CLOCK_SYSVAR_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const SYSVAR_OWNER_ID: &str = "Sysvar1111111111111111111111111111111111111";

const BIN_ARRAY_BITMAP_SEED: &[u8] = b"bitmap";
const ORACLE_SEED: &[u8] = b"oracle";

const MINT_BASE_LEN: usize = 82;
const MINT_DECIMALS_OFFSET: usize = 44;
const MINT_INITIALIZED_OFFSET: usize = 45;

const CLOCK_DATA_LEN: usize = 40;
const CLOCK_SLOT_OFFSET: usize = 0;
const CLOCK_EPOCH_START_TIMESTAMP_OFFSET: usize = 8;
const CLOCK_EPOCH_OFFSET: usize = 16;
const CLOCK_LEADER_SCHEDULE_EPOCH_OFFSET: usize = 24;
const CLOCK_UNIX_TIMESTAMP_OFFSET: usize = 32;

const LB_PAIR_FUNCTION_TYPE_OFFSET: usize = 35;
const LB_PAIR_PAIR_TYPE_OFFSET: usize = 75;
const LB_PAIR_STATUS_OFFSET: usize = 82;
const LB_PAIR_ACTIVATION_TYPE_OFFSET: usize = 86;
const LB_PAIR_CREATOR_POOL_ON_OFF_CONTROL_OFFSET: usize = 87;
const LB_PAIR_RESERVE_X_OFFSET: usize = 152;
const LB_PAIR_RESERVE_Y_OFFSET: usize = 184;
const LB_PAIR_REWARD_MINT_0_OFFSET: usize = 264;
const LB_PAIR_REWARD_MINT_1_OFFSET: usize = 408;
const LB_PAIR_ORACLE_OFFSET: usize = 552;
const LB_PAIR_ACTIVATION_POINT_OFFSET: usize = 816;
const LB_PAIR_TOKEN_X_PROGRAM_FLAG_OFFSET: usize = 880;
const LB_PAIR_TOKEN_Y_PROGRAM_FLAG_OFFSET: usize = 881;
const LB_PAIR_VERSION_OFFSET: usize = 882;

const PAIR_STATUS_ENABLED: u8 = 0;
const PAIR_STATUS_DISABLED: u8 = 1;
const PAIR_TYPE_PERMISSIONLESS: u8 = 0;
const PAIR_TYPE_PERMISSION: u8 = 1;
const PAIR_TYPE_CUSTOMIZABLE_PERMISSIONLESS: u8 = 2;
const PAIR_TYPE_PERMISSIONLESS_V2: u8 = 3;
const FUNCTION_TYPE_UNDETERMINED: u8 = 0;
const FUNCTION_TYPE_LIQUIDITY_MINING: u8 = 1;
const FUNCTION_TYPE_LIMIT_ORDER: u8 = 2;
const ACTIVATION_TYPE_SLOT: u8 = 0;
const ACTIVATION_TYPE_TIMESTAMP: u8 = 1;
const TOKEN_PROGRAM_FLAG_SPL: u8 = 0;
const TOKEN_PROGRAM_FLAG_2022: u8 = 1;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraLiveAdmissionState {
    pub function_type: u8,
    pub pair_type: u8,
    pub status: u8,
    pub activation_type: u8,
    pub creator_pool_on_off_control: u8,
    pub reserve_x: [u8; 32],
    pub reserve_y: [u8; 32],
    pub reward_mint_0: [u8; 32],
    pub reward_mint_1: [u8; 32],
    pub oracle: [u8; 32],
    pub activation_point: u64,
    pub token_x_program_flag: u8,
    pub token_y_program_flag: u8,
    pub version: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraLiveObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub encoded_data_len: usize,
    pub decoded_data_len: usize,
    pub lb_pair: MeteoraLbPairState,
    pub admission: MeteoraLiveAdmissionState,
}

#[derive(Debug)]
pub struct MeteoraLiveBaseHydration {
    pub normalized: NormalizedPoolState,
    pub snapshot: MeteoraDlmmSnapshot,
    pub admission: MeteoraLiveAdmissionState,
    pub token_x_program: String,
    pub token_y_program: String,
}

#[derive(Debug)]
pub struct MeteoraLivePreparedState {
    pub normalized: NormalizedPoolState,
    pub snapshot: MeteoraDlmmSnapshot,
    pub admission: MeteoraLiveAdmissionState,
    pub token_x_program: String,
    pub token_y_program: String,
}

pub fn meteora_program_subscribe_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 60,
        "method": "programSubscribe",
        "params": [
            METEORA_DLMM_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": LB_PAIR_ACCOUNT_LEN
                    }
                ]
            }
        ]
    })
}

pub fn parse_meteora_program_notification(
    payload: &Value,
) -> Result<Option<MeteoraLiveObservation>, String> {
    if payload.get("method").and_then(Value::as_str) != Some("programNotification") {
        return Ok(None);
    }

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "Meteora notification missing account owner".to_owned())?;
    if owner != METEORA_DLMM_PROGRAM_ID {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Meteora notification missing slot".to_owned())?;
    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "Meteora notification missing pubkey".to_owned())?
        .to_owned();
    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "Meteora notification missing base64 account data".to_owned())?;
    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "Meteora notification missing account-data encoding".to_owned())?;
    if encoding != "base64" {
        return Err(format!(
            "unexpected Meteora account-data encoding: {encoding}"
        ));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Meteora base64 account data: {error}"))?;
    let lb_pair = decode_lb_pair(owner, &decoded_data)
        .map_err(|error| format!("Meteora LB pair decode failed: {error:?}"))?;
    let admission = decode_admission_state(&decoded_data)?;
    validate_admission_identity(&pubkey, &lb_pair, &admission)?;

    Ok(Some(MeteoraLiveObservation {
        pubkey,
        slot,
        owner: owner.to_owned(),
        encoded_data_len: encoded_data.len(),
        decoded_data_len: decoded_data.len(),
        lb_pair,
        admission,
    }))
}

pub fn meteora_base_hydration_account_pubkeys(
    observation: &MeteoraLiveObservation,
) -> Result<[String; 5], String> {
    Ok([
        observation.pubkey.clone(),
        bs58::encode(observation.lb_pair.mint_x).into_string(),
        bs58::encode(observation.lb_pair.mint_y).into_string(),
        CLOCK_SYSVAR_ID.to_owned(),
        bs58::encode(derive_bitmap_extension(decode_pubkey(&observation.pubkey)?).0).into_string(),
    ])
}

pub fn parse_meteora_base_hydration_response(
    observation: &MeteoraLiveObservation,
    payload: &Value,
    generation_id: u64,
    account_update_received_at_unix_ms: u64,
    hydrated_at_unix_ms: u64,
) -> Result<MeteoraLiveBaseHydration, String> {
    let slot = hydration_response_slot(payload, observation.slot, "Meteora base hydration")?;
    let accounts = hydration_response_accounts(payload, 5, "Meteora base hydration")?;

    let lb_pair_data = decode_required_rpc_account(
        &accounts[0],
        METEORA_DLMM_PROGRAM_ID,
        "Meteora LB pair snapshot",
    )?;
    let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &lb_pair_data)
        .map_err(|error| format!("Meteora LB pair snapshot decode failed: {error:?}"))?;
    let admission = decode_admission_state(&lb_pair_data)?;
    verify_stable_identity(observation, &lb_pair, &admission)?;

    let (token_x_program, token_x_decimals) = parse_mint_account(
        &accounts[1],
        admission.token_x_program_flag,
        "Meteora token X mint",
    )?;
    let (token_y_program, token_y_decimals) = parse_mint_account(
        &accounts[2],
        admission.token_y_program_flag,
        "Meteora token Y mint",
    )?;
    let clock_data = decode_required_rpc_account(&accounts[3], SYSVAR_OWNER_ID, "Meteora Clock")?;
    let clock = decode_clock(&clock_data)?;
    let bitmap_extension = parse_optional_bitmap_extension(&accounts[4], observation)?;

    let source = MeteoraSnapshotSource {
        source_slot: slot,
        generation_id,
    };
    let snapshot = MeteoraDlmmSnapshot::new(
        decode_pubkey(&observation.pubkey)?,
        lb_pair,
        Vec::new(),
        bitmap_extension,
        clock,
        DlmmProtocolProfile::V0_12,
        source,
    )
    .map_err(|error| format!("Meteora base snapshot rejected: {error:?}"))?;

    let normalized = normalized_pool(
        observation,
        &admission,
        token_x_decimals,
        token_y_decimals,
        clock,
        slot,
        (account_update_received_at_unix_ms, hydrated_at_unix_ms),
    )?;

    Ok(MeteoraLiveBaseHydration {
        normalized,
        snapshot,
        admission,
        token_x_program,
        token_y_program,
    })
}

pub fn meteora_initial_bin_array_pubkeys(
    base: &MeteoraLiveBaseHydration,
) -> Result<Vec<String>, String> {
    let mut targets = Vec::new();

    for swap_for_y in [true, false] {
        let plan = meteora_initial_hydration_plan(&base.snapshot, swap_for_y)
            .map_err(|error| format!("Meteora initial hydration plan failed: {error:?}"))?;

        for target in plan.targets {
            if targets
                .iter()
                .all(|existing: &([u8; 32], i64)| existing.0 != target.pubkey)
            {
                targets.push((target.pubkey, target.index));
            }
        }
    }

    if targets.is_empty() {
        return Err("Meteora pool has no initialized directional BinArray target".to_owned());
    }

    Ok(targets
        .into_iter()
        .map(|(pubkey, _)| bs58::encode(pubkey).into_string())
        .collect())
}

pub fn meteora_quote_hydration_account_pubkeys(
    observation: &MeteoraLiveObservation,
    bin_array_pubkeys: &[String],
) -> Result<Vec<String>, String> {
    let mut accounts = meteora_base_hydration_account_pubkeys(observation)?.to_vec();
    accounts.extend(bin_array_pubkeys.iter().cloned());
    Ok(accounts)
}

pub fn parse_meteora_quote_hydration_response(
    observation: &MeteoraLiveObservation,
    expected_bin_array_pubkeys: &[String],
    payload: &Value,
    generation_id: u64,
    account_update_received_at_unix_ms: u64,
    hydrated_at_unix_ms: u64,
) -> Result<MeteoraLivePreparedState, String> {
    let expected_count = 5_usize
        .checked_add(expected_bin_array_pubkeys.len())
        .ok_or_else(|| "Meteora quote hydration account count overflow".to_owned())?;
    let slot = hydration_response_slot(payload, observation.slot, "Meteora quote hydration")?;
    let accounts = hydration_response_accounts(payload, expected_count, "Meteora quote hydration")?;

    let lb_pair_data = decode_required_rpc_account(
        &accounts[0],
        METEORA_DLMM_PROGRAM_ID,
        "Meteora LB pair quote snapshot",
    )?;
    let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &lb_pair_data)
        .map_err(|error| format!("Meteora quote LB pair decode failed: {error:?}"))?;
    let admission = decode_admission_state(&lb_pair_data)?;
    verify_stable_identity(observation, &lb_pair, &admission)?;

    let (token_x_program, token_x_decimals) = parse_mint_account(
        &accounts[1],
        admission.token_x_program_flag,
        "Meteora token X mint",
    )?;
    let (token_y_program, token_y_decimals) = parse_mint_account(
        &accounts[2],
        admission.token_y_program_flag,
        "Meteora token Y mint",
    )?;
    let clock_data = decode_required_rpc_account(&accounts[3], SYSVAR_OWNER_ID, "Meteora Clock")?;
    let clock = decode_clock(&clock_data)?;
    let bitmap_extension = parse_optional_bitmap_extension(&accounts[4], observation)?;
    let lb_pair_pubkey = decode_pubkey(&observation.pubkey)?;

    let mut bin_arrays = Vec::with_capacity(expected_bin_array_pubkeys.len());
    for (offset, expected_pubkey) in expected_bin_array_pubkeys.iter().enumerate() {
        let account_index = 5_usize
            .checked_add(offset)
            .ok_or_else(|| "Meteora quote hydration bin-array index overflow".to_owned())?;
        let data = decode_required_rpc_account(
            &accounts[account_index],
            METEORA_DLMM_PROGRAM_ID,
            "Meteora BinArray",
        )?;
        let state = decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, DlmmProtocolProfile::V0_12)
            .map_err(|error| format!("Meteora BinArray decode failed: {error:?}"))?;
        let pubkey = decode_pubkey(expected_pubkey)?;
        let (canonical_pubkey, _) =
            crate::meteora::derive_bin_array_pda(lb_pair_pubkey, state.index)
                .map_err(|error| format!("Meteora BinArray PDA derivation failed: {error:?}"))?;
        if pubkey != canonical_pubkey {
            return Err(format!(
                "Meteora BinArray pubkey mismatch: expected={} index={} canonical={}",
                expected_pubkey,
                state.index,
                bs58::encode(canonical_pubkey).into_string()
            ));
        }
        bin_arrays.push(MeteoraBinArraySnapshotInput { pubkey, state });
    }

    let source = MeteoraSnapshotSource {
        source_slot: slot,
        generation_id,
    };
    let snapshot = MeteoraDlmmSnapshot::new(
        lb_pair_pubkey,
        lb_pair,
        bin_arrays,
        bitmap_extension,
        clock,
        DlmmProtocolProfile::V0_12,
        source,
    )
    .map_err(|error| format!("Meteora quote snapshot rejected: {error:?}"))?;

    require_initial_directional_hydration(&snapshot)?;

    let normalized = normalized_pool(
        observation,
        &admission,
        token_x_decimals,
        token_y_decimals,
        clock,
        slot,
        (account_update_received_at_unix_ms, hydrated_at_unix_ms),
    )?;

    Ok(MeteoraLivePreparedState {
        normalized,
        snapshot,
        admission,
        token_x_program,
        token_y_program,
    })
}

fn decode_admission_state(data: &[u8]) -> Result<MeteoraLiveAdmissionState, String> {
    if data.len() != LB_PAIR_ACCOUNT_LEN {
        return Err(format!(
            "Meteora LB pair admission length mismatch: expected {LB_PAIR_ACCOUNT_LEN}, got {}",
            data.len()
        ));
    }
    let discriminator = read_array::<8>(data, 0, "Meteora LB pair discriminator")?;
    if discriminator != LB_PAIR_DISCRIMINATOR {
        return Err("Meteora LB pair admission discriminator mismatch".to_owned());
    }

    let state = MeteoraLiveAdmissionState {
        function_type: read_u8(data, LB_PAIR_FUNCTION_TYPE_OFFSET, "function_type")?,
        pair_type: read_u8(data, LB_PAIR_PAIR_TYPE_OFFSET, "pair_type")?,
        status: read_u8(data, LB_PAIR_STATUS_OFFSET, "status")?,
        activation_type: read_u8(data, LB_PAIR_ACTIVATION_TYPE_OFFSET, "activation_type")?,
        creator_pool_on_off_control: read_u8(
            data,
            LB_PAIR_CREATOR_POOL_ON_OFF_CONTROL_OFFSET,
            "creator_pool_on_off_control",
        )?,
        reserve_x: read_array::<32>(data, LB_PAIR_RESERVE_X_OFFSET, "reserve_x")?,
        reserve_y: read_array::<32>(data, LB_PAIR_RESERVE_Y_OFFSET, "reserve_y")?,
        reward_mint_0: read_array::<32>(data, LB_PAIR_REWARD_MINT_0_OFFSET, "reward_mint_0")?,
        reward_mint_1: read_array::<32>(data, LB_PAIR_REWARD_MINT_1_OFFSET, "reward_mint_1")?,
        oracle: read_array::<32>(data, LB_PAIR_ORACLE_OFFSET, "oracle")?,
        activation_point: read_u64(data, LB_PAIR_ACTIVATION_POINT_OFFSET, "activation_point")?,
        token_x_program_flag: read_u8(
            data,
            LB_PAIR_TOKEN_X_PROGRAM_FLAG_OFFSET,
            "token_x_program_flag",
        )?,
        token_y_program_flag: read_u8(
            data,
            LB_PAIR_TOKEN_Y_PROGRAM_FLAG_OFFSET,
            "token_y_program_flag",
        )?,
        version: read_u8(data, LB_PAIR_VERSION_OFFSET, "version")?,
    };

    validate_admission_values(&state)?;
    Ok(state)
}

fn validate_admission_values(state: &MeteoraLiveAdmissionState) -> Result<(), String> {
    if !matches!(
        state.pair_type,
        PAIR_TYPE_PERMISSIONLESS
            | PAIR_TYPE_PERMISSION
            | PAIR_TYPE_CUSTOMIZABLE_PERMISSIONLESS
            | PAIR_TYPE_PERMISSIONLESS_V2
    ) {
        return Err(format!("unsupported Meteora pair_type {}", state.pair_type));
    }
    if !matches!(state.status, PAIR_STATUS_ENABLED | PAIR_STATUS_DISABLED) {
        return Err(format!("invalid Meteora pair status {}", state.status));
    }
    let activation_gated = matches!(
        state.pair_type,
        PAIR_TYPE_PERMISSION | PAIR_TYPE_CUSTOMIZABLE_PERMISSIONLESS
    );
    if activation_gated
        && !matches!(
            state.activation_type,
            ACTIVATION_TYPE_SLOT | ACTIVATION_TYPE_TIMESTAMP
        )
    {
        return Err(format!(
            "unsupported Meteora activation_type {}",
            state.activation_type
        ));
    }
    if !supports_limit_order_profile(state)? {
        return Err(format!(
            "unsupported Meteora function profile for sealed M13 quote path: function_type={}",
            state.function_type
        ));
    }
    validate_token_program_flag(state.token_x_program_flag, "token X")?;
    validate_token_program_flag(state.token_y_program_flag, "token Y")?;
    Ok(())
}

fn supports_limit_order_profile(state: &MeteoraLiveAdmissionState) -> Result<bool, String> {
    match state.function_type {
        FUNCTION_TYPE_LIMIT_ORDER => Ok(true),
        FUNCTION_TYPE_LIQUIDITY_MINING => Ok(false),
        FUNCTION_TYPE_UNDETERMINED => {
            Ok(state.reward_mint_0 == [0_u8; 32] && state.reward_mint_1 == [0_u8; 32])
        }
        value => Err(format!("unsupported Meteora function_type {value}")),
    }
}

fn validate_admission_identity(
    pool_id: &str,
    lb_pair: &MeteoraLbPairState,
    admission: &MeteoraLiveAdmissionState,
) -> Result<(), String> {
    let lb_pair_pubkey = decode_pubkey(pool_id)?;
    let (reserve_x, _) = derive_reserve_pda(lb_pair_pubkey, lb_pair.mint_x)?;
    let (reserve_y, _) = derive_reserve_pda(lb_pair_pubkey, lb_pair.mint_y)?;
    let (oracle, _) = derive_oracle_pda(lb_pair_pubkey)?;

    if admission.reserve_x != reserve_x {
        return Err("Meteora reserve_x does not match canonical PDA".to_owned());
    }
    if admission.reserve_y != reserve_y {
        return Err("Meteora reserve_y does not match canonical PDA".to_owned());
    }
    if admission.oracle != oracle {
        return Err("Meteora oracle does not match canonical PDA".to_owned());
    }
    Ok(())
}

fn verify_stable_identity(
    observation: &MeteoraLiveObservation,
    snapshot: &MeteoraLbPairState,
    admission: &MeteoraLiveAdmissionState,
) -> Result<(), String> {
    if snapshot.mint_x != observation.lb_pair.mint_x {
        return Err("Meteora hydration mint_x changed".to_owned());
    }
    if snapshot.mint_y != observation.lb_pair.mint_y {
        return Err("Meteora hydration mint_y changed".to_owned());
    }
    if admission.function_type != observation.admission.function_type {
        return Err("Meteora hydration function_type changed".to_owned());
    }
    if admission.pair_type != observation.admission.pair_type {
        return Err("Meteora hydration pair_type changed".to_owned());
    }
    if admission.reward_mint_0 != observation.admission.reward_mint_0
        || admission.reward_mint_1 != observation.admission.reward_mint_1
    {
        return Err("Meteora hydration reward profile changed".to_owned());
    }
    if admission.reserve_x != observation.admission.reserve_x {
        return Err("Meteora hydration reserve_x changed".to_owned());
    }
    if admission.reserve_y != observation.admission.reserve_y {
        return Err("Meteora hydration reserve_y changed".to_owned());
    }
    if admission.oracle != observation.admission.oracle {
        return Err("Meteora hydration oracle changed".to_owned());
    }

    validate_admission_identity(&observation.pubkey, snapshot, admission)
}

fn parse_optional_bitmap_extension(
    account: &Value,
    observation: &MeteoraLiveObservation,
) -> Result<Option<MeteoraBitmapExtensionState>, String> {
    if account.is_null() {
        return Ok(None);
    }

    let data =
        decode_required_rpc_account(account, METEORA_DLMM_PROGRAM_ID, "Meteora bitmap extension")?;
    if data.len() != BITMAP_EXTENSION_ACCOUNT_LEN {
        return Err(format!(
            "Meteora bitmap extension length mismatch: expected {}, got {}",
            BITMAP_EXTENSION_ACCOUNT_LEN,
            data.len()
        ));
    }
    let extension = decode_bitmap_extension(METEORA_DLMM_PROGRAM_ID, &data)
        .map_err(|error| format!("Meteora bitmap extension decode failed: {error:?}"))?;
    if extension.lb_pair != decode_pubkey(&observation.pubkey)? {
        return Err("Meteora bitmap extension points to a different LB pair".to_owned());
    }
    Ok(Some(extension))
}

fn normalized_pool(
    observation: &MeteoraLiveObservation,
    admission: &MeteoraLiveAdmissionState,
    token_x_decimals: u8,
    token_y_decimals: u8,
    clock: MeteoraClockSnapshot,
    source_slot: u64,
    timing: (u64, u64),
) -> Result<NormalizedPoolState, String> {
    Ok(NormalizedPoolState {
        pool_id: observation.pubkey.clone(),
        venue: Venue::Meteora,
        program_id: observation.owner.clone(),
        source_slot,
        token_a: NormalizedToken {
            mint: bs58::encode(observation.lb_pair.mint_x).into_string(),
            vault: bs58::encode(admission.reserve_x).into_string(),
            decimals: token_x_decimals,
        },
        token_b: NormalizedToken {
            mint: bs58::encode(observation.lb_pair.mint_y).into_string(),
            vault: bs58::encode(admission.reserve_y).into_string(),
            decimals: token_y_decimals,
        },
        trading_state: trading_state(admission, clock)?,
        quote_reserves: QuoteReserveState::Unavailable,
        account_update_received_at_unix_ms: timing.0,
        normalized_at_unix_ms: timing.1,
    })
}

fn trading_state(
    admission: &MeteoraLiveAdmissionState,
    clock: MeteoraClockSnapshot,
) -> Result<PoolTradingState, String> {
    if admission.status == PAIR_STATUS_DISABLED {
        return Ok(PoolTradingState::SwapDisabled);
    }
    if admission.status != PAIR_STATUS_ENABLED {
        return Err(format!("invalid Meteora pair status {}", admission.status));
    }

    match admission.pair_type {
        PAIR_TYPE_PERMISSIONLESS | PAIR_TYPE_PERMISSIONLESS_V2 => {
            return Ok(PoolTradingState::Tradable);
        }
        PAIR_TYPE_PERMISSION | PAIR_TYPE_CUSTOMIZABLE_PERMISSIONLESS => {}
        value => return Err(format!("unsupported Meteora pair_type {value}")),
    }

    if admission.activation_point == 0 {
        return Ok(PoolTradingState::Tradable);
    }

    let current_point = match admission.activation_type {
        ACTIVATION_TYPE_SLOT => clock.slot,
        ACTIVATION_TYPE_TIMESTAMP => u64::try_from(clock.unix_timestamp)
            .map_err(|_| "Meteora Clock unix_timestamp is negative".to_owned())?,
        value => return Err(format!("unsupported Meteora activation_type {value}")),
    };

    if current_point >= admission.activation_point {
        Ok(PoolTradingState::Tradable)
    } else {
        Ok(PoolTradingState::NotYetOpen)
    }
}

fn parse_mint_account(
    account: &Value,
    expected_program_flag: u8,
    label: &str,
) -> Result<(String, u8), String> {
    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing owner"))?;
    let expected_owner = token_program_from_flag(expected_program_flag)?;
    if owner != expected_owner {
        return Err(format!(
            "{label} owner mismatch: flag={expected_program_flag} expected={expected_owner} got={owner}"
        ));
    }

    let data = decode_account_data(account, label)?;
    if data.len() < MINT_BASE_LEN {
        return Err(format!(
            "{label} shorter than Mint base layout: expected at least {MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }
    if read_u8(&data, MINT_INITIALIZED_OFFSET, label)? != 1 {
        return Err(format!("{label} is not initialized"));
    }
    if owner == SPL_TOKEN_PROGRAM_ID && data.len() != MINT_BASE_LEN {
        return Err(format!(
            "{label} legacy SPL Mint length mismatch: expected {MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }
    if owner == TOKEN_2022_PROGRAM_ID {
        ensure_supported_token_2022_mint_extensions(&data, label)?;
    }

    Ok((
        owner.to_owned(),
        read_u8(&data, MINT_DECIMALS_OFFSET, label)?,
    ))
}

fn token_program_from_flag(flag: u8) -> Result<&'static str, String> {
    match flag {
        TOKEN_PROGRAM_FLAG_SPL => Ok(SPL_TOKEN_PROGRAM_ID),
        TOKEN_PROGRAM_FLAG_2022 => Ok(TOKEN_2022_PROGRAM_ID),
        _ => Err(format!("unsupported Meteora token program flag {flag}")),
    }
}

fn validate_token_program_flag(flag: u8, label: &str) -> Result<(), String> {
    token_program_from_flag(flag)
        .map(|_| ())
        .map_err(|error| format!("{label} {error}"))
}

fn require_initial_directional_hydration(snapshot: &MeteoraDlmmSnapshot) -> Result<(), String> {
    let mut hydrated_target_count = 0usize;

    for swap_for_y in [true, false] {
        let plan = meteora_initial_hydration_plan(snapshot, swap_for_y)
            .map_err(|error| format!("Meteora final hydration plan failed: {error:?}"))?;
        for target in plan.targets {
            let hydrated = snapshot
                .bin_array_by_index(target.index)
                .map_err(|error| format!("Meteora final BinArray lookup failed: {error:?}"))?;
            if hydrated.is_none() {
                return Err(format!(
                    "Meteora coherent refresh changed initial BinArray target: index={}",
                    target.index
                ));
            }
            hydrated_target_count = hydrated_target_count
                .checked_add(1)
                .ok_or_else(|| "Meteora hydrated target count overflow".to_owned())?;
        }
    }

    if hydrated_target_count == 0 {
        return Err("Meteora pool has no initialized directional BinArray target".to_owned());
    }
    Ok(())
}

fn hydration_response_slot(payload: &Value, min_slot: u64, label: &str) -> Result<u64, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!("{label} RPC error: {error}"));
    }
    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label} missing context slot"))?;
    if slot < min_slot {
        return Err(format!(
            "stale {label}: minimum_slot={min_slot} snapshot_slot={slot}"
        ));
    }
    Ok(slot)
}

fn hydration_response_accounts<'a>(
    payload: &'a Value,
    expected_count: usize,
    label: &str,
) -> Result<&'a [Value], String> {
    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label} missing account array"))?;
    if accounts.len() != expected_count {
        return Err(format!(
            "{label} expected {expected_count} accounts, got {}",
            accounts.len()
        ));
    }
    Ok(accounts)
}

fn decode_required_rpc_account(
    account: &Value,
    expected_owner: &str,
    label: &str,
) -> Result<Vec<u8>, String> {
    if account.is_null() {
        return Err(format!("{label} account is missing"));
    }
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

fn decode_account_data(account: &Value, label: &str) -> Result<Vec<u8>, String> {
    let encoded = account
        .pointer("/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing base64 data"))?;
    let encoding = account
        .pointer("/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing data encoding"))?;
    if encoding != "base64" {
        return Err(format!("{label} unexpected data encoding {encoding}"));
    }
    BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("{label} invalid base64 data: {error}"))
}

fn decode_clock(data: &[u8]) -> Result<MeteoraClockSnapshot, String> {
    if data.len() != CLOCK_DATA_LEN {
        return Err(format!(
            "Meteora Clock length mismatch: expected {CLOCK_DATA_LEN}, got {}",
            data.len()
        ));
    }

    Ok(MeteoraClockSnapshot {
        slot: read_u64(data, CLOCK_SLOT_OFFSET, "Clock slot")?,
        epoch_start_timestamp: read_i64(
            data,
            CLOCK_EPOCH_START_TIMESTAMP_OFFSET,
            "Clock epoch_start_timestamp",
        )?,
        epoch: read_u64(data, CLOCK_EPOCH_OFFSET, "Clock epoch")?,
        leader_schedule_epoch: read_u64(
            data,
            CLOCK_LEADER_SCHEDULE_EPOCH_OFFSET,
            "Clock leader_schedule_epoch",
        )?,
        unix_timestamp: read_i64(data, CLOCK_UNIX_TIMESTAMP_OFFSET, "Clock unix_timestamp")?,
    })
}

fn derive_reserve_pda(lb_pair: [u8; 32], mint: [u8; 32]) -> Result<([u8; 32], u8), String> {
    Pubkey::try_find_program_address(&[&lb_pair, &mint], &METEORA_DLMM_PROGRAM_PUBKEY)
        .map(|(pubkey, bump)| (pubkey.to_bytes(), bump))
        .ok_or_else(|| "could not derive Meteora reserve PDA".to_owned())
}

fn derive_oracle_pda(lb_pair: [u8; 32]) -> Result<([u8; 32], u8), String> {
    Pubkey::try_find_program_address(&[ORACLE_SEED, &lb_pair], &METEORA_DLMM_PROGRAM_PUBKEY)
        .map(|(pubkey, bump)| (pubkey.to_bytes(), bump))
        .ok_or_else(|| "could not derive Meteora Oracle PDA".to_owned())
}

fn derive_bitmap_extension(lb_pair: [u8; 32]) -> ([u8; 32], u8) {
    let (pubkey, bump) = Pubkey::find_program_address(
        &[BIN_ARRAY_BITMAP_SEED, &lb_pair],
        &METEORA_DLMM_PROGRAM_PUBKEY,
    );
    (pubkey.to_bytes(), bump)
}

fn decode_pubkey(encoded: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(encoded)
        .into_vec()
        .map_err(|error| format!("invalid Solana pubkey {encoded}: {error}"))?;
    let decoded_len = decoded.len();
    decoded.try_into().map_err(|_| {
        format!("invalid Solana pubkey length: value={encoded} decoded_len={decoded_len}")
    })
}

fn read_u8(data: &[u8], offset: usize, label: &str) -> Result<u8, String> {
    data.get(offset)
        .copied()
        .ok_or_else(|| format!("{label} outside account data"))
}

fn read_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    Ok(u64::from_le_bytes(read_array::<8>(data, offset, label)?))
}

fn read_i64(data: &[u8], offset: usize, label: &str) -> Result<i64, String> {
    Ok(i64::from_le_bytes(read_array::<8>(data, offset, label)?))
}

fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("{label} offset overflow"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("{label} outside account data"))?;
    <[u8; N]>::try_from(bytes).map_err(|_| format!("{label} had invalid length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn canonical_reserve_and_oracle_pdas_are_distinct() -> Result<(), String> {
        let pool = [7_u8; 32];
        let mint_x = [8_u8; 32];
        let mint_y = [9_u8; 32];
        let reserve_x = derive_reserve_pda(pool, mint_x)?.0;
        let reserve_y = derive_reserve_pda(pool, mint_y)?.0;
        let oracle = derive_oracle_pda(pool)?.0;

        assert_ne!(reserve_x, reserve_y);
        assert_ne!(reserve_x, oracle);
        assert_ne!(reserve_y, oracle);
        Ok(())
    }

    #[test]
    fn trading_state_fails_closed_for_disabled_pair() -> Result<(), String> {
        let admission = test_admission(PAIR_STATUS_DISABLED, ACTIVATION_TYPE_SLOT, 0);
        let state = trading_state(&admission, test_clock(100, 1_000))?;

        assert_eq!(state, PoolTradingState::SwapDisabled);
        Ok(())
    }

    #[test]
    fn permission_pair_honors_slot_activation() -> Result<(), String> {
        let mut admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_SLOT, 101);
        admission.pair_type = PAIR_TYPE_PERMISSION;
        let state = trading_state(&admission, test_clock(100, 1_000))?;

        assert_eq!(state, PoolTradingState::NotYetOpen);
        Ok(())
    }

    #[test]
    fn customizable_pair_honors_timestamp_activation() -> Result<(), String> {
        let mut admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_TIMESTAMP, 1_000);
        admission.pair_type = PAIR_TYPE_CUSTOMIZABLE_PERMISSIONLESS;
        let state = trading_state(&admission, test_clock(100, 1_000))?;

        assert_eq!(state, PoolTradingState::Tradable);
        Ok(())
    }

    #[test]
    fn permissionless_pair_ignores_future_activation_point() -> Result<(), String> {
        let admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_SLOT, 101);
        let state = trading_state(&admission, test_clock(100, 1_000))?;

        assert_eq!(state, PoolTradingState::Tradable);
        Ok(())
    }

    #[test]
    fn permissionless_v2_ignores_activation_point() -> Result<(), String> {
        let mut admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_SLOT, 101);
        admission.pair_type = PAIR_TYPE_PERMISSIONLESS_V2;

        validate_admission_values(&admission)?;
        let state = trading_state(&admission, test_clock(100, 1_000))?;

        assert_eq!(state, PoolTradingState::Tradable);
        Ok(())
    }

    #[test]
    fn function_profile_matches_pinned_v0_12_limit_order_rules() -> Result<(), String> {
        let mut admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_SLOT, 0);

        admission.function_type = FUNCTION_TYPE_LIMIT_ORDER;
        admission.reward_mint_0 = [7_u8; 32];
        admission.reward_mint_1 = [8_u8; 32];
        assert!(supports_limit_order_profile(&admission)?);

        admission.function_type = FUNCTION_TYPE_LIQUIDITY_MINING;
        assert!(!supports_limit_order_profile(&admission)?);

        admission.function_type = FUNCTION_TYPE_UNDETERMINED;
        admission.reward_mint_0 = [0_u8; 32];
        admission.reward_mint_1 = [0_u8; 32];
        assert!(supports_limit_order_profile(&admission)?);

        admission.reward_mint_1 = [9_u8; 32];
        assert!(!supports_limit_order_profile(&admission)?);

        admission.function_type = 3;
        assert!(supports_limit_order_profile(&admission).is_err());
        Ok(())
    }

    #[test]
    fn admission_rejects_unknown_pair_type() {
        let mut admission = test_admission(PAIR_STATUS_ENABLED, ACTIVATION_TYPE_SLOT, 0);
        admission.pair_type = 4;

        assert!(matches!(
            validate_admission_values(&admission),
            Err(error) if error.contains("unsupported Meteora pair_type 4")
        ));
    }

    #[test]
    fn token_program_flags_map_only_to_locked_programs() {
        assert_eq!(
            token_program_from_flag(TOKEN_PROGRAM_FLAG_SPL),
            Ok(SPL_TOKEN_PROGRAM_ID)
        );
        assert_eq!(
            token_program_from_flag(TOKEN_PROGRAM_FLAG_2022),
            Ok(TOKEN_2022_PROGRAM_ID)
        );
        assert!(token_program_from_flag(2).is_err());
    }

    fn test_admission(
        status: u8,
        activation_type: u8,
        activation_point: u64,
    ) -> MeteoraLiveAdmissionState {
        MeteoraLiveAdmissionState {
            function_type: FUNCTION_TYPE_LIMIT_ORDER,
            pair_type: PAIR_TYPE_PERMISSIONLESS,
            status,
            activation_type,
            creator_pool_on_off_control: 0,
            reserve_x: [1_u8; 32],
            reserve_y: [2_u8; 32],
            reward_mint_0: [0_u8; 32],
            reward_mint_1: [0_u8; 32],
            oracle: [3_u8; 32],
            activation_point,
            token_x_program_flag: TOKEN_PROGRAM_FLAG_SPL,
            token_y_program_flag: TOKEN_PROGRAM_FLAG_SPL,
            version: 0,
        }
    }

    fn test_clock(slot: u64, unix_timestamp: i64) -> MeteoraClockSnapshot {
        MeteoraClockSnapshot {
            slot,
            epoch_start_timestamp: 0,
            epoch: 0,
            leader_schedule_epoch: 0,
            unix_timestamp,
        }
    }
}
