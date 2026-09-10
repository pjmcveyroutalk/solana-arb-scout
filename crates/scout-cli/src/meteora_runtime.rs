use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use crate::rpc_transport;
use crate::runtime_quote::MeteoraRuntimeQuoteState;
use crate::ws_transport;
use futures_util::StreamExt;
use reqwest::Client;
use scout_cli::meteora::{
    bin_array_bitmap_bit, bin_id_to_bin_array_index, derive_bin_array_pda,
    MeteoraBitmapExtensionState, MeteoraDlmmSnapshot, MeteoraInternalBitmap, BIN_ARRAY_MAX_INDEX,
    BIN_ARRAY_MIN_INDEX, INTERNAL_BITMAP_MAX_INDEX, INTERNAL_BITMAP_MIN_INDEX,
    LB_PAIR_ACCOUNT_LEN, LB_PAIR_DISCRIMINATOR, METEORA_DLMM_PROGRAM_ID,
};
use scout_cli::meteora_live::{
    meteora_base_hydration_account_pubkeys,
    meteora_program_subscribe_request as live_program_subscribe_request,
    meteora_quote_hydration_account_pubkeys, parse_meteora_base_hydration_response,
    parse_meteora_program_notification, parse_meteora_quote_hydration_response,
    MeteoraLiveObservation,
};
use scout_cli::meteora_m13::MeteoraM13PreparedQuote;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

const MAX_BIN_ARRAYS_PER_DIRECTION: usize = 3;
const BASE_HYDRATION_REQUEST_ID: u64 = 61;
const QUOTE_HYDRATION_REQUEST_ID: u64 = 62;
const EXACT_PAIR_FORWARD_REQUEST_ID: u64 = 63;
const EXACT_PAIR_REVERSE_REQUEST_ID: u64 = 64;
const METEORA_MINT_X_OFFSET: usize = 88;
const METEORA_MINT_Y_OFFSET: usize = 120;
const MAX_EXACT_PAIR_CANDIDATES_PER_ORIENTATION: usize = 5;

pub fn program_subscribe_request() -> Value {
    live_program_subscribe_request()
}

pub async fn observe_and_prepare<S>(
    rpc_client: &Client,
    rpc_url: &str,
    reader: &mut S,
    max_observations: usize,
    observation_timeout: Duration,
) -> Result<BTreeMap<String, MeteoraRuntimeQuoteState>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    if max_observations == 0 {
        return Err("Meteora runtime observation bound must be greater than zero".to_owned());
    }
    if observation_timeout.is_zero() {
        return Err("Meteora runtime observation timeout must be greater than zero".to_owned());
    }

    println!("\nVenue adapter: Meteora DLMM");

    let mut prepared = BTreeMap::new();
    let mut observed = 0usize;
    let mut next_generation_id = 1u64;
    let observation_window_started = Instant::now();

    while observed < max_observations {
        let remaining = observation_timeout.saturating_sub(observation_window_started.elapsed());
        if remaining.is_zero() {
            break;
        }

        let Some(payload) = ws_transport::next_json_message_optional(reader, remaining).await?
        else {
            break;
        };
        let trigger_received_at_unix_ms = crate::unix_time_ms_now()?;

        let observation = match parse_meteora_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("meteora_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "meteora_observation: pool={} slot={} version={}",
            observation.pubkey, observation.slot, observation.admission.version
        );

        match hydrate_with_next_generation(
            rpc_client,
            rpc_url,
            &observation,
            trigger_received_at_unix_ms,
            &mut next_generation_id,
        )
        .await
        {
            Ok(runtime) => {
                insert_runtime_state(&mut prepared, runtime, "meteora_runtime_prepared");
            }
            Err(error) => {
                println!(
                    "meteora_hydration_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("meteora_live_observation_count={observed}");
    println!("meteora_live_eligible_count={}", prepared.len());
    println!(
        "meteora_live_observation_elapsed_ms={}",
        observation_window_started.elapsed().as_millis()
    );

    reacquire_canonical_anchor_pair(
        rpc_client,
        rpc_url,
        &mut prepared,
        &mut next_generation_id,
    )
    .await?;

    if prepared.is_empty() {
        println!(concat!(
            "meteora_production_admission_unavailable: ",
            "no bounded runtime-ready Meteora pool observed or reacquired"
        ));
    } else {
        println!("READ-ONLY METEORA PRODUCTION ADMISSION PASS");
    }

    Ok(prepared)
}

async fn reacquire_canonical_anchor_pair(
    rpc_client: &Client,
    rpc_url: &str,
    prepared: &mut BTreeMap<String, MeteoraRuntimeQuoteState>,
    next_generation_id: &mut u64,
) -> Result<(), String> {
    for (anchor_mint, intermediate_mint) in [
        (WRAPPED_SOL_MINT, USDC_MINT),
        (WRAPPED_SOL_MINT, USDT_MINT),
    ] {
        if prepared
            .values()
            .any(|runtime| runtime_matches_pair(runtime, anchor_mint, intermediate_mint))
        {
            println!(
                "meteora_exact_pair_reacquisition_not_needed: anchor={} intermediate={}",
                anchor_mint, intermediate_mint
            );
            return Ok(());
        }

        println!(
            "meteora_exact_pair_reacquisition_start: anchor={} intermediate={}",
            anchor_mint, intermediate_mint
        );

        if let Some(runtime) = discover_exact_pair(
            rpc_client,
            rpc_url,
            anchor_mint,
            intermediate_mint,
            next_generation_id,
        )
        .await?
        {
            println!(
                concat!(
                    "meteora_exact_pair_reacquired: anchor={} intermediate={} ",
                    "pool={} source_slot={}"
                ),
                anchor_mint,
                intermediate_mint,
                runtime.normalized.pool_id,
                runtime.normalized.source_slot
            );
            insert_runtime_state(
                prepared,
                runtime,
                "meteora_exact_pair_runtime_prepared",
            );
            println!("READ-ONLY METEORA EXACT-PAIR REACQUISITION PASS");
            return Ok(());
        }

        println!(
            "meteora_exact_pair_reacquisition_unavailable: anchor={} intermediate={}",
            anchor_mint, intermediate_mint
        );
    }

    Ok(())
}

async fn discover_exact_pair(
    rpc_client: &Client,
    rpc_url: &str,
    anchor_mint: &str,
    intermediate_mint: &str,
    next_generation_id: &mut u64,
) -> Result<Option<MeteoraRuntimeQuoteState>, String> {
    for request in pair_lookup_requests(anchor_mint, intermediate_mint) {
        let request_id = request
            .get("id")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Meteora exact-pair request missing id".to_owned())?;
        let label = format!(
            "Meteora exact-pair lookup anchor={anchor_mint} intermediate={intermediate_mint} id={request_id}"
        );

        let payload = match fetch_program_accounts(rpc_client, rpc_url, &request, &label).await {
            Ok(payload) => payload,
            Err(error) => {
                println!(
                    "meteora_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                    anchor_mint, intermediate_mint
                );
                continue;
            }
        };

        let observations = match parse_pair_lookup_response(&payload) {
            Ok(observations) => observations,
            Err(error) => {
                println!(
                    "meteora_exact_pair_lookup_rejected: anchor={} intermediate={} reason={error}",
                    anchor_mint, intermediate_mint
                );
                continue;
            }
        };

        println!(
            "meteora_exact_pair_lookup_parsed: anchor={} intermediate={} observation_count={}",
            anchor_mint,
            intermediate_mint,
            observations.len()
        );

        for observation in observations
            .into_iter()
            .take(MAX_EXACT_PAIR_CANDIDATES_PER_ORIENTATION)
        {
            let trigger_received_at_unix_ms = crate::unix_time_ms_now()?;

            let runtime = match hydrate_with_next_generation(
                rpc_client,
                rpc_url,
                &observation,
                trigger_received_at_unix_ms,
                next_generation_id,
            )
            .await
            {
                Ok(runtime) => runtime,
                Err(error) => {
                    println!(
                        "meteora_exact_pair_candidate_rejected: pool={} reason={error}",
                        observation.pubkey
                    );
                    continue;
                }
            };

            if !runtime_matches_pair(&runtime, anchor_mint, intermediate_mint) {
                println!(
                    "meteora_exact_pair_candidate_rejected: pool={} reason=normalized mint pair mismatch",
                    runtime.normalized.pool_id
                );
                continue;
            }

            return Ok(Some(runtime));
        }
    }

    Ok(None)
}

fn pair_lookup_requests(anchor_mint: &str, intermediate_mint: &str) -> [Value; 2] {
    [
        pair_lookup_request(
            EXACT_PAIR_FORWARD_REQUEST_ID,
            anchor_mint,
            intermediate_mint,
        ),
        pair_lookup_request(
            EXACT_PAIR_REVERSE_REQUEST_ID,
            intermediate_mint,
            anchor_mint,
        ),
    ]
}

fn pair_lookup_request(request_id: u64, mint_x: &str, mint_y: &str) -> Value {
    let discriminator = bs58::encode(LB_PAIR_DISCRIMINATOR).into_string();

    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getProgramAccounts",
        "params": [
            METEORA_DLMM_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "withContext": true,
                "filters": [
                    {
                        "dataSize": LB_PAIR_ACCOUNT_LEN
                    },
                    {
                        "memcmp": {
                            "offset": 0,
                            "bytes": discriminator
                        }
                    },
                    {
                        "memcmp": {
                            "offset": METEORA_MINT_X_OFFSET,
                            "bytes": mint_x
                        }
                    },
                    {
                        "memcmp": {
                            "offset": METEORA_MINT_Y_OFFSET,
                            "bytes": mint_y
                        }
                    }
                ]
            }
        ]
    })
}

fn parse_pair_lookup_response(payload: &Value) -> Result<Vec<MeteoraLiveObservation>, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Meteora exact-pair getProgramAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "Meteora exact-pair getProgramAccounts response missing context slot".to_owned()
        })?;

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Meteora exact-pair getProgramAccounts response missing account array".to_owned()
        })?;

    let mut observations = Vec::with_capacity(accounts.len());

    for entry in accounts {
        let pubkey = entry.get("pubkey").and_then(Value::as_str).ok_or_else(|| {
            "Meteora exact-pair getProgramAccounts entry missing pubkey".to_owned()
        })?;
        let account = entry.get("account").ok_or_else(|| {
            "Meteora exact-pair getProgramAccounts entry missing account".to_owned()
        })?;

        let notification = json!({
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": slot
                    },
                    "value": {
                        "pubkey": pubkey,
                        "account": account
                    }
                }
            }
        });

        let observation = parse_meteora_program_notification(&notification)?
            .ok_or_else(|| "Meteora exact-pair lookup account did not decode".to_owned())?;

        if observations
            .iter()
            .any(|existing: &MeteoraLiveObservation| existing.pubkey == observation.pubkey)
        {
            continue;
        }

        observations.push(observation);
    }

    Ok(observations)
}

async fn fetch_program_accounts(
    rpc_client: &Client,
    rpc_url: &str,
    request: &Value,
    label: &str,
) -> Result<Value, String> {
    let started_at = Instant::now();

    let request_id = request
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("{label} request missing id"))?;

    println!(
        "rpc_request_start: label={} id={} method=getProgramAccounts",
        label, request_id
    );

    let payload = rpc_transport::post_json(rpc_client, rpc_url, request, label)
        .await
        .map_err(|error| {
            format!(
                "{label} RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    println!(
        "rpc_request_finish: label={} elapsed_ms={} rpc_error={}",
        label,
        started_at.elapsed().as_millis(),
        payload.get("error").is_some()
    );

    Ok(payload)
}

async fn hydrate_with_next_generation(
    rpc_client: &Client,
    rpc_url: &str,
    observation: &MeteoraLiveObservation,
    trigger_received_at_unix_ms: u64,
    next_generation_id: &mut u64,
) -> Result<MeteoraRuntimeQuoteState, String> {
    let base_generation_id = *next_generation_id;
    let quote_generation_id = base_generation_id
        .checked_add(1)
        .ok_or_else(|| "Meteora runtime generation id overflow".to_owned())?;
    *next_generation_id = quote_generation_id
        .checked_add(1)
        .ok_or_else(|| "Meteora runtime generation id overflow".to_owned())?;

    hydrate_observation(
        rpc_client,
        rpc_url,
        observation,
        trigger_received_at_unix_ms,
        base_generation_id,
        quote_generation_id,
    )
    .await
}

fn insert_runtime_state(
    prepared: &mut BTreeMap<String, MeteoraRuntimeQuoteState>,
    runtime: MeteoraRuntimeQuoteState,
    label: &str,
) {
    let pool_id = runtime.normalized.pool_id.clone();
    let source_slot = runtime.normalized.source_slot;

    let replace = should_replace_runtime_state(
        source_slot,
        prepared
            .get(&pool_id)
            .map(|existing| existing.normalized.source_slot),
    );

    if replace {
        println!(
            "{}: pool={} source_slot={} generation_id={}",
            label,
            pool_id,
            source_slot,
            runtime.snapshot.source().generation_id
        );
        prepared.insert(pool_id, runtime);
    }
}

fn runtime_matches_pair(
    runtime: &MeteoraRuntimeQuoteState,
    anchor_mint: &str,
    intermediate_mint: &str,
) -> bool {
    let token_a = runtime.normalized.token_a.mint.as_str();
    let token_b = runtime.normalized.token_b.mint.as_str();

    (token_a == anchor_mint && token_b == intermediate_mint)
        || (token_a == intermediate_mint && token_b == anchor_mint)
}

async fn hydrate_observation(
    rpc_client: &Client,
    rpc_url: &str,
    observation: &MeteoraLiveObservation,
    trigger_received_at_unix_ms: u64,
    base_generation_id: u64,
    quote_generation_id: u64,
) -> Result<MeteoraRuntimeQuoteState, String> {
    let base_pubkeys = meteora_base_hydration_account_pubkeys(observation)?;
    let base_payload = fetch_multiple_accounts(
        rpc_client,
        rpc_url,
        BASE_HYDRATION_REQUEST_ID,
        &base_pubkeys,
        observation.slot,
        "Meteora runtime base hydration",
    )
    .await?;

    let base_hydrated_at_unix_ms = crate::unix_time_ms_now()?;
    let base = parse_meteora_base_hydration_response(
        observation,
        &base_payload,
        base_generation_id,
        trigger_received_at_unix_ms,
        base_hydrated_at_unix_ms,
    )?;

    let base_source_slot = base.snapshot.source().source_slot;
    let bin_array_pubkeys = certified_directional_bin_array_pubkeys(&base.snapshot)?;
    let quote_pubkeys = meteora_quote_hydration_account_pubkeys(observation, &bin_array_pubkeys)?;
    let quote_payload = fetch_multiple_accounts(
        rpc_client,
        rpc_url,
        QUOTE_HYDRATION_REQUEST_ID,
        &quote_pubkeys,
        base_source_slot,
        "Meteora runtime quote hydration",
    )
    .await?;

    let quote_hydrated_at_unix_ms = crate::unix_time_ms_now()?;
    let prepared = parse_meteora_quote_hydration_response(
        observation,
        &bin_array_pubkeys,
        &quote_payload,
        quote_generation_id,
        trigger_received_at_unix_ms,
        quote_hydrated_at_unix_ms,
    )?;

    let quote_plan = certified_directional_bin_array_pubkeys(&prepared.snapshot)?;
    if quote_plan != bin_array_pubkeys {
        return Err(format!(
            concat!(
                "Meteora runtime BinArray plan changed between base and quote hydration: ",
                "base={:?} quote={:?}"
            ),
            bin_array_pubkeys, quote_plan
        ));
    }

    let snapshot_source_slot = prepared.snapshot.source().source_slot;
    if prepared.normalized.source_slot != snapshot_source_slot {
        return Err(format!(
            concat!(
                "Meteora runtime normalized/snapshot source-slot mismatch: ",
                "normalized={} snapshot={}"
            ),
            prepared.normalized.source_slot, snapshot_source_slot
        ));
    }

    MeteoraM13PreparedQuote::from_snapshot(&prepared.normalized, &prepared.snapshot)?;

    Ok(MeteoraRuntimeQuoteState {
        normalized: prepared.normalized,
        snapshot: prepared.snapshot,
    })
}

fn certified_directional_bin_array_pubkeys(
    snapshot: &MeteoraDlmmSnapshot,
) -> Result<Vec<String>, String> {
    certified_directional_bin_array_pubkeys_from_parts(
        snapshot.lb_pair_pubkey(),
        snapshot.lb_pair().active_id,
        &snapshot.lb_pair().bin_array_bitmap,
        snapshot.bitmap_extension(),
    )
}

fn certified_directional_bin_array_pubkeys_from_parts(
    lb_pair_pubkey: [u8; 32],
    active_id: i32,
    bin_array_bitmap: &MeteoraInternalBitmap,
    bitmap_extension: Option<&MeteoraBitmapExtensionState>,
) -> Result<Vec<String>, String> {
    let start_index = bin_id_to_bin_array_index(active_id)
        .map_err(|error| format!("Meteora runtime active BinArray index failed: {error:?}"))?;
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for swap_for_y in [true, false] {
        let terminal = if swap_for_y {
            BIN_ARRAY_MIN_INDEX
        } else {
            BIN_ARRAY_MAX_INDEX
        };
        let step = if swap_for_y { -1_i64 } else { 1_i64 };
        let mut index = start_index;
        let mut taken = 0usize;

        while taken < MAX_BIN_ARRAYS_PER_DIRECTION {
            let outside_internal =
                !(INTERNAL_BITMAP_MIN_INDEX..=INTERNAL_BITMAP_MAX_INDEX).contains(&index);

            if outside_internal && bitmap_extension.is_none() {
                break;
            }

            let initialized = bin_array_bitmap_bit(bin_array_bitmap, bitmap_extension, index)
                .map_err(|error| {
                    format!(
                        concat!(
                            "Meteora runtime certified BinArray search failed: ",
                            "direction={} index={} error={:?}"
                        ),
                        direction_label(swap_for_y),
                        index,
                        error
                    )
                })?;

            if initialized {
                let (pubkey, _) = derive_bin_array_pda(lb_pair_pubkey, index).map_err(|error| {
                    format!("Meteora runtime BinArray PDA derivation failed: {error:?}")
                })?;
                let encoded = bs58::encode(pubkey).into_string();

                if seen.insert(encoded.clone()) {
                    ordered.push(encoded);
                }

                taken = taken
                    .checked_add(1)
                    .ok_or_else(|| "Meteora runtime BinArray take counter overflow".to_owned())?;
            }

            if index == terminal {
                break;
            }

            index = index.checked_add(step).ok_or_else(|| {
                format!(
                    concat!(
                        "Meteora runtime directional BinArray index overflow: ",
                        "direction={} index={}"
                    ),
                    direction_label(swap_for_y),
                    index
                )
            })?;
        }
    }

    if ordered.is_empty() {
        return Err("Meteora runtime pool has no initialized BinArrays".to_owned());
    }

    Ok(ordered)
}

async fn fetch_multiple_accounts(
    rpc_client: &Client,
    rpc_url: &str,
    request_id: u64,
    pubkeys: &[String],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String> {
    if pubkeys.is_empty() {
        return Err(format!("{label} account list must not be empty"));
    }

    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getMultipleAccounts",
        "params": [
            pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64",
                "minContextSlot": min_context_slot
            }
        ]
    });

    let started_at = Instant::now();

    println!(
        "rpc_request_start: label={} id={} method=getMultipleAccounts account_count={}",
        label,
        request_id,
        pubkeys.len()
    );

    let payload = rpc_transport::post_json(rpc_client, rpc_url, &request, label)
        .await
        .map_err(|error| {
            format!(
                "{label} RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    println!(
        "rpc_request_finish: label={} id={} elapsed_ms={} rpc_error={}",
        label,
        request_id,
        started_at.elapsed().as_millis(),
        payload.get("error").is_some()
    );

    Ok(payload)
}

fn should_replace_runtime_state(source_slot: u64, existing_source_slot: Option<u64>) -> bool {
    match existing_source_slot {
        Some(existing_source_slot) => source_slot >= existing_source_slot,
        None => true,
    }
}

fn direction_label(swap_for_y: bool) -> &'static str {
    if swap_for_y {
        "x_to_y"
    } else {
        "y_to_x"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_cli::meteora::{
        DlmmProtocolProfile, MeteoraClockSnapshot, MeteoraDlmmFailure, MeteoraLbPairState,
        MeteoraSnapshotSource, MAX_BIN_PER_ARRAY,
    };
    use scout_core::{
        NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
    };

    const LB_PAIR: [u8; 32] = [7_u8; 32];

    #[test]
    fn runtime_subscription_reuses_certified_meteora_live_request() {
        let request = program_subscribe_request();

        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("programSubscribe")
        );
        assert_eq!(request.get("id").and_then(Value::as_u64), Some(60));
        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(METEORA_DLMM_PROGRAM_ID)
        );
    }

    #[test]
    fn exact_pair_lookup_requests_are_bounded_and_bidirectional() -> Result<(), String> {
        let requests = pair_lookup_requests(WRAPPED_SOL_MINT, USDC_MINT);
        let discriminator = bs58::encode(LB_PAIR_DISCRIMINATOR).into_string();
        let expected = [
            (EXACT_PAIR_FORWARD_REQUEST_ID, WRAPPED_SOL_MINT, USDC_MINT),
            (EXACT_PAIR_REVERSE_REQUEST_ID, USDC_MINT, WRAPPED_SOL_MINT),
        ];

        assert_eq!(requests.len(), 2);

        for (request, (request_id, mint_x, mint_y)) in requests.iter().zip(expected) {
            assert_eq!(request.get("id").and_then(Value::as_u64), Some(request_id));
            assert_eq!(
                request.get("method").and_then(Value::as_str),
                Some("getProgramAccounts")
            );
            assert_eq!(
                request.pointer("/params/0").and_then(Value::as_str),
                Some(METEORA_DLMM_PROGRAM_ID)
            );
            assert_eq!(
                request
                    .pointer("/params/1/withContext")
                    .and_then(Value::as_bool),
                Some(true)
            );

            let filters = request
                .pointer("/params/1/filters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Meteora exact-pair lookup must contain filters".to_owned())?;

            assert_eq!(
                filters.len(),
                4,
                "exact-pair lookup must never degrade to a broad pool scan"
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/0/dataSize")
                    .and_then(Value::as_u64),
                Some(LB_PAIR_ACCOUNT_LEN as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/1/memcmp/offset")
                    .and_then(Value::as_u64),
                Some(0)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/1/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(discriminator.as_str())
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/2/memcmp/offset")
                    .and_then(Value::as_u64),
                Some(METEORA_MINT_X_OFFSET as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/2/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(mint_x)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/3/memcmp/offset")
                    .and_then(Value::as_u64),
                Some(METEORA_MINT_Y_OFFSET as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/3/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(mint_y)
            );
        }

        Ok(())
    }

    #[test]
    fn exact_pair_parser_rejects_rpc_errors() -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "id": EXACT_PAIR_FORWARD_REQUEST_ID,
            "error": {
                "code": -32000,
                "message": "bounded test error"
            }
        });

        let error = match parse_pair_lookup_response(&payload) {
            Ok(_) => return Err("Meteora exact-pair RPC error did not fail closed".to_owned()),
            Err(error) => error,
        };

        assert!(error.contains("returned an RPC error"));
        Ok(())
    }

    #[test]
    fn certified_selector_collects_three_initialized_arrays_per_direction(
    ) -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = test_lb_pair(0);
        for index in [-3_i64, -2, -1, 0, 1, 2, 3] {
            set_internal_bit(&mut lb_pair.bin_array_bitmap, index);
        }
        let snapshot = test_snapshot(lb_pair, None)?;

        let selected = certified_directional_bin_array_pubkeys(&snapshot)
            .map_err(|_| MeteoraDlmmFailure::InvalidLayout)?;
        let expected_indexes = [0_i64, -1, -2, 1, 2];
        let expected = expected_indexes
            .into_iter()
            .map(|index| {
                derive_bin_array_pda(LB_PAIR, index)
                    .map(|(pubkey, _)| bs58::encode(pubkey).into_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(selected, expected);
        Ok(())
    }

    #[test]
    fn certified_selector_stops_cleanly_without_bitmap_extension() -> Result<(), MeteoraDlmmFailure>
    {
        let mut lb_pair = test_lb_pair(511 * MAX_BIN_PER_ARRAY);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 511);
        let snapshot = test_snapshot(lb_pair, None)?;

        let selected = certified_directional_bin_array_pubkeys(&snapshot)
            .map_err(|_| MeteoraDlmmFailure::InvalidLayout)?;
        let (pubkey, _) = derive_bin_array_pda(LB_PAIR, 511)?;

        assert_eq!(selected, vec![bs58::encode(pubkey).into_string()]);
        Ok(())
    }

    #[test]
    fn newer_source_slot_replaces_existing_runtime_state() {
        assert!(should_replace_runtime_state(101, Some(100)));
        assert!(should_replace_runtime_state(100, Some(100)));
        assert!(!should_replace_runtime_state(99, Some(100)));
        assert!(should_replace_runtime_state(1, None));
    }

    #[test]
    fn prepared_meteora_runtime_state_produces_quote_readiness() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(test_lb_pair(0), None)?;
        let normalized = test_normalized_pool(&snapshot);
        let mut meteora_runtime = BTreeMap::new();
        meteora_runtime.insert(
            normalized.pool_id.clone(),
            MeteoraRuntimeQuoteState {
                normalized: test_normalized_pool(&snapshot),
                snapshot,
            },
        );

        let readiness = crate::runtime_quote::readiness_for_pool_with_meteora(
            &normalized,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &meteora_runtime,
        );

        assert!(readiness.is_some());
        Ok(())
    }

    #[test]
    fn missing_meteora_runtime_state_fails_readiness_closed() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(test_lb_pair(0), None)?;
        let normalized = test_normalized_pool(&snapshot);

        let readiness = crate::runtime_quote::readiness_for_pool_with_meteora(
            &normalized,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        assert!(readiness.is_none());
        Ok(())
    }

    fn test_snapshot(
        lb_pair: MeteoraLbPairState,
        extension: Option<MeteoraBitmapExtensionState>,
    ) -> Result<MeteoraDlmmSnapshot, MeteoraDlmmFailure> {
        MeteoraDlmmSnapshot::new(
            LB_PAIR,
            lb_pair,
            Vec::new(),
            extension,
            MeteoraClockSnapshot {
                slot: 100,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            DlmmProtocolProfile::V0_12,
            MeteoraSnapshotSource {
                source_slot: 100,
                generation_id: 1,
            },
        )
    }

    fn test_normalized_pool(snapshot: &MeteoraDlmmSnapshot) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: bs58::encode(snapshot.lb_pair_pubkey()).into_string(),
            venue: Venue::Meteora,
            program_id: METEORA_DLMM_PROGRAM_ID.to_owned(),
            source_slot: snapshot.source().source_slot,
            token_a: NormalizedToken {
                mint: bs58::encode(snapshot.mint_x()).into_string(),
                vault: bs58::encode([3_u8; 32]).into_string(),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: bs58::encode(snapshot.mint_y()).into_string(),
                vault: bs58::encode([4_u8; 32]).into_string(),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Unavailable,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    fn test_lb_pair(active_id: i32) -> MeteoraLbPairState {
        MeteoraLbPairState {
            base_factor: 1,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
            active_id,
            bin_step: 1,
            mint_x: [1_u8; 32],
            mint_y: [2_u8; 32],
            bin_array_bitmap: [0_u64; 16],
        }
    }

    fn set_internal_bit(bitmap: &mut [u64; 16], index: i64) {
        let offset = index - INTERNAL_BITMAP_MIN_INDEX;
        let word_index = (offset / 64) as usize;
        let bit_index = (offset % 64) as u32;
        bitmap[word_index] |= 1_u64 << bit_index;
    }
}
