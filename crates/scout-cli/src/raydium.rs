use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};

pub const RAYDIUM_CPMM_PROGRAM_ID: &str = "CPMMoo8L3F4NbTegBCKVNunggL7H1ZpdTHKxQB5qKP1C";

const POOL_STATE_LEN: usize = 637;
const POOL_STATE_DISCRIMINATOR: [u8; 8] = [247, 237, 227, 245, 215, 195, 222, 70];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmPoolState {
    pub amm_config: String,
    pub token_0_vault: String,
    pub token_1_vault: String,
    pub token_0_mint: String,
    pub token_1_mint: String,
    pub status: u8,
    pub lp_mint_decimals: u8,
    pub mint_0_decimals: u8,
    pub mint_1_decimals: u8,
    pub lp_supply: u64,
    pub protocol_fees_token_0: u64,
    pub protocol_fees_token_1: u64,
    pub fund_fees_token_0: u64,
    pub fund_fees_token_1: u64,
    pub open_time: u64,
    pub recent_epoch: u64,
    pub creator_fee_on: u8,
    pub enable_creator_fee: bool,
    pub creator_fees_token_0: u64,
    pub creator_fees_token_1: u64,
}

impl RaydiumCpmmPoolState {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "amm_config={} ",
                "mint0={} mint1={} ",
                "vault0={} vault1={} ",
                "status={} ",
                "lp_decimals={} mint0_decimals={} mint1_decimals={} ",
                "lp_supply={} ",
                "protocol_fees0={} protocol_fees1={} ",
                "fund_fees0={} fund_fees1={} ",
                "creator_fee_on={} creator_fee_enabled={} ",
                "creator_fees0={} creator_fees1={} ",
                "open_time={} recent_epoch={}"
            ),
            self.amm_config,
            self.token_0_mint,
            self.token_1_mint,
            self.token_0_vault,
            self.token_1_vault,
            self.status,
            self.lp_mint_decimals,
            self.mint_0_decimals,
            self.mint_1_decimals,
            self.lp_supply,
            self.protocol_fees_token_0,
            self.protocol_fees_token_1,
            self.fund_fees_token_0,
            self.fund_fees_token_1,
            self.creator_fee_on,
            self.enable_creator_fee,
            self.creator_fees_token_0,
            self.creator_fees_token_1,
            self.open_time,
            self.recent_epoch,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RaydiumCpmmAccountObservation {
    pub pubkey: String,
    pub slot: u64,
    pub owner: String,
    pub encoded_data_len: usize,
    pub decoded_data_len: usize,
    pub pool_state: RaydiumCpmmPoolState,
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
                "encoding": "base64",
                "filters": [
                    {
                        "dataSize": POOL_STATE_LEN
                    }
                ]
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

    let encoding = payload
        .pointer("/params/result/value/account/data/1")
        .and_then(Value::as_str)
        .ok_or_else(|| "Raydium notification missing account-data encoding".to_owned())?;

    if encoding != "base64" {
        return Err(format!(
            "unexpected Raydium account-data encoding: {encoding}"
        ));
    }

    let decoded_data = BASE64_STANDARD
        .decode(encoded_data)
        .map_err(|error| format!("invalid Raydium base64 account data: {error}"))?;

    let pool_state = decode_pool_state(&decoded_data)?;

    Ok(Some(RaydiumCpmmAccountObservation {
        pubkey,
        slot,
        owner,
        encoded_data_len: encoded_data.len(),
        decoded_data_len: decoded_data.len(),
        pool_state,
    }))
}

fn decode_pool_state(data: &[u8]) -> Result<RaydiumCpmmPoolState, String> {
    if data.len() != POOL_STATE_LEN {
        return Err(format!(
            "unexpected Raydium PoolState length: expected {POOL_STATE_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = data
        .get(0..POOL_STATE_DISCRIMINATOR.len())
        .ok_or_else(|| "Raydium PoolState missing discriminator".to_owned())?;

    if discriminator != POOL_STATE_DISCRIMINATOR {
        return Err("unexpected Raydium PoolState discriminator".to_owned());
    }

    let mut offset = POOL_STATE_DISCRIMINATOR.len();

    let amm_config = read_pubkey(data, &mut offset)?;
    let _pool_creator = read_pubkey(data, &mut offset)?;
    let token_0_vault = read_pubkey(data, &mut offset)?;
    let token_1_vault = read_pubkey(data, &mut offset)?;
    let _lp_mint = read_pubkey(data, &mut offset)?;
    let token_0_mint = read_pubkey(data, &mut offset)?;
    let token_1_mint = read_pubkey(data, &mut offset)?;
    let _token_0_program = read_pubkey(data, &mut offset)?;
    let _token_1_program = read_pubkey(data, &mut offset)?;
    let _observation_key = read_pubkey(data, &mut offset)?;

    let _auth_bump = read_u8(data, &mut offset)?;
    let status = read_u8(data, &mut offset)?;
    let lp_mint_decimals = read_u8(data, &mut offset)?;
    let mint_0_decimals = read_u8(data, &mut offset)?;
    let mint_1_decimals = read_u8(data, &mut offset)?;

    let lp_supply = read_u64(data, &mut offset)?;
    let protocol_fees_token_0 = read_u64(data, &mut offset)?;
    let protocol_fees_token_1 = read_u64(data, &mut offset)?;
    let fund_fees_token_0 = read_u64(data, &mut offset)?;
    let fund_fees_token_1 = read_u64(data, &mut offset)?;
    let open_time = read_u64(data, &mut offset)?;
    let recent_epoch = read_u64(data, &mut offset)?;

    let creator_fee_on = read_u8(data, &mut offset)?;
    let enable_creator_fee_raw = read_u8(data, &mut offset)?;

    let enable_creator_fee = match enable_creator_fee_raw {
        0 => false,
        1 => true,
        other => {
            return Err(format!(
                "invalid Raydium creator-fee boolean value: {other}"
            ));
        }
    };

    skip(data, &mut offset, 6)?;

    let creator_fees_token_0 = read_u64(data, &mut offset)?;
    let creator_fees_token_1 = read_u64(data, &mut offset)?;

    skip(data, &mut offset, 28 * 8)?;

    if offset != data.len() {
        return Err(format!(
            "Raydium PoolState decoder ended at {offset}, account length is {}",
            data.len()
        ));
    }

    Ok(RaydiumCpmmPoolState {
        amm_config,
        token_0_vault,
        token_1_vault,
        token_0_mint,
        token_1_mint,
        status,
        lp_mint_decimals,
        mint_0_decimals,
        mint_1_decimals,
        lp_supply,
        protocol_fees_token_0,
        protocol_fees_token_1,
        fund_fees_token_0,
        fund_fees_token_1,
        open_time,
        recent_epoch,
        creator_fee_on,
        enable_creator_fee,
        creator_fees_token_0,
        creator_fees_token_1,
    })
}

fn read_pubkey(data: &[u8], offset: &mut usize) -> Result<String, String> {
    let bytes = take::<32>(data, offset)?;
    Ok(bs58::encode(bytes).into_string())
}

fn read_u8(data: &[u8], offset: &mut usize) -> Result<u8, String> {
    let bytes = take::<1>(data, offset)?;
    Ok(bytes[0])
}

fn read_u64(data: &[u8], offset: &mut usize) -> Result<u64, String> {
    Ok(u64::from_le_bytes(take::<8>(data, offset)?))
}

fn skip(data: &[u8], offset: &mut usize, len: usize) -> Result<(), String> {
    let end = offset
        .checked_add(len)
        .ok_or_else(|| "Raydium PoolState offset overflow".to_owned())?;

    if end > data.len() {
        return Err("Raydium PoolState ended unexpectedly".to_owned());
    }

    *offset = end;
    Ok(())
}

fn take<const N: usize>(data: &[u8], offset: &mut usize) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| "Raydium PoolState offset overflow".to_owned())?;

    let slice = data
        .get(*offset..end)
        .ok_or_else(|| "Raydium PoolState ended unexpectedly".to_owned())?;

    let bytes = <[u8; N]>::try_from(slice)
        .map_err(|_| "Raydium PoolState field had unexpected size".to_owned())?;

    *offset = end;
    Ok(bytes)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn subscription_targets_raydium_cpmm_pool_state_size() {
        let request = program_subscribe_request();

        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(RAYDIUM_CPMM_PROGRAM_ID)
        );
        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("programSubscribe")
        );
        assert_eq!(
            request
                .pointer("/params/1/filters/0/dataSize")
                .and_then(Value::as_u64),
            Some(POOL_STATE_LEN as u64)
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
    fn decodes_deterministic_pool_state_fixture() -> Result<(), String> {
        let data = fixture_pool_state();
        let state = decode_pool_state(&data)?;

        assert_eq!(data.len(), POOL_STATE_LEN);
        assert_eq!(state.amm_config, bs58::encode([1u8; 32]).into_string());
        assert_eq!(state.token_0_vault, bs58::encode([3u8; 32]).into_string());
        assert_eq!(state.token_1_vault, bs58::encode([4u8; 32]).into_string());
        assert_eq!(state.token_0_mint, bs58::encode([6u8; 32]).into_string());
        assert_eq!(state.token_1_mint, bs58::encode([7u8; 32]).into_string());
        assert_eq!(state.status, 0);
        assert_eq!(state.lp_mint_decimals, 9);
        assert_eq!(state.mint_0_decimals, 6);
        assert_eq!(state.mint_1_decimals, 6);
        assert_eq!(state.lp_supply, 1_000);
        assert_eq!(state.protocol_fees_token_0, 10);
        assert_eq!(state.protocol_fees_token_1, 11);
        assert_eq!(state.fund_fees_token_0, 12);
        assert_eq!(state.fund_fees_token_1, 13);
        assert_eq!(state.open_time, 1_234_567);
        assert_eq!(state.recent_epoch, 500);
        assert_eq!(state.creator_fee_on, 0);
        assert!(state.enable_creator_fee);
        assert_eq!(state.creator_fees_token_0, 14);
        assert_eq!(state.creator_fees_token_1, 15);

        Ok(())
    }

    #[test]
    fn rejects_wrong_pool_state_discriminator() {
        let mut data = fixture_pool_state();
        data[0] ^= 0xff;

        assert!(decode_pool_state(&data).is_err());
    }

    #[test]
    fn rejects_wrong_pool_state_length() {
        let mut data = fixture_pool_state();
        let _ = data.pop();

        assert!(decode_pool_state(&data).is_err());
    }

    #[test]
    fn parses_and_decodes_read_only_raydium_observation() -> Result<(), String> {
        let data = fixture_pool_state();
        let encoded_data = BASE64_STANDARD.encode(&data);

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
                                encoded_data,
                                "base64"
                            ],
                            "executable": false,
                            "lamports": 1,
                            "owner": RAYDIUM_CPMM_PROGRAM_ID,
                            "rentEpoch": 0,
                            "space": POOL_STATE_LEN
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
        assert_eq!(observation.decoded_data_len, POOL_STATE_LEN);
        assert_eq!(observation.pool_state.lp_supply, 1_000);

        Ok(())
    }

    fn fixture_pool_state() -> Vec<u8> {
        let mut data = Vec::with_capacity(POOL_STATE_LEN);

        data.extend_from_slice(&POOL_STATE_DISCRIMINATOR);

        for seed in 1u8..=10 {
            data.extend(std::iter::repeat(seed).take(32));
        }

        data.extend_from_slice(&[
            250, // auth_bump
            0,   // status
            9,   // lp_mint_decimals
            6,   // mint_0_decimals
            6,   // mint_1_decimals
        ]);

        for value in [1_000u64, 10, 11, 12, 13, 1_234_567, 500] {
            data.extend_from_slice(&value.to_le_bytes());
        }

        data.push(0);
        data.push(1);
        data.extend_from_slice(&[0u8; 6]);

        data.extend_from_slice(&14u64.to_le_bytes());
        data.extend_from_slice(&15u64.to_le_bytes());

        data.extend_from_slice(&[0u8; 28 * 8]);

        data
    }
}
