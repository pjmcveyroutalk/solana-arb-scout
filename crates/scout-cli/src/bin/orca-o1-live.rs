#![allow(dead_code)]

#[path = "../orca.rs"]
mod orca;

use futures_util::{SinkExt, StreamExt};
use reqwest::Client;
use serde_json::{json, Value};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const SOLANA_WS_URL: &str = "wss://api.mainnet-beta.solana.com";

const ORCA_O1_TOTAL_TIMEOUT: Duration = Duration::from_secs(120);
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_ORCA_OBSERVATIONS: usize = 25;

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    match timeout(ORCA_O1_TOTAL_TIMEOUT, run_orca_o1()).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "Orca O1 live observation exceeded {} seconds",
            ORCA_O1_TOTAL_TIMEOUT.as_secs()
        )),
    }
}

async fn run_orca_o1() -> Result<(), String> {
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

    println!("Orca O1 live read-only pool-state observation");
    println!("No routing, quote admission, signing, submission, or execution capability.");

    let mut observed = 0usize;

    while observed < MAX_ORCA_OBSERVATIONS {
        let payload = next_json_message(&mut websocket).await?;

        let observation = match orca::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("orca_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "orca_observation: pool={} slot={} {}",
            observation.pubkey,
            observation.slot,
            observation.pool_state.summary()
        );

        let hydration_payload = fetch_hydration(&rpc_client, &observation).await?;

        let snapshot = match orca::parse_hydration_response(&observation, &hydration_payload) {
            Ok(snapshot) => snapshot,
            Err(error) => {
                println!(
                    "orca_hydration_rejected: pool={} reason={error}",
                    observation.pubkey
                );
                continue;
            }
        };

        println!("orca_hydration: {}", snapshot.summary());

        let received_at_unix_ms = unix_ms()?;
        let normalized_at_unix_ms = unix_ms()?;

        let normalized = match orca::hydrate_normalized_observation(
            &observation,
            &snapshot,
            received_at_unix_ms,
            normalized_at_unix_ms,
        ) {
            Ok(normalized) => normalized,
            Err(error) => {
                println!(
                    "orca_normalization_rejected: pool={} reason={error}",
                    observation.pubkey
                );
                continue;
            }
        };

        println!("orca_normalized_pool: {}", normalized.summary());
        println!("orca_live_observation_count={observed}");
        println!("READ-ONLY ORCA WHIRLPOOL O1 OBSERVATION PASS");

        return Ok(());
    }

    Err(format!(
        "Orca O1 observed {observed} Whirlpool updates without one ordinary pool completing hydration"
    ))
}

async fn fetch_hydration(
    rpc_client: &Client,
    observation: &orca::OrcaWhirlpoolAccountObservation,
) -> Result<Value, String> {
    let pubkeys = orca::hydration_account_pubkeys(observation);

    let request = json!({
        "jsonrpc": "2.0",
        "id": 19,
        "method": "getMultipleAccounts",
        "params": [
            pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    });

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("Orca hydration RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!("Orca hydration RPC returned HTTP status {status}"));
    }

    response
        .json::<Value>()
        .await
        .map_err(|error| format!("invalid Orca hydration RPC JSON: {error}"))
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
                    "Solana WebSocket closed before Orca O1 completed: {frame:?}"
                ));
            }
            _ => {}
        }
    }
}

fn unix_ms() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before Unix epoch: {error}"))?;

    u64::try_from(elapsed.as_millis())
        .map_err(|_| "Unix millisecond timestamp exceeded u64".to_owned())
}
