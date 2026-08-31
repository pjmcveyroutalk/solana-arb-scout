mod pumpswap;
mod raydium;
mod registry;
mod route;

use futures_util::{future::join_all, SinkExt, StreamExt};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::{generate_two_leg_routes, USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState};
use serde_json::{json, Value};
use std::time::{Duration, SystemTime, UNIX_EPOCH};
use tokio::time::timeout;
use tokio_tungstenite::{connect_async, tungstenite::Message};

const SOLANA_WSS_URL: &str = "wss://api.mainnet-beta.solana.com";
const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const RAYDIUM_API_POOL_MINT_URL: &str = "https://api-v3.raydium.io/pools/info/mint";

const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 15;
const MAX_TARGETED_ROUTE_LOOKUPS: usize = 15;
const TARGETED_LOOKUP_BATCH_SIZE: usize = 5;

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
const LOCATOR_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
struct RouteDiscoveryPair {
    anchor_mint: String,
    intermediate_mint: String,
    pumpswap_pool_id: String,
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

    let mut raydium_states = match observe_raydium_cpmm(&rpc_client).await {
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

    if !has_current_route(&raydium_states, &pumpswap_states) {
        println!(
            "\nREAD-ONLY RUNG 9 DISCOVERY: bounded live sample contains no same-pair cross-venue route"
        );

        let targeted_states =
            match discover_targeted_raydium_overlap(&rpc_client, &pumpswap_states).await {
                Ok(states) => states,
                Err(error) => {
                    eprintln!("Rung 9 targeted discovery failed: {error}");
                    std::process::exit(1);
                }
            };

        println!("targeted_raydium_state_count={}", targeted_states.len());

        raydium_states.extend(targeted_states);
    }

    if let Err(error) = validate_registry_and_routes(raydium_states, pumpswap_states) {
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

async fn discover_targeted_raydium_overlap(
    rpc_client: &Client,
    pumpswap_states: &[NormalizedPoolState],
) -> Result<Vec<NormalizedPoolState>, String> {
    println!("\nRung 9 targeted discovery");
    println!("Raydium API is locator-only; Solana on-chain state remains authoritative.");

    let candidates = collect_route_discovery_pairs(pumpswap_states);

    println!("targeted_lookup_pair_count={}", candidates.len());

    if candidates.is_empty() {
        return Ok(Vec::new());
    }

    let mut successful_locator_calls = 0usize;

    for batch in candidates.chunks(TARGETED_LOOKUP_BATCH_SIZE) {
        let lookup_results = join_all(batch.iter().map(|candidate| async move {
            let result = fetch_raydium_locator_pool_id(
                rpc_client,
                &candidate.anchor_mint,
                &candidate.intermediate_mint,
            )
            .await;

            (candidate, result)
        }))
        .await;

        for (candidate, lookup_result) in lookup_results {
            let pool_id = match lookup_result {
                Ok(Some(pool_id)) => {
                    successful_locator_calls += 1;
                    pool_id
                }
                Ok(None) => {
                    successful_locator_calls += 1;
                    continue;
                }
                Err(error) => {
                    println!(
                        "targeted_locator_rejected: anchor={} intermediate={} reason={}",
                        candidate.anchor_mint, candidate.intermediate_mint, error
                    );
                    continue;
                }
            };

            println!(
                "route_locator_candidate: anchor={} intermediate={} pumpswap_pool={} raydium_pool={}",
                candidate.anchor_mint,
                candidate.intermediate_mint,
                candidate.pumpswap_pool_id,
                pool_id,
            );

            let observation = match fetch_raydium_pool_observation(rpc_client, &pool_id).await {
                Ok(observation) => observation,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: pool={} reason={}",
                        pool_id, error
                    );
                    continue;
                }
            };

            if !raydium_observation_matches_pair(
                &observation,
                &candidate.anchor_mint,
                &candidate.intermediate_mint,
            ) {
                println!(
                    "targeted_onchain_candidate_rejected: pool={} reason=on-chain mint pair mismatch",
                    pool_id
                );
                continue;
            }

            let account_update_received_at_unix_ms = unix_time_ms_now()?;
            let hydration_started_at_unix_ms = unix_time_ms_now()?;

            let hydration_payload = match fetch_raydium_hydration(rpc_client, &observation).await {
                Ok(payload) => payload,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: pool={} reason={}",
                        pool_id, error
                    );
                    continue;
                }
            };

            let snapshot = match raydium::parse_hydration_response(&observation, &hydration_payload)
            {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: pool={} reason={}",
                        pool_id, error
                    );
                    continue;
                }
            };

            let hydrated_at_unix_ms = unix_time_ms_now()?;

            let normalized = match raydium::hydrate_normalized_observation(
                &observation,
                &snapshot,
                account_update_received_at_unix_ms,
                hydrated_at_unix_ms,
            ) {
                Ok(normalized) => normalized,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: pool={} reason={}",
                        pool_id, error
                    );
                    continue;
                }
            };

            if !normalized_pool_is_eligible(&normalized) {
                println!(
                    "targeted_onchain_candidate_rejected: pool={} reason=current normalized state is not registry-eligible",
                    pool_id
                );
                continue;
            }

            let hydration_duration_ms =
                hydrated_at_unix_ms.saturating_sub(hydration_started_at_unix_ms);

            println!(
                "targeted_raydium_pool: anchor={} intermediate={} pool={} source_slot={} reserve_slot={} hydration_duration_ms={}",
                candidate.anchor_mint,
                candidate.intermediate_mint,
                normalized.pool_id,
                normalized.source_slot,
                snapshot.slot,
                hydration_duration_ms,
            );
            println!("targeted_normalized_pool: {}", normalized.summary());
            println!("READ-ONLY RUNG 9 TARGETED DISCOVERY PASS");

            return Ok(vec![normalized]);
        }
    }

    if successful_locator_calls == 0 {
        return Err(
            "all bounded Raydium locator requests failed before a response was received".to_owned(),
        );
    }

    Ok(Vec::new())
}

fn collect_route_discovery_pairs(
    pumpswap_states: &[NormalizedPoolState],
) -> Vec<RouteDiscoveryPair> {
    let mut pairs = Vec::new();

    for pool in pumpswap_states {
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
            pumpswap_pool_id: pool.pool_id.clone(),
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

async fn fetch_raydium_locator_pool_id(
    rpc_client: &Client,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> Result<Option<String>, String> {
    let url = format!(
        "{RAYDIUM_API_POOL_MINT_URL}?mint1={anchor_mint}&mint2={intermediate_mint}&poolType=standard&poolSortField=liquidity&sortType=desc&pageSize=20&page=1"
    );

    let response = rpc_client
        .get(url)
        .timeout(LOCATOR_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("Raydium locator request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!("Raydium locator returned HTTP status {status}"));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Raydium locator returned invalid JSON: {error}"))?;

    raydium_locator_pool_id_from_payload(&payload)
}

fn raydium_locator_pool_id_from_payload(payload: &Value) -> Result<Option<String>, String> {
    if payload.get("success").and_then(Value::as_bool) != Some(true) {
        let message = payload
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("unknown Raydium API error");

        return Err(format!("Raydium locator rejected request: {message}"));
    }

    let pools = payload
        .pointer("/data/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Raydium locator response missing pool array".to_owned())?;

    for pool in pools {
        let program_id = pool.get("programId").and_then(Value::as_str);

        if program_id != Some(raydium::RAYDIUM_CPMM_PROGRAM_ID) {
            continue;
        }

        let pool_id = pool
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Raydium CPMM locator result missing pool id".to_owned())?;

        return Ok(Some(pool_id.to_owned()));
    }

    Ok(None)
}

async fn fetch_raydium_pool_observation(
    rpc_client: &Client,
    pool_id: &str,
) -> Result<raydium::RaydiumCpmmAccountObservation, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 6,
        "method": "getAccountInfo",
        "params": [
            pool_id,
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
        .map_err(|error| format!("targeted Raydium pool RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "targeted Raydium pool RPC returned HTTP status {status}"
        ));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("targeted Raydium pool RPC returned invalid JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        return Err(format!(
            "targeted Raydium getAccountInfo returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "targeted Raydium getAccountInfo missing context slot".to_owned())?;

    let account = payload
        .pointer("/result/value")
        .ok_or_else(|| "targeted Raydium getAccountInfo missing account value".to_owned())?;

    if account.is_null() {
        return Err("targeted Raydium pool account does not exist".to_owned());
    }

    let notification = json!({
        "method": "programNotification",
        "params": {
            "result": {
                "context": {
                    "slot": slot
                },
                "value": {
                    "pubkey": pool_id,
                    "account": account
                }
            }
        }
    });

    raydium::parse_program_notification(&notification)?
        .ok_or_else(|| "targeted Raydium pool did not decode as a program observation".to_owned())
}

fn raydium_observation_matches_pair(
    observation: &raydium::RaydiumCpmmAccountObservation,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> bool {
    let token_0 = observation.pool_state.token_0_mint.as_str();
    let token_1 = observation.pool_state.token_1_mint.as_str();

    (token_0 == anchor_mint && token_1 == intermediate_mint)
        || (token_1 == anchor_mint && token_0 == intermediate_mint)
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn locator_selects_only_verified_raydium_cpmm_program() {
        let payload = json!({
            "success": true,
            "data": {
                "data": [
                    {
                        "id": "wrong-program-pool",
                        "programId": "SomeOtherProgram11111111111111111111111111"
                    },
                    {
                        "id": "verified-cpmm-pool",
                        "programId": raydium::RAYDIUM_CPMM_PROGRAM_ID
                    }
                ]
            }
        });

        assert_eq!(
            raydium_locator_pool_id_from_payload(&payload),
            Ok(Some("verified-cpmm-pool".to_owned()))
        );
    }

    #[test]
    fn locator_does_not_accept_unrelated_standard_pool() {
        let payload = json!({
            "success": true,
            "data": {
                "data": [
                    {
                        "id": "wrong-program-pool",
                        "programId": "SomeOtherProgram11111111111111111111111111"
                    }
                ]
            }
        });

        assert_eq!(raydium_locator_pool_id_from_payload(&payload), Ok(None));
    }
}
