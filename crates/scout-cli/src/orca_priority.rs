use crate::costs::{
    self, select_priority_fee, PriorityFeeObservation, PriorityFeeObservationSample,
    PriorityObservationState,
};
use crate::orca_live::PreparedOrca;
use crate::quote::{TwoLegRouteQuote, VenueFeeComponents, VenueLegQuote};
use crate::raydium::RaydiumHydrationSnapshot;
use crate::route::RouteLeg;
use crate::rpc_transport;
use crate::runtime_quote::MeteoraRuntimeQuoteState;
use reqwest::Client;
use scout_cli::meteora_contention::meteora_quote_contention_footprint;
use scout_cli::meteora_m13::{MeteoraM13ExactInputQuote, MeteoraM13PreparedQuote};
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

pub struct RoutePriorityContexts<'a> {
    pub raydium: &'a BTreeMap<String, RaydiumHydrationSnapshot>,
    pub orca: &'a BTreeMap<String, PreparedOrca>,
    pub meteora: &'a BTreeMap<String, MeteoraRuntimeQuoteState>,
}

pub async fn observe_route(
    rpc_client: &Client,
    rpc_url: &str,
    leg_1: &RouteLeg,
    leg_2: &RouteLeg,
    raydium_quote_contexts: &BTreeMap<String, RaydiumHydrationSnapshot>,
    orca_prepared: &BTreeMap<String, PreparedOrca>,
    cache: &mut BTreeMap<Vec<String>, PriorityObservationState>,
) -> PriorityObservationState {
    let meteora = BTreeMap::new();
    let contexts = RoutePriorityContexts {
        raydium: raydium_quote_contexts,
        orca: orca_prepared,
        meteora: &meteora,
    };

    observe_route_with_contexts(rpc_client, rpc_url, leg_1, leg_2, None, &contexts, cache).await
}

pub async fn observe_route_with_contexts(
    rpc_client: &Client,
    rpc_url: &str,
    leg_1: &RouteLeg,
    leg_2: &RouteLeg,
    route_quote: Option<&TwoLegRouteQuote>,
    contexts: &RoutePriorityContexts<'_>,
    cache: &mut BTreeMap<Vec<String>, PriorityObservationState>,
) -> PriorityObservationState {
    let (accounts, provenance) = match route_scope(leg_1, leg_2, route_quote, contexts) {
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
                    println!("rung11c_priority_selection_unknown: no positive localized samples");
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
    route_quote: Option<&TwoLegRouteQuote>,
    contexts: &RoutePriorityContexts<'_>,
) -> Result<(Vec<String>, String), String> {
    let leg_1_quote = route_quote.map(|quote| &quote.leg_1);
    let leg_2_quote = route_quote.map(|quote| &quote.leg_2);

    let (leg_1_accounts, leg_1_provenance) = venue_scope(leg_1, leg_1_quote, contexts)?;
    let (leg_2_accounts, leg_2_provenance) = venue_scope(leg_2, leg_2_quote, contexts)?;

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
    leg_quote: Option<&VenueLegQuote>,
    contexts: &RoutePriorityContexts<'_>,
) -> Result<(Vec<String>, String), String> {
    match leg.venue() {
        Venue::RaydiumCpmm => {
            let snapshot = contexts.raydium.get(leg.pool_id()).ok_or_else(|| {
                format!(
                    "missing Raydium priority context for route pool {}",
                    leg.pool_id()
                )
            })?;
            let footprint = costs::raydium_contention_footprint(leg.pool_id(), snapshot)?;
            Ok((
                footprint.accounts().to_vec(),
                footprint.provenance().to_owned(),
            ))
        }
        Venue::Orca => {
            let prepared = contexts.orca.get(leg.pool_id()).ok_or_else(|| {
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
        Venue::Meteora => meteora_scope(leg, leg_quote, contexts),
    }
}

fn meteora_scope(
    leg: &RouteLeg,
    leg_quote: Option<&VenueLegQuote>,
    contexts: &RoutePriorityContexts<'_>,
) -> Result<(Vec<String>, String), String> {
    let leg_quote = leg_quote.ok_or_else(|| {
        format!(
            "missing quote-bound Meteora priority evidence for route pool {}",
            leg.pool_id()
        )
    })?;

    let runtime = contexts.meteora.get(leg.pool_id()).ok_or_else(|| {
        format!(
            "missing Meteora runtime priority context for route pool {}",
            leg.pool_id()
        )
    })?;

    if runtime.normalized.pool_id.as_str() != leg.pool_id() {
        return Err(format!(
            "Meteora runtime priority pool mismatch: route={} runtime={}",
            leg.pool_id(), runtime.normalized.pool_id
        ));
    }

    if runtime.normalized.source_slot != leg.source_slot() {
        return Err(format!(
            concat!(
                "Meteora runtime priority route-slot mismatch: pool={} ",
                "route_slot={} runtime_slot={}"
            ),
            leg.pool_id(),
            leg.source_slot(),
            runtime.normalized.source_slot
        ));
    }

    let prepared = MeteoraM13PreparedQuote::from_snapshot(&runtime.normalized, &runtime.snapshot)?;
    let exact_quote =
        prepared.quote_exact_input(leg.input_mint(), leg_quote.amount_in_requested_raw)?;

    validate_meteora_leg_quote(leg, leg_quote, &exact_quote)?;

    let footprint = meteora_quote_contention_footprint(&runtime.snapshot, &exact_quote)?;
    let accounts = footprint.account_pubkeys_base58();
    validate_accounts(&accounts)?;

    let provenance = format!(
        concat!(
            "{}; source_slot={} generation_id={} requested_input_raw={} ",
            "swap_for_y={}"
        ),
        footprint.provenance(),
        footprint.source.source_slot,
        footprint.source.generation_id,
        footprint.requested_input_raw,
        footprint.swap_for_y
    );

    Ok((accounts, provenance))
}

fn validate_meteora_leg_quote(
    leg: &RouteLeg,
    leg_quote: &VenueLegQuote,
    exact_quote: &MeteoraM13ExactInputQuote,
) -> Result<(), String> {
    if leg_quote.venue != Venue::Meteora {
        return Err(format!(
            "Meteora priority quote venue mismatch: expected=meteora actual={}",
            leg_quote.venue.label()
        ));
    }

    if leg_quote.pool_id.as_str() != leg.pool_id() {
        return Err(format!(
            "Meteora priority quote pool mismatch: route={} quote={}",
            leg.pool_id(), leg_quote.pool_id
        ));
    }

    if exact_quote.input_mint.as_str() != leg.input_mint()
        || exact_quote.output_mint.as_str() != leg.output_mint()
    {
        return Err(format!(
            concat!(
                "Meteora priority quote direction mismatch: route_input={} exact_input={} ",
                "route_output={} exact_output={}"
            ),
            leg.input_mint(),
            exact_quote.input_mint,
            leg.output_mint(),
            exact_quote.output_mint
        ));
    }

    if exact_quote.requested_input_raw != leg_quote.amount_in_requested_raw
        || exact_quote.consumed_input_raw != leg_quote.amount_in_consumed_raw
        || exact_quote.unspent_input_raw != leg_quote.amount_in_unspent_raw
        || exact_quote.amount_out_raw != leg_quote.amount_out_raw
        || exact_quote.source_slot != leg_quote.quote_source_slot
    {
        return Err(format!(
            "Meteora priority quote economics mismatch for pool {}",
            leg.pool_id()
        ));
    }

    let VenueFeeComponents::Meteora {
        trading_fee_raw,
        protocol_fee_raw,
        user_fee_raw,
        fee_on_input,
    } = &leg_quote.fees
    else {
        return Err(format!(
            "Meteora priority quote fee-component mismatch for pool {}",
            leg.pool_id()
        ));
    };

    if exact_quote.trading_fee_raw != *trading_fee_raw
        || exact_quote.protocol_fee_raw != *protocol_fee_raw
        || exact_quote.user_fee_raw != *user_fee_raw
        || exact_quote.fee_on_input != *fee_on_input
    {
        return Err(format!(
            "Meteora priority quote fee mismatch for pool {}",
            leg.pool_id()
        ));
    }

    Ok(())
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

    let payload = rpc_transport::post_json(
        rpc_client,
        rpc_url,
        &request,
        "Orca route localized priority",
    )
    .await
    .map_err(|error| format!("{error} after {} ms", started_at.elapsed().as_millis()))?;

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
                concat!(
                    "localized priority-fee contention account is invalid: ",
                    "account={} error={}"
                ),
                account, error
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
            concat!(
                "priority-fee response id mismatch: expected={} actual={}"
            ),
            ORCA_PRIORITY_FEE_RPC_REQUEST_ID,
            response_id
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

