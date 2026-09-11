use reqwest::Client;
use scout_cli::meteora::METEORA_DLMM_PROGRAM_ID;
use scout_cli::meteora_contention::meteora_quote_contention_footprint;
use scout_cli::meteora_live::{
    meteora_base_hydration_account_pubkeys, meteora_initial_bin_array_pubkeys,
    meteora_quote_hydration_account_pubkeys, parse_meteora_base_hydration_response,
    parse_meteora_program_notification, parse_meteora_quote_hydration_response,
    MeteoraLiveObservation,
};
use scout_cli::meteora_m13::{meteora_m13_pair, MeteoraM13PreparedQuote};
use scout_cli::rpc_transport;
use serde_json::{json, Value};
use std::fs;
use std::path::Path;
use std::time::{SystemTime, UNIX_EPOCH};
use tokio::time::Duration;

const SOLANA_RPC_URL: &str = "https://api.mainnet-beta.solana.com";
const METEORA_POOL: &str = "314P1kBWgm7t2vdg8j9HMKPv83q99DNhu4NeXMA6KwsG";
const RAYDIUM_COUNTERPART: &str = "fAjTnZ9QqJkUmrr8cXutkYhpVge2qqtSZNt9qKn7YC2";
const WRAPPED_SOL_MINT: &str = "So11111111111111111111111111111111111111112";
const USDC_MINT: &str = "EPjFWdd5AufqSSqeM2qN1xzybapC8G4wEGGkZwyTDt1v";
const POOL_REQUEST_ID: u64 = 71;
const BASE_REQUEST_ID: u64 = 72;
const QUOTE_REQUEST_ID: u64 = 73;
const BASE_GENERATION_ID: u64 = 1;
const QUOTE_GENERATION_ID: u64 = 2;
const QUOTE_AMOUNTS_RAW: [u64; 5] = [999_750, 1_000_000, 100_000, 10_000, 1_000];
const RPC_CONNECT_TIMEOUT: Duration = Duration::from_secs(5);
const RPC_REQUEST_TIMEOUT: Duration = Duration::from_secs(20);
const EVIDENCE_PATH: &str = "artifacts/stage-d-meteora/contention.json";

#[tokio::main]
async fn main() -> Result<(), String> {
    rustls::crypto::ring::default_provider()
        .install_default()
        .map_err(|_| "could not install rustls ring crypto provider".to_owned())?;

    println!("Meteora Stage D live contention proof");
    println!("Read-only: no signing, submission, borrowing, or value movement.");

    let rpc_client = Client::builder()
        .connect_timeout(RPC_CONNECT_TIMEOUT)
        .timeout(RPC_REQUEST_TIMEOUT)
        .build()
        .map_err(|error| format!("could not build bounded Solana RPC client: {error}"))?;

    let observation = fetch_pool_observation(&rpc_client).await?;
    let received_at_unix_ms = unix_ms()?;
    let base_pubkeys = meteora_base_hydration_account_pubkeys(&observation)?;
    let base_payload = fetch_multiple_accounts(
        &rpc_client,
        BASE_REQUEST_ID,
        base_pubkeys.as_slice(),
        observation.slot,
        "Meteora Stage D base hydration",
    )
    .await?;
    let base = parse_meteora_base_hydration_response(
        &observation,
        &base_payload,
        BASE_GENERATION_ID,
        received_at_unix_ms,
        unix_ms()?,
    )?;

    validate_stage_c_pair(&base.snapshot)?;

    let bin_array_pubkeys = meteora_initial_bin_array_pubkeys(&base)?;
    let quote_pubkeys = meteora_quote_hydration_account_pubkeys(&observation, &bin_array_pubkeys)?;
    let quote_payload = fetch_multiple_accounts(
        &rpc_client,
        QUOTE_REQUEST_ID,
        &quote_pubkeys,
        base.snapshot.source().source_slot,
        "Meteora Stage D quote hydration",
    )
    .await?;
    let hydrated_at_unix_ms = unix_ms()?;
    let prepared = parse_meteora_quote_hydration_response(
        &observation,
        &bin_array_pubkeys,
        &quote_payload,
        QUOTE_GENERATION_ID,
        received_at_unix_ms,
        hydrated_at_unix_ms,
    )?;
    let prepared_quote =
        MeteoraM13PreparedQuote::from_snapshot(&prepared.normalized, &prepared.snapshot)?;
    let quote = first_full_fill_quote(&prepared_quote)?;
    let footprint = meteora_quote_contention_footprint(&prepared.snapshot, &quote)?;

    let expected_account_count = 4usize
        .checked_add(usize::from(prepared.snapshot.bitmap_extension().is_some()))
        .and_then(|count| count.checked_add(quote.touched_bin_arrays.len()))
        .ok_or_else(|| "Meteora Stage D contention account count overflow".to_owned())?;

    if footprint.accounts.len() != expected_account_count {
        return Err(format!(
            "Meteora Stage D contention account count mismatch: expected={} actual={}",
            expected_account_count,
            footprint.accounts.len()
        ));
    }

    let source = prepared.snapshot.source();
    let account_evidence = footprint
        .accounts
        .iter()
        .map(|account| {
            json!({
                "kind": format!("{:?}", account.kind),
                "pubkey": bs58::encode(account.pubkey).into_string(),
            })
        })
        .collect::<Vec<_>>();
    let evidence = json!({
        "schema": "scout.meteora.stage-d-contention.v1",
        "scope": "read-only deterministic venue-local contention proof",
        "rpc_url": SOLANA_RPC_URL,
        "pool": METEORA_POOL,
        "stage_c_route_context": {
            "pair": "WSOL/USDC",
            "raydium_counterpart": RAYDIUM_COUNTERPART,
            "provenance": "exact Meteora pool from certified Stage C live route evidence"
        },
        "source_slot": source.source_slot,
        "generation_id": source.generation_id,
        "quote": {
            "input_mint": quote.input_mint.as_str(),
            "output_mint": quote.output_mint.as_str(),
            "requested_input_raw": quote.requested_input_raw,
            "consumed_input_raw": quote.consumed_input_raw,
            "unspent_input_raw": quote.unspent_input_raw,
            "amount_out_raw": quote.amount_out_raw,
            "swap_for_y": quote.swap_for_y,
            "touched_bin_arrays": &quote.touched_bin_arrays,
        },
        "contention": {
            "account_count": footprint.accounts.len(),
            "accounts": account_evidence,
            "provenance": footprint.provenance(),
            "complete_future_transaction_writable_set": false,
        },
        "captured_at_unix_ms": hydrated_at_unix_ms,
    });

    write_evidence(&evidence)?;

    println!(
        concat!(
            "meteora_stage_d_live_contention: pool={} input={} output={} ",
            "amount_in_raw={} amount_out_raw={} source_slot={} generation_id={} ",
            "touched_bin_arrays={:?} account_count={}"
        ),
        METEORA_POOL,
        quote.input_mint,
        quote.output_mint,
        quote.requested_input_raw,
        quote.amount_out_raw,
        source.source_slot,
        source.generation_id,
        quote.touched_bin_arrays,
        footprint.accounts.len()
    );
    println!("meteora_stage_d_live_accounts: [{}]", footprint.account_pubkeys_base58().join(","));
    println!("meteora_stage_d_live_provenance: {}", footprint.provenance());
    println!("meteora_stage_d_evidence={EVIDENCE_PATH}");
    println!("READ-ONLY METEORA STAGE D CONTENTION FOOTPRINT PASS");

    Ok(())
}

async fn fetch_pool_observation(rpc_client: &Client) -> Result<MeteoraLiveObservation, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": POOL_REQUEST_ID,
        "method": "getAccountInfo",
        "params": [
            METEORA_POOL,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    });
    let payload = rpc_transport::post_json(
        rpc_client,
        SOLANA_RPC_URL,
        &request,
        "Meteora Stage D pool observation",
    )
    .await?;

    if let Some(error) = payload.get("error") {
        return Err(format!("Meteora Stage D getAccountInfo returned an RPC error: {error}"));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Meteora Stage D pool observation missing context slot".to_owned())?;
    let account = payload
        .pointer("/result/value")
        .ok_or_else(|| "Meteora Stage D pool observation missing account".to_owned())?;

    if account.is_null() {
        return Err("Meteora Stage D certified route pool is missing".to_owned());
    }

    let notification = json!({
        "method": "programNotification",
        "params": {
            "result": {
                "context": {"slot": slot},
                "value": {
                    "pubkey": METEORA_POOL,
                    "account": account
                }
            }
        }
    });

    parse_meteora_program_notification(&notification)?
        .ok_or_else(|| "Meteora Stage D certified route pool did not decode".to_owned())
}

async fn fetch_multiple_accounts(
    rpc_client: &Client,
    request_id: u64,
    account_pubkeys: &[String],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String> {
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

    rpc_transport::post_json(rpc_client, SOLANA_RPC_URL, &request, label).await
}

fn validate_stage_c_pair(snapshot: &scout_cli::meteora::MeteoraDlmmSnapshot) -> Result<(), String> {
    let (mint_x, mint_y) = meteora_m13_pair(snapshot);
    let expected = (mint_x == WRAPPED_SOL_MINT && mint_y == USDC_MINT)
        || (mint_x == USDC_MINT && mint_y == WRAPPED_SOL_MINT);

    if !expected {
        return Err(format!(
            "Meteora Stage D certified route pair changed: mint_x={mint_x} mint_y={mint_y}"
        ));
    }

    if snapshot.lb_pair_pubkey() != decode_pubkey(METEORA_POOL)? {
        return Err("Meteora Stage D certified route pool identity changed".to_owned());
    }

    Ok(())
}

fn first_full_fill_quote(
    prepared: &MeteoraM13PreparedQuote<'_>,
) -> Result<scout_cli::meteora_m13::MeteoraM13ExactInputQuote, String> {
    let mut last_error = None;

    for amount_in_raw in QUOTE_AMOUNTS_RAW {
        match prepared.quote_exact_input(USDC_MINT, amount_in_raw) {
            Ok(quote) => return Ok(quote),
            Err(error) => last_error = Some(error),
        }
    }

    Err(format!(
        "Meteora Stage D certified route produced no bounded USDC full-fill quote: {}",
        last_error.unwrap_or_else(|| "no quote attempt completed".to_owned())
    ))
}

fn decode_pubkey(encoded: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(encoded)
        .into_vec()
        .map_err(|error| format!("invalid Solana pubkey {encoded}: {error}"))?;
    let decoded_len = decoded.len();

    decoded.try_into().map_err(|_| {
        format!("invalid Solana pubkey length: value={encoded} decoded_len={decoded_len}")
    })
}

fn write_evidence(evidence: &Value) -> Result<(), String> {
    if let Some(parent) = Path::new(EVIDENCE_PATH).parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!("could not create Meteora Stage D evidence directory: {error}")
        })?;
    }

    let encoded = serde_json::to_vec_pretty(evidence)
        .map_err(|error| format!("could not encode Meteora Stage D evidence: {error}"))?;
    fs::write(EVIDENCE_PATH, encoded)
        .map_err(|error| format!("could not write Meteora Stage D evidence: {error}"))
}

fn unix_ms() -> Result<u64, String> {
    let elapsed = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?;

    u64::try_from(elapsed.as_millis())
        .map_err(|_| "Unix epoch milliseconds exceeded u64".to_owned())
}
