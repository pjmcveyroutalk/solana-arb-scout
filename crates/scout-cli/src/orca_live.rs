#![allow(dead_code)]

use crate::orca;
use crate::orca_o2;
use crate::orca_o2::{OrcaQuoteAccount, OrcaQuoteSnapshotInputs};
use crate::quote::{
    orca_quote_readiness_for_pool, OrcaQuoteReadinessEvidence, OrcaQuoteSnapshot, QuoteReadiness,
};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use reqwest::Client;
use scout_core::NormalizedPoolState;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};

const CLOCK_SYSVAR_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const READINESS_QUOTE_AMOUNT_RAW: u64 = 1_000_000;

const WHIRLPOOL_INDEX: usize = 0;
const MINT_A_INDEX: usize = 1;
const MINT_B_INDEX: usize = 2;
const TICK_ARRAY_START_INDEX: usize = 3;
const CLOCK_INDEX: usize = 8;

#[derive(Debug, Clone)]
struct OrcaSnapshotPlan {
    pubkeys: Vec<String>,
    tick_array_start_indexes: [i32; 5],
}

#[derive(Debug, Clone)]
struct DecodedRpcAccount {
    owner: String,
    data: Vec<u8>,
}

pub struct PreparedOrca {
    pub normalized: NormalizedPoolState,
    pub readiness: QuoteReadiness,
    pub quote_snapshot: OrcaQuoteSnapshot,
    pub anchor_mint: String,
    pub intermediate_mint: String,
    pub priority_contention_accounts: [String; 9],
}

pub fn anchor_pair(pool: &orca::OrcaWhirlpoolState) -> Option<(&str, &str)> {
    for anchor in [WRAPPED_SOL_MINT, USDC_MINT, USDT_MINT] {
        if pool.token_mint_a == anchor && pool.token_mint_b != anchor {
            return Some((anchor, pool.token_mint_b.as_str()));
        }

        if pool.token_mint_b == anchor && pool.token_mint_a != anchor {
            return Some((anchor, pool.token_mint_a.as_str()));
        }
    }

    None
}

pub async fn prepare_orca(
    rpc_client: &Client,
    rpc_url: &str,
    observation: &orca::OrcaWhirlpoolAccountObservation,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> Result<PreparedOrca, String> {
    let plan = build_orca_snapshot_plan(observation)?;

    let payload = fetch_multiple_accounts(
        rpc_client,
        rpc_url,
        40,
        &plan.pubkeys,
        observation.slot,
        "Orca O2 hydration",
    )
    .await?;

    let snapshot_slot = response_slot(&payload, "Orca O2")?;
    let accounts = response_accounts(&payload, "Orca O2")?;

    if accounts.len() != 9 {
        return Err(format!(
            "Orca O2 expected exactly 9 accounts, got {}",
            accounts.len()
        ));
    }

    require_present(accounts, WHIRLPOOL_INDEX, "Whirlpool")?;
    require_present(accounts, MINT_A_INDEX, "mint A")?;
    require_present(accounts, MINT_B_INDEX, "mint B")?;
    require_present(accounts, CLOCK_INDEX, "Clock")?;

    let whirlpool_account = decode_required_account(
        accounts,
        WHIRLPOOL_INDEX,
        orca::ORCA_WHIRLPOOL_PROGRAM_ID,
        "Orca Whirlpool",
    )?;

    let snapshot_pool = orca_o2::decode_whirlpool_state(&whirlpool_account.data)?;

    orca_o2::verify_stable_pool_identity(&observation.pool_state, &snapshot_pool)?;

    let snapshot_window = orca_o2::bounded_tick_array_start_indexes(&snapshot_pool)?;

    if snapshot_window != plan.tick_array_start_indexes {
        return Err(format!(
            "Orca tick-array window changed: trigger={:?} snapshot={:?}",
            plan.tick_array_start_indexes, snapshot_window
        ));
    }

    let mint_a = decode_required_account_any_owner(accounts, MINT_A_INDEX, "Orca mint A")?;
    let mint_b = decode_required_account_any_owner(accounts, MINT_B_INDEX, "Orca mint B")?;
    let clock = decode_required_account_any_owner(accounts, CLOCK_INDEX, "Orca Clock")?;

    let quote_inputs = OrcaQuoteSnapshotInputs {
        clock: OrcaQuoteAccount {
            pubkey: CLOCK_SYSVAR_ID,
            owner: &clock.owner,
            data: &clock.data,
        },
        mint_a: OrcaQuoteAccount {
            pubkey: &snapshot_pool.token_mint_a,
            owner: &mint_a.owner,
            data: &mint_a.data,
        },
        mint_b: OrcaQuoteAccount {
            pubkey: &snapshot_pool.token_mint_b,
            owner: &mint_b.owner,
            data: &mint_b.data,
        },
    };

    let resolved = orca_o2::resolve_quote_snapshot_inputs(&snapshot_pool, quote_inputs)?;

    let quote_a_to_b = orca_o2::quote_exact_input(
        &snapshot_pool,
        orca_o2::decode_whirlpool_facade(&whirlpool_account.data)?,
        &snapshot_pool.token_mint_a,
        READINESS_QUOTE_AMOUNT_RAW,
        decode_tick_arrays(accounts, observation, &plan)?,
        resolved.clock.unix_timestamp,
        None,
        resolved.transfer_fee_a,
        resolved.transfer_fee_b,
    )?;

    let quote_b_to_a = orca_o2::quote_exact_input(
        &snapshot_pool,
        orca_o2::decode_whirlpool_facade(&whirlpool_account.data)?,
        &snapshot_pool.token_mint_b,
        READINESS_QUOTE_AMOUNT_RAW,
        decode_tick_arrays(accounts, observation, &plan)?,
        resolved.clock.unix_timestamp,
        None,
        resolved.transfer_fee_a,
        resolved.transfer_fee_b,
    )?;

    let base_hydration_payload = json!({
        "jsonrpc": "2.0",
        "result": {
            "context": {
                "slot": snapshot_slot
            },
            "value": [
                accounts[WHIRLPOOL_INDEX].clone(),
                accounts[MINT_A_INDEX].clone(),
                accounts[MINT_B_INDEX].clone()
            ]
        },
        "id": 41
    });

    let base_hydration = orca::parse_hydration_response(observation, &base_hydration_payload)?;

    let now = unix_time_ms_now()?;

    let normalized = orca::hydrate_normalized_observation(observation, &base_hydration, now, now)?;

    let evidence = OrcaQuoteReadinessEvidence::from_o2_quotes(
        &observation.pubkey,
        &snapshot_pool.token_mint_a,
        &snapshot_pool.token_mint_b,
        snapshot_slot,
        quote_a_to_b,
        quote_b_to_a,
    )?;

    let readiness = orca_quote_readiness_for_pool(&normalized, &evidence)?;

    let quote_snapshot = OrcaQuoteSnapshot::from_o2_hydration(
        &normalized,
        &evidence,
        snapshot_slot,
        base_hydration.token_a_decimals,
        base_hydration.token_b_decimals,
        orca_o2::decode_whirlpool_facade(&whirlpool_account.data)?,
        decode_tick_arrays(accounts, observation, &plan)?,
        resolved.clock.unix_timestamp,
        None,
        resolved.transfer_fee_a,
        resolved.transfer_fee_b,
    )?;

    let priority_contention_accounts =
        build_priority_contention_accounts(observation, &snapshot_pool, &plan)?;

    println!(
        concat!(
            "orca_priority_contention_ready: pool={} account_count={} accounts=[{}] ",
            "provenance=Orca SwapV2 deterministic protocol writable subset"
        ),
        observation.pubkey,
        priority_contention_accounts.len(),
        priority_contention_accounts.join(",")
    );

    println!(
        concat!(
            "orca_o2_ready: pool={} trigger_slot={} snapshot_slot={} ",
            "anchor={} intermediate={}"
        ),
        observation.pubkey, observation.slot, snapshot_slot, anchor_mint, intermediate_mint
    );

    Ok(PreparedOrca {
        normalized,
        readiness,
        quote_snapshot,
        anchor_mint: anchor_mint.to_owned(),
        intermediate_mint: intermediate_mint.to_owned(),
        priority_contention_accounts,
    })
}

fn build_priority_contention_accounts(
    observation: &orca::OrcaWhirlpoolAccountObservation,
    snapshot_pool: &orca::OrcaWhirlpoolState,
    plan: &OrcaSnapshotPlan,
) -> Result<[String; 9], String> {
    Ok([
        observation.pubkey.clone(),
        snapshot_pool.token_vault_a.clone(),
        snapshot_pool.token_vault_b.clone(),
        plan.pubkeys[TICK_ARRAY_START_INDEX].clone(),
        plan.pubkeys[TICK_ARRAY_START_INDEX + 1].clone(),
        plan.pubkeys[TICK_ARRAY_START_INDEX + 2].clone(),
        plan.pubkeys[TICK_ARRAY_START_INDEX + 3].clone(),
        plan.pubkeys[TICK_ARRAY_START_INDEX + 4].clone(),
        orca_o2::oracle_pda(&observation.pubkey)?,
    ])
}

fn build_orca_snapshot_plan(
    observation: &orca::OrcaWhirlpoolAccountObservation,
) -> Result<OrcaSnapshotPlan, String> {
    if observation.pool_state.is_adaptive_fee() {
        return Err("Orca live preparation currently admits only non-adaptive O2 pools".to_owned());
    }

    let tick_array_start_indexes =
        orca_o2::bounded_tick_array_start_indexes(&observation.pool_state)?;

    let mut pubkeys = Vec::with_capacity(9);

    pubkeys.push(observation.pubkey.clone());
    pubkeys.push(observation.pool_state.token_mint_a.clone());
    pubkeys.push(observation.pool_state.token_mint_b.clone());

    for start_tick_index in tick_array_start_indexes {
        pubkeys.push(orca_o2::tick_array_pda(
            &observation.pubkey,
            start_tick_index,
        )?);
    }

    pubkeys.push(CLOCK_SYSVAR_ID.to_owned());

    Ok(OrcaSnapshotPlan {
        pubkeys,
        tick_array_start_indexes,
    })
}

fn decode_tick_arrays(
    accounts: &[Value],
    observation: &orca::OrcaWhirlpoolAccountObservation,
    plan: &OrcaSnapshotPlan,
) -> Result<[orca_whirlpools_core::TickArrayFacade; 5], String> {
    Ok([
        decode_tick_array_or_zero(
            accounts,
            TICK_ARRAY_START_INDEX,
            &observation.pubkey,
            plan.tick_array_start_indexes[0],
        )?,
        decode_tick_array_or_zero(
            accounts,
            TICK_ARRAY_START_INDEX + 1,
            &observation.pubkey,
            plan.tick_array_start_indexes[1],
        )?,
        decode_tick_array_or_zero(
            accounts,
            TICK_ARRAY_START_INDEX + 2,
            &observation.pubkey,
            plan.tick_array_start_indexes[2],
        )?,
        decode_tick_array_or_zero(
            accounts,
            TICK_ARRAY_START_INDEX + 3,
            &observation.pubkey,
            plan.tick_array_start_indexes[3],
        )?,
        decode_tick_array_or_zero(
            accounts,
            TICK_ARRAY_START_INDEX + 4,
            &observation.pubkey,
            plan.tick_array_start_indexes[4],
        )?,
    ])
}

async fn fetch_multiple_accounts<T>(
    rpc_client: &Client,
    rpc_url: &str,
    request_id: u64,
    pubkeys: &[T],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String>
where
    T: AsRef<str>,
{
    let keys = pubkeys.iter().map(|key| key.as_ref()).collect::<Vec<_>>();

    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getMultipleAccounts",
        "params": [
            keys,
            {
                "commitment": "processed",
                "encoding": "base64",
                "minContextSlot": min_context_slot
            }
        ]
    });

    post_rpc(rpc_client, rpc_url, &request, label).await
}

async fn post_rpc(
    rpc_client: &Client,
    rpc_url: &str,
    request: &Value,
    label: &str,
) -> Result<Value, String> {
    let response = rpc_client
        .post(rpc_url)
        .json(request)
        .send()
        .await
        .map_err(|error| format!("{label} RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!("{label} RPC returned HTTP status {status}"));
    }

    response
        .json::<Value>()
        .await
        .map_err(|error| format!("{label} returned invalid JSON: {error}"))
}

fn response_slot(payload: &Value, label: &str) -> Result<u64, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!("{label} RPC error: {error}"));
    }

    payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label} response missing context slot"))
}

fn response_accounts<'a>(payload: &'a Value, label: &str) -> Result<&'a Vec<Value>, String> {
    payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label} response missing account array"))
}

fn require_present(accounts: &[Value], index: usize, label: &str) -> Result<(), String> {
    let account = accounts
        .get(index)
        .ok_or_else(|| format!("Orca {label} account index missing"))?;

    if account.is_null() {
        return Err(format!("Orca required {label} account is missing"));
    }

    Ok(())
}

fn decode_required_account(
    accounts: &[Value],
    index: usize,
    expected_owner: &str,
    label: &str,
) -> Result<DecodedRpcAccount, String> {
    let decoded = decode_required_account_any_owner(accounts, index, label)?;

    if decoded.owner != expected_owner {
        return Err(format!(
            "{label} owner mismatch: expected {expected_owner}, got {}",
            decoded.owner
        ));
    }

    Ok(decoded)
}

fn decode_required_account_any_owner(
    accounts: &[Value],
    index: usize,
    label: &str,
) -> Result<DecodedRpcAccount, String> {
    let account = accounts
        .get(index)
        .ok_or_else(|| format!("{label} account index missing"))?;

    if account.is_null() {
        return Err(format!("{label} account is missing"));
    }

    decode_rpc_account(account, label)
}

fn decode_tick_array_or_zero(
    accounts: &[Value],
    index: usize,
    whirlpool: &str,
    expected_start_tick_index: i32,
) -> Result<orca_whirlpools_core::TickArrayFacade, String> {
    let account = accounts
        .get(index)
        .ok_or_else(|| format!("Orca tick-array account index {index} missing"))?;

    if account.is_null() {
        return Ok(orca_o2::zeroed_tick_array(expected_start_tick_index));
    }

    let decoded = decode_rpc_account(account, "Orca tick array")?;

    orca_o2::decode_tick_array_account(
        &decoded.data,
        &decoded.owner,
        whirlpool,
        expected_start_tick_index,
    )
}

fn decode_rpc_account(account: &Value, label: &str) -> Result<DecodedRpcAccount, String> {
    let owner = account
        .get("owner")
        .and_then(Value::as_str)
        .ok_or_else(|| format!("{label} missing owner"))?
        .to_owned();

    let data = account
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| format!("{label} missing account data array"))?;

    if data.len() != 2 {
        return Err(format!(
            "{label} account data expected exactly 2 elements, got {}",
            data.len()
        ));
    }

    let encoded = data[0]
        .as_str()
        .ok_or_else(|| format!("{label} account data payload is not a string"))?;

    let encoding = data[1]
        .as_str()
        .ok_or_else(|| format!("{label} account data encoding is not a string"))?;

    if encoding != "base64" {
        return Err(format!(
            "{label} account data encoding mismatch: expected base64, got {encoding}"
        ));
    }

    let decoded = BASE64_STANDARD
        .decode(encoded)
        .map_err(|error| format!("{label} invalid base64 account data: {error}"))?;

    Ok(DecodedRpcAccount {
        owner,
        data: decoded,
    })
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?;

    u64::try_from(duration.as_millis())
        .map_err(|_| "Unix millisecond timestamp overflow".to_owned())
}
