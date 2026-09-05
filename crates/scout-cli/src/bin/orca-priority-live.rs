#![allow(dead_code)]

#[path = "../costs.rs"]
mod costs;
#[path = "../discovery.rs"]
mod discovery;
#[path = "../economics.rs"]
mod economics;
#[path = "../orca.rs"]
mod orca;
#[path = "../orca_live.rs"]
mod orca_live;
#[path = "../orca_o2.rs"]
mod orca_o2;
#[path = "../orca_o2_quote_inputs.rs"]
mod orca_o2_quote_inputs;
#[path = "../orca_priority.rs"]
mod orca_priority;
#[path = "../pumpswap.rs"]
mod pumpswap;
#[path = "../quote.rs"]
mod quote;
#[path = "../raydium.rs"]
mod raydium;
#[path = "../registry.rs"]
mod registry;
#[path = "../route.rs"]
mod route;
#[path = "../sizing.rs"]
mod sizing;

use discovery::{parse_raydium_pair_lookup_response, raydium_pair_lookup_requests};
use futures_util::{SinkExt, StreamExt};
use quote::{quote_readiness_for_pool, quote_two_leg_exact_input, VenueQuoteContext};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::generate_two_leg_routes;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const SOLANA_WS_URL: &str = "wss://api.mainnet-beta.solana.com";
const TOTAL_TIMEOUT: Duration = Duration::from_secs(210);
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const LOOKUP_PACING: Duration = Duration::from_millis(250);
const MAX_ORCA_OBSERVATIONS: usize = 75;
const PRIORITY_PROBE_AMOUNT_RAW: u64 = 1_000_000;

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    match timeout(TOTAL_TIMEOUT, run_priority_proof()).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "Orca priority live proof exceeded {} seconds",
            TOTAL_TIMEOUT.as_secs()
        )),
    }
}

async fn run_priority_proof() -> Result<(), String> {
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

    println!("Scout Orca + Raydium localized priority live proof");
    println!("Read-only RPC and quote computation only; no signing, submission, or execution.");

    let mut observed = 0usize;
    let mut anchor_candidates = 0usize;
    let mut o2_ready_candidates = 0usize;

    while observed < MAX_ORCA_OBSERVATIONS {
        let payload = next_json_message(&mut websocket).await?;

        let observation = match orca::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("orca_priority_live_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        let Some((anchor_mint, intermediate_mint)) =
            orca_live::anchor_pair(&observation.pool_state)
        else {
            continue;
        };

        if observation.pool_state.is_adaptive_fee() {
            continue;
        }

        anchor_candidates += 1;

        let prepared = match orca_live::prepare_orca(
            &rpc_client,
            SOLANA_RPC_URL,
            &observation,
            anchor_mint,
            intermediate_mint,
        )
        .await
        {
            Ok(prepared) => prepared,
            Err(error) => {
                println!(
                    "orca_priority_live_o2_rejected: pool={} reason={error}",
                    observation.pubkey
                );
                continue;
            }
        };

        o2_ready_candidates += 1;

        if try_raydium_priority_route(&rpc_client, prepared).await? {
            println!("orca_priority_live_observation_count={observed}");
            println!("orca_priority_live_anchor_candidate_count={anchor_candidates}");
            println!("orca_priority_live_o2_ready_candidate_count={o2_ready_candidates}");
            println!("READ-ONLY ORCA-RAYDIUM LOCALIZED PRIORITY PROOF PASS");
            return Ok(());
        }
    }

    Err(format!(
        "Orca priority live proof exhausted {observed} observations: anchor_candidates={anchor_candidates} o2_ready_candidates={o2_ready_candidates}"
    ))
}

async fn try_raydium_priority_route(
    rpc_client: &Client,
    prepared: orca_live::PreparedOrca,
) -> Result<bool, String> {
    for request in raydium_pair_lookup_requests(&prepared.anchor_mint, &prepared.intermediate_mint) {
        sleep(LOOKUP_PACING).await;

        let payload = post_rpc(rpc_client, &request, "Orca priority Raydium exact-pair lookup").await?;
        let observations = parse_raydium_pair_lookup_response(&payload)?;

        for observation in observations {
            let (normalized, snapshot, readiness) =
                match hydrate_raydium(rpc_client, &observation).await {
                    Ok(value) => value,
                    Err(error) => {
                        println!(
                            "orca_priority_live_raydium_rejected: pool={} reason={error}",
                            observation.pubkey
                        );
                        continue;
                    }
                };

            let context = VenueQuoteContext::Raydium {
                pool_id: normalized.pool_id.clone(),
                snapshot: &snapshot,
            };

            let mut registry = ActiveMintRegistry::new();
            registry.upsert(
                prepared.normalized.clone(),
                Some(prepared.readiness.clone()),
            )?;
            registry.upsert(normalized.clone(), Some(readiness))?;

            let eligible = registry.current_eligible_pools();
            let routes = generate_two_leg_routes(&eligible)
                .into_iter()
                .filter(|route| {
                    route.anchor_mint() == prepared.anchor_mint.as_str()
                        && route.intermediate_mint() == prepared.intermediate_mint.as_str()
                })
                .collect::<Vec<_>>();

            if routes.len() != 2 {
                continue;
            }

            let mut quoted_routes = Vec::new();

            for route in routes {
                let quote = if route.leg_1().venue() == scout_core::Venue::Orca {
                    quote_two_leg_exact_input(
                        &route,
                        PRIORITY_PROBE_AMOUNT_RAW,
                        &prepared.quote_snapshot,
                        &context,
                    )
                } else {
                    quote_two_leg_exact_input(
                        &route,
                        PRIORITY_PROBE_AMOUNT_RAW,
                        &context,
                        &prepared.quote_snapshot,
                    )
                };

                match quote {
                    Ok(quote) => {
                        println!(
                            "orca_priority_live_route_quote: route=[{}] quote=[{}]",
                            route.summary(),
                            quote.summary()
                        );
                        quoted_routes.push(route);
                    }
                    Err(error) => {
                        println!(
                            "orca_priority_live_route_quote_rejected: route=[{}] reason={error}",
                            route.summary()
                        );
                    }
                }
            }

            if quoted_routes.is_empty() {
                continue;
            }

            let raydium_pool_id = normalized.pool_id.clone();
            let orca_pool_id = prepared.normalized.pool_id.clone();

            let mut raydium_contexts = BTreeMap::new();
            raydium_contexts.insert(raydium_pool_id.clone(), snapshot);

            let mut orca_prepared = BTreeMap::new();
            orca_prepared.insert(orca_pool_id.clone(), prepared);

            let mut cache = BTreeMap::new();

            for route in &quoted_routes {
                let state = orca_priority::observe_route(
                    rpc_client,
                    SOLANA_RPC_URL,
                    route.leg_1(),
                    route.leg_2(),
                    &raydium_contexts,
                    &orca_prepared,
                    &mut cache,
                )
                .await;

                match state {
                    costs::PriorityObservationState::Available(observation) => {
                        println!(
                            "orca_priority_live_observation_pass: route=[{}] {}",
                            route.summary(),
                            observation.summary()
                        );
                    }
                    costs::PriorityObservationState::Unavailable(reason) => {
                        return Err(format!(
                            "Orca + Raydium localized priority observation failed closed for route [{}]: {reason}",
                            route.summary()
                        ));
                    }
                }
            }

            println!(
                "orca_priority_live_pair_pass: orca_pool={} raydium_pool={} route_count={}",
                orca_pool_id,
                raydium_pool_id,
                quoted_routes.len()
            );

            return Ok(true);
        }
    }

    Ok(false)
}

async fn hydrate_raydium(
    rpc_client: &Client,
    observation: &raydium::RaydiumCpmmAccountObservation,
) -> Result<
    (
        scout_core::NormalizedPoolState,
        raydium::RaydiumHydrationSnapshot,
        quote::QuoteReadiness,
    ),
    String,
> {
    let pubkeys = raydium::hydration_account_pubkeys(observation);

    let payload = fetch_multiple_accounts(
        rpc_client,
        71,
        &pubkeys,
        observation.slot,
        "Orca priority Raydium hydration",
    )
    .await?;

    let snapshot = raydium::parse_hydration_response(observation, &payload)?;
    let now = unix_time_ms_now()?;
    let normalized = raydium::hydrate_normalized_observation(observation, &snapshot, now, now)?;

    let context = VenueQuoteContext::Raydium {
        pool_id: normalized.pool_id.clone(),
        snapshot: &snapshot,
    };
    let readiness = quote_readiness_for_pool(&normalized, &context)?;

    Ok((normalized, snapshot, readiness))
}

async fn fetch_multiple_accounts<T>(
    rpc_client: &Client,
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

    post_rpc(rpc_client, &request, label).await
}

async fn post_rpc(rpc_client: &Client, request: &Value, label: &str) -> Result<Value, String> {
    let response = rpc_client
        .post(SOLANA_RPC_URL)
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

        println!("orca_priority_live_program_subscription_confirmed");
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
                    "Solana WebSocket closed before Orca priority proof completed: {frame:?}"
                ));
            }
            _ => {}
        }
    }
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock precedes Unix epoch: {error}"))?;

    u64::try_from(duration.as_millis())
        .map_err(|_| "Unix millisecond timestamp overflow".to_owned())
}
