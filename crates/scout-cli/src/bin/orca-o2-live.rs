#![allow(dead_code)]

#[path = "../orca.rs"]
mod orca;
#[path = "../orca_o2.rs"]
mod orca_o2;
#[path = "../orca_o2_quote_inputs.rs"]
mod orca_o2_quote_inputs;

use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use futures_util::{SinkExt, StreamExt};
use orca_o2::{OrcaQuoteAccount, OrcaQuoteSnapshotInputs};
use reqwest::Client;
use serde_json::{json, Value};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const SOLANA_WS_URL: &str = "wss://api.mainnet-beta.solana.com";

const CLOCK_SYSVAR_ID: &str = "SysvarC1ock11111111111111111111111111111111";

const ORCA_O2_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ORCA_OBSERVATIONS: usize = 25;
const QUOTE_AMOUNT_RAW: u64 = 1_000_000;

const WHIRLPOOL_INDEX: usize = 0;
const MINT_A_INDEX: usize = 1;
const MINT_B_INDEX: usize = 2;
const TICK_ARRAY_START_INDEX: usize = 3;
const CLOCK_INDEX: usize = 8;
const ORACLE_INDEX: usize = 9;

#[derive(Debug, Clone)]
struct OrcaO2SnapshotPlan {
    pubkeys: Vec<String>,
    tick_array_start_indexes: [i32; 5],
    adaptive_fee: bool,
}

#[derive(Debug, Clone)]
struct DecodedRpcAccount {
    owner: String,
    data: Vec<u8>,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    match timeout(ORCA_O2_TOTAL_TIMEOUT, run_orca_o2()).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "Orca O2 live parity exceeded {} seconds",
            ORCA_O2_TOTAL_TIMEOUT.as_secs()
        )),
    }
}

async fn run_orca_o2() -> Result<(), String> {
    let rpc_client = Client::builder()
        .connect_timeout(RPC_CONNECT_TIMEOUT)
        .timeout(RPC_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("could not build bounded Solana RPC client: {error}"))?;

    let request = SOLANA_WS_URL
        .into_client_request()
        .map_err(|error| format!("invalid Solana WebSocket request: {error}"))?;

    let (mut websocket, _) = connect_async(request)
        .await
        .map_err(|error| format!("could not connect to Solana WebSocket: {error}"))?;

    websocket
        .send(Message::Text(orca::program_subscribe_request().to_string()))
        .await
        .map_err(|error| format!("could not subscribe to Orca Whirlpool: {error}"))?;

    wait_for_subscription_confirmation(&mut websocket).await?;

    println!("Orca O2 live read-only deterministic parity");
    println!("No routing admission, signing, submission, borrowing, or execution capability.");

    let mut observed = 0usize;

    while observed < MAX_ORCA_OBSERVATIONS {
        let payload = next_json_message(&mut websocket).await?;

        let observation = match orca::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("orca_o2_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "orca_o2_observation: pool={} slot={} {}",
            observation.pubkey,
            observation.slot,
            observation.pool_state.summary()
        );

        let plan = match build_snapshot_plan(&observation) {
            Ok(plan) => plan,
            Err(error) => {
                println!(
                    "orca_o2_plan_rejected: pool={} reason={error}",
                    observation.pubkey
                );
                continue;
            }
        };

        let hydration_payload = fetch_snapshot(&rpc_client, &observation, &plan).await?;

        match parse_and_quote_snapshot(&observation, &plan, &hydration_payload) {
            Ok(()) => {
                println!("orca_o2_live_observation_count={observed}");
                println!("READ-ONLY ORCA WHIRLPOOL O2 LIVE PARITY PASS");
                return Ok(());
            }
            Err(error) => {
                println!(
                    "orca_o2_snapshot_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    Err(format!(
        "Orca O2 observed {observed} Whirlpool updates without one completing live parity"
    ))
}

fn build_snapshot_plan(
    observation: &orca::OrcaWhirlpoolAccountObservation,
) -> Result<OrcaO2SnapshotPlan, String> {
    let tick_array_start_indexes =
        orca_o2::bounded_tick_array_start_indexes(&observation.pool_state)?;

    let mut pubkeys = Vec::with_capacity(if observation.pool_state.is_adaptive_fee() {
        10
    } else {
        9
    });

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

    let adaptive_fee = observation.pool_state.is_adaptive_fee();

    if adaptive_fee {
        pubkeys.push(orca_o2::oracle_pda(&observation.pubkey)?);
    }

    Ok(OrcaO2SnapshotPlan {
        pubkeys,
        tick_array_start_indexes,
        adaptive_fee,
    })
}

fn snapshot_request(
    observation: &orca::OrcaWhirlpoolAccountObservation,
    plan: &OrcaO2SnapshotPlan,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 20,
        "method": "getMultipleAccounts",
        "params": [
            plan.pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64",
                "minContextSlot": observation.slot
            }
        ]
    })
}

async fn fetch_snapshot(
    rpc_client: &Client,
    observation: &orca::OrcaWhirlpoolAccountObservation,
    plan: &OrcaO2SnapshotPlan,
) -> Result<Value, String> {
    let request = snapshot_request(observation, plan);

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("Orca O2 hydration RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "Orca O2 hydration RPC returned HTTP status {status}"
        ));
    }

    response
        .json::<Value>()
        .await
        .map_err(|error| format!("invalid Orca O2 hydration RPC JSON: {error}"))
}

fn parse_and_quote_snapshot(
    observation: &orca::OrcaWhirlpoolAccountObservation,
    plan: &OrcaO2SnapshotPlan,
    payload: &Value,
) -> Result<(), String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Solana getMultipleAccounts returned an RPC error: {error}"
        ));
    }

    let snapshot_slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Orca O2 response missing context slot".to_owned())?;

    if snapshot_slot < observation.slot {
        return Err(format!(
            "stale Orca O2 snapshot: trigger_slot={} snapshot_slot={snapshot_slot}",
            observation.slot
        ));
    }

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Orca O2 response missing account array".to_owned())?;

    let expected_count = if plan.adaptive_fee { 10 } else { 9 };

    if accounts.len() != expected_count {
        return Err(format!(
            "Orca O2 expected exactly {expected_count} accounts, got {}",
            accounts.len()
        ));
    }

    require_present(accounts, WHIRLPOOL_INDEX, "Whirlpool")?;
    require_present(accounts, MINT_A_INDEX, "mint A")?;
    require_present(accounts, MINT_B_INDEX, "mint B")?;
    require_present(accounts, CLOCK_INDEX, "Clock")?;

    if plan.adaptive_fee {
        require_present(accounts, ORACLE_INDEX, "Oracle")?;
    }

    let whirlpool_account = decode_required_account(
        accounts,
        WHIRLPOOL_INDEX,
        orca::ORCA_WHIRLPOOL_PROGRAM_ID,
        "Orca O2 Whirlpool",
    )?;

    let snapshot_pool = orca_o2::decode_whirlpool_state(&whirlpool_account.data)?;

    orca_o2::verify_stable_pool_identity(&observation.pool_state, &snapshot_pool)?;

    let snapshot_window = orca_o2::bounded_tick_array_start_indexes(&snapshot_pool)?;

    if snapshot_window != plan.tick_array_start_indexes {
        return Err(format!(
            concat!(
                "Orca O2 tick-array window changed between trigger and snapshot: ",
                "trigger={:?} snapshot={:?}"
            ),
            plan.tick_array_start_indexes, snapshot_window
        ));
    }

    let whirlpool_facade = orca_o2::decode_whirlpool_facade(&whirlpool_account.data)?;

    orca_o2::verify_whirlpool_facade_matches_pool(&snapshot_pool, &whirlpool_facade)?;

    let mint_a = decode_required_account_any_owner(accounts, MINT_A_INDEX, "Orca O2 mint A")?;
    let mint_b = decode_required_account_any_owner(accounts, MINT_B_INDEX, "Orca O2 mint B")?;
    let clock = decode_required_account_any_owner(accounts, CLOCK_INDEX, "Orca O2 Clock")?;

    let tick_arrays = [
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
    ];

    let oracle = if plan.adaptive_fee {
        let oracle_account = decode_required_account(
            accounts,
            ORACLE_INDEX,
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            "Orca O2 Oracle",
        )?;

        Some(orca_o2::decode_oracle_facade(
            &oracle_account.data,
            &oracle_account.owner,
            &observation.pubkey,
        )?)
    } else {
        None
    };

    let quote_snapshot = OrcaQuoteSnapshotInputs {
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

    let resolved = orca_o2::resolve_quote_snapshot_inputs(&snapshot_pool, quote_snapshot)?;

    let quote_a_to_b = orca_o2::quote_exact_input(
        &snapshot_pool,
        whirlpool_facade,
        &snapshot_pool.token_mint_a,
        QUOTE_AMOUNT_RAW,
        tick_arrays,
        resolved.clock.unix_timestamp,
        oracle,
        resolved.transfer_fee_a,
        resolved.transfer_fee_b,
    )?;

    let whirlpool_facade = orca_o2::decode_whirlpool_facade(&whirlpool_account.data)?;

    let tick_arrays = [
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
    ];

    let oracle = if plan.adaptive_fee {
        let oracle_account = decode_required_account(
            accounts,
            ORACLE_INDEX,
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            "Orca O2 Oracle",
        )?;

        Some(orca_o2::decode_oracle_facade(
            &oracle_account.data,
            &oracle_account.owner,
            &observation.pubkey,
        )?)
    } else {
        None
    };

    let quote_b_to_a = orca_o2::quote_exact_input(
        &snapshot_pool,
        whirlpool_facade,
        &snapshot_pool.token_mint_b,
        QUOTE_AMOUNT_RAW,
        tick_arrays,
        resolved.clock.unix_timestamp,
        oracle,
        resolved.transfer_fee_a,
        resolved.transfer_fee_b,
    )?;

    println!(
        concat!(
            "orca_o2_snapshot: pool={} trigger_slot={} snapshot_slot={} ",
            "adaptive_fee={} clock_epoch={} clock_timestamp={}"
        ),
        observation.pubkey,
        observation.slot,
        snapshot_slot,
        plan.adaptive_fee,
        resolved.clock.epoch,
        resolved.clock.unix_timestamp
    );

    println!(
        concat!(
            "orca_o2_quote_a_to_b: token_in={} token_est_out={} ",
            "token_min_out={} trade_fee={} fee_rate_min={} fee_rate_max={}"
        ),
        quote_a_to_b.token_in,
        quote_a_to_b.token_est_out,
        quote_a_to_b.token_min_out,
        quote_a_to_b.trade_fee,
        quote_a_to_b.trade_fee_rate_min,
        quote_a_to_b.trade_fee_rate_max
    );

    println!(
        concat!(
            "orca_o2_quote_b_to_a: token_in={} token_est_out={} ",
            "token_min_out={} trade_fee={} fee_rate_min={} fee_rate_max={}"
        ),
        quote_b_to_a.token_in,
        quote_b_to_a.token_est_out,
        quote_b_to_a.token_min_out,
        quote_b_to_a.trade_fee,
        quote_b_to_a.trade_fee_rate_min,
        quote_b_to_a.trade_fee_rate_max
    );

    Ok(())
}

fn require_present(accounts: &[Value], index: usize, label: &str) -> Result<(), String> {
    let account = accounts
        .get(index)
        .ok_or_else(|| format!("Orca O2 {label} account index missing"))?;

    if account.is_null() {
        return Err(format!("Orca O2 required {label} account is missing"));
    }

    Ok(())
}

fn decode_required_account(
    accounts: &[Value],
    index: usize,
    expected_owner: &str,
    label: &str,
) -> Result<DecodedRpcAccount, String> {
    let account = accounts
        .get(index)
        .ok_or_else(|| format!("{label} account index missing"))?;

    if account.is_null() {
        return Err(format!("{label} account is missing"));
    }

    let decoded = decode_rpc_account(account, label)?;

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
        .ok_or_else(|| format!("Orca O2 tick-array account index {index} missing"))?;

    if account.is_null() {
        return Ok(orca_o2::zeroed_tick_array(expected_start_tick_index));
    }

    let decoded = decode_rpc_account(account, "Orca O2 tick array")?;

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

async fn wait_for_subscription_confirmation<S>(websocket: &mut S) -> Result<(), String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let payload = next_json_message(websocket).await?;

        if payload.get("id").and_then(Value::as_u64) != Some(18) {
            continue;
        }

        if let Some(error) = payload.get("error") {
            return Err(format!(
                "Orca program subscription returned an RPC error: {error}"
            ));
        }

        payload
            .get("result")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Orca program subscription confirmation missing result".to_owned())?;

        println!("orca_program_subscription_confirmed");

        return Ok(());
    }
}

async fn next_json_message<S>(websocket: &mut S) -> Result<Value, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let message = websocket
            .next()
            .await
            .ok_or_else(|| "Solana WebSocket stream ended".to_owned())?
            .map_err(|error| format!("Solana WebSocket read failed: {error}"))?;

        match message {
            Message::Text(text) => {
                return serde_json::from_str(text.as_ref())
                    .map_err(|error| format!("invalid Solana WebSocket JSON: {error}"));
            }
            Message::Binary(bytes) => {
                return serde_json::from_slice(bytes.as_ref())
                    .map_err(|error| format!("invalid binary Solana WebSocket JSON: {error}"));
            }
            Message::Close(frame) => {
                return Err(format!(
                    "Solana WebSocket closed before Orca O2 completed: {frame:?}"
                ));
            }
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use solana_pubkey::Pubkey;

    fn sample_pool(adaptive_fee: bool) -> orca::OrcaWhirlpoolState {
        orca::OrcaWhirlpoolState {
            whirlpools_config: Pubkey::new_unique().to_string(),
            whirlpool_bump: 255,
            tick_spacing: 64,
            fee_tier_index_seed: if adaptive_fee { 32 } else { 64 },
            fee_rate: 3_000,
            protocol_fee_rate: 300,
            liquidity: 1_000_000_000,
            sqrt_price: 18_446_744_073_709_551_616u128,
            tick_current_index: 0,
            token_mint_a: Pubkey::new_unique().to_string(),
            token_vault_a: Pubkey::new_unique().to_string(),
            token_mint_b: Pubkey::new_unique().to_string(),
            token_vault_b: Pubkey::new_unique().to_string(),
        }
    }

    fn observation(adaptive_fee: bool) -> orca::OrcaWhirlpoolAccountObservation {
        orca::OrcaWhirlpoolAccountObservation {
            pubkey: Pubkey::new_unique().to_string(),
            slot: 123_456,
            owner: orca::ORCA_WHIRLPOOL_PROGRAM_ID.to_owned(),
            encoded_data_len: 0,
            decoded_data_len: 653,
            pool_state: sample_pool(adaptive_fee),
        }
    }

    #[test]
    fn ordinary_snapshot_plan_has_exact_order_and_count() -> Result<(), String> {
        let observation = observation(false);
        let plan = build_snapshot_plan(&observation)?;

        assert_eq!(plan.pubkeys.len(), 9);
        assert_eq!(plan.pubkeys[0], observation.pubkey);
        assert_eq!(plan.pubkeys[1], observation.pool_state.token_mint_a);
        assert_eq!(plan.pubkeys[2], observation.pool_state.token_mint_b);
        assert_eq!(plan.pubkeys[8], CLOCK_SYSVAR_ID);
        assert!(!plan.adaptive_fee);

        for index in 0..5 {
            let expected = orca_o2::tick_array_pda(
                &observation.pubkey,
                plan.tick_array_start_indexes[index],
            )?;

            assert_eq!(plan.pubkeys[3 + index], expected);
        }

        Ok(())
    }

    #[test]
    fn adaptive_snapshot_plan_appends_oracle_as_tenth_account() -> Result<(), String> {
        let observation = observation(true);
        let plan = build_snapshot_plan(&observation)?;

        assert_eq!(plan.pubkeys.len(), 10);
        assert!(plan.adaptive_fee);
        assert_eq!(plan.pubkeys[8], CLOCK_SYSVAR_ID);
        assert_eq!(
            plan.pubkeys[9],
            orca_o2::oracle_pda(&observation.pubkey)?
        );

        Ok(())
    }

    #[test]
    fn snapshot_request_binds_min_context_slot_to_trigger() -> Result<(), String> {
        let observation = observation(false);
        let plan = build_snapshot_plan(&observation)?;
        let request = snapshot_request(&observation, &plan);

        assert_eq!(
            request
                .pointer("/params/1/minContextSlot")
                .and_then(Value::as_u64),
            Some(observation.slot)
        );

        assert_eq!(
            request.pointer("/params/1/commitment").and_then(Value::as_str),
            Some("processed")
        );

        assert_eq!(
            request.pointer("/params/1/encoding").and_then(Value::as_str),
            Some("base64")
        );

        Ok(())
    }

    #[test]
    fn stable_identity_accepts_mutable_snapshot_changes() -> Result<(), String> {
        let trigger = sample_pool(false);
        let mut snapshot = trigger.clone();

        snapshot.liquidity = snapshot
            .liquidity
            .checked_add(1)
            .ok_or_else(|| "test liquidity overflow".to_owned())?;

        snapshot.sqrt_price = snapshot
            .sqrt_price
            .checked_add(1)
            .ok_or_else(|| "test sqrt_price overflow".to_owned())?;

        snapshot.tick_current_index = snapshot
            .tick_current_index
            .checked_add(1)
            .ok_or_else(|| "test tick index overflow".to_owned())?;

        orca_o2::verify_stable_pool_identity(&trigger, &snapshot)
    }

    #[test]
    fn stable_identity_rejects_mint_change() -> Result<(), String> {
        let trigger = sample_pool(false);
        let mut snapshot = trigger.clone();

        snapshot.token_mint_a = Pubkey::new_unique().to_string();

        match orca_o2::verify_stable_pool_identity(&trigger, &snapshot) {
            Ok(()) => Err("stable identity accepted changed mint A".to_owned()),
            Err(error) => {
                assert!(error.contains("token_mint_a changed"));
                Ok(())
            }
        }
    }
}
