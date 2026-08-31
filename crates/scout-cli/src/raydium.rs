use serde_json::{json, Value};

pub const RAYDIUM_CPMM_PROGRAM_ID: &str =
    "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

pub const RAYDIUM_CPMM_POOL_STATE_DISCRIMINATOR: [u8; 8] =
    [247, 237, 227, 245, 215, 195, 222, 70];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmAccountObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub data_len: usize,
}

pub fn program_subscribe_request() -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": 2,
        "method": "programSubscribe",
        "params": [
            RAYDIUM_CPMM_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64"
            }
        ]
    })
}

pub fn parse_program_notification(
    payload: &Value,
) -> Result<Option<RaydiumCpmmAccountObservation>, String> {
    if payload.get("method").and_then(Value::as_str) != Some("programNotification") {
        return Ok(None);
    }

    let slot = payload
        .pointer("/params/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Raydium notification missing slot".to_owned())?;

    let pubkey = payload
        .pointer("/params/result/value/pubkey")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing pubkey".to_owned())?
        .to_owned();

    let owner = payload
        .pointer("/params/result/value/account/owner")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing owner".to_owned())?
        .to_owned();

    if owner != RAYDIUM_CPMM_PROGRAM_ID {
        return Err(format!("unexpected Raydium account owner: {owner}"));
    }

    let encoded_data = payload
        .pointer("/params/result/value/account/data/0")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing base64 account data".to_owned())?;

    Ok(Some(RaydiumCpmmAccountObservation {
        pubkey,
        slot,
        owner,
        data_len: encoded_data.len(),
    }))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_targets_raydium_cpmm() {
        let request = program_subscribe_request();

        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(RAYDIUM_CPMM_PROGRAM_ID)
        );
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("programSubscribe")
        );
    }

    #[test]
    fn ignores_non_program_notifications() -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "slotNotification"
        });

        assert_eq!(parse_program_notification(&payload)?, None);

        Ok(())
    }

    #[test]
    fn parses_read_only_raydium_account_observation() -> Result<(), String> {
        let payload = json!({
            "jsonrpc": "2.0",
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": 123456
                    },
                    "value": {
                        "pubkey": "ExamplePool111111111111111111111111111111111",
                        "account": {
                            "data": [
                                "AQIDBA==",
                                "base64"
                            ],
                            "executable": false,
                            "lamports": 1,
                            "owner": RAYDIUM_CPMM_PROGRAM_ID,
                            "rentEpoch": 0,
                            "space": 3
                        }
                    }
                },
                "subscription": 99
            }
        });

        let observation = parse_program_notification(&payload)?
            .ok_or_else(|| "expected Raydium observation".to_owned())?;

        assert_eq!(observation.slot, 123456);
        assert_eq!(observation.owner, RAYDIUM_CPMM_PROGRAM_ID);
        assert_eq!(observation.data_len, 8);

        Ok(())
    }
}
