mod pumpswap;
mod raydium;
mod registry;
mod route;

use futures_util::{SinkExt, StreamExt};
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

const PUMPSWAP_POOL_DISCRIMINATOR_BASE58: &str = "hQrXeCntzbV";
const PUMPSWAP_BASE_MINT_OFFSET: usize = 43;
const PUMPSWAP_QUOTE_MINT_OFFSET: usize = 75;

const MAX_SLOT_OBSERVATIONS: usize = 5;
const MAX_RAYDIUM_OBSERVATIONS: usize = 5;
const MAX_PUMPSWAP_OBSERVATIONS: usize = 15;

const RAYDIUM_INVENTORY_PAGE_SIZE: usize = 100;
const MAX_RAYDIUM_CANDIDATES_PER_ANCHOR: usize = 10;

const OBSERVATION_TIMEOUT: Duration = Duration::from_secs(30);
const RPC_TIMEOUT: Duration = Duration::from_secs(10);
const LOCATOR_TIMEOUT: Duration = Duration::from_secs(3);

#[derive(Debug, Clone, PartialEq, Eq)]
struct RaydiumInventoryCandidate {
    anchor_mint: String,
    intermediate_mint: String,
    raydium_pool_id: String,
}

struct TargetedRouteDiscovery {
    raydium_state: NormalizedPoolState,
    pumpswap_state: NormalizedPoolState,
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

    let mut pumpswap_states = match observe_pumpswap(&rpc_client).await {
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

        let targeted_discovery = match discover_targeted_cross_venue_overlap(&rpc_client).await {
            Ok(discovery) => discovery,
            Err(error) => {
                eprintln!("Rung 9 targeted discovery failed: {error}");
                std::process::exit(1);
            }
        };

        match targeted_discovery {
            Some(discovery) => {
                println!("targeted_raydium_state_count=1");
                println!("targeted_pumpswap_state_count=1");

                raydium_states.push(discovery.raydium_state);
                pumpswap_states.push(discovery.pumpswap_state);
            }
            None => {
                println!("targeted_raydium_state_count=0");
                println!("targeted_pumpswap_state_count=0");
            }
        }
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

async fn discover_targeted_cross_venue_overlap(
    rpc_client: &Client,
) -> Result<Option<TargetedRouteDiscovery>, String> {
    println!("\nRung 9 targeted cross-venue discovery");
    println!("Raydium API is locator-only; Solana on-chain state remains authoritative.");

    let candidates = collect_raydium_inventory_candidates(rpc_client).await?;

    println!(
        "raydium_anchor_inventory_candidate_count={}",
        candidates.len()
    );

    if candidates.is_empty() {
        return Ok(None);
    }

    let mut successful_pumpswap_lookup_calls = 0usize;

    for candidate in candidates {
        let orientations = [
            (
                candidate.anchor_mint.as_str(),
                candidate.intermediate_mint.as_str(),
            ),
            (
                candidate.intermediate_mint.as_str(),
                candidate.anchor_mint.as_str(),
            ),
        ];

        let mut pumpswap_pool_ids = Vec::new();

        for (base_mint, quote_mint) in orientations {
            match fetch_pumpswap_pair_pool_ids(rpc_client, base_mint, quote_mint).await {
                Ok(pool_ids) => {
                    successful_pumpswap_lookup_calls += 1;

                    for pool_id in pool_ids {
                        if !pumpswap_pool_ids.contains(&pool_id) {
                            pumpswap_pool_ids.push(pool_id);
                        }
                    }
                }
                Err(error) => {
                    println!(
                        "targeted_pumpswap_locator_rejected: anchor={} intermediate={} base={} quote={} reason={}",
                        candidate.anchor_mint,
                        candidate.intermediate_mint,
                        base_mint,
                        quote_mint,
                        error,
                    );
                }
            }
        }

        if pumpswap_pool_ids.is_empty() {
            println!(
                "targeted_pumpswap_no_pair: anchor={} intermediate={} raydium_pool={}",
                candidate.anchor_mint, candidate.intermediate_mint, candidate.raydium_pool_id,
            );
            continue;
        }

        for pumpswap_pool_id in pumpswap_pool_ids {
            println!(
                "route_locator_candidate: anchor={} intermediate={} raydium_pool={} pumpswap_pool={}",
                candidate.anchor_mint,
                candidate.intermediate_mint,
                candidate.raydium_pool_id,
                pumpswap_pool_id,
            );

            let raydium_observation = match fetch_raydium_pool_observation(
                rpc_client,
                &candidate.raydium_pool_id,
            )
            .await
            {
                Ok(observation) => observation,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason={}",
                        candidate.raydium_pool_id, error
                    );
                    continue;
                }
            };

            if !raydium_observation_matches_pair(
                &raydium_observation,
                &candidate.anchor_mint,
                &candidate.intermediate_mint,
            ) {
                println!(
                    "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason=on-chain mint pair mismatch",
                    candidate.raydium_pool_id
                );
                continue;
            }

            let pumpswap_observation =
                match fetch_pumpswap_pool_observation(rpc_client, &pumpswap_pool_id).await {
                    Ok(observation) => observation,
                    Err(error) => {
                        println!(
                            "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason={}",
                            pumpswap_pool_id, error
                        );
                        continue;
                    }
                };

            if !pumpswap_observation_matches_pair(
                &pumpswap_observation,
                &candidate.anchor_mint,
                &candidate.intermediate_mint,
            ) {
                println!(
                    "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason=on-chain mint pair mismatch",
                    pumpswap_pool_id
                );
                continue;
            }

            let raydium_received_at_unix_ms = unix_time_ms_now()?;
            let raydium_hydration_started_at_unix_ms = unix_time_ms_now()?;

            let raydium_hydration_payload =
                match fetch_raydium_hydration(rpc_client, &raydium_observation).await {
                    Ok(payload) => payload,
                    Err(error) => {
                        println!(
                            "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason={}",
                            candidate.raydium_pool_id, error
                        );
                        continue;
                    }
                };

            let raydium_snapshot = match raydium::parse_hydration_response(
                &raydium_observation,
                &raydium_hydration_payload,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason={}",
                        candidate.raydium_pool_id, error
                    );
                    continue;
                }
            };

            let raydium_hydrated_at_unix_ms = unix_time_ms_now()?;

            let raydium_normalized = match raydium::hydrate_normalized_observation(
                &raydium_observation,
                &raydium_snapshot,
                raydium_received_at_unix_ms,
                raydium_hydrated_at_unix_ms,
            ) {
                Ok(normalized) => normalized,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason={}",
                        candidate.raydium_pool_id, error
                    );
                    continue;
                }
            };

            if !normalized_pool_is_eligible(&raydium_normalized) {
                println!(
                    "targeted_onchain_candidate_rejected: venue=raydium_cpmm pool={} reason=current normalized state is not registry-eligible",
                    candidate.raydium_pool_id
                );
                continue;
            }

            let pumpswap_received_at_unix_ms = unix_time_ms_now()?;
            let pumpswap_hydration_started_at_unix_ms = unix_time_ms_now()?;

            let pumpswap_hydration_payload =
                match fetch_pumpswap_hydration(rpc_client, &pumpswap_observation).await {
                    Ok(payload) => payload,
                    Err(error) => {
                        println!(
                            "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason={}",
                            pumpswap_pool_id, error
                        );
                        continue;
                    }
                };

            let pumpswap_snapshot = match pumpswap::parse_hydration_response(
                &pumpswap_observation,
                &pumpswap_hydration_payload,
            ) {
                Ok(snapshot) => snapshot,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason={}",
                        pumpswap_pool_id, error
                    );
                    continue;
                }
            };

            let pumpswap_hydrated_at_unix_ms = unix_time_ms_now()?;

            let pumpswap_normalized = match pumpswap::hydrate_normalized_observation(
                &pumpswap_observation,
                &pumpswap_snapshot,
                pumpswap_received_at_unix_ms,
                pumpswap_hydrated_at_unix_ms,
            ) {
                Ok(normalized) => normalized,
                Err(error) => {
                    println!(
                        "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason={}",
                        pumpswap_pool_id, error
                    );
                    continue;
                }
            };

            if !normalized_pool_is_eligible(&pumpswap_normalized) {
                println!(
                    "targeted_onchain_candidate_rejected: venue=pumpswap pool={} reason=current normalized state is not registry-eligible",
                    pumpswap_pool_id
                );
                continue;
            }

            let raydium_hydration_duration_ms =
                raydium_hydrated_at_unix_ms.saturating_sub(raydium_hydration_started_at_unix_ms);
            let pumpswap_hydration_duration_ms =
                pumpswap_hydrated_at_unix_ms.saturating_sub(pumpswap_hydration_started_at_unix_ms);

            println!(
                "targeted_raydium_pool: anchor={} intermediate={} pool={} source_slot={} reserve_slot={} hydration_duration_ms={}",
                candidate.anchor_mint,
                candidate.intermediate_mint,
                raydium_normalized.pool_id,
                raydium_normalized.source_slot,
                raydium_snapshot.slot,
                raydium_hydration_duration_ms,
            );

            println!(
                "targeted_pumpswap_pool: anchor={} intermediate={} pool={} source_slot={} reserve_slot={} hydration_duration_ms={}",
                candidate.anchor_mint,
                candidate.intermediate_mint,
                pumpswap_normalized.pool_id,
                pumpswap_normalized.source_slot,
                pumpswap_snapshot.slot,
                pumpswap_hydration_duration_ms,
            );

            println!(
                "targeted_raydium_normalized_pool: {}",
                raydium_normalized.summary()
            );
            println!(
                "targeted_pumpswap_normalized_pool: {}",
                pumpswap_normalized.summary()
            );
            println!("READ-ONLY RUNG 9 TARGETED DISCOVERY PASS");

            return Ok(Some(TargetedRouteDiscovery {
                raydium_state: raydium_normalized,
                pumpswap_state: pumpswap_normalized,
            }));
        }
    }

    if successful_pumpswap_lookup_calls == 0 {
        return Err(
            "all bounded PumpSwap exact-pair lookup requests failed before a response was received"
                .to_owned(),
        );
    }

    Ok(None)
}

async fn collect_raydium_inventory_candidates(
    rpc_client: &Client,
) -> Result<Vec<RaydiumInventoryCandidate>, String> {
    let mut candidates = Vec::new();

    for anchor_mint in [WRAPPED_SOL_MINT, USDC_MINT, USDT_MINT] {
        let anchor_candidates = fetch_raydium_anchor_inventory(rpc_client, anchor_mint).await?;

        println!(
            "raydium_anchor_inventory: anchor={} cpmm_candidate_count={}",
            anchor_mint,
            anchor_candidates.len()
        );

        for candidate in anchor_candidates {
            if candidates
                .iter()
                .any(|existing: &RaydiumInventoryCandidate| {
                    existing.raydium_pool_id == candidate.raydium_pool_id
                })
            {
                continue;
            }

            candidates.push(candidate);
        }
    }

    Ok(candidates)
}

async fn fetch_raydium_anchor_inventory(
    rpc_client: &Client,
    anchor_mint: &str,
) -> Result<Vec<RaydiumInventoryCandidate>, String> {
    let url = format!(
        "{RAYDIUM_API_POOL_MINT_URL}?mint1={anchor_mint}&poolType=standard&poolSortField=liquidity&sortType=desc&pageSize={RAYDIUM_INVENTORY_PAGE_SIZE}&page=1"
    );

    let response = rpc_client
        .get(url)
        .timeout(LOCATOR_TIMEOUT)
        .send()
        .await
        .map_err(|error| format!("Raydium anchor inventory request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "Raydium anchor inventory returned HTTP status {status}"
        ));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Raydium anchor inventory returned invalid JSON: {error}"))?;

    raydium_anchor_candidates_from_payload(&payload, anchor_mint)
}

fn raydium_anchor_candidates_from_payload(
    payload: &Value,
    anchor_mint: &str,
) -> Result<Vec<RaydiumInventoryCandidate>, String> {
    if payload.get("success").and_then(Value::as_bool) != Some(true) {
        let message = payload
            .get("msg")
            .and_then(Value::as_str)
            .unwrap_or("unknown Raydium API error");

        return Err(format!(
            "Raydium anchor inventory rejected request: {message}"
        ));
    }

    let pools = payload
        .pointer("/data/data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Raydium anchor inventory response missing pool array".to_owned())?;

    let mut candidates = Vec::new();

    for pool in pools {
        if pool.get("programId").and_then(Value::as_str) != Some(raydium::RAYDIUM_CPMM_PROGRAM_ID) {
            continue;
        }

        let pool_id = pool
            .get("id")
            .and_then(Value::as_str)
            .ok_or_else(|| "Raydium CPMM inventory result missing pool id".to_owned())?;

        let mint_a = pool
            .pointer("/mintA/address")
            .and_then(Value::as_str)
            .or_else(|| pool.pointer("/mint1/address").and_then(Value::as_str))
            .ok_or_else(|| "Raydium CPMM inventory result missing first mint address".to_owned())?;

        let mint_b = pool
            .pointer("/mintB/address")
            .and_then(Value::as_str)
            .or_else(|| pool.pointer("/mint2/address").and_then(Value::as_str))
            .ok_or_else(|| {
                "Raydium CPMM inventory result missing second mint address".to_owned()
            })?;

        let intermediate_mint = if mint_a == anchor_mint && mint_b != anchor_mint {
            mint_b
        } else if mint_b == anchor_mint && mint_a != anchor_mint {
            mint_a
        } else {
            continue;
        };

        if candidates
            .iter()
            .any(|existing: &RaydiumInventoryCandidate| {
                existing.intermediate_mint == intermediate_mint
            })
        {
            continue;
        }

        candidates.push(RaydiumInventoryCandidate {
            anchor_mint: anchor_mint.to_owned(),
            intermediate_mint: intermediate_mint.to_owned(),
            raydium_pool_id: pool_id.to_owned(),
        });

        if candidates.len() >= MAX_RAYDIUM_CANDIDATES_PER_ANCHOR {
            break;
        }
    }

    Ok(candidates)
}

fn pumpswap_pair_lookup_request(request_id: u64, base_mint: &str, quote_mint: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getProgramAccounts",
        "params": [
            pumpswap::PUMPSWAP_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "withContext": true,
                "dataSlice": {
                    "offset": 0,
                    "length": 0
                },
                "filters": [
                    {
                        "memcmp": {
                            "offset": 0,
                            "bytes": PUMPSWAP_POOL_DISCRIMINATOR_BASE58
                        }
                    },
                    {
                        "memcmp": {
                            "offset": PUMPSWAP_BASE_MINT_OFFSET,
                            "bytes": base_mint
                        }
                    },
                    {
                        "memcmp": {
                            "offset": PUMPSWAP_QUOTE_MINT_OFFSET,
                            "bytes": quote_mint
                        }
                    }
                ]
            }
        ]
    })
}

async fn fetch_pumpswap_pair_pool_ids(
    rpc_client: &Client,
    base_mint: &str,
    quote_mint: &str,
) -> Result<Vec<String>, String> {
    let request = pumpswap_pair_lookup_request(7, base_mint, quote_mint);

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("PumpSwap exact-pair RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "PumpSwap exact-pair RPC returned HTTP status {status}"
        ));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("PumpSwap exact-pair RPC returned invalid JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        return Err(format!(
            "PumpSwap exact-pair getProgramAccounts returned an RPC error: {error}"
        ));
    }

    pumpswap_pair_pool_ids_from_payload(&payload)
}

fn pumpswap_pair_pool_ids_from_payload(payload: &Value) -> Result<Vec<String>, String> {
    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .or_else(|| payload.get("result").and_then(Value::as_array))
        .ok_or_else(|| {
            "PumpSwap exact-pair getProgramAccounts response missing account array".to_owned()
        })?;

    let mut pool_ids = Vec::new();

    for account in accounts {
        let pool_id = account
            .get("pubkey")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                "PumpSwap exact-pair getProgramAccounts result missing pubkey".to_owned()
            })?;

        if !pool_ids.iter().any(|existing| existing == pool_id) {
            pool_ids.push(pool_id.to_owned());
        }
    }

    Ok(pool_ids)
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

async fn fetch_pumpswap_pool_observation(
    rpc_client: &Client,
    pool_id: &str,
) -> Result<pumpswap::PumpSwapAccountObservation, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 8,
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
        .map_err(|error| format!("targeted PumpSwap pool RPC request failed: {error}"))?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "targeted PumpSwap pool RPC returned HTTP status {status}"
        ));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("targeted PumpSwap pool RPC returned invalid JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        return Err(format!(
            "targeted PumpSwap getAccountInfo returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "targeted PumpSwap getAccountInfo missing context slot".to_owned())?;

    let account = payload
        .pointer("/result/value")
        .ok_or_else(|| "targeted PumpSwap getAccountInfo missing account value".to_owned())?;

    if account.is_null() {
        return Err("targeted PumpSwap pool account does not exist".to_owned());
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

    pumpswap::parse_program_notification(&notification)?
        .ok_or_else(|| "targeted PumpSwap pool did not decode as a program observation".to_owned())
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
    fn inventory_selects_verified_cpmm_and_derives_counter_mint() {
        let payload = json!({
            "success": true,
            "data": {
                "data": [
                    {
                        "id": "wrong-program-pool",
                        "programId": "SomeOtherProgram11111111111111111111111111",
                        "mintA": {
                            "address": WRAPPED_SOL_MINT
                        },
                        "mintB": {
                            "address": "WrongCounterMint111111111111111111111111"
                        }
                    },
                    {
                        "id": "verified-cpmm-pool",
                        "programId": raydium::RAYDIUM_CPMM_PROGRAM_ID,
                        "mintA": {
                            "address": WRAPPED_SOL_MINT
                        },
                        "mintB": {
                            "address": "CounterMint1111111111111111111111111111"
                        }
                    }
                ]
            }
        });

        assert_eq!(
            raydium_anchor_candidates_from_payload(&payload, WRAPPED_SOL_MINT),
            Ok(vec![RaydiumInventoryCandidate {
                anchor_mint: WRAPPED_SOL_MINT.to_owned(),
                intermediate_mint: "CounterMint1111111111111111111111111111".to_owned(),
                raydium_pool_id: "verified-cpmm-pool".to_owned(),
            }])
        );
    }

    #[test]
    fn inventory_accepts_documented_mint1_mint2_shape() {
        let payload = json!({
            "success": true,
            "data": {
                "data": [
                    {
                        "id": "verified-cpmm-pool",
                        "programId": raydium::RAYDIUM_CPMM_PROGRAM_ID,
                        "mint1": {
                            "address": "CounterMint1111111111111111111111111111"
                        },
                        "mint2": {
                            "address": WRAPPED_SOL_MINT
                        }
                    }
                ]
            }
        });

        assert_eq!(
            raydium_anchor_candidates_from_payload(&payload, WRAPPED_SOL_MINT),
            Ok(vec![RaydiumInventoryCandidate {
                anchor_mint: WRAPPED_SOL_MINT.to_owned(),
                intermediate_mint: "CounterMint1111111111111111111111111111".to_owned(),
                raydium_pool_id: "verified-cpmm-pool".to_owned(),
            }])
        );
    }

    #[test]
    fn pumpswap_pair_lookup_uses_exact_layout_offsets() {
        let request = pumpswap_pair_lookup_request(7, "base-mint", "quote-mint");

        assert_eq!(
            request
                .pointer("/params/1/filters/0/memcmp/offset")
                .and_then(Value::as_u64),
            Some(0)
        );

        assert_eq!(
            request
                .pointer("/params/1/filters/0/memcmp/bytes")
                .and_then(Value::as_str),
            Some(PUMPSWAP_POOL_DISCRIMINATOR_BASE58)
        );

        assert_eq!(
            request
                .pointer("/params/1/filters/1/memcmp/offset")
                .and_then(Value::as_u64),
            Some(PUMPSWAP_BASE_MINT_OFFSET as u64)
        );

        assert_eq!(
            request
                .pointer("/params/1/filters/1/memcmp/bytes")
                .and_then(Value::as_str),
            Some("base-mint")
        );

        assert_eq!(
            request
                .pointer("/params/1/filters/2/memcmp/offset")
                .and_then(Value::as_u64),
            Some(PUMPSWAP_QUOTE_MINT_OFFSET as u64)
        );

        assert_eq!(
            request
                .pointer("/params/1/filters/2/memcmp/bytes")
                .and_then(Value::as_str),
            Some("quote-mint")
        );
    }

    #[test]
    fn pumpswap_pair_lookup_parser_extracts_pool_ids() {
        let payload = json!({
            "result": {
                "context": {
                    "slot": 123
                },
                "value": [
                    {
                        "pubkey": "pool-one",
                        "account": {}
                    },
                    {
                        "pubkey": "pool-two",
                        "account": {}
                    }
                ]
            }
        });

        assert_eq!(
            pumpswap_pair_pool_ids_from_payload(&payload),
            Ok(vec!["pool-one".to_owned(), "pool-two".to_owned()])
        );
    }
}
