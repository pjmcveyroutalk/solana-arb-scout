use super::{BlockEvidence, BlockTransactionEvidence};
use crate::forensics::{CandidateEvidence, RouteAnalysis};
use serde_json::Value;
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum EconomicsCompleteness {
    Complete,
    Incomplete,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CaptureCompleteness {
    Incomplete,
    CompleteNoAtomicMatch,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TimingCompleteness {
    Incomplete,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OpportunityCohort {
    pub schema_version: &'static str,
    pub source_run_id: String,
    pub source_record_sequence: u64,
    pub candidate_id: String,
    pub route_id: String,
    pub usd_size: u64,
    pub source_economics_status: String,
    pub economics_status: EconomicsCompleteness,
    pub capture_status: CaptureCompleteness,
    pub capture_reason: Option<String>,
    pub candidate_found_at_unix_ms: u64,
    pub quote_complete_at_unix_ms: Option<u64>,
    pub economics_complete_at_unix_ms: Option<u64>,
    pub hypothetical_ready_at_unix_ms: Option<u64>,
    pub candidate_to_quote_ms: Option<u64>,
    pub candidate_to_economics_ms: Option<u64>,
    pub quote_to_economics_ms: Option<u64>,
    pub market_correction_at_unix_ms: Option<u64>,
    pub decision_margin_ms: Option<u64>,
    pub timing_status: TimingCompleteness,
    pub decision_eligible: bool,
}

#[allow(dead_code)]
pub fn build_opportunity_cohort(
    candidate: &CandidateEvidence,
    analysis: &RouteAnalysis,
) -> Result<OpportunityCohort, String> {
    validate_candidate_identity(candidate)?;

    if analysis.route_id != candidate.route.route_id {
        return Err(format!(
            "R14B route identity mismatch: candidate={} analysis={}",
            candidate.route.route_id, analysis.route_id
        ));
    }

    let capture_status = capture_completeness(analysis)?;
    let economics_status = economics_completeness(candidate)?;
    let candidate_to_quote_ms = checked_elapsed(
        candidate.candidate_found_at_unix_ms,
        candidate.quote_complete_at_unix_ms,
        "quote completion",
    )?;
    let candidate_to_economics_ms = checked_elapsed(
        candidate.candidate_found_at_unix_ms,
        candidate.economics_complete_at_unix_ms,
        "economics completion",
    )?;
    let quote_to_economics_ms = match (
        candidate.quote_complete_at_unix_ms,
        candidate.economics_complete_at_unix_ms,
    ) {
        (Some(quote), Some(economics)) => Some(economics.checked_sub(quote).ok_or_else(|| {
            "R14B economics completion precedes quote completion".to_owned()
        })?),
        _ => None,
    };

    Ok(OpportunityCohort {
        schema_version: "r14-niche-v1",
        source_run_id: candidate.source_run_id.clone(),
        source_record_sequence: candidate.source_record_sequence,
        candidate_id: candidate.candidate_id.clone(),
        route_id: candidate.route.route_id.clone(),
        usd_size: candidate.usd_size,
        source_economics_status: candidate.source_status.clone(),
        economics_status,
        capture_status,
        capture_reason: analysis.reason.clone(),
        candidate_found_at_unix_ms: candidate.candidate_found_at_unix_ms,
        quote_complete_at_unix_ms: candidate.quote_complete_at_unix_ms,
        economics_complete_at_unix_ms: candidate.economics_complete_at_unix_ms,
        hypothetical_ready_at_unix_ms: candidate.hypothetical_ready_at_unix_ms,
        candidate_to_quote_ms,
        candidate_to_economics_ms,
        quote_to_economics_ms,
        market_correction_at_unix_ms: None,
        decision_margin_ms: None,
        timing_status: TimingCompleteness::Incomplete,
        decision_eligible: false,
    })
}

fn validate_candidate_identity(candidate: &CandidateEvidence) -> Result<(), String> {
    if candidate.source_run_id.trim().is_empty() {
        return Err("R14B candidate source run_id must not be empty".to_owned());
    }
    if candidate.source_record_sequence == 0 {
        return Err("R14B candidate source record sequence must be nonzero".to_owned());
    }
    if candidate.candidate_id.trim().is_empty() {
        return Err("R14B candidate_id must not be empty".to_owned());
    }
    if candidate.route.route_id.trim().is_empty() {
        return Err("R14B candidate route_id must not be empty".to_owned());
    }
    Ok(())
}

fn economics_completeness(
    candidate: &CandidateEvidence,
) -> Result<EconomicsCompleteness, String> {
    match candidate.source_status.as_str() {
        "quote_rejected" => {
            if candidate.economics_complete_at_unix_ms.is_some()
                || candidate.hypothetical_ready_at_unix_ms.is_some()
            {
                return Err(
                    "R14B quote-rejected candidate contains economics or ready timing".to_owned(),
                );
            }

            if candidate
                .quote_complete_at_unix_ms
                .is_some_and(|quote| quote < candidate.candidate_found_at_unix_ms)
            {
                return Err("R14B quote completion precedes candidate discovery".to_owned());
            }

            Ok(EconomicsCompleteness::Incomplete)
        }
        "economics_unresolved" | "economics_resolved_nonpositive" => {
            validate_economics_timing(candidate, false)?;

            Ok(
                if candidate.source_status == "economics_resolved_nonpositive" {
                    EconomicsCompleteness::Complete
                } else {
                    EconomicsCompleteness::Incomplete
                },
            )
        }
        "economics_resolved_positive" => {
            validate_economics_timing(candidate, true)?;
            Ok(EconomicsCompleteness::Complete)
        }
        other => Err(format!("R14B unsupported R12 candidate status {other}")),
    }
}

fn validate_economics_timing(
    candidate: &CandidateEvidence,
    requires_ready: bool,
) -> Result<(), String> {
    let quote = candidate
        .quote_complete_at_unix_ms
        .ok_or_else(|| "R14B economics candidate missing quote completion timing".to_owned())?;
    let economics = candidate
        .economics_complete_at_unix_ms
        .ok_or_else(|| "R14B economics candidate missing economics completion timing".to_owned())?;

    if quote < candidate.candidate_found_at_unix_ms {
        return Err("R14B quote completion precedes candidate discovery".to_owned());
    }

    if economics < quote {
        return Err("R14B economics completion precedes quote completion".to_owned());
    }

    match (requires_ready, candidate.hypothetical_ready_at_unix_ms) {
        (true, Some(ready)) if ready >= economics => Ok(()),
        (true, Some(_)) => Err("R14B hypothetical-ready precedes economics completion".to_owned()),
        (true, None) => Err("R14B positive candidate missing hypothetical-ready timing".to_owned()),
        (false, None) => Ok(()),
        (false, Some(_)) => {
            Err("R14B non-positive or unresolved candidate contains ready timing".to_owned())
        }
    }
}

fn capture_completeness(analysis: &RouteAnalysis) -> Result<CaptureCompleteness, String> {
    match analysis.status.as_str() {
        "search_incomplete" => {
            if !analysis.matches.is_empty() {
                return Err("R14B incomplete search unexpectedly contains landed matches".to_owned());
            }

            Ok(CaptureCompleteness::Incomplete)
        }
        "no_atomic_match_complete" => {
            if !analysis.matches.is_empty() {
                return Err(
                    "R14B complete no-match result unexpectedly contains matches".to_owned(),
                );
            }

            Ok(CaptureCompleteness::CompleteNoAtomicMatch)
        }
        "atomic_route_match"
        | "atomic_route_amounts_unresolved"
        | "atomic_route_outcome_resolved" => Err(
            "R14B landed-match cohort construction is deferred until landed evidence exists"
                .to_owned(),
        ),
        other => Err(format!("R14B unsupported R13 analysis status {other}")),
    }
}

fn checked_elapsed(start: u64, end: Option<u64>, label: &str) -> Result<Option<u64>, String> {
    match end {
        Some(end) => end
            .checked_sub(start)
            .map(Some)
            .ok_or_else(|| format!("R14B {label} precedes candidate discovery")),
        None => Ok(None),
    }
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
        let evidence =
            block_transaction(4, "sig", vec!["payer"], 1, 0, 0, Vec::new(), Vec::new());

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

#[cfg(test)]
mod cohort_tests {
    use super::*;
    use crate::forensics::{LegEvidence, RouteEvidence};

    fn candidate(status: &str) -> CandidateEvidence {
        CandidateEvidence {
            source_run_id: "run".to_owned(),
            source_record_sequence: 2,
            candidate_id: "candidate".to_owned(),
            source_status: status.to_owned(),
            usd_size: 10,
            candidate_found_at_unix_ms: 1_000,
            quote_complete_at_unix_ms: Some(1_010),
            economics_complete_at_unix_ms: Some(1_020),
            hypothetical_ready_at_unix_ms: None,
            route: RouteEvidence {
                route_id: "route".to_owned(),
                anchor_mint: "anchor".to_owned(),
                intermediate_mint: "middle".to_owned(),
                leg_1: LegEvidence {
                    venue: "raydium_cpmm".to_owned(),
                    pool_id: "pool-1".to_owned(),
                    input_mint: "anchor".to_owned(),
                    output_mint: "middle".to_owned(),
                    source_slot: 100,
                },
                leg_2: LegEvidence {
                    venue: "pumpswap".to_owned(),
                    pool_id: "pool-2".to_owned(),
                    input_mint: "middle".to_owned(),
                    output_mint: "anchor".to_owned(),
                    source_slot: 100,
                },
            },
        }
    }

    fn analysis(status: &str) -> RouteAnalysis {
        RouteAnalysis {
            route_id: "route".to_owned(),
            status: status.to_owned(),
            reason: None,
            matches: Vec::new(),
        }
    }

    #[test]
    fn complete_no_match_builds_minimal_cohort() -> Result<(), String> {
        let cohort = build_opportunity_cohort(
            &candidate("economics_unresolved"),
            &analysis("no_atomic_match_complete"),
        )?;

        assert_eq!(
            cohort.economics_status,
            EconomicsCompleteness::Incomplete
        );
        assert_eq!(
            cohort.capture_status,
            CaptureCompleteness::CompleteNoAtomicMatch
        );
        assert_eq!(cohort.candidate_to_quote_ms, Some(10));
        assert_eq!(cohort.candidate_to_economics_ms, Some(20));
        assert_eq!(cohort.quote_to_economics_ms, Some(10));
        assert_eq!(cohort.market_correction_at_unix_ms, None);
        assert_eq!(cohort.decision_margin_ms, None);
        assert!(!cohort.decision_eligible);

        Ok(())
    }

    #[test]
    fn incomplete_search_remains_capture_incomplete() -> Result<(), String> {
        let cohort = build_opportunity_cohort(
            &candidate("economics_unresolved"),
            &analysis("search_incomplete"),
        )?;

        assert_eq!(cohort.capture_status, CaptureCompleteness::Incomplete);
        Ok(())
    }

    #[test]
    fn route_identity_mismatch_fails_closed() {
        let mut route_analysis = analysis("no_atomic_match_complete");
        route_analysis.route_id = "other-route".to_owned();

        assert!(build_opportunity_cohort(
            &candidate("economics_unresolved"),
            &route_analysis
        )
        .is_err());
    }

    #[test]
    fn landed_status_fails_closed_until_evidence_exists() {
        assert!(build_opportunity_cohort(
            &candidate("economics_unresolved"),
            &analysis("atomic_route_match")
        )
        .is_err());
    }

    #[test]
    fn malformed_positive_timing_fails_closed() {
        let positive = candidate("economics_resolved_positive");

        assert!(
            build_opportunity_cohort(&positive, &analysis("no_atomic_match_complete")).is_err()
        );
    }
}
