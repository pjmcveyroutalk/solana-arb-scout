use crate::raydium::{self, RaydiumCpmmAccountObservation};
use serde_json::{json, Value};

const RAYDIUM_POOL_STATE_LEN: usize = 637;
const RAYDIUM_POOL_STATE_DISCRIMINATOR_B58: &str = "iUE1qg7KXeV";
const RAYDIUM_TOKEN_0_MINT_OFFSET: usize = 168;
const RAYDIUM_TOKEN_1_MINT_OFFSET: usize = 200;

pub fn raydium_pair_lookup_requests(anchor_mint: &str, intermediate_mint: &str) -> [Value; 2] {
    [
        raydium_pair_lookup_request(9, anchor_mint, intermediate_mint),
        raydium_pair_lookup_request(10, intermediate_mint, anchor_mint),
    ]
}

fn raydium_pair_lookup_request(request_id: u64, token_0_mint: &str, token_1_mint: &str) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getProgramAccounts",
        "params": [
            raydium::RAYDIUM_CPMM_PROGRAM_ID,
            {
                "commitment": "processed",
                "encoding": "base64",
                "withContext": true,
                "filters": [
                    {
                        "dataSize": RAYDIUM_POOL_STATE_LEN
                    },
                    {
                        "memcmp": {
                            "offset": 0,
                            "bytes": RAYDIUM_POOL_STATE_DISCRIMINATOR_B58
                        }
                    },
                    {
                        "memcmp": {
                            "offset": RAYDIUM_TOKEN_0_MINT_OFFSET,
                            "bytes": token_0_mint
                        }
                    },
                    {
                        "memcmp": {
                            "offset": RAYDIUM_TOKEN_1_MINT_OFFSET,
                            "bytes": token_1_mint
                        }
                    }
                ]
            }
        ]
    })
}

pub fn parse_raydium_pair_lookup_response(
    payload: &Value,
) -> Result<Vec<RaydiumCpmmAccountObservation>, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Raydium exact-pair getProgramAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| {
            "Raydium exact-pair getProgramAccounts response missing context slot".to_owned()
        })?;

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| {
            "Raydium exact-pair getProgramAccounts response missing account array".to_owned()
        })?;

    let mut observations = Vec::with_capacity(accounts.len());

    for entry in accounts {
        let pubkey = entry.get("pubkey").and_then(Value::as_str).ok_or_else(|| {
            "Raydium exact-pair getProgramAccounts entry missing pubkey".to_owned()
        })?;
        let account = entry.get("account").ok_or_else(|| {
            "Raydium exact-pair getProgramAccounts entry missing account".to_owned()
        })?;

        let notification = json!({
            "method": "programNotification",
            "params": {
                "result": {
                    "context": {
                        "slot": slot
                    },
                    "value": {
                        "pubkey": pubkey,
                        "account": account
                    }
                }
            }
        });

        let observation = raydium::parse_program_notification(&notification)?
            .ok_or_else(|| "Raydium exact-pair lookup account did not decode".to_owned())?;

        if observations
            .iter()
            .any(|existing: &RaydiumCpmmAccountObservation| existing.pubkey == observation.pubkey)
        {
            continue;
        }

        observations.push(observation);
    }

    Ok(observations)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::route::{USDC_MINT, WRAPPED_SOL_MINT};
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    const TEST_INTERMEDIATE_MINT: &str = USDC_MINT;

    #[test]
    fn pair_lookup_requests_cover_both_orientations_and_both_mints() -> Result<(), String> {
        let requests = raydium_pair_lookup_requests(WRAPPED_SOL_MINT, TEST_INTERMEDIATE_MINT);

        assert_eq!(requests.len(), 2);

        let expected = [
            (WRAPPED_SOL_MINT, TEST_INTERMEDIATE_MINT),
            (TEST_INTERMEDIATE_MINT, WRAPPED_SOL_MINT),
        ];

        for (request, (token_0_mint, token_1_mint)) in requests.iter().zip(expected) {
            assert_eq!(
                request.get("method").and_then(Value::as_str),
                Some("getProgramAccounts")
            );
            assert_eq!(
                request.pointer("/params/0").and_then(Value::as_str),
                Some(raydium::RAYDIUM_CPMM_PROGRAM_ID)
            );

            let filters = request
                .pointer("/params/1/filters")
                .and_then(Value::as_array)
                .ok_or_else(|| "Raydium pair lookup must contain filters".to_owned())?;

            assert_eq!(
                filters.len(),
                4,
                "exact-pair lookup must never degrade to an anchor-only scan"
            );

            assert_eq!(
                request
                    .pointer("/params/1/filters/0/dataSize")
                    .and_then(Value::as_u64),
                Some(RAYDIUM_POOL_STATE_LEN as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/1/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(RAYDIUM_POOL_STATE_DISCRIMINATOR_B58)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/2/memcmp/offset")
                    .and_then(Value::as_u64),
                Some(RAYDIUM_TOKEN_0_MINT_OFFSET as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/2/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(token_0_mint)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/3/memcmp/offset")
                    .and_then(Value::as_u64),
                Some(RAYDIUM_TOKEN_1_MINT_OFFSET as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/3/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(token_1_mint)
            );
        }

        Ok(())
    }

    #[test]
    fn parser_reuses_raydium_decoder_and_preserves_context_slot() -> Result<(), String> {
        let mut data = vec![0u8; RAYDIUM_POOL_STATE_LEN];
        let discriminator = bs58::decode(RAYDIUM_POOL_STATE_DISCRIMINATOR_B58)
            .into_vec()
            .map_err(|error| format!("test discriminator decode failed: {error}"))?;
        let anchor = bs58::decode(WRAPPED_SOL_MINT)
            .into_vec()
            .map_err(|error| format!("test anchor decode failed: {error}"))?;
        let intermediate = bs58::decode(TEST_INTERMEDIATE_MINT)
            .into_vec()
            .map_err(|error| format!("test intermediate decode failed: {error}"))?;

        data[0..8].copy_from_slice(&discriminator);
        data[RAYDIUM_TOKEN_0_MINT_OFFSET..RAYDIUM_TOKEN_0_MINT_OFFSET + 32]
            .copy_from_slice(&anchor);
        data[RAYDIUM_TOKEN_1_MINT_OFFSET..RAYDIUM_TOKEN_1_MINT_OFFSET + 32]
            .copy_from_slice(&intermediate);

        let payload = json!({
            "jsonrpc": "2.0",
            "result": {
                "context": { "slot": 777 },
                "value": [
                    {
                        "pubkey": "11111111111111111111111111111111",
                        "account": {
                            "data": [BASE64_STANDARD.encode(data), "base64"],
                            "executable": false,
                            "lamports": 1,
                            "owner": raydium::RAYDIUM_CPMM_PROGRAM_ID,
                            "rentEpoch": 0,
                            "space": RAYDIUM_POOL_STATE_LEN
                        }
                    }
                ]
            },
            "id": 9
        });

        let observations = parse_raydium_pair_lookup_response(&payload)?;

        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].slot, 777);
        assert_eq!(observations[0].pool_state.token_0_mint, WRAPPED_SOL_MINT);
        assert_eq!(
            observations[0].pool_state.token_1_mint,
            TEST_INTERMEDIATE_MINT
        );

        Ok(())
    }
}
