use crate::costs::{
    select_priority_fee, PriorityFeeObservation, PriorityFeeObservationSample, PriorityObservationState,
};
use reqwest::Client;
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::str::FromStr;
use std::time::Instant;

const ORCA_PRIORITY_FEE_RPC_REQUEST_ID: u64 = 19;
const MAX_LOCALIZED_PRIORITY_ACCOUNTS: usize = 128;
const ORCA_PRIORITY_PROVENANCE: &str = concat!(
    "Orca SwapV2 deterministic protocol writable subset: whirlpool, token_vault_a, ",
    "token_vault_b, five bounded tick arrays, oracle; not the complete future transaction ",
    "writable set; executor-dependent user token accounts are excluded"
);

pub async fn observe(
    rpc_client: &Client,
    rpc_url: &str,
    pool_id: &str,
    accounts: &[String],
) -> PriorityObservationState {
    let observation = match fetch_observation(rpc_client, rpc_url, pool_id, accounts).await {
        Ok(observation) => observation,
        Err(error) => {
            println!("orca_priority_observation_unavailable: pool={pool_id} reason={error}");
            return PriorityObservationState::Unavailable(error);
        }
    };

    println!(
        "orca_priority_observation: pool={} {}",
        pool_id,
        observation.summary()
    );

    match select_priority_fee(&observation) {
        Ok(Some(selection)) => {
            println!(
                "orca_priority_selection: pool={} {}",
                pool_id,
                selection.summary()
            );
        }
        Ok(None) => {
            println!(
                "orca_priority_selection_unknown: pool={} reason=no positive localized samples",
                pool_id
            );
        }
        Err(error) => {
            println!("orca_priority_selection_rejected: pool={pool_id} reason={error}");
            return PriorityObservationState::Unavailable(error);
        }
    }

    PriorityObservationState::Available(observation)
}

async fn fetch_observation(
    rpc_client: &Client,
    rpc_url: &str,
    pool_id: &str,
    accounts: &[String],
) -> Result<PriorityFeeObservation, String> {
    validate_accounts(accounts)?;

    let request = json!({
        "jsonrpc": "2.0",
        "id": ORCA_PRIORITY_FEE_RPC_REQUEST_ID,
        "method": "getRecentPrioritizationFees",
        "params": [accounts]
    });

    let started_at = Instant::now();

    println!(
        concat!(
            "rpc_request_start: label=Orca localized priority pool={} ",
            "method=getRecentPrioritizationFees account_count={}"
        ),
        pool_id,
        accounts.len()
    );

    let response = rpc_client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Orca localized priority RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Orca localized priority RPC returned HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        ));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        format!(
            "Orca localized priority RPC returned invalid JSON after {} ms: {error}",
            started_at.elapsed().as_millis()
        )
    })?;

    parse_response(&payload, accounts)
}

fn validate_accounts(accounts: &[String]) -> Result<(), String> {
    if accounts.is_empty() {
        return Err("Orca localized priority contention footprint must not be empty".to_owned());
    }

    if accounts.len() > MAX_LOCALIZED_PRIORITY_ACCOUNTS {
        return Err(format!(
            concat!(
                "Orca localized priority contention footprint exceeds RPC maximum: ",
                "count={} max={}"
            ),
            accounts.len(),
            MAX_LOCALIZED_PRIORITY_ACCOUNTS
        ));
    }

    for account in accounts {
        Pubkey::from_str(account).map_err(|error| {
            format!(
                "Orca localized priority contention account is invalid: account={account} error={error}"
            )
        })?;
    }

    Ok(())
}

fn parse_response(payload: &Value, accounts: &[String]) -> Result<PriorityFeeObservation, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Orca getRecentPrioritizationFees returned an RPC error: {error}"
        ));
    }

    let jsonrpc = payload
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| "Orca priority-fee response missing jsonrpc version".to_owned())?;

    if jsonrpc != "2.0" {
        return Err(format!(
            "Orca priority-fee response has unexpected jsonrpc version: {jsonrpc}"
        ));
    }

    let response_id = payload
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Orca priority-fee response missing numeric id".to_owned())?;

    if response_id != ORCA_PRIORITY_FEE_RPC_REQUEST_ID {
        return Err(format!(
            concat!(
                "Orca priority-fee response id mismatch: expected={} actual={}"
            ),
            ORCA_PRIORITY_FEE_RPC_REQUEST_ID, response_id
        ));
    }

    let result = payload
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| "Orca priority-fee response missing result array".to_owned())?;

    let mut samples = Vec::with_capacity(result.len());

    for row in result {
        let slot = row
            .get("slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| "Orca priority-fee observation missing slot".to_owned())?;

        let micro_lamports_per_cu = row
            .get("prioritizationFee")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!(
                    "Orca priority-fee observation at slot {slot} missing prioritizationFee"
                )
            })?;

        samples.push(PriorityFeeObservationSample {
            slot,
            micro_lamports_per_cu,
        });
    }

    Ok(PriorityFeeObservation {
        samples,
        scope_accounts: accounts.to_vec(),
        scope_provenance: ORCA_PRIORITY_PROVENANCE.to_owned(),
    })
}

                  
