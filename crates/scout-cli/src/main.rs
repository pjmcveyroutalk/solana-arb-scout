mod raydium;

use futures_util::{SinkExt, StreamExt};
use serde_json::{json, Value};
use std::time::Duration;
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SOLANA_WSS_URL: &str = "wss://api.mainnet-beta.solana.com";
const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 1;
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);

#[tokio::main]
async fn main() {
    println!("ARB Scout V0 — READ ONLY");
    println!("No signing. No wallet. No transaction execution.");

    if let Err(error) = observe_slots().await {
        eprintln!("Live slot observation failed: {error}");
        std::process::exit(1);
    }

    if let Err(error) = observe_raydium_cpmm().await {
        eprintln!("Raydium CPMM observation failed: {error}");
        std::process::exit(1);
    }
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

async fn observe_raydium_cpmm() -> Result<(), String> {
    println!("DEX adapter: Raydium CPMM");
    println!("Program: {}", raydium::RAYDIUM_CPMM_PROGRAM_ID);

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

        let Some(observation) = raydium::parse_program_notification(&payload)? else {
            continue;
        };

        observations += 1;

        println!(
            "raydium[{observations}/{MAX_RAYDIUM_OBSERVATIONS}] slot={} pubkey={} owner={} encoded_data_len={}",
            observation.slot,
            observation.pubkey,
            observation.owner,
            observation.data_len
        );
    }

    if !subscription_confirmed {
        return Err("Raydium CPMM subscription was never confirmed".to_owned());
    }

    println!("READ-ONLY RAYDIUM CPMM ADAPTER PASS");

    Ok(())
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
