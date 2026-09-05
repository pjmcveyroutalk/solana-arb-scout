use crate::{orca, orca_live};
use futures_util::StreamExt;
use reqwest::Client;
use serde_json::Value;
use std::collections::BTreeMap;
use tokio::time::{timeout, Duration};
use tokio_tungstenite::tungstenite::Message;

const MAX_ORCA_OBSERVATIONS: usize = 10;
const ORCA_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn observe_and_prepare<S>(
    rpc_client: &Client,
    rpc_url: &str,
    reader: &mut S,
) -> Result<BTreeMap<String, orca_live::PreparedOrca>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    println!("\nVenue adapter: Orca Whirlpool");

    let mut prepared_by_pool = BTreeMap::new();
    let mut observed = 0usize;
    let mut anchor_candidates = 0usize;

    while observed < MAX_ORCA_OBSERVATIONS && prepared_by_pool.is_empty() {
        let Some(payload) = next_json_message(reader).await? else {
            break;
        };

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

        let Some((anchor_mint, intermediate_mint)) =
            orca_live::anchor_pair(&observation.pool_state)
        else {
            continue;
        };

        anchor_candidates += 1;

        if observation.pool_state.is_adaptive_fee() {
            println!(
                concat!(
                    "orca_preparation_rejected: pool={} reason=",
                    "adaptive-fee pool is not admitted by current production O2 preparation"
                ),
                observation.pubkey
            );
            continue;
        }

        match orca_live::prepare_orca(
            rpc_client,
            rpc_url,
            &observation,
            anchor_mint,
            intermediate_mint,
        )
        .await
        {
            Ok(prepared) => {
                println!(
                    "orca_production_ready: pool={} slot={} anchor={} intermediate={}",
                    prepared.normalized.pool_id,
                    prepared.normalized.source_slot,
                    prepared.anchor_mint,
                    prepared.intermediate_mint
                );

                prepared_by_pool.insert(prepared.normalized.pool_id.clone(), prepared);
            }
            Err(error) => {
                println!(
                    "orca_preparation_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("orca_live_observation_count={observed}");
    println!("orca_live_anchor_candidate_count={anchor_candidates}");
    println!("orca_live_eligible_count={}", prepared_by_pool.len());

    if prepared_by_pool.is_empty() {
        println!(
            "orca_production_admission_unavailable: no bounded O2-ready Orca pool observed"
        );
    } else {
        println!("READ-ONLY ORCA PRODUCTION ADMISSION PASS");
    }

    Ok(prepared_by_pool)
}

async fn next_json_message<S>(reader: &mut S) -> Result<Option<Value>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let next = match timeout(ORCA_OBSERVATION_TIMEOUT, reader.next()).await {
            Ok(next) => next,
            Err(_) => return Ok(None),
        };

        let message = match next {
            Some(Ok(message)) => message,
            Some(Err(error)) => {
                return Err(format!("Orca WebSocket read failed: {error}"));
            }
            None => {
                return Err("Orca WebSocket stream ended".to_owned());
            }
        };

        match message {
            Message::Text(text) => {
                let payload = serde_json::from_str::<Value>(text.as_ref())
                    .map_err(|error| format!("Orca WebSocket returned invalid JSON: {error}"))?;
                return Ok(Some(payload));
            }
            Message::Binary(bytes) => {
                let payload = serde_json::from_slice::<Value>(bytes.as_ref())
                    .map_err(|error| format!("Orca WebSocket returned invalid JSON: {error}"))?;
                return Ok(Some(payload));
            }
            Message::Close(_) => {
                return Err("Orca WebSocket stream closed".to_owned());
            }
            Message::Ping(_) | Message::Pong(_) | Message::Frame(_) => {}
        }
    }
}
