use crate::costs::{self, JitoObservationState, PriorityObservationState};
use crate::economics::{EconomicsCostModel, ExpectedNetEconomics, RequiredCost};
use crate::quote::{TwoLegRouteQuote, VenueFeeComponents, VenueLegQuote};
use crate::route::TwoLegRouteCandidate;
use crate::sizing::SolUsdPrice;
use scout_core::NormalizedPoolState;
use serde_json::{json, Value};
use std::collections::BTreeMap;
use std::env;
use std::fs::{create_dir_all, File, OpenOptions};
use std::io::{BufRead, BufReader, BufWriter, Write};
use std::path::{Path, PathBuf};
use std::process;
use std::time::{SystemTime, UNIX_EPOCH};

pub const SCHEMA_VERSION: &str = "r12-shadow-v1";
pub const MAX_RECORDS_PER_RUN: u64 = 2_048;
pub const OUTPUT_DIRECTORY: &str = "artifacts/r12-shadow";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct CandidateTiming {
    pub candidate_found_at_unix_ms: u64,
    pub quote_complete_at_unix_ms: Option<u64>,
    pub economics_complete_at_unix_ms: Option<u64>,
    pub hypothetical_ready_at_unix_ms: Option<u64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct LifecycleState {
    first_seen_at_unix_ms: u64,
    last_seen_at_unix_ms: u64,
    observation_count: u64,
}

impl LifecycleState {
    fn observe(&mut self, observed_at_unix_ms: u64) -> Result<(), String> {
        self.first_seen_at_unix_ms = self.first_seen_at_unix_ms.min(observed_at_unix_ms);
        self.last_seen_at_unix_ms = self.last_seen_at_unix_ms.max(observed_at_unix_ms);
        self.observation_count = self
            .observation_count
            .checked_add(1)
            .ok_or_else(|| "R12 lifecycle observation count overflow".to_owned())?;
        Ok(())
    }

    fn lifetime_ms(self) -> Result<u64, String> {
        self.last_seen_at_unix_ms
            .checked_sub(self.first_seen_at_unix_ms)
            .ok_or_else(|| "R12 lifecycle timestamp underflow".to_owned())
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct SummaryCounts {
    route_rejections: u64,
    quote_rejections: u64,
    economics_unresolved: u64,
    economics_resolved_nonpositive: u64,
    economics_resolved_positive: u64,
}

impl SummaryCounts {
    fn new() -> Self {
        Self {
            route_rejections: 0,
            quote_rejections: 0,
            economics_unresolved: 0,
            economics_resolved_nonpositive: 0,
            economics_resolved_positive: 0,
        }
    }

    fn total_evidence(self) -> Result<u64, String> {
        [
            self.route_rejections,
            self.quote_rejections,
            self.economics_unresolved,
            self.economics_resolved_nonpositive,
            self.economics_resolved_positive,
        ]
        .into_iter()
        .try_fold(0u64, |total, count| {
            total
                .checked_add(count)
                .ok_or_else(|| "R12 summary count overflow".to_owned())
        })
    }

    fn increment_status(&mut self, status: &str) -> Result<(), String> {
        let counter = match status {
            "quote_rejected" => &mut self.quote_rejections,
            "economics_unresolved" => &mut self.economics_unresolved,
            "economics_resolved_nonpositive" => &mut self.economics_resolved_nonpositive,
            "economics_resolved_positive" => &mut self.economics_resolved_positive,
            other => return Err(format!("unsupported R12 candidate status: {other}")),
        };

        *counter = counter
            .checked_add(1)
            .ok_or_else(|| "R12 summary status count overflow".to_owned())?;
        Ok(())
    }
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
            "github_run_id": self.run_id.as_deref(),
            "github_run_attempt": self.run_attempt.as_deref(),
            "github_sha": self.sha.as_deref(),
            "github_workflow": self.workflow.as_deref(),
            "github_job": self.job.as_deref(),
            "github_ref": self.git_ref.as_deref(),
        })
    }
}

pub struct ShadowRecorder {
    run_id: String,
    output_path: PathBuf,
    writer: BufWriter<File>,
    next_sequence: u64,
    records_written: u64,
    lifecycle: BTreeMap<String, LifecycleState>,
    summary: SummaryCounts,
    github: GithubActionsProvenance,
    pool_timing: BTreeMap<String, NormalizedPoolState>,
}

impl ShadowRecorder {
    pub fn start(
        eligible_pools: &[NormalizedPoolState],
        route_count: usize,
        usd_grid: &[u64],
        sol_usd: &SolUsdPrice,
        usdc_usd: Option<&SolUsdPrice>,
        usdt_usd: Option<&SolUsdPrice>,
    ) -> Result<Self, String> {
        Self::start_in_directory(
            Path::new(OUTPUT_DIRECTORY),
            eligible_pools,
            route_count,
            usd_grid,
            sol_usd,
            usdc_usd,
            usdt_usd,
        )
    }

    fn start_in_directory(
        output_directory: &Path,
        eligible_pools: &[NormalizedPoolState],
        route_count: usize,
        usd_grid: &[u64],
        sol_usd: &SolUsdPrice,
        usdc_usd: Option<&SolUsdPrice>,
        usdt_usd: Option<&SolUsdPrice>,
    ) -> Result<Self, String> {
        create_dir_all(output_directory)
            .map_err(|error| format!("could not create R12 output directory: {error}"))?;

        let now = unix_time_ms_now()?;
        let run_id = build_run_id(now);
        let output_path = output_directory.join(format!("{run_id}.jsonl"));
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&output_path)
            .map_err(|error| {
                format!(
                    "could not create immutable R12 run file {}: {error}",
                    output_path.display()
                )
            })?;

        let mut pool_timing = BTreeMap::new();
        for pool in eligible_pools {
            if pool_timing
                .insert(pool.pool_id.clone(), pool.clone())
                .is_some()
            {
                return Err(format!(
                    "R12 eligible-pool timing map contained duplicate pool id {}",
                    pool.pool_id
                ));
            }
        }

        let mut recorder = Self {
            run_id,
            output_path,
            writer: BufWriter::new(file),
            next_sequence: 1,
            records_written: 0,
            lifecycle: BTreeMap::new(),
            summary: SummaryCounts::new(),
            github: GithubActionsProvenance::from_environment(),
            pool_timing,
        };

        let payload = json!({
            "route_candidate_count": route_count,
            "eligible_pool_count": eligible_pools.len(),
            "usd_size_grid": usd_grid,
            "recorder_capacity": MAX_RECORDS_PER_RUN,
            "pyth_usd": pyth_bundle_value(sol_usd, usdc_usd, usdt_usd),
        });
        recorder.write_event("run_start", now, payload)?;

        Ok(recorder)
    }

    pub fn output_path(&self) -> &Path {
        &self.output_path
    }

    pub fn record_route_rejection(
        &mut self,
        route: &TwoLegRouteCandidate,
        candidate_found_at_unix_ms: u64,
        reason: &str,
    ) -> Result<(), String> {
        if reason.trim().is_empty() {
            return Err("R12 route rejection reason must not be empty".to_owned());
        }

        let observed_at = unix_time_ms_now()?;
        let payload = json!({
            "route": self.route_value(route)?,
            "candidate_found_at_unix_ms": candidate_found_at_unix_ms,
            "reason": reason,
        });
        self.write_event("route_rejection", observed_at, payload)?;
        self.summary.route_rejections = self
            .summary
            .route_rejections
            .checked_add(1)
            .ok_or_else(|| "R12 route rejection count overflow".to_owned())?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_quote_rejection(
        &mut self,
        route: &TwoLegRouteCandidate,
        usd_size: u64,
        requested_anchor_input_raw: Option<u64>,
        reason: &str,
        timing: CandidateTiming,
        sol_usd: &SolUsdPrice,
        usdc_usd: Option<&SolUsdPrice>,
        usdt_usd: Option<&SolUsdPrice>,
    ) -> Result<(), String> {
        if reason.trim().is_empty() {
            return Err("R12 quote rejection reason must not be empty".to_owned());
        }

        let observed_at = unix_time_ms_now()?;
        let candidate_id = canonical_candidate_id(route, usd_size);
        let lifecycle = self.observe_candidate(&candidate_id, observed_at)?;
        let payload = json!({
            "candidate_id": candidate_id,
            "status": "quote_rejected",
            "route": self.route_value(route)?,
            "usd_size": usd_size,
            "requested_anchor_input_raw": requested_anchor_input_raw,
            "quote": Value::Null,
            "quote_rejection_reason": reason,
            "cost_model": Value::Null,
            "cost_model_error": Value::Null,
            "treasury_evaluation": Value::Null,
            "flash_evaluation": Value::Null,
            "priority_observation": Value::Null,
            "jito_observation": Value::Null,
            "pyth_usd": pyth_bundle_value(sol_usd, usdc_usd, usdt_usd),
            "timing": candidate_timing_value(timing),
            "lifecycle": lifecycle_value(lifecycle)?,
        });
        self.write_event("candidate_evaluation", observed_at, payload)?;
        self.summary.increment_status("quote_rejected")?;
        Ok(())
    }

    #[allow(clippy::too_many_arguments)]
    pub fn record_economics_evaluation(
        &mut self,
        route: &TwoLegRouteCandidate,
        usd_size: u64,
        anchor_decimals: u8,
        quote: &TwoLegRouteQuote,
        cost_model: Option<&EconomicsCostModel>,
        cost_model_error: Option<&str>,
        treasury: Option<&Result<ExpectedNetEconomics, String>>,
        flash: Option<&Result<ExpectedNetEconomics, String>>,
        priority_observation: &PriorityObservationState,
        jito_observation: &JitoObservationState,
        timing: CandidateTiming,
        sol_usd: &SolUsdPrice,
        usdc_usd: Option<&SolUsdPrice>,
        usdt_usd: Option<&SolUsdPrice>,
    ) -> Result<&'static str, String> {
        match (cost_model, cost_model_error) {
            (Some(_), None) => {}
            (None, Some(error)) if !error.trim().is_empty() => {}
            (None, Some(_)) => {
                return Err("R12 cost-model error must not be empty".to_owned());
            }
            _ => {
                return Err(
                    "R12 successful quote requires exactly one of cost model or cost-model error"
                        .to_owned(),
                );
            }
        }

        let status = classify_economics_status(treasury, flash);
        let observed_at = unix_time_ms_now()?;
        let candidate_id = canonical_candidate_id(route, usd_size);
        let lifecycle = self.observe_candidate(&candidate_id, observed_at)?;

        let gross_delta_raw = i128::from(quote.anchor_output_raw)
            .checked_sub(i128::from(quote.anchor_input_requested_raw))
            .ok_or_else(|| "R12 gross delta subtraction overflow".to_owned())?;

        let payload = json!({
            "candidate_id": candidate_id,
            "status": status,
            "route": self.route_value(route)?,
            "usd_size": usd_size,
            "anchor_decimals": anchor_decimals,
            "requested_anchor_input_raw": quote.anchor_input_requested_raw,
            "quote": route_quote_value(quote),
            "quote_rejection_reason": Value::Null,
            "gross_delta_raw": gross_delta_raw.to_string(),
            "cost_model": cost_model.map(cost_model_value).unwrap_or(Value::Null),
            "cost_model_error": cost_model_error,
            "treasury_evaluation": funding_result_value(treasury),
            "flash_evaluation": funding_result_value(flash),
            "priority_observation": priority_state_value(priority_observation),
            "jito_observation": jito_state_value(jito_observation),
            "pyth_usd": pyth_bundle_value(sol_usd, usdc_usd, usdt_usd),
            "timing": candidate_timing_value(timing),
            "lifecycle": lifecycle_value(lifecycle)?,
        });
        self.write_event("candidate_evaluation", observed_at, payload)?;
        self.summary.increment_status(status)?;
        Ok(status)
    }

    pub fn finish(mut self) -> Result<PathBuf, String> {
        let evidence_count = self.summary.total_evidence()?;
        if evidence_count == 0 {
            return Err("R12 run produced no candidate or route-rejection evidence".to_owned());
        }

        let observed_at = unix_time_ms_now()?;
        let payload = json!({
            "route_rejections": self.summary.route_rejections,
            "quote_rejections": self.summary.quote_rejections,
            "economics_unresolved": self.summary.economics_unresolved,
            "economics_resolved_nonpositive": self.summary.economics_resolved_nonpositive,
            "economics_resolved_positive": self.summary.economics_resolved_positive,
            "evidence_record_count": evidence_count,
            "candidate_identity_count": self.lifecycle.len(),
        });
        self.write_event("run_end", observed_at, payload)?;
        self.writer
            .flush()
            .map_err(|error| format!("could not flush R12 JSONL evidence: {error}"))?;
        self.writer
            .get_ref()
            .sync_all()
            .map_err(|error| format!("could not sync R12 JSONL evidence: {error}"))?;

        validate_jsonl_replay(&self.output_path)?;
        Ok(self.output_path)
    }

    fn observe_candidate(
        &mut self,
        candidate_id: &str,
        observed_at_unix_ms: u64,
    ) -> Result<LifecycleState, String> {
        let state = self
            .lifecycle
            .entry(candidate_id.to_owned())
            .or_insert(LifecycleState {
                first_seen_at_unix_ms: observed_at_unix_ms,
                last_seen_at_unix_ms: observed_at_unix_ms,
                observation_count: 0,
            });
        state.observe(observed_at_unix_ms)?;
        Ok(*state)
    }

    fn route_value(&self, route: &TwoLegRouteCandidate) -> Result<Value, String> {
        Ok(json!({
            "route_id": canonical_route_id(route),
            "anchor_mint": route.anchor_mint(),
            "intermediate_mint": route.intermediate_mint(),
            "leg_1": self.route_leg_value(route.leg_1())?,
            "leg_2": self.route_leg_value(route.leg_2())?,
        }))
    }

    fn route_leg_value(&self, leg: &crate::route::RouteLeg) -> Result<Value, String> {
        let pool = self.pool_timing.get(leg.pool_id()).ok_or_else(|| {
            format!(
                "R12 route leg pool missing from eligible-pool timing evidence: {}",
                leg.pool_id()
            )
        })?;

        Ok(json!({
            "venue": leg.venue().label(),
            "pool_id": leg.pool_id(),
            "input_mint": leg.input_mint(),
            "output_mint": leg.output_mint(),
            "source_slot": leg.source_slot(),
            "account_update_received_at_unix_ms": pool.account_update_received_at_unix_ms,
            "normalized_at_unix_ms": pool.normalized_at_unix_ms,
        }))
    }

    fn write_event(
        &mut self,
        event_type: &str,
        observed_at_unix_ms: u64,
        payload: Value,
    ) -> Result<(), String> {
        if self.records_written >= MAX_RECORDS_PER_RUN {
            return Err(format!(
                "R12 recorder capacity exhausted: max_records={MAX_RECORDS_PER_RUN}"
            ));
        }

        if event_type != "run_end" && self.records_written >= MAX_RECORDS_PER_RUN - 1 {
            return Err(format!(
                "R12 recorder capacity reserved for run_end: max_records={MAX_RECORDS_PER_RUN}"
            ));
        }

        let record = json!({
            "schema_version": SCHEMA_VERSION,
            "event_type": event_type,
            "run_id": self.run_id.as_str(),
            "record_sequence": self.next_sequence,
            "observed_at_unix_ms": observed_at_unix_ms,
            "github_actions": self.github.as_json(),
            "payload": payload,
        });
        let mut line = serde_json::to_vec(&record)
            .map_err(|error| format!("could not serialize R12 record: {error}"))?;
        line.push(b'\n');
        self.writer
            .write_all(&line)
            .map_err(|error| format!("could not append complete R12 record: {error}"))?;
        self.writer
            .flush()
            .map_err(|error| format!("could not flush appended R12 record: {error}"))?;

        self.records_written = self
            .records_written
            .checked_add(1)
            .ok_or_else(|| "R12 record count overflow".to_owned())?;
        self.next_sequence = self
            .next_sequence
            .checked_add(1)
            .ok_or_else(|| "R12 record sequence overflow".to_owned())?;
        Ok(())
    }
}

pub fn canonical_route_id(route: &TwoLegRouteCandidate) -> String {
    format!(
        "anchor={}|intermediate={}|leg1={}:{}|leg2={}:{}",
        route.anchor_mint(),
        route.intermediate_mint(),
        route.leg_1().venue().label(),
        route.leg_1().pool_id(),
        route.leg_2().venue().label(),
        route.leg_2().pool_id(),
    )
}

pub fn canonical_candidate_id(route: &TwoLegRouteCandidate, usd_size: u64) -> String {
    format!("{}|usd={usd_size}", canonical_route_id(route))
}

pub fn validate_jsonl_replay(path: &Path) -> Result<(), String> {
    let file = File::open(path)
        .map_err(|error| format!("could not open R12 JSONL for replay: {error}"))?;
    let mut reader = BufReader::new(file);
    let mut line = String::new();
    let mut expected_sequence = 1u64;
    let mut expected_run_id: Option<String> = None;
    let mut event_count = 0u64;
    let mut evidence_count = 0u64;
    let mut saw_run_end = false;
    let mut lifecycle = BTreeMap::<String, LifecycleState>::new();

    loop {
        line.clear();
        let bytes = reader
            .read_line(&mut line)
            .map_err(|error| format!("could not read R12 JSONL replay line: {error}"))?;
        if bytes == 0 {
            break;
        }
        if !line.ends_with('\n') {
            return Err("R12 JSONL contains a non-newline-terminated record".to_owned());
        }

        let record: Value = serde_json::from_str(&line)
            .map_err(|error| format!("R12 JSONL contains malformed JSON: {error}"))?;
        validate_record_envelope(&record, expected_sequence, &mut expected_run_id)?;

        let event_type = required_str(&record, "event_type")?;
        let payload = record
            .get("payload")
            .ok_or_else(|| "R12 record missing payload".to_owned())?;

        match event_type {
            "run_start" => {
                if event_count != 0 {
                    return Err("R12 run_start must be the first record".to_owned());
                }
            }
            "route_rejection" => {
                validate_route_payload(payload)?;
                evidence_count = evidence_count
                    .checked_add(1)
                    .ok_or_else(|| "R12 replay evidence count overflow".to_owned())?;
            }
            "candidate_evaluation" => {
                validate_candidate_payload(payload, &mut lifecycle)?;
                evidence_count = evidence_count
                    .checked_add(1)
                    .ok_or_else(|| "R12 replay evidence count overflow".to_owned())?;
            }
            "run_end" => {
                if saw_run_end {
                    return Err("R12 JSONL contains multiple run_end records".to_owned());
                }
                if evidence_count == 0 {
                    return Err("R12 run_end appeared without evidence records".to_owned());
                }
                let recorded_evidence = required_u64(payload, "evidence_record_count")?;
                if recorded_evidence != evidence_count {
                    return Err(format!(
                        "R12 run_end evidence count mismatch: expected={evidence_count} actual={recorded_evidence}"
                    ));
                }
                saw_run_end = true;
            }
            other => {
                return Err(format!(
                    "R12 replay encountered unknown event type: {other}"
                ))
            }
        }

        if saw_run_end && event_type != "run_end" {
            return Err("R12 replay encountered evidence after run_end".to_owned());
        }

        event_count = event_count
            .checked_add(1)
            .ok_or_else(|| "R12 replay event count overflow".to_owned())?;
        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "R12 replay sequence overflow".to_owned())?;
    }

    if event_count == 0 {
        return Err("R12 JSONL replay found an empty file".to_owned());
    }
    if !saw_run_end {
        return Err("R12 JSONL replay did not find run_end".to_owned());
    }

    Ok(())
}

fn validate_record_envelope(
    record: &Value,
    expected_sequence: u64,
    expected_run_id: &mut Option<String>,
) -> Result<(), String> {
    let schema = required_str(record, "schema_version")?;
    if schema != SCHEMA_VERSION {
        return Err(format!(
            "R12 replay encountered unknown schema version: {schema}"
        ));
    }

    let run_id = required_str(record, "run_id")?;
    if run_id.trim().is_empty() {
        return Err("R12 replay encountered empty run_id".to_owned());
    }

    if let Some(expected) = expected_run_id.as_deref() {
        if run_id != expected {
            return Err(format!(
                "R12 replay run_id changed within file: expected={expected} actual={run_id}"
            ));
        }
    } else {
        *expected_run_id = Some(run_id.to_owned());
    }

    let sequence = required_u64(record, "record_sequence")?;
    if sequence != expected_sequence {
        return Err(format!(
            "R12 replay sequence mismatch: expected={expected_sequence} actual={sequence}"
        ));
    }

    required_u64(record, "observed_at_unix_ms")?;
    Ok(())
}

fn validate_route_payload(payload: &Value) -> Result<(), String> {
    let route = payload
        .get("route")
        .ok_or_else(|| "R12 route rejection missing route".to_owned())?;
    validate_route_value(route)?;
    let reason = required_str(payload, "reason")?;
    if reason.trim().is_empty() {
        return Err("R12 replay found empty route rejection reason".to_owned());
    }
    required_u64(payload, "candidate_found_at_unix_ms")?;
    Ok(())
}

fn validate_candidate_payload(
    payload: &Value,
    lifecycle: &mut BTreeMap<String, LifecycleState>,
) -> Result<(), String> {
    let route = payload
        .get("route")
        .ok_or_else(|| "R12 candidate record missing route".to_owned())?;
    validate_route_value(route)?;
    let usd_size = required_u64(payload, "usd_size")?;
    let candidate_id = required_str(payload, "candidate_id")?;
    let canonical = canonical_candidate_id_from_json(route, usd_size)?;
    if candidate_id != canonical {
        return Err(format!(
            "R12 candidate identity mismatch: expected={canonical} actual={candidate_id}"
        ));
    }

    let status = required_str(payload, "status")?;
    if !matches!(
        status,
        "quote_rejected"
            | "economics_unresolved"
            | "economics_resolved_nonpositive"
            | "economics_resolved_positive"
    ) {
        return Err(format!(
            "R12 replay encountered unknown candidate status: {status}"
        ));
    }

    let lifecycle_value = payload
        .get("lifecycle")
        .ok_or_else(|| "R12 candidate record missing lifecycle".to_owned())?;
    let first_seen = required_u64(lifecycle_value, "first_seen_at_unix_ms")?;
    let last_seen = required_u64(lifecycle_value, "last_seen_at_unix_ms")?;
    let observation_count = required_u64(lifecycle_value, "observation_count")?;
    let lifetime_ms = required_u64(lifecycle_value, "lifetime_ms")?;
    let expected_lifetime = last_seen
        .checked_sub(first_seen)
        .ok_or_else(|| "R12 replay lifecycle timestamp underflow".to_owned())?;
    if lifetime_ms != expected_lifetime {
        return Err("R12 replay lifecycle duration mismatch".to_owned());
    }

    let state = lifecycle
        .entry(candidate_id.to_owned())
        .or_insert(LifecycleState {
            first_seen_at_unix_ms: first_seen,
            last_seen_at_unix_ms: last_seen,
            observation_count: 0,
        });
    state.first_seen_at_unix_ms = state.first_seen_at_unix_ms.min(first_seen);
    state.last_seen_at_unix_ms = state.last_seen_at_unix_ms.max(last_seen);
    state.observation_count = state
        .observation_count
        .checked_add(1)
        .ok_or_else(|| "R12 replay lifecycle count overflow".to_owned())?;

    if state.first_seen_at_unix_ms != first_seen
        || state.last_seen_at_unix_ms != last_seen
        || state.observation_count != observation_count
    {
        return Err("R12 replay lifecycle aggregation mismatch".to_owned());
    }

    let timing = payload
        .get("timing")
        .ok_or_else(|| "R12 candidate record missing timing".to_owned())?;
    let candidate_found_at = required_u64(timing, "candidate_found_at_unix_ms")?;
    validate_optional_u64(timing, "quote_complete_at_unix_ms")?;
    validate_optional_u64(timing, "economics_complete_at_unix_ms")?;
    validate_optional_u64(timing, "hypothetical_ready_at_unix_ms")?;

    match status {
        "quote_rejected" => {
            let reason = payload
                .get("quote_rejection_reason")
                .and_then(Value::as_str)
                .ok_or_else(|| {
                    "R12 quote_rejected record missing exact rejection reason".to_owned()
                })?;
            if reason.trim().is_empty() {
                return Err("R12 quote_rejected record has empty rejection reason".to_owned());
            }
            if !payload.get("quote").is_some_and(Value::is_null) {
                return Err("R12 quote_rejected record unexpectedly contains a quote".to_owned());
            }
            if timing
                .get("economics_complete_at_unix_ms")
                .is_some_and(|value| !value.is_null())
                || timing
                    .get("hypothetical_ready_at_unix_ms")
                    .is_some_and(|value| !value.is_null())
            {
                return Err("R12 quote_rejected record contains economics/ready timing".to_owned());
            }
        }
        "economics_unresolved"
        | "economics_resolved_nonpositive"
        | "economics_resolved_positive" => {
            let quote_complete = timing
                .get("quote_complete_at_unix_ms")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    "R12 economics candidate missing quote_complete timestamp".to_owned()
                })?;
            let economics_complete = timing
                .get("economics_complete_at_unix_ms")
                .and_then(Value::as_u64)
                .ok_or_else(|| {
                    "R12 economics candidate missing economics_complete timestamp".to_owned()
                })?;
            if quote_complete < candidate_found_at || economics_complete < quote_complete {
                return Err("R12 candidate timing chronology is invalid".to_owned());
            }

            validate_quote_and_gross_delta(payload)?;
            if let Some(cost_model) = payload.get("cost_model").filter(|value| !value.is_null()) {
                validate_cost_model(cost_model)?;
            }
            validate_status_against_modes(payload, status)?;

            let ready = timing.get("hypothetical_ready_at_unix_ms");
            if status == "economics_resolved_positive" {
                if !ready.is_some_and(|value| !value.is_null()) {
                    return Err(
                        "R12 positive resolved candidate missing hypothetical_ready timestamp"
                            .to_owned(),
                    );
                }
                let ready_at = ready
                    .and_then(Value::as_u64)
                    .ok_or_else(|| "R12 hypothetical_ready timestamp must be u64".to_owned())?;
                if ready_at < economics_complete {
                    return Err("R12 hypothetical_ready precedes economics_complete".to_owned());
                }
            } else if ready.is_some_and(|value| !value.is_null()) {
                return Err(
                    "R12 non-positive or unresolved candidate has hypothetical_ready timestamp"
                        .to_owned(),
                );
            }
        }
        other => return Err(format!("R12 candidate status became unsupported: {other}")),
    }

    Ok(())
}

fn validate_optional_u64(value: &Value, field: &str) -> Result<(), String> {
    match value.get(field) {
        Some(Value::Null) => Ok(()),
        Some(value) if value.as_u64().is_some() => Ok(()),
        _ => Err(format!("R12 JSON field {field} must be null or u64")),
    }
}

fn validate_quote_and_gross_delta(payload: &Value) -> Result<(), String> {
    let quote = payload
        .get("quote")
        .filter(|value| !value.is_null())
        .ok_or_else(|| "R12 economics candidate missing quote".to_owned())?;
    let requested = required_u64(quote, "anchor_input_requested_raw")?;
    let output = required_u64(quote, "anchor_output_raw")?;
    required_u64(quote, "anchor_input_consumed_raw")?;
    required_u64(quote, "anchor_input_unspent_raw")?;
    validate_quote_leg(
        quote
            .get("leg_1")
            .ok_or_else(|| "R12 quote missing leg_1".to_owned())?,
    )?;
    validate_quote_leg(
        quote
            .get("leg_2")
            .ok_or_else(|| "R12 quote missing leg_2".to_owned())?,
    )?;

    let recorded = required_str(payload, "gross_delta_raw")?
        .parse::<i128>()
        .map_err(|error| format!("R12 gross_delta_raw is invalid i128: {error}"))?;
    let expected = i128::from(output)
        .checked_sub(i128::from(requested))
        .ok_or_else(|| "R12 replay gross delta overflow".to_owned())?;
    if recorded != expected {
        return Err(format!(
            "R12 replay gross delta mismatch: expected={expected} actual={recorded}"
        ));
    }
    Ok(())
}

fn validate_quote_leg(leg: &Value) -> Result<(), String> {
    required_str(leg, "venue")?;
    required_str(leg, "pool_id")?;
    required_u64(leg, "amount_in_requested_raw")?;
    required_u64(leg, "amount_in_consumed_raw")?;
    required_u64(leg, "amount_in_unspent_raw")?;
    required_u64(leg, "amount_out_raw")?;
    required_u64(leg, "quote_source_slot")?;
    if !leg.get("fees").is_some_and(Value::is_object) {
        return Err("R12 quote leg missing fee components".to_owned());
    }
    Ok(())
}

fn validate_cost_model(model: &Value) -> Result<(), String> {
    let basis_id = required_str(model, "basis_id")?;
    if basis_id.trim().is_empty() {
        return Err("R12 cost model has empty basis id".to_owned());
    }
    let common = model
        .get("common")
        .ok_or_else(|| "R12 cost model missing common costs".to_owned())?;
    for field in [
        "base_fee",
        "priority_fee",
        "submission_cost",
        "expected_failure_cost",
        "safety_reserve",
    ] {
        validate_required_cost(
            common
                .get(field)
                .ok_or_else(|| format!("R12 cost model missing {field}"))?,
        )?;
    }
    let treasury = model
        .pointer("/treasury/capital_cost")
        .ok_or_else(|| "R12 cost model missing treasury capital cost".to_owned())?;
    validate_required_cost(treasury)?;
    let flash = model
        .pointer("/flash/borrowing_cost")
        .ok_or_else(|| "R12 cost model missing flash borrowing cost".to_owned())?;
    validate_required_cost(flash)?;
    Ok(())
}

fn validate_required_cost(cost: &Value) -> Result<(), String> {
    match required_str(cost, "state")? {
        "known" => {
            required_u64(cost, "amount_anchor_raw")?;
            let kind = required_str(cost, "provenance_kind")?;
            if kind != "observed" && kind != "modeled_assumption" {
                return Err(format!(
                    "R12 known cost has invalid provenance kind: {kind}"
                ));
            }
            let provenance = required_str(cost, "provenance")?;
            if provenance.trim().is_empty() {
                return Err("R12 known cost has empty provenance".to_owned());
            }
        }
        "unknown" => {
            if cost.get("amount_anchor_raw").is_some() {
                return Err("R12 unknown cost must not contain amount_anchor_raw".to_owned());
            }
            let kind = required_str(cost, "provenance_kind")?;
            if kind != "observed" && kind != "modeled_assumption" {
                return Err(format!(
                    "R12 unknown cost has invalid provenance kind: {kind}"
                ));
            }
            let reason = required_str(cost, "reason")?;
            if reason.trim().is_empty() {
                return Err("R12 unknown cost has empty reason".to_owned());
            }
        }
        other => return Err(format!("R12 cost has unknown state: {other}")),
    }
    Ok(())
}

fn validate_status_against_modes(payload: &Value, status: &str) -> Result<(), String> {
    let treasury = payload
        .get("treasury_evaluation")
        .ok_or_else(|| "R12 candidate missing treasury evaluation".to_owned())?;
    let flash = payload
        .get("flash_evaluation")
        .ok_or_else(|| "R12 candidate missing flash evaluation".to_owned())?;

    let treasury_resolved = treasury
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "resolved");
    let flash_resolved = flash
        .get("state")
        .and_then(Value::as_str)
        .is_some_and(|state| state == "resolved");

    if treasury_resolved && flash_resolved {
        validate_resolved_mode(payload, "treasury_evaluation")?;
        validate_resolved_mode(payload, "flash_evaluation")?;
        let positive = treasury
            .get("positive")
            .and_then(Value::as_bool)
            .ok_or_else(|| "R12 resolved treasury evaluation missing positive flag".to_owned())?
            || flash
                .get("positive")
                .and_then(Value::as_bool)
                .ok_or_else(|| "R12 resolved flash evaluation missing positive flag".to_owned())?;
        let expected = if positive {
            "economics_resolved_positive"
        } else {
            "economics_resolved_nonpositive"
        };
        if status != expected {
            return Err(format!(
                "R12 candidate status mismatch: expected={expected} actual={status}"
            ));
        }
    } else if status != "economics_unresolved" {
        return Err(format!(
            "R12 candidate must be economics_unresolved unless both funding modes resolve: actual={status}"
        ));
    }
    Ok(())
}

fn validate_resolved_mode(payload: &Value, field: &str) -> Result<(), String> {
    let value = payload
        .get(field)
        .ok_or_else(|| format!("R12 resolved candidate missing {field}"))?;
    if required_str(value, "state")? != "resolved" {
        return Err(format!("R12 resolved candidate has unresolved {field}"));
    }
    required_str(value, "expected_net_raw")?
        .parse::<i128>()
        .map_err(|error| format!("R12 {field} expected_net_raw is invalid i128: {error}"))?;
    Ok(())
}

fn validate_route_value(route: &Value) -> Result<(), String> {
    let route_id = required_str(route, "route_id")?;
    let canonical = canonical_route_id_from_json(route)?;
    if route_id != canonical {
        return Err(format!(
            "R12 route identity mismatch: expected={canonical} actual={route_id}"
        ));
    }
    for field in ["leg_1", "leg_2"] {
        let leg = route
            .get(field)
            .ok_or_else(|| format!("R12 route missing {field}"))?;
        required_str(leg, "input_mint")?;
        required_str(leg, "output_mint")?;
        required_u64(leg, "source_slot")?;
        required_u64(leg, "account_update_received_at_unix_ms")?;
        required_u64(leg, "normalized_at_unix_ms")?;
    }
    Ok(())
}

fn canonical_route_id_from_json(route: &Value) -> Result<String, String> {
    let anchor = required_str(route, "anchor_mint")?;
    let intermediate = required_str(route, "intermediate_mint")?;
    let leg_1 = route
        .get("leg_1")
        .ok_or_else(|| "R12 route missing leg_1".to_owned())?;
    let leg_2 = route
        .get("leg_2")
        .ok_or_else(|| "R12 route missing leg_2".to_owned())?;
    let leg_1_venue = required_str(leg_1, "venue")?;
    let leg_1_pool = required_str(leg_1, "pool_id")?;
    let leg_2_venue = required_str(leg_2, "venue")?;
    let leg_2_pool = required_str(leg_2, "pool_id")?;

    Ok(format!(
        "anchor={anchor}|intermediate={intermediate}|leg1={leg_1_venue}:{leg_1_pool}|leg2={leg_2_venue}:{leg_2_pool}"
    ))
}

fn canonical_candidate_id_from_json(route: &Value, usd_size: u64) -> Result<String, String> {
    Ok(format!(
        "{}|usd={usd_size}",
        canonical_route_id_from_json(route)?
    ))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("R12 JSON field {field} must be a string"))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("R12 JSON field {field} must be a u64"))
}

fn classify_economics_status(
    treasury: Option<&Result<ExpectedNetEconomics, String>>,
    flash: Option<&Result<ExpectedNetEconomics, String>>,
) -> &'static str {
    match (treasury, flash) {
        (Some(Ok(treasury)), Some(Ok(flash))) => {
            if treasury.is_positive() || flash.is_positive() {
                "economics_resolved_positive"
            } else {
                "economics_resolved_nonpositive"
            }
        }
        _ => "economics_unresolved",
    }
}

fn funding_result_value(result: Option<&Result<ExpectedNetEconomics, String>>) -> Value {
    match result {
        Some(Ok(economics)) => json!({
            "state": "resolved",
            "funding_mode": economics.funding_mode.label(),
            "cost_basis_id": economics.cost_basis_id.as_str(),
            "anchor_mint": economics.anchor_mint.as_str(),
            "anchor_input_requested_raw": economics.anchor_input_requested_raw,
            "anchor_output_raw": economics.anchor_output_raw,
            "gross_delta_raw": economics.gross_delta_raw.to_string(),
            "common_cost_raw": economics.common_cost_raw,
            "funding_cost_raw": economics.funding_cost_raw,
            "total_external_cost_raw": economics.total_external_cost_raw,
            "expected_net_raw": economics.expected_net_raw.to_string(),
            "positive": economics.is_positive(),
        }),
        Some(Err(error)) => json!({
            "state": "unresolved",
            "error": error,
        }),
        None => Value::Null,
    }
}

fn cost_model_value(model: &EconomicsCostModel) -> Value {
    json!({
        "basis_id": model.basis_id(),
        "common": {
            "base_fee": required_cost_value(&model.common.base_fee),
            "priority_fee": required_cost_value(&model.common.priority_fee),
            "submission_cost": required_cost_value(&model.common.submission_cost),
            "expected_failure_cost": required_cost_value(&model.common.expected_failure_cost),
            "safety_reserve": required_cost_value(&model.common.safety_reserve),
        },
        "treasury": {
            "capital_cost": required_cost_value(&model.treasury.capital_cost),
        },
        "flash": {
            "borrowing_cost": required_cost_value(&model.flash.borrowing_cost),
        },
    })
}

fn required_cost_value(cost: &RequiredCost) -> Value {
    match cost {
        RequiredCost::Known(cost) => json!({
            "state": "known",
            "amount_anchor_raw": cost.amount_anchor_raw(),
            "provenance_kind": cost.provenance_kind().label(),
            "provenance": cost.provenance(),
        }),
        RequiredCost::Unknown {
            provenance_kind,
            reason,
        } => json!({
            "state": "unknown",
            "provenance_kind": provenance_kind.label(),
            "reason": reason,
        }),
    }
}

fn priority_state_value(state: &PriorityObservationState) -> Value {
    match state {
        PriorityObservationState::Available(observation) => {
            let selection = match costs::select_priority_fee(observation) {
                Ok(Some(selection)) => json!({
                    "state": "selected",
                    "selected_micro_lamports_per_cu": selection.selected_micro_lamports_per_cu,
                    "total_sample_count": selection.total_sample_count,
                    "positive_sample_count": selection.positive_sample_count,
                    "min_slot": selection.min_slot,
                    "max_slot": selection.max_slot,
                    "policy_id": selection.policy_id,
                }),
                Ok(None) => json!({
                    "state": "unresolved",
                    "reason": "no positive localized samples",
                }),
                Err(error) => json!({
                    "state": "rejected",
                    "reason": error,
                }),
            };

            json!({
                "state": "available",
                "scope_accounts": observation.scope_accounts.as_slice(),
                "scope_provenance": observation.scope_provenance.as_str(),
                "samples": observation.samples.iter().map(|sample| json!({
                    "slot": sample.slot,
                    "micro_lamports_per_cu": sample.micro_lamports_per_cu,
                })).collect::<Vec<_>>(),
                "selection": selection,
            })
        }
        PriorityObservationState::Unavailable(reason) => json!({
            "state": "unavailable",
            "reason": reason,
        }),
    }
}

fn jito_state_value(state: &JitoObservationState) -> Value {
    match state {
        JitoObservationState::Available(observation) => json!({
            "state": "available",
            "time": observation.time.as_str(),
            "landed_tips_25th_lamports": observation.landed_tips_25th_lamports,
            "landed_tips_50th_lamports": observation.landed_tips_50th_lamports,
            "landed_tips_75th_lamports": observation.landed_tips_75th_lamports,
            "landed_tips_95th_lamports": observation.landed_tips_95th_lamports,
            "landed_tips_99th_lamports": observation.landed_tips_99th_lamports,
            "ema_landed_tips_50th_lamports": observation.ema_landed_tips_50th_lamports,
            "role": "observed_market_telemetry_only",
        }),
        JitoObservationState::Unavailable(reason) => json!({
            "state": "unavailable",
            "reason": reason,
        }),
    }
}

fn pyth_bundle_value(
    sol_usd: &SolUsdPrice,
    usdc_usd: Option<&SolUsdPrice>,
    usdt_usd: Option<&SolUsdPrice>,
) -> Value {
    json!({
        "sol_usd": pyth_price_value(sol_usd),
        "usdc_usd": usdc_usd.map(pyth_price_value),
        "usdt_usd": usdt_usd.map(pyth_price_value),
    })
}

fn pyth_price_value(price: &SolUsdPrice) -> Value {
    json!({
        "price": price.price,
        "confidence": price.confidence,
        "exponent": price.exponent,
        "publish_time": price.publish_time,
        "posted_slot": price.posted_slot,
        "rpc_slot": price.rpc_slot,
    })
}

fn route_quote_value(quote: &TwoLegRouteQuote) -> Value {
    json!({
        "anchor_mint": quote.anchor_mint.as_str(),
        "intermediate_mint": quote.intermediate_mint.as_str(),
        "anchor_input_requested_raw": quote.anchor_input_requested_raw,
        "anchor_input_consumed_raw": quote.anchor_input_consumed_raw,
        "anchor_input_unspent_raw": quote.anchor_input_unspent_raw,
        "anchor_output_raw": quote.anchor_output_raw,
        "leg_1": venue_leg_quote_value(&quote.leg_1),
        "leg_2": venue_leg_quote_value(&quote.leg_2),
    })
}

fn venue_leg_quote_value(quote: &VenueLegQuote) -> Value {
    json!({
        "venue": quote.venue.label(),
        "pool_id": quote.pool_id.as_str(),
        "amount_in_requested_raw": quote.amount_in_requested_raw,
        "amount_in_consumed_raw": quote.amount_in_consumed_raw,
        "amount_in_unspent_raw": quote.amount_in_unspent_raw,
        "amount_out_raw": quote.amount_out_raw,
        "fees": venue_fee_value(&quote.fees),
        "quote_source_slot": quote.quote_source_slot,
    })
}

fn venue_fee_value(fees: &VenueFeeComponents) -> Value {
    match fees {
        VenueFeeComponents::RaydiumCpmm {
            trade_fee_raw,
            protocol_fee_raw,
            fund_fee_raw,
            creator_fee_raw,
        } => json!({
            "kind": "raydium_cpmm",
            "trade_fee_raw": trade_fee_raw,
            "protocol_fee_raw": protocol_fee_raw,
            "fund_fee_raw": fund_fee_raw,
            "creator_fee_raw": creator_fee_raw,
        }),
        VenueFeeComponents::PumpSwap {
            lp_fee_raw,
            protocol_fee_raw,
            creator_fee_raw,
        } => json!({
            "kind": "pumpswap",
            "lp_fee_raw": lp_fee_raw,
            "protocol_fee_raw": protocol_fee_raw,
            "creator_fee_raw": creator_fee_raw,
        }),
    }
}

fn candidate_timing_value(timing: CandidateTiming) -> Value {
    json!({
        "candidate_found_at_unix_ms": timing.candidate_found_at_unix_ms,
        "quote_complete_at_unix_ms": timing.quote_complete_at_unix_ms,
        "economics_complete_at_unix_ms": timing.economics_complete_at_unix_ms,
        "hypothetical_ready_at_unix_ms": timing.hypothetical_ready_at_unix_ms,
    })
}

fn lifecycle_value(lifecycle: LifecycleState) -> Result<Value, String> {
    Ok(json!({
        "first_seen_at_unix_ms": lifecycle.first_seen_at_unix_ms,
        "last_seen_at_unix_ms": lifecycle.last_seen_at_unix_ms,
        "observation_count": lifecycle.observation_count,
        "lifetime_ms": lifecycle.lifetime_ms()?,
    }))
}

fn env_nonempty(name: &str) -> Option<String> {
    env::var(name).ok().filter(|value| !value.trim().is_empty())
}

fn build_run_id(now_unix_ms: u64) -> String {
    let github_run = env_nonempty("GITHUB_RUN_ID").unwrap_or_else(|| "local".to_owned());
    let attempt = env_nonempty("GITHUB_RUN_ATTEMPT").unwrap_or_else(|| "0".to_owned());
    sanitize_component(&format!(
        "r12-{github_run}-{attempt}-{now_unix_ms}-{}",
        process::id()
    ))
}

fn sanitize_component(value: &str) -> String {
    value
        .chars()
        .map(|character| {
            if character.is_ascii_alphanumeric() || character == '-' || character == '_' {
                character
            } else {
                '_'
            }
        })
        .collect()
}

fn unix_time_ms_now() -> Result<u64, String> {
    let duration = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_err(|error| format!("system clock before Unix epoch: {error}"))?;
    u64::try_from(duration.as_millis())
        .map_err(|_| "Unix timestamp milliseconds exceeded u64".to_owned())
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::economics::{
        CommonEconomicsCosts, CostProvenanceKind, EconomicsCostModel, ExpectedNetEconomics,
        FlashFundingCosts, FundingMode, TreasuryFundingCosts,
    };
    use crate::route::{generate_two_leg_routes, WRAPPED_SOL_MINT};
    use scout_core::{NormalizedToken, PoolTradingState, QuoteReserveState, Venue};
    use std::fs;
    use std::sync::atomic::{AtomicU64, Ordering};

    static TEST_COUNTER: AtomicU64 = AtomicU64::new(0);
    const TEST_TOKEN: &str = "Token1111111111111111111111111111111111111";

    fn sample_pool(venue: Venue, pool_id: &str) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: pool_id.to_owned(),
            venue,
            program_id: format!("{}-program", venue.label()),
            source_slot: 100,
            token_a: NormalizedToken {
                mint: WRAPPED_SOL_MINT.to_owned(),
                vault: format!("{pool_id}-vault-a"),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: TEST_TOKEN.to_owned(),
                vault: format!("{pool_id}-vault-b"),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Available {
                token_a_raw: 1_000,
                token_b_raw: 2_000,
                source_slot: 100,
            },
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    fn route_fixture() -> Result<TwoLegRouteCandidate, String> {
        generate_two_leg_routes(&[
            sample_pool(Venue::RaydiumCpmm, "raydium-pool"),
            sample_pool(Venue::PumpSwap, "pumpswap-pool"),
        ])
        .into_iter()
        .next()
        .ok_or_else(|| "fixture must produce route".to_owned())
    }

    fn resolved(mode: FundingMode, expected_net_raw: i128) -> Result<ExpectedNetEconomics, String> {
        Ok(ExpectedNetEconomics {
            funding_mode: mode,
            cost_basis_id: "fixture".to_owned(),
            anchor_mint: WRAPPED_SOL_MINT.to_owned(),
            anchor_input_requested_raw: 1_000,
            anchor_output_raw: 1_100,
            gross_delta_raw: 100,
            common_cost_raw: 10,
            funding_cost_raw: 5,
            total_external_cost_raw: 15,
            expected_net_raw,
        })
    }

    fn temp_path(label: &str) -> PathBuf {
        let sequence = TEST_COUNTER.fetch_add(1, Ordering::Relaxed);
        env::temp_dir().join(format!(
            "solana-arb-scout-r12-{label}-{}-{sequence}.jsonl",
            process::id()
        ))
    }

    #[test]
    fn candidate_identity_is_stable_and_size_specific() -> Result<(), String> {
        let route = route_fixture()?;
        let one = canonical_candidate_id(&route, 1);
        let five = canonical_candidate_id(&route, 5);
        assert_eq!(one, canonical_candidate_id(&route, 1));
        assert_ne!(one, five);
        assert!(one.contains("leg1="));
        assert!(one.contains("leg2="));
        Ok(())
    }

    #[test]
    fn status_requires_both_funding_modes_to_resolve() {
        let treasury = resolved(FundingMode::Treasury, 10);
        let flash = resolved(FundingMode::Flash, -1);
        let unresolved: Result<ExpectedNetEconomics, String> = Err("unknown cost".to_owned());

        assert_eq!(
            classify_economics_status(Some(&treasury), Some(&flash)),
            "economics_resolved_positive"
        );
        assert_eq!(
            classify_economics_status(Some(&treasury), Some(&unresolved)),
            "economics_unresolved"
        );
        assert_eq!(
            classify_economics_status(None, Some(&flash)),
            "economics_unresolved"
        );
    }

    #[test]
    fn replay_envelope_rejects_sequence_gap() -> Result<(), String> {
        let record = json!({
            "schema_version": SCHEMA_VERSION,
            "event_type": "run_start",
            "run_id": "fixture-run",
            "record_sequence": 3,
            "observed_at_unix_ms": 1,
            "payload": {},
        });
        let mut run_id = None;
        let result = validate_record_envelope(&record, 2, &mut run_id);
        assert!(matches!(result, Err(error) if error.contains("sequence mismatch")));
        Ok(())
    }

    #[test]
    fn replay_candidate_rejects_identity_mismatch() -> Result<(), String> {
        let route = json!({
            "route_id": "anchor=a|intermediate=b|leg1=raydium_cpmm:p1|leg2=pumpswap:p2",
            "anchor_mint": "a",
            "intermediate_mint": "b",
            "leg_1": {
                "venue": "raydium_cpmm",
                "pool_id": "p1",
                "input_mint": "a",
                "output_mint": "b",
                "source_slot": 1,
                "account_update_received_at_unix_ms": 1,
                "normalized_at_unix_ms": 2,
            },
            "leg_2": {
                "venue": "pumpswap",
                "pool_id": "p2",
                "input_mint": "b",
                "output_mint": "a",
                "source_slot": 1,
                "account_update_received_at_unix_ms": 1,
                "normalized_at_unix_ms": 2,
            },
        });
        let payload = json!({
            "candidate_id": "wrong",
            "status": "quote_rejected",
            "route": route,
            "usd_size": 1,
        });
        let mut lifecycle = BTreeMap::new();
        let result = validate_candidate_payload(&payload, &mut lifecycle);
        assert!(matches!(result, Err(error) if error.contains("identity mismatch")));
        Ok(())
    }

    #[test]
    fn known_and_unknown_costs_preserve_distinct_shapes() -> Result<(), String> {
        let known = RequiredCost::known(7, CostProvenanceKind::Observed, "fixture observed")?;
        let unknown =
            RequiredCost::unknown(CostProvenanceKind::ModeledAssumption, "fixture unresolved")?;
        let known_json = required_cost_value(&known);
        let unknown_json = required_cost_value(&unknown);

        assert_eq!(known_json["state"], "known");
        assert_eq!(known_json["amount_anchor_raw"], 7);
        assert_eq!(unknown_json["state"], "unknown");
        assert!(unknown_json.get("amount_anchor_raw").is_none());
        assert_eq!(unknown_json["reason"], "fixture unresolved");
        Ok(())
    }

    #[test]
    fn lifecycle_uses_first_last_and_count_without_fabrication() -> Result<(), String> {
        let mut lifecycle = LifecycleState {
            first_seen_at_unix_ms: 1_000,
            last_seen_at_unix_ms: 1_000,
            observation_count: 0,
        };
        lifecycle.observe(1_000)?;
        lifecycle.observe(1_250)?;
        assert_eq!(lifecycle.first_seen_at_unix_ms, 1_000);
        assert_eq!(lifecycle.last_seen_at_unix_ms, 1_250);
        assert_eq!(lifecycle.observation_count, 2);
        assert_eq!(lifecycle.lifetime_ms()?, 250);
        Ok(())
    }

    fn usd_price() -> SolUsdPrice {
        SolUsdPrice {
            price: 10_000_000_000,
            confidence: 1_000,
            exponent: -8,
            publish_time: 1_700_000_000,
            posted_slot: 100,
            rpc_slot: 100,
        }
    }

    fn quote_fixture(route: &TwoLegRouteCandidate) -> TwoLegRouteQuote {
        let leg_1 = VenueLegQuote {
            venue: route.leg_1().venue(),
            pool_id: route.leg_1().pool_id().to_owned(),
            amount_in_requested_raw: 1_000,
            amount_in_consumed_raw: 1_000,
            amount_in_unspent_raw: 0,
            amount_out_raw: 2_000,
            fees: VenueFeeComponents::RaydiumCpmm {
                trade_fee_raw: 5,
                protocol_fee_raw: 1,
                fund_fee_raw: 1,
                creator_fee_raw: 0,
            },
            quote_source_slot: 100,
        };
        let leg_2 = VenueLegQuote {
            venue: route.leg_2().venue(),
            pool_id: route.leg_2().pool_id().to_owned(),
            amount_in_requested_raw: 2_000,
            amount_in_consumed_raw: 2_000,
            amount_in_unspent_raw: 0,
            amount_out_raw: 1_100,
            fees: VenueFeeComponents::PumpSwap {
                lp_fee_raw: 5,
                protocol_fee_raw: 1,
                creator_fee_raw: 0,
            },
            quote_source_slot: 100,
        };
        TwoLegRouteQuote {
            anchor_mint: route.anchor_mint().to_owned(),
            intermediate_mint: route.intermediate_mint().to_owned(),
            anchor_input_requested_raw: 1_000,
            anchor_input_consumed_raw: 1_000,
            anchor_input_unspent_raw: 0,
            anchor_output_raw: 1_100,
            leg_1,
            leg_2,
        }
    }

    fn cost_model_fixture() -> Result<EconomicsCostModel, String> {
        EconomicsCostModel::new(
            "r12-replay-fixture",
            CommonEconomicsCosts {
                base_fee: RequiredCost::known(
                    5,
                    CostProvenanceKind::Observed,
                    "fixture observed base fee",
                )?,
                priority_fee: RequiredCost::known(
                    7,
                    CostProvenanceKind::ModeledAssumption,
                    "fixture modeled priority fee",
                )?,
                submission_cost: RequiredCost::unknown(
                    CostProvenanceKind::ModeledAssumption,
                    "fixture submission policy unresolved",
                )?,
                expected_failure_cost: RequiredCost::unknown(
                    CostProvenanceKind::ModeledAssumption,
                    "fixture expected failure unresolved",
                )?,
                safety_reserve: RequiredCost::unknown(
                    CostProvenanceKind::ModeledAssumption,
                    "fixture safety reserve unresolved",
                )?,
            },
            TreasuryFundingCosts {
                capital_cost: RequiredCost::unknown(
                    CostProvenanceKind::ModeledAssumption,
                    "fixture treasury capital unresolved",
                )?,
            },
            FlashFundingCosts {
                borrowing_cost: RequiredCost::known(
                    0,
                    CostProvenanceKind::ModeledAssumption,
                    "fixture provider fee zero",
                )?,
            },
        )
    }

    #[test]
    fn recorder_writes_and_replays_self_contained_unresolved_economics() -> Result<(), String> {
        let directory = env::temp_dir().join(format!(
            "solana-arb-scout-r12-dir-{}-{}",
            process::id(),
            TEST_COUNTER.fetch_add(1, Ordering::Relaxed)
        ));
        let pools = vec![
            sample_pool(Venue::RaydiumCpmm, "raydium-pool"),
            sample_pool(Venue::PumpSwap, "pumpswap-pool"),
        ];
        let routes = generate_two_leg_routes(&pools);
        let route = routes
            .first()
            .ok_or_else(|| "fixture route missing".to_owned())?;
        let quote = quote_fixture(route);
        let model = cost_model_fixture()?;
        let treasury =
            crate::economics::evaluate_expected_net_for_mode(&quote, &model, FundingMode::Treasury);
        let flash =
            crate::economics::evaluate_expected_net_for_mode(&quote, &model, FundingMode::Flash);
        assert!(treasury.is_err());
        assert!(flash.is_err());

        let price = usd_price();
        let mut recorder = ShadowRecorder::start_in_directory(
            &directory,
            &pools,
            routes.len(),
            &[1, 5],
            &price,
            None,
            None,
        )?;
        let status = recorder.record_economics_evaluation(
            route,
            1,
            9,
            &quote,
            Some(&model),
            None,
            Some(&treasury),
            Some(&flash),
            &PriorityObservationState::Unavailable("fixture priority unavailable".to_owned()),
            &JitoObservationState::Unavailable("fixture Jito unavailable".to_owned()),
            CandidateTiming {
                candidate_found_at_unix_ms: 2_000,
                quote_complete_at_unix_ms: Some(2_001),
                economics_complete_at_unix_ms: Some(2_002),
                hypothetical_ready_at_unix_ms: None,
            },
            &price,
            None,
            None,
        )?;
        assert_eq!(status, "economics_unresolved");
        let path = recorder.finish()?;
        validate_jsonl_replay(&path)?;
        let text = fs::read_to_string(&path).map_err(|error| error.to_string())?;
        assert!(text.contains("fixture observed base fee"));
        assert!(text.contains("fixture submission policy unresolved"));
        assert!(text.contains("\"state\":\"unknown\""));
        assert!(text.contains("\"amount_anchor_raw\":0"));
        let _ = fs::remove_file(&path);
        let _ = fs::remove_dir(&directory);
        Ok(())
    }

    #[test]
    fn replay_rejects_malformed_json() -> Result<(), String> {
        let path = temp_path("malformed");
        fs::write(&path, "{not-json}\n").map_err(|error| error.to_string())?;
        let result = validate_jsonl_replay(&path);
        let _ = fs::remove_file(&path);
        assert!(matches!(result, Err(error) if error.contains("malformed JSON")));
        Ok(())
    }

    #[test]
    fn replay_rejects_truncated_non_newline_record() -> Result<(), String> {
        let path = temp_path("truncated");
        fs::write(
            &path,
            format!(
                "{{\"schema_version\":\"{}\",\"event_type\":\"run_start\",\"run_id\":\"x\",\"record_sequence\":1,\"observed_at_unix_ms\":1,\"payload\":{{}}}}",
                SCHEMA_VERSION
            ),
        )
        .map_err(|error| error.to_string())?;
        let result = validate_jsonl_replay(&path);
        let _ = fs::remove_file(&path);
        assert!(matches!(result, Err(error) if error.contains("non-newline-terminated")));
        Ok(())
    }

    #[test]
    fn replay_rejects_unknown_schema() -> Result<(), String> {
        let path = temp_path("schema");
        fs::write(
            &path,
            concat!(
                "{\"schema_version\":\"future\",\"event_type\":\"run_start\",",
                "\"run_id\":\"x\",\"record_sequence\":1,\"observed_at_unix_ms\":1,",
                "\"payload\":{}}\n"
            ),
        )
        .map_err(|error| error.to_string())?;
        let result = validate_jsonl_replay(&path);
        let _ = fs::remove_file(&path);
        assert!(matches!(result, Err(error) if error.contains("unknown schema")));
        Ok(())
    }
}
