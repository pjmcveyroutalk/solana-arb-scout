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
            let r
