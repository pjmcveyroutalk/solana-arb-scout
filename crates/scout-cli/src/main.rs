mod pumpswap;
mod raydium;
mod registry;

use futures_util::{SinkExt, StreamExt};
use registry::ActiveMintRegistry;
use reqwest::Client;
use scout_core::NormalizedPoolState;
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SOLANA_WSS_URL: &str = "wss://api.mainnet-beta.solana.com";
const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 5;
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

#[tokio::main]
async fn main() {
    println!("ARB Scout V0 — READ ONLY");
    println!("No signing. No wallet. No transaction execution.");

    if let Err(error) = install_crypto_provider() {
        eprintln!("TLS crypto provider initialization failed: {error}");
        std::process::exit(1);
    }

    if let Err(error) = observe_slots().await {
        eprintln!("Live slot observation failed: {error}");
        std::process::exit(1);
    }

    let rpc_client = match build_rpc_client() {
        Ok(client) => client,
        Err(error) => {
            eprintln!("Solana HTTP RPC client initialization failed: {error}");
            std::process::exit(1);
        }
    };

    let raydium_states = match observe_raydium_cpmm(&rpc_client).await {
        Ok(states) => states,
        Err(error) => {
            eprintln!("Raydium CPMM observation failed: {error}");
            std::process::exit(1);
        }
    };

    let pumpswap_states = match observe_pumpswap(&rpc_client).await {
        Ok(states) => states,
        Err(error) => {
            eprintln!("PumpSwap observation failed: {error}");
            std::process::exit(1);
        }
    };

    if let Err(error) = validate_active_mint_registry(raydium_states, pumpswap_states) {
        eprintln!("Active-mint registry validation failed: {error}");
        std::process::exit(1);
    }
}

fn install_crypto_provider() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install ring as the process TLS provider".to_owned())
}

fn build_rpc_client() -> Result<Client, String> {
    Client::builder()
        .timeout(RPC_TIMEOUT)
        .build()
        .map_err(|error| format!("could not build HTTP RPC client: {error}"))
}

async fn connect() -> Result<
    tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>,
    String,
> {
    let (socket, _) = connect_async(SOLANA_WSS_URL)
        .await
        .map_err(|error| format!("WebSocket connection error: {error}"))?;

    Ok(socket)
}

async fn observe_slots() -> Result<(), String> {
    println!("Live boundary: Solana mainnet slotSubscribe\n");

    let socket = connect().await?;

    println!("Connected: {SOLANA_WSS_URL}");

    let (mut writer, mut reader) = socket.split();

    let subscription = json!({
        "jsonrpc": "2.0",
        "id": 1,
        "method": "slotSubscribe"
    });

    writer
        .send(Message::Text(subscription.to_string()))
        .await
        .map_err(|error| format!("subscription send error: {error}"))?;

    let mut subscription_confirmed = false;
    let mut observed_slots = 0usize;

    while observed_slots < MAX_SLOT_OBSERVATIONS {
        let payload = next_json_message(&mut reader).await?;

        if payload.get("id") == Some(&Value::from(1)) {
            if let Some(subscription_id) = payload.get("result").and_then(Value::as_u64) {
                println!("Subscription confirmed: {subscription_id}");
                subscription_confirmed = true;
                continue;
            }

            return Err(format!("subscription rejected: {payload}"));
        }

        if payload.get("method").and_then(Value::as_str) != Some("slotNotification") {
            continue;
        }

        let slot = payload
            .pointer("/params/result/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("slot notification missing slot: {payload}"))?;

        observed_slots += 1;
        println!("slot[{observed_slots}/{MAX_SLOT_OBSERVATIONS}] = {slot}");
    }

    if !subscription_confirmed {
        return Err("slot subscription was never confirmed".to_owned());
    }

    println!("\nREAD-ONLY LIVE STREAM PASS");
    println!("Observed {observed_slots} Solana mainnet slot updates.\n");

    Ok(())
}

async fn observe_raydium_cpmm(rpc_client: &Client) -> Result<Vec<NormalizedPoolState>, String> {
    println!("DEX adapter: Raydium CPMM");
    println!("Program: {}", raydium::RAYDIUM_CPMM_PROGRAM_ID);
    println!("Hydration boundary: Solana mainnet read-only HTTP RPC");

    let socket = connect().await?;
    let (mut writer, mut reader) = socket.split();

    writer
        .send(Message::Text(
            raydium::program_subscribe_request().to_string(),
        ))
        .await
        .map_err(|error| format!("Raydium subscription send error: {error}"))?;

    let mut subscription_confirmed = false;
    let mut observations = 0usize;
    let mut normalized_states = Vec::with_capacity(MAX_RAYDIUM_OBSERVATIONS);

    while observations < MAX_RAYDIUM_OBSERVATIONS {
        let payload = next_json_message(&mut reader).await?;

        if payload.get("id") == Some(&Value::from(2)) {
            if let Some(subscription_id) = payload.get("result").and_then(Value::as_u64) {
                println!("Raydium subscription confirmed: {subscription_id}");
                subscription_confirmed = true;
                continue;
            }

            return Err(format!("Raydium subscription rejected: {payload}"));
        }

        let account_update_received_at_unix_ms = unix_time_ms_now()?;

        let Some(observation) = raydium::parse_program_notification(&payload)? else {
            continue;
        };

        let hydration_started_at_unix_ms = unix_time_ms_now()?;
        let hydration_payload = fetch_raydium_hydration(rpc_client, &observation).await?;
        let snapshot = raydium::parse_hydration_response(&observation, &hydration_payload)?;
        let hydrated_at_unix_ms = unix_time_ms_now()?;

        let normalized = raydium::hydrate_normalized_observation(
            &observation,
            &snapshot,
            account_update_received_at_unix_ms,
            hydrated_at_unix_ms,
        )?;

        observations += 1;

        let hydration_duration_ms =
            hydrated_at_unix_ms.saturating_sub(hydration_started_at_unix_ms);
        let total_observation_duration_ms =
            hydrated_at_unix_ms.saturating_sub(account_update_received_at_unix_ms);

        println!(
            "raydium[{observations}/{MAX_RAYDIUM_OBSERVATIONS}] slot={} reserve_slot={} pubkey={} owner={} encoded_data_len={} decoded_data_len={}",
            observation.slot,
            snapshot.slot,
            observation.pubkey,
            observation.owner,
            observation.encoded_data_len,
            observation.decoded_data_len,
        );

        println!("decoded_pool: {}", observation.pool_state.summary());
        println!("hydrated_reserves: {}", snapshot.summary());
        println!(
            "timing: received_at_ms={} hydration_started_at_ms={} hydrated_at_ms={} hydration_duration_ms={} total_observation_duration_ms={}",
            account_update_received_at_unix_ms,
            hydration_started_at_unix_ms,
            hydrated_at_unix_ms,
            hydration_duration_ms,
            total_observation_duration_ms,
        );
        println!("normalized_pool: {}", normalized.summary());

        normalized_states.push(normalized);
    }

    if !subscription_confirmed {
        return Err("Raydium CPMM subscription was never confirmed".to_owned());
    }

    println!("READ-ONLY RAYDIUM CPMM ACCOUNT DECODER PASS");
    println!("READ-ONLY NORMALIZED POOL STATE PASS");
    println!("READ-ONLY RAYDIUM VAULT HYDRATION PASS");
    println!("READ-ONLY RUNG 6 OBSERVATION COLLECTION PASS");

    Ok(normalized_states)
}

async fn fetch_raydium_hydration(
    rpc_client: &Client,
    observation: &raydium::RaydiumCpmmAccountObservation,
) -> Result<Value, String> {
    let account_pubkeys = raydium::hydration_account_pubkeys(observation);

    fetch_hydration(rpc_client, 3, account_pubkeys, observation.slot, "Raydium").await
}

async fn observe_pumpswap(rpc_client: &Client) -> Result<Vec<NormalizedPoolState>, String> {
    println!("\nDEX adapter: PumpSwap");
    println!("Program: {}", pumpswap::PUMPSWAP_PROGRAM_ID);
    println!("Hydration boundary: Solana mainnet read-only HTTP RPC");

    let socket = connect().await?;
    let (mut writer, mut reader) = socket.split();

    writer
        .send(Message::Text(
            pumpswap::program_subscribe_request().to_string(),
        ))
        .await
        .map_err(|error| format!("PumpSwap subscription send error: {error}"))?;

    let mut subscription_confirmed = false;
    let mut observations = 0usize;
    let mut normalized_states = Vec::with_capacity(MAX_PUMPSWAP_OBSERVATIONS);

    while observations < MAX_PUMPSWAP_OBSERVATIONS {
        let payload = next_json_message(&mut reader).await?;

        if payload.get("id") == Some(&Value::from(4)) {
            if let Some(subscription_id) = payload.get("result").and_then(Value::as_u64) {
                println!("PumpSwap subscription confirmed: {subscription_id}");
                subscription_confirmed = true;
                continue;
            }

            return Err(format!("PumpSwap subscription rejected: {payload}"));
        }

        let account_update_received_at_unix_ms = unix_time_ms_now()?;

        let Some(observation) = pumpswap::parse_program_notification(&payload)? else {
            continue;
        };

        let hydration_started_at_unix_ms = unix_time_ms_now()?;
        let hydration_payload = fetch_pumpswap_hydration(rpc_client, &observation).await?;
        let snapshot = pumpswap::parse_hydration_response(&observation, &hydration_payload)?;
        let hydrated_at_unix_ms = unix_time_ms_now()?;

        let normalized = pumpswap::hydrate_normalized_observation(
            &observation,
            &snapshot,
            account_update_received_at_unix_ms,
            hydrated_at_unix_ms,
        )?;

        observations += 1;

        let hydration_duration_ms =
            hydrated_at_unix_ms.saturating_sub(hydration_started_at_unix_ms);
        let total_observation_duration_ms =
            hydrated_at_unix_ms.saturating_sub(account_update_received_at_unix_ms);

        println!(
            "pumpswap[{observations}/{MAX_PUMPSWAP_OBSERVATIONS}] slot={} reserve_slot={} pubkey={} owner={} encoded_data_len={} decoded_data_len={}",
            observation.slot,
            snapshot.slot,
            observation.pubkey,
            observation.owner,
            observation.encoded_data_len,
            observation.decoded_data_len,
        );

        println!("decoded_pool: {}", observation.pool_state.summary());
        println!("hydrated_reserves: {}", snapshot.summary());
        println!(
            "timing: received_at_ms={} hydration_started_at_ms={} hydrated_at_ms={} hydration_duration_ms={} total_observation_duration_ms={}",
            account_update_received_at_unix_ms,
            hydration_started_at_unix_ms,
            hydrated_at_unix_ms,
            hydration_duration_ms,
            total_observation_duration_ms,
        );
        println!("normalized_pool: {}", normalized.summary());

        normalized_states.push(normalized);
    }

    if !subscription_confirmed {
        return Err("PumpSwap subscription was never confirmed".to_owned());
    }

    println!("READ-ONLY PUMPSWAP ACCOUNT DECODER PASS");
    println!("READ-ONLY PUMPSWAP SNAPSHOT HYDRATION PASS");
    println!("READ-ONLY SECOND VENUE NORMALIZATION PASS");
    println!("READ-ONLY RUNG 7 OBSERVATION COLLECTION PASS");

    Ok(normalized_states)
}

async fn fetch_pumpswap_hydration(
    rpc_client: &Client,
    observation: &pumpswap::PumpSwapAccountObservation,
) -> Result<Value, String> {
    let account_pubkeys = pumpswap::hydration_account_pubkeys(observation);

    fetch_hydration(rpc_client, 5, account_pubkeys, observation.slot, "PumpSwap").await
}

fn validate_active_mint_registry(
    raydium_states: Vec<NormalizedPoolState>,
    pumpswap_states: Vec<NormalizedPoolState>,
) -> Result<(), String> {
    println!("\nRegistry: Active Mint");

    let mut registry = ActiveMintRegistry::new();

    for state in raydium_states.into_iter().chain(pumpswap_states) {
        registry.upsert(state);
    }

    let active_mints = registry.active_mints();

    println!("registry_current_pools={}", registry.current_pool_count());

    if active_mints.is_empty() {
        return Err(
            "bounded live sample produced no mint represented by two eligible venues".to_owned(),
        );
    }

    for active_mint in &active_mints {
        println!("active_mint: {}", active_mint.summary());
    }

    println!("READ-ONLY ACTIVE-MINT REGISTRY PASS");
    println!("READ-ONLY RUNG 8 ACTIVE-MINT DETECTION PASS");

    Ok(())
}

async fn fetch_hydration<const N: usize>(
    rpc_client: &Client,
    request_id: u64,
    account_pubkeys: [String; N],
    min_context_slot: u64,
    venue: &str,
) -> Result<Value, String> {
    let account_pubkeys = account_pubkeys.to_vec();

    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getMultipleAccounts",
        "params": [
            account_pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64",
                "minContextSlot": min_context_slot
            }
        ]
    });

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("{venue} hydration RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "{venue} hydration RPC returned HTTP status {status}"
        ));
    }

    response
        .json::<Value>()
        .await
        .map_err(|error| format!("{venue} hydration RPC returned invalid JSON: {error}"))
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before Unix epoch: {error}"))?;

    u64::try_from(duration.as_millis())
        .map_err(|_| "Unix timestamp milliseconds exceeded u64".to_owned())
}

async fn next_json_message<S>(reader: &mut S) -> Result<Value, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let next_message = timeout(OBSERVATION_TIMEOUT, reader.next())
            .await
            .map_err(|_| "timed out waiting for Solana data".to_owned())?
            .ok_or_else(|| "Solana WebSocket stream closed".to_owned())?
            .map_err(|error| format!("WebSocket receive error: {error}"))?;

        if !next_message.is_text() {
            continue;
        }

        let text = next_message
            .into_text()
            .map_err(|error| format!("invalid text frame: {error}"))?;

        return serde_json::from_str(&text).map_err(|error| format!("invalid JSON: {error}"));
    }
}
