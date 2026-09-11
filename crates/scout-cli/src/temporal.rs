use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};

pub const SCHEMA_VERSION: &str = "stage-f-temporal-v1";
pub const REQUIRED_SOURCE_RUNS: usize = 3;
pub const OUTPUT_DIRECTORY: &str = "artifacts/stage-f-temporal";
pub const OUTPUT_FILE_NAME: &str = "temporal-evidence.json";

const R12_SCHEMA_VERSION: &str = "r12-shadow-v1";

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalCandidate {
    pub candidate_id: String,
    pub first_seen_at_unix_ms: u64,
    pub last_seen_at_unix_ms: u64,
    pub observation_count: u64,
    pub source_run_count: u64,
    pub lifetime_ms: u64,
    pub quote_rejected_count: u64,
    pub economics_unresolved_count: u64,
    pub economics_resolved_nonpositive_count: u64,
    pub economics_resolved_positive_count: u64,
    pub source_run_ids: Vec<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TemporalEvidence {
    pub source_paths: Vec<PathBuf>,
    pub source_run_ids: Vec<String>,
    pub candidates: Vec<TemporalCandidate>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct CandidateAccumulator {
    first_seen_at_unix_ms: u64,
    last_seen_at_unix_ms: u64,
    observation_count: u64,
    source_run_ids: BTreeSet<String>,
    quote_rejected_count: u64,
    economics_unresolved_count: u64,
    economics_resolved_nonpositive_count: u64,
    economics_resolved_positive_count: u64,
}

impl CandidateAccumulator {
    fn new(observed_at_unix_ms: u64) -> Self {
        Self {
            first_seen_at_unix_ms: observed_at_unix_ms,
            last_seen_at_unix_ms: observed_at_unix_ms,
            observation_count: 0,
            source_run_ids: BTreeSet::new(),
            quote_rejected_count: 0,
            economics_unresolved_count: 0,
            economics_resolved_nonpositive_count: 0,
            economics_resolved_positive_count: 0,
        }
    }

    fn observe(
        &mut self,
        run_id: &str,
        observed_at_unix_ms: u64,
        status: &str,
    ) -> Result<(), String> {
        self.first_seen_at_unix_ms = self.first_seen_at_unix_ms.min(observed_at_unix_ms);
        self.last_seen_at_unix_ms = self.last_seen_at_unix_ms.max(observed_at_unix_ms);
        self.observation_count = self
            .observation_count
            .checked_add(1)
            .ok_or_else(|| "Stage F temporal observation count overflow".to_owned())?;
        self.source_run_ids.insert(run_id.to_owned());

        let counter = match status {
            "quote_rejected" => &mut self.quote_rejected_count,
            "economics_unresolved" => &mut self.economics_unresolved_count,
            "economics_resolved_nonpositive" => &mut self.economics_resolved_nonpositive_count,
            "economics_resolved_positive" => &mut self.economics_resolved_positive_count,
            other => {
                return Err(format!(
                    "Stage F temporal source has unsupported candidate status: {other}"
                ))
            }
        };

        *counter = counter
            .checked_add(1)
            .ok_or_else(|| "Stage F temporal status count overflow".to_owned())?;

        Ok(())
    }

    fn finish(self, candidate_id: String) -> Result<TemporalCandidate, String> {
        let lifetime_ms = self
            .last_seen_at_unix_ms
            .checked_sub(self.first_seen_at_unix_ms)
            .ok_or_else(|| "Stage F temporal lifecycle timestamp underflow".to_owned())?;

        let classified_count = [
            self.quote_rejected_count,
            self.economics_unresolved_count,
            self.economics_resolved_nonpositive_count,
            self.economics_resolved_positive_count,
        ]
        .into_iter()
        .try_fold(0u64, |total, count| {
            total
                .checked_add(count)
                .ok_or_else(|| "Stage F temporal classified-count overflow".to_owned())
        })?;

        if classified_count != self.observation_count {
            return Err(format!(
                "Stage F temporal status accounting mismatch for {candidate_id}: observations={} classified={classified_count}",
                self.observation_count
            ));
        }

        Ok(TemporalCandidate {
            candidate_id,
            first_seen_at_unix_ms: self.first_seen_at_unix_ms,
            last_seen_at_unix_ms: self.last_seen_at_unix_ms,
            observation_count: self.observation_count,
            source_run_count: u64::try_from(self.source_run_ids.len())
                .map_err(|_| "Stage F temporal source-run count exceeds u64".to_owned())?,
            lifetime_ms,
            quote_rejected_count: self.quote_rejected_count,
            economics_unresolved_count: self.economics_unresolved_count,
            economics_resolved_nonpositive_count: self.economics_resolved_nonpositive_count,
            economics_resolved_positive_count: self.economics_resolved_positive_count,
            source_run_ids: self.source_run_ids.into_iter().collect(),
        })
    }
}

#[derive(Debug)]
struct ParsedRun {
    path: PathBuf,
    run_id: String,
    candidates: Vec<ParsedCandidate>,
}

#[derive(Debug)]
struct ParsedCandidate {
    candidate_id: String,
    observed_at_unix_ms: u64,
    status: String,
}

pub fn aggregate_r12_runs(paths: &[PathBuf]) -> Result<TemporalEvidence, String> {
    if paths.len() != REQUIRED_SOURCE_RUNS {
        return Err(format!(
            "Stage F temporal evidence requires exactly {REQUIRED_SOURCE_RUNS} completed R12 runs; received {}",
            paths.len()
        ));
    }

    let mut parsed_runs = Vec::with_capacity(paths.len());
    let mut unique_run_ids = BTreeSet::new();

    for path in paths {
        let parsed = parse_completed_r12_run(path)?;
        if !unique_run_ids.insert(parsed.run_id.clone()) {
            return Err(format!(
                "Stage F temporal evidence received duplicate R12 run_id: {}",
                parsed.run_id
            ));
        }
        parsed_runs.push(parsed);
    }

    parsed_runs.sort_by(|left, right| left.run_id.cmp(&right.run_id));

    let mut candidates = BTreeMap::<String, CandidateAccumulator>::new();

    for run in &parsed_runs {
        for observation in &run.candidates {
            let state = candidates
                .entry(observation.candidate_id.clone())
                .or_insert_with(|| CandidateAccumulator::new(observation.observed_at_unix_ms));

            state.observe(
                &run.run_id,
                observation.observed_at_unix_ms,
                &observation.status,
            )?;
        }
    }

    if candidates.is_empty() {
        return Err("Stage F temporal evidence contains no candidate observations".to_owned());
    }

    let candidates = candidates
        .into_iter()
        .map(|(candidate_id, state)| state.finish(candidate_id))
        .collect::<Result<Vec<_>, _>>()?;

    Ok(TemporalEvidence {
        source_paths: parsed_runs.iter().map(|run| run.path.clone()).collect(),
        source_run_ids: parsed_runs.iter().map(|run| run.run_id.clone()).collect(),
        candidates,
    })
}

pub fn write_evidence(evidence: &TemporalEvidence, output_path: &Path) -> Result<(), String> {
    if evidence.source_run_ids.len() != REQUIRED_SOURCE_RUNS {
        return Err(format!(
            "Stage F temporal write requires exactly {REQUIRED_SOURCE_RUNS} source run ids"
        ));
    }

    if evidence.candidates.is_empty() {
        return Err("Stage F temporal write requires candidate evidence".to_owned());
    }

    if let Some(parent) = output_path.parent() {
        fs::create_dir_all(parent).map_err(|error| {
            format!(
                "could not create Stage F temporal output directory {}: {error}",
                parent.display()
            )
        })?;
    }

    let candidate_values = evidence
        .candidates
        .iter()
        .map(candidate_value)
        .collect::<Result<Vec<_>, _>>()?;

    let document = json!({
        "schema_version": SCHEMA_VERSION,
        "scope": "read_only_repeated_observation_evidence",
        "authority": "advisory evidence only; temporal observations do not influence route discovery, quote generation, economics decisions, signing, submission, or execution",
        "source_run_count": evidence.source_run_ids.len(),
        "source_run_ids": evidence.source_run_ids,
        "source_paths": evidence
            .source_paths
            .iter()
            .map(|path| path.display().to_string())
            .collect::<Vec<_>>(),
        "candidate_identity_count": evidence.candidates.len(),
        "candidates": candidate_values,
    });

    let mut bytes = serde_json::to_vec_pretty(&document)
        .map_err(|error| format!("could not serialize Stage F temporal evidence: {error}"))?;
    bytes.push(b'\n');

    fs::write(output_path, bytes).map_err(|error| {
        format!(
            "could not write Stage F temporal evidence {}: {error}",
            output_path.display()
        )
    })?;

    validate_evidence_file(output_path)
}

pub fn validate_evidence_file(path: &Path) -> Result<(), String> {
    let bytes = fs::read(path).map_err(|error| {
        format!(
            "could not read Stage F temporal evidence {}: {error}",
            path.display()
        )
    })?;

    if bytes.is_empty() {
        return Err("Stage F temporal evidence is empty".to_owned());
    }

    if !bytes.ends_with(b"\n") {
        return Err("Stage F temporal evidence is not newline terminated".to_owned());
    }

    let document: Value = serde_json::from_slice(&bytes)
        .map_err(|error| format!("Stage F temporal evidence is invalid JSON: {error}"))?;

    if required_str(&document, "schema_version")? != SCHEMA_VERSION {
        return Err("Stage F temporal evidence schema mismatch".to_owned());
    }

    if required_u64(&document, "source_run_count")?
        != u64::try_from(REQUIRED_SOURCE_RUNS)
            .map_err(|_| "Stage F required source-run count exceeds u64".to_owned())?
    {
        return Err("Stage F temporal evidence source-run count mismatch".to_owned());
    }

    let source_run_ids = required_array(&document, "source_run_ids")?;
    if source_run_ids.len() != REQUIRED_SOURCE_RUNS {
        return Err("Stage F temporal evidence source-run id count mismatch".to_owned());
    }

    let mut unique_run_ids = BTreeSet::new();
    for run_id in source_run_ids {
        let run_id = run_id
            .as_str()
            .ok_or_else(|| "Stage F source_run_ids must contain strings".to_owned())?;
        if run_id.trim().is_empty() {
            return Err("Stage F source_run_ids contains an empty id".to_owned());
        }
        if !unique_run_ids.insert(run_id) {
            return Err("Stage F source_run_ids contains a duplicate id".to_owned());
        }
    }

    let candidates = required_array(&document, "candidates")?;
    if candidates.is_empty() {
        return Err("Stage F temporal evidence contains no candidates".to_owned());
    }

    let expected_identity_count = required_u64(&document, "candidate_identity_count")?;
    let actual_identity_count = u64::try_from(candidates.len())
        .map_err(|_| "Stage F candidate identity count exceeds u64".to_owned())?;

    if expected_identity_count != actual_identity_count {
        return Err("Stage F temporal candidate identity count mismatch".to_owned());
    }

    let mut candidate_ids = BTreeSet::new();
    for candidate in candidates {
        validate_candidate_value(candidate, &mut candidate_ids)?;
    }

    Ok(())
}

fn parse_completed_r12_run(path: &Path) -> Result<ParsedRun, String> {
    let bytes = fs::read(path)
        .map_err(|error| format!("could not read R12 source {}: {error}", path.display()))?;

    if bytes.is_empty() {
        return Err(format!("R12 source is empty: {}", path.display()));
    }

    if !bytes.ends_with(b"\n") {
        return Err(format!(
            "R12 source is not newline terminated: {}",
            path.display()
        ));
    }

    let text = std::str::from_utf8(&bytes)
        .map_err(|error| format!("R12 source is not UTF-8 {}: {error}", path.display()))?;

    let mut expected_sequence = 1u64;
    let mut run_id: Option<String> = None;
    let mut saw_start = false;
    let mut saw_end = false;
    let mut candidates = Vec::new();

    for line in text.lines() {
        let record: Value = serde_json::from_str(line)
            .map_err(|error| format!("could not parse R12 source record: {error}"))?;

        if required_str(&record, "schema_version")? != R12_SCHEMA_VERSION {
            return Err(format!(
                "Stage F source schema is not {R12_SCHEMA_VERSION}: {}",
                path.display()
            ));
        }

        let sequence = required_u64(&record, "record_sequence")?;
        if sequence != expected_sequence {
            return Err(format!(
                "R12 source sequence mismatch in {}: expected={expected_sequence} actual={sequence}",
                path.display()
            ));
        }

        expected_sequence = expected_sequence
            .checked_add(1)
            .ok_or_else(|| "Stage F source sequence overflow".to_owned())?;

        let record_run_id = required_str(&record, "run_id")?;
        match run_id.as_deref() {
            None => run_id = Some(record_run_id.to_owned()),
            Some(expected) if expected == record_run_id => {}
            Some(_) => {
                return Err(format!(
                    "R12 source run_id changed within file: {}",
                    path.display()
                ))
            }
        }

        let event_type = required_str(&record, "event_type")?;
        match event_type {
            "run_start" => {
                if saw_start || saw_end || sequence != 1 {
                    return Err(format!(
                        "R12 source run_start lifecycle invalid: {}",
                        path.display()
                    ));
                }
                saw_start = true;
            }
            "candidate_evaluation" => {
                if !saw_start || saw_end {
                    return Err(format!(
                        "R12 candidate outside completed lifecycle: {}",
                        path.display()
                    ));
                }

                let payload = required_object(&record, "payload")?;
                let candidate_id = required_str(payload, "candidate_id")?.to_owned();
                let status = required_str(payload, "status")?.to_owned();
                let observed_at_unix_ms = required_u64(&record, "observed_at_unix_ms")?;

                validate_candidate_identity(payload, &candidate_id)?;

                candidates.push(ParsedCandidate {
                    candidate_id,
                    observed_at_unix_ms,
                    status,
                });
            }
            "route_rejection" => {
                if !saw_start || saw_end {
                    return Err(format!(
                        "R12 route rejection outside completed lifecycle: {}",
                        path.display()
                    ));
                }
            }
            "run_end" => {
                if !saw_start || saw_end {
                    return Err(format!(
                        "R12 source run_end lifecycle invalid: {}",
                        path.display()
                    ));
                }
                saw_end = true;
            }
            other => {
                return Err(format!(
                    "Stage F source contains unsupported R12 event type {other}"
                ))
            }
        }
    }

    if !saw_start || !saw_end {
        return Err(format!(
            "Stage F requires completed R12 source with run_start and run_end: {}",
            path.display()
        ));
    }

    if candidates.is_empty() {
        return Err(format!(
            "Stage F completed R12 source contains no candidates: {}",
            path.display()
        ));
    }

    Ok(ParsedRun {
        path: path.to_path_buf(),
        run_id: run_id.ok_or_else(|| "Stage F source run_id unavailable".to_owned())?,
        candidates,
    })
}

fn validate_candidate_identity(payload: &Value, candidate_id: &str) -> Result<(), String> {
    let route = required_object(payload, "route")?;
    let route_id = required_str(route, "route_id")?;
    let usd_size = required_u64(payload, "usd_size")?;
    let canonical = format!("{route_id}|usd={usd_size}");

    if candidate_id != canonical {
        return Err(format!(
            "Stage F candidate identity mismatch: expected={canonical} actual={candidate_id}"
        ));
    }

    Ok(())
}

fn candidate_value(candidate: &TemporalCandidate) -> Result<Value, String> {
    if candidate.observation_count == 0 {
        return Err(format!(
            "Stage F candidate has zero observations: {}",
            candidate.candidate_id
        ));
    }

    if candidate.source_run_count == 0 {
        return Err(format!(
            "Stage F candidate has zero source runs: {}",
            candidate.candidate_id
        ));
    }

    if candidate.source_run_count > candidate.observation_count {
        return Err(format!(
            "Stage F candidate source-run count exceeds observation count: {}",
            candidate.candidate_id
        ));
    }

    let expected_lifetime = candidate
        .last_seen_at_unix_ms
        .checked_sub(candidate.first_seen_at_unix_ms)
        .ok_or_else(|| "Stage F candidate lifecycle timestamp underflow".to_owned())?;

    if candidate.lifetime_ms != expected_lifetime {
        return Err(format!(
            "Stage F candidate lifetime mismatch: {}",
            candidate.candidate_id
        ));
    }

    Ok(json!({
        "candidate_id": candidate.candidate_id,
        "first_seen_at_unix_ms": candidate.first_seen_at_unix_ms,
        "last_seen_at_unix_ms": candidate.last_seen_at_unix_ms,
        "observation_count": candidate.observation_count,
        "source_run_count": candidate.source_run_count,
        "lifetime_ms": candidate.lifetime_ms,
        "source_run_ids": candidate.source_run_ids,
        "status_counts": {
            "quote_rejected": candidate.quote_rejected_count,
            "economics_unresolved": candidate.economics_unresolved_count,
            "economics_resolved_nonpositive": candidate.economics_resolved_nonpositive_count,
            "economics_resolved_positive": candidate.economics_resolved_positive_count,
        },
    }))
}

fn validate_candidate_value(
    candidate: &Value,
    candidate_ids: &mut BTreeSet<String>,
) -> Result<(), String> {
    let candidate_id = required_str(candidate, "candidate_id")?;
    if !candidate_ids.insert(candidate_id.to_owned()) {
        return Err(format!(
            "Stage F temporal evidence contains duplicate candidate id: {candidate_id}"
        ));
    }

    let first_seen = required_u64(candidate, "first_seen_at_unix_ms")?;
    let last_seen = required_u64(candidate, "last_seen_at_unix_ms")?;
    let observation_count = required_u64(candidate, "observation_count")?;
    let source_run_count = required_u64(candidate, "source_run_count")?;
    let lifetime_ms = required_u64(candidate, "lifetime_ms")?;

    if observation_count == 0 {
        return Err(format!(
            "Stage F candidate has zero observations: {candidate_id}"
        ));
    }

    if source_run_count == 0 || source_run_count > observation_count {
        return Err(format!(
            "Stage F candidate has invalid source-run count: {candidate_id}"
        ));
    }

    let expected_lifetime = last_seen
        .checked_sub(first_seen)
        .ok_or_else(|| "Stage F temporal lifecycle timestamp underflow".to_owned())?;

    if lifetime_ms != expected_lifetime {
        return Err(format!(
            "Stage F temporal lifetime mismatch for {candidate_id}"
        ));
    }

    let source_run_ids = required_array(candidate, "source_run_ids")?;
    let source_run_id_count = u64::try_from(source_run_ids.len())
        .map_err(|_| "Stage F candidate source-run id count exceeds u64".to_owned())?;

    if source_run_id_count != source_run_count {
        return Err(format!(
            "Stage F candidate source-run id count mismatch for {candidate_id}"
        ));
    }

    let mut unique_run_ids = BTreeSet::new();
    for run_id in source_run_ids {
        let run_id = run_id
            .as_str()
            .ok_or_else(|| "Stage F candidate source_run_ids must contain strings".to_owned())?;
        if !unique_run_ids.insert(run_id) {
            return Err(format!(
                "Stage F candidate contains duplicate source run id: {candidate_id}"
            ));
        }
    }

    let status_counts = required_object(candidate, "status_counts")?;
    let classified_count = [
        required_u64(status_counts, "quote_rejected")?,
        required_u64(status_counts, "economics_unresolved")?,
        required_u64(status_counts, "economics_resolved_nonpositive")?,
        required_u64(status_counts, "economics_resolved_positive")?,
    ]
    .into_iter()
    .try_fold(0u64, |total, count| {
        total
            .checked_add(count)
            .ok_or_else(|| "Stage F temporal validation count overflow".to_owned())
    })?;

    if classified_count != observation_count {
        return Err(format!(
            "Stage F candidate status-count mismatch for {candidate_id}"
        ));
    }

    Ok(())
}

fn required_object<'a>(value: &'a Value, field: &str) -> Result<&'a Value, String> {
    let object = value
        .get(field)
        .ok_or_else(|| format!("Stage F JSON missing field {field}"))?;

    if !object.is_object() {
        return Err(format!("Stage F JSON field {field} must be an object"));
    }

    Ok(object)
}

fn required_array<'a>(value: &'a Value, field: &str) -> Result<&'a Vec<Value>, String> {
    value
        .get(field)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Stage F JSON field {field} must be an array"))
}

fn required_str<'a>(value: &'a Value, field: &str) -> Result<&'a str, String> {
    value
        .get(field)
        .and_then(Value::as_str)
        .ok_or_else(|| format!("Stage F JSON field {field} must be a string"))
}

fn required_u64(value: &Value, field: &str) -> Result<u64, String> {
    value
        .get(field)
        .and_then(Value::as_u64)
        .ok_or_else(|| format!("Stage F JSON field {field} must be a u64"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::fs::{create_dir_all, remove_dir_all, write};
    use std::time::{SystemTime, UNIX_EPOCH};

    fn temp_dir(label: &str) -> PathBuf {
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| duration.as_nanos())
            .unwrap_or_default();
        std::env::temp_dir().join(format!(
            "scout-stage-f-{label}-{}-{now}",
            std::process::id()
        ))
    }

    fn candidate_payload(candidate_id: &str, route_id: &str, usd_size: u64, status: &str) -> Value {
        json!({
            "candidate_id": candidate_id,
            "status": status,
            "usd_size": usd_size,
            "route": {
                "route_id": route_id
            }
        })
    }

    fn write_source(
        path: &Path,
        run_id: &str,
        observed_at_unix_ms: u64,
        status: &str,
    ) -> Result<(), String> {
        let route_id = "anchor=SOL|intermediate=USDC|leg1=raydium:pool-a|leg2=meteora:pool-b";
        let candidate_id = format!("{route_id}|usd=10");

        let records = [
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "run_start",
                "run_id": run_id,
                "record_sequence": 1,
                "observed_at_unix_ms": observed_at_unix_ms.saturating_sub(1),
                "payload": {}
            }),
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "candidate_evaluation",
                "run_id": run_id,
                "record_sequence": 2,
                "observed_at_unix_ms": observed_at_unix_ms,
                "payload": candidate_payload(&candidate_id, route_id, 10, status)
            }),
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "run_end",
                "run_id": run_id,
                "record_sequence": 3,
                "observed_at_unix_ms": observed_at_unix_ms.saturating_add(1),
                "payload": {}
            }),
        ];

        let mut bytes = Vec::new();
        for record in records {
            let mut line = serde_json::to_vec(&record)
                .map_err(|error| format!("test serialization failed: {error}"))?;
            line.push(b'\n');
            bytes.extend(line);
        }

        write(path, bytes).map_err(|error| format!("test source write failed: {error}"))
    }

    #[test]
    fn aggregates_cross_run_lifecycle_and_preserves_status_classes() -> Result<(), String> {
        let dir = temp_dir("aggregate");
        create_dir_all(&dir).map_err(|error| format!("test mkdir failed: {error}"))?;

        let paths = [
            dir.join("one.jsonl"),
            dir.join("two.jsonl"),
            dir.join("three.jsonl"),
        ];

        write_source(&paths[0], "run-a", 1_000, "economics_unresolved")?;
        write_source(&paths[1], "run-b", 1_500, "economics_resolved_nonpositive")?;
        write_source(&paths[2], "run-c", 2_250, "economics_resolved_positive")?;

        let evidence = aggregate_r12_runs(&paths)?;
        assert_eq!(evidence.source_run_ids, vec!["run-a", "run-b", "run-c"]);
        assert_eq!(evidence.candidates.len(), 1);

        let candidate = &evidence.candidates[0];
        assert_eq!(candidate.first_seen_at_unix_ms, 1_000);
        assert_eq!(candidate.last_seen_at_unix_ms, 2_250);
        assert_eq!(candidate.observation_count, 3);
        assert_eq!(candidate.source_run_count, 3);
        assert_eq!(candidate.lifetime_ms, 1_250);
        assert_eq!(candidate.quote_rejected_count, 0);
        assert_eq!(candidate.economics_unresolved_count, 1);
        assert_eq!(candidate.economics_resolved_nonpositive_count, 1);
        assert_eq!(candidate.economics_resolved_positive_count, 1);

        remove_dir_all(&dir).map_err(|error| format!("test cleanup failed: {error}"))?;
        Ok(())
    }

    #[test]
    fn rejects_duplicate_source_run_ids() -> Result<(), String> {
        let dir = temp_dir("duplicate-runs");
        create_dir_all(&dir).map_err(|error| format!("test mkdir failed: {error}"))?;

        let paths = [
            dir.join("one.jsonl"),
            dir.join("two.jsonl"),
            dir.join("three.jsonl"),
        ];

        write_source(&paths[0], "run-a", 1_000, "economics_unresolved")?;
        write_source(&paths[1], "run-a", 1_500, "economics_unresolved")?;
        write_source(&paths[2], "run-c", 2_000, "economics_unresolved")?;

        let error = match aggregate_r12_runs(&paths) {
            Ok(_) => return Err("duplicate run id unexpectedly succeeded".to_owned()),
            Err(error) => error,
        };
        assert!(error.contains("duplicate R12 run_id"));

        remove_dir_all(&dir).map_err(|cleanup_error| {
            format!("test cleanup failed after {error}: {cleanup_error}")
        })?;
        Ok(())
    }

    #[test]
    fn rejects_candidate_identity_mismatch() -> Result<(), String> {
        let dir = temp_dir("identity");
        create_dir_all(&dir).map_err(|error| format!("test mkdir failed: {error}"))?;

        let path = dir.join("bad.jsonl");
        let records = [
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "run_start",
                "run_id": "run-a",
                "record_sequence": 1,
                "observed_at_unix_ms": 999,
                "payload": {}
            }),
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "candidate_evaluation",
                "run_id": "run-a",
                "record_sequence": 2,
                "observed_at_unix_ms": 1_000,
                "payload": candidate_payload("wrong-id", "route-a", 10, "economics_unresolved")
            }),
            json!({
                "schema_version": R12_SCHEMA_VERSION,
                "event_type": "run_end",
                "run_id": "run-a",
                "record_sequence": 3,
                "observed_at_unix_ms": 1_001,
                "payload": {}
            }),
        ];

        let mut bytes = Vec::new();
        for record in records {
            let mut line = serde_json::to_vec(&record)
                .map_err(|error| format!("test serialization failed: {error}"))?;
            line.push(b'\n');
            bytes.extend(line);
        }
        write(&path, bytes).map_err(|error| format!("test source write failed: {error}"))?;

        let error = match parse_completed_r12_run(&path) {
            Ok(_) => return Err("identity mismatch unexpectedly succeeded".to_owned()),
            Err(error) => error,
        };
        assert!(error.contains("candidate identity mismatch"));

        remove_dir_all(&dir).map_err(|cleanup_error| {
            format!("test cleanup failed after {error}: {cleanup_error}")
        })?;
        Ok(())
    }

    #[test]
    fn evidence_round_trip_revalidates() -> Result<(), String> {
        let dir = temp_dir("round-trip");
        create_dir_all(&dir).map_err(|error| format!("test mkdir failed: {error}"))?;

        let paths = [
            dir.join("one.jsonl"),
            dir.join("two.jsonl"),
            dir.join("three.jsonl"),
        ];

        write_source(&paths[0], "run-a", 1_000, "quote_rejected")?;
        write_source(&paths[1], "run-b", 1_250, "economics_unresolved")?;
        write_source(&paths[2], "run-c", 1_500, "economics_resolved_nonpositive")?;

        let evidence = aggregate_r12_runs(&paths)?;
        let output = dir.join("temporal.json");
        write_evidence(&evidence, &output)?;
        validate_evidence_file(&output)?;

        remove_dir_all(&dir).map_err(|error| format!("test cleanup failed: {error}"))?;
        Ok(())
    }
}
