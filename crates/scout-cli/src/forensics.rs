use crate::{pumpswap, raydium};
use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs::{self, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::time::{SystemTime, UNIX_EPOCH};

pub const R13_SCHEMA_VERSION: &str = "r13-forensics-v1";
pub const MAX_FORWARD_SLOTS: u64 = 32;

const SIGNATURE_PAGE_LIMIT: usize = 100;
const MAX_SIGNATURE_PAGES_PER_POOL: usize = 2;
const MAX_TRANSACTION_CANDIDATES_PER_ROUTE: usize = 16;
const MAX_RECORDS_PER_RUN: u64 = 512;
const COMPUTE_BUDGET_PROGRAM_ID: &str = "ComputeBudget111111111111111111111111111111";

const RAYDIUM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];
const RAYDIUM_SWAP_BASE_OUTPUT: [u8; 8] = [55, 217, 98, 86, 163, 74, 180, 173];
const PUMPSWAP_BUY: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
const PUMPSWAP_SELL: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R12RouteObservation {
    pub run_id: String,
    pub route_id: String,
    pub anchor_mint: String,
    pub intermediate_mint: String,
    pub leg_1: R12LegObservation,
    pub leg_2: R12LegObservation,
    pub candidate_ids: Vec<String>,
    pub statuses: BTreeSet<String>,
    pub earliest_candidate_found_at_unix_ms: u64,
    pub earliest_quote_complete_at_unix_ms: Option<u64>,
    pub earliest_economics_complete_at_unix_ms: Option<u64>,
    pub earliest_hypothetical_ready_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct R12LegObservation {
    pub venue: String,
    pub pool_id: String,
    pub input_mint: String,
    pub output_mint: String,
    pub source_slot: u64,
}

impl R12RouteObservation {
    pub fn start_slot(&self) -> u64 {
        self.leg_1.source_slot.max(self.leg_2.source_slot)
    }

    pub fn end_slot(&self) -> Result<u64, String> {
        self.start_slot()
            .checked_add(MAX_FORWARD_SLOTS)
            .ok_or_else(|| "R13 forward-slot window overflow".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RouteSearchStatus {
    Matched(TransactionMatch),
    NoMatchComplete,
    SearchIncomplete(String),
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TransactionMatch {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub fee_lamports: u64,
    pub compute_units_consumed: Option<u64>,
    pub requested_compute_unit_limit: Option<u64>,
    pub requested_compute_unit_price_micro_lamports: Option<u64>,
    pub jito_tip_lamports: Option<u64>,
    pub instruction_order: Vec<String>,
    pub pre_balances: Vec<u64>,
    pub post_balances: Vec<u64>,
    pub pre_token_balances: Value,
    pub post_token_balances: Value,
}

impl TransactionMatch {
    fn as_json(&self) -> Value {
        json!({
            "signature": self.signature,
            "slot": self.slot,
            "block_time": self.block_time,
            "fee_lamports": self.fee_lamports,
            "compute_units_consumed": self.compute_units_consumed,
            "compute_budget": {
                "requested_compute_unit_limit": self.requested_compute_unit_limit,
                "requested_compute_unit_price_micro_lamports": self.requested_compute_unit_price_micro_lamports,
            },
            "jito_tip_lamports": self.jito_tip_lamports,
            "jito_tip_status": if self.jito_tip_lamports.is_some() { "known" } else { "unknown" },
            "instruction_order": self.instruction_order,
            "pre_balances": self.pre_balances,
            "post_balances": self.post_balances,
            "pre_token_balances": self.pre_token_balances,
            "post_token_balances": self.post_token_balances,
        })
    }
}

#[derive(Debug, Clone)]
struct SignatureObservation {
    signature: String,
    slot: u64,
    succeeded: bool,
}

#[derive(Debug, Clone)]
struct SignatureScan {
    observations: Vec<SignatureObservation>,
    complete_through_start_slot: bool,
    reason: Option<String>,
}

#[derive(Debug, Clone)]
struct ResolvedInstruction {
    order: usize,
    program_id: String,
    account_keys: Vec<String>,
    data: Vec<u8>,
}

#[derive(Debug)]
pub struct R13RunResult {
    pub output_path: PathBuf,
    pub route_count: usize,
    pub matched_count: usize,
    pub no_match_complete_count: usize,
    pub search_incomplete_count: usize,
}

pub async fn analyze_r12_shadow(
    rpc_client: &Client,
    r12_path: &Path,
) -> Result<R13RunResult, String> {
    let routes = load_completed_r12_routes(r12_path)?;
    if routes.is_empty() {
        return Err("R13 received completed R12 evidence with no candidate routes".to_owned());
    }

    fs::create_dir_all("artifacts/r13-forensics")
        .map_err(|error| format!("could not create R13 artifact directory: {error}"))?;

    let now_ms = unix_time_ms_now()?;
    let run_id = r13_run_id(now_ms)?;
    let output_path = PathBuf::from(format!("artifacts/r13-forensics/{run_id}.jsonl"));
    let file = OpenOptions::new()
        .create_new(true)
        .write(true)
        .open(&output_path)
        .map_err(|error| format!("could not create R13 JSONL file: {error}"))?;
    let mut writer = R13Writer::new(BufWriter::new(file), run_id, output_path.clone());

    writer.write_event(
        "forensics_run_start",
        json!({
            "source_r12_path": r12_path.display().to_string(),
            "route_count": routes.len(),
            "max_forward_slots": MAX_FORWARD_SLOTS,
            "signature_page_limit": SIGNATURE_PAGE_LIMIT,
            "max_signature_pages_per_pool": MAX_SIGNATURE_PAGES_PER_POOL,
            "max_transaction_candidates_per_route": MAX_TRANSACTION_CANDIDATES_PER_ROUTE,
            "matching_policy": "exact-two-pool + exact-two-venue-program + recognized-swap-instruction + leg-order",
            "timing_policy": "slot-primary; block_time coarse nullable seconds; no invented millisecond chain ordering",
            "jito_tip_policy": "unknown unless independently provable from transaction evidence",
        }),
    )?;

    let mut matched_count = 0usize;
    let mut no_match_complete_count = 0usize;
    let mut search_incomplete_count = 0usize;

    for route in &routes {
        let status = search_route(rpc_client, route).await?;
        match &status {
            RouteSearchStatus::Matched(_) => matched_count += 1,
            RouteSearchStatus::NoMatchComplete => no_match_complete_count += 1,
            RouteSearchStatus::SearchIncomplete(_) => search_incomplete_count += 1,
        }

        writer.write_event(
            "route_search_result",
            route_forensics_payload(route, &status)?,
        )?;
    }

    writer.write_event(
        "forensics_run_end",
        json!({
            "route_count": routes.len(),
            "matched_count": matched_count,
            "no_match_complete_count": no_match_complete_count,
            "search_incomplete_count": search_incomplete_count,
        }),
    )?;
    writer.finish()?;
    validate_r13_jsonl(&output_path)?;

    Ok(R13RunResult {
        output_path,
        route_count: routes.len(),
        matched_count,
        no_match_complete_count,
        search_incomplete_count,
    })
}

fn route_forensics_payload(
    route: &R12RouteObservation,
    status: &RouteSearchStatus,
) -> Result<Value, String> {
    let end_slot = route.end_slot()?;
    let (status_label, reason, transaction) = match status {
        RouteSearchStatus::Matched(transaction) => (
            "atomic_route_match",
            Value::Null,
            transaction.as_json(),
        ),
        RouteSearchStatus::NoMatchComplete => (
            "no_atomic_match_complete",
            Value::Null,
            Value::Null,
        ),
        RouteSearchStatus::SearchIncomplete(reason) => (
            "search_incomplete",
            Value::String(reason.clone()),
            Value::Null,
        ),
    };

    Ok(json!({
        "source_r12_run_id": route.run_id,
        "route_id": route.route_id,
        "candidate_ids": route.candidate_ids,
        "r12_statuses": route.statuses,
        "anchor_mint": route.anchor_mint,
        "intermediate_mint": route.intermediate_mint,
        "leg_1": leg_json(&route.leg_1),
        "leg_2": leg_json(&route.leg_2),
        "r12_timing": {
            "earliest_candidate_found_at_unix_ms": route.earliest_candidate_found_at_unix_ms,
            "earliest_quote_complete_at_unix_ms": route.earliest_quote_complete_at_unix_ms,
            "earliest_economics_complete_at_unix_ms": route.earliest_economics_complete_at_unix_ms,
            "earliest_hypothetical_ready_at_unix_ms": route.earliest_hypothetical_ready_at_unix_ms,
        },
        "chain_search": {
            "start_slot": route.start_slot(),
            "end_slot": end_slot,
            "forward_slot_span": MAX_FORWARD_SLOTS,
            "status": status_label,
            "reason": reason,
        },
        "transaction": transaction,
        "captureability": captureability_json(route, status),
    }))
}

fn captureability_json(route: &R12RouteObservation, status: &RouteSearchStatus) -> Value {
    match status {
        RouteSearchStatus::Matched(transaction) => {
            let slot_delta = transaction.slot.checked_sub(route.start_slot());
            json!({
                "matched_chain_transaction": true,
                "slot_delta_from_latest_route_source": slot_delta,
                "scout_candidate_existed_before_or_at_matched_slot": slot_delta.is_some(),
                "scout_quote_completed_locally": route.earliest_quote_complete_at_unix_ms.is_some(),
                "scout_economics_completed_locally": route.earliest_economics_complete_at_unix_ms.is_some(),
                "scout_hypothetical_ready_locally": route.earliest_hypothetical_ready_at_unix_ms.is_some(),
                "profitably_executable_claim": false,
                "profitably_executable_reason": "R13 is read-only forensics; unresolved economics or structural matching cannot prove profitable execution",
            })
        }
        RouteSearchStatus::NoMatchComplete => json!({
            "matched_chain_transaction": false,
            "bounded_atomic_search_complete": true,
            "profitably_executable_claim": false,
            "interpretation": "no qualifying atomic two-leg transaction matched the exact two-pool route inside the bounded slot window; this does not prove no arbitrage occurred",
        }),
        RouteSearchStatus::SearchIncomplete(reason) => json!({
            "matched_chain_transaction": false,
            "bounded_atomic_search_complete": false,
            "profitably_executable_claim": false,
            "interpretation": "history search was incomplete and cannot support a no-match claim",
            "reason": reason,
        }),
    }
}

fn leg_json(leg: &R12LegObservation) -> Value {
    json!({
        "venue": leg.venue,
        "pool_id": leg.pool_id,
        "input_mint": leg.input_mint,
        "output_mint": leg.output_mint,
        "source_slot": leg.source_slot,
    })
}

async fn search_route(
    rpc_client: &Client,
    route: &R12RouteObservation,
) -> Result<RouteSearchStatus, String> {
    let start_slot = route.start_slot();
    let end_slot = route.end_slot()?;

    let confirmed_slot = fetch_confirmed_slot(rpc_client).await?;
    if confirmed_slot < end_slot {
        return Ok(RouteSearchStatus::SearchIncomplete(format!(
            "confirmed chain has not reached forensic end_slot: confirmed_slot={confirmed_slot} end_slot={end_slot}"
        )));
    }

    let left = fetch_pool_signatures(
        rpc_client,
        &route.leg_1.pool_id,
        start_slot,
        end_slot,
    )
    .await?;
    let right = fetch_pool_signatures(
        rpc_client,
        &route.leg_2.pool_id,
        start_slot,
        end_slot,
    )
    .await?;

    if !left.complete_through_start_slot || !right.complete_through_start_slot {
        let reason = format!(
            "signature history incomplete: leg1={} leg2={}",
            left.reason.as_deref().unwrap_or("complete"),
            right.reason.as_deref().unwrap_or("complete")
        );
        return Ok(RouteSearchStatus::SearchIncomplete(reason));
    }

    let left_signatures = left
        .observations
        .iter()
        .filter(|observation| {
            observation.succeeded
                && observation.slot >= start_slot
                && observation.slot <= end_slot
        })
        .map(|observation| observation.signature.as_str())
        .collect::<BTreeSet<_>>();

    let mut intersection = right
        .observations
        .iter()
        .filter(|observation| {
            observation.succeeded
                && observation.slot >= start_slot
                && observation.slot <= end_slot
                && left_signatures.contains(observation.signature.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();

    intersection.sort_by_key(|observation| observation.slot);

    if intersection.len() > MAX_TRANSACTION_CANDIDATES_PER_ROUTE {
        return Ok(RouteSearchStatus::SearchIncomplete(format!(
            "atomic candidate intersection exceeded bounded transaction cap: count={} cap={}",
            intersection.len(),
            MAX_TRANSACTION_CANDIDATES_PER_ROUTE
        )));
    }

    for observation in intersection {
        if let Some(transaction) =
            fetch_and_match_transaction(rpc_client, route, &observation.signature).await?
        {
            return Ok(RouteSearchStatus::Matched(transaction));
        }
    }

    Ok(RouteSearchStatus::NoMatchComplete)
}

async fn fetch_confirmed_slot(rpc_client: &Client) -> Result<u64, String> {
    let result = rpc_request(
        rpc_client,
        "getSlot",
        json!([{"commitment": "confirmed"}]),
    )
    .await?;
    result
        .as_u64()
        .ok_or_else(|| "R13 getSlot result was not u64".to_owned())
}

async fn fetch_pool_signatures(
    rpc_client: &Client,
    pool_id: &str,
    start_slot: u64,
    end_slot: u64,
) -> Result<SignatureScan, String> {
    let mut observations = Vec::new();
    let mut before: Option<String> = None;
    let mut complete = false;
    let mut reason = None;

    for page_index in 0..MAX_SIGNATURE_PAGES_PER_POOL {
        let mut config = json!({
            "commitment": "confirmed",
            "limit": SIGNATURE_PAGE_LIMIT
        });

        if let Some(before_signature) = before.as_deref() {
            config["before"] = Value::String(before_signature.to_owned());
        }

        let result = rpc_request(
            rpc_client,
            "getSignaturesForAddress",
            json!([pool_id, config]),
        )
        .await?;

        let entries = result.as_array().ok_or_else(|| {
            format!("R13 getSignaturesForAddress result for {pool_id} was not an array")
        })?;

        if entries.is_empty() {
            complete = true;
            break;
        }

        for entry in entries {
            let signature = required_str(entry, "signature")?.to_owned();
            let slot = required_u64(entry, "slot")?;
            let succeeded = entry.get("err").map_or(false, Value::is_null);
            observations.push(SignatureObservation {
                signature,
                slot,
                succeeded,
            });
        }

        let oldest_slot = entries
            .last()
            .and_then(|entry| entry.get("slot"))
            .and_then(Value::as_u64)
            .ok_or_else(|| {
                format!("R13 signature page for {pool_id} missing oldest slot")
            })?;

        if oldest_slot <= start_slot {
            complete = true;
            break;
        }

        if entries.len() < SIGNATURE_PAGE_LIMIT {
            complete = true;
            break;
        }

        before = entries
            .last()
            .and_then(|entry| entry.get("signature"))
            .and_then(Value::as_str)
            .map(ToOwned::to_owned);

        if before.is_none() {
            reason = Some(format!(
                "signature pagination for {pool_id} could not obtain before cursor"
            ));
            break;
        }

        if page_index + 1 == MAX_SIGNATURE_PAGES_PER_POOL {
            reason = Some(format!(
                "signature pagination for {pool_id} saturated before start_slot={start_slot}; end_slot={end_slot}"
            ));
        }
    }

    observations.sort_by_key(|observation| observation.slot);
    observations.dedup_by(|left, right| left.signature == right.signature);

    Ok(SignatureScan {
        observations,
        complete_through_start_slot: complete,
        reason,
    })
}

async fn fetch_and_match_transaction(
    rpc_client: &Client,
    route: &R12RouteObservation,
    signature: &str,
) -> Result<Option<TransactionMatch>, String> {
    let transaction = rpc_request(
        rpc_client,
        "getTransaction",
        json!([
            signature,
            {
                "commitment": "confirmed",
                "encoding": "json",
                "maxSupportedTransactionVersion": 0
            }
        ]),
    )
    .await?;

    if transaction.is_null() {
        return Ok(None);
    }

    let meta = required_object(&transaction, "meta")?;
    if meta.get("err").map_or(true, |value| !value.is_null()) {
        return Ok(None);
    }

    let slot = required_u64(&transaction, "slot")?;
    if slot < route.start_slot() || slot > route.end_slot()? {
        return Ok(None);
    }

    let account_keys = resolved_account_keys(&transaction)?;
    let instructions = resolved_instructions(&transaction, &account_keys)?;

    let first = find_route_swap_instruction(
        &instructions,
        &route.leg_1,
        route.leg_1.venue.as_str(),
    );
    let second = find_route_swap_instruction(
        &instructions,
        &route.leg_2,
        route.leg_2.venue.as_str(),
    );

    let (first, second) = match (first, second) {
        (Some(first), Some(second)) => (first, second),
        _ => return Ok(None),
    };

    if first.order >= second.order {
        return Ok(None);
    }

    let instruction_order = vec![
        format!(
            "{}:{}:{}",
            route.leg_1.venue, route.leg_1.pool_id, first.order
        ),
        format!(
            "{}:{}:{}",
            route.leg_2.venue, route.leg_2.pool_id, second.order
        ),
    ];

    let (requested_compute_unit_limit, requested_compute_unit_price_micro_lamports) =
        compute_budget_requests(&instructions)?;

    Ok(Some(TransactionMatch {
        signature: signature.to_owned(),
        slot,
        block_time: optional_i64(&transaction, "blockTime")?,
        fee_lamports: required_u64(meta, "fee")?,
        compute_units_consumed: optional_u64(meta, "computeUnitsConsumed")?,
        requested_compute_unit_limit,
        requested_compute_unit_price_micro_lamports,
        jito_tip_lamports: None,
        instruction_order,
        pre_balances: u64_array(meta, "preBalances")?,
        post_balances: u64_array(meta, "postBalances")?,
        pre_token_balances: meta
            .get("preTokenBalances")
            .cloned()
            .unwrap_or(Value::Null),
        post_token_balances: meta
            .get("postTokenBalances")
            .cloned()
            .unwrap_or(Value::Null),
    }))
}

fn find_route_swap_instruction<'a>(
    instructions: &'a [ResolvedInstruction],
    leg: &R12LegObservation,
    venue: &str,
) -> Option<&'a ResolvedInstruction> {
    let expected_program = match venue {
        "raydium_cpmm" => raydium::PROGRAM_ID,
        "pumpswap" => pumpswap::PROGRAM_ID,
        _ => return None,
    };

    instructions.iter().find(|instruction| {
        instruction.program_id == expected_program
            && instruction
                .account_keys
                .iter()
                .any(|account| account == &leg.pool_id)
            && recognized_swap(venue, &instruction.data)
    })
}

fn recognized_swap(venue: &str, data: &[u8]) -> bool {
    if data.len() < 8 {
        return false;
    }

    let discriminator = &data[..8];

    match venue {
        "raydium_cpmm" => {
            discriminator == RAYDIUM_SWAP_BASE_INPUT
                || discriminator == RAYDIUM_SWAP_BASE_OUTPUT
        }
        "pumpswap" => discriminator == PUMPSWAP_BUY || discriminator == PUMPSWAP_SELL,
        _ => false,
    }
}

fn resolved_account_keys(transaction: &Value) -> Result<Vec<String>, String> {
    let message = transaction
        .pointer("/transaction/message")
        .ok_or_else(|| "R13 transaction missing message".to_owned())?;

    let static_keys = message
        .get("accountKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 transaction message missing accountKeys".to_owned())?;

    let mut keys = Vec::with_capacity(static_keys.len());

    for key in static_keys {
        if let Some(text) = key.as_str() {
            keys.push(text.to_owned());
        } else if let Some(pubkey) = key.get("pubkey").and_then(Value::as_str) {
            keys.push(pubkey.to_owned());
        } else {
            return Err("R13 transaction account key had unsupported shape".to_owned());
        }
    }

    let meta = required_object(transaction, "meta")?;
    if let Some(loaded) = meta.get("loadedAddresses") {
        if !loaded.is_null() {
            for field in ["writable", "readonly"] {
                if let Some(array) = loaded.get(field).and_then(Value::as_array) {
                    for key in array {
                        keys.push(
                            key.as_str()
                                .ok_or_else(|| {
                                    format!(
                                        "R13 loadedAddresses.{field} contained non-string key"
                                    )
                                })?
                                .to_owned(),
                        );
                    }
                }
            }
        }
    }

    Ok(keys)
}

fn resolved_instructions(
    transaction: &Value,
    account_keys: &[String],
) -> Result<Vec<ResolvedInstruction>, String> {
    let message = transaction
        .pointer("/transaction/message")
        .ok_or_else(|| "R13 transaction missing message".to_owned())?;
    let outer = message
        .get("instructions")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 transaction message missing instructions".to_owned())?;

    let meta = required_object(transaction, "meta")?;
    let mut inner_by_outer = BTreeMap::<usize, Vec<Value>>::new();

    if let Some(inner_groups) = meta.get("innerInstructions") {
        if !inner_groups.is_null() {
            let groups = inner_groups
                .as_array()
                .ok_or_else(|| "R13 innerInstructions was not an array".to_owned())?;
            for group in groups {
                let outer_index = required_u64(group, "index")?;
                let outer_index = usize::try_from(outer_index)
                    .map_err(|_| "R13 inner instruction index overflow".to_owned())?;
                let entries = group
                    .get("instructions")
                    .and_then(Value::as_array)
                    .ok_or_else(|| {
                        "R13 inner instruction group missing instructions".to_owned()
                    })?;
                inner_by_outer
                    .entry(outer_index)
                    .or_default()
                    .extend(entries.iter().cloned());
            }
        }
    }

    let mut resolved = Vec::new();
    let mut order = 0usize;

    for (outer_index, instruction) in outer.iter().enumerate() {
        resolved.push(resolve_instruction(instruction, account_keys, order)?);
        order = order
            .checked_add(1)
            .ok_or_else(|| "R13 instruction order overflow".to_owned())?;

        if let Some(inner) = inner_by_outer.get(&outer_index) {
            for instruction in inner {
                resolved.push(resolve_instruction(instruction, account_keys, order)?);
                order = order
                    .checked_add(1)
                    .ok_or_else(|| "R13 instruction order overflow".to_owned())?;
            }
        }
    }

    Ok(resolved)
}

fn resolve_instruction(
    instruction: &Value,
    account_keys: &[String],
    order: usize,
) -> Result<ResolvedInstruction, String> {
    let program_id_index = required_u64(instruction, "programIdIndex")?;
    let program_id_index = usize::try_from(program_id_index)
        .map_err(|_| "R13 programIdIndex overflow".to_owned())?;
    let program_id = account_keys
        .get(program_id_index)
        .ok_or_else(|| "R13 programIdIndex outside resolved account keys".to_owned())?
        .clone();

    let account_indices = instruction
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 instruction missing accounts".to_owned())?;

    let mut resolved_accounts = Vec::with_capacity(account_indices.len());
    for index in account_indices {
        let index = index
            .as_u64()
            .ok_or_else(|| "R13 instruction account index was not u64".to_owned())?;
        let index = usize::try_from(index)
            .map_err(|_| "R13 instruction account index overflow".to_owned())?;
        resolved_accounts.push(
            account_keys
                .get(index)
                .ok_or_else(|| {
                    "R13 instruction account index outside resolved account keys".to_owned()
                })?
                .clone(),
        );
    }

    let encoded_data = required_str(instruction, "data")?;
    let data = bs58::decode(encoded_data)
        .into_vec()
        .map_err(|error| format!("R13 instruction data was invalid base58: {error}"))?;

    Ok(ResolvedInstruction {
        order,
        program_id,
        account_keys: resolved_accounts,
        data,
    })
}

fn compute_budget_requests(
    instructions: &[ResolvedInstruction],
) -> Result<(Option<u64>, Option<u64>), String> {
    let mut limit = None;
    let mut price = None;

    for instruction in instructions {
        if instruction.program_id != COMPUTE_BUDGET_PROGRAM_ID || instruction.data.is_empty() {
            continue;
        }

        match instruction.data[0] {
            2 if instruction.data.len() >= 5 => {
                let mut bytes = [0u8; 4];
                bytes.copy_from_slice(&instruction.data[1..5]);
                limit = Some(u64::from(u32::from_le_bytes(bytes)));
            }
            3 if instruction.data.len() >= 9 => {
                let mut bytes = [0u8; 8];
                bytes.copy_from_slice(&instruction.data[1..9]);
                price = Some(u64::from_le_bytes(bytes));
            }
            _ => {}
        }
    }

    Ok((limit, price))
}

async fn rpc_request(
    rpc_client: &Client,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let request = json!({
        "jsonrpc": "2.0",
        "id": 13,
        "method": method,
        "params": params,
    });

    let response = rpc_client
        .post("https://api.mainnet-beta.solana.com")
        .json(&request)
        .send()
        .await
        .map_err(|error| format!("R13 {method} RPC transport failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("R13 {method} RPC HTTP status {status}"));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("R13 {method} RPC returned invalid JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        return Err(format!("R13 {method} RPC error: {error}"));
    }

    payload
        .get("result")
        .cloned()
        .ok_or_else(|| format!("R13 {method} RPC response missing result"))
}

pub fn load_completed_r12_routes(path: &Path) -> Result<Vec<R12RouteObservation>, String> {
    let file =
        File::open(path).map_err(|error| format!("could not open R12 evidence: {error}"))?;
    let reader = BufReader::new(file);

    let mut expected_sequence = 1u64;
    let mut run_id: Option<String> = None;
    let mut saw_start = false;
    let mut saw_run_end = false;
    let mut routes = BTreeMap::<String, R12RouteObservation>::new();

    for line_result in reader.lines() {
        let line =
            line_result.map_err(|error| format!("could not read R12 evidence: {error}"))?;
        if line.trim().is_empty() {
            return Err("R13 rejected blank line in R12 JSONL".to_owned());
        }

        let record: Value = serde_json::from_str(&line)
            .map_err(|error| format!("R13 rejected malformed R12 JSONL: {error}"))?;

        if required_str(&record, "schema_version")? != "r12-shadow-v1" {
            return Err("R13 rejected unsupported R12 schema".to_owned());
        }

        let record_run_id = required_str(&record, "run_id")?;
        if let Some(expected) = run_id.as_deref() {
            if record_run_id != expected {
                return Err("R13 rejected R12 run_id change".to_owned());
            }
        } else {
            run_id = Some(record_run_id.to_owned());
        }

        let sequence = required_u64(&record, "record_sequence")?;
        if sequence != expected_sequence {
            return Err(format!(
                "R13 rejected non-contiguous R12 sequence: expected={expected_sequence} actual={sequence}"
            ));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "R13 R12 sequence overflow".to_owned())?;

        let event_type = required_str(&record, "event_type")?;
        match event_type {
            "run_start" => {
                if saw_start || sequence != 1 {
                    return Err("R13 rejected invalid R12 run_start".to_owned());
                }
                saw_start = true;
            }
            "candidate_evaluation" => {
                if !saw_start || saw_run_end {
                    return Err("R13 rejected R12 candidate outside run lifecycle".to_owned());
                }
                let payload = required_object(&record, "payload")?;
                aggregate_candidate(
                    routes.entry(required_str(
                        required_object(payload, "route")?,
                        "route_id",
                    )?
                    .to_owned()),
                    record_run_id,
                    payload,
                )?;
            }
            "route_rejection" => {
                if !saw_start || saw_run_end {
                    return Err("R13 rejected R12 route rejection outside run lifecycle".to_owned());
                }
            }
            "run_end" => {
                if saw_run_end {
                    return Err("R13 rejected R12 JSONL with multiple run_end records".to_owned());
                }
                saw_run_end = true;
            }
            other => {
                return Err(format!("R13 rejected unknown R12 event_type {other}"));
            }
        }
    }

    if !saw_start || !saw_run_end {
        return Err("R13 requires a completed R12 run with run_start and run_end".to_owned());
    }

    Ok(routes.into_values().collect())
}

fn aggregate_candidate(
    entry: std::collections::btree_map::Entry<'_, String, R12RouteObservation>,
    run_id: &str,
    payload: &Value,
) -> Result<(), String> {
    let candidate_id = required_str(payload, "candidate_id")?.to_owned();
    let status = required_str(payload, "status")?.to_owned();
    let route = required_object(payload, "route")?;
    let timing = required_object(payload, "timing")?;

    let candidate_found = required_u64(timing, "candidate_found_at_unix_ms")?;
    let quote_complete = optional_u64(timing, "quote_complete_at_unix_ms")?;
    let economics_complete = optional_u64(timing, "economics_complete_at_unix_ms")?;
    let hypothetical_ready = optional_u64(timing, "hypothetical_ready_at_unix_ms")?;

    match entry {
        std::collections::btree_map::Entry::Vacant(vacant) => {
            let mut statuses = BTreeSet::new();
            statuses.insert(status);
            vacant.insert(R12RouteObservation {
                run_id: run_id.to_owned(),
                route_id: required_str(route, "route_id")?.to_owned(),
                anchor_mint: required_str(route, "anchor_mint")?.to_owned(),
                intermediate_mint: required_str(route, "intermediate_mint")?.to_owned(),
                leg_1: parse_leg(required_object(route, "leg_1")?)?,
                leg_2: parse_leg(required_object(route, "leg_2")?)?,
                candidate_ids: vec![candidate_id],
                statuses,
                earliest_candidate_found_at_unix_ms: candidate_found,
                earliest_quote_complete_at_unix_ms: quote_complete,
                earliest_economics_complete_at_unix_ms: economics_complete,
                earliest_hypothetical_ready_at_unix_ms: hypothetical_ready,
            });
        }
        std::collections::btree_map::Entry::Occupied(mut occupied) => {
            let existing = occupied.get_mut();

            if existing.anchor_mint != required_str(route, "anchor_mint")?
                || existing.intermediate_mint != required_str(route, "intermediate_mint")?
                || existing.leg_1 != parse_leg(required_object(route, "leg_1")?)?
                || existing.leg_2 != parse_leg(required_object(route, "leg_2")?)?
            {
                return Err(format!(
                    "R13 route identity collision with inconsistent route payload: {}",
                    existing.route_id
                ));
            }

            existing.candidate_ids.push(candidate_id);
            existing.statuses.insert(status);
            existing.earliest_candidate_found_at_unix_ms = existing
                .earliest_candidate_found_at_unix_ms
                .min(candidate_found);
            existing.earliest_quote_complete_at_unix_ms =
                min_option(existing.earliest_quote_complete_at_unix_ms, quote_complete);
            existing.earliest_economics_complete_at_unix_ms = min_option(
                existing.earliest_economics_complete_at_unix_ms,
                economics_complete,
            );
            existing.earliest_hypothetical_ready_at_unix_ms = min_option(
                existing.earliest_hypothetical_ready_at_unix_ms,
                hypothetical_ready,
            );
        }
    }

    Ok(())
}

fn min_option(left: Option<u64>, right: Option<u64>) -> Option<u64> {
    match (left, right) {
        (Some(left), Some(right)) => Some(left.min(right)),
        (Some(left), None) => Some(left),
        (None, Some(right)) => Some(right),
        (None, None) => None,
    }
}

fn parse_leg(value: &Value) -> Result<R12LegObservation, String> {
    Ok(R12LegObservation {
        venue: required_str(value, "venue")?.to_owned(),
        pool_id: required_str(value, "pool_id")?.to_owned(),
        input_mint: required_str(value, "input_mint")?.to_owned(),
        output_mint: required_str(value, "output_mint")?.to_owned(),
        source_slot: required_u64(value, "source_slot")?,
    })
}

struct R13Writer {
    writer: BufWriter<File>,
    run_id: String,
    output_path: PathBuf,
    next_sequence: u64,
    records_written: u64,
    finished: bool,
}

impl R13Writer {
    fn new(writer: BufWriter<File>, run_id: String, output_path: PathBuf) -> Self {
        Self {
            writer,
            run_id,
            output_path,
            next_sequence: 1,
            records_written: 0,
            finished: false,
        }
    }

    fn write_event(&mut self, event_type: &str, payload: Value) -> Result<(), String> {
        if self.finished {
            return Err("R13 writer is already finished".to_owned());
        }
        if self.records_written >= MAX_RECORDS_PER_RUN {
            return Err("R13 recorder capacity exhausted".to_owned());
        }

        let record = json!({
            "schema_version": R13_SCHEMA_VERSION,
            "event_type": event_type,
            "run_id": self.run_id,
            "record_sequence": self.next_sequence,
            "observed_at_unix_ms": unix_time_ms_now()?,
            "github_actions": {
                "github_run_id": std::env::var("GITHUB_RUN_ID").ok(),
                "github_run_attempt": std::env::var("GITHUB_RUN_ATTEMPT").ok(),
                "github_sha": std::env::var("GITHUB_SHA").ok(),
                "github_workflow": std::env::var("GITHUB_WORKFLOW").ok(),
                "github_job": std::env::var("GITHUB_JOB").ok(),
                "github_ref": std::env::var("GITHUB_REF").ok(),
            },
            "payload": payload,
        });

        serde_json::to_writer(&mut self.writer, &record)
            .map_err(|error| format!("could not serialize R13 record: {error}"))?;
        self.writer
            .write_all(b"\n")
            .map_err(|error| format!("could not terminate R13 record: {error}"))?;
        self.writer
            .flush()
            .map_err(|error| format!("could not flush R13 record: {error}"))?;

        self.records_written = self
            .records_written
            .checked_add(1)
            .ok_or_else(|| "R13 record count overflow".to_owned())?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "R13 sequence overflow".to_owned())?;
        Ok(())
    }

    fn finish(&mut self) -> Result<(), String> {
        if self.finished {
            return Err("R13 writer already finished".to_owned());
        }
        self.writer
            .flush()
            .map_err(|error| format!("could not flush completed R13 JSONL: {error}"))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|error| format!("could not sync completed R13 JSONL: {error}"))?;
        self.finished = true;
        Ok(())
    }
}

pub fn validate_r13_jsonl(path: &Path) -> Result<(), String> {
    let file =
        File::open(path).map_err(|error| format!("could not open R13 JSONL replay: {error}"))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut expected_sequence = 1u64;
    let mut expected_run_id: Option<String> = None;
    let mut saw_start = false;
    let mut saw_end = false;
    let mut route_records = 0u64;

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| format!("could not read R13 JSONL replay: {error}"))?;
        if bytes == 0 {
            break;
        }
        if !line.ends_with('\n') {
            return Err("R13 JSONL contains non-newline-terminated record".to_owned());
        }

        let record: Value = serde_json::from_str(&line)
            .map_err(|error| format!("R13 JSONL malformed record: {error}"))?;

        if required_str(&record, "schema_version")? != R13_SCHEMA_VERSION {
            return Err("R13 JSONL schema mismatch".to_owned());
        }

        let run_id = required_str(&record, "run_id")?;
        if let Some(expected) = expected_run_id.as_deref() {
            if run_id != expected {
                return Err("R13 JSONL run_id changed".to_owned());
            }
        } else {
            expected_run_id = Some(run_id.to_owned());
        }

        let sequence = required_u64(&record, "record_sequence")?;
        if sequence != expected_sequence {
            return Err(format!(
                "R13 JSONL sequence mismatch expected={expected_sequence} actual={sequence}"
            ));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "R13 replay sequence overflow".to_owned())?;

        required_u64(&record, "observed_at_unix_ms")?;
        let payload = required_object(&record, "payload")?;

        match required_str(&record, "event_type")? {
            "forensics_run_start" => {
                if saw_start || saw_end || sequence != 1 {
                    return Err("R13 invalid run_start lifecycle".to_owned());
                }
                saw_start = true;
            }
            "route_search_result" => {
                if !saw_start || saw_end {
                    return Err("R13 route_search_result outside run lifecycle".to_owned());
                }
                validate_route_forensics(payload)?;
                route_records = route_records
                    .checked_add(1)
                    .ok_or_else(|| "R13 replay route count overflow".to_owned())?;
            }
            "forensics_run_end" => {
                if !saw_start || saw_end {
                    return Err("R13 invalid run_end lifecycle".to_owned());
                }
                saw_end = true;
            }
            other => return Err(format!("R13 JSONL unknown event_type {other}")),
        }
    }

    if !saw_start || !saw_end || route_records == 0 {
        return Err("R13 JSONL incomplete lifecycle".to_owned());
    }
    Ok(())
}

fn validate_route_forensics(payload: &Value) -> Result<(), String> {
    required_str(payload, "source_r12_run_id")?;
    required_str(payload, "route_id")?;
    required_str(payload, "anchor_mint")?;
    required_str(payload, "intermediate_mint")?;
    required_object(payload, "leg_1")?;
    required_object(payload, "leg_2")?;

    let chain_search = required_object(payload, "chain_search")?;
    let start_slot = required_u64(chain_search, "start_slot")?;
    let end_slot = required_u64(chain_search, "end_slot")?;
    if end_slot < start_slot {
        return Err("R13 replay end_slot precedes start_slot".to_owned());
    }

    match required_str(chain_search, "status")? {
        "atomic_route_match" => {
            if payload.get("transaction").map_or(true, Value::is_null) {
                return Err("R13 matched record missing transaction".to_owned());
            }
        }
        "no_atomic_match_complete" | "search_incomplete" => {
            if payload
                .get("transaction")
                .is_some_and(|value| !value.is_null())
            {
                return Err("R13 unmatched record unexpectedly contains transaction".to_owned());
            }
        }
        other => return Err(format!("R13 replay unknown search status {other}")),
    }

    Ok(())
}

fn r13_run_id(now_ms: u64) -> Result<String, String> {
    let github_run = std::env::var("GITHUB_RUN_ID").unwrap_or_else(|_| "local".to_owned());
    let github_attempt =
        std::env::var("GITHUB_RUN_ATTEMPT").unwrap_or_else(|_| "0".to_owned());
    Ok(format!(
        "r13-{github_run}-{github_attempt}-{now_ms}-{}",
        std::process::id()
    ))
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock is before Unix epoch: {error}"))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| "Unix millisecond timestamp does not fit u64".to_owned())
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, String> {
    let object = value
        .get(field)
        .ok_or_else(|| format!("missing required field {field}"))?;
    if !object.is_object() {
        return Err(format!("required field {field} was not an object"));
    }
    Ok(object)
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("missing or invalid string field {field}"))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("missing or invalid u64 field {field}"))
}

fn optional_u64(value: &Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("invalid optional u64 field {field}")),
    }
}

fn optional_i64(value: &Value, field: &str) -> Result<Option<i64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("invalid optional i64 field {field}")),
    }
}

fn u64_array(value: &Value, field: &str) -> Result<Vec<u64>, String> {
    let array = value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("missing or invalid u64 array {field}"))?;
    array
        .iter()
        .map(|entry| {
            entry
                .as_u64()
                .ok_or_else(|| format!("invalid u64 entry in {field}"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;

    fn temp_path(name: &str) -> Result<PathBuf, String> {
        let now = unix_time_ms_now()?;
        Ok(std::env::temp_dir().join(format!(
            "scout-{name}-{}-{now}.jsonl",
            std::process::id()
        )))
    }

    fn sample_candidate(sequence: u64, usd: u64) -> Value {
        json!({
            "schema_version": "r12-shadow-v1",
            "event_type": "candidate_evaluation",
            "run_id": "r12-test",
            "record_sequence": sequence,
            "observed_at_unix_ms": 1100 + usd,
            "payload": {
                "candidate_id": format!("route|usd={usd}"),
                "status": "economics_unresolved",
                "route": {
                    "route_id": "anchor=SOL|intermediate=T|leg1=raydium_cpmm:R|leg2=pumpswap:P",
                    "anchor_mint": "SOL",
                    "intermediate_mint": "T",
                    "leg_1": {
                        "venue": "raydium_cpmm",
                        "pool_id": "R",
                        "input_mint": "SOL",
                        "output_mint": "T",
                        "source_slot": 100
                    },
                    "leg_2": {
                        "venue": "pumpswap",
                        "pool_id": "P",
                        "input_mint": "T",
                        "output_mint": "SOL",
                        "source_slot": 101
                    }
                },
                "timing": {
                    "candidate_found_at_unix_ms": 1000 + usd,
                    "quote_complete_at_unix_ms": 1010 + usd,
                    "economics_complete_at_unix_ms": 1020 + usd,
                    "hypothetical_ready_at_unix_ms": null
                }
            }
        })
    }

    #[test]
    fn loads_and_aggregates_completed_r12_routes() -> Result<(), String> {
        let path = temp_path("r13-r12-load")?;
        let mut file = File::create(&path)
            .map_err(|error| format!("test could not create fixture: {error}"))?;

        let records = vec![
            json!({
                "schema_version": "r12-shadow-v1",
                "event_type": "run_start",
                "run_id": "r12-test",
                "record_sequence": 1,
                "observed_at_unix_ms": 1,
                "payload": {}
            }),
            sample_candidate(2, 1),
            sample_candidate(3, 5),
            json!({
                "schema_version": "r12-shadow-v1",
                "event_type": "run_end",
                "run_id": "r12-test",
                "record_sequence": 4,
                "observed_at_unix_ms": 4,
                "payload": {}
            }),
        ];

        for record in records {
            writeln!(
                file,
                "{}",
                serde_json::to_string(&record)
                    .map_err(|error| format!("test serialization failed: {error}"))?
            )
            .map_err(|error| format!("test fixture write failed: {error}"))?;
        }

        let routes = load_completed_r12_routes(&path)?;
        fs::remove_file(&path)
            .map_err(|error| format!("test fixture cleanup failed: {error}"))?;

        assert_eq!(routes.len(), 1);
        assert_eq!(routes[0].candidate_ids.len(), 2);
        assert_eq!(routes[0].start_slot(), 101);
        assert_eq!(routes[0].end_slot()?, 133);
        assert_eq!(routes[0].earliest_candidate_found_at_unix_ms, 1001);
        Ok(())
    }

    #[test]
    fn rejects_incomplete_r12_run() -> Result<(), String> {
        let path = temp_path("r13-r12-incomplete")?;
        let mut file = File::create(&path)
            .map_err(|error| format!("test could not create fixture: {error}"))?;

        let start = json!({
            "schema_version": "r12-shadow-v1",
            "event_type": "run_start",
            "run_id": "r12-test",
            "record_sequence": 1,
            "observed_at_unix_ms": 1,
            "payload": {}
        });
        writeln!(
            file,
            "{}",
            serde_json::to_string(&start)
                .map_err(|error| format!("test serialization failed: {error}"))?
        )
        .map_err(|error| format!("test fixture write failed: {error}"))?;

        let result = load_completed_r12_routes(&path);
        fs::remove_file(&path)
            .map_err(|error| format!("test fixture cleanup failed: {error}"))?;
        assert!(result.is_err());
        Ok(())
    }

    #[test]
    fn recognizes_locked_swap_discriminators() {
        assert!(recognized_swap(
            "raydium_cpmm",
            &RAYDIUM_SWAP_BASE_INPUT
        ));
        assert!(recognized_swap(
            "raydium_cpmm",
            &RAYDIUM_SWAP_BASE_OUTPUT
        ));
        assert!(recognized_swap("pumpswap", &PUMPSWAP_BUY));
        assert!(recognized_swap("pumpswap", &PUMPSWAP_SELL));
        assert!(!recognized_swap(
            "pumpswap",
            &RAYDIUM_SWAP_BASE_INPUT
        ));
    }

    #[test]
    fn parses_compute_budget_requests() -> Result<(), String> {
        let mut limit_data = vec![2];
        limit_data.extend_from_slice(&400_000u32.to_le_bytes());
        let mut price_data = vec![3];
        price_data.extend_from_slice(&12_345u64.to_le_bytes());

        let instructions = vec![
            ResolvedInstruction {
                order: 0,
                program_id: COMPUTE_BUDGET_PROGRAM_ID.to_owned(),
                account_keys: Vec::new(),
                data: limit_data,
            },
            ResolvedInstruction {
                order: 1,
                program_id: COMPUTE_BUDGET_PROGRAM_ID.to_owned(),
                account_keys: Vec::new(),
                data: price_data,
            },
        ];

        let (limit, price) = compute_budget_requests(&instructions)?;
        assert_eq!(limit, Some(400_000));
        assert_eq!(price, Some(12_345));
        Ok(())
    }
}
