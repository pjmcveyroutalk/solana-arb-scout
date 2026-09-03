use reqwest::Client;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};

const SIGNATURE_PAGE_LIMIT: usize = 100;
const MAX_SIGNATURE_PAGES_PER_POOL: usize = 2;
const MAX_TRANSACTION_FETCHES: usize = 32;

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
pub struct TransactionEvidence {
    pub signature: String,
    pub value: Value,
}

#[derive(Debug, Clone)]
pub struct AcquisitionBundle {
    pub confirmed_tip_slot: Option<u64>,
    pub histories: BTreeMap<HistoryRequest, AddressHistory>,
    pub transactions: BTreeMap<String, TransactionEvidence>,
    pub incomplete_reasons: Vec<String>,
}

impl AcquisitionBundle {
    pub fn is_complete(&self) -> bool {
        self.incomplete_reasons.is_empty()
            && self
                .histories
                .values()
                .all(|history| history.complete_through_start_slot)
    }
}

pub async fn acquire(
    client: &Client,
    rpc_url: &str,
    requests: &[HistoryRequest],
) -> AcquisitionBundle {
    let unique_requests = requests.iter().cloned().collect::<BTreeSet<_>>();
    let mut bundle = AcquisitionBundle {
        confirmed_tip_slot: None,
        histories: BTreeMap::new(),
        transactions: BTreeMap::new(),
        incomplete_reasons: Vec::new(),
    };

    let confirmed_tip_slot = match fetch_confirmed_slot(client, rpc_url).await {
        Ok(slot) => {
            bundle.confirmed_tip_slot = Some(slot);
            slot
        }
        Err(error) => {
            bundle
                .incomplete_reasons
                .push(format!("confirmed tip unavailable: {error}"));
            return bundle;
        }
    };

    for request in unique_requests {
        if confirmed_tip_slot < request.end_slot {
            bundle.incomplete_reasons.push(format!(
                "chain has not reached requested end slot: address={} confirmed_tip={} end_slot={}",
                request.address, confirmed_tip_slot, request.end_slot
            ));
            bundle.histories.insert(
                request.clone(),
                AddressHistory {
                    request,
                    observations: Vec::new(),
                    complete_through_start_slot: false,
                    reason: Some("confirmed chain tip precedes requested end slot".to_owned()),
                },
            );
            continue;
        }

        match fetch_address_history(client, rpc_url, &request).await {
            Ok(history) => {
                if !history.complete_through_start_slot {
                    bundle.incomplete_reasons.push(format!(
                        "history incomplete: address={} start_slot={} end_slot={} reason={}",
                        request.address,
                        request.start_slot,
                        request.end_slot,
                        history.reason.as_deref().unwrap_or("unknown")
                    ));
                }
                bundle.histories.insert(request, history);
            }
            Err(error) => {
                bundle.incomplete_reasons.push(format!(
                    "history RPC failed: address={} start_slot={} end_slot={} error={error}",
                    request.address, request.start_slot, request.end_slot
                ));
                bundle.histories.insert(
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

    let signatures = candidate_signatures(&bundle.histories);
    if signatures.len() > MAX_TRANSACTION_FETCHES {
        bundle.incomplete_reasons.push(format!(
            "transaction candidate cap exceeded: count={} cap={MAX_TRANSACTION_FETCHES}",
            signatures.len()
        ));
        return bundle;
    }

    for signature in signatures {
        match fetch_transaction(client, rpc_url, &signature).await {
            Ok(Some(value)) => {
                bundle.transactions.insert(
                    signature.clone(),
                    TransactionEvidence { signature, value },
                );
            }
            Ok(None) => bundle.incomplete_reasons.push(format!(
                "getTransaction returned null for intersecting signature {signature}"
            )),
            Err(error) => bundle.incomplete_reasons.push(format!(
                "getTransaction failed for intersecting signature {signature}: {error}"
            )),
        }
    }

    bundle
}

fn candidate_signatures(
    histories: &BTreeMap<HistoryRequest, AddressHistory>,
) -> BTreeSet<String> {
    let mut occurrence_count = BTreeMap::<String, usize>::new();

    for history in histories.values() {
        let mut seen_in_history = BTreeSet::new();
        for observation in &history.observations {
            if observation.succeeded
                && observation.slot >= history.request.start_slot
                && observation.slot <= history.request.end_slot
            {
                seen_in_history.insert(observation.signature.clone());
            }
        }
        for signature in seen_in_history {
            *occurrence_count.entry(signature).or_default() += 1;
        }
    }

    occurrence_count
        .into_iter()
        .filter_map(|(signature, count)| (count >= 2).then_some(signature))
        .collect()
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

    for page_index in 0..MAX_SIGNATURE_PAGES_PER_POOL {
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
            let signature = required_str(entry, "signature")?.to_owned();
            let slot = required_u64(entry, "slot")?;
            observations.push(SignatureObservation {
                signature,
                slot,
                succeeded: entry.get("err").is_some_and(Value::is_null),
                block_time: optional_i64(entry, "blockTime")?,
            });
        }

        let oldest_slot = entries
            .last()
            .and_then(|entry| entry.get("slot"))
            .and_then(Value::as_u64)
            .ok_or_else(|| "signature page missing oldest slot".to_owned())?;

        if oldest_slot <= request.start_slot {
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
            reason = Some("pagination cursor unavailable".to_owned());
            break;
        }

        if page_index + 1 == MAX_SIGNATURE_PAGES_PER_POOL {
            reason = Some(format!(
                "pagination saturated before start_slot={}",
                request.start_slot
            ));
        }
    }

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

    fn history(
        address: &str,
        signatures: &[(&str, u64, bool)],
    ) -> (HistoryRequest, AddressHistory) {
        let request = HistoryRequest {
            address: address.to_owned(),
            start_slot: 100,
            end_slot: 132,
        };
        let observations = signatures
            .iter()
            .map(|(signature, slot, succeeded)| SignatureObservation {
                signature: (*signature).to_owned(),
                slot: *slot,
                succeeded: *succeeded,
                block_time: None,
            })
            .collect();

        (
            request.clone(),
            AddressHistory {
                request,
                observations,
                complete_through_start_slot: true,
                reason: None,
            },
        )
    }

    #[test]
    fn candidate_signatures_require_two_distinct_histories() {
        let mut histories = BTreeMap::new();
        let (left_request, left) =
            history("left", &[("shared", 110, true), ("left-only", 111, true)]);
        let (right_request, right) =
            history("right", &[("shared", 110, true), ("right-only", 112, true)]);
        histories.insert(left_request, left);
        histories.insert(right_request, right);

        assert_eq!(
            candidate_signatures(&histories),
            BTreeSet::from(["shared".to_owned()])
        );
    }

    #[test]
    fn failed_signatures_do_not_become_transaction_candidates() {
        let mut histories = BTreeMap::new();
        let (left_request, left) = history("left", &[("failed", 110, false)]);
        let (right_request, right) = history("right", &[("failed", 110, false)]);
        histories.insert(left_request, left);
        histories.insert(right_request, right);

        assert!(candidate_signatures(&histories).is_empty());
    }

    #[test]
    fn bundle_is_incomplete_when_any_history_is_incomplete() {
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
                reason: Some("saturated".to_owned()),
            },
        );

        let bundle = AcquisitionBundle {
            confirmed_tip_slot: Some(200),
            histories,
            transactions: BTreeMap::new(),
            incomplete_reasons: vec!["history incomplete".to_owned()],
        };
        assert!(!bundle.is_complete());
    }
}
