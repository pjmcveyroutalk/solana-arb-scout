mod costs;
mod discovery;
pub mod economics;
mod pumpswap;
mod quote;
mod raydium;
mod registry;
mod route;
mod sizing;

use discovery::{parse_raydium_pair_lookup_response, raydium_pair_lookup_requests};
use futures_util::{SinkExt, StreamExt};
use quote::{quote_two_leg_exact_input, VenueQuoteContext};
use registry::ActiveMintRegistry;
use reqwest::Client;
use route::{generate_two_leg_routes, RouteLeg, USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use scout_core::{NormalizedPoolState, PoolTradingState, QuoteReserveState};
use serde_json::{json, Value};
use sizing::{
    parse_pyth_usd_price, pyth_usd_price_request, usd_dollars_to_anchor_raw, PythUsdFeed,
    SolUsdPrice, USD_SIZE_GRID,
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

#[derive(Debug)]
struct PythUsdPrices {
    sol: SolUsdPrice,
    usdc: Option<SolUsdPrice>,
    usdt: Option<SolUsdPrice>,
}

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
        println!("\nRung 9 deterministic exact-pair route reacquisition");

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
                &pumpswap_states,
                &pumpswap_quote_contexts,
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

    let usd_prices = fetch_pyth_usd_prices(&rpc_client).await?;

    validate_registry_routes_and_sizes(
        &rpc_client,
        raydium_states,
        pumpswap_states,
        &raydium_quote_contexts,
        &pumpswap_quote_contexts,
        &usd_prices,
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
    live_pumpswap_states: &[NormalizedPoolState],
    live_pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
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

    let mut seen_raydium_pairs = BTreeSet::new();
    let mut raydium_pair_count = 0usize;

    for raydium_normalized in live_raydium_states {
        let Some((anchor_mint, intermediate_mint)) = anchor_pair_from_pool(raydium_normalized)
        else {
            continue;
        };

        if !seen_raydium_pairs.insert((anchor_mint.clone(), intermediate_mint.clone())) {
            continue;
        }

        raydium_pair_count += 1;

        if raydium_pair_count > MAX_TARGETED_ROUTE_LOOKUPS {
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
                        "raydium_pool={} pumpswap_pool={} source=raydium-live-exact-pair"
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
        "deterministic_raydium_to_pumpswap_exhausted: unique_pairs={}",
        seen_raydium_pairs.len()
    );
    println!("deterministic_reverse_exact_pair_probe_start");

    let mut seen_pumpswap_pairs = BTreeSet::new();
    let mut pumpswap_pair_count = 0usize;
    let mut successful_raydium_pair_responses = 0usize;

    for pumpswap_normalized in live_pumpswap_states {
        let Some((anchor_mint, intermediate_mint)) = anchor_pair_from_pool(pumpswap_normalized)
        else {
            continue;
        };

        if !seen_pumpswap_pairs.insert((anchor_mint.clone(), intermediate_mint.clone())) {
            continue;
        }

        pumpswap_pair_count += 1;

        if pumpswap_pair_count > MAX_TARGETED_ROUTE_LOOKUPS {
            break;
        }

        let Some(pumpswap_snapshot) =
            live_pumpswap_quote_contexts.get(&pumpswap_normalized.pool_id)
        else {
            println!(
                "deterministic_reverse_probe_rejected: pumpswap_pool={} reason=missing live quote context",
                pumpswap_normalized.pool_id
            );
            continue;
        };

        println!(
            concat!(
                "deterministic_reverse_probe_pair_start: anchor={} intermediate={} ",
                "pumpswap_pool={} pair={}/{}"
            ),
            anchor_mint,
            intermediate_mint,
            pumpswap_normalized.pool_id,
            pumpswap_pair_count,
            MAX_TARGETED_ROUTE_LOOKUPS
        );

        for raydium_request in raydium_pair_lookup_requests(&anchor_mint, &intermediate_mint) {
            let label = format!(
                "Raydium exact-pair lookup anchor={anchor_mint} intermediate={intermediate_mint}"
            );

            let raydium_payload = match fetch_program_accounts(rpc_client, &raydium_request, &label)
                .await
            {
                Ok(payload) => payload,
                Err(error) => {
                    println!(
                        "deterministic_raydium_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                        anchor_mint, intermediate_mint
                    );
                    continue;
                }
            };

            let raydium_observations = match parse_raydium_pair_lookup_response(&raydium_payload) {
                Ok(observations) => {
                    successful_raydium_pair_responses += 1;
                    observations
                }
                Err(error) => {
                    println!(
                        "deterministic_raydium_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                        anchor_mint, intermediate_mint
                    );
                    continue;
                }
            };

            println!(
                "deterministic_raydium_exact_pair_lookup_parsed: anchor={} intermediate={} observation_count={}",
                anchor_mint,
                intermediate_mint,
                raydium_observations.len()
            );

            for raydium_observation in raydium_observations {
                if !raydium_observation_matches_pair(
                    &raydium_observation,
                    &anchor_mint,
                    &intermediate_mint,
                ) {
                    continue;
                }

                let raydium_hydration_result =
                    hydrate_raydium_observation(rpc_client, &raydium_observation).await;

                let (raydium_normalized, raydium_snapshot) = match raydium_hydration_result {
                    Ok(result) => result,
                    Err(error) => {
                        println!(
                            "deterministic_raydium_exact_pair_candidate_rejected: pool={} reason={error}",
                            raydium_observation.pubkey
                        );
                        continue;
                    }
                };

                if !normalized_pool_is_eligible(&raydium_normalized) {
                    continue;
                }

                println!(
                    concat!(
                        "deterministic_cross_venue_overlap: anchor={} intermediate={} ",
                        "raydium_pool={} pumpswap_pool={} source=pumpswap-live-exact-pair-reverse"
                    ),
                    anchor_mint,
                    intermediate_mint,
                    raydium_normalized.pool_id,
                    pumpswap_normalized.pool_id,
                );
                println!("READ-ONLY RUNG 9 DETERMINISTIC DISCOVERY PASS");

                let mut raydium_contexts = BTreeMap::new();
                raydium_contexts.insert(raydium_normalized.pool_id.clone(), raydium_snapshot);

                let mut pumpswap_contexts = BTreeMap::new();
                pumpswap_contexts.insert(
                    pumpswap_normalized.pool_id.clone(),
                    pumpswap_snapshot.clone(),
                );

                return Ok((
                    vec![raydium_normalized],
                    raydium_contexts,
                    vec![pumpswap_normalized.clone()],
                    pumpswap_contexts,
                ));
            }
        }
    }

    println!(
        "deterministic_reverse_exact_pair_probe_exhausted: unique_pairs={}",
        seen_pumpswap_pairs.len()
    );

    if successful_raydium_pair_responses == 0 {
        return Err(
            "all bounded Raydium exact-pair lookup requests failed before a valid RPC response was parsed"
                .to_owned(),
        );
    }

    Err(
        "Rung 9 bounded bidirectional exact-pair discovery found no live Raydium-PumpSwap same-pair overlap"
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
    let request_id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
    let started_at = Instant::now();

    println!("rpc_request_start: label={label} id={request_id} method=getProgramAccounts");

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(request)
        .send()
        .await
        .map_err(|error| {
            format!(
                "{label} RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "{label} RPC returned HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        ));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        format!(
            "{label} RPC returned invalid JSON after {} ms: {error}",
            started_at.elapsed().as_millis()
        )
    })?;

    let result_count = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .map_or(0, Vec::len);

    println!(
        concat!(
            "rpc_request_finish: label={} id={} status={} elapsed_ms={} ",
            "result_count={} rpc_error={}"
        ),
        label,
        request_id,
        status,
        started_at.elapsed().as_millis(),
        result_count,
        payload.get("error").is_some()
    );

    Ok(payload)
}

async fn fetch_pyth_usd_price(
    rpc_client: &Client,
    feed: PythUsdFeed,
) -> Result<SolUsdPrice, String> {
    let request = pyth_usd_price_request(feed);
    let request_id = request.get("id").and_then(Value::as_u64).unwrap_or(0);
    let method = request
        .get("method")
        .and_then(Value::as_str)
        .unwrap_or("unknown");
    let started_at = Instant::now();

    println!(
        "rpc_request_start: label=Pyth {} id={} method={}",
        feed.label(),
        request_id,
        method
    );

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Pyth {} RPC request failed after {} ms: {error}",
                feed.label(),
                started_at.elapsed().as_millis()
            )
        })?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "Pyth {} RPC returned HTTP status {} after {} ms",
            feed.label(),
            status,
            started_at.elapsed().as_millis()
        ));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        format!(
            "Pyth {} RPC returned invalid JSON after {} ms: {error}",
            feed.label(),
            started_at.elapsed().as_millis()
        )
    })?;

    println!(
        "rpc_request_finish: label=Pyth {} id={} status={} elapsed_ms={} rpc_error={}",
        feed.label(),
        request_id,
        status,
        started_at.elapsed().as_millis(),
        payload.get("error").is_some()
    );

    let now = unix_time_seconds_now()?;
    parse_pyth_usd_price(&payload, now, feed)
}

async fn fetch_pyth_usd_prices(rpc_client: &Client) -> Result<PythUsdPrices, String> {
    println!("\nRung 10 sizing oracle: Pyth SOL/USD");

    let sol = fetch_pyth_usd_price(rpc_client, PythUsdFeed::Sol).await?;
    println!("sol_usd_price: {}", sol.summary());
    println!("READ-ONLY PYTH SOL/USD VALIDATION PASS");

    println!("\nRung 11D stablecoin external-cost conversion oracle");

    let usdc = match fetch_pyth_usd_price(rpc_client, PythUsdFeed::Usdc).await {
        Ok(price) => {
            println!("usdc_usd_price: {}", price.summary());
            Some(price)
        }
        Err(error) => {
            println!("rung11d_usdc_usd_observation_unavailable: {error}");
            None
        }
    };

    let usdt = match fetch_pyth_usd_price(rpc_client, PythUsdFeed::Usdt).await {
        Ok(price) => {
            println!("usdt_usd_price: {}", price.summary());
            Some(price)
        }
        Err(error) => {
            println!("rung11d_usdt_usd_observation_unavailable: {error}");
            None
        }
    };

    if usdc.is_some() && usdt.is_some() {
        println!("READ-ONLY RUNG 11D STABLECOIN USD OBSERVATION PASS");
    } else {
        println!(
            "rung11d_stablecoin_usd_observation_incomplete: usdc_available={} usdt_available={}",
            usdc.is_some(),
            usdt.is_some()
        );
    }

    Ok(PythUsdPrices { sol, usdc, usdt })
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

fn anchor_pair_from_pool(pool: &NormalizedPoolState) -> Option<(String, String)> {
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

fn raydium_observation_matches_pair(
    observation: &raydium::RaydiumCpmmAccountObservation,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> bool {
    let token_0_mint = observation.pool_state.token_0_mint.as_str();
    let token_1_mint = observation.pool_state.token_1_mint.as_str();

    (token_0_mint == anchor_mint && token_1_mint == intermediate_mint)
        || (token_1_mint == anchor_mint && token_0_mint == intermediate_mint)
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

async fn validate_registry_routes_and_sizes(
    rpc_client: &Client,
    raydium_states: Vec<NormalizedPoolState>,
    pumpswap_states: Vec<NormalizedPoolState>,
    raydium_quote_contexts: &BTreeMap<String, raydium::RaydiumHydrationSnapshot>,
    pumpswap_quote_contexts: &BTreeMap<String, pumpswap::PumpSwapHydrationSnapshot>,
    usd_prices: &PythUsdPrices,
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
    let mut rung11c_quote_records = Vec::new();

    for (route_index, route_candidate) in route_candidates.iter().enumerate() {
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
                Some(&usd_prices.sol),
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
                    rung11c_quote_records.push((
                        route_index,
                        dollars,
                        anchor_decimals,
                        route_quote,
                    ));
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

    println!("\nRung 11C read-only cost observation");

    let jito_observation = observe_jito_tip_floor(rpc_client).await;
    let mut priority_cache = BTreeMap::<Vec<String>, costs::PriorityObservationState>::new();
    let mut route_priority_observations = BTreeMap::<usize, costs::PriorityObservationState>::new();
    let mut rung11c_route_scope_attempts = 0usize;

    let successful_route_indices = rung11c_quote_records
        .iter()
        .map(|(route_index, _, _, _)| *route_index)
        .collect::<BTreeSet<_>>();

    for route_index in successful_route_indices {
        let Some(route_candidate) = route_candidates.get(route_index) else {
            continue;
        };

        let leg_1_context = match quote_context_for_leg(
            route_candidate.leg_1(),
            raydium_quote_contexts,
            pumpswap_quote_contexts,
        ) {
            Ok(context) => context,
            Err(error) => {
                println!(
                    "rung11c_priority_scope_unknown: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                route_priority_observations.insert(
                    route_index,
                    costs::PriorityObservationState::Unavailable(error),
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
                    "rung11c_priority_scope_unknown: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                route_priority_observations.insert(
                    route_index,
                    costs::PriorityObservationState::Unavailable(error),
                );
                continue;
            }
        };

        rung11c_route_scope_attempts += 1;

        let priority_observation = route_priority_observation(
            rpc_client,
            route_candidate.leg_1(),
            &leg_1_context,
            route_candidate.leg_2(),
            &leg_2_context,
            &mut priority_cache,
        )
        .await;

        route_priority_observations.insert(route_index, priority_observation);
    }

    let external_cost_usd_prices = costs::ExternalCostUsdPrices::new(
        &usd_prices.sol,
        usd_prices.usdc.as_ref(),
        usd_prices.usdt.as_ref(),
    );

    for (route_index, dollars, anchor_decimals, route_quote) in &rung11c_quote_records {
        let Some(route_candidate) = route_candidates.get(*route_index) else {
            continue;
        };
        let Some(priority_observation) = route_priority_observations.get(route_index) else {
            println!(
                "rung11c_cost_model_rejected: route=[{}] reason=missing priority observation state",
                route_candidate.summary()
            );
            continue;
        };

        let cost_model = match costs::economics_cost_model_with_usd_prices(
            route_candidate.anchor_mint(),
            *anchor_decimals,
            priority_observation,
            &jito_observation,
            Some(&external_cost_usd_prices),
        ) {
            Ok(model) => model,
            Err(error) => {
                println!(
                    "rung11c_cost_model_rejected: route=[{}] reason={error}",
                    route_candidate.summary()
                );
                continue;
            }
        };

        for funding_mode in [
            economics::FundingMode::Treasury,
            economics::FundingMode::Flash,
        ] {
            match economics::evaluate_expected_net_for_mode(route_quote, &cost_model, funding_mode)
            {
                Ok(result) => {
                    println!("rung11c_expected_net: usd=${dollars} {}", result.summary());
                }
                Err(error) => {
                    println!(
                        concat!(
                            "rung11c_expected_net_fail_closed: usd=${} funding={} ",
                            "route=[{}] reason={}"
                        ),
                        dollars,
                        funding_mode.label(),
                        route_candidate.summary(),
                        error
                    );
                }
            }
        }
    }

    if rung11c_route_scope_attempts > 0 {
        println!("READ-ONLY RUNG 11C COST OBSERVATION PASS");
    }

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

async fn observe_jito_tip_floor(rpc_client: &Client) -> costs::JitoObservationState {
    let started_at = Instant::now();
    let response = match rpc_client.get(costs::JITO_TIP_FLOOR_URL).send().await {
        Ok(response) => response,
        Err(error) => {
            let reason = format!(
                "Jito tip-floor HTTP request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            );
            println!("rung11c_jito_observation_unavailable: {reason}");
            return costs::JitoObservationState::Unavailable(reason);
        }
    };

    let status = response.status();
    if !status.is_success() {
        let reason = format!(
            "Jito tip-floor HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        );
        println!("rung11c_jito_observation_unavailable: {reason}");
        return costs::JitoObservationState::Unavailable(reason);
    }

    let payload = match response.json::<Value>().await {
        Ok(payload) => payload,
        Err(error) => {
            let reason = format!(
                "Jito tip-floor response was invalid JSON after {} ms: {error}",
                started_at.elapsed().as_millis()
            );
            println!("rung11c_jito_observation_unavailable: {reason}");
            return costs::JitoObservationState::Unavailable(reason);
        }
    };

    match costs::parse_jito_tip_floor_response(&payload) {
        Ok(observation) => {
            println!("rung11c_jito_observation: {}", observation.summary());
            costs::JitoObservationState::Available(observation)
        }
        Err(error) => {
            println!("rung11c_jito_observation_unavailable: {error}");
            costs::JitoObservationState::Unavailable(error)
        }
    }
}

async fn route_priority_observation(
    rpc_client: &Client,
    leg_1: &RouteLeg,
    leg_1_context: &VenueQuoteContext<'_>,
    leg_2: &RouteLeg,
    leg_2_context: &VenueQuoteContext<'_>,
    cache: &mut BTreeMap<Vec<String>, costs::PriorityObservationState>,
) -> costs::PriorityObservationState {
    let leg_1_raydium = match leg_1_context {
        VenueQuoteContext::Raydium { snapshot, .. } => Some(*snapshot),
        VenueQuoteContext::PumpSwap { .. } => None,
    };
    let leg_2_raydium = match leg_2_context {
        VenueQuoteContext::Raydium { snapshot, .. } => Some(*snapshot),
        VenueQuoteContext::PumpSwap { .. } => None,
    };

    let footprint = match costs::route_contention_footprint(
        costs::VenueContentionInput {
            venue: leg_1.venue(),
            pool_id: leg_1.pool_id(),
            raydium_snapshot: leg_1_raydium,
        },
        costs::VenueContentionInput {
            venue: leg_2.venue(),
            pool_id: leg_2.pool_id(),
            raydium_snapshot: leg_2_raydium,
        },
    ) {
        Ok(footprint) => footprint,
        Err(error) => {
            println!(
                concat!(
                    "rung11c_priority_scope_unknown: leg1_pool={} leg2_pool={} ",
                    "reason={}"
                ),
                leg_1.pool_id(),
                leg_2.pool_id(),
                error
            );
            return costs::PriorityObservationState::Unavailable(error);
        }
    };

    println!("rung11c_priority_scope: {}", footprint.summary());

    let cache_key = footprint.accounts().to_vec();
    if let Some(cached) = cache.get(&cache_key) {
        println!(
            "rung11c_priority_observation_cache_hit: account_count={}",
            cache_key.len()
        );
        return cached.clone();
    }

    let observation = fetch_localized_priority_observation(rpc_client, &footprint).await;
    cache.insert(cache_key, observation.clone());
    observation
}

async fn fetch_localized_priority_observation(
    rpc_client: &Client,
    footprint: &costs::DeterministicVenueContentionFootprint,
) -> costs::PriorityObservationState {
    let request = costs::localized_priority_fee_request(footprint);
    let started_at = Instant::now();

    println!(
        concat!(
            "rpc_request_start: label=Rung11C localized priority ",
            "method=getRecentPrioritizationFees account_count={}"
        ),
        footprint.accounts().len()
    );

    let response = match rpc_client.post(SOLANA_RPC_URL).json(&request).send().await {
        Ok(response) => response,
        Err(error) => {
            let reason = format!(
                "localized priority RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            );
            println!("rung11c_priority_observation_unavailable: {reason}");
            return costs::PriorityObservationState::Unavailable(reason);
        }
    };

    let status = response.status();
    if !status.is_success() {
        let reason = format!(
            "localized priority RPC returned HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        );
        println!("rung11c_priority_observation_unavailable: {reason}");
        return costs::PriorityObservationState::Unavailable(reason);
    }

    let payload = match response.json::<Value>().await {
        Ok(payload) => payload,
        Err(error) => {
            let reason = format!(
                "localized priority RPC returned invalid JSON after {} ms: {error}",
                started_at.elapsed().as_millis()
            );
            println!("rung11c_priority_observation_unavailable: {reason}");
            return costs::PriorityObservationState::Unavailable(reason);
        }
    };

    match costs::parse_localized_priority_fee_response(&payload, footprint) {
        Ok(observation) => {
            println!("rung11c_priority_observation: {}", observation.summary());

            match costs::select_priority_fee(&observation) {
                Ok(Some(selection)) => {
                    println!("rung11c_priority_selection: {}", selection.summary());
                }
                Ok(None) => {
                    println!("rung11c_priority_selection_unknown: no positive localized samples");
                }
                Err(error) => {
                    println!("rung11c_priority_selection_rejected: {error}");
                    return costs::PriorityObservationState::Unavailable(error);
                }
            }

            costs::PriorityObservationState::Available(observation)
        }
        Err(error) => {
            println!("rung11c_priority_observation_unavailable: {error}");
            costs::PriorityObservationState::Unavailable(error)
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

    let started_at = Instant::now();

    println!(
        "rpc_request_start: label={} hydration id={} method=getMultipleAccounts account_count={}",
        venue, request_id, N
    );

    let response = rpc_client
        .post(SOLANA_RPC_URL)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            format!(
                "{venue} hydration RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    let status = response.status();

    if !status.is_success() {
        return Err(format!(
            "{venue} hydration RPC returned HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        ));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        format!(
            "{venue} hydration RPC returned invalid JSON after {} ms: {error}",
            started_at.elapsed().as_millis()
        )
    })?;

    println!(
        concat!(
            "rpc_request_finish: label={} hydration id={} status={} ",
            "elapsed_ms={} rpc_error={}"
        ),
        venue,
        request_id,
        status,
        started_at.elapsed().as_millis(),
        payload.get("error").is_some()
    );

    Ok(payload)
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
