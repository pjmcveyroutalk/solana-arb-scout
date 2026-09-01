mod discovery;
mod pumpswap;
mod quote;
mod raydium;
mod registry;
mod route;
mod sizing;

use discovery::{
    parse_raydium_anchor_lookup_response, raydium_anchor_lookup_requests,
    route_candidate_from_observation,
};
use futures_util::{SinkExt, StreamExt};
use quote::{quote_two_leg_exact_input, VenueQuoteContext};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::{generate_two_leg_routes, RouteLeg};
use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState};
use serde_json::{json, Value};
use sizing::{
    parse_sol_usd_price, sol_usd_price_request, usd_dollars_to_anchor_raw, SolUsdPrice,
    USD_SIZE_GRID,
};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{timeout, Duration};
use tokio_tungstenite::{
    connect_async,
    tungstenite::{client::IntoClientRequest, Message},
};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const SOLANA_WS_URL: &str = "wss://api.mainnet-beta.solana.com";
const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(15);
const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 15;
const MAX_TARGETED_ROUTE_LOOKUPS: usize = 15;

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteDiscoveryPair {
    anchor_mint: String,
    intermediate_mint: String,
    raydium_pool_id: String,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    let rpc_client = Client::new();

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
        ) = discover_deterministic_cross_venue_overlap(&rpc_client).await?;

        merge_normalized_states(&mut raydium_states, discovered_raydium_states);
        merge_quote_contexts(&mut raydium_quote_contexts, discovered_raydium_contexts);
        merge_normalized_states(&mut pumpswap_states, discovered_pumpswap_states);
        merge_quote_contexts(&mut pumpswap_quote_contexts, discovered_pumpswap_contexts);
    }

    let sol_usd_price = fetch_sol_usd_price(&rpc_client).await?;

    validate_registry_routes_and_sizes(
        raydium_states,
        pumpswap_states,
        &raydium_quote_contexts,
        &pumpswap_quote_contexts,
        &sol_usd_price,
    )
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

    let mut seen_raydium_pools = BTreeSet::new();
    let mut pair_count = 0usize;
    let mut successful_anchor_responses = 0usize;

    for request in raydium_anchor_lookup_requests() {
        let payload =
            match fetch_program_accounts(rpc_client, &request, "Raydium anchor lookup").await {
                Ok(payload) => payload,
                Err(error) => {
                    println!("raydium_anchor_lookup_rejected: {error}");
                    continue;
                }
            };

        let observations = match parse_raydium_anchor_lookup_response(&payload) {
            Ok(observations) => {
                successful_anchor_responses += 1;
                observations
            }
            Err(error) => {
                println!("raydium_anchor_lookup_rejected: {error}");
                continue;
            }
        };

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
                let pumpswap_payload = match fetch_program_accounts(
                    rpc_client,
                    &pumpswap_request,
                    "PumpSwap pair lookup",
                )
                .await
                {
                    Ok(payload) => payload,
                    Err(error) => {
                        println!(
                            "targeted_pumpswap_lookup_rejected: anchor={} intermediate={} reason={error}",
                            discovery_candidate.anchor_mint,
                            discovery_candidate.intermediate_mint
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
                            "targeted_pumpswap_lookup_rejected: anchor={} intermediate={} reason={error}",
                            discovery_candidate.anchor_mint,
                            discovery_candidate.intermediate_mint
                        );
                        continue;
                    }
                };

                for pumpswap_observation in pumpswap_observations {
                    if !pumpswap_observation_matches_pair(
                        &pumpswap_observation,
                        &discovery_candidate.anchor_mint,
                        &discovery_candidate.intermediate_mint,
                    ) {
                        continue;
                    }

                    let (pumpswap_normalized, pumpswap_snapshot) =
                        match hydrate_pumpswap_observation(rpc_client, &pumpswap_observation).await
                        {
                            Ok(result) => result,
                            Err(error) => {
                                println!(
                                    "targeted_pumpswap_candidate_rejected: pool={} reason={error}",
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
                            "raydium_pool={} pumpswap_pool={}"
                        ),
                        discovery_candidate.anchor_mint,
                        discovery_candidate.intermediate_mint,
                        raydium_normalized.pool_id,
                        pumpswap_normalized.pool_id,
                    );
                    println!("READ-ONLY RUNG 9 DETERMINISTIC DISCOVERY PASS");

                    let mut raydium_contexts = BTreeMap::new();
                    raydium_contexts.insert(raydium_normalized.pool_id.clone(), raydium_snapshot);

                    let mut pumpswap_contexts = BTreeMap::new();
                    pumpswap_contexts
                        .insert(pumpswap_normalized.pool_id.clone(), pumpswap_snapshot);

                    return Ok((
                        vec![raydium_normalized],
                        raydium_contexts,
                        vec![pumpswap_normalized],
                        pumpswap_contexts,
                    ));
                }
            }
        }

        if pair_count >= MAX_TARGETED_ROUTE_LOOKUPS {
            break;
        }
    }

    if successful_anchor_responses == 0 {
        return Err(
            "all bounded Raydium anchor lookup requests failed before a valid RPC response was parsed"
                .to_owned(),
        );
    }

    Err(
        "Rung 9 deterministic discovery found no live Raydium-PumpSwap same-pair overlap"
            .to_owned(),
    )
}

async fn hydrate_raydium_observation(
    rpc_client: &Client,
    observation: &raydium::RaydiumCpmmAccountObservation,
) -> Result<(NormalizedPoolState, raydium::RaydiumHydrationSnapshot), String> {
    let received_at = unix_time_ms_now()?;
    let request_accounts = raydium::hydration_account_pubkeys(observation);
    let payload = fetch_hydration(
        rpc_client,
        3,
        request_accounts,
        observation.slot,
        "Raydium CPMM",
    )
    .await?;
    let snapshot = raydium::parse_hydration_response(observation, &payload)?;
    let hydrated_at = unix_time_ms_now()?;
    let normalized =
        raydium::hydrate_normalized_observation(observation, &snapshot, received_at, hydrated_at)?;

    Ok((normalized, snapshot))
}

async fn hydrate_pumpswap_observation(
    rpc_client: &Client,
    observation: &pumpswap::PumpSwapAccountObservation,
) -> Result<(NormalizedPoolState, pumpswap::PumpSwapHydrationSnapshot), String> {
    let received_at = unix_time_ms_now()?;
    let request_accounts = pumpswap::hydration_account_pubkeys(observation);
    let payload = fetch_hydration(
        rpc_client,
        5,
        request_accounts,
        observation.slot,
        "PumpSwap",
    )
    .await?;
    let snapshot = pumpswap::parse_hydration_response(observation, &payload)?;
    let hydrated_at = unix_time_ms_now()?;
    let normalized =
        pumpswap::hydrate_normalized_observation(observation, &snapshot, received_at, hydrated_at)?;

    Ok((normalized, snapshot))
}

async fn fetch_program_accounts(
    rpc_client: &Client,
    request: &Value,
    label: &str,
) -> Result<Value, String> {
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
        .map_err(|error| format!("{label} RPC returned invalid JSON: {error}"))
}

async fn fetch_sol_usd_price(rpc_client: &Client) -> Result<SolUsdPrice, String> {
    println!("\nRung 10 sizing oracle: Pyth SOL/USD");

    let request = sol_usd_price_request();

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("Pyth SOL/USD RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!("Pyth SOL/USD RPC returned HTTP status {status}"));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Pyth SOL/USD RPC returned invalid JSON: {error}"))?;

    let now = unix_time_seconds_now()?;
    let price = parse_sol_usd_price(&payload, now)?;

    println!("sol_usd_price: {}", price.summary());
    println!("READ-ONLY PYTH SOL/USD VALIDATION PASS");

    Ok(price)
}

fn merge_normalized_states(
    destination: &mut Vec<NormalizedPoolState>,
    additions: Vec<NormalizedPoolState>,
) {
    for state in additions {
        if let Some(existing) = destination
            .iter_mut()
            .find(|existing| existing.pool_id == state.pool_id)
        {
            if state.source_slot >= existing.source_slot {
                *existing = state;
            }
        } else {
            destination.push(state);
        }
    }
}

fn merge_quote_contexts<T>(destination: &mut BTreeMap<String, T>, additions: BTreeMap<String, T>) {
    for (pool_id, context) in additions {
        destination.insert(pool_id, context);
    }
}

fn pumpswap_observation_matches_pair(
    observation: &pumpswap::PumpSwapAccountObservation,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> bool {
    let base_mint = observation.pool_state.base_mint.as_str();
    let quote_mint = observation.pool_state.quote_mint.as_str();

    (base_mint == anchor_mint && quote_mint == intermediate_mint)
        || (quote_mint == anchor_mint && base_mint == intermediate_mint)
}

fn normalized_pool_is_eligible(pool: &NormalizedPoolState) -> bool {
    if pool.trading_state != PoolTradingState::Tradable {
        return false;
    }

    matches!(
        &pool.quote_reserves,
        QuoteReserveState::Available {
            token_a_raw,
            token_b_raw,
            ..
        } if *token_a_raw > 0 && *token_b_raw > 0
    )
}

fn validate_registry_routes_and_sizes(
    raydium_states: Vec<NormalizedPoolState>,
    pumpswap_states: Vec<NormalizedPoolState>,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    sol_usd_price: &SolUsdPrice,
) -> Result<(), String> {
    println!("\nRegistry: Active Mint");

    let mut registry = ActiveMintRegistry::new();

    for state in raydium_states.into_iter().chain(pumpswap_states) {
        registry.upsert(state);
    }

    let active_mints = registry.active_mints();

    println!("registry_current_pools={}", registry.current_pool_count());

    if active_mints.is_empty() {
        return Err("live state produced no mint represented by two eligible venues".to_owned());
    }

    for active_mint in &active_mints {
        println!("active_mint: {}", active_mint.summary());
    }

    println!("READ-ONLY ACTIVE-MINT REGISTRY PASS");
    println!("READ-ONLY RUNG 8 ACTIVE-MINT DETECTION PASS");

    println!("\nRoute engine: Two-Leg Circular");

    let eligible_pools = registry.current_eligible_pools();
    let route_candidates = generate_two_leg_routes(&eligible_pools);

    println!("route_candidate_count={}", route_candidates.len());

    for route_candidate in &route_candidates {
        println!("route_candidate: {}", route_candidate.summary());
    }

    if route_candidates.is_empty() {
        return Err(
            "Rung 9 Gate C requires at least one real same-pair cross-venue route".to_owned(),
        );
    }

    println!("READ-ONLY TWO-LEG ROUTE ENGINE PASS");
    println!("READ-ONLY RUNG 9 ROUTE CANDIDATE PASS");

    println!("\nRung 10 deterministic USD size-grid quote engine");

    let mut successful_routes = 0usize;
    let mut successful_grid_quotes = 0usize;

    for route_candidate in &route_candidates {
        let leg_1_context = match quote_context_for_leg(
            route_candidate.leg_1(),
            raydium_quote_contexts,
            pumpswap_quote_contexts,
        ) {
            Ok(context) => context,
            Err(error) => {
                println!(
                    "rung10_route_rejected: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                continue;
            }
        };

        let leg_2_context = match quote_context_for_leg(
            route_candidate.leg_2(),
            raydium_quote_contexts,
            pumpswap_quote_contexts,
        ) {
            Ok(context) => context,
            Err(error) => {
                println!(
                    "rung10_route_rejected: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                continue;
            }
        };

        let anchor_decimals =
            match context_mint_decimals(&leg_1_context, route_candidate.anchor_mint()) {
                Ok(decimals) => decimals,
                Err(error) => {
                    println!(
                        "rung10_route_rejected: route=[{}] reason={error}",
                        route_candidate.summary()
                    );
                    continue;
                }
            };

        let mut route_grid_quotes = 0usize;

        for dollars in USD_SIZE_GRID {
            let amount_in_raw = match usd_dollars_to_anchor_raw(
                dollars,
                route_candidate.anchor_mint(),
                anchor_decimals,
                Some(sol_usd_price),
            ) {
                Ok(amount) => amount,
                Err(error) => {
                    println!(
                        "rung10_size_rejected: route=[{}] usd=${dollars} reason={error}",
                        route_candidate.summary()
                    );
                    continue;
                }
            };

            match quote_two_leg_exact_input(
                route_candidate,
                amount_in_raw,
                &leg_1_context,
                &leg_2_context,
            ) {
                Ok(route_quote) => {
                    route_grid_quotes += 1;
                    successful_grid_quotes += 1;
                    println!(
                        "rung10_grid_quote: usd=${dollars} {}",
                        route_quote.summary()
                    );
                }
                Err(error) => {
                    println!(
                        "rung10_grid_quote_rejected: route=[{}] usd=${dollars} amount_in_raw={amount_in_raw} reason={error}",
                        route_candidate.summary()
                    );
                }
            }
        }

        if route_grid_quotes == USD_SIZE_GRID.len() {
            successful_routes += 1;
            println!("rung10_complete_grid_route: {}", route_candidate.summary());
        }
    }

    if successful_routes == 0 {
        return Err(
            "Rung 10 produced no route with the complete $1-$1,000 deterministic quote grid"
                .to_owned(),
        );
    }

    println!("rung10_complete_grid_route_count={successful_routes}");
    println!("rung10_successful_grid_quote_count={successful_grid_quotes}");
    println!("READ-ONLY RUNG 10 SIZE + QUOTE ENGINE PASS");

    Ok(())
}

fn quote_context_for_leg<'a>(
    leg: &RouteLeg,
    raydium_quote_contexts: &'a BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &'a BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
) -> Result<VenueQuoteContext<'a>, String> {
    match leg.venue() {
        scout_core::Venue::RaydiumCpmm => raydium_quote_contexts
            .get(leg.pool_id())
            .map(|snapshot| VenueQuoteContext::Raydium {
                pool_id: leg.pool_id().to_owned(),
                snapshot,
            })
            .ok_or_else(|| {
                format!(
                    "missing Raydium quote context for route pool {}",
                    leg.pool_id()
                )
            }),
        scout_core::Venue::PumpSwap => pumpswap_quote_contexts
            .get(leg.pool_id())
            .map(|snapshot| VenueQuoteContext::PumpSwap {
                pool_id: leg.pool_id().to_owned(),
                snapshot,
            })
            .ok_or_else(|| {
                format!(
                    "missing PumpSwap quote context for route pool {}",
                    leg.pool_id()
                )
            }),
        other => Err(format!("unsupported Rung 10 quote venue {}", other.label())),
    }
}

fn context_mint_decimals(context: &VenueQuoteContext<'_>, mint: &str) -> Result<u8, String> {
    match context {
        VenueQuoteContext::Raydium { snapshot, .. } => {
            if snapshot.pool_state.token_0_mint == mint {
                Ok(snapshot.pool_state.mint_0_decimals)
            } else if snapshot.pool_state.token_1_mint == mint {
                Ok(snapshot.pool_state.mint_1_decimals)
            } else {
                Err(format!("mint {mint} is not in Raydium quote context"))
            }
        }
        VenueQuoteContext::PumpSwap { snapshot, .. } => {
            if snapshot.pool_state.base_mint == mint {
                Ok(snapshot.base_decimals)
            } else if snapshot.pool_state.quote_mint == mint {
                Ok(snapshot.quote_decimals)
            } else {
                Err(format!("mint {mint} is not in PumpSwap quote context"))
            }
        }
    }
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

fn unix_time_seconds_now() -> Result<i64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before Unix epoch: {error}"))?;

    i64::try_from(duration.as_secs()).map_err(|_| "Unix timestamp seconds exceeded i64".to_owned())
}

async fn wait_for_subscription_confirmation<S>(
    reader: &mut S,
    request_id: u64,
    label: &str,
) -> Result<(), String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    loop {
        let payload = next_json_message(reader).await?;

        if payload.get("id").and_then(Value::as_u64) != Some(request_id) {
            continue;
        }

        if let Some(error) = payload.get("error") {
            return Err(format!("{label} subscription rejected: {error}"));
        }

        let subscription_id = payload
            .get("result")
            .and_then(Value::as_u64)
            .ok_or_else(|| format!("{label} subscription response missing id"))?;

        println!("{label}_subscription_id={subscription_id}");
        return Ok(());
    }
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
