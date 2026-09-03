use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SIGNATURE_PAGE_LIMIT: usize = 100;
pub const MAX_SIGNATURE_PAGES_PER_ADDRESS: usize = 2;
pub const MAX_TRANSACTION_FETCHES: usize = 32;
pub const MAX_BLOCK_TRANSACTIONS: usize = 10_000;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoryRequest {
    pub address: String,
    pub start_slot: u64,
    pub end_slot: u64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct SignatureObservation {
    pub signature: String,
    pub slot: u64,
    pub err: Value,
    pub memo: Option<String>,
    pub block_time: Option<i64>,
    pub confirmation_status: Option<String>,
}

#[derive(Debug, Clone)]
pub struct AddressHistory {
    pub request: HistoryRequest,
    pub observations: Vec<SignatureObservation>,
    pub complete_through_start_slot: bool,
    pub reason: Option<String>,
}

#[derive(Debug, Clone)]
pub struct HistoryAcquisition {
    pub confirmed_tip_slot: Option<u64>,
    pub histories: BTreeMap<HistoryRequest, AddressHistory>,
    pub incomplete_reasons: Vec<String>,
}

impl HistoryAcquisition {
    pub fn is_complete(&self) -> bool {
        self.incomplete_reasons.is_empty()
            && self
                .histories
                .values()
                .all(|history| history.complete_through_start_slot)
    }
}

#[derive(Debug, Clone)]
pub struct TransactionEvidence {
    pub signature: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct TransactionAcquisition {
    pub transactions: BTreeMap<String, TransactionEvidence>,
    pub incomplete_reasons: Vec<String>,
}

impl TransactionAcquisition {
    pub fn is_complete(&self) -> bool {
        self.incomplete_reasons.is_empty()
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockTransactionEvidence {
    pub block_index: usize,
    pub signature: String,
    pub value: Value,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BlockEvidence {
    pub slot: u64,
    pub blockhash: String,
    pub previous_blockhash: String,
    pub parent_slot: u64,
    pub block_time: Option<i64>,
    pub target_signature: String,
    pub target_block_index: usize,
    pub transactions: Vec<BlockTransactionEvidence>,
}

#[derive(Debug, Clone)]
pub struct BlockAcquisition {
    pub block: Option<BlockEvidence>,
    pub incomplete_reasons: Vec<String>,
}

impl BlockAcquisition {
    pub fn is_complete(&self) -> bool {
        self.block.is_some() && self.incomplete_reasons.is_empty()
    }
}

pub async fn acquire_histories(
    client: &Client,
    rpc_url: &str,
    requests: &[HistoryRequest],
) -> HistoryAcquisition {
    let requested = requests.iter().cloned().collect::<BTreeSet<_>>();
    let mut acquisition = HistoryAcquisition {
        confirmed_tip_slot: None,
        histories: BTreeMap::new(),
        incomplete_reasons: Vec::new(),
    };

    if requested.is_empty() {
        return acquisition;
    }

    let confirmed_tip_slot = match fetch_confirmed_slot(client, rpc_url).await {
        Ok(slot) => {
            acquisition.confirmed_tip_slot = Some(slot);
            slot
        }
        Err(error) => {
            acquisition
                .incomplete_reasons
                .push(format!("confirmed tip unavailable: {error}"));
            return acquisition;
        }
    };

    let mut union_by_address: BTreeMap<String, (u64, u64)> = BTreeMap::new();
    for request in &requested {
        if request.start_slot > request.end_slot {
            insert_incomplete_history(
                &mut acquisition,
                request.clone(),
                format!(
                    "invalid history window: start_slot={} end_slot={}",
                    request.start_slot, request.end_slot
                ),
            );
            continue;
        }

        union_by_address
            .entry(request.address.clone())
            .and_modify(|window| {
                window.0 = window.0.min(request.start_slot);
                window.1 = window.1.max(request.end_slot);
            })
            .or_insert((request.start_slot, request.end_slot));
    }

    let mut union_histories = BTreeMap::new();
    for (address, (start_slot, end_slot)) in union_by_address {
        let union_request = HistoryRequest {
            address: address.clone(),
            start_slot,
            end_slot,
        };

        if confirmed_tip_slot < end_slot {
            let reason = format!(
                "confirmed chain tip precedes requested end slot: confirmed_tip={} end_slot={}",
                confirmed_tip_slot, end_slot
            );
            acquisition
                .incomplete_reasons
                .push(format!("address={address} {reason}"));
            union_histories.insert(
                address,
                AddressHistory {
                    request: union_request,
                    observations: Vec::new(),
                    complete_through_start_slot: false,
                    reason: Some(reason),
                },
            );
            continue;
        }

        match fetch_address_history(client, rpc_url, &union_request).await {
            Ok(history) => {
                if !history.complete_through_start_slot {
                    acquisition.incomplete_reasons.push(format!(
                        "history incomplete: address={} start_slot={} end_slot={} reason={}",
                        history.request.address,
                        history.request.start_slot,
                        history.request.end_slot,
                        history.reason.as_deref().unwrap_or("unknown")
                    ));
                }
                union_histories.insert(address, history);
            }
            Err(error) => {
                acquisition.incomplete_reasons.push(format!(
                    "history RPC failed: address={} start_slot={} end_slot={} error={error}",
                    union_request.address, union_request.start_slot, union_request.end_slot
                ));
                union_histories.insert(
                    address,
                    AddressHistory {
                        request: union_request,
                        observations: Vec::new(),
                        complete_through_start_slot: false,
                        reason: Some(error),
                    },
                );
            }
        }
    }

    for request in requested {
        if acquisition.histories.contains_key(&request) {
            continue;
        }

        let Some(union_history) = union_histories.get(&request.address) else {
            insert_incomplete_history(
                &mut acquisition,
                request,
                "union history unavailable for requested address".to_owned(),
            );
            continue;
        };

        let observations = union_history
            .observations
            .iter()
            .filter(|observation| {
                observation.slot >= request.start_slot && observation.slot <= request.end_slot
            })
            .cloned()
            .collect();

        acquisition.histories.insert(
            request.clone(),
            AddressHistory {
                request,
                observations,
                complete_through_start_slot: union_history.complete_through_start_slot,
                reason: union_history.reason.clone(),
            },
        );
    }

    acquisition
}

fn insert_incomplete_history(
    acquisition: &mut HistoryAcquisition,
    request: HistoryRequest,
    reason: String,
) {
    acquisition.incomplete_reasons.push(format!(
        "address={} start_slot={} end_slot={} {reason}",
        request.address, request.start_slot, request.end_slot
    ));
    acquisition.histories.insert(
        request.clone(),
        AddressHistory {
            request,
            observations: Vec::new(),
            complete_through_start_slot: false,
            reason: Some(reason),
        },
    );
}

pub async fn acquire_transactions(
    client: &Client,
    rpc_url: &str,
    signatures: &BTreeSet<String>,
) -> TransactionAcquisition {
    let mut acquisition = TransactionAcquisition {
        transactions: BTreeMap::new(),
        incomplete_reasons: Vec::new(),
    };

    if signatures.len() > MAX_TRANSACTION_FETCHES {
        acquisition.incomplete_reasons.push(format!(
            "transaction candidate cap exceeded: count={} cap={MAX_TRANSACTION_FETCHES}",
            signatures.len()
        ));
        return acquisition;
    }

    for signature in signatures {
        match fetch_transaction(client, rpc_url, signature).await {
            Ok(Some(value)) => {
                acquisition.transactions.insert(
                    signature.clone(),
                    TransactionEvidence {
                        signature: signature.clone(),
                        value,
                    },
                );
            }
            Ok(None) => acquisition.incomplete_reasons.push(format!(
                "getTransaction returned null for requested signature {signature}"
            )),
            Err(error) => acquisition.incomplete_reasons.push(format!(
                "getTransaction failed for requested signature {signature}: {error}"
            )),
        }
    }

    acquisition
}

pub async fn acquire_block_for_signature(
    client: &Client,
    rpc_url: &str,
    slot: u64,
    target_signature: &str,
) -> BlockAcquisition {
    let mut acquisition = BlockAcquisition {
        block: None,
        incomplete_reasons: Vec::new(),
    };

    if target_signature.trim().is_empty() {
        acquisition
            .incomplete_reasons
            .push("R14 block target signature must not be empty".to_owned());
        return acquisition;
    }

    match fetch_block(client, rpc_url, slot).await {
        Ok(Some(value)) => match parse_block_evidence(slot, target_signature, &value) {
            Ok(block) => acquisition.block = Some(block),
            Err(error) => acquisition
                .incomplete_reasons
                .push(format!("R14 block evidence invalid: {error}")),
        },
        Ok(None) => acquisition.incomplete_reasons.push(format!(
            "getBlock returned null for requested slot {slot}"
        )),
        Err(error) => acquisition.incomplete_reasons.push(format!(
            "getBlock failed for requested slot {slot}: {error}"
        )),
    }

    acquisition
}

pub async fn fetch_confirmed_slot(client: &Client, rpc_url: &str) -> Result<u64, String> {
    let result = rpc_request(
        client,
        rpc_url,
        "getSlot",
        json!([{"commitment": "confirmed"}]),
    )
    .await?;

    result
        .as_u64()
        .ok_or_else(|| "getSlot result was not u64".to_owned())
}

async fn fetch_address_history(
    client: &Client,
    rpc_url: &str,
    request: &HistoryRequest,
) -> Result<AddressHistory, String> {
    let mut observations = Vec::new();
    let mut before: Option<String> = None;
    let mut complete = false;
    let mut reason = None;

    for page_index in 0..MAX_SIGNATURE_PAGES_PER_ADDRESS {
        let mut config = json!({
            "commitment": "confirmed",
            "limit": SIGNATURE_PAGE_LIMIT
        });

        if let Some(cursor) = before.as_deref() {
            config["before"] = Value::String(cursor.to_owned());
        }

        let result = rpc_request(
            client,
            rpc_url,
            "getSignaturesForAddress",
            json!([request.address, config]),
        )
        .await?;

        let entries = result
            .as_array()
            .ok_or_else(|| "getSignaturesForAddress result was not an array".to_owned())?;

        if entries.is_empty() {
            complete = true;
            break;
        }

        for entry in entries {
            observations.push(parse_signature_observation(entry)?);
        }

        let oldest = entries
            .last()
            .ok_or_else(|| "signature page unexpectedly empty".to_owned())?;
        let oldest_slot = required_u64(oldest, "slot")?;

        // The requested lower bound is inclusive. If a full page ends exactly
        // on start_slot, additional signatures from that same slot may still
        // exist on the next page. Only a slot strictly below start_slot proves
        // that the inclusive lower boundary has been fully crossed.
        if oldest_slot < request.start_slot {
            complete = true;
            break;
        }

        if entries.len() < SIGNATURE_PAGE_LIMIT {
            complete = true;
            break;
        }

        before = Some(required_str(oldest, "signature")?.to_owned());

        if page_index + 1 == MAX_SIGNATURE_PAGES_PER_ADDRESS {
            reason = Some(format!(
                "pagination saturated before reaching start_slot={}",
                request.start_slot
            ));
        }
    }

    observations.retain(|observation| {
        observation.slot >= request.start_slot && observation.slot <= request.end_slot
    });
    observations.sort_by(|left, right| {
        left.slot
            .cmp(&right.slot)
            .then_with(|| left.signature.cmp(&right.signature))
    });
    observations.dedup_by(|left, right| left.signature == right.signature);

    Ok(AddressHistory {
        request: request.clone(),
        observations,
        complete_through_start_slot: complete,
        reason,
    })
}

fn parse_signature_observation(value: &Value) -> Result<SignatureObservation, String> {
    let err = value
        .get("err")
        .cloned()
        .ok_or_else(|| "missing err field".to_owned())?;

    Ok(SignatureObservation {
        signature: required_str(value, "signature")?.to_owned(),
        slot: required_u64(value, "slot")?,
        err,
        memo: optional_string(value, "memo")?,
        block_time: optional_i64(value, "blockTime")?,
        confirmation_status: optional_string(value, "confirmationStatus")?,
    })
}

async fn fetch_transaction(
    client: &Client,
    rpc_url: &str,
    signature: &str,
) -> Result<Option<Value>, String> {
    let result = rpc_request(
        client,
        rpc_url,
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

    if result.is_null() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

async fn fetch_block(
    client: &Client,
    rpc_url: &str,
    slot: u64,
) -> Result<Option<Value>, String> {
    let result = rpc_request(
        client,
        rpc_url,
        "getBlock",
        json!([
            slot,
            {
                "commitment": "confirmed",
                "encoding": "json",
                "transactionDetails": "full",
                "rewards": false,
                "maxSupportedTransactionVersion": 0
            }
        ]),
    )
    .await?;

    if result.is_null() {
        Ok(None)
    } else {
        Ok(Some(result))
    }
}

fn parse_block_evidence(
    slot: u64,
    target_signature: &str,
    value: &Value,
) -> Result<BlockEvidence, String> {
    if target_signature.trim().is_empty() {
        return Err("target signature must not be empty".to_owned());
    }

    let blockhash = required_str(value, "blockhash")?.to_owned();
    let previous_blockhash = required_str(value, "previousBlockhash")?.to_owned();
    let parent_slot = required_u64(value, "parentSlot")?;
    let block_time = optional_i64(value, "blockTime")?;

    let transactions = value
        .get("transactions")
        .and_then(Value::as_array)
        .ok_or_else(|| "getBlock result missing transactions array".to_owned())?;

    if transactions.len() > MAX_BLOCK_TRANSACTIONS {
        return Err(format!(
            "block transaction cap exceeded: count={} cap={MAX_BLOCK_TRANSACTIONS}",
            transactions.len()
        ));
    }

    let mut transaction_evidence = Vec::with_capacity(transactions.len());
    let mut target_block_index: Option<usize> = None;

    for (block_index, transaction_value) in transactions.iter().enumerate() {
        let transaction = transaction_value
            .get("transaction")
            .ok_or_else(|| format!("block transaction {block_index} missing transaction"))?;

        let signatures = transaction
            .get("signatures")
            .and_then(Value::as_array)
            .ok_or_else(|| {
                format!("block transaction {block_index} missing signatures array")
            })?;

        let signature = signatures
            .first()
            .and_then(Value::as_str)
            .ok_or_else(|| {
                format!("block transaction {block_index} missing primary signature")
            })?;

        if signature.trim().is_empty() {
            return Err(format!(
                "block transaction {block_index} primary signature was empty"
            ));
        }

        if signature == target_signature {
            if target_block_index.is_some() {
                return Err(format!(
                    "target signature appeared more than once in requested block: {target_signature}"
                ));
            }
            target_block_index = Some(block_index);
        }

        transaction_evidence.push(BlockTransactionEvidence {
            block_index,
            signature: signature.to_owned(),
            value: transaction_value.clone(),
        });
    }

    let target_block_index = target_block_index.ok_or_else(|| {
        format!(
            "target signature was not present in requested block: slot={slot} signature={target_signature}"
        )
    })?;

    Ok(BlockEvidence {
        slot,
        blockhash,
        previous_blockhash,
        parent_slot,
        block_time,
        target_signature: target_signature.to_owned(),
        target_block_index,
        transactions: transaction_evidence,
    })
}

async fn rpc_request(
    client: &Client,
    rpc_url: &str,
    method: &str,
    params: Value,
) -> Result<Value, String> {
    let response = client
        .post(rpc_url)
        .json(&json!({
            "jsonrpc": "2.0",
            "id": 13,
            "method": method,
            "params": params,
        }))
        .send()
        .await
        .map_err(|error| format!("{method} transport failed: {error}"))?;

    let status = response.status();
    if !status.is_success() {
        return Err(format!("{method} HTTP status {status}"));
    }

    let payload = response
        .json::<Value>()
        .await
        .map_err(|error| format!("{method} returned invalid JSON: {error}"))?;

    if let Some(error) = payload.get("error") {
        return Err(format!("{method} RPC error: {error}"));
    }

    payload
        .get("result")
        .cloned()
        .ok_or_else(|| format!("{method} response missing result"))
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

fn optional_i64(value: &Value, field: &str) -> Result<Option<i64>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_i64()
            .map(Some)
            .ok_or_else(|| format!("invalid optional i64 field {field}")),
    }
}

fn optional_string(value: &Value, field: &str) -> Result<Option<String>, String> {
    match value.get(field) {
        None | Some(Value::Null) => Ok(None),
        Some(other) => other
            .as_str()
            .map(|text| Some(text.to_owned()))
            .ok_or_else(|| format!("invalid optional string field {field}")),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn test_block(transactions: Vec<Value>) -> Value {
        json!({
            "blockhash": "blockhash",
            "previousBlockhash": "previous-blockhash",
            "parentSlot": 122,
            "blockTime": 1_700_000_000,
            "transactions": transactions
        })
    }

    fn test_block_transaction(signature: &str) -> Value {
        json!({
            "meta": {
                "err": null,
                "fee": 5_000
            },
            "transaction": {
                "message": {
                    "accountKeys": [],
                    "header": {
                        "numRequiredSignatures": 1,
                        "numReadonlySignedAccounts": 0,
                        "numReadonlyUnsignedAccounts": 0
                    },
                    "instructions": [],
                    "recentBlockhash": "recent-blockhash"
                },
                "signatures": [signature]
            },
            "version": "legacy"
        })
    }

    #[test]
    fn signature_parser_preserves_raw_rpc_evidence() -> Result<(), String> {
        let error = json!({"InstructionError": [1, {"Custom": 6001}]});
        let value = json!({
            "signature": "sig",
            "slot": 123,
            "err": error,
            "memo": "memo",
            "blockTime": null,
            "confirmationStatus": "confirmed"
        });

        let observation = parse_signature_observation(&value)?;
        assert_eq!(observation.signature, "sig");
        assert_eq!(observation.slot, 123);
        assert_eq!(observation.err, error);
        assert_eq!(observation.memo.as_deref(), Some("memo"));
        assert_eq!(observation.block_time, None);
        assert_eq!(
            observation.confirmation_status.as_deref(),
            Some("confirmed")
        );

        Ok(())
    }

    #[test]
    fn signature_parser_preserves_null_error() -> Result<(), String> {
        let value = json!({
            "signature": "sig",
            "slot": 123,
            "err": null,
            "memo": null,
            "blockTime": 1_700_000_000,
            "confirmationStatus": null
        });

        let observation = parse_signature_observation(&value)?;
        assert!(observation.err.is_null());
        assert_eq!(observation.memo, None);
        assert_eq!(observation.block_time, Some(1_700_000_000));
        assert_eq!(observation.confirmation_status, None);

        Ok(())
    }

    #[test]
    fn signature_parser_rejects_missing_error_field() {
        let value = json!({
            "signature": "sig",
            "slot": 123,
            "memo": null,
            "blockTime": null,
            "confirmationStatus": "confirmed"
        });

        assert!(parse_signature_observation(&value).is_err());
    }

    #[test]
    fn signature_parser_rejects_invalid_optional_fields() {
        let invalid_block_time = json!({
            "signature": "sig",
            "slot": 123,
            "err": null,
            "memo": null,
            "blockTime": "not-an-integer",
            "confirmationStatus": "confirmed"
        });
        assert!(parse_signature_observation(&invalid_block_time).is_err());

        let invalid_memo = json!({
            "signature": "sig",
            "slot": 123,
            "err": null,
            "memo": 42,
            "blockTime": null,
            "confirmationStatus": "confirmed"
        });
        assert!(parse_signature_observation(&invalid_memo).is_err());
    }

    #[test]
    fn history_acquisition_requires_every_history_complete() {
        let request = HistoryRequest {
            address: "pool".to_owned(),
            start_slot: 100,
            end_slot: 132,
        };
        let mut histories = BTreeMap::new();
        histories.insert(
            request.clone(),
            AddressHistory {
                request,
                observations: Vec::new(),
                complete_through_start_slot: false,
                reason: Some("pagination saturated".to_owned()),
            },
        );

        let acquisition = HistoryAcquisition {
            confirmed_tip_slot: Some(200),
            histories,
            incomplete_reasons: vec!["history incomplete".to_owned()],
        };
        assert!(!acquisition.is_complete());
    }

    #[test]
    fn transaction_acquisition_is_incomplete_when_any_fetch_is_unresolved() {
        let acquisition = TransactionAcquisition {
            transactions: BTreeMap::new(),
            incomplete_reasons: vec!["getTransaction returned null".to_owned()],
        };
        assert!(!acquisition.is_complete());
    }

    #[test]
    fn transaction_acquisition_can_be_complete_with_no_requested_signatures() {
        let acquisition = TransactionAcquisition {
            transactions: BTreeMap::new(),
            incomplete_reasons: Vec::new(),
        };
        assert!(acquisition.is_complete());
    }

    #[test]
    fn block_acquisition_requires_evidence_and_no_incomplete_reason() {
        let incomplete = BlockAcquisition {
            block: None,
            incomplete_reasons: Vec::new(),
        };
        assert!(!incomplete.is_complete());

        let block = parse_block_evidence(
            123,
            "target",
            &test_block(vec![test_block_transaction("target")]),
        )
        .expect("fixture must parse");

        let complete = BlockAcquisition {
            block: Some(block),
            incomplete_reasons: Vec::new(),
        };
        assert!(complete.is_complete());

        let unresolved = BlockAcquisition {
            block: complete.block,
            incomplete_reasons: vec!["unresolved".to_owned()],
        };
        assert!(!unresolved.is_complete());
    }

    #[test]
    fn block_parser_preserves_order_and_locates_target() -> Result<(), String> {
        let block = test_block(vec![
            test_block_transaction("first"),
            test_block_transaction("target"),
            test_block_transaction("third"),
        ]);

        let evidence = parse_block_evidence(123, "target", &block)?;

        assert_eq!(evidence.slot, 123);
        assert_eq!(evidence.blockhash, "blockhash");
        assert_eq!(evidence.previous_blockhash, "previous-blockhash");
        assert_eq!(evidence.parent_slot, 122);
        assert_eq!(evidence.block_time, Some(1_700_000_000));
        assert_eq!(evidence.target_signature, "target");
        assert_eq!(evidence.target_block_index, 1);
        assert_eq!(evidence.transactions.len(), 3);
        assert_eq!(evidence.transactions[0].block_index, 0);
        assert_eq!(evidence.transactions[0].signature, "first");
        assert_eq!(evidence.transactions[1].block_index, 1);
        assert_eq!(evidence.transactions[1].signature, "target");
        assert_eq!(evidence.transactions[2].block_index, 2);
        assert_eq!(evidence.transactions[2].signature, "third");

        Ok(())
    }

    #[test]
    fn block_parser_rejects_missing_target() {
        let block = test_block(vec![
            test_block_transaction("first"),
            test_block_transaction("second"),
        ]);

        assert!(parse_block_evidence(123, "target", &block).is_err());
    }

    #[test]
    fn block_parser_rejects_duplicate_target() {
        let block = test_block(vec![
            test_block_transaction("target"),
            test_block_transaction("target"),
        ]);

        assert!(parse_block_evidence(123, "target", &block).is_err());
    }

    #[test]
    fn block_parser_rejects_missing_transaction_array() {
        let block = json!({
            "blockhash": "blockhash",
            "previousBlockhash": "previous-blockhash",
            "parentSlot": 122,
            "blockTime": null
        });

        assert!(parse_block_evidence(123, "target", &block).is_err());
    }

    #[test]
    fn block_parser_rejects_malformed_primary_signature() {
        let block = test_block(vec![json!({
            "meta": {"err": null},
            "transaction": {
                "message": {},
                "signatures": []
            },
            "version": "legacy"
        })]);

        assert!(parse_block_evidence(123, "target", &block).is_err());
    }

    #[test]
    fn block_parser_rejects_empty_target_signature() {
        let block = test_block(vec![test_block_transaction("target")]);

        assert!(parse_block_evidence(123, "", &block).is_err());
    }

    #[test]
    fn block_parser_fails_closed_above_transaction_cap() {
        let transactions =
            vec![test_block_transaction("other"); MAX_BLOCK_TRANSACTIONS + 1];
        let block = test_block(transactions);

        assert!(parse_block_evidence(123, "target", &block).is_err());
    }
}
