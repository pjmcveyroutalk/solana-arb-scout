use crate::raydium::{self, RaydiumCpmmAccountObservation};
use crate::route::{USDC_MINT, USDT_MINT, WRAPPED_SOL_MINT};
use serde_json::{json, Value};

const RAYDIUM_POOL_STATE_LEN: usize = 637;
const RAYDIUM_POOL_STATE_DISCRIMINATOR_B58: &str = "iUE1qg7KXeV";
const RAYDIUM_TOKEN_0_MINT_OFFSET: usize = 168;
const RAYDIUM_TOKEN_1_MINT_OFFSET: usize = 200;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumRouteDiscoveryCandidate {
    pub anchor_mint: String,
    pub intermediate_mint: String,
    pub observation: RaydiumCpmmAccountObservation,
}

pub fn raydium_anchor_lookup_requests() -> [Value; 6] {
    [
        raydium_anchor_lookup_request(9, WRAPPED_SOL_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
        raydium_anchor_lookup_request(10, WRAPPED_SOL_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
        raydium_anchor_lookup_request(11, USDC_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
        raydium_anchor_lookup_request(12, USDC_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
        raydium_anchor_lookup_request(13, USDT_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
        raydium_anchor_lookup_request(14, USDT_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
    ]
}

fn raydium_anchor_lookup_request(request_id: u64, anchor_mint: &str, mint_offset: usize) -> Value {
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
                            "offset": mint_offset,
                            "bytes": anchor_mint
                        }
                    }
                ]
            }
        ]
    })
}

pub fn parse_raydium_anchor_lookup_response(
    payload: &Value,
) -> Result<Vec<RaydiumCpmmAccountObservation>, String> {
    if let Some(error) = payload.get("error") {
        return Err(format!(
            "Raydium getProgramAccounts returned an RPC error: {error}"
        ));
    }

    let slot = payload
        .pointer("/result/context/slot")
        .and_then(Value::as_u64)
        .ok_or_else(|| "Raydium getProgramAccounts response missing context slot".to_owned())?;

    let accounts = payload
        .pointer("/result/value")
        .and_then(Value::as_array)
        .ok_or_else(|| "Raydium getProgramAccounts response missing account array".to_owned())?;

    let mut observations = Vec::with_capacity(accounts.len());

    for entry in accounts {
        let pubkey = entry
            .get("pubkey")
            .and_then(Value::as_str)
            .ok_or_else(|| "Raydium getProgramAccounts entry missing pubkey".to_owned())?;
        let account = entry
            .get("account")
            .ok_or_else(|| "Raydium getProgramAccounts entry missing account".to_owned())?;

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
            .ok_or_else(|| "Raydium anchor lookup account did not decode".to_owned())?;

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

pub fn route_candidate_from_observation(
    observation: RaydiumCpmmAccountObservation,
) -> Option<RaydiumRouteDiscoveryCandidate> {
    for anchor_mint in [WRAPPED_SOL_MINT, USDC_MINT, USDT_MINT] {
        if observation.pool_state.token_0_mint == anchor_mint
            && observation.pool_state.token_1_mint != anchor_mint
        {
            return Some(RaydiumRouteDiscoveryCandidate {
                anchor_mint: anchor_mint.to_owned(),
                intermediate_mint: observation.pool_state.token_1_mint.clone(),
                observation,
            });
        }

        if observation.pool_state.token_1_mint == anchor_mint
            && observation.pool_state.token_0_mint != anchor_mint
        {
            return Some(RaydiumRouteDiscoveryCandidate {
                anchor_mint: anchor_mint.to_owned(),
                intermediate_mint: observation.pool_state.token_0_mint.clone(),
                observation,
            });
        }
    }

    None
}

#[cfg(test)]
mod tests {
    use super::*;
    use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};

    const TEST_INTERMEDIATE_MINT: &str = "4k3Dyjzvzp8eMZWUXbBCjEvwSkkk59S5iVJgkX6R";

    #[test]
    fn lookup_requests_cover_all_anchor_mint_positions() {
        let requests = raydium_anchor_lookup_requests();
        assert_eq!(requests.len(), 6);

        let expected = [
            (WRAPPED_SOL_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
            (WRAPPED_SOL_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
            (USDC_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
            (USDC_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
            (USDT_MINT, RAYDIUM_TOKEN_0_MINT_OFFSET),
            (USDT_MINT, RAYDIUM_TOKEN_1_MINT_OFFSET),
        ];

        for (request, (anchor_mint, offset)) in requests.iter().zip(expected) {
            assert_eq!(
                request.get("method").and_then(Value::as_str),
                Some("getProgramAccounts")
            );
            assert_eq!(
                request.pointer("/params/0").and_then(Value::as_str),
                Some(raydium::RAYDIUM_CPMM_PROGRAM_ID)
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
                Some(offset as u64)
            );
            assert_eq!(
                request
                    .pointer("/params/1/filters/2/memcmp/bytes")
                    .and_then(Value::as_str),
                Some(anchor_mint)
            );
        }
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

        let observations = parse_raydium_anchor_lookup_response(&payload)?;
        assert_eq!(observations.len(), 1);
        assert_eq!(observations[0].slot, 777);
        assert_eq!(observations[0].pool_state.token_0_mint, WRAPPED_SOL_MINT);
        assert_eq!(
            observations[0].pool_state.token_1_mint,
            TEST_INTERMEDIATE_MINT
        );

        let candidate = route_candidate_from_observation(observations[0].clone())
            .ok_or_else(|| "test observation did not produce route candidate".to_owned())?;
        assert_eq!(candidate.anchor_mint, WRAPPED_SOL_MINT);
        assert_eq!(candidate.intermediate_mint, TEST_INTERMEDIATE_MINT);

        Ok(())
    }
}
