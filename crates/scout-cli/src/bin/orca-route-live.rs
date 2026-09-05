#![allow(dead_code)]

#[path = "../discovery.rs"]
mod discovery;
#[path = "../orca.rs"]
mod orca;
#[path = "../orca_live.rs"]
mod orca_live;
#[path = "../orca_o2.rs"]
mod orca_o2;
#[path = "../orca_o2_quote_inputs.rs"]
mod orca_o2_quote_inputs;
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

use discovery::{parse_raydium_pair_lookup_response, raydium_pair_lookup_requests};
use futures_util::{SinkExt, StreamExt};
use quote::{
    quote_readiness_for_pool, quote_two_leg_exact_input, QuoteReadiness, VenueQuoteContext,
};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::generate_two_leg_routes;
use scout_core::{NormalizedPoolState, Venue};
use serde_json::{json, Value};
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
const MAX_ORCA_OBSERVATIONS: usize = 50;

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    match timeout(TOTAL_TIMEOUT, run_live_route_proof()).await {
        Ok(result) => result,
        Err(_) => Err(format!(
            "Orca live route proof exceeded {} seconds",
            TOTAL_TIMEOUT.as_secs()
        )),
    }
}

async fn run_live_route_proof() -> Result<(), String> {
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

    println!("Scout Orca cross-venue live route proof");
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
                println!("orca_route_observation_rejected: {error}");
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
            println!(
                "orca_route_candidate_skipped: pool={} reason=adaptive-fee normalization is not admitted by the current production-normalization contract",
                observation.pubkey
            );
            continue;
        }

        anchor_candidates += 1;

        println!(
            "orca_route_candidate: pool={} slot={} anchor={} intermediate={}",
            observation.pubkey, observation.slot, anchor_mint, intermediate_mint
        );

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
                    "orca_route_o2_rejected: pool={} reason={error}",
                    observation.pubkey
                );
                continue;
            }
        };

        o2_ready_candidates += 1;

        if try_raydium_route(&rpc_client, &prepared).await? {
            println!("orca_route_live_observation_count={observed}");
            println!("orca_route_anchor_candidate_count={anchor_candidates}");
            println!("orca_route_o2_ready_candidate_count={o2_ready_candidates}");
            println!("READ-ONLY ORCA-RAYDIUM LIVE ROUTE PROOF PASS");
            return Ok(());
        }

        if try_pumpswap_route(&rpc_client, &prepared).await? {
            println!("orca_route_live_observation_count={observed}");
            println!("orca_route_anchor_candidate_count={anchor_candidates}");
            println!("orca_route_o2_ready_candidate_count={o2_ready_candidates}");
            println!("READ-ONLY ORCA-PUMPSWAP LIVE ROUTE PROOF PASS");
            return Ok(());
        }
    }

    Err(format!(
        "Orca live route proof exhausted {observed} observations: anchor_candidates={anchor_candidates} o2_ready_candidates={o2_ready_candidates}"
    ))
}

async fn try_raydium_route(
    rpc_client: &Client,
    orca: &orca_live::PreparedOrca,
) -> Result<bool, String> {
    for request in raydium_pair_lookup_requests(&orca.anchor_mint, &orca.intermediate_mint) {
        sleep(LOOKUP_PACING).await;

        let payload = post_rpc(rpc_client, &request, "Raydium exact-pair lookup").await?;

        let observations = parse_raydium_pair_lookup_response(&payload)?;

        for observation in observations {
            let (normalized, snapshot, readiness) =
                match hydrate_raydium(rpc_client, &observation).await {
                    Ok(value) => value,
                    Err(error) => {
                        println!(
                            "orca_route_raydium_candidate_rejected: pool={} reason={error}",
                            observation.pubkey
                        );
                        continue;
                    }
                };

            let context = VenueQuoteContext::Raydium {
                pool_id: normalized.pool_id.clone(),
                snapshot: &snapshot,
            };

            if prove_cross_venue_route(orca, normalized, readiness, &context)? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

async fn try_pumpswap_route(
    rpc_client: &Client,
    orca: &orca_live::PreparedOrca,
) -> Result<bool, String> {
    for request in pumpswap::pair_lookup_requests(&orca.anchor_mint, &orca.intermediate_mint) {
        sleep(LOOKUP_PACING).await;

        let payload = post_rpc(rpc_client, &request, "PumpSwap exact-pair lookup").await?;

        let observations = pumpswap::parse_pair_lookup_response(&payload)?;

        for observation in observations {
            let (normalized, snapshot, readiness) =
                match hydrate_pumpswap(rpc_client, &observation).await {
                    Ok(value) => value,
                    Err(error) => {
                        println!(
                            "orca_route_pumpswap_candidate_rejected: pool={} reason={error}",
                            observation.pubkey
                        );
                        continue;
                    }
                };

            let context = VenueQuoteContext::PumpSwap {
                pool_id: normalized.pool_id.clone(),
                snapshot: &snapshot,
            };

            if prove_cross_venue_route(orca, normalized, readiness, &context)? {
                return Ok(true);
            }
        }
    }

    Ok(false)
}

fn prove_cross_venue_route(
    orca: &orca_live::PreparedOrca,
    counterpart: NormalizedPoolState,
    counterpart_readiness: QuoteReadiness,
    counterpart_context: &VenueQuoteContext<'_>,
) -> Result<bool, String> {
    let mut registry = ActiveMintRegistry::new();

    registry.upsert(orca.normalized.clone(), Some(orca.readiness.clone()))?;

    registry.upsert(counterpart.clone(), Some(counterpart_readiness))?;

    if registry.active_mints().is_empty() {
        return Ok(false);
    }

    let eligible = registry.current_eligible_pools();

    let routes = generate_two_leg_routes(&eligible)
        .into_iter()
        .filter(|route| {
            route.anchor_mint() == orca.anchor_mint.as_str()
                && route.intermediate_mint() == orca.intermediate_mint.as_str()
        })
        .collect::<Vec<_>>();

    if routes.len() != 2 {
        println!(
            "orca_route_candidate_rejected: counterpart_pool={} reason=expected exactly 2 directional routes got {}",
            counterpart.pool_id,
            routes.len()
        );
        return Ok(false);
    }

    let decimals = anchor_decimals(&orca.normalized, &orca.anchor_mint)?;

    for amount_in_raw in probe_amounts(decimals)? {
        let mut successful = Vec::new();

        for route in &routes {
            let result = if route.leg_1().venue() == Venue::Orca {
                quote_two_leg_exact_input(
                    route,
                    amount_in_raw,
                    &orca.quote_snapshot,
                    counterpart_context,
                )
            } else if route.leg_2().venue() == Venue::Orca {
                quote_two_leg_exact_input(
                    route,
                    amount_in_raw,
                    counterpart_context,
                    &orca.quote_snapshot,
                )
            } else {
                return Err("generated route unexpectedly omitted Orca".to_owned());
            };

            match result {
                Ok(quote) => successful.push((route, quote)),
                Err(error) => {
                    println!(
                        "orca_route_quote_probe_rejected: amount_in_raw={} route=[{}] reason={error}",
                        amount_in_raw,
                        route.summary()
                    );
                }
            }
        }

        if successful.len() != 2 {
            continue;
        }

        println!(
            "orca_route_registry_pass: eligible_pool_count={} active_mint_count={}",
            eligible.len(),
            registry.active_mints().len()
        );

        for (route, quote) in successful {
            println!("orca_route_candidate_pass: {}", route.summary());

            println!("orca_route_quote_pass: {}", quote.summary());
        }

        println!(
            "orca_route_pair_pass: anchor={} intermediate={} orca_pool={} counterpart_venue={} counterpart_pool={} amount_in_raw={}",
            orca.anchor_mint,
            orca.intermediate_mint,
            orca.normalized.pool_id,
            counterpart.venue.label(),
            counterpart.pool_id,
            amount_in_raw
        );

        return Ok(true);
    }

    Ok(false)
}

fn anchor_decimals(pool: &NormalizedPoolState, anchor_mint: &str) -> Result<u8, String> {
    if pool.token_a.mint == anchor_mint {
        Ok(pool.token_a.decimals)
    } else if pool.token_b.mint == anchor_mint {
        Ok(pool.token_b.decimals)
    } else {
        Err(format!(
            "anchor mint {anchor_mint} is not in pool {}",
            pool.pool_id
        ))
    }
}

fn probe_amounts(decimals: u8) -> Result<Vec<u64>, String> {
    let whole = 10u64
        .checked_pow(u32::from(decimals))
        .ok_or_else(|| format!("anchor decimals {decimals} exceed u64 sizing"))?;

    let mut amounts = Vec::new();

    for divisor in [1_000u64, 100, 10, 1] {
        let amount = whole / divisor;

        if amount > 0 && !amounts.contains(&amount) {
            amounts.push(amount);
        }
    }

    if amounts.is_empty() {
        amounts.push(1);
    }

    Ok(amounts)
}

async fn hydrate_raydium(
    rpc_client: &Client,
    observation: &raydium::RaydiumCpmmAccountObservation,
) -> Result<
    (
        NormalizedPoolState,
        raydium::RaydiumHydrationSnapshot,
        QuoteReadiness,
    ),
    String,
> {
    let pubkeys = raydium::hydration_account_pubkeys(observation);

    let payload = fetch_multiple_accounts(
        rpc_client,
        50,
        &pubkeys,
        observation.slot,
        "Raydium route hydration",
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

async fn hydrate_pumpswap(
    rpc_client: &Client,
    observation: &pumpswap::PumpSwapAccountObservation,
) -> Result<
    (
        NormalizedPoolState,
        pumpswap::PumpSwapHydrationSnapshot,
        QuoteReadiness,
    ),
    String,
> {
    let pubkeys = pumpswap::hydration_account_pubkeys(observation);

    let payload = fetch_multiple_accounts(
        rpc_client,
        60,
        &pubkeys,
        observation.slot,
        "PumpSwap route hydration",
    )
    .await?;

    let snapshot = pumpswap::parse_hydration_response(observation, &payload)?;

    let now = unix_time_ms_now()?;

    let normalized = pumpswap::hydrate_normalized_observation(observation, &snapshot, now, now)?;

    let context = VenueQuoteContext::PumpSwap {
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

        println!("orca_route_program_subscription_confirmed");

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
                    "Solana WebSocket closed before Orca route proof completed: {frame:?}"
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

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_orca_pool(token_a: &str, token_b: &str) -> orca::OrcaWhirlpoolState {
        orca::OrcaWhirlpoolState {
            whirlpools_config: "config".to_owned(),
            whirlpool_bump: 255,
            tick_spacing: 64,
            fee_tier_index_seed: 64,
            fee_rate: 3_000,
            protocol_fee_rate: 300,
            liquidity: 1,
            sqrt_price: 1,
            tick_current_index: 0,
            token_mint_a: token_a.to_owned(),
            token_vault_a: "vault-a".to_owned(),
            token_mint_b: token_b.to_owned(),
            token_vault_b: "vault-b".to_owned(),
        }
    }

    #[test]
    fn anchor_pair_prefers_wrapped_sol() {
        let pool = sample_orca_pool(route::USDC_MINT, route::WRAPPED_SOL_MINT);

        assert_eq!(
            orca_live::anchor_pair(&pool),
            Some((route::WRAPPED_SOL_MINT, route::USDC_MINT))
        );
    }

    #[test]
    fn probe_amounts_are_bounded_and_increasing() -> Result<(), String> {
        assert_eq!(probe_amounts(6)?, vec![1_000, 10_000, 100_000, 1_000_000]);

        Ok(())
    }
}
