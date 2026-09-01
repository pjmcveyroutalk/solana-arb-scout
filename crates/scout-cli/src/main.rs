mod costs;
mod discovery;
pub mod economics;
mod pumpswap;
mod quote;
mod raydium;
mod registry;
mod route;
mod sizing;

use costs::{
    economics_cost_model, localized_priority_fee_request,
    parse_localized_priority_fee_response, route_contention_footprint,
    DeterministicVenueContentionFootprint, PriorityFeeObservation, PriorityObservationState,
};
use discovery::{
    parse_raydium_anchor_lookup_response, raydium_anchor_lookup_requests,
    route_candidate_from_observation,
};
use economics::{evaluate_expected_net_for_mode, FundingMode};
use futures_util::{SinkExt, StreamExt};
use quote::{quote_two_leg_exact_input, VenueQuoteContext};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::{generate_two_leg_routes, RouteLeg, USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState};
use serde_json::{json, Value};
use sizing::{
    parse_sol_usd_price, sol_usd_price_request, usd_dollars_to_anchor_raw, SolUsdPrice,
    USD_SIZE_GRID,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Instant, SystemTime, UNIX_EPOCH};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const SOLANA_WS_URL: &str = "wss://api.mainnet-beta.solana.com";
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(15);
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(15);
const DETERMINISTIC_DISCOVERY_TIMEOUT: Duration = Duration::from_secs(210);
const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 15;
const MAX_TARGETED_ROUTE_LOOKUPS: usize = 15;

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    let rpc_client = Client::builder()
        .connect_timeout(RPC_CONNECT_TIMEOUT)
        .timeout(RPC_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("could not build bounded Solana RPC client: {error}"))?;

    let request = SOLANA_WS_URL
        .into_client_request()
        .map_err(|error| format!("invalid Solana WebSocket request: {error}"))?;

    let (websocket, _) = connect_async(request)
        .await
        .map_err(|error| format!("could not connect to Solana WebSocket: {error}"))?;

    let (mut writer, mut reader) = websocket.split();

    writer
        .send(Message::Text(
            json!({
                "jsonrpc": "2.0",
                "id": 1,
                "method": "slotSubscribe"
            })
            .to_string(),
        ))
        .await
        .map_err(|error| format!("could not subscribe to Solana slots: {error}"))?;

    wait_for_subscription_confirmation(&mut reader, 1, "slot").await?;

    println!("Scout V0 live read-only Solana stream");
    println!("No signing, transaction construction, submission, or execution capability.");

    observe_slots(&mut reader).await?;

    writer
        .send(Message::Text(
            raydium::program_subscribe_request().to_string(),
        ))
        .await
        .map_err(|error| format!("could not subscribe to Raydium CPMM: {error}"))?;

    wait_for_subscription_confirmation(&mut reader, 2, "Raydium CPMM").await?;

    let (mut raydium_states, mut raydium_quote_contexts) =
        observe_raydium(&rpc_client, &mut reader).await?;

    writer
        .send(Message::Text(
            pumpswap::program_subscribe_request().to_string(),
        ))
        .await
        .map_err(|error| format!("could not subscribe to PumpSwap: {error}"))?;

    wait_for_subscription_confirmation(&mut reader, 4, "PumpSwap").await?;

    let (mut pumpswap_states, mut pumpswap_quote_contexts) =
        observe_pumpswap(&rpc_client, &mut reader).await?;

    let initial_routes = {
        let mut registry = ActiveMintRegistry::new();

        for state in raydium_states
            .iter()
            .cloned()
            .chain(pumpswap_states.iter().cloned())
        {
            registry.upsert(state);
        }

        generate_two_leg_routes(&registry.current_eligible_pools())
    };

    if initial_routes.is_empty() {
        println!("\nRung 9 deterministic anchor-filtered route reacquisition");

        let (
            discovered_raydium_states,
            discovered_raydium_contexts,
            discovered_pumpswap_states,
            discovered_pumpswap_contexts,
        ) = timeout(
            DETERMINISTIC_DISCOVERY_TIMEOUT,
            discover_deterministic_cross_venue_overlap(
                &rpc_client,
                &raydium_states,
                &raydium_quote_contexts,
            ),
        )
        .await
        .map_err(|_| {
            format!(
                "Rung 9 deterministic discovery exceeded {} seconds",
                DETERMINISTIC_DISCOVERY_TIMEOUT.as_secs()
            )
        })??;

        merge_normalized_states(&mut raydium_states, discovered_raydium_states);
        merge_quote_contexts(&mut raydium_quote_contexts, discovered_raydium_contexts);
        merge_normalized_states(&mut pumpswap_states, discovered_pumpswap_states);
        merge_quote_contexts(&mut pumpswap_quote_contexts, discovered_pumpswap_contexts);
    }

    let sol_usd_price = fetch_sol_usd_price(&rpc_client).await?;

    validate_registry_routes_and_sizes(
        &rpc_client,
        raydium_states,
        pumpswap_states,
        &raydium_quote_contexts,
        &pumpswap_quote_contexts,
        &sol_usd_price,
    )
    .await
}

async fn observe_slots<S>(reader: &mut S) -> Result<(), String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    println!("\nTransport: live slots");

    let mut observed = 0usize;

    while observed < MAX_SLOT_OBSERVATIONS {
        let payload = next_json_message(reader).await?;

        if payload.get("method").and_then(Value::as_str) != Some("slotNotification") {
            continue;
        }

        let slot = payload
            .pointer("/params/result/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| "slot notification missing slot".to_owned())?;

        observed += 1;
        println!("slot_observation: slot={slot}");
    }

    println!("READ-ONLY LIVE SLOT PASS");
    Ok(())
}

async fn observe_raydium<S>(
    rpc_client: &Client,
    reader: &mut S,
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    ),
    String,
>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    println!("\nVenue adapter: Raydium CPMM");

    let mut states = Vec::new();
    let mut quote_contexts = BTreeMap::new();
    let mut observed = 0usize;

    while observed < MAX_RAYDIUM_OBSERVATIONS {
        let payload = next_json_message(reader).await?;

        let observation = match raydium::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("raydium_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "raydium_observation: pool={} slot={} {}",
            observation.pubkey,
            observation.slot,
            observation.pool_state.summary()
        );

        match hydrate_raydium_observation(rpc_client, &observation).await {
            Ok((normalized, snapshot)) => {
                println!("raydium_hydration: {}", snapshot.summary());
                println!("raydium_normalized_pool: {}", normalized.summary());

                if normalized_pool_is_eligible(&normalized) {
                    quote_contexts.insert(normalized.pool_id.clone(), snapshot);
                    states.push(normalized);
                }
            }
            Err(error) => {
                println!(
                    "raydium_hydration_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("raydium_live_observation_count={observed}");
    println!("raydium_live_eligible_count={}", states.len());
    println!("READ-ONLY RAYDIUM CPMM OBSERVATION PASS");

    Ok((states, quote_contexts))
}

async fn observe_pumpswap<S>(
    rpc_client: &Client,
    reader: &mut S,
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    ),
    String,
>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    println!("\nVenue adapter: PumpSwap");

    let mut states = Vec::new();
    let mut quote_contexts = BTreeMap::new();
    let mut observed = 0usize;

    while observed < MAX_PUMPSWAP_OBSERVATIONS {
        let payload = next_json_message(reader).await?;

        let observation = match pumpswap::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("pumpswap_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "pumpswap_observation: pool={} slot={} {}",
            observation.pubkey,
            observation.slot,
            observation.pool_state.summary()
        );

        match hydrate_pumpswap_observation(rpc_client, &observation).await {
            Ok((normalized, snapshot)) => {
                println!("pumpswap_hydration: {}", snapshot.summary());
                println!("pumpswap_normalized_pool: {}", normalized.summary());

                if normalized_pool_is_eligible(&normalized) {
                    quote_contexts.insert(normalized.pool_id.clone(), snapshot);
                    states.push(normalized);
                }
            }
            Err(error) => {
                println!(
                    "pumpswap_hydration_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("pumpswap_live_observation_count={observed}");
    println!("pumpswap_live_eligible_count={}", states.len());
    println!("READ-ONLY PUMPSWAP OBSERVATION PASS");

    Ok((states, quote_contexts))
}

async fn discover_deterministic_cross_venue_overlap(
    rpc_client: &Client,
    live_raydium_states: &[NormalizedPoolState],
    live_raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
        Vec<NormalizedPoolState>,
        BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    ),
    String,
> {
    println!("Solana on-chain state remains authoritative.");

    let mut seen_live_pairs = BTreeSet::new();
    let mut exact_pair_count = 0usize;

    for raydium_normalized in live_raydium_states {
        let Some((anchor_mint, intermediate_mint)) = anchor_pair_from_pool(raydium_normalized)
        else {
            continue;
        };

        if !seen_live_pairs.insert((anchor_mint.clone(), intermediate_mint.clone())) {
            continue;
        }

        exact_pair_count += 1;

        if exact_pair_count > MAX_TARGETED_ROUTE_LOOKUPS {
            break;
        }

        let Some(raydium_snapshot) = live_raydium_quote_contexts.get(&raydium_normalized.pool_id)
        else {
            println!(
                "deterministic_exact_pair_probe_rejected: raydium_pool={} reason=missing live quote context",
                raydium_normalized.pool_id
            );
            continue;
        };

        println!(
            concat!(
                "deterministic_exact_pair_probe_start: anchor={} intermediate={} ",
                "raydium_pool={}"
            ),
            anchor_mint, intermediate_mint, raydium_normalized.pool_id
        );

        for pumpswap_request in pumpswap::pair_lookup_requests(&anchor_mint, &intermediate_mint) {
            let label = format!(
                "PumpSwap exact-pair lookup anchor={anchor_mint} intermediate={intermediate_mint}"
            );

            let pumpswap_payload = match fetch_program_accounts(
                rpc_client,
                &pumpswap_request,
                &label,
            )
            .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    println!(
                        "deterministic_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                        anchor_mint, intermediate_mint
                    );
                    continue;
                }
            };

            let pumpswap_observations = match pumpswap::parse_pair_lookup_response(
                &pumpswap_payload,
            ) {
                Ok(observations) => observations,
                Err(error) => {
                    println!(
                        "deterministic_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                        anchor_mint, intermediate_mint
                    );
                    continue;
                }
            };

            println!(
                "deterministic_exact_pair_lookup_parsed: anchor={} intermediate={} observation_count={}",
                anchor_mint,
                intermediate_mint,
                pumpswap_observations.len()
            );

            for pumpswap_observation in pumpswap_observations {
                if !pumpswap_observation_matches_pair(
                    &pumpswap_observation,
                    &anchor_mint,
                    &intermediate_mint,
                ) {
                    continue;
                }

                let pumpswap_hydration_result =
                    hydrate_pumpswap_observation(rpc_client, &pumpswap_observation).await;

                let (pumpswap_normalized, pumpswap_snapshot) = match pumpswap_hydration_result {
                    Ok(result) => result,
                    Err(error) => {
                        println!(
                            "deterministic_exact_pair_candidate_rejected: pool={} reason={error}",
                            pumpswap_observation.pubkey
                        );
                        continue;
                    }
                };

                if !normalized_pool_is_eligible(&pumpswap_normalized) {
                    continue;
                }

                println!(
                    concat!(
                        "deterministic_cross_venue_overlap: anchor={} intermediate={} ",
                        "raydium_pool={} pumpswap_pool={} source=live-exact-pair"
                    ),
                    anchor_mint,
                    intermediate_mint,
                    raydium_normalized.pool_id,
                    pumpswap_normalized.pool_id,
                );
                println!("READ-ONLY RUNG 9 DETERMINISTIC DISCOVERY PASS");

                let mut raydium_contexts = BTreeMap::new();
                raydium_contexts
                    .insert(raydium_normalized.pool_id.clone(), raydium_snapshot.clone());

                let mut pumpswap_contexts = BTreeMap::new();
                pumpswap_contexts.insert(pumpswap_normalized.pool_id.clone(), pumpswap_snapshot);

                return Ok((
                    vec![raydium_normalized.clone()],
                    raydium_contexts,
                    vec![pumpswap_normalized],
                    pumpswap_contexts,
                ));
            }
        }
    }

    println!(
        "deterministic_exact_pair_probe_exhausted: unique_pairs={}",
        seen_live_pairs.len()
    );
    println!("deterministic_broad_anchor_fallback_start");

    let mut seen_raydium_pools = BTreeSet::new();
    let mut pair_count = 0usize;
    let mut successful_anchor_responses = 0usize;

    for request in raydium_anchor_lookup_requests() {
        let request_id = request.get("id").and_then(Value::as_u64).unwrap_or(0);

        let payload =
            match fetch_program_accounts(rpc_client, &request, "Raydium anchor lookup").await {
                Ok(payload) => payload,
                Err(error) => {
                    println!("raydium_anchor_lookup_rejected: id={request_id} reason={error}");
                    continue;
                }
            };

        let observations = match parse_raydium_anchor_lookup_response(&payload) {
            Ok(observations) => {
                successful_anchor_responses += 1;
                observations
            }
            Err(error) => {
                println!("raydium_anchor_lookup_rejected: id={request_id} reason={error}");
                continue;
            }
        };

        println!(
            "raydium_anchor_lookup_parsed: id={} observation_count={}",
            request_id,
            observations.len()
        );

        for observation in observations {
            if !seen_raydium_pools.insert(observation.pubkey.clone()) {
                continue;
            }

            let Some(discovery_candidate) = route_candidate_from_observation(observation) else {
                continue;
            };

            pair_count += 1;

            if pair_count > MAX_TARGETED_ROUTE_LOOKUPS {
                break;
            }

            println!(
                concat!(
                    "targeted_raydium_candidate_start: anchor={} intermediate={} ",
                    "pool={} candidate={}/{}"
                ),
                discovery_candidate.anchor_mint,
                discovery_candidate.intermediate_mint,
                discovery_candidate.observation.pubkey,
                pair_count,
                MAX_TARGETED_ROUTE_LOOKUPS
            );

            let (raydium_normalized, raydium_snapshot) =
                match hydrate_raydium_observation(rpc_client, &discovery_candidate.observation)
                    .await
                {
                    Ok(result) => result,
                    Err(error) => {
                        println!(
                            "targeted_raydium_candidate_rejected: pool={} reason={error}",
                            discovery_candidate.observation.pubkey
                        );
                        continue;
                    }
                };

            if !normalized_pool_is_eligible(&raydium_normalized) {
                continue;
            }

            let pumpswap_requests = pumpswap::pair_lookup_requests(
                &discovery_candidate.anchor_mint,
                &discovery_candidate.intermediate_mint,
            );

            for pumpswap_request in pumpswap_requests {
                let label = format!(
                    "PumpSwap pair lookup anchor={} intermediate={}",
                    discovery_candidate.anchor_mint, discovery_candidate.intermediate_mint
                );

                let pumpswap_payload = match fetch_program_accounts(
                    rpc_client,
                    &pumpswap_request,
                    &label,
      
