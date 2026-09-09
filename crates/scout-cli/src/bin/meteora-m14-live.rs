use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use reqwest::header::RETRY_AFTER;
use reqwest::Client;
use scout_cli::meteora::{
    bin_id_to_bin_array_index, derive_bin_array_pda, next_initialized_bin_array_index,
    BIN_ARRAY_ACCOUNT_LEN, BIN_ARRAY_DISCRIMINATOR, BIN_ARRAY_MAX_INDEX, BIN_ARRAY_MIN_INDEX,
    BIN_ARRAY_VERSION_OFFSET, BIN_ARRAY_VERSION_V3, METEORA_DLMM_PROGRAM_ID,
};
use scout_cli::meteora_live::{
    meteora_base_hydration_account_pubkeys, meteora_quote_hydration_account_pubkeys,
    parse_meteora_base_hydration_response, parse_meteora_program_notification,
    parse_meteora_quote_hydration_response, MeteoraLiveObservation, MeteoraLivePreparedState,
};
use scout_cli::meteora_m13::{
    meteora_m13_pair, MeteoraM13ExactInputQuote, MeteoraM13PreparedQuote,
};
use serde_json::{json, Value};
use std::collections::BTreeSet;
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::{sleep, Duration};

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const METEORA_DATA_API_URL: &str = "https://dlmm.datapi.meteora.ag/pools";
const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const QUOTE_AMOUNT_RAW: u64 = 1_000_000;
const MAX_BIN_ARRAYS_PER_DIRECTION: usize = 3;
const CANDIDATE_BATCH_SIZE: usize = 64;
const DISCOVERY_PAGE_SIZE: usize = 250;
const MAX_DISCOVERY_CANDIDATES: usize = DISCOVERY_PAGE_SIZE;
const MIN_DISCOVERY_TVL_USD: u64 = 10_000;
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const RPC_MAX_ATTEMPTS: usize = 5;
const RPC_INITIAL_BACKOFF: Duration = Duration::from_millis(500);
const M14_EVIDENCE_PATH: &str = "artifacts/m14-meteora/frozen-mainnet.json";

struct QualifiedM14Capture {
    pool: String,
    observation: MeteoraLiveObservation,
    base_source_slot: u64,
    bin_array_pubkeys: Vec<String>,
    final_pubkeys: Vec<String>,
    frozen_payload: Value,
    prepared: MeteoraLivePreparedState,
    frozen_hydrated_at_unix_ms: u64,
    x_to_y: MeteoraM13ExactInputQuote,
    y_to_x: MeteoraM13ExactInputQuote,
}

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    println!("Meteora M14 frozen/mainnet Scout capture");
    println!("Read-only: no signing, submission, borrowing, or value movement.");

    let rpc_client = Client::builder()
        .connect_timeout(RPC_CONNECT_TIMEOUT)
        .timeout(RPC_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("could not build bounded Solana RPC client: {error}"))?;

    let candidates = fetch_meteora_m14_candidates(&rpc_client).await?;
    let bounded_candidates = candidates
        .iter()
        .take(MAX_DISCOVERY_CANDIDATES)
        .cloned()
        .collect::<Vec<_>>();
    let mut rejection_count = 0usize;
    let mut examined_count = 0usize;
    let mut qualified = None;

    'candidate_batches: for (batch_index, candidate_batch) in
        bounded_candidates.chunks(CANDIDATE_BATCH_SIZE).enumerate()
    {
        let candidate_payload = fetch_candidate_accounts(&rpc_client, candidate_batch).await?;
        let candidate_slot = candidate_payload
            .pointer("/result/context/slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Meteora M14 candidate batch missing context slot".to_owned())?;
        let candidate_accounts = candidate_payload
            .pointer("/result/value")
            .and_then(Value::as_array)
            .ok_or_else(|| "Meteora M14 candidate batch missing result.value array".to_owned())?;
        if candidate_accounts.len() != candidate_batch.len() {
            return Err(format!(
                "Meteora M14 candidate batch account count mismatch: expected={} actual={}",
                candidate_batch.len(),
                candidate_accounts.len()
            ));
        }
        let trigger_received_at_unix_ms = unix_ms()?;

        println!(
            "meteora_m14_candidate_batch: batch={} candidates={} slot={}",
            batch_index + 1,
            candidate_batch.len(),
            candidate_slot
        );

        for (pool, account) in candidate_batch.iter().zip(candidate_accounts) {
            examined_count = examined_count
                .checked_add(1)
                .ok_or_else(|| "Meteora M14 examined counter overflow".to_owned())?;

            let result = match observation_from_account(pool, candidate_slot, account) {
                Ok(observation) => {
                    qualify_m14_candidate(
                        &rpc_client,
                        pool,
                        observation,
                        trigger_received_at_unix_ms,
                    )
                    .await
                }
                Err(error) => Err(error),
            };

            match result {
                Ok(capture) => {
                    qualified = Some(capture);
                    break 'candidate_batches;
                }
                Err(error) if is_rpc_infrastructure_error(&error) => {
                    return Err(format!(
                        "Meteora M14 certification infrastructure failure while qualifying \
                         pool={pool}: {error}"
                    ));
                }
                Err(error) => {
                    rejection_count = rejection_count
                        .checked_add(1)
                        .ok_or_else(|| "Meteora M14 rejection counter overflow".to_owned())?;
                    println!("meteora_m14_candidate_rejected: pool={pool} reason={error}");
                }
            }
        }
    }

    let capture = qualified.ok_or_else(|| {
        format!(
            "Meteora M14 found no qualified legacy-SPL v3 target within {} candidates; \
             rejected={rejection_count}",
            examined_count
        )
    })?;

    let (mint_x, mint_y) = meteora_m13_pair(&capture.prepared.snapshot);
    let x_to_y = &capture.x_to_y;
    let y_to_x = &capture.y_to_x;

    let source = capture.prepared.snapshot.source();
    let evidence = json!({
        "schema": "scout.meteora.m14.frozen-mainnet.v1",
        "scope": "read-only differential certification",
        "pool": capture.pool.as_str(),
        "rpc_url": SOLANA_RPC_URL,
        "discovery": {
            "source": METEORA_DATA_API_URL,
            "sort": "pool_created_at:desc",
            "filter": "is_blacklisted=false && tvl>10000",
            "max_candidates": MAX_DISCOVERY_CANDIDATES,
            "rejected_before_selection": rejection_count,
            "qualification": "legacy SPL mints + frozen v3 plan + bilateral full-fill quote",
        },
        "trigger_slot": capture.observation.slot,
        "base_source_slot": capture.base_source_slot,
        "source_slot": source.source_slot,
        "generation_id": source.generation_id,
        "quote_amount_raw": QUOTE_AMOUNT_RAW,
        "mint_x": mint_x.as_str(),
        "mint_y": mint_y.as_str(),
        "token_x_program": capture.prepared.token_x_program.as_str(),
        "token_y_program": capture.prepared.token_y_program.as_str(),
        "bin_array_pubkeys": &capture.bin_array_pubkeys,
        "frozen_account_pubkeys": &capture.final_pubkeys,
        "frozen_rpc_payload": &capture.frozen_payload,
        "scout_quotes": {
            "x_to_y": quote_json(x_to_y),
            "y_to_x": quote_json(y_to_x),
        },
        "reference": {
            "repository": "MeteoraAg/dlmm-sdk",
            "commit": "576919e3e4368e542c402f000b4264724f7f23ec",
            "function": "commons::quote::quote_exact_in",
            "rust_toolchain": "1.85.0",
        },
        "scout_toolchain": "1.80.0",
        "captured_at_unix_ms": capture.frozen_hydrated_at_unix_ms,
    });

    if let Some(parent) = Path::new(M14_EVIDENCE_PATH).parent() {
        fs::create_dir_all(parent)
            .map_err(|error| format!("could not create M14 evidence directory: {error}"))?;
    }

    let encoded = serde_json::to_vec_pretty(&evidence)
        .map_err(|error| format!("could not encode M14 evidence JSON: {error}"))?;
    fs::write(M14_EVIDENCE_PATH, encoded)
        .map_err(|error| format!("could not write M14 evidence fixture: {error}"))?;

    println!(
        "meteora_m14_capture: pool={} trigger_slot={} base_slot={} source_slot={} \
         bin_arrays={} rejected_candidates={}",
        capture.pool,
        capture.observation.slot,
        capture.base_source_slot,
        source.source_slot,
        capture.bin_array_pubkeys.len(),
        rejection_count
    );
    println!(
        "meteora_m14_scout_x_to_y: amount_in={} amount_out={} fee={} protocol_fee={}",
        x_to_y.requested_input_raw,
        x_to_y.amount_out_raw,
        x_to_y.trading_fee_raw,
        x_to_y.protocol_fee_raw
    );
    println!(
        "meteora_m14_scout_y_to_x: amount_in={} amount_out={} fee={} protocol_fee={}",
        y_to_x.requested_input_raw,
        y_to_x.amount_out_raw,
        y_to_x.trading_fee_raw,
        y_to_x.protocol_fee_raw
    );
    println!("meteora_m14_evidence={M14_EVIDENCE_PATH}");
    println!("READ-ONLY METEORA M14 FROZEN MAINNET CAPTURE PASS");

    Ok(())
}

async fn fetch_meteora_m14_candidates(client: &Client) -> Result<Vec<String>, String> {
    let filter = format!("is_blacklisted=false && tvl>{MIN_DISCOVERY_TVL_USD}");
    let response = client
        .get(METEORA_DATA_API_URL)
        .query(&[
            ("page", "1".to_owned()),
            ("page_size", DISCOVERY_PAGE_SIZE.to_string()),
            ("sort_by", "pool_created_at:desc".to_owned()),
            ("filter_by", filter),
        ])
        .send()
        .await
        .map_err(|error| format!("Meteora M14 Data API request failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Meteora M14 Data API returned HTTP status {status}"
        ));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("Meteora M14 Data API returned invalid JSON: {error}"))?;
    let pools = payload
        .get("data")
        .and_then(Value::as_array)
        .ok_or_else(|| "Meteora M14 Data API response missing data array".to_owned())?;

    let mut candidates = Vec::new();
    let mut seen = BTreeSet::new();
    for pool in pools {
        let Some(address) = pool.get("address").and_then(Value::as_str) else {
            continue;
        };
        if seen.insert(address.to_owned()) {
            candidates.push(address.to_owned());
        }
    }

    if candidates.is_empty() {
        return Err("Meteora M14 Data API returned no bounded candidates".to_owned());
    }

    println!(
        "meteora_m14_discovery: source={} candidates={} bounded_to={}",
        METEORA_DATA_API_URL,
        candidates.len(),
        candidates.len().min(MAX_DISCOVERY_CANDIDATES)
    );

    Ok(candidates)
}

async fn qualify_m14_candidate(
    client: &Client,
    pool: &str,
    observation: MeteoraLiveObservation,
    trigger_received_at_unix_ms: u64,
) -> Result<QualifiedM14Capture, String> {
    let base_pubkeys = meteora_base_hydration_account_pubkeys(&observation)?;
    let base_payload = fetch_multiple_accounts(
        client,
        &base_pubkeys,
        observation.slot,
        "Meteora M14 base hydration",
    )
    .await?;

    let base_hydrated_at_unix_ms = unix_ms()?;
    let base = parse_meteora_base_hydration_response(
        &observation,
        &base_payload,
        1,
        trigger_received_at_unix_ms,
        base_hydrated_at_unix_ms,
    )?;

    require_legacy_spl_token_programs(&base.token_x_program, &base.token_y_program)?;

    let base_source_slot = base.snapshot.source().source_slot;
    let bin_array_pubkeys = bounded_directional_bin_array_pubkeys(&base.snapshot)?;
    let final_pubkeys = meteora_quote_hydration_account_pubkeys(&observation, &bin_array_pubkeys)?;

    let frozen_payload = fetch_multiple_accounts_slice(
        client,
        &final_pubkeys,
        base_source_slot,
        "Meteora M14 frozen quote hydration",
    )
    .await?;

    require_v3_frozen_bin_array_plan(&frozen_payload, &bin_array_pubkeys)?;

    let frozen_hydrated_at_unix_ms = unix_ms()?;
    let prepared = parse_meteora_quote_hydration_response(
        &observation,
        &bin_array_pubkeys,
        &frozen_payload,
        2,
        trigger_received_at_unix_ms,
        frozen_hydrated_at_unix_ms,
    )?;

    require_legacy_spl_token_programs(&prepared.token_x_program, &prepared.token_y_program)?;

    let frozen_bin_array_pubkeys = bounded_directional_bin_array_pubkeys(&prepared.snapshot)?;
    if frozen_bin_array_pubkeys != bin_array_pubkeys {
        return Err(format!(
            "Meteora M14 BinArray plan changed between base and frozen snapshots: \
             base={bin_array_pubkeys:?} frozen={frozen_bin_array_pubkeys:?}"
        ));
    }

    let quote_context =
        MeteoraM13PreparedQuote::from_snapshot(&prepared.normalized, &prepared.snapshot)?;
    let (mint_x, mint_y) = meteora_m13_pair(&prepared.snapshot);
    let x_to_y = quote_context.quote_exact_input(&mint_x, QUOTE_AMOUNT_RAW)?;
    let y_to_x = quote_context.quote_exact_input(&mint_y, QUOTE_AMOUNT_RAW)?;

    require_quote_identity(&x_to_y, &mint_x, &mint_y, true)?;
    require_quote_identity(&y_to_x, &mint_y, &mint_x, false)?;

    println!(
        "meteora_m14_candidate_qualified: pool={} source_slot={} bin_arrays={}",
        pool,
        prepared.snapshot.source().source_slot,
        bin_array_pubkeys.len()
    );

    Ok(QualifiedM14Capture {
        pool: pool.to_owned(),
        observation,
        base_source_slot,
        bin_array_pubkeys,
        final_pubkeys,
        frozen_payload,
        prepared,
        frozen_hydrated_at_unix_ms,
        x_to_y,
        y_to_x,
    })
}

fn require_v3_frozen_bin_array_plan(
    payload: &Value,
    expected_bin_array_pubkeys: &[String],
) -> Result<(), String> {
    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Meteora M14 frozen hydration missing result.value array".to_owned())?;
    let expected_count = 5_usize
        .checked_add(expected_bin_array_pubkeys.len())
        .ok_or_else(|| "Meteora M14 frozen account count overflow".to_owned())?;

    if accounts.len() != expected_count {
        return Err(format!(
            "Meteora M14 frozen account count mismatch: expected={expected_count} actual={}",
            accounts.len()
        ));
    }

    for (offset, expected_pubkey) in expected_bin_array_pubkeys.iter().enumerate() {
        let account_index = 5_usize
            .checked_add(offset)
            .ok_or_else(|| "Meteora M14 frozen BinArray index overflow".to_owned())?;
        let account = accounts
            .get(account_index)
            .ok_or_else(|| format!("Meteora M14 frozen BinArray missing: {expected_pubkey}"))?;

        if account.is_null() {
            return Err(format!(
                "Meteora M14 frozen BinArray does not exist: {expected_pubkey}"
            ));
        }

        let owner = account
            .get("owner")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("Meteora M14 frozen BinArray missing owner: {expected_pubkey}")
            })?;
        if owner != METEORA_DLMM_PROGRAM_ID {
            return Err(format!(
                "Meteora M14 frozen BinArray owner mismatch: pubkey={expected_pubkey} owner={owner}"
            ));
        }

        let encoding = account
            .pointer("/data/1")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("Meteora M14 frozen BinArray missing encoding: {expected_pubkey}")
            })?;
        if encoding != "base64" {
            return Err(format!(
                "Meteora M14 frozen BinArray encoding mismatch: \
                 pubkey={expected_pubkey} encoding={encoding}"
            ));
        }

        let encoded = account
            .pointer("/data/0")
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("Meteora M14 frozen BinArray missing base64 data: {expected_pubkey}")
            })?;
        let data = BASE64_STANDARD.decode(encoded).map_err(|error| {
            format!("Meteora M14 frozen BinArray base64 decode failed: {expected_pubkey}: {error}")
        })?;

        if data.len() != BIN_ARRAY_ACCOUNT_LEN {
            return Err(format!(
                "Meteora M14 frozen BinArray length mismatch: pubkey={expected_pubkey} \
                 expected={} actual={}",
                BIN_ARRAY_ACCOUNT_LEN,
                data.len()
            ));
        }

        if data.get(..BIN_ARRAY_DISCRIMINATOR.len()) != Some(BIN_ARRAY_DISCRIMINATOR.as_slice()) {
            return Err(format!(
                "Meteora M14 frozen BinArray discriminator mismatch: {expected_pubkey}"
            ));
        }

        let version = *data.get(BIN_ARRAY_VERSION_OFFSET).ok_or_else(|| {
            format!("Meteora M14 frozen BinArray missing version byte: {expected_pubkey}")
        })?;
        if version != BIN_ARRAY_VERSION_V3 {
            return Err(format!(
                "Meteora M14 frozen BinArray is outside locked v3 profile: \
                 pubkey={expected_pubkey} version={version}"
            ));
        }
    }

    Ok(())
}

async fn fetch_candidate_accounts(client: &Client, pubkeys: &[String]) -> Result<Value, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1401,
        "method": "getMultipleAccounts",
        "params": [
            pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    });

    rpc_json(client, request, "Meteora M14 candidate batch").await
}

async fn fetch_multiple_accounts<const N: usize>(
    client: &Client,
    pubkeys: &[String; N],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String> {
    fetch_multiple_accounts_slice(client, pubkeys.as_slice(), min_context_slot, label).await
}

async fn fetch_multiple_accounts_slice(
    client: &Client,
    pubkeys: &[String],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 1402,
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

    rpc_json(client, request, label).await
}

async fn rpc_json(client: &Client, request: Value, label: &str) -> Result<Value, String> {
    let mut backoff = RPC_INITIAL_BACKOFF;

    for attempt in 1..=RPC_MAX_ATTEMPTS {
        let response = match client.post(SOLANA_RPC_URL).json(&request).send().await {
            Ok(response) => response,
            Err(error) => {
                if attempt == RPC_MAX_ATTEMPTS {
                    return Err(format!(
                        "{label} RPC infrastructure request failed after {attempt} attempts: {error}"
                    ));
                }
                println!(
                    "meteora_m14_rpc_retry: label={label} attempt={attempt} \
                     reason=request_error backoff_ms={}",
                    backoff.as_millis()
                );
                sleep(backoff).await;
                backoff = backoff.checked_mul(2).unwrap_or(Duration::from_secs(8));
                continue;
            }
        };

        let status = response.status();
        if status.as_u16() == 429 || status.is_server_error() {
            if attempt == RPC_MAX_ATTEMPTS {
                return Err(format!(
                    "{label} RPC infrastructure HTTP status {status} after {attempt} attempts"
                ));
            }
            let retry_after = if status.as_u16() == 429 {
                response
                    .headers()
                    .get(RETRY_AFTER)
                    .and_then(|value| value.to_str().ok())
                    .and_then(|value| value.parse::<u64>().ok())
                    .map(Duration::from_secs)
            } else {
                None
            };
            let retry_delay = retry_after.map_or(backoff, |server_delay| server_delay.max(backoff));
            println!(
                "meteora_m14_rpc_retry: label={label} attempt={attempt} status={status} \
                 backoff_ms={} retry_after_ms={} wait_ms={}",
                backoff.as_millis(),
                retry_after.map_or(0, |delay| delay.as_millis()),
                retry_delay.as_millis()
            );
            sleep(retry_delay).await;
            backoff = backoff.checked_mul(2).unwrap_or(Duration::from_secs(8));
            continue;
        }

        if !status.is_success() {
            return Err(format!("{label} RPC returned HTTP status {status}"));
        }

        let payload = response
            .json::<Value>()
            .await
            .map_err(|error| format!("{label} returned invalid JSON: {error}"))?;

        if let Some(error) = payload.get("error") {
            return Err(format!("{label} RPC error: {error}"));
        }

        return Ok(payload);
    }

    Err(format!("{label} RPC infrastructure retry loop exhausted"))
}

fn is_rpc_infrastructure_error(error: &str) -> bool {
    error.contains("RPC infrastructure ")
}

fn observation_from_account(
    pubkey: &str,
    slot: u64,
    account: &Value,
) -> Result<MeteoraLiveObservation, String> {
    if account.is_null() {
        return Err(format!("Meteora M14 pool account {pubkey} does not exist"));
    }

    let synthetic_notification = json!({
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

    parse_meteora_program_notification(&synthetic_notification)?
        .ok_or_else(|| "Meteora M14 target was not admitted as a DLMM observation".to_owned())
}

fn bounded_directional_bin_array_pubkeys(
    snapshot: &scout_cli::meteora::MeteoraDlmmSnapshot,
) -> Result<Vec<String>, String> {
    let start_index = bin_id_to_bin_array_index(snapshot.lb_pair().active_id)
        .map_err(|error| format!("Meteora M14 active BinArray index failed: {error:?}"))?;
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for swap_for_y in [true, false] {
        let terminal = if swap_for_y {
            BIN_ARRAY_MIN_INDEX
        } else {
            BIN_ARRAY_MAX_INDEX
        };
        let step = if swap_for_y { -1_i64 } else { 1_i64 };
        let mut search_index = start_index;
        let mut taken = 0usize;

        while taken < MAX_BIN_ARRAYS_PER_DIRECTION {
            let next = next_initialized_bin_array_index(
                &snapshot.lb_pair().bin_array_bitmap,
                snapshot.bitmap_extension(),
                search_index,
                swap_for_y,
            )
            .map_err(|error| {
                format!(
                    "Meteora M14 directional BinArray search failed: direction={} \
                     start={} error={error:?}",
                    direction_label(swap_for_y),
                    search_index
                )
            })?;

            let Some(index) = next else {
                break;
            };

            let (pubkey, _) = derive_bin_array_pda(snapshot.lb_pair_pubkey(), index)
                .map_err(|error| format!("Meteora M14 BinArray PDA failed: {error:?}"))?;
            let encoded = bs58::encode(pubkey).into_string();

            if seen.insert(encoded.clone()) {
                ordered.push(encoded);
            }

            taken += 1;

            if index == terminal {
                break;
            }

            search_index = index.checked_add(step).ok_or_else(|| {
                format!(
                    "Meteora M14 directional BinArray index overflow: direction={} index={index}",
                    direction_label(swap_for_y)
                )
            })?;
        }
    }

    if ordered.is_empty() {
        return Err("Meteora M14 target has no initialized BinArrays".to_owned());
    }

    Ok(ordered)
}

fn require_legacy_spl_token_programs(
    token_x_program: &str,
    token_y_program: &str,
) -> Result<(), String> {
    if token_x_program != SPL_TOKEN_PROGRAM_ID || token_y_program != SPL_TOKEN_PROGRAM_ID {
        return Err(format!(
            "Meteora M14 certification target must use legacy SPL Token on both mints: \
             token_x_program={token_x_program} token_y_program={token_y_program}"
        ));
    }

    Ok(())
}

fn require_quote_identity(
    quote: &MeteoraM13ExactInputQuote,
    expected_input: &str,
    expected_output: &str,
    expected_swap_for_y: bool,
) -> Result<(), String> {
    if quote.input_mint != expected_input
        || quote.output_mint != expected_output
        || quote.swap_for_y != expected_swap_for_y
    {
        return Err("Meteora M14 quote identity mismatch".to_owned());
    }

    if quote.requested_input_raw != QUOTE_AMOUNT_RAW
        || quote.consumed_input_raw != QUOTE_AMOUNT_RAW
        || quote.unspent_input_raw != 0
        || quote.amount_out_raw == 0
    {
        return Err(format!(
            "Meteora M14 quote failed full-fill requirement: input={} requested={} \
             consumed={} unspent={} output={}",
            quote.input_mint,
            quote.requested_input_raw,
            quote.consumed_input_raw,
            quote.unspent_input_raw,
            quote.amount_out_raw
        ));
    }

    Ok(())
}

fn quote_json(quote: &MeteoraM13ExactInputQuote) -> Value {
    json!({
        "input_mint": quote.input_mint.as_str(),
        "output_mint": quote.output_mint.as_str(),
        "requested_input_raw": quote.requested_input_raw,
        "consumed_input_raw": quote.consumed_input_raw,
        "unspent_input_raw": quote.unspent_input_raw,
        "amount_out_raw": quote.amount_out_raw,
        "trading_fee_raw": quote.trading_fee_raw,
        "protocol_fee_raw": quote.protocol_fee_raw,
        "user_fee_raw": quote.user_fee_raw,
        "fee_on_input": quote.fee_on_input,
        "swap_for_y": quote.swap_for_y,
        "touched_bin_arrays": &quote.touched_bin_arrays,
        "source_slot": quote.source_slot,
        "generation_id": quote.generation_id,
    })
}

fn direction_label(swap_for_y: bool) -> &'static str {
    if swap_for_y {
        "x_to_y"
    } else {
        "y_to_x"
    }
}

fn unix_ms() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before Unix epoch: {error}"))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| "system clock milliseconds overflow u64".to_owned())
}

