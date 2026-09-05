use crate::forensics_rpc::{
    AddressHistory, HistoryAcquisition, HistoryRequest, TransactionAcquisition, TransactionEvidence,
};
use crate::{orca, pumpswap, raydium};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::env;
use std::fs::{self, create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Read, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

pub const R13_SCHEMA_VERSION: &str = "r13-forensics-v1";
pub const OUTPUT_DIRECTORY: &str = "artifacts/r13-forensics";
pub const MAX_FORWARD_SLOTS: u64 = 32;
pub const MAX_RECORDS_PER_RUN: u64 = 512;

const MATURITY_POLICY_ID: &str = "r13-confirmed-slot-maturity-v1";

const RAYDIUM_SWAP_BASE_INPUT: [u8; 8] = [143, 190, 90, 218, 196, 30, 51, 222];
const RAYDIUM_SWAP_BASE_OUTPUT: [u8; 8] = [55, 217, 98, 86, 163, 74, 180, 173];
const PUMPSWAP_BUY: [u8; 8] = [102, 6, 61, 18, 1, 218, 235, 234];
const PUMPSWAP_SELL: [u8; 8] = [51, 230, 133, 164, 1, 127, 131, 173];
const ORCA_SWAP: [u8; 8] = [248, 198, 158, 145, 225, 117, 135, 200];
const ORCA_SWAP_V2: [u8; 8] = [43, 4, 237, 11, 26, 201, 30, 98];
const ORCA_SWAP_DIRECTION_OFFSET: usize = 41;
const ORCA_SWAP_MIN_DATA_LEN: usize = 42;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CandidateEvidence {
    pub source_run_id: String,
    pub source_record_sequence: u64,
    pub candidate_id: String,
    pub source_status: String,
    pub usd_size: u64,
    pub candidate_found_at_unix_ms: u64,
    pub quote_complete_at_unix_ms: Option<u64>,
    pub economics_complete_at_unix_ms: Option<u64>,
    pub hypothetical_ready_at_unix_ms: Option<u64>,
    pub route: RouteEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteEvidence {
    pub route_id: String,
    pub anchor_mint: String,
    pub intermediate_mint: String,
    pub leg_1: LegEvidence,
    pub leg_2: LegEvidence,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LegEvidence {
    pub venue: String,
    pub pool_id: String,
    pub input_mint: String,
    pub output_mint: String,
    pub source_slot: u64,
}

impl RouteEvidence {
    pub fn start_slot(&self) -> u64 {
        self.leg_1.source_slot.max(self.leg_2.source_slot)
    }

    pub fn end_slot(&self) -> Result<u64, String> {
        self.start_slot()
            .checked_add(MAX_FORWARD_SLOTS)
            .ok_or_else(|| "R13 route end-slot overflow".to_owned())
    }

    fn history_request(&self, leg: &LegEvidence) -> Result<HistoryRequest, String> {
        Ok(HistoryRequest {
            address: leg.pool_id.clone(),
            start_slot: self.start_slot(),
            end_slot: self.end_slot()?,
        })
    }
}

#[derive(Debug, Clone)]
pub struct ForensicsPlan {
    pub source_path: PathBuf,
    pub source_run_id: String,
    pub source_github_actions: Value,
    pub candidates: Vec<CandidateEvidence>,
    pub routes: BTreeMap<String, RouteEvidence>,
    pub history_requests: Vec<HistoryRequest>,
}

impl ForensicsPlan {
    pub fn required_end_slot(&self) -> Result<u64, String> {
        let mut required_end_slot: Option<u64> = None;

        for route in self.routes.values() {
            let end_slot = route.end_slot()?;
            required_end_slot = Some(match required_end_slot {
                Some(current) => current.max(end_slot),
                None => end_slot,
            });
        }

        required_end_slot.ok_or_else(|| "R13 plan contains no route end slot".to_owned())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct EvidenceMaturity {
    pub required_end_slot: u64,
    pub initial_confirmed_tip: Option<u64>,
    pub final_confirmed_tip: Option<u64>,
    pub poll_attempts: u64,
    pub rpc_error_count: u64,
    pub wait_elapsed_ms: u64,
    pub maturity_reached: bool,
}

impl EvidenceMaturity {
    fn validate_for_plan(&self, plan: &ForensicsPlan) -> Result<(), String> {
        let expected_end_slot = plan.required_end_slot()?;

        if self.required_end_slot != expected_end_slot {
            return Err(format!(
                "R13 maturity end-slot mismatch: expected={expected_end_slot} actual={}",
                self.required_end_slot
            ));
        }

        if self.poll_attempts == 0 {
            return Err("R13 maturity evidence requires at least one poll attempt".to_owned());
        }

        if self.rpc_error_count > self.poll_attempts {
            return Err(format!(
                "R13 maturity RPC error count exceeds poll attempts: errors={} attempts={}",
                self.rpc_error_count, self.poll_attempts
            ));
        }

        if self.initial_confirmed_tip.is_none() && self.final_confirmed_tip.is_some() {
            return Err(
                "R13 maturity final confirmed tip cannot exist without an initial confirmed tip"
                    .to_owned(),
            );
        }

        if self.maturity_reached {
            let final_tip = self
                .final_confirmed_tip
                .ok_or_else(|| "R13 mature evidence requires a final confirmed tip".to_owned())?;

            if final_tip < self.required_end_slot {
                return Err(format!(
                    "R13 maturity marked reached below required end slot: final_tip={final_tip} required_end_slot={}",
                    self.required_end_slot
                ));
            }
        } else if self
            .final_confirmed_tip
            .is_some_and(|slot| slot >= self.required_end_slot)
        {
            return Err(format!(
                "R13 maturity marked unresolved despite confirmed tip reaching end slot: final_tip={} required_end_slot={}",
                self.final_confirmed_tip.unwrap_or_default(),
                self.required_end_slot
            ));
        }

        Ok(())
    }

    fn status(&self) -> &'static str {
        if self.maturity_reached {
            "mature"
        } else {
            "window_not_mature"
        }
    }

    fn reason(&self) -> Option<String> {
        if self.maturity_reached {
            None
        } else {
            Some(match self.final_confirmed_tip {
                Some(slot) => format!(
                    "confirmed chain tip did not reach required forensic end slot within bounded maturity wait: final_confirmed_tip={slot} required_end_slot={}",
                    self.required_end_slot
                ),
                None => format!(
                    "no confirmed chain tip was acquired within bounded maturity wait: required_end_slot={}",
                    self.required_end_slot
                ),
            })
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "policy_id": MATURITY_POLICY_ID,
            "required_end_slot": self.required_end_slot,
            "initial_confirmed_tip": self.initial_confirmed_tip,
            "final_confirmed_tip": self.final_confirmed_tip,
            "poll_attempts": self.poll_attempts,
            "rpc_error_count": self.rpc_error_count,
            "wait_elapsed_ms": self.wait_elapsed_ms,
            "maturity_reached": self.maturity_reached,
            "status": self.status(),
            "reason": self.reason(),
            "authority": "Solana getSlot commitment=confirmed; slot numbers are authoritative and elapsed milliseconds are operational telemetry only",
        })
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RouteIntersection {
    pub route_id: String,
    pub signatures: BTreeSet<String>,
    pub complete: bool,
    pub incomplete_reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct IntersectionPlan {
    pub routes: BTreeMap<String, RouteIntersection>,
    pub required_signatures: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct InstructionCoordinate {
    outer_index: usize,
    inner_index: Option<usize>,
    stack_height: Option<u64>,
}

#[derive(Debug, Clone)]
struct ResolvedInstruction {
    coordinate: InstructionCoordinate,
    program_id: String,
    account_keys: Vec<String>,
    data: Vec<u8>,
}

#[derive(Debug, Clone)]
struct MatchedLeg {
    coordinate: InstructionCoordinate,
    venue: String,
    pool_id: String,
    input_mint: String,
    output_mint: String,
    user_input_token_account: String,
    user_output_token_account: String,
    mints_verified_by_instruction: bool,
}

#[derive(Debug, Clone)]
pub struct TransactionMatch {
    pub signature: String,
    pub slot: u64,
    pub block_time: Option<i64>,
    pub fee_lamports: u64,
    pub compute_units_consumed: Option<u64>,
    pub leg_1: Value,
    pub leg_2: Value,
    pub amount_evidence: Value,
    pub outcome_resolved: bool,
}

impl TransactionMatch {
    fn as_json(&self) -> Value {
        json!({
            "signature": self.signature,
            "slot": self.slot,
            "block_time": self.block_time,
            "fee_lamports": self.fee_lamports,
            "compute_units_consumed": self.compute_units_consumed,
            "jito_tip": {
                "status": "unknown",
                "lamports": Value::Null,
                "reason": "R13 does not infer Jito tips from meta.fee",
            },
            "leg_1": self.leg_1,
            "leg_2": self.leg_2,
            "amount_evidence": self.amount_evidence,
            "outcome_resolved": self.outcome_resolved,
        })
    }
}

#[derive(Debug, Clone)]
pub struct RouteAnalysis {
    pub route_id: String,
    pub status: String,
    pub reason: Option<String>,
    pub matches: Vec<TransactionMatch>,
}

#[derive(Debug)]
pub struct R13RunResult {
    pub output_path: PathBuf,
    pub route_count: usize,
    pub candidate_count: usize,
    pub transaction_match_count: usize,
    pub search_incomplete_count: usize,
    pub no_atomic_match_complete_count: usize,
    pub atomic_route_match_count: usize,
    pub atomic_route_amounts_unresolved_count: usize,
    pub atomic_route_outcome_resolved_count: usize,
    pub window_not_mature_candidate_count: usize,
    pub maturity_reached: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct GithubActionsProvenance {
    run_id: Option<String>,
    run_attempt: Option<String>,
    sha: Option<String>,
    workflow: Option<String>,
    job: Option<String>,
    git_ref: Option<String>,
}

impl GithubActionsProvenance {
    fn from_environment() -> Self {
        Self {
            run_id: env_nonempty("GITHUB_RUN_ID"),
            run_attempt: env_nonempty("GITHUB_RUN_ATTEMPT"),
            sha: env_nonempty("GITHUB_SHA"),
            workflow: env_nonempty("GITHUB_WORKFLOW"),
            job: env_nonempty("GITHUB_JOB"),
            git_ref: env_nonempty("GITHUB_REF"),
        }
    }

    fn as_json(&self) -> Value {
        json!({
            "github_run_id": self.run_id,
            "github_run_attempt": self.run_attempt,
            "github_sha": self.sha,
            "github_workflow": self.workflow,
            "github_job": self.job,
            "github_ref": self.git_ref,
        })
    }
}

pub fn load_plan(path: &Path) -> Result<ForensicsPlan, String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read completed R12 evidence {}: {error}",
            path.display()
        )
    })?;
    if bytes.is_empty() {
        return Err("R13 source R12 evidence is empty".to_owned());
    }
    if !bytes.ends_with(b"\n") {
        return Err("R13 source R12 evidence is not newline terminated".to_owned());
    }

    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("R13 source R12 evidence is not UTF-8: {error}"))?;

    let mut expected_sequence = 1u64;
    let mut source_run_id: Option<String> = None;
    let mut source_github_actions: Option<Value> = None;
    let mut candidates = Vec::new();
    let mut routes = BTreeMap::new();
    let mut saw_start = false;
    let mut saw_end = false;

    for line in text.lines() {
        let record: Value = serde_json::from_str(line)
            .map_err(|error| format!("R13 could not parse R12 JSONL record: {error}"))?;
        if required_str(&record, "schema_version")? != "r12-shadow-v1" {
            return Err("R13 source schema is not r12-shadow-v1".to_owned());
        }
        let sequence = required_u64(&record, "record_sequence")?;
        if sequence != expected_sequence {
            return Err(format!(
                "R13 source R12 sequence mismatch: expected={expected_sequence} actual={sequence}"
            ));
        }
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "R13 source sequence overflow".to_owned())?;

        let run_id = required_str(&record, "run_id")?.to_owned();
        match source_run_id.as_deref() {
            None => source_run_id = Some(run_id.clone()),
            Some(expected) if expected == run_id => {}
            Some(_) => return Err("R13 source R12 run_id changed within file".to_owned()),
        }

        let github = record
            .get("github_actions")
            .cloned()
            .ok_or_else(|| "R13 source R12 record missing github_actions".to_owned())?;
        match source_github_actions.as_ref() {
            None => source_github_actions = Some(github.clone()),
            Some(expected) if expected == &github => {}
            Some(_) => {
                return Err("R13 source R12 GitHub provenance changed within file".to_owned())
            }
        }

        let event_type = required_str(&record, "event_type")?;
        match event_type {
            "run_start" => {
                if saw_start || sequence != 1 {
                    return Err("R13 source R12 run_start lifecycle invalid".to_owned());
                }
                saw_start = true;
            }
            "candidate_evaluation" => {
                if !saw_start || saw_end {
                    return Err("R13 source candidate outside completed lifecycle".to_owned());
                }
                let candidate = parse_candidate(&record)?;
                if routes
                    .insert(candidate.route.route_id.clone(), candidate.route.clone())
                    .is_some_and(|previous| previous != candidate.route)
                {
                    return Err(format!(
                        "R13 source route_id maps to conflicting route evidence: {}",
                        candidate.route.route_id
                    ));
                }
                candidates.push(candidate);
            }
            "route_rejection" => {
                if !saw_start || saw_end {
                    return Err("R13 source route rejection outside completed lifecycle".to_owned());
                }
            }
            "run_end" => {
                if !saw_start || saw_end {
                    return Err("R13 source R12 run_end lifecycle invalid".to_owned());
                }
                saw_end = true;
            }
            other => {
                return Err(format!(
                    "R13 source contains unsupported R12 event type {other}"
                ))
            }
        }
    }

    if !saw_start || !saw_end {
        return Err("R13 requires a completed R12 run with run_start and run_end".to_owned());
    }
    if candidates.is_empty() {
        return Err("R13 completed R12 source contains no candidate_evaluation records".to_owned());
    }

    let mut candidate_keys = BTreeSet::new();
    for candidate in &candidates {
        let key = (
            candidate.source_run_id.clone(),
            candidate.source_record_sequence,
            candidate.candidate_id.clone(),
        );
        if !candidate_keys.insert(key) {
            return Err("R13 source contains duplicate candidate provenance".to_owned());
        }
    }

    let mut history_requests = BTreeSet::new();
    for route in routes.values() {
        history_requests.insert(route.history_request(&route.leg_1)?);
        history_requests.insert(route.history_request(&route.leg_2)?);
    }

    Ok(ForensicsPlan {
        source_path: path.to_path_buf(),
        source_run_id: source_run_id.ok_or_else(|| "R13 source run_id unavailable".to_owned())?,
        source_github_actions: source_github_actions
            .ok_or_else(|| "R13 source GitHub provenance unavailable".to_owned())?,
        candidates,
        routes,
        history_requests: history_requests.into_iter().collect(),
    })
}

fn parse_candidate(record: &Value) -> Result<CandidateEvidence, String> {
    let payload = required_object(record, "payload")?;
    let route = parse_route(required_object(payload, "route")?)?;
    let timing = required_object(payload, "timing")?;

    Ok(CandidateEvidence {
        source_run_id: required_str(record, "run_id")?.to_owned(),
        source_record_sequence: required_u64(record, "record_sequence")?,
        candidate_id: required_str(payload, "candidate_id")?.to_owned(),
        source_status: required_str(payload, "status")?.to_owned(),
        usd_size: required_u64(payload, "usd_size")?,
        candidate_found_at_unix_ms: required_u64(timing, "candidate_found_at_unix_ms")?,
        quote_complete_at_unix_ms: optional_u64(timing, "quote_complete_at_unix_ms")?,
        economics_complete_at_unix_ms: optional_u64(timing, "economics_complete_at_unix_ms")?,
        hypothetical_ready_at_unix_ms: optional_u64(timing, "hypothetical_ready_at_unix_ms")?,
        route,
    })
}

fn parse_route(value: &Value) -> Result<RouteEvidence, String> {
    let route = RouteEvidence {
        route_id: required_str(value, "route_id")?.to_owned(),
        anchor_mint: required_str(value, "anchor_mint")?.to_owned(),
        intermediate_mint: required_str(value, "intermediate_mint")?.to_owned(),
        leg_1: parse_leg(required_object(value, "leg_1")?)?,
        leg_2: parse_leg(required_object(value, "leg_2")?)?,
    };

    if route.leg_1.input_mint != route.anchor_mint
        || route.leg_1.output_mint != route.intermediate_mint
        || route.leg_2.input_mint != route.intermediate_mint
        || route.leg_2.output_mint != route.anchor_mint
    {
        return Err(format!(
            "R13 source route mint continuity invalid for {}",
            route.route_id
        ));
    }

    if route.leg_1.pool_id == route.leg_2.pool_id {
        return Err(format!(
            "R13 source route unexpectedly reuses one pool for both legs: {}",
            route.route_id
        ));
    }

    Ok(route)
}

fn parse_leg(value: &Value) -> Result<LegEvidence, String> {
    Ok(LegEvidence {
        venue: required_str(value, "venue")?.to_owned(),
        pool_id: required_str(value, "pool_id")?.to_owned(),
        input_mint: required_str(value, "input_mint")?.to_owned(),
        output_mint: required_str(value, "output_mint")?.to_owned(),
        source_slot: required_u64(value, "source_slot")?,
    })
}

pub fn intersect_route_histories(
    plan: &ForensicsPlan,
    acquisition: &HistoryAcquisition,
) -> Result<IntersectionPlan, String> {
    let mut routes = BTreeMap::new();
    let mut required_signatures = BTreeSet::new();

    for route in plan.routes.values() {
        let left_request = route.history_request(&route.leg_1)?;
        let right_request = route.history_request(&route.leg_2)?;

        let left = acquisition.histories.get(&left_request);
        let right = acquisition.histories.get(&right_request);

        let (Some(left), Some(right)) = (left, right) else {
            routes.insert(
                route.route_id.clone(),
                RouteIntersection {
                    route_id: route.route_id.clone(),
                    signatures: BTreeSet::new(),
                    complete: false,
                    incomplete_reason: Some(
                        "one or both exact pool histories are unavailable".to_owned(),
                    ),
                },
            );
            continue;
        };

        if !left.complete_through_start_slot || !right.complete_through_start_slot {
            let reason = format!(
                "exact route history incomplete: leg1={} leg2={}",
                left.reason.as_deref().unwrap_or("complete=false"),
                right.reason.as_deref().unwrap_or("complete=false")
            );

            routes.insert(
                route.route_id.clone(),
                RouteIntersection {
                    route_id: route.route_id.clone(),
                    signatures: BTreeSet::new(),
                    complete: false,
                    incomplete_reason: Some(reason),
                },
            );
            continue;
        }

        let left_signatures = observed_signatures(left);
        let right_signatures = observed_signatures(right);

        let signatures = left_signatures
            .intersection(&right_signatures)
            .cloned()
            .collect::<BTreeSet<_>>();

        required_signatures.extend(signatures.iter().cloned());

        routes.insert(
            route.route_id.clone(),
            RouteIntersection {
                route_id: route.route_id.clone(),
                signatures,
                complete: true,
                incomplete_reason: None,
            },
        );
    }

    Ok(IntersectionPlan {
        routes,
        required_signatures,
    })
}

fn observed_signatures(history: &AddressHistory) -> BTreeSet<String> {
    history
        .observations
        .iter()
        .map(|observation| observation.signature.clone())
        .collect()
}

pub fn analyze_transactions(
    plan: &ForensicsPlan,
    intersections: &IntersectionPlan,
    transactions: &TransactionAcquisition,
) -> Result<BTreeMap<String, RouteAnalysis>, String> {
    let mut analyses = BTreeMap::new();

    for route in plan.routes.values() {
        let intersection = intersections
            .routes
            .get(&route.route_id)
            .ok_or_else(|| format!("R13 missing route intersection for {}", route.route_id))?;

        if !intersection.complete {
            analyses.insert(
                route.route_id.clone(),
                RouteAnalysis {
                    route_id: route.route_id.clone(),
                    status: "search_incomplete".to_owned(),
                    reason: intersection.incomplete_reason.clone(),
                    matches: Vec::new(),
                },
            );
            continue;
        }

        if intersection.signatures.is_empty() {
            analyses.insert(
                route.route_id.clone(),
                RouteAnalysis {
                    route_id: route.route_id.clone(),
                    status: "no_atomic_match_complete".to_owned(),
                    reason: None,
                    matches: Vec::new(),
                },
            );
            continue;
        }

        let mut missing = Vec::new();
        let mut matches = Vec::new();

        for signature in &intersection.signatures {
            let Some(evidence) = transactions.transactions.get(signature) else {
                missing.push(signature.clone());
                continue;
            };

            match match_transaction(route, evidence) {
                Ok(Some(matched)) => matches.push(matched),
                Ok(None) => {}
                Err(error) => {
                    missing.push(format!("{signature} (malformed/unresolved: {error})"));
                }
            }
        }

        if !missing.is_empty() {
            analyses.insert(
                route.route_id.clone(),
                RouteAnalysis {
                    route_id: route.route_id.clone(),
                    status: "search_incomplete".to_owned(),
                    reason: Some(format!(
                        "required intersecting transactions unavailable: {}",
                        missing.join(",")
                    )),
                    matches: Vec::new(),
                },
            );
            continue;
        }

        if matches.is_empty() {
            analyses.insert(
                route.route_id.clone(),
                RouteAnalysis {
                    route_id: route.route_id.clone(),
                    status: "no_atomic_match_complete".to_owned(),
                    reason: Some(
                        "intersecting signatures existed but none proved both exact supported route legs"
                            .to_owned(),
                    ),
                    matches,
                },
            );
            continue;
        }

        let outcome_resolved = matches.iter().any(|matched| matched.outcome_resolved);

        analyses.insert(
            route.route_id.clone(),
            RouteAnalysis {
                route_id: route.route_id.clone(),
                status: if outcome_resolved {
                    "atomic_route_outcome_resolved".to_owned()
                } else {
                    "atomic_route_amounts_unresolved".to_owned()
                },
                reason: if outcome_resolved {
                    None
                } else {
                    Some(
                        "atomic route structure proven; realized cost basis remains fail-closed"
                            .to_owned(),
                    )
                },
                matches,
            },
        );
    }

    Ok(analyses)
}

fn match_transaction(
    route: &RouteEvidence,
    evidence: &TransactionEvidence,
) -> Result<Option<TransactionMatch>, String> {
    let transaction = &evidence.value;
    let meta = required_object(transaction, "meta")?;

    if !meta.get("err").is_some_and(Value::is_null) {
        return Ok(None);
    }

    let account_keys = resolved_account_keys(transaction, meta)?;
    let instructions = resolved_instructions(transaction, meta, &account_keys)?;

    let leg_1 = instructions
        .iter()
        .filter_map(|instruction| match_leg(&route.leg_1, instruction).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    let leg_2 = instructions
        .iter()
        .filter_map(|instruction| match_leg(&route.leg_2, instruction).transpose())
        .collect::<Result<Vec<_>, _>>()?;

    let mut ordered_pair: Option<(MatchedLeg, MatchedLeg)> = None;
    let mut unresolved_mint_evidence = false;

    for first in &leg_1 {
        for second in &leg_2 {
            if !coordinate_precedes(&first.coordinate, &second.coordinate) {
                continue;
            }

            let first_proof = matched_leg_mints_proven(first, meta, &account_keys)?;
            let second_proof = matched_leg_mints_proven(second, meta, &account_keys)?;

            match (first_proof, second_proof) {
                (Some(true), Some(true)) => {
                    ordered_pair = Some((first.clone(), second.clone()));
                    break;
                }
                (None, _) | (_, None) => unresolved_mint_evidence = true,
                _ => {}
            }
        }

        if ordered_pair.is_some() {
            break;
        }
    }

    let Some((first, second)) = ordered_pair else {
        if unresolved_mint_evidence {
            return Err(
                "R13 Orca legacy swap mint identity could not be proven from transaction token-balance evidence"
                    .to_owned(),
            );
        }

        return Ok(None);
    };

    let slot = required_u64(transaction, "slot")?;
    let block_time = optional_i64(transaction, "blockTime")?;
    let fee_lamports = required_u64(meta, "fee")?;
    let compute_units_consumed = optional_u64(meta, "computeUnitsConsumed")?;

    let amount_evidence = reconstruct_amount_evidence(route, &first, &second, meta, &account_keys)?;

    Ok(Some(TransactionMatch {
        signature: evidence.signature.clone(),
        slot,
        block_time,
        fee_lamports,
        compute_units_consumed,
        leg_1: matched_leg_json(&first),
        leg_2: matched_leg_json(&second),
        amount_evidence,
        outcome_resolved: false,
    }))
}

fn match_leg(
    leg: &LegEvidence,
    instruction: &ResolvedInstruction,
) -> Result<Option<MatchedLeg>, String> {
    if instruction.program_id != venue_program_id(&leg.venue)? {
        return Ok(None);
    }

    let Some(discriminator) = instruction.data.get(..8) else {
        return Ok(None);
    };

    match leg.venue.as_str() {
        "raydium_cpmm" => {
            if discriminator != RAYDIUM_SWAP_BASE_INPUT && discriminator != RAYDIUM_SWAP_BASE_OUTPUT
            {
                return Ok(None);
            }

            if instruction.account_keys.len() < 13 {
                return Ok(None);
            }

            if instruction.account_keys[3] != leg.pool_id
                || instruction.account_keys[10] != leg.input_mint
                || instruction.account_keys[11] != leg.output_mint
            {
                return Ok(None);
            }

            Ok(Some(MatchedLeg {
                coordinate: instruction.coordinate.clone(),
                venue: leg.venue.clone(),
                pool_id: leg.pool_id.clone(),
                input_mint: leg.input_mint.clone(),
                output_mint: leg.output_mint.clone(),
                user_input_token_account: instruction.account_keys[4].clone(),
                user_output_token_account: instruction.account_keys[5].clone(),
                mints_verified_by_instruction: true,
            }))
        }
        "pumpswap" => {
            if instruction.account_keys.len() < 9 || instruction.account_keys[0] != leg.pool_id {
                return Ok(None);
            }

            let base_mint = &instruction.account_keys[3];
            let quote_mint = &instruction.account_keys[4];

            if discriminator == PUMPSWAP_BUY {
                if &leg.input_mint != quote_mint || &leg.output_mint != base_mint {
                    return Ok(None);
                }

                Ok(Some(MatchedLeg {
                    coordinate: instruction.coordinate.clone(),
                    venue: leg.venue.clone(),
                    pool_id: leg.pool_id.clone(),
                    input_mint: leg.input_mint.clone(),
                    output_mint: leg.output_mint.clone(),
                    user_input_token_account: instruction.account_keys[6].clone(),
                    user_output_token_account: instruction.account_keys[5].clone(),
                    mints_verified_by_instruction: true,
                }))
            } else if discriminator == PUMPSWAP_SELL {
                if &leg.input_mint != base_mint || &leg.output_mint != quote_mint {
                    return Ok(None);
                }

                Ok(Some(MatchedLeg {
                    coordinate: instruction.coordinate.clone(),
                    venue: leg.venue.clone(),
                    pool_id: leg.pool_id.clone(),
                    input_mint: leg.input_mint.clone(),
                    output_mint: leg.output_mint.clone(),
                    user_input_token_account: instruction.account_keys[5].clone(),
                    user_output_token_account: instruction.account_keys[6].clone(),
                    mints_verified_by_instruction: true,
                }))
            } else {
                Ok(None)
            }
        }
        "orca" => match_orca_leg(leg, instruction, discriminator),
        other => Err(format!("R13 unsupported venue {other}")),
    }
}

fn match_orca_leg(
    leg: &LegEvidence,
    instruction: &ResolvedInstruction,
    discriminator: &[u8],
) -> Result<Option<MatchedLeg>, String> {
    if discriminator == ORCA_SWAP_V2 {
        if instruction.account_keys.len() < 15 || instruction.account_keys[4] != leg.pool_id {
            return Ok(None);
        }

        let a_to_b = parse_orca_swap_direction(&instruction.data)?;
        let token_mint_a = &instruction.account_keys[5];
        let token_mint_b = &instruction.account_keys[6];

        let (input_mint, output_mint, user_input, user_output) = if a_to_b {
            (
                token_mint_a,
                token_mint_b,
                &instruction.account_keys[7],
                &instruction.account_keys[9],
            )
        } else {
            (
                token_mint_b,
                token_mint_a,
                &instruction.account_keys[9],
                &instruction.account_keys[7],
            )
        };

        if input_mint != &leg.input_mint || output_mint != &leg.output_mint {
            return Ok(None);
        }

        return Ok(Some(MatchedLeg {
            coordinate: instruction.coordinate.clone(),
            venue: leg.venue.clone(),
            pool_id: leg.pool_id.clone(),
            input_mint: leg.input_mint.clone(),
            output_mint: leg.output_mint.clone(),
            user_input_token_account: user_input.clone(),
            user_output_token_account: user_output.clone(),
            mints_verified_by_instruction: true,
        }));
    }

    if discriminator == ORCA_SWAP {
        if instruction.account_keys.len() < 11 || instruction.account_keys[2] != leg.pool_id {
            return Ok(None);
        }

        let a_to_b = parse_orca_swap_direction(&instruction.data)?;
        let (user_input, user_output) = if a_to_b {
            (&instruction.account_keys[3], &instruction.account_keys[5])
        } else {
            (&instruction.account_keys[5], &instruction.account_keys[3])
        };

        return Ok(Some(MatchedLeg {
            coordinate: instruction.coordinate.clone(),
            venue: leg.venue.clone(),
            pool_id: leg.pool_id.clone(),
            input_mint: leg.input_mint.clone(),
            output_mint: leg.output_mint.clone(),
            user_input_token_account: user_input.clone(),
            user_output_token_account: user_output.clone(),
            mints_verified_by_instruction: false,
        }));
    }

    Ok(None)
}

fn parse_orca_swap_direction(data: &[u8]) -> Result<bool, String> {
    if data.len() < ORCA_SWAP_MIN_DATA_LEN {
        return Err(format!(
            "R13 Orca swap instruction data truncated: expected_at_least={ORCA_SWAP_MIN_DATA_LEN} actual={}",
            data.len()
        ));
    }

    let amount_specified_is_input = data[40];
    let a_to_b = data[ORCA_SWAP_DIRECTION_OFFSET];

    if amount_specified_is_input > 1 {
        return Err(format!(
            "R13 Orca swap amount_specified_is_input bool invalid: {amount_specified_is_input}"
        ));
    }

    match a_to_b {
        0 => Ok(false),
        1 => Ok(true),
        other => Err(format!("R13 Orca swap a_to_b bool invalid: {other}")),
    }
}

fn matched_leg_mints_proven(
    leg: &MatchedLeg,
    meta: &Value,
    account_keys: &[String],
) -> Result<Option<bool>, String> {
    if leg.mints_verified_by_instruction {
        return Ok(Some(true));
    }

    let pre = token_balances_by_account(meta, "preTokenBalances", account_keys)?;
    let post = token_balances_by_account(meta, "postTokenBalances", account_keys)?;

    let input_mint = token_account_mint_evidence(&leg.user_input_token_account, &pre, &post)?;
    let output_mint = token_account_mint_evidence(&leg.user_output_token_account, &pre, &post)?;

    let (Some(input_mint), Some(output_mint)) = (input_mint, output_mint) else {
        return Ok(None);
    };

    Ok(Some(
        input_mint == leg.input_mint && output_mint == leg.output_mint,
    ))
}

fn token_account_mint_evidence(
    account: &str,
    pre: &BTreeMap<String, Value>,
    post: &BTreeMap<String, Value>,
) -> Result<Option<String>, String> {
    let before = pre
        .get(account)
        .and_then(|value| value.get("mint"))
        .and_then(Value::as_str);
    let after = post
        .get(account)
        .and_then(|value| value.get("mint"))
        .and_then(Value::as_str);

    match (before, after) {
        (Some(before), Some(after)) if before != after => Err(format!(
            "R13 token account mint changed across transaction evidence: account={account} pre={before} post={after}"
        )),
        (Some(mint), _) | (_, Some(mint)) => Ok(Some(mint.to_owned())),
        (None, None) => Ok(None),
    }
}

fn coordinate_precedes(left: &InstructionCoordinate, right: &InstructionCoordinate) -> bool {
    match left.outer_index.cmp(&right.outer_index) {
        std::cmp::Ordering::Less => true,
        std::cmp::Ordering::Greater => false,
        std::cmp::Ordering::Equal => match (left.inner_index, right.inner_index) {
            (None, Some(_)) => true,
            (Some(left_inner), Some(right_inner)) => left_inner < right_inner,
            _ => false,
        },
    }
}

fn matched_leg_json(leg: &MatchedLeg) -> Value {
    json!({
        "venue": leg.venue,
        "pool_id": leg.pool_id,
        "input_mint": leg.input_mint,
        "output_mint": leg.output_mint,
        "user_input_token_account": leg.user_input_token_account,
        "user_output_token_account": leg.user_output_token_account,
        "mint_identity_proof": if leg.mints_verified_by_instruction {
            "instruction_accounts"
        } else {
            "transaction_token_balances"
        },
        "instruction_coordinate": {
            "outer_index": leg.coordinate.outer_index,
            "inner_index": leg.coordinate.inner_index,
            "stack_height": leg.coordinate.stack_height,
        },
    })
}

fn reconstruct_amount_evidence(
    route: &RouteEvidence,
    first: &MatchedLeg,
    second: &MatchedLeg,
    meta: &Value,
    account_keys: &[String],
) -> Result<Value, String> {
    let watched = BTreeSet::from([
        first.user_input_token_account.clone(),
        first.user_output_token_account.clone(),
        second.user_input_token_account.clone(),
        second.user_output_token_account.clone(),
    ]);

    let pre = token_balances_by_account(meta, "preTokenBalances", account_keys)?;
    let post = token_balances_by_account(meta, "postTokenBalances", account_keys)?;

    let mut accounts = Vec::new();
    let mut complete = true;

    for account in watched {
        let before = pre.get(&account);
        let after = post.get(&account);

        if before.is_none() || after.is_none() {
            complete = false;
        }

        accounts.push(json!({
            "account": account,
            "pre": before,
            "post": after,
        }));
    }

    Ok(json!({
        "anchor_mint": route.anchor_mint,
        "intermediate_mint": route.intermediate_mint,
        "watched_user_token_accounts": accounts,
        "token_balance_snapshots_complete": complete,
        "realized_transaction_cost_basis_complete": false,
        "reason": if complete {
            "token balance snapshots available, but R13 does not fabricate a complete realized cost basis from token deltas plus meta.fee"
        } else {
            "one or more route user token accounts lack pre/post token balance evidence"
        },
    }))
}

fn token_balances_by_account(
    meta: &Value,
    field: &str,
    account_keys: &[String],
) -> Result<BTreeMap<String, Value>, String> {
    let mut result = BTreeMap::new();

    let values = match meta.get(field) {
        None | Some(Value::Null) => return Ok(result),
        Some(value) => value
            .as_array()
            .ok_or_else(|| format!("R13 {field} was not an array"))?,
    };

    for value in values {
        let index = required_u64(value, "accountIndex")? as usize;
        let account = account_keys
            .get(index)
            .ok_or_else(|| format!("R13 {field} accountIndex out of range"))?
            .clone();

        let mint = required_str(value, "mint")?;
        let token_amount = required_object(value, "uiTokenAmount")?;
        let amount = required_str(token_amount, "amount")?;
        let decimals = required_u64(token_amount, "decimals")?;

        result.insert(
            account,
            json!({
                "mint": mint,
                "amount_raw": amount,
                "decimals": decimals,
            }),
        );
    }

    Ok(result)
}

fn resolved_account_keys(transaction: &Value, meta: &Value) -> Result<Vec<String>, String> {
    let message = transaction
        .pointer("/transaction/message")
        .ok_or_else(|| "R13 transaction missing transaction.message".to_owned())?;

    let static_keys = message
        .get("accountKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 raw transaction missing message.accountKeys".to_owned())?;

    let mut keys = Vec::new();

    for key in static_keys {
        keys.push(
            key.as_str()
                .ok_or_else(|| "R13 raw account key was not a string".to_owned())?
                .to_owned(),
        );
    }

    if let Some(loaded) = meta.get("loadedAddresses") {
        if !loaded.is_null() {
            append_string_array(&mut keys, loaded, "writable")?;
            append_string_array(&mut keys, loaded, "readonly")?;
        }
    }

    Ok(keys)
}

fn append_string_array(
    destination: &mut Vec<String>,
    object: &Value,
    field: &str,
) -> Result<(), String> {
    let values = object
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("R13 loadedAddresses.{field} missing or invalid"))?;

    for value in values {
        destination.push(
            value
                .as_str()
                .ok_or_else(|| format!("R13 loadedAddresses.{field} contained non-string"))?
                .to_owned(),
        );
    }

    Ok(())
}

fn resolved_instructions(
    transaction: &Value,
    meta: &Value,
    account_keys: &[String],
) -> Result<Vec<ResolvedInstruction>, String> {
    let outer = transaction
        .pointer("/transaction/message/instructions")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 raw transaction missing compiled instructions".to_owned())?;

    let mut inner_by_outer: BTreeMap<usize, Vec<&Value>> = BTreeMap::new();

    if let Some(groups) = meta.get("innerInstructions") {
        if !groups.is_null() {
            let groups = groups
                .as_array()
                .ok_or_else(|| "R13 innerInstructions was not an array".to_owned())?;

            for group in groups {
                let outer_index = required_u64(group, "index")? as usize;
                let instructions = group
                    .get("instructions")
                    .and_then(Value::as_array)
                    .ok_or_else(|| "R13 inner instruction group missing instructions".to_owned())?;

                inner_by_outer.insert(outer_index, instructions.iter().collect());
            }
        }
    }

    let mut resolved = Vec::new();

    for (outer_index, instruction) in outer.iter().enumerate() {
        resolved.push(resolve_instruction(
            instruction,
            account_keys,
            InstructionCoordinate {
                outer_index,
                inner_index: None,
                stack_height: optional_u64(instruction, "stackHeight")?,
            },
        )?);

        if let Some(inner) = inner_by_outer.get(&outer_index) {
            for (inner_index, instruction) in inner.iter().enumerate() {
                resolved.push(resolve_instruction(
                    instruction,
                    account_keys,
                    InstructionCoordinate {
                        outer_index,
                        inner_index: Some(inner_index),
                        stack_height: optional_u64(instruction, "stackHeight")?,
                    },
                )?);
            }
        }
    }

    Ok(resolved)
}

fn resolve_instruction(
    instruction: &Value,
    account_keys: &[String],
    coordinate: InstructionCoordinate,
) -> Result<ResolvedInstruction, String> {
    let program_index = required_u64(instruction, "programIdIndex")? as usize;

    let program_id = account_keys
        .get(program_index)
        .ok_or_else(|| "R13 programIdIndex out of range".to_owned())?
        .clone();

    let indexes = instruction
        .get("accounts")
        .and_then(Value::as_array)
        .ok_or_else(|| "R13 compiled instruction missing accounts".to_owned())?;

    let mut instruction_accounts = Vec::new();

    for index in indexes {
        let index = index
            .as_u64()
            .ok_or_else(|| "R13 instruction account index was not u64".to_owned())?
            as usize;

        instruction_accounts.push(
            account_keys
                .get(index)
                .ok_or_else(|| "R13 instruction account index out of range".to_owned())?
                .clone(),
        );
    }

    let data = bs58::decode(required_str(instruction, "data")?)
        .into_vec()
        .map_err(|error| format!("R13 could not decode instruction data: {error}"))?;

    Ok(ResolvedInstruction {
        coordinate,
        program_id,
        account_keys: instruction_accounts,
        data,
    })
}

fn venue_program_id(venue: &str) -> Result<&'static str, String> {
    match venue {
        "raydium_cpmm" => Ok(raydium::RAYDIUM_CPMM_PROGRAM_ID),
        "pumpswap" => Ok(pumpswap::PUMPSWAP_PROGRAM_ID),
        "orca" => Ok(orca::ORCA_WHIRLPOOL_PROGRAM_ID),
        other => Err(format!("R13 unsupported route venue {other}")),
    }
}

pub fn write_forensics_artifact(
    plan: &ForensicsPlan,
    maturity: &EvidenceMaturity,
    intersections: Option<&IntersectionPlan>,
    analyses: Option<&BTreeMap<String, RouteAnalysis>>,
) -> Result<R13RunResult, String> {
    write_forensics_artifact_in_directory(
        Path::new(OUTPUT_DIRECTORY),
        plan,
        maturity,
        intersections,
        analyses,
    )
}

fn write_forensics_artifact_in_directory(
    output_directory: &Path,
    plan: &ForensicsPlan,
    maturity: &EvidenceMaturity,
    intersections: Option<&IntersectionPlan>,
    analyses: Option<&BTreeMap<String, RouteAnalysis>>,
) -> Result<R13RunResult, String> {
    maturity.validate_for_plan(plan)?;

    match (maturity.maturity_reached, intersections, analyses) {
        (true, Some(_), Some(_)) | (false, None, None) => {}
        (true, _, _) => {
            return Err(
                "R13 mature evidence requires route intersections and transaction analyses"
                    .to_owned(),
            )
        }
        (false, _, _) => {
            return Err(
                "R13 immature evidence must not contain route-search intersections or analyses"
                    .to_owned(),
            )
        }
    }

    create_dir_all(output_directory)
        .map_err(|error| format!("could not create R13 output directory: {error}"))?;

    let now = unix_time_ms_now()?;
    let run_id = build_run_id(now);
    let output_path = output_directory.join(format!("{run_id}.jsonl"));

    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&output_path)
        .map_err(|error| format!("could not create immutable R13 artifact: {error}"))?;

    let github = GithubActionsProvenance::from_environment();

    let mut writer = R13Writer {
        run_id,
        writer: BufWriter::new(file),
        next_sequence: 1,
        records_written: 0,
        github,
    };

    writer.write_event(
        "forensics_run_start",
        now,
        json!({
            "source_r12": {
                "path": plan.source_path.display().to_string(),
                "run_id": plan.source_run_id,
                "github_actions": plan.source_github_actions,
                "candidate_count": plan.candidates.len(),
                "route_count": plan.routes.len(),
            },
            "search_contract": {
                "start_slot_policy": "max(recorded_leg_source_slots)",
                "forward_slots": MAX_FORWARD_SLOTS,
                "atomic_scope": "exact two-pool intersection followed by exact supported venue instruction proof",
                "maturity_policy_id": MATURITY_POLICY_ID,
            },
        }),
    )?;

    writer.write_event("maturity_result", unix_time_ms_now()?, maturity.as_json())?;

    if !maturity.maturity_reached {
        for candidate in &plan.candidates {
            write_candidate_annotation(&mut writer, candidate, "window_not_mature", &[])?;
        }

        writer.write_event(
            "forensics_run_end",
            unix_time_ms_now()?,
            json!({
                "route_count": plan.routes.len(),
                "route_search_result_count": 0,
                "candidate_annotation_count": plan.candidates.len(),
                "transaction_match_count": 0,
                "search_incomplete_count": 0,
                "no_atomic_match_complete_count": 0,
                "atomic_route_match_count": 0,
                "atomic_route_amounts_unresolved_count": 0,
                "atomic_route_outcome_resolved_count": 0,
                "window_not_mature_candidate_count": plan.candidates.len(),
                "maturity_reached": false,
            }),
        )?;

        writer.finish()?;
        validate_r13_jsonl(&output_path, plan.candidates.len(), plan.routes.len())?;

        return Ok(R13RunResult {
            output_path,
            route_count: plan.routes.len(),
            candidate_count: plan.candidates.len(),
            transaction_match_count: 0,
            search_incomplete_count: 0,
            no_atomic_match_complete_count: 0,
            atomic_route_match_count: 0,
            atomic_route_amounts_unresolved_count: 0,
            atomic_route_outcome_resolved_count: 0,
            window_not_mature_candidate_count: plan.candidates.len(),
            maturity_reached: false,
        });
    }

    let intersections = intersections.ok_or_else(|| {
        "R13 mature evidence missing route intersections after contract validation".to_owned()
    })?;
    let analyses = analyses.ok_or_else(|| {
        "R13 mature evidence missing route analyses after contract validation".to_owned()
    })?;

    let mut transaction_match_count = 0usize;
    let mut search_incomplete_count = 0usize;
    let mut no_atomic_match_complete_count = 0usize;
    let mut atomic_route_match_count = 0usize;
    let mut atomic_route_amounts_unresolved_count = 0usize;
    let mut atomic_route_outcome_resolved_count = 0usize;

    for route in plan.routes.values() {
        let intersection = intersections
            .routes
            .get(&route.route_id)
            .ok_or_else(|| format!("R13 missing intersection for {}", route.route_id))?;

        let analysis = analyses
            .get(&route.route_id)
            .ok_or_else(|| format!("R13 missing analysis for {}", route.route_id))?;

        if analysis.route_id != route.route_id {
            return Err(format!(
                "R13 route analysis identity mismatch: expected={} actual={}",
                route.route_id, analysis.route_id
            ));
        }

        match analysis.status.as_str() {
            "search_incomplete" => search_incomplete_count += 1,
            "no_atomic_match_complete" => no_atomic_match_complete_count += 1,
            "atomic_route_match" => atomic_route_match_count += 1,
            "atomic_route_amounts_unresolved" => atomic_route_amounts_unresolved_count += 1,
            "atomic_route_outcome_resolved" => atomic_route_outcome_resolved_count += 1,
            other => return Err(format!("R13 unsupported analysis status {other}")),
        }

        let intersecting_signatures = intersection.signatures.iter().cloned().collect::<Vec<_>>();

        writer.write_event(
            "route_search_result",
            unix_time_ms_now()?,
            json!({
                "route": route_json(route),
                "window": {
                    "start_slot": route.start_slot(),
                    "end_slot": route.end_slot()?,
                },
                "history_complete": intersection.complete,
                "intersecting_signature_count": intersecting_signatures.len(),
                "intersecting_signatures": intersecting_signatures,
                "status": analysis.status,
                "reason": analysis.reason,
            }),
        )?;

        for matched in &analysis.matches {
            transaction_match_count += 1;

            writer.write_event(
                "transaction_match",
                unix_time_ms_now()?,
                json!({
                    "route_id": route.route_id,
                    "status": if matched.outcome_resolved {
                        "atomic_route_outcome_resolved"
                    } else {
                        "atomic_route_amounts_unresolved"
                    },
                    "transaction": matched.as_json(),
                }),
            )?;
        }
    }

    for candidate in &plan.candidates {
        let analysis = analyses.get(&candidate.route.route_id).ok_or_else(|| {
            format!(
                "R13 missing candidate route analysis {}",
                candidate.route.route_id
            )
        })?;

        let matched_signatures = analysis
            .matches
            .iter()
            .map(|item| item.signature.clone())
            .collect::<Vec<_>>();

        write_candidate_annotation(
            &mut writer,
            candidate,
            &analysis.status,
            &matched_signatures,
        )?;
    }

    writer.write_event(
        "forensics_run_end",
        unix_time_ms_now()?,
        json!({
            "route_count": plan.routes.len(),
            "route_search_result_count": plan.routes.len(),
            "candidate_annotation_count": plan.candidates.len(),
            "transaction_match_count": transaction_match_count,
            "search_incomplete_count": search_incomplete_count,
            "no_atomic_match_complete_count": no_atomic_match_complete_count,
            "atomic_route_match_count": atomic_route_match_count,
            "atomic_route_amounts_unresolved_count": atomic_route_amounts_unresolved_count,
            "atomic_route_outcome_resolved_count": atomic_route_outcome_resolved_count,
            "window_not_mature_candidate_count": 0,
            "maturity_reached": true,
        }),
    )?;

    writer.finish()?;
    validate_r13_jsonl(&output_path, plan.candidates.len(), plan.routes.len())?;

    Ok(R13RunResult {
        output_path,
        route_count: plan.routes.len(),
        candidate_count: plan.candidates.len(),
        transaction_match_count,
        search_incomplete_count,
        no_atomic_match_complete_count,
        atomic_route_match_count,
        atomic_route_amounts_unresolved_count,
        atomic_route_outcome_resolved_count,
        window_not_mature_candidate_count: 0,
        maturity_reached: true,
    })
}

fn write_candidate_annotation(
    writer: &mut R13Writer,
    candidate: &CandidateEvidence,
    captureability_status: &str,
    matched_signatures: &[String],
) -> Result<(), String> {
    validate_candidate_status(captureability_status)?;

    writer.write_event(
        "candidate_annotation",
        unix_time_ms_now()?,
        json!({
            "source_r12": {
                "run_id": candidate.source_run_id,
                "record_sequence": candidate.source_record_sequence,
                "candidate_id": candidate.candidate_id,
                "status": candidate.source_status,
                "usd_size": candidate.usd_size,
            },
            "route_id": candidate.route.route_id,
            "captureability_status": captureability_status,
            "matched_signatures": matched_signatures,
            "timing": {
                "candidate_found_at_unix_ms": candidate.candidate_found_at_unix_ms,
                "quote_complete_at_unix_ms": candidate.quote_complete_at_unix_ms,
                "economics_complete_at_unix_ms": candidate.economics_complete_at_unix_ms,
                "hypothetical_ready_at_unix_ms": candidate.hypothetical_ready_at_unix_ms,
                "timing_comparison_policy": "slot_delta_primary; R12 local milliseconds and Solana blockTime are not treated as synchronized millisecond clocks",
            },
            "profitability_claim": "none",
        }),
    )
}

struct R13Writer {
    run_id: String,
    writer: BufWriter<File>,
    next_sequence: u64,
    records_written: u64,
    github: GithubActionsProvenance,
}

impl R13Writer {
    fn write_event(
        &mut self,
        event_type: &str,
        observed_at_unix_ms: u64,
        payload: Value,
    ) -> Result<(), String> {
        if self.records_written >= MAX_RECORDS_PER_RUN {
            return Err("R13 recorder capacity exhausted".to_owned());
        }

        if event_type != "forensics_run_end" && self.records_written >= MAX_RECORDS_PER_RUN - 1 {
            return Err("R13 recorder capacity reserved for forensics_run_end".to_owned());
        }

        let record = json!({
            "schema_version": R13_SCHEMA_VERSION,
            "event_type": event_type,
            "run_id": self.run_id,
            "record_sequence": self.next_sequence,
            "observed_at_unix_ms": observed_at_unix_ms,
            "github_actions": self.github.as_json(),
            "payload": payload,
        });

        let mut bytes = serde_json::to_vec(&record)
            .map_err(|error| format!("could not serialize R13 record: {error}"))?;

        bytes.push(b'\n');

        self.writer
            .write_all(&bytes)
            .map_err(|error| format!("could not append R13 record: {error}"))?;

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
            .ok_or_else(|| "R13 record sequence overflow".to_owned())?;

        Ok(())
    }

    fn finish(mut self) -> Result<(), String> {
        self.writer
            .flush()
            .map_err(|error| format!("could not flush R13 artifact: {error}"))?;

        self.writer
            .get_ref()
            .sync_all()
            .map_err(|error| format!("could not sync R13 artifact: {error}"))
    }
}

pub fn validate_r13_jsonl(
    path: &Path,
    expected_candidate_count: usize,
    expected_route_count: usize,
) -> Result<(), String> {
    let mut bytes = Vec::new();

    File::open(path)
        .map_err(|error| format!("could not open R13 artifact for replay: {error}"))?
        .read_to_end(&mut bytes)
        .map_err(|error| format!("could not read R13 artifact for replay: {error}"))?;

    if bytes.is_empty() || !bytes.ends_with(b"\n") {
        return Err("R13 replay requires non-empty newline-terminated JSONL".to_owned());
    }

    let mut expected_sequence = 1u64;
    let mut run_id: Option<String> = None;
    let mut github: Option<Value> = None;
    let mut saw_start = false;
    let mut saw_maturity = false;
    let mut saw_end = false;
    let mut maturity_reached: Option<bool> = None;
    let mut route_results = 0usize;
    let mut candidate_annotations = 0usize;
    let mut transaction_matches = 0usize;
    let mut search_incomplete_count = 0usize;
    let mut no_atomic_match_complete_count = 0usize;
    let mut atomic_route_match_count = 0usize;
    let mut atomic_route_amounts_unresolved_count = 0usize;
    let mut atomic_route_outcome_resolved_count = 0usize;
    let mut window_not_mature_candidate_count = 0usize;
    let mut annotated_candidates = BTreeSet::new();

    for line in bytes
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
    {
        let record: Value = serde_json::from_slice(line)
            .map_err(|error| format!("R13 replay malformed JSON: {error}"))?;

        if required_str(&record, "schema_version")? != R13_SCHEMA_VERSION {
            return Err("R13 replay schema mismatch".to_owned());
        }

        let sequence = required_u64(&record, "record_sequence")?;

        if sequence != expected_sequence {
            return Err(format!(
                "R13 replay sequence mismatch: expected={expected_sequence} actual={sequence}"
            ));
        }

        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "R13 replay sequence overflow".to_owned())?;

        let current_run_id = required_str(&record, "run_id")?.to_owned();

        match run_id.as_deref() {
            None => run_id = Some(current_run_id.clone()),
            Some(expected) if expected == current_run_id => {}
            Some(_) => return Err("R13 replay run_id changed".to_owned()),
        }

        let current_github = record
            .get("github_actions")
            .cloned()
            .ok_or_else(|| "R13 replay missing github_actions".to_owned())?;

        match github.as_ref() {
            None => github = Some(current_github.clone()),
            Some(expected) if expected == &current_github => {}
            Some(_) => return Err("R13 replay GitHub provenance changed".to_owned()),
        }

        let event_type = required_str(&record, "event_type")?;

        match event_type {
            "forensics_run_start" => {
                if saw_start || saw_maturity || saw_end || sequence != 1 {
                    return Err("R13 replay invalid run_start lifecycle".to_owned());
                }

                saw_start = true;
            }
            "maturity_result" => {
                if !saw_start || saw_maturity || saw_end {
                    return Err("R13 replay invalid maturity_result lifecycle".to_owned());
                }

                let payload = required_object(&record, "payload")?;

                if required_str(payload, "policy_id")? != MATURITY_POLICY_ID {
                    return Err("R13 replay maturity policy mismatch".to_owned());
                }

                required_u64(payload, "required_end_slot")?;
                optional_u64(payload, "initial_confirmed_tip")?;
                let final_tip = optional_u64(payload, "final_confirmed_tip")?;
                let poll_attempts = required_u64(payload, "poll_attempts")?;
                let rpc_error_count = required_u64(payload, "rpc_error_count")?;
                required_u64(payload, "wait_elapsed_ms")?;

                if poll_attempts == 0 || rpc_error_count > poll_attempts {
                    return Err("R13 replay maturity poll counts invalid".to_owned());
                }

                let reached = required_bool(payload, "maturity_reached")?;
                let status = required_str(payload, "status")?;

                if reached && status != "mature" {
                    return Err("R13 replay maturity status/reached mismatch".to_owned());
                }

                if !reached && status != "window_not_mature" {
                    return Err("R13 replay immature status/reached mismatch".to_owned());
                }

                let required_end_slot = required_u64(payload, "required_end_slot")?;

                if reached {
                    let final_tip = final_tip.ok_or_else(|| {
                        "R13 replay mature evidence missing final confirmed tip".to_owned()
                    })?;

                    if final_tip < required_end_slot {
                        return Err(
                            "R13 replay mature evidence ends below required slot".to_owned()
                        );
                    }
                } else if final_tip.is_some_and(|slot| slot >= required_end_slot) {
                    return Err(
                        "R13 replay window_not_mature despite final tip reaching required slot"
                            .to_owned(),
                    );
                }

                saw_maturity = true;
                maturity_reached = Some(reached);
            }
            "route_search_result" => {
                if !saw_start || !saw_maturity || saw_end {
                    return Err("R13 replay route_search_result outside lifecycle".to_owned());
                }

                if maturity_reached != Some(true) {
                    return Err(
                        "R13 replay route search exists before evidence window matured".to_owned(),
                    );
                }

                route_results += 1;

                let payload = required_object(&record, "payload")?;
                let status = required_str(payload, "status")?;
                validate_route_status(status)?;

                match status {
                    "search_incomplete" => search_incomplete_count += 1,
                    "no_atomic_match_complete" => no_atomic_match_complete_count += 1,
                    "atomic_route_match" => atomic_route_match_count += 1,
                    "atomic_route_amounts_unresolved" => atomic_route_amounts_unresolved_count += 1,
                    "atomic_route_outcome_resolved" => atomic_route_outcome_resolved_count += 1,
                    _ => {}
                }

                let count = required_u64(payload, "intersecting_signature_count")? as usize;
                let signatures = required_array(payload, "intersecting_signatures")?;

                if signatures.len() != count {
                    return Err(
                        "R13 replay intersecting signature count does not match retained signatures"
                            .to_owned(),
                    );
                }

                let mut unique_signatures = BTreeSet::new();

                for signature in signatures {
                    let signature = signature.as_str().ok_or_else(|| {
                        "R13 replay intersecting signature was not a string".to_owned()
                    })?;

                    if !unique_signatures.insert(signature.to_owned()) {
                        return Err(
                            "R13 replay duplicate retained intersecting signature".to_owned()
                        );
                    }
                }
            }
            "transaction_match" => {
                if !saw_start || !saw_maturity || saw_end {
                    return Err("R13 replay transaction_match outside lifecycle".to_owned());
                }

                if maturity_reached != Some(true) {
                    return Err(
                        "R13 replay transaction match exists before evidence window matured"
                            .to_owned(),
                    );
                }

                transaction_matches += 1;

                let payload = required_object(&record, "payload")?;
                let status = required_str(payload, "status")?;

                if status != "atomic_route_amounts_unresolved"
                    && status != "atomic_route_outcome_resolved"
                {
                    return Err("R13 replay transaction_match status invalid".to_owned());
                }

                required_str(payload, "route_id")?;
                required_object(payload, "transaction")?;
            }
            "candidate_annotation" => {
                if !saw_start || !saw_maturity || saw_end {
                    return Err("R13 replay candidate_annotation outside lifecycle".to_owned());
                }

                candidate_annotations += 1;

                let payload = required_object(&record, "payload")?;
                let status = required_str(payload, "captureability_status")?;
                validate_candidate_status(status)?;

                match maturity_reached {
                    Some(true) if status == "window_not_mature" => {
                        return Err(
                            "R13 replay mature run contains window_not_mature candidate".to_owned()
                        )
                    }
                    Some(false) if status != "window_not_mature" => {
                        return Err(
                            "R13 replay immature run contains post-search candidate status"
                                .to_owned(),
                        )
                    }
                    Some(false) => window_not_mature_candidate_count += 1,
                    Some(true) => {}
                    None => {
                        return Err(
                            "R13 replay candidate annotation preceded maturity result".to_owned()
                        )
                    }
                }

                let source = required_object(payload, "source_r12")?;

                let key = format!(
                    "{}:{}:{}",
                    required_str(source, "run_id")?,
                    required_u64(source, "record_sequence")?,
                    required_str(source, "candidate_id")?
                );

                if !annotated_candidates.insert(key) {
                    return Err("R13 replay duplicate candidate annotation".to_owned());
                }
            }
            "forensics_run_end" => {
                if !saw_start || !saw_maturity || saw_end {
                    return Err("R13 replay invalid run_end lifecycle".to_owned());
                }

                saw_end = true;

                let payload = required_object(&record, "payload")?;
                let final_maturity_reached = required_bool(payload, "maturity_reached")?;

                if Some(final_maturity_reached) != maturity_reached {
                    return Err("R13 replay final maturity state mismatch".to_owned());
                }

                if required_u64(payload, "route_count")? as usize != expected_route_count
                    || required_u64(payload, "route_search_result_count")? as usize != route_results
                    || required_u64(payload, "candidate_annotation_count")? as usize
                        != expected_candidate_count
                    || required_u64(payload, "transaction_match_count")? as usize
                        != transaction_matches
                    || required_u64(payload, "search_incomplete_count")? as usize
                        != search_incomplete_count
                    || required_u64(payload, "no_atomic_match_complete_count")? as usize
                        != no_atomic_match_complete_count
                    || required_u64(payload, "atomic_route_match_count")? as usize
                        != atomic_route_match_count
                    || required_u64(payload, "atomic_route_amounts_unresolved_count")? as usize
                        != atomic_route_amounts_unresolved_count
                    || required_u64(payload, "atomic_route_outcome_resolved_count")? as usize
                        != atomic_route_outcome_resolved_count
                    || required_u64(payload, "window_not_mature_candidate_count")? as usize
                        != window_not_mature_candidate_count
                {
                    return Err("R13 replay final counts mismatch".to_owned());
                }
            }
            other => return Err(format!("R13 replay unsupported event type {other}")),
        }
    }

    if !saw_start || !saw_maturity || !saw_end {
        return Err("R13 replay incomplete lifecycle".to_owned());
    }

    match maturity_reached {
        Some(true) if route_results != expected_route_count => {
            return Err(format!(
                "R13 replay mature route result count mismatch: expected={expected_route_count} actual={route_results}"
            ))
        }
        Some(false) if route_results != 0 => {
            return Err(
                "R13 replay immature evidence must contain zero route_search_result records"
                    .to_owned(),
            )
        }
        Some(false) if transaction_matches != 0 => {
            return Err(
                "R13 replay immature evidence must contain zero transaction_match records"
                    .to_owned(),
            )
        }
        Some(_) => {}
        None => return Err("R13 replay maturity result unavailable".to_owned()),
    }

    if candidate_annotations != expected_candidate_count
        || annotated_candidates.len() != expected_candidate_count
    {
        return Err(format!(
            "R13 replay candidate annotation coverage mismatch: expected={expected_candidate_count} actual={candidate_annotations}"
        ));
    }

    if maturity_reached == Some(false)
        && window_not_mature_candidate_count != expected_candidate_count
    {
        return Err(format!(
            "R13 replay immature candidate coverage mismatch: expected={expected_candidate_count} actual={window_not_mature_candidate_count}"
        ));
    }

    Ok(())
}

fn validate_route_status(status: &str) -> Result<(), String> {
    match status {
        "search_incomplete"
        | "no_atomic_match_complete"
        | "atomic_route_match"
        | "atomic_route_amounts_unresolved"
        | "atomic_route_outcome_resolved" => Ok(()),
        other => Err(format!("R13 unsupported captureability status {other}")),
    }
}

fn validate_candidate_status(status: &str) -> Result<(), String> {
    if status == "window_not_mature" {
        Ok(())
    } else {
        validate_route_status(status)
    }
}

fn route_json(route: &RouteEvidence) -> Value {
    json!({
        "route_id": route.route_id,
        "anchor_mint": route.anchor_mint,
        "intermediate_mint": route.intermediate_mint,
        "leg_1": leg_json(&route.leg_1),
        "leg_2": leg_json(&route.leg_2),
    })
}

fn leg_json(leg: &LegEvidence) -> Value {
    json!({
        "venue": leg.venue,
        "pool_id": leg.pool_id,
        "input_mint": leg.input_mint,
        "output_mint": leg.output_mint,
        "source_slot": leg.source_slot,
    })
}

fn build_run_id(now_ms: u64) -> String {
    let gha_run = env_nonempty("GITHUB_RUN_ID").unwrap_or_else(|| "local".to_owned());
    let gha_attempt = env_nonempty("GITHUB_RUN_ATTEMPT").unwrap_or_else(|| "0".to_owned());

    format!("r13-{gha_run}-{gha_attempt}-{now_ms}-{}", process::id())
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("R13 system clock precedes Unix epoch: {error}"))?;

    u64::try_from(duration.as_millis())
        .map_err(|_| "R13 Unix millisecond timestamp overflow".to_owned())
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, String> {
    value
        .get(field)
        .filter(|item| item.is_object())
        .ok_or_else(|| format!("R13 missing or invalid object field {field}"))
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("R13 missing or invalid array field {field}"))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("R13 missing or invalid string field {field}"))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("R13 missing or invalid u64 field {field}"))
}

fn required_bool(value: &Value, field: &str) -> Result<bool, String> {
    value
        .get(field)
        .and_then(Value::as_bool)
        .ok_or_else(|| format!("R13 missing or invalid bool field {field}"))
}

fn optional_u64(value: &Value, field: &str) -> Result<Option<u64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_u64()
            .map(Some)
            .ok_or_else(|| format!("R13 invalid optional u64 field {field}")),
    }
}

fn optional_i64(value: &Value, field: &str) -> Result<Option<i64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("R13 invalid optional i64 field {field}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forensics_rpc::{AddressHistory, SignatureObservation};

    fn leg(venue: &str, pool: &str, input: &str, output: &str, slot: u64) -> LegEvidence {
        LegEvidence {
            venue: venue.to_owned(),
            pool_id: pool.to_owned(),
            input_mint: input.to_owned(),
            output_mint: output.to_owned(),
            source_slot: slot,
        }
    }

    fn route() -> RouteEvidence {
        RouteEvidence {
            route_id: "route".to_owned(),
            anchor_mint: "anchor".to_owned(),
            intermediate_mint: "middle".to_owned(),
            leg_1: leg("raydium_cpmm", "ray", "anchor", "middle", 100),
            leg_2: leg("pumpswap", "pump", "middle", "anchor", 101),
        }
    }

    fn candidate(route: RouteEvidence) -> CandidateEvidence {
        CandidateEvidence {
            source_run_id: "source-run".to_owned(),
            source_record_sequence: 2,
            candidate_id: "candidate".to_owned(),
            source_status: "economics_unresolved".to_owned(),
            usd_size: 10,
            candidate_found_at_unix_ms: 1000,
            quote_complete_at_unix_ms: Some(1001),
            economics_complete_at_unix_ms: Some(1002),
            hypothetical_ready_at_unix_ms: None,
            route,
        }
    }

    fn plan_with_candidate() -> Result<ForensicsPlan, String> {
        let route = route();
        let left_request = route.history_request(&route.leg_1)?;
        let right_request = route.history_request(&route.leg_2)?;

        Ok(ForensicsPlan {
            source_path: PathBuf::from("source"),
            source_run_id: "source-run".to_owned(),
            source_github_actions: json!({}),
            candidates: vec![candidate(route.clone())],
            routes: BTreeMap::from([("route".to_owned(), route)]),
            history_requests: vec![left_request, right_request],
        })
    }

    fn resolved_instruction(program_id: &str, account_keys: Vec<&str>, data: Vec<u8>) -> ResolvedInstruction {
        ResolvedInstruction {
            coordinate: InstructionCoordinate {
                outer_index: 0,
                inner_index: None,
                stack_height: Some(1),
            },
            program_id: program_id.to_owned(),
            account_keys: account_keys
                .into_iter()
                .map(str::to_owned)
                .collect::<Vec<_>>(),
            data,
        }
    }

    fn orca_swap_data(discriminator: [u8; 8], a_to_b: bool) -> Vec<u8> {
        let mut data = vec![0u8; ORCA_SWAP_MIN_DATA_LEN];
        data[..8].copy_from_slice(&discriminator);
        data[40] = 1;
        data[ORCA_SWAP_DIRECTION_OFFSET] = u8::from(a_to_b);
        data
    }

    fn token_balance(account_index: u64, mint: &str) -> Value {
        json!({
            "accountIndex": account_index,
            "mint": mint,
            "uiTokenAmount": {
                "amount": "1",
                "decimals": 6,
            },
        })
    }

    #[test]
    fn search_window_uses_maximum_leg_slot() -> Result<(), String> {
        let route = route();

        assert_eq!(route.start_slot(), 101);
        assert_eq!(route.end_slot()?, 133);

        Ok(())
    }

    #[test]
    fn plan_required_end_slot_is_maximum_route_end() -> Result<(), String> {
        let mut second = route();
        second.route_id = "route-two".to_owned();
        second.leg_1.source_slot = 150;
        second.leg_2.source_slot = 149;

        let first = route();

        let plan = ForensicsPlan {
            source_path: PathBuf::from("source"),
            source_run_id: "run".to_owned(),
            source_github_actions: json!({}),
            candidates: Vec::new(),
            routes: BTreeMap::from([
                ("route".to_owned(), first),
                ("route-two".to_owned(), second),
            ]),
            history_requests: Vec::new(),
        };

        assert_eq!(plan.required_end_slot()?, 182);

        Ok(())
    }

    #[test]
    fn maturity_contract_distinguishes_window_not_mature() -> Result<(), String> {
        let plan = plan_with_candidate()?;

        let immature = EvidenceMaturity {
            required_end_slot: 133,
            initial_confirmed_tip: Some(102),
            final_confirmed_tip: Some(120),
            poll_attempts: 5,
            rpc_error_count: 0,
            wait_elapsed_ms: 2000,
            maturity_reached: false,
        };

        immature.validate_for_plan(&plan)?;
        assert_eq!(immature.status(), "window_not_mature");
        assert!(immature.reason().is_some());

        let mature = EvidenceMaturity {
            required_end_slot: 133,
            initial_confirmed_tip: Some(102),
            final_confirmed_tip: Some(133),
            poll_attempts: 8,
            rpc_error_count: 1,
            wait_elapsed_ms: 3500,
            maturity_reached: true,
        };

        mature.validate_for_plan(&plan)?;
        assert_eq!(mature.status(), "mature");
        assert!(mature.reason().is_none());

        Ok(())
    }

    #[test]
    fn maturity_contract_rejects_false_immaturity() -> Result<(), String> {
        let plan = plan_with_candidate()?;

        let maturity = EvidenceMaturity {
            required_end_slot: 133,
            initial_confirmed_tip: Some(102),
            final_confirmed_tip: Some(134),
            poll_attempts: 8,
            rpc_error_count: 0,
            wait_elapsed_ms: 3500,
            maturity_reached: false,
        };

        assert!(maturity.validate_for_plan(&plan).is_err());

        Ok(())
    }

    #[test]
    fn exact_route_intersection_uses_only_two_requested_pool_histories() -> Result<(), String> {
        let route = route();

        let left_request = route.history_request(&route.leg_1)?;
        let right_request = route.history_request(&route.leg_2)?;

        let history = |request: HistoryRequest, signatures: &[&str]| AddressHistory {
            request,
            observations: signatures
                .iter()
                .map(|signature| SignatureObservation {
                    signature: (*signature).to_owned(),
                    slot: 110,
                    err: Value::Null,
                    memo: None,
                    block_time: None,
                    confirmation_status: Some("confirmed".to_owned()),
                })
                .collect(),
            complete_through_start_slot: true,
            reason: None,
        };

        let plan = ForensicsPlan {
            source_path: PathBuf::from("source"),
            source_run_id: "run".to_owned(),
            source_github_actions: json!({}),
            candidates: Vec::new(),
            routes: BTreeMap::from([("route".to_owned(), route.clone())]),
            history_requests: vec![left_request.clone(), right_request.clone()],
        };

        let acquisition = HistoryAcquisition {
            confirmed_tip_slot: Some(200),
            histories: BTreeMap::from([
                (
                    left_request.clone(),
                    history(left_request, &["shared", "left"]),
                ),
                (
                    right_request.clone(),
                    history(right_request, &["shared", "right"]),
                ),
            ]),
            incomplete_reasons: Vec::new(),
        };

        let intersections = intersect_route_histories(&plan, &acquisition)?;

        assert_eq!(
            intersections.required_signatures,
            BTreeSet::from(["shared".to_owned()])
        );

        Ok(())
    }

    #[test]
    fn incomplete_history_never_becomes_complete_no_match() -> Result<(), String> {
        let route = route();

        let left_request = route.history_request(&route.leg_1)?;
        let right_request = route.history_request(&route.leg_2)?;

        let plan = ForensicsPlan {
            source_path: PathBuf::from("source"),
            source_run_id: "run".to_owned(),
            source_github_actions: json!({}),
            candidates: Vec::new(),
            routes: BTreeMap::from([("route".to_owned(), route.clone())]),
            history_requests: vec![left_request.clone(), right_request.clone()],
        };

        let acquisition = HistoryAcquisition {
            confirmed_tip_slot: Some(200),
            histories: BTreeMap::from([
                (
                    left_request.clone(),
                    AddressHistory {
                        request: left_request,
                        observations: Vec::new(),
                        complete_through_start_slot: false,
                        reason: Some("saturated".to_owned()),
                    },
                ),
                (
                    right_request.clone(),
                    AddressHistory {
                        request: right_request,
                        observations: Vec::new(),
                        complete_through_start_slot: true,
                        reason: None,
                    },
                ),
            ]),
            incomplete_reasons: vec!["saturated".to_owned()],
        };

        let intersections = intersect_route_histories(&plan, &acquisition)?;

        assert!(!intersections.routes["route"].complete);

        Ok(())
    }

    #[test]
    fn instruction_coordinates_preserve_outer_inner_order() {
        let outer = InstructionCoordinate {
            outer_index: 2,
            inner_index: None,
            stack_height: Some(1),
        };

        let inner = InstructionCoordinate {
            outer_index: 2,
            inner_index: Some(0),
            stack_height: Some(2),
        };

        let later = InstructionCoordinate {
            outer_index: 3,
            inner_index: None,
            stack_height: Some(1),
        };

        assert!(coordinate_precedes(&outer, &inner));
        assert!(coordinate_precedes(&inner, &later));
        assert!(!coordinate_precedes(&later, &inner));
    }

    #[test]
    fn orca_swap_v2_matches_both_directions_with_instruction_mint_proof() -> Result<(), String> {
        let accounts = vec![
            "token-program-a",
            "token-program-b",
            "memo",
            "authority",
            "orca-pool",
            "mint-a",
            "mint-b",
            "owner-a",
            "vault-a",
            "owner-b",
            "vault-b",
            "tick-0",
            "tick-1",
            "tick-2",
            "oracle",
        ];

        let a_to_b_instruction = resolved_instruction(
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            accounts.clone(),
            orca_swap_data(ORCA_SWAP_V2, true),
        );
        let a_to_b_leg = leg("orca", "orca-pool", "mint-a", "mint-b", 100);
        let a_to_b = match_leg(&a_to_b_leg, &a_to_b_instruction)?
            .ok_or_else(|| "Orca SwapV2 A-to-B fixture did not match".to_owned())?;

        assert_eq!(a_to_b.user_input_token_account, "owner-a");
        assert_eq!(a_to_b.user_output_token_account, "owner-b");
        assert!(a_to_b.mints_verified_by_instruction);

        let b_to_a_instruction = resolved_instruction(
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            accounts,
            orca_swap_data(ORCA_SWAP_V2, false),
        );
        let b_to_a_leg = leg("orca", "orca-pool", "mint-b", "mint-a", 100);
        let b_to_a = match_leg(&b_to_a_leg, &b_to_a_instruction)?
            .ok_or_else(|| "Orca SwapV2 B-to-A fixture did not match".to_owned())?;

        assert_eq!(b_to_a.user_input_token_account, "owner-b");
        assert_eq!(b_to_a.user_output_token_account, "owner-a");
        assert!(b_to_a.mints_verified_by_instruction);

        Ok(())
    }

    #[test]
    fn orca_legacy_swap_requires_and_accepts_token_balance_mint_proof() -> Result<(), String> {
        let instruction = resolved_instruction(
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            vec![
                "token-program",
                "authority",
                "orca-pool",
                "owner-a",
                "vault-a",
                "owner-b",
                "vault-b",
                "tick-0",
                "tick-1",
                "tick-2",
                "oracle",
            ],
            orca_swap_data(ORCA_SWAP, true),
        );
        let route_leg = leg("orca", "orca-pool", "mint-a", "mint-b", 100);
        let matched = match_leg(&route_leg, &instruction)?
            .ok_or_else(|| "Orca legacy swap fixture did not match".to_owned())?;

        assert!(!matched.mints_verified_by_instruction);
        assert_eq!(matched.user_input_token_account, "owner-a");
        assert_eq!(matched.user_output_token_account, "owner-b");

        let account_keys = vec!["owner-a".to_owned(), "owner-b".to_owned()];
        let complete = json!({
            "preTokenBalances": [
                token_balance(0, "mint-a"),
                token_balance(1, "mint-b")
            ],
            "postTokenBalances": [
                token_balance(0, "mint-a"),
                token_balance(1, "mint-b")
            ]
        });

        assert_eq!(
            matched_leg_mints_proven(&matched, &complete, &account_keys)?,
            Some(true)
        );

        let missing = json!({
            "preTokenBalances": [],
            "postTokenBalances": []
        });

        assert_eq!(
            matched_leg_mints_proven(&matched, &missing, &account_keys)?,
            None
        );

        let mismatched = json!({
            "preTokenBalances": [
                token_balance(0, "wrong-mint"),
                token_balance(1, "mint-b")
            ],
            "postTokenBalances": [
                token_balance(0, "wrong-mint"),
                token_balance(1, "mint-b")
            ]
        });

        assert_eq!(
            matched_leg_mints_proven(&matched, &mismatched, &account_keys)?,
            Some(false)
        );

        Ok(())
    }

    #[test]
    fn orca_swap_rejects_wrong_pool_and_malformed_direction() -> Result<(), String> {
        let accounts = vec![
            "token-program-a",
            "token-program-b",
            "memo",
            "authority",
            "wrong-pool",
            "mint-a",
            "mint-b",
            "owner-a",
            "vault-a",
            "owner-b",
            "vault-b",
            "tick-0",
            "tick-1",
            "tick-2",
            "oracle",
        ];
        let route_leg = leg("orca", "orca-pool", "mint-a", "mint-b", 100);

        let wrong_pool = resolved_instruction(
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            accounts.clone(),
            orca_swap_data(ORCA_SWAP_V2, true),
        );

        assert!(match_leg(&route_leg, &wrong_pool)?.is_none());

        let mut malformed_data = orca_swap_data(ORCA_SWAP_V2, true);
        malformed_data[ORCA_SWAP_DIRECTION_OFFSET] = 2;

        let mut valid_accounts = accounts;
        valid_accounts[4] = "orca-pool";

        let malformed = resolved_instruction(
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            valid_accounts,
            malformed_data,
        );

        assert!(matches!(
            match_leg(&route_leg, &malformed),
            Err(error) if error.contains("a_to_b bool invalid")
        ));

        Ok(())
    }

    #[test]
    fn failed_transaction_is_not_a_route_match() -> Result<(), String> {
        let evidence = TransactionEvidence {
            signature: "sig".to_owned(),
            value: json!({
                "slot": 110,
                "blockTime": null,
                "transaction": {
                    "message": {
                        "accountKeys": [],
                        "instructions": []
                    }
                },
                "meta": {
                    "err": {
                        "InstructionError": [0, "Custom"]
                    },
                    "fee": 5000
                }
            }),
        };

        assert!(match_transaction(&route(), &evidence)?.is_none());

        Ok(())
    }

    #[test]
    fn immature_artifact_contains_no_route_search_results() -> Result<(), String> {
        let plan = plan_with_candidate()?;
        let maturity = EvidenceMaturity {
            required_end_slot: 133,
            initial_confirmed_tip: Some(102),
            final_confirmed_tip: Some(120),
            poll_attempts: 5,
            rpc_error_count: 0,
            wait_elapsed_ms: 2000,
            maturity_reached: false,
        };

        let directory = env::temp_dir().join(format!(
            "r13-forensics-immature-test-{}-{}",
            process::id(),
            unix_time_ms_now()?
        ));

        if directory.exists() {
            fs::remove_dir_all(&directory)
                .map_err(|error| format!("could not clear R13 test directory: {error}"))?;
        }

        let result =
            write_forensics_artifact_in_directory(&directory, &plan, &maturity, None, None)?;

        assert!(!result.maturity_reached);
        assert_eq!(result.window_not_mature_candidate_count, 1);

        let text = fs::read_to_string(&result.output_path)
            .map_err(|error| format!("could not read R13 immature test artifact: {error}"))?;

        let mut maturity_results = 0usize;
        let mut route_results = 0usize;
        let mut immature_annotations = 0usize;

        for line in text.lines() {
            let record: Value = serde_json::from_str(line)
                .map_err(|error| format!("could not parse R13 immature test artifact: {error}"))?;

            match required_str(&record, "event_type")? {
                "maturity_result" => maturity_results += 1,
                "route_search_result" => route_results += 1,
                "candidate_annotation" => {
                    let payload = required_object(&record, "payload")?;
                    if required_str(payload, "captureability_status")? == "window_not_mature" {
                        immature_annotations += 1;
                    }
                }
                _ => {}
            }
        }

        assert_eq!(maturity_results, 1);
        assert_eq!(route_results, 0);
        assert_eq!(immature_annotations, 1);

        fs::remove_dir_all(&directory)
            .map_err(|error| format!("could not remove R13 test directory: {error}"))?;

        Ok(())
    }

    #[test]
    fn mature_artifact_retains_exact_intersecting_signatures() -> Result<(), String> {
        let plan = plan_with_candidate()?;

        let maturity = EvidenceMaturity {
            required_end_slot: 133,
            initial_confirmed_tip: Some(102),
            final_confirmed_tip: Some(133),
            poll_attempts: 8,
            rpc_error_count: 0,
            wait_elapsed_ms: 3500,
            maturity_reached: true,
        };

        let intersections = IntersectionPlan {
            routes: BTreeMap::from([(
                "route".to_owned(),
                RouteIntersection {
                    route_id: "route".to_owned(),
                    signatures: BTreeSet::from([
                        "signature-a".to_owned(),
                        "signature-b".to_owned(),
                    ]),
                    complete: true,
                    incomplete_reason: None,
                },
            )]),
            required_signatures: BTreeSet::from([
                "signature-a".to_owned(),
                "signature-b".to_owned(),
            ]),
        };

        let analyses = BTreeMap::from([(
            "route".to_owned(),
            RouteAnalysis {
                route_id: "route".to_owned(),
                status: "no_atomic_match_complete".to_owned(),
                reason: Some(
                    "intersecting signatures existed but none proved both exact supported route legs"
                        .to_owned(),
                ),
                matches: Vec::new(),
            },
        )]);

        let directory = env::temp_dir().join(format!(
            "r13-forensics-mature-test-{}-{}",
            process::id(),
            unix_time_ms_now()?
        ));

        if directory.exists() {
            fs::remove_dir_all(&directory)
                .map_err(|error| format!("could not clear R13 test directory: {error}"))?;
        }

        let result = write_forensics_artifact_in_directory(
            &directory,
            &plan,
            &maturity,
            Some(&intersections),
            Some(&analyses),
        )?;

        assert!(result.maturity_reached);

        let text = fs::read_to_string(&result.output_path)
            .map_err(|error| format!("could not read R13 mature test artifact: {error}"))?;

        let mut retained = BTreeSet::new();

        for line in text.lines() {
            let record: Value = serde_json::from_str(line)
                .map_err(|error| format!("could not parse R13 mature test artifact: {error}"))?;

            if required_str(&record, "event_type")? != "route_search_result" {
                continue;
            }

            let payload = required_object(&record, "payload")?;
            for signature in required_array(payload, "intersecting_signatures")? {
                retained.insert(
                    signature
                        .as_str()
                        .ok_or_else(|| "R13 mature test retained non-string signature".to_owned())?
                        .to_owned(),
                );
            }
        }

        assert_eq!(
            retained,
            BTreeSet::from(["signature-a".to_owned(), "signature-b".to_owned()])
        );

        fs::remove_dir_all(&directory)
            .map_err(|error| format!("could not remove R13 test directory: {error}"))?;

        Ok(())
    }

    #[test]
    fn status_vocabularies_keep_maturity_separate_from_route_search() {
        assert!(validate_route_status("search_incomplete").is_ok());
        assert!(validate_route_status("no_atomic_match_complete").is_ok());
        assert!(validate_route_status("atomic_route_match").is_ok());
        assert!(validate_route_status("atomic_route_amounts_unresolved").is_ok());
        assert!(validate_route_status("atomic_route_outcome_resolved").is_ok());

        assert!(validate_route_status("window_not_mature").is_err());
        assert!(validate_candidate_status("window_not_mature").is_ok());
        assert!(validate_candidate_status("profitable_winner").is_err());
    }
}
