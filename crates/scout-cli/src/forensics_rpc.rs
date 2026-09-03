use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

pub const SIGNATURE_PAGE_LIMIT: usize = 100;
pub const MAX_SIGNATURE_PAGES_PER_ADDRESS: usize = 2;
pub const MAX_TRANSACTION_FETCHES: usize = 32;

#[derive(Debug, Clone, PartialEq, Eq, PartialOrd, Ord)]
pub struct HistoryRequest {
    pub address: String,
    pub start_slot: u64,
    pub end_slot: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SignatureObservation {
    pub signature: String,
    pub slot: u64,
    pub succeeded: bool,
    pub block_time: Option<i64>,
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

pub async fn acquire_histories(
    client: &Client,
    rpc_url: &str,
    requests: &[HistoryRequest],
) -> HistoryAcquisition {
    let unique_requests = requests.iter().cloned().collect::<BTreeSet<_>>();
    let mut acquisition = HistoryAcquisition {
        confirmed_tip_slot: None,
        histories: BTreeMap::new(),
        incomplete_reasons: Vec::new(),
    };

    if unique_requests.is_empty() {
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

    for request in unique_requests {
        if request.start_slot > request.end_slot {
            let reason = format!(
                "invalid history window: start_slot={} end_slot={}",
                request.start_slot, request.end_slot
            );
            acquisition
                .incomplete_reasons
                .push(format!("address={} {reason}", request.address));
            acquisition.histories.insert(
                request.clone(),
                AddressHistory {
                    request,
                    observations: Vec::new(),
                    complete_through_start_slot: false,
                    reason: Some(reason),
                },
            );
            continue;
        }

        if confirmed_tip_slot < request.end_slot {
            let reason = format!(
                "confirmed chain tip precedes requested end slot: confirmed_tip={} end_slot={}",
                confirmed_tip_slot, request.end_slot
            );
            acquisition
                .incomplete_reasons
                .push(format!("address={} {reason}", request.address));
            acquisition.histories.insert(
                request.clone(),
                AddressHistory {
                    request,
                    observations: Vec::new(),
                    complete_through_start_slot: false,
                    reason: Some(reason),
                },
            );
            continue;
        }

        match fetch_address_history(client, rpc_url, &request).await {
            Ok(history) => {
                if !history.complete_through_start_slot {
                    acquisition.incomplete_reasons.push(format!(
                        "history incomplete: address={} start_slot={} end_slot={} reason={}",
                        request.address,
                        request.start_slot,
                        request.end_slot,
                        history.reason.as_deref().unwrap_or("unknown")
                    ));
                }
                acquisition.histories.insert(request, history);
            }
            Err(error) => {
                acquisition.incomplete_reasons.push(format!(
                    "history RPC failed: address={} start_slot={} end_slot={} error={error}",
                    request.address, request.start_slot, request.end_slot
                ));
                acquisition.histories.insert(
                    request.clone(),
                    AddressHistory {
                        request,
                        observations: Vec::new(),
                        complete_through_start_slot: false,
                        reason: Some(error),
                    },
                );
            }
        }
    }

    acquisition
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

async fn fetch_confirmed_slot(client: &Client, rpc_url: &str) -> Result<u64, String> {
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

        if oldest_slot <= request.start_slot {
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
    Ok(SignatureObservation {
        signature: required_str(value, "signature")?.to_owned(),
        slot: required_u64(value, "slot")?,
        succeeded: value.get("err").is_some_and(Value::is_null),
        block_time: optional_i64(value, "blockTime")?,
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signature_parser_accepts_nullable_block_time_and_success() {
        let value = json!({
            "signature": "sig",
            "slot": 123,
            "err": null,
            "blockTime": null
        });

        let observation = parse_signature_observation(&value).expect("signature must parse");
        assert_eq!(observation.signature, "sig");
        assert_eq!(observation.slot, 123);
        assert!(observation.succeeded);
        assert_eq!(observation.block_time, None);
    }

    #[test]
    fn signature_parser_rejects_invalid_block_time() {
        let value = json!({
            "signature": "sig",
            "slot": 123,
            "err": null,
            "blockTime": "not-an-integer"
        });

        assert!(parse_signature_observation(&value).is_err());
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
}
