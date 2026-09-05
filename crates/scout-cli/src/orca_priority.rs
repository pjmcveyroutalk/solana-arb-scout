use crate::costs::{
    self, select_priority_fee, PriorityFeeObservation, PriorityFeeObservationSample,
    PriorityObservationState,
};
use crate::orca_live::PreparedOrca;
use crate::raydium::RaydiumHydrationSnapshot;
use crate::route::RouteLeg;
use reqwest::Client;
use scout_core::Venue;
use serde_json::{json, Value};
use solana_pubkey::Pubkey;
use std::collections::{BTreeMap, BTreeSet};
use std::str::FromStr;
use std::time::Instant;

const ORCA_PRIORITY_FEE_RPC_REQUEST_ID: u64 = 19;
const MAX_LOCALIZED_PRIORITY_ACCOUNTS: usize = 128;
const ORCA_PRIORITY_PROVENANCE: &str = concat!(
    "Orca SwapV2 deterministic protocol writable subset: whirlpool, token_vault_a, ",
    "token_vault_b, five bounded tick arrays, oracle; not the complete future transaction ",
    "writable set; executor-dependent user token accounts are excluded"
);

pub async fn observe_route(
    rpc_client: &Client,
    rpc_url: &str,
    leg_1: &RouteLeg,
    leg_2: &RouteLeg,
    raydium_quote_contexts: &BTreeMap<String, RaydiumHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, PreparedOrca>,
    cache: &mut BTreeMap<Vec<String>, PriorityObservationState>,
) -> PriorityObservationState {
    let (accounts, provenance) = match route_scope(
        leg_1,
        leg_2,
        raydium_quote_contexts,
        orca_prepared,
    ) {
        Ok(scope) => scope,
        Err(error) => {
            println!(
                "rung11c_priority_scope_unknown: leg1_pool={} leg2_pool={} reason={error}",
                leg_1.pool_id(),
                leg_2.pool_id()
            );
            return PriorityObservationState::Unavailable(error);
        }
    };

    println!(
        "rung11c_priority_scope: account_count={} accounts=[{}] provenance={}",
        accounts.len(),
        accounts.join(","),
        provenance
    );

    if let Some(cached) = cache.get(&accounts) {
        println!(
            "rung11c_priority_observation_cache_hit: account_count={}",
            accounts.len()
        );
        return cached.clone();
    }

    let observation = match fetch_observation(rpc_client, rpc_url, &accounts, &provenance).await {
        Ok(observation) => {
            println!("rung11c_priority_observation: {}", observation.summary());

            match select_priority_fee(&observation) {
                Ok(Some(selection)) => {
                    println!("rung11c_priority_selection: {}", selection.summary());
                    PriorityObservationState::Available(observation)
                }
                Ok(None) => {
                    println!(
                        "rung11c_priority_selection_unknown: no positive localized samples"
                    );
                    PriorityObservationState::Available(observation)
                }
                Err(error) => {
                    println!("rung11c_priority_selection_rejected: {error}");
                    PriorityObservationState::Unavailable(error)
                }
            }
        }
        Err(error) => {
            println!("rung11c_priority_observation_unavailable: {error}");
            PriorityObservationState::Unavailable(error)
        }
    };

    cache.insert(accounts, observation.clone());
    observation
}

fn route_scope(
    leg_1: &RouteLeg,
    leg_2: &RouteLeg,
    raydium_quote_contexts: &BTreeMap<String, RaydiumHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, PreparedOrca>,
) -> Result<(Vec<String>, String), String> {
    let (leg_1_accounts, leg_1_provenance) =
        venue_scope(leg_1, raydium_quote_contexts, orca_prepared)?;
    let (leg_2_accounts, leg_2_provenance) =
        venue_scope(leg_2, raydium_quote_contexts, orca_prepared)?;

    let accounts = leg_1_accounts
        .into_iter()
        .chain(leg_2_accounts)
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect::<Vec<_>>();

    validate_accounts(&accounts)?;

    Ok((
        accounts,
        format!(
            concat!(
                "two-leg deterministic venue contention union; leg1=[{}]; leg2=[{}]; ",
                "not the complete future transaction writable set"
            ),
            leg_1_provenance, leg_2_provenance
        ),
    ))
}

fn venue_scope(
    leg: &RouteLeg,
    raydium_quote_contexts: &BTreeMap<String, RaydiumHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, PreparedOrca>,
) -> Result<(Vec<String>, String), String> {
    match leg.venue() {
        Venue::RaydiumCpmm => {
            let snapshot = raydium_quote_contexts.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing Raydium priority context for route pool {}",
                    leg.pool_id()
                )
            })?;
            let footprint = costs::raydium_contention_footprint(leg.pool_id(), snapshot)?;
            Ok((footprint.accounts().to_vec(), footprint.provenance().to_owned()))
        }
        Venue::Orca => {
            let prepared = orca_prepared.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing Orca priority context for route pool {}",
                    leg.pool_id()
                )
            })?;
            let accounts = prepared.priority_contention_accounts.to_vec();
            validate_accounts(&accounts)?;
            Ok((accounts, ORCA_PRIORITY_PROVENANCE.to_owned()))
        }
        Venue::PumpSwap => match costs::pumpswap_contention_footprint(leg.pool_id()) {
            Ok(_) => Err(format!(
                "PumpSwap priority contention footprint unexpectedly resolved for pool {}",
                leg.pool_id()
            )),
            Err(error) => Err(error),
        },
        Venue::Meteora => Err(format!(
            "Meteora runtime priority contention footprint is not enabled: pool={}",
            leg.pool_id()
        )),
    }
}

async fn fetch_observation(
    rpc_client: &Client,
    rpc_url: &str,
    accounts: &[String],
    provenance: &str,
) -> Result<PriorityFeeObservation, String> {
    validate_accounts(accounts)?;

    if provenance.trim().is_empty() {
        return Err("localized priority-fee contention provenance must not be empty".to_owned());
    }

    let request = json!({
        "jsonrpc": "2.0",
        "id": ORCA_PRIORITY_FEE_RPC_REQUEST_ID,
        "method": "getRecentPrioritizationFees",
        "params": [accounts]
    });

    let started_at = Instant::now();

    println!(
        concat!(
            "rpc_request_start: label=Orca route localized priority ",
            "method=getRecentPrioritizationFees account_count={}"
        ),
        accounts.len()
    );

    let response = rpc_client
        .post(rpc_url)
        .json(&request)
        .send()
        .await
        .map_err(|error| {
            format!(
                "Orca route localized priority RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!(
            "Orca route localized priority RPC returned HTTP status {status} after {} ms",
            started_at.elapsed().as_millis()
        ));
    }

    let payload = response.json::<Value>().await.map_err(|error| {
        format!(
            "Orca route localized priority RPC returned invalid JSON after {} ms: {error}",
            started_at.elapsed().as_millis()
        )
    })?;

    parse_response(&payload, accounts, provenance)
}

fn validate_accounts(accounts: &[String]) -> Result<(), String> {
    if accounts.is_empty() {
        return Err("localized priority-fee contention footprint must not be empty".to_owned());
    }

    if accounts.len() > MAX_LOCALIZED_PRIORITY_ACCOUNTS {
        return Err(format!(
            concat!(
                "localized priority-fee contention footprint exceeds RPC maximum: ",
                "count={} max={}"
            ),
            accounts.len(),
            MAX_LOCALIZED_PRIORITY_ACCOUNTS
        ));
    }

    for account in accounts {
        Pubkey::from_str(account).map_err(|error| {
            format!(
                "localized priority-fee contention account is invalid: account={account} error={error}"
            )
        })?;
    }

    Ok(())
}

fn parse_response(
    payload: &Value,
    accounts: &[String],
    provenance: &str,
) -> Result<PriorityFeeObservation, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "getRecentPrioritizationFees returned an RPC error: {error}"
        ));
    }

    let jsonrpc = payload
        .get("jsonrpc")
        .and_then(Value::as_str)
        .ok_or_else(|| "priority-fee response missing jsonrpc version".to_owned())?;

    if jsonrpc != "2.0" {
        return Err(format!(
            "priority-fee response has unexpected jsonrpc version: {jsonrpc}"
        ));
    }

    let response_id = payload
        .get("id")
        .and_then(Value::as_u64)
        .ok_or_else(|| "priority-fee response missing numeric id".to_owned())?;

    if response_id != ORCA_PRIORITY_FEE_RPC_REQUEST_ID {
        return Err(format!(
            "priority-fee response id mismatch: expected={ORCA_PRIORITY_FEE_RPC_REQUEST_ID} actual={response_id}"
        ));
    }

    let result = payload
        .get("result")
        .and_then(Value::as_array)
        .ok_or_else(|| "priority-fee response missing result array".to_owned())?;

    let mut samples = Vec::with_capacity(result.len());

    for row in result {
        let slot = row
            .get("slot")
            .and_then(Value::as_u64)
            .ok_or_else(|| "priority-fee observation missing slot".to_owned())?;

        let micro_lamports_per_cu = row
            .get("prioritizationFee")
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!("priority-fee observation at slot {slot} missing prioritizationFee")
            })?;

        samples.push(PriorityFeeObservationSample {
            slot,
            micro_lamports_per_cu,
        });
    }

    Ok(PriorityFeeObservation {
        samples,
        scope_accounts: accounts.to_vec(),
        scope_provenance: provenance.to_owned(),
    })
}
