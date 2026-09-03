use super::{BlockEvidence, BlockTransactionEvidence};
use crate::forensics::{CandidateEvidence, RouteAnalysis, TransactionMatch};
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RequestedAccountSource {
    Static,
    AltWritable,
    AltReadonly,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedAccount {
    pub address: String,
    pub source: RequestedAccountSource,
    pub signer: bool,
    pub writable: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedLockTransaction {
    pub block_index: usize,
    pub signature: String,
    pub accounts: Vec<RequestedAccount>,
    pub fee_lamports: u64,
    pub compute_units_consumed: Option<u64>,
    pub succeeded: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedLockOverlap {
    pub block_index: usize,
    pub signature: String,
    pub write_write: BTreeSet<String>,
    pub target_write_other_read: BTreeSet<String>,
    pub target_read_other_write: BTreeSet<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RequestedLockNeighborhood {
    pub slot: u64,
    pub target_signature: String,
    pub target_block_index: usize,
    pub target: RequestedLockTransaction,
    pub overlapping_transactions: Vec<RequestedLockOverlap>,
    pub nonoverlapping_transaction_count: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub struct OpportunityCohort {
    pub schema_version: &'static str,
    pub cohort_id: String,
    pub evidence: Value,
    pub reconstruction: Value,
    pub outcome: Value,
    pub interpretation: Value,
}

struct CaptureReconstruction {
    capture_status: &'static str,
    requested_lock_status: &'static str,
    value: Value,
}

#[allow(dead_code)]
pub fn build_opportunity_cohort(
    candidate: &CandidateEvidence,
    analysis: &RouteAnalysis,
    neighborhoods: &BTreeMap<String, RequestedLockNeighborhood>,
) -> Result<OpportunityCohort, String> {
    validate_candidate_identity(candidate)?;
    validate_route_analysis(candidate, analysis)?;

    let timing = build_timing_reconstruction(candidate)?;
    let economics_status = economics_completeness(&candidate.source_status)?;
    let capture = build_capture_reconstruction(candidate, analysis, neighborhoods)?;

    let mut unresolved_reasons = Vec::new();

    if economics_status != "ECONOMICS_COMPLETE" {
        unresolved_reasons.push(
            "R12 source status does not establish complete candidate economics".to_owned(),
        );
    }

    unresolved_reasons.push(
        "market correction time is absent from the current evidence interface, so Decision Margin cannot be derived"
            .to_owned(),
    );

    if capture.capture_status == "CAPTURE_INCOMPLETE" {
        unresolved_reasons
            .push("R13 capture search is incomplete for this candidate route".to_owned());
    }

    if capture.requested_lock_status == "INCOMPLETE" {
        unresolved_reasons.push(
            "one or more landed matched transactions lack validated requested-lock neighborhood evidence"
                .to_owned(),
        );
    }

    let cohort_id = format!(
        "{}:{}:{}",
        candidate.source_run_id, candidate.source_record_sequence, candidate.candidate_id
    );

    let evidence = json!({
        "source_r12": {
            "truth": "OBSERVED",
            "run_id": candidate.source_run_id,
            "record_sequence": candidate.source_record_sequence,
            "candidate_id": candidate.candidate_id,
            "status": candidate.source_status,
            "usd_size": candidate.usd_size,
            "route": candidate_route_value(candidate),
            "timing": {
                "candidate_found_at_unix_ms": candidate.candidate_found_at_unix_ms,
                "quote_complete_at_unix_ms": candidate.quote_complete_at_unix_ms,
                "economics_complete_at_unix_ms": candidate.economics_complete_at_unix_ms,
                "hypothetical_ready_at_unix_ms": candidate.hypothetical_ready_at_unix_ms,
            },
            "payload_policy": "R14B references the authoritative R12 record instead of duplicating the complete quote/economics payload",
        },
        "source_r13": {
            "truth": "DERIVED",
            "route_id": analysis.route_id,
            "status": analysis.status,
            "reason": analysis.reason,
            "matched_transaction_count": analysis.matches.len(),
            "payload_policy": "R14B consumes validated R13 route analysis and landed transaction evidence without changing R13 meaning",
        },
    });

    let reconstruction = json!({
        "timing": timing,
        "capture": capture.value,
    });

    let outcome = json!({
        "economics_status": economics_status,
        "timing_status": "TIMING_INCOMPLETE",
        "capture_status": capture.capture_status,
        "requested_lock_status": capture.requested_lock_status,
        "decision_eligible": false,
        "unresolved_reasons": unresolved_reasons,
    });

    let interpretation = json!({
        "truth": "UNKNOWN_NOT_OBSERVABLE",
        "causal_class": "unknown",
        "market_correction_at_unix_ms": Value::Null,
        "decision_margin_ms": Value::Null,
        "private_competitor_count": Value::Null,
        "exact_jito_auction_membership": Value::Null,
        "losing_private_bids": Value::Null,
        "searcher_intended_max_bid": Value::Null,
        "profitability_claim": "none",
        "niche_decision": "deferred_to_r14c",
        "reason": "R14B constructs deterministic evidence cohorts only; inference and the SELECT NICHE / REJECT CURRENT APPROACH / INSUFFICIENT EVIDENCE decision remain R14C responsibilities",
    });

    Ok(OpportunityCohort {
        schema_version: "r14-niche-v1",
        cohort_id,
        evidence,
        reconstruction,
        outcome,
        interpretation,
    })
}

fn validate_candidate_identity(candidate: &CandidateEvidence) -> Result<(), String> {
    if candidate.source_run_id.trim().is_empty() {
        return Err("R14B candidate source run_id must not be empty".to_owned());
    }

    if candidate.candidate_id.trim().is_empty() {
        return Err("R14B candidate_id must not be empty".to_owned());
    }

    if candidate.route.route_id.trim().is_empty() {
        return Err("R14B candidate route_id must not be empty".to_owned());
    }

    if candidate.source_record_sequence == 0 {
        return Err("R14B candidate source record sequence must be nonzero".to_owned());
    }

    Ok(())
}

fn validate_route_analysis(
    candidate: &CandidateEvidence,
    analysis: &RouteAnalysis,
) -> Result<(), String> {
    if analysis.route_id != candidate.route.route_id {
        return Err(format!(
            "R14B route identity mismatch: candidate={} analysis={}",
            candidate.route.route_id, analysis.route_id
        ));
    }

    match analysis.status.as_str() {
        "search_incomplete" | "no_atomic_match_complete" => {
            if !analysis.matches.is_empty() {
                return Err(format!(
                    "R14B status {} must not contain landed transaction matches",
                    analysis.status
                ));
            }
        }
        "atomic_route_match"
        | "atomic_route_amounts_unresolved"
        | "atomic_route_outcome_resolved" => {
            if analysis.matches.is_empty() {
                return Err(format!(
                    "R14B status {} requires at least one landed transaction match",
                    analysis.status
                ));
            }
        }
        other => {
            return Err(format!(
                "R14B unsupported R13 route analysis status {other}"
            ))
        }
    }

    if analysis.status == "atomic_route_outcome_resolved"
        && !analysis.matches.iter().any(|matched| matched.outcome_resolved)
    {
        return Err(
            "R14B atomic_route_outcome_resolved requires resolved transaction outcome evidence"
                .to_owned(),
        );
    }

    if analysis.status == "atomic_route_amounts_unresolved"
        && analysis.matches.iter().any(|matched| matched.outcome_resolved)
    {
        return Err(
            "R14B atomic_route_amounts_unresolved conflicts with resolved transaction outcome evidence"
                .to_owned(),
        );
    }

    let mut seen_signatures = BTreeSet::new();

    for matched in &analysis.matches {
        if matched.signature.trim().is_empty() {
            return Err("R14B landed transaction signature must not be empty".to_owned());
        }

        if !seen_signatures.insert(matched.signature.clone()) {
            return Err(format!(
                "R14B duplicate landed transaction signature {}",
                matched.signature
            ));
        }

        validate_transaction_match_window(candidate, matched)?;
    }

    Ok(())
}

fn validate_transaction_match_window(
    candidate: &CandidateEvidence,
    matched: &TransactionMatch,
) -> Result<(), String> {
    let start_slot = candidate.route.start_slot();
    let end_slot = candidate.route.end_slot()?;

    if matched.slot < start_slot || matched.slot > end_slot {
        return Err(format!(
            "R14B landed transaction slot outside R13 route window: signature={} slot={} start_slot={} end_slot={}",
            matched.signature, matched.slot, start_slot, end_slot
        ));
    }

    Ok(())
}

fn economics_completeness(status: &str) -> Result<&'static str, String> {
    match status {
        "economics_resolved_positive" | "economics_resolved_nonpositive" => {
            Ok("ECONOMICS_COMPLETE")
        }
        "economics_unresolved" | "quote_rejected" => Ok("ECONOMICS_INCOMPLETE"),
        other => Err(format!("R14B unsupported R12 candidate status {other}")),
    }
}

fn build_timing_reconstruction(candidate: &CandidateEvidence) -> Result<Value, String> {
    if candidate.economics_complete_at_unix_ms.is_some()
        && candidate.quote_complete_at_unix_ms.is_none()
    {
        return Err(
            "R14B economics completion timestamp cannot exist without quote completion".to_owned(),
        );
    }

    if candidate.hypothetical_ready_at_unix_ms.is_some()
        && candidate.economics_complete_at_unix_ms.is_none()
    {
        return Err(
            "R14B hypothetical-ready timestamp cannot exist without economics completion"
                .to_owned(),
        );
    }

    let quote_latency_ms = checked_elapsed_ms(
        candidate.candidate_found_at_unix_ms,
        candidate.quote_complete_at_unix_ms,
        "candidate_found_to_quote_complete",
    )?;

    let economics_latency_ms = checked_elapsed_ms(
        candidate.candidate_found_at_unix_ms,
        candidate.economics_complete_at_unix_ms,
        "candidate_found_to_economics_complete",
    )?;

    let hypothetical_ready_latency_ms = checked_elapsed_ms(
        candidate.candidate_found_at_unix_ms,
        candidate.hypothetical_ready_at_unix_ms,
        "candidate_found_to_hypothetical_ready",
    )?;

    let quote_to_economics_ms = match (
        candidate.quote_complete_at_unix_ms,
        candidate.economics_complete_at_unix_ms,
    ) {
        (Some(start), Some(end)) => Some(
            end.checked_sub(start).ok_or_else(|| {
                "R14B economics completion timestamp precedes quote completion".to_owned()
            })?,
        ),
        _ => None,
    };

    if let (Some(economics_complete), Some(hypothetical_ready)) = (
        candidate.economics_complete_at_unix_ms,
        candidate.hypothetical_ready_at_unix_ms,
    ) {
        hypothetical_ready
            .checked_sub(economics_complete)
            .ok_or_else(|| {
                "R14B hypothetical-ready timestamp precedes economics completion".to_owned()
            })?;
    }

    Ok(json!({
        "truth": "DERIVED",
        "clock_scope": "R12 local monotonic ordering represented as recorded Unix milliseconds within one Scout run",
        "candidate_found_at_unix_ms": {
            "truth": "OBSERVED",
            "value": candidate.candidate_found_at_unix_ms,
        },
        "quote_complete_at_unix_ms": {
            "truth": "OBSERVED",
            "value": candidate.quote_complete_at_unix_ms,
        },
        "economics_complete_at_unix_ms": {
            "truth": "OBSERVED",
            "value": candidate.economics_complete_at_unix_ms,
        },
        "hypothetical_ready_at_unix_ms": {
            "truth": "OBSERVED",
            "value": candidate.hypothetical_ready_at_unix_ms,
        },
        "quote_latency_ms": quote_latency_ms,
        "economics_latency_ms": economics_latency_ms,
        "quote_to_economics_ms": quote_to_economics_ms,
        "hypothetical_ready_latency_ms": hypothetical_ready_latency_ms,
        "market_correction_at_unix_ms": {
            "truth": "UNKNOWN_NOT_OBSERVABLE",
            "value": Value::Null,
            "reason": "current R12/R13 interface does not provide a same-clock market-correction timestamp",
        },
        "information_margin_ms": {
            "truth": "UNKNOWN_NOT_OBSERVABLE",
            "value": Value::Null,
            "reason": "market-correction time is unavailable",
        },
        "decision_margin_ms": {
            "truth": "UNKNOWN_NOT_OBSERVABLE",
            "value": Value::Null,
            "reason": "market-correction time is unavailable",
        },
        "timing_status": "TIMING_INCOMPLETE",
        "comparison_policy": "R12 local milliseconds are never directly compared to Solana blockTime",
    }))
}

fn checked_elapsed_ms(
    start: u64,
    end: Option<u64>,
    label: &str,
) -> Result<Option<u64>, String> {
    match end {
        None => Ok(None),
        Some(end) => end
            .checked_sub(start)
            .map(Some)
            .ok_or_else(|| format!("R14B {label} timestamp ordering is invalid")),
    }
}

fn build_capture_reconstruction(
    candidate: &CandidateEvidence,
    analysis: &RouteAnalysis,
    neighborhoods: &BTreeMap<String, RequestedLockNeighborhood>,
) -> Result<CaptureReconstruction, String> {
    let capture_status = match analysis.status.as_str() {
        "search_incomplete" => "CAPTURE_INCOMPLETE",
        "no_atomic_match_complete" => "CAPTURE_COMPLETE_NO_ATOMIC_MATCH",
        "atomic_route_match"
        | "atomic_route_amounts_unresolved"
        | "atomic_route_outcome_resolved" => "CAPTURE_COMPLETE_LANDED_MATCH",
        other => {
            return Err(format!(
                "R14B unsupported R13 capture analysis status {other}"
            ))
        }
    };

    let mut matches = Vec::new();
    let mut all_requested_lock_evidence_available = true;

    for matched in &analysis.matches {
        let requested_lock = match neighborhoods.get(&matched.signature) {
            Some(neighborhood) => {
                validate_requested_lock_neighborhood(matched, neighborhood)?;
                requested_lock_neighborhood_value(neighborhood)
            }
            None => {
                all_requested_lock_evidence_available = false;
                json!({
                    "truth": "UNKNOWN_NOT_OBSERVABLE",
                    "status": "unknown",
                    "reason": "validated requested-lock neighborhood was not supplied for this landed matched transaction",
                })
            }
        };

        matches.push(json!({
            "truth": "OBSERVED",
            "signature": matched.signature,
            "slot": matched.slot,
            "block_time": matched.block_time,
            "observed_capture_economics": {
                "truth": "OBSERVED",
                "network_fee_lamports": matched.fee_lamports,
                "compute_units_consumed": matched.compute_units_consumed,
                "jito_tip": {
                    "truth": "UNKNOWN_NOT_OBSERVABLE",
                    "lamports": Value::Null,
                    "reason": "R14B does not infer Jito tips from meta.fee",
                },
            },
            "route_leg_1": matched.leg_1,
            "route_leg_2": matched.leg_2,
            "amount_evidence": matched.amount_evidence,
            "outcome_resolved": matched.outcome_resolved,
            "requested_lock_neighborhood": requested_lock,
        }));
    }

    let requested_lock_status = if analysis.matches.is_empty() {
        "NOT_APPLICABLE_NO_MATCH"
    } else if all_requested_lock_evidence_available {
        "COMPLETE"
    } else {
        "INCOMPLETE"
    };

    let value = json!({
        "truth": "DERIVED",
        "route_id": candidate.route.route_id,
        "r13_status": analysis.status,
        "r13_reason": analysis.reason,
        "capture_status": capture_status,
        "matched_transaction_count": analysis.matches.len(),
        "matched_transactions": matches,
        "requested_lock_status": requested_lock_status,
        "requested_lock_semantics": "requested-message lock overlap only; runtime-exact lock conflict and Jito auction membership are not claimed",
        "no_match_semantics": "a complete bounded no-atomic-match result is evidence that no exact supported two-leg atomic match was proven in the R13 window; it is not zero contention and is not proof that no arbitrage activity existed",
    });

    Ok(CaptureReconstruction {
        capture_status,
        requested_lock_status,
        value,
    })
}

fn validate_requested_lock_neighborhood(
    matched: &TransactionMatch,
    neighborhood: &RequestedLockNeighborhood,
) -> Result<(), String> {
    if neighborhood.target_signature != matched.signature {
        return Err(format!(
            "R14B requested-lock target signature mismatch: match={} neighborhood={}",
            matched.signature, neighborhood.target_signature
        ));
    }

    if neighborhood.slot != matched.slot {
        return Err(format!(
            "R14B requested-lock slot mismatch: signature={} match_slot={} neighborhood_slot={}",
            matched.signature, matched.slot, neighborhood.slot
        ));
    }

    if neighborhood.target.signature != matched.signature {
        return Err(format!(
            "R14B requested-lock target transaction identity mismatch: match={} target={}",
            matched.signature, neighborhood.target.signature
        ));
    }

    if neighborhood.target.block_index != neighborhood.target_block_index {
        return Err(format!(
            "R14B requested-lock target block index mismatch: signature={} declared={} transaction={}",
            matched.signature, neighborhood.target_block_index, neighborhood.target.block_index
        ));
    }

    if neighborhood.target.fee_lamports != matched.fee_lamports {
        return Err(format!(
            "R14B requested-lock fee mismatch: signature={} r13_fee={} block_fee={}",
            matched.signature, matched.fee_lamports, neighborhood.target.fee_lamports
        ));
    }

    if neighborhood.target.compute_units_consumed != matched.compute_units_consumed {
        return Err(format!(
            "R14B requested-lock compute-unit mismatch: signature={} r13_cu={:?} block_cu={:?}",
            matched.signature,
            matched.compute_units_consumed,
            neighborhood.target.compute_units_consumed
        ));
    }

    Ok(())
}

fn requested_lock_neighborhood_value(neighborhood: &RequestedLockNeighborhood) -> Value {
    let overlaps = neighborhood
        .overlapping_transactions
        .iter()
        .map(|overlap| {
            let write_write = overlap.write_write.iter().cloned().collect::<Vec<_>>();
            let target_write_other_read = overlap
                .target_write_other_read
                .iter()
                .cloned()
                .collect::<Vec<_>>();
            let target_read_other_write = overlap
                .target_read_other_write
                .iter()
                .cloned()
                .collect::<Vec<_>>();

            json!({
                "block_index": overlap.block_index,
                "signature": overlap.signature,
                "write_write": write_write,
                "target_write_other_read": target_write_other_read,
                "target_read_other_write": target_read_other_write,
            })
        })
        .collect::<Vec<_>>();

    json!({
        "truth": "DERIVED",
        "status": "available",
        "semantic_scope": "observed landed requested-lock neighborhood; runtime-exact conflict and Jito auction membership are not claimed",
        "slot": neighborhood.slot,
        "target_signature": neighborhood.target_signature,
        "target_block_index": neighborhood.target_block_index,
        "target": {
            "fee_lamports": neighborhood.target.fee_lamports,
            "compute_units_consumed": neighborhood.target.compute_units_consumed,
            "succeeded": neighborhood.target.succeeded,
        },
        "overlapping_transaction_count": neighborhood.overlapping_transactions.len(),
        "nonoverlapping_transaction_count": neighborhood.nonoverlapping_transaction_count,
        "overlapping_transactions": overlaps,
    })
}

fn candidate_route_value(candidate: &CandidateEvidence) -> Value {
    json!({
        "route_id": candidate.route.route_id,
        "anchor_mint": candidate.route.anchor_mint,
        "intermediate_mint": candidate.route.intermediate_mint,
        "start_slot": candidate.route.start_slot(),
        "leg_1": {
            "venue": candidate.route.leg_1.venue,
            "pool_id": candidate.route.leg_1.pool_id,
            "input_mint": candidate.route.leg_1.input_mint,
            "output_mint": candidate.route.leg_1.output_mint,
            "source_slot": candidate.route.leg_1.source_slot,
        },
        "leg_2": {
            "venue": candidate.route.leg_2.venue,
            "pool_id": candidate.route.leg_2.pool_id,
            "input_mint": candidate.route.leg_2.input_mint,
            "output_mint": candidate.route.leg_2.output_mint,
            "source_slot": candidate.route.leg_2.source_slot,
        },
    })
}

#[allow(dead_code)]
pub fn analyze_requested_lock_neighborhood(
    block: &BlockEvidence,
) -> Result<RequestedLockNeighborhood, String> {
    let target_evidence = block
        .transactions
        .get(block.target_block_index)
        .ok_or_else(|| {
            format!(
                "R14 target block index out of range: index={} transaction_count={}",
                block.target_block_index,
                block.transactions.len()
            )
        })?;

    if target_evidence.signature != block.target_signature {
        return Err(format!(
            "R14 target identity mismatch: block_signature={} transaction_signature={}",
            block.target_signature, target_evidence.signature
        ));
    }

    let target = parse_requested_lock_transaction(target_evidence)?;
    let target_roles = requested_role_sets(&target.accounts);

    let mut overlapping_transactions = Vec::new();
    let mut nonoverlapping_transaction_count = 0usize;

    for transaction in &block.transactions {
        if transaction.block_index == block.target_block_index {
            continue;
        }

        let parsed = parse_requested_lock_transaction(transaction)?;
        let other_roles = requested_role_sets(&parsed.accounts);

        let write_write = intersection(&target_roles.writable, &other_roles.writable);
        let target_write_other_read = intersection(&target_roles.writable, &other_roles.readonly);
        let target_read_other_write = intersection(&target_roles.readonly, &other_roles.writable);

        if write_write.is_empty()
            && target_write_other_read.is_empty()
            && target_read_other_write.is_empty()
        {
            nonoverlapping_transaction_count = nonoverlapping_transaction_count
                .checked_add(1)
                .ok_or_else(|| "R14 nonoverlapping transaction count overflow".to_owned())?;
            continue;
        }

        overlapping_transactions.push(RequestedLockOverlap {
            block_index: parsed.block_index,
            signature: parsed.signature,
            write_write,
            target_write_other_read,
            target_read_other_write,
        });
    }

    Ok(RequestedLockNeighborhood {
        slot: block.slot,
        target_signature: block.target_signature.clone(),
        target_block_index: block.target_block_index,
        target,
        overlapping_transactions,
        nonoverlapping_transaction_count,
    })
}

fn parse_requested_lock_transaction(
    evidence: &BlockTransactionEvidence,
) -> Result<RequestedLockTransaction, String> {
    validate_supported_version(&evidence.value)?;

    let transaction = evidence.value.get("transaction").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing transaction object",
            evidence.block_index
        )
    })?;

    let message = transaction.get("message").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing message",
            evidence.block_index
        )
    })?;

    let header = message.get("header").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing message header",
            evidence.block_index
        )
    })?;

    let static_keys = message
        .get("accountKeys")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            format!(
                "R14 block transaction {} missing accountKeys",
                evidence.block_index
            )
        })?;

    let num_required_signatures =
        required_usize(header, "numRequiredSignatures", evidence.block_index)?;
    let num_readonly_signed_accounts =
        required_usize(header, "numReadonlySignedAccounts", evidence.block_index)?;
    let num_readonly_unsigned_accounts =
        required_usize(header, "numReadonlyUnsignedAccounts", evidence.block_index)?;

    if num_required_signatures > static_keys.len() {
        return Err(format!(
            "R14 block transaction {} requires more signers than static account keys",
            evidence.block_index
        ));
    }

    if num_readonly_signed_accounts > num_required_signatures {
        return Err(format!(
            "R14 block transaction {} readonly signed count exceeds signer count",
            evidence.block_index
        ));
    }

    let unsigned_count = static_keys.len() - num_required_signatures;
    if num_readonly_unsigned_accounts > unsigned_count {
        return Err(format!(
            "R14 block transaction {} readonly unsigned count exceeds unsigned account count",
            evidence.block_index
        ));
    }

    let writable_signed_count = num_required_signatures - num_readonly_signed_accounts;
    let writable_unsigned_count = unsigned_count - num_readonly_unsigned_accounts;
    let writable_unsigned_end = num_required_signatures + writable_unsigned_count;

    let mut accounts = Vec::new();

    for (index, value) in static_keys.iter().enumerate() {
        let address = value.as_str().ok_or_else(|| {
            format!(
                "R14 block transaction {} static account key {} was not a string",
                evidence.block_index, index
            )
        })?;

        let signer = index < num_required_signatures;
        let writable = if signer {
            index < writable_signed_count
        } else {
            index < writable_unsigned_end
        };

        accounts.push(RequestedAccount {
            address: address.to_owned(),
            source: RequestedAccountSource::Static,
            signer,
            writable,
        });
    }

    append_loaded_accounts(&mut accounts, evidence)?;

    let meta = evidence.value.get("meta").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing meta",
            evidence.block_index
        )
    })?;

    if meta.is_null() {
        return Err(format!(
            "R14 block transaction {} meta was null",
            evidence.block_index
        ));
    }

    let fee_lamports = meta.get("fee").and_then(Value::as_u64).ok_or_else(|| {
        format!(
            "R14 block transaction {} missing or invalid fee",
            evidence.block_index
        )
    })?;

    let compute_units_consumed = optional_u64(meta, "computeUnitsConsumed", evidence.block_index)?;

    let err = meta.get("err").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing meta.err",
            evidence.block_index
        )
    })?;

    Ok(RequestedLockTransaction {
        block_index: evidence.block_index,
        signature: evidence.signature.clone(),
        accounts,
        fee_lamports,
        compute_units_consumed,
        succeeded: err.is_null(),
    })
}

fn validate_supported_version(value: &Value) -> Result<(), String> {
    match value.get("version") {
        Some(Value::String(version)) if version == "legacy" => Ok(()),
        Some(Value::Number(version)) if version.as_u64() == Some(0) => Ok(()),
        Some(other) => Err(format!(
            "R14 unsupported transaction version in bounded block evidence: {other}"
        )),
        None => Err("R14 block transaction missing version".to_owned()),
    }
}

fn append_loaded_accounts(
    accounts: &mut Vec<RequestedAccount>,
    evidence: &BlockTransactionEvidence,
) -> Result<(), String> {
    let meta = evidence.value.get("meta").ok_or_else(|| {
        format!(
            "R14 block transaction {} missing meta",
            evidence.block_index
        )
    })?;

    let Some(loaded) = meta.get("loadedAddresses") else {
        return Ok(());
    };

    if loaded.is_null() {
        return Ok(());
    }

    append_loaded_account_array(
        accounts,
        loaded,
        "writable",
        RequestedAccountSource::AltWritable,
        true,
        evidence.block_index,
    )?;

    append_loaded_account_array(
        accounts,
        loaded,
        "readonly",
        RequestedAccountSource::AltReadonly,
        false,
        evidence.block_index,
    )?;

    Ok(())
}

fn append_loaded_account_array(
    accounts: &mut Vec<RequestedAccount>,
    loaded: &Value,
    field: &str,
    source: RequestedAccountSource,
    writable: bool,
    block_index: usize,
) -> Result<(), String> {
    let values = loaded.get(field).and_then(Value::as_array).ok_or_else(|| {
        format!("R14 block transaction {block_index} loadedAddresses.{field} missing or invalid")
    })?;

    for (index, value) in values.iter().enumerate() {
        let address = value.as_str().ok_or_else(|| {
            format!(
                "R14 block transaction {block_index} loadedAddresses.{field}[{index}] was not a string"
            )
        })?;

        accounts.push(RequestedAccount {
            address: address.to_owned(),
            source: source.clone(),
            signer: false,
            writable,
        });
    }

    Ok(())
}

#[derive(Debug)]
struct RequestedRoleSets {
    writable: BTreeSet<String>,
    readonly: BTreeSet<String>,
}

fn requested_role_sets(accounts: &[RequestedAccount]) -> RequestedRoleSets {
    let mut writable = BTreeSet::new();
    let mut readonly = BTreeSet::new();
    let mut merged: BTreeMap<String, bool> = BTreeMap::new();

    for account in accounts {
        merged
            .entry(account.address.clone())
            .and_modify(|current| *current |= account.writable)
            .or_insert(account.writable);
    }

    for (address, is_writable) in merged {
        if is_writable {
            writable.insert(address);
        } else {
            readonly.insert(address);
        }
    }

    RequestedRoleSets { writable, readonly }
}

fn intersection(left: &BTreeSet<String>, right: &BTreeSet<String>) -> BTreeSet<String> {
    left.intersection(right).cloned().collect()
}

fn required_usize(value: &Value, field: &str, block_index: usize) -> Result<usize, String> {
    let raw = value.get(field).and_then(Value::as_u64).ok_or_else(|| {
        format!("R14 block transaction {block_index} missing or invalid header field {field}")
    })?;

    usize::try_from(raw).map_err(|_| {
        format!("R14 block transaction {block_index} header field {field} exceeds usize")
    })
}

fn optional_u64(value: &Value, field: &str, block_index: usize) -> Result<Option<u64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other.as_u64().map(Some).ok_or_else(|| {
            format!("R14 block transaction {block_index} invalid optional u64 field {field}")
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::forensics::{LegEvidence, RouteEvidence};
    use serde_json::json;

    #[allow(clippy::too_many_arguments)]
    fn block_transaction(
        block_index: usize,
        signature: &str,
        account_keys: Vec<&str>,
        num_required_signatures: u64,
        num_readonly_signed_accounts: u64,
        num_readonly_unsigned_accounts: u64,
        loaded_writable: Vec<&str>,
        loaded_readonly: Vec<&str>,
    ) -> BlockTransactionEvidence {
        BlockTransactionEvidence {
            block_index,
            signature: signature.to_owned(),
            value: json!({
                "meta": {
                    "err": null,
                    "fee": 5_000,
                    "computeUnitsConsumed": 123_456,
                    "loadedAddresses": {
                        "writable": loaded_writable,
                        "readonly": loaded_readonly
                    }
                },
                "transaction": {
                    "message": {
                        "accountKeys": account_keys,
                        "header": {
                            "numRequiredSignatures": num_required_signatures,
                            "numReadonlySignedAccounts": num_readonly_signed_accounts,
                            "numReadonlyUnsignedAccounts": num_readonly_unsigned_accounts
                        },
                        "instructions": [],
                        "recentBlockhash": "recent-blockhash"
                    },
                    "signatures": [signature]
                },
                "version": 0
            }),
        }
    }

    fn legacy_block_transaction(
        block_index: usize,
        signature: &str,
        account_keys: Vec<&str>,
        num_required_signatures: u64,
        num_readonly_signed_accounts: u64,
        num_readonly_unsigned_accounts: u64,
    ) -> BlockTransactionEvidence {
        BlockTransactionEvidence {
            block_index,
            signature: signature.to_owned(),
            value: json!({
                "meta": {
                    "err": null,
                    "fee": 5_000
                },
                "transaction": {
                    "message": {
                        "accountKeys": account_keys,
                        "header": {
                            "numRequiredSignatures": num_required_signatures,
                            "numReadonlySignedAccounts": num_readonly_signed_accounts,
                            "numReadonlyUnsignedAccounts": num_readonly_unsigned_accounts
                        },
                        "instructions": [],
                        "recentBlockhash": "recent-blockhash"
                    },
                    "signatures": [signature]
                },
                "version": "legacy"
            }),
        }
    }

    fn candidate_evidence(status: &str) -> CandidateEvidence {
        CandidateEvidence {
            source_run_id: "r12-run".to_owned(),
            source_record_sequence: 7,
            candidate_id: "candidate-7".to_owned(),
            source_status: status.to_owned(),
            usd_size: 100,
            candidate_found_at_unix_ms: 1_000,
            quote_complete_at_unix_ms: Some(1_100),
            economics_complete_at_unix_ms: Some(1_200),
            hypothetical_ready_at_unix_ms: None,
            route: RouteEvidence {
                route_id: "route-1".to_owned(),
                anchor_mint: "anchor".to_owned(),
                intermediate_mint: "intermediate".to_owned(),
                leg_1: LegEvidence {
                    venue: "raydium_cpmm".to_owned(),
                    pool_id: "pool-a".to_owned(),
                    input_mint: "anchor".to_owned(),
                    output_mint: "intermediate".to_owned(),
                    source_slot: 100,
                },
                leg_2: LegEvidence {
                    venue: "pumpswap".to_owned(),
                    pool_id: "pool-b".to_owned(),
                    input_mint: "intermediate".to_owned(),
                    output_mint: "anchor".to_owned(),
                    source_slot: 101,
                },
            },
        }
    }

    fn transaction_match(signature: &str, slot: u64) -> TransactionMatch {
        TransactionMatch {
            signature: signature.to_owned(),
            slot,
            block_time: Some(1_700_000_000),
            fee_lamports: 5_000,
            compute_units_consumed: Some(123_456),
            leg_1: json!({"pool_id": "pool-a"}),
            leg_2: json!({"pool_id": "pool-b"}),
            amount_evidence: json!({"token_balance_snapshots_complete": true}),
            outcome_resolved: false,
        }
    }

    fn requested_lock_neighborhood(
        signature: &str,
        slot: u64,
        fee_lamports: u64,
        compute_units_consumed: Option<u64>,
    ) -> RequestedLockNeighborhood {
        RequestedLockNeighborhood {
            slot,
            target_signature: signature.to_owned(),
            target_block_index: 4,
            target: RequestedLockTransaction {
                block_index: 4,
                signature: signature.to_owned(),
                accounts: Vec::new(),
                fee_lamports,
                compute_units_consumed,
                succeeded: true,
            },
            overlapping_transactions: vec![RequestedLockOverlap {
                block_index: 5,
                signature: "neighbor".to_owned(),
                write_write: BTreeSet::from(["shared-write".to_owned()]),
                target_write_other_read: BTreeSet::new(),
                target_read_other_write: BTreeSet::new(),
            }],
            nonoverlapping_transaction_count: 8,
        }
    }

    #[test]
    fn complete_no_atomic_match_remains_complete_no_match_not_zero_contention(
    ) -> Result<(), String> {
        let candidate = candidate_evidence("economics_unresolved");
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "no_atomic_match_complete".to_owned(),
            reason: None,
            matches: Vec::new(),
        };

        let cohort = build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new())?;

        assert_eq!(
            cohort.outcome["capture_status"],
            json!("CAPTURE_COMPLETE_NO_ATOMIC_MATCH")
        );
        assert_eq!(
            cohort.outcome["requested_lock_status"],
            json!("NOT_APPLICABLE_NO_MATCH")
        );
        assert_eq!(
            cohort.outcome["economics_status"],
            json!("ECONOMICS_INCOMPLETE")
        );
        assert_eq!(cohort.outcome["decision_eligible"], json!(false));
        assert_eq!(
            cohort.reconstruction["timing"]["decision_margin_ms"]["value"],
            Value::Null
        );

        Ok(())
    }

    #[test]
    fn search_incomplete_remains_capture_incomplete() -> Result<(), String> {
        let candidate = candidate_evidence("economics_resolved_positive");
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "search_incomplete".to_owned(),
            reason: Some("bounded history unavailable".to_owned()),
            matches: Vec::new(),
        };

        let cohort = build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new())?;

        assert_eq!(
            cohort.outcome["capture_status"],
            json!("CAPTURE_INCOMPLETE")
        );
        assert_eq!(
            cohort.outcome["economics_status"],
            json!("ECONOMICS_COMPLETE")
        );
        assert_eq!(cohort.outcome["timing_status"], json!("TIMING_INCOMPLETE"));
        assert_eq!(cohort.outcome["decision_eligible"], json!(false));

        Ok(())
    }

    #[test]
    fn matched_transaction_attaches_validated_requested_lock_neighborhood(
    ) -> Result<(), String> {
        let candidate = candidate_evidence("economics_resolved_positive");
        let matched = transaction_match("matched", 110);
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "atomic_route_amounts_unresolved".to_owned(),
            reason: None,
            matches: vec![matched],
        };

        let mut neighborhoods = BTreeMap::new();
        neighborhoods.insert(
            "matched".to_owned(),
            requested_lock_neighborhood("matched", 110, 5_000, Some(123_456)),
        );

        let cohort = build_opportunity_cohort(&candidate, &analysis, &neighborhoods)?;

        assert_eq!(
            cohort.outcome["capture_status"],
            json!("CAPTURE_COMPLETE_LANDED_MATCH")
        );
        assert_eq!(
            cohort.outcome["requested_lock_status"],
            json!("COMPLETE")
        );
        assert_eq!(
            cohort.reconstruction["capture"]["matched_transactions"][0]
                ["requested_lock_neighborhood"]["status"],
            json!("available")
        );
        assert_eq!(
            cohort.reconstruction["capture"]["matched_transactions"][0]
                ["requested_lock_neighborhood"]["overlapping_transaction_count"],
            json!(1)
        );

        Ok(())
    }

    #[test]
    fn missing_requested_lock_neighborhood_is_explicitly_incomplete() -> Result<(), String> {
        let candidate = candidate_evidence("economics_resolved_positive");
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "atomic_route_amounts_unresolved".to_owned(),
            reason: None,
            matches: vec![transaction_match("matched", 110)],
        };

        let cohort = build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new())?;

        assert_eq!(
            cohort.outcome["requested_lock_status"],
            json!("INCOMPLETE")
        );
        assert_eq!(
            cohort.reconstruction["capture"]["matched_transactions"][0]
                ["requested_lock_neighborhood"]["status"],
            json!("unknown")
        );

        Ok(())
    }

    #[test]
    fn cohort_route_identity_mismatch_fails_closed() {
        let candidate = candidate_evidence("economics_unresolved");
        let analysis = RouteAnalysis {
            route_id: "different-route".to_owned(),
            status: "no_atomic_match_complete".to_owned(),
            reason: None,
            matches: Vec::new(),
        };

        assert!(build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new()).is_err());
    }

    #[test]
    fn nonmonotonic_candidate_timing_fails_closed() {
        let mut candidate = candidate_evidence("economics_unresolved");
        candidate.quote_complete_at_unix_ms = Some(999);

        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "no_atomic_match_complete".to_owned(),
            reason: None,
            matches: Vec::new(),
        };

        assert!(build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new()).is_err());
    }

    #[test]
    fn matched_transaction_outside_route_window_fails_closed() {
        let candidate = candidate_evidence("economics_resolved_positive");
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "atomic_route_amounts_unresolved".to_owned(),
            reason: None,
            matches: vec![transaction_match("matched", 200)],
        };

        assert!(build_opportunity_cohort(&candidate, &analysis, &BTreeMap::new()).is_err());
    }

    #[test]
    fn requested_lock_fee_mismatch_fails_closed() {
        let candidate = candidate_evidence("economics_resolved_positive");
        let analysis = RouteAnalysis {
            route_id: candidate.route.route_id.clone(),
            status: "atomic_route_amounts_unresolved".to_owned(),
            reason: None,
            matches: vec![transaction_match("matched", 110)],
        };

        let mut neighborhoods = BTreeMap::new();
        neighborhoods.insert(
            "matched".to_owned(),
            requested_lock_neighborhood("matched", 110, 9_999, Some(123_456)),
        );

        assert!(build_opportunity_cohort(&candidate, &analysis, &neighborhoods).is_err());
    }

    #[test]
    fn legacy_header_roles_are_derived_from_requested_message_metadata() -> Result<(), String> {
        let evidence = legacy_block_transaction(
            0,
            "target",
            vec![
                "signer-write",
                "signer-read",
                "unsigned-write",
                "unsigned-read",
            ],
            2,
            1,
            1,
        );

        let parsed = parse_requested_lock_transaction(&evidence)?;

        assert_eq!(parsed.accounts.len(), 4);
        assert!(parsed.accounts[0].signer);
        assert!(parsed.accounts[0].writable);
        assert!(parsed.accounts[1].signer);
        assert!(!parsed.accounts[1].writable);
        assert!(!parsed.accounts[2].signer);
        assert!(parsed.accounts[2].writable);
        assert!(!parsed.accounts[3].signer);
        assert!(!parsed.accounts[3].writable);

        Ok(())
    }

    #[test]
    fn version_zero_loaded_addresses_preserve_requested_roles() -> Result<(), String> {
        let evidence = block_transaction(
            0,
            "target",
            vec!["payer", "program"],
            1,
            0,
            1,
            vec!["alt-write"],
            vec!["alt-read"],
        );

        let parsed = parse_requested_lock_transaction(&evidence)?;

        assert_eq!(parsed.accounts.len(), 4);
        assert_eq!(
            parsed.accounts[2].source,
            RequestedAccountSource::AltWritable
        );
        assert!(parsed.accounts[2].writable);
        assert_eq!(
            parsed.accounts[3].source,
            RequestedAccountSource::AltReadonly
        );
        assert!(!parsed.accounts[3].writable);

        Ok(())
    }

    #[test]
    fn neighborhood_records_write_write_overlap() -> Result<(), String> {
        let target = block_transaction(
            0,
            "target",
            vec!["payer-a", "shared"],
            1,
            0,
            0,
            Vec::new(),
            Vec::new(),
        );

        let other = block_transaction(
            1,
            "other",
            vec!["payer-b", "shared"],
            1,
            0,
            0,
            Vec::new(),
            Vec::new(),
        );

        let block = test_block(vec![target, other], 0, "target");
        let neighborhood = analyze_requested_lock_neighborhood(&block)?;

        assert_eq!(neighborhood.overlapping_transactions.len(), 1);
        assert!(neighborhood.overlapping_transactions[0]
            .write_write
            .contains("shared"));

        Ok(())
    }

    #[test]
    fn neighborhood_records_target_write_other_read_overlap() -> Result<(), String> {
        let target = block_transaction(
            0,
            "target",
            vec!["payer-a", "shared"],
            1,
            0,
            0,
            Vec::new(),
            Vec::new(),
        );

        let other = block_transaction(
            1,
            "other",
            vec!["payer-b", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let block = test_block(vec![target, other], 0, "target");
        let neighborhood = analyze_requested_lock_neighborhood(&block)?;

        assert!(neighborhood.overlapping_transactions[0]
            .target_write_other_read
            .contains("shared"));

        Ok(())
    }

    #[test]
    fn neighborhood_records_target_read_other_write_overlap() -> Result<(), String> {
        let target = block_transaction(
            0,
            "target",
            vec!["payer-a", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let other = block_transaction(
            1,
            "other",
            vec!["payer-b", "shared"],
            1,
            0,
            0,
            Vec::new(),
            Vec::new(),
        );

        let block = test_block(vec![target, other], 0, "target");
        let neighborhood = analyze_requested_lock_neighborhood(&block)?;

        assert!(neighborhood.overlapping_transactions[0]
            .target_read_other_write
            .contains("shared"));

        Ok(())
    }

    #[test]
    fn readonly_readonly_only_is_not_requested_lock_overlap() -> Result<(), String> {
        let target = block_transaction(
            0,
            "target",
            vec!["payer-a", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let other = block_transaction(
            1,
            "other",
            vec!["payer-b", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let block = test_block(vec![target, other], 0, "target");
        let neighborhood = analyze_requested_lock_neighborhood(&block)?;

        assert!(neighborhood.overlapping_transactions.is_empty());
        assert_eq!(neighborhood.nonoverlapping_transaction_count, 1);

        Ok(())
    }

    #[test]
    fn duplicate_address_roles_merge_conservatively_to_writable() -> Result<(), String> {
        let target = block_transaction(
            0,
            "target",
            vec!["payer", "shared", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let other = block_transaction(
            1,
            "other",
            vec!["payer-b", "shared"],
            1,
            0,
            1,
            Vec::new(),
            Vec::new(),
        );

        let block = test_block(vec![target, other], 0, "target");
        let neighborhood = analyze_requested_lock_neighborhood(&block)?;

        assert!(neighborhood.overlapping_transactions[0]
            .target_write_other_read
            .contains("shared"));

        Ok(())
    }

    #[test]
    fn transaction_metadata_is_preserved() -> Result<(), String> {
        let evidence = block_transaction(4, "sig", vec!["payer"], 1, 0, 0, Vec::new(), Vec::new());

        let parsed = parse_requested_lock_transaction(&evidence)?;

        assert_eq!(parsed.block_index, 4);
        assert_eq!(parsed.signature, "sig");
        assert_eq!(parsed.fee_lamports, 5_000);
        assert_eq!(parsed.compute_units_consumed, Some(123_456));
        assert!(parsed.succeeded);

        Ok(())
    }

    #[test]
    fn malformed_header_fails_closed() {
        let evidence = legacy_block_transaction(0, "bad", vec!["only-key"], 2, 0, 0);

        assert!(parse_requested_lock_transaction(&evidence).is_err());
    }

    #[test]
    fn unsupported_transaction_version_fails_closed() {
        let mut evidence = legacy_block_transaction(0, "bad", vec!["payer"], 1, 0, 0);
        evidence.value["version"] = json!(1);

        assert!(parse_requested_lock_transaction(&evidence).is_err());
    }

    #[test]
    fn target_identity_mismatch_fails_closed() {
        let transaction = legacy_block_transaction(0, "actual", vec!["payer"], 1, 0, 0);

        let block = test_block(vec![transaction], 0, "declared");

        assert!(analyze_requested_lock_neighborhood(&block).is_err());
    }

    fn test_block(
        transactions: Vec<BlockTransactionEvidence>,
        target_block_index: usize,
        target_signature: &str,
    ) -> BlockEvidence {
        BlockEvidence {
            slot: 123,
            blockhash: "blockhash".to_owned(),
            previous_blockhash: "previous-blockhash".to_owned(),
            parent_slot: 122,
            block_time: Some(1_700_000_000),
            target_signature: target_signature.to_owned(),
            target_block_index,
            transactions,
        }
    }
}
