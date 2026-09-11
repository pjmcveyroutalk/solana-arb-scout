#[path = "../temporal.rs"]
mod temporal;

use std::env;
use std::path::{Path, PathBuf};

fn main() -> Result<(), String> {
    let source_paths = env::args_os().skip(1).map(PathBuf::from).collect::<Vec<_>>();

    if source_paths.len() != temporal::REQUIRED_SOURCE_RUNS {
        return Err(format!(
            "Stage F temporal runner requires exactly {} completed R12 paths; received {}",
            temporal::REQUIRED_SOURCE_RUNS,
            source_paths.len()
        ));
    }

    let evidence = temporal::aggregate_r12_runs(&source_paths)?;
    let output_path =
        Path::new(temporal::OUTPUT_DIRECTORY).join(temporal::OUTPUT_FILE_NAME);

    temporal::write_evidence(&evidence, &output_path)?;
    temporal::validate_evidence_file(&output_path)?;

    let repeated_candidate_count = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.source_run_count > 1)
        .count();

    let required_source_runs_u64 = u64::try_from(temporal::REQUIRED_SOURCE_RUNS)
        .map_err(|_| "Stage F required source-run count exceeds u64".to_owned())?;

    let three_run_candidate_count = evidence
        .candidates
        .iter()
        .filter(|candidate| candidate.source_run_count == required_source_runs_u64)
        .count();

    println!(
        "stage_f_temporal_summary: source_run_count={} candidate_identity_count={} repeated_candidate_count={} three_run_candidate_count={}",
        evidence.source_run_ids.len(),
        evidence.candidates.len(),
        repeated_candidate_count,
        three_run_candidate_count
    );
    println!(
        "stage_f_temporal_output_complete: {}",
        output_path.display()
    );
    println!("READ-ONLY STAGE F TEMPORAL EVIDENCE PASS");

    Ok(())
}
