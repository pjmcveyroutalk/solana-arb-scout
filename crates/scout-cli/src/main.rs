mod pumpswap;
mod quote;
mod raydium;
mod registry;
mod route;

use futures_util::{SinkExt, StreamExt};
use quote::{one_whole_anchor_input_raw, quote_two_leg_exact_input, VenueQuoteContext};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::{generate_two_leg_routes, RouteLeg, USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState};
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SOLANA_WSS_URL: &str = "wss://api.mainnet-beta.solana.com";
const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";

const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 15;
const MAX_TARGETED_ROUTE_LOOKUPS: usize = 15;

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteDiscoveryPair {
    anchor_mint: String,
    intermediate_mint: String,
    raydium_pool_id: String,
}

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

    let (raydium_states, raydium_quote_contexts) = match observe_raydium_cpmm(&rpc_client).await {
        Ok(result) => result,
        Err(error) => {
            eprintln!("Raydium CPMM observation failed: {error}");
            std::process::exit(1);
        }
    };

    let (mut pumpswap_states, mut pumpswap_quote_contexts) =
        match observe_pumpswap(&rpc_client).await {
            Ok(result) => result,
            Err(error) => {
                eprintln!("PumpSwap observation failed: {error}");
                std::process::exit(1);
            }
        };

    if !has_current_route(&raydium_states, &pumpswap_states) {
        println!(
            "\nREAD-ONLY RUNG 9 DISCOVERY: bounded live sample contains no same-pair cross-venue route"
        );

        let (targeted_states, targeted_quote_contexts) =
            match discover_targeted_pumpswap_overlap(&rpc_client, &raydium_states).await {
                Ok(result) => result,
                Err(error) => {
                    eprintln!("Rung 9 targeted discovery failed: {error}");
                    std::process::exit(1);
                }
            };

        println!("targeted_pumpswap_state_count={}", targeted_states.len());

        pumpswap_states.extend(targeted_states);
        pumpswap_quote_contexts.extend(targeted_quote_contexts);
    }

    if let Err(error) = validate_registry_and_routes(
        raydium_states,
        pumpswap_states,
        &raydium_quote_contexts,
        &pumpswap_quote_contexts,
    ) {
        eprintln!("Registry/route validation failed: {error}");
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

async fn observe_raydium_cpmm(
    rpc_client: &Client,
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    ),
    String,
> {
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
    let mut quote_contexts = BTreeMap::new();

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

        quote_contexts.insert(normalized.pool_id.clone(), snapshot);
        normalized_states.push(normalized);
    }

    if !subscription_confirmed {
        return Err("Raydium CPMM subscription was never confirmed".to_owned());
    }

    println!("READ-ONLY RAYDIUM CPMM ACCOUNT DECODER PASS");
    println!("READ-ONLY NORMALIZED POOL STATE PASS");
    println!("READ-ONLY RAYDIUM VAULT HYDRATION PASS");
    println!("READ-ONLY RUNG 6 OBSERVATION COLLECTION PASS");

    Ok((normalized_states, quote_contexts))
}

async fn fetch_raydium_hydration(
    rpc_client: &Client,
    observation: &raydium::RaydiumCpmmAccountObservation,
) -> Result<Value, String> {
    let account_pubkeys = raydium::hydration_account_pubkeys(observation);

    fetch_hydration(rpc_client, 3, account_pubkeys, observation.slot, "Raydium").await
}

async fn observe_pumpswap(
    rpc_client: &Client,
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    ),
    String,
> {
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
    let mut quote_contexts = BTreeMap::new();

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

        quote_contexts.insert(normalized.pool_id.clone(), snapshot);
        normalized_states.push(normalized);
    }

    if !subscription_confirmed {
        return Err("PumpSwap subscription was never confirmed".to_owned());
    }

    println!("READ-ONLY PUMPSWAP ACCOUNT DECODER PASS");
    println!("READ-ONLY PUMPSWAP SNAPSHOT HYDRATION PASS");
    println!("READ-ONLY SECOND VENUE NORMALIZATION PASS");
    println!("READ-ONLY RUNG 7 OBSERVATION COLLECTION PASS");

    Ok((normalized_states, quote_contexts))
}

async fn fetch_pumpswap_hydration(
    rpc_client: &Client,
    observation: &pumpswap::PumpSwapAccountObservation,
) -> Result<Value, String> {
    let account_pubkeys = pumpswap::hydration_account_pubkeys(observation);

    fetch_hydration(rpc_client, 5, account_pubkeys, observation.slot, "PumpSwap").await
}

fn has_current_route(
    raydium_states: &[NormalizedPoolState],
    pumpswap_states: &[NormalizedPoolState],
) -> bool {
    let mut registry = ActiveMintRegistry::new();

    for state in raydium_states.iter().chain(pumpswap_states).cloned() {
        registry.upsert(state);
    }

    !generate_two_leg_routes(&registry.current_eligible_pools()).is_empty()
}

async fn discover_targeted_pumpswap_overlap(
    rpc_client: &Client,
    raydium_states: &[NormalizedPoolState],
) -> Result<
    (
        Vec<NormalizedPoolState>,
        BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    ),
    String,
> {
    println!("\nRung 9 targeted PumpSwap discovery");
    println!("Solana on-chain state remains authoritative.");

    let candidates = collect_route_discovery_pairs(raydium_states);

    println!("targeted_lookup_pair_count={}", candidates.len());

    if candidates.is_empty() {
        return Ok((Vec::new(), BTreeMap::new()));
    }

    let mut successful_rpc_responses = 0usize;

    for candidate in candidates {
        let requests =
            pumpswap::pair_lookup_requests(&candidate.anchor_mint, &candidate.intermediate_mint);

        for request in requests {
            let payload = match fetch_pumpswap_pair_lookup(rpc_client, &request).await {
                Ok(payload) => payload,
                Err(error) => {
                    println!(
                        "targeted_pumpswap_lookup_rejected: anchor={} intermediate={} raydium_pool={} reason={}",
                        candidate.anchor_mint,
                        candidate.intermediate_mint,
                        candidate.raydium_pool_id,
                        error,
                    );
                    continue;
                }
            };

            let observations = match pumpswap::parse_pair_lookup_response(&payload) {
                Ok(observations) => {
                    successful_rpc_responses += 1;
                    observations
                }
                Err(error) => {
                    println!(
                        "targeted_pumpswap_lookup_rejected: anchor={} intermediate={} raydium_pool={} reason={}",
                        candidate.anchor_mint,
                        candidate.intermediate_mint,
                        candidate.raydium_pool_id,
                        error,
                    );
                    continue;
                }
            };

            if observations.is_empty() {
                continue;
            }

            for observation in observations {
                if !pumpswap_observation_matches_pair(
                    &observation,
                    &candidate.anchor_mint,
                    &candidate.intermediate_mint,
                ) {
                    println!(
                        "targeted_pumpswap_candidate_rejected: pool={} reason=on-chain mint pair mismatch",
                        observation.pubkey
                    );
                    continue;
                }

                let account_update_received_at_unix_ms = unix_time_ms_now()?;
                let hydration_started_at_unix_ms = unix_time_ms_now()?;

                let hydration_payload =
                    match fetch_pumpswap_hydration(rpc_client, &observation).await {
                        Ok(payload) => payload,
                        Err(error) => {
                            println!(
                                "targeted_pumpswap_candidate_rejected: pool={} reason={}",
                                observation.pubkey, error
                            );
                            continue;
                        }
                    };

                let snapshot =
                    match pumpswap::parse_hydration_response(&observation, &hydration_payload) {
                        Ok(snapshot) => snapshot,
                        Err(error) => {
                            println!(
                                "targeted_pumpswap_candidate_rejected: pool={} reason={}",
                                observation.pubkey, error
                            );
                            continue;
                        }
                    };

                let hydrated_at_unix_ms = unix_time_ms_now()?;

                let normalized = match pumpswap::hydrate_normalized_observation(
                    &observation,
                    &snapshot,
                    account_update_received_at_unix_ms,
                    hydrated_at_unix_ms,
                ) {
                    Ok(normalized) => normalized,
                    Err(error) => {
                        println!(
                            "targeted_pumpswap_candidate_rejected: pool={} reason={}",
                            observation.pubkey, error
                        );
                        continue;
                    }
                };

                if !normalized_pool_is_eligible(&normalized) {
                    println!(
                        "targeted_pumpswap_candidate_rejected: pool={} reason=current normalized state is not registry-eligible",
                        observation.pubkey
                    );
                    continue;
                }

                let hydration_duration_ms =
                    hydrated_at_unix_ms.saturating_sub(hydration_started_at_unix_ms);

                println!(
                    "targeted_pumpswap_pool: anchor={} intermediate={} raydium_pool={} pumpswap_pool={} source_slot={} reserve_slot={} hydration_duration_ms={}",
                    candidate.anchor_mint,
                    candidate.intermediate_mint,
                    candidate.raydium_pool_id,
                    normalized.pool_id,
                    normalized.source_slot,
                    snapshot.slot,
                    hydration_duration_ms,
                );
                println!("targeted_normalized_pool: {}", normalized.summary());
                println!("READ-ONLY RUNG 9 TARGETED DISCOVERY PASS");

                let mut quote_contexts = BTreeMap::new();
                quote_contexts.insert(normalized.pool_id.clone(), snapshot);

                return Ok((vec![normalized], quote_contexts));
            }
        }

        println!(
            "targeted_pumpswap_no_pair: anchor={} intermediate={} raydium_pool={}",
            candidate.anchor_mint, candidate.intermediate_mint, candidate.raydium_pool_id
        );
    }

    if successful_rpc_responses == 0 {
        return Err(
            "all bounded PumpSwap pair lookup requests failed before a valid RPC response was parsed"
                .to_owned(),
        );
    }

    Ok((Vec::new(), BTreeMap::new()))
}

async fn fetch_pumpswap_pair_lookup(rpc_client: &Client, request: &Value) -> Result<Value, String> {
    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(request)
        .send()
        .await
        .map_err(|error| format!("PumpSwap pair lookup RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "PumpSwap pair lookup RPC returned HTTP status {status}"
        ));
    }

    response
        .json::<Value>()
        .await
        .map_err(|error| format!("PumpSwap pair lookup RPC returned invalid JSON: {error}"))
}

fn collect_route_discovery_pairs(
    raydium_states: &[NormalizedPoolState],
) -> Vec<RouteDiscoveryPair> {
    let mut pairs = Vec::new();

    for pool in raydium_states {
        if !normalized_pool_is_eligible(pool) {
            continue;
        }

        let Some((anchor_mint, intermediate_mint)) = route_pair_from_pool(pool) else {
            continue;
        };

        if pairs.iter().any(|existing: &RouteDiscoveryPair| {
            existing.anchor_mint == anchor_mint && existing.intermediate_mint == intermediate_mint
        }) {
            continue;
        }

        pairs.push(RouteDiscoveryPair {
            anchor_mint,
            intermediate_mint,
            raydium_pool_id: pool.pool_id.clone(),
        });

        if pairs.len() >= MAX_TARGETED_ROUTE_LOOKUPS {
            break;
        }
    }

    pairs
}

fn route_pair_from_pool(pool: &NormalizedPoolState) -> Option<(String, String)> {
    for anchor_mint in [WRAPPED_SOL_MINT, USDC_MINT, USDT_MINT] {
        if pool.token_a.mint == anchor_mint && pool.token_b.mint != anchor_mint {
            return Some((anchor_mint.to_owned(), pool.token_b.mint.clone()));
        }

        if pool.token_b.mint == anchor_mint && pool.token_a.mint != anchor_mint {
            return Some((anchor_mint.to_owned(), pool.token_a.mint.clone()));
        }
    }

    None
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

fn validate_registry_and_routes(
    raydium_states: Vec<NormalizedPoolState>,
    pumpswap_states: Vec<NormalizedPoolState>,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
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
        println!(
            "READ-ONLY RUNG 9 DIAGNOSTIC: no permitted same-pair cross-venue route observed after targeted discovery"
        );

        return Err(
            "Rung 9 Gate C requires at least one real same-pair cross-venue route".to_owned(),
        );
    }

    println!("READ-ONLY TWO-LEG ROUTE ENGINE PASS");
    println!("READ-ONLY RUNG 9 ROUTE CANDIDATE PASS");

    println!("\nRung 10 raw quote engine: one whole anchor token probe");

    let mut successful_quotes = 0usize;

    for route_candidate in &route_candidates {
        let leg_1_context = match quote_context_for_leg(
            route_candidate.leg_1(),
            raydium_quote_contexts,
            pumpswap_quote_contexts,
        ) {
            Ok(context) => context,
            Err(error) => {
                println!(
                    "rung10_raw_quote_rejected: route=[{}] reason={error}",
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
                    "rung10_raw_quote_rejected: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                continue;
            }
        };

        let amount_in_raw = match one_whole_anchor_input_raw(route_candidate, &leg_1_context) {
            Ok(amount) => amount,
            Err(error) => {
                println!(
                    "rung10_raw_quote_rejected: route=[{}] reason={error}",
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
                successful_quotes += 1;
                println!("rung10_raw_quote: {}", route_quote.summary());
            }
            Err(error) => {
                println!(
                    "rung10_raw_quote_rejected: route=[{}] reason={error}",
                    route_candidate.summary()
                );
            }
        }
    }

    if successful_quotes == 0 {
        return Err(
            "Rung 10 raw quote engine produced no valid two-leg quote from live route state"
                .to_owned(),
        );
    }

    println!("rung10_raw_quote_count={successful_quotes}");
    println!("READ-ONLY RUNG 10 RAW ROUTE QUOTE ENGINE PASS");

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
        other => Err(format!(
            "unsupported Rung 10 quote venue {}",
            other.label()
        )),
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
