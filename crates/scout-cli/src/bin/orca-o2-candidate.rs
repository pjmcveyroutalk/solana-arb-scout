#![allow(dead_code)]

#[path = "../orca.rs"]
mod orca;

use orca::OrcaWhirlpoolState;
use orca_whirlpools_core::{
    get_tick_array_start_tick_index, swap_quote_by_input_token, ExactInSwapQuote, OracleFacade,
    TickArrayFacade, TickArrays, TickFacade, TransferFee, WhirlpoolFacade, TICK_ARRAY_SIZE,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const FIXED_TICK_ARRAY_DISCRIMINATOR: [u8; 8] =
    [0x45, 0x61, 0xbd, 0xbe, 0x07, 0x42, 0xbb];

const TICK_SERIALIZED_LEN: usize = 113;
const FIXED_TICK_ARRAY_LEN: usize = 8 + 4 + (TICK_ARRAY_SIZE * TICK_SERIALIZED_LEN) + 32;

fn main() -> Result<(), String> {
    println!("Orca O2 quote-readiness candidate");
    println!("Read-only. No routing admission, signing, submission, or execution.");
    println!("Authoritative quote core: orca_whirlpools_core 2.1.1");
    println!("Rust contract: 1.80");
    Ok(())
}

pub fn bounded_tick_array_start_indexes(
    pool: &OrcaWhirlpoolState,
) -> Result<[i32; 5], String> {
    if pool.tick_spacing == 0 {
        return Err("Orca tick spacing must be greater than zero".to_owned());
    }

    let current =
        get_tick_array_start_tick_index(pool.tick_current_index, pool.tick_spacing);

    let tick_array_size = i32::try_from(TICK_ARRAY_SIZE)
        .map_err(|_| "Orca tick-array size does not fit i32".to_owned())?;

    let offset = i32::from(pool.tick_spacing)
        .checked_mul(tick_array_size)
        .ok_or_else(|| "Orca tick-array offset overflow".to_owned())?;

    let double_offset = offset
        .checked_mul(2)
        .ok_or_else(|| "Orca doubled tick-array offset overflow".to_owned())?;

    let plus_one = current
        .checked_add(offset)
        .ok_or_else(|| "Orca +1 tick-array index overflow".to_owned())?;

    let plus_two = current
        .checked_add(double_offset)
        .ok_or_else(|| "Orca +2 tick-array index overflow".to_owned())?;

    let minus_one = current
        .checked_sub(offset)
        .ok_or_else(|| "Orca -1 tick-array index overflow".to_owned())?;

    let minus_two = current
        .checked_sub(double_offset)
        .ok_or_else(|| "Orca -2 tick-array index overflow".to_owned())?;

    Ok([current, plus_one, plus_two, minus_one, minus_two])
}

pub fn tick_array_pda(
    whirlpool: &str,
    start_tick_index: i32,
) -> Result<String, String> {
    let whirlpool_pubkey = Pubkey::from_str(whirlpool)
        .map_err(|error| format!("invalid Orca Whirlpool pubkey: {error}"))?;

    let program_id = Pubkey::from_str(orca::ORCA_WHIRLPOOL_PROGRAM_ID)
        .map_err(|error| format!("invalid Orca program id: {error}"))?;

    let start_tick_index_string = start_tick_index.to_string();

    let seeds: [&[u8]; 3] = [
        b"tick_array",
        whirlpool_pubkey.as_ref(),
        start_tick_index_string.as_bytes(),
    ];

    let (address, _) = Pubkey::try_find_program_address(&seeds, &program_id)
        .ok_or_else(|| "could not derive Orca tick-array PDA".to_owned())?;

    Ok(address.to_string())
}

pub fn decode_fixed_tick_array(
    data: &[u8],
    expected_whirlpool: &str,
    expected_start_tick_index: i32,
) -> Result<TickArrayFacade, String> {
    if data.len() != FIXED_TICK_ARRAY_LEN {
        return Err(format!(
            concat!(
                "unsupported Orca tick-array representation: ",
                "expected fixed array length {}, got {}"
            ),
            FIXED_TICK_ARRAY_LEN,
            data.len()
        ));
    }

    let discriminator = read_array::<8>(data, 0, "tick-array discriminator")?;

    if discriminator != FIXED_TICK_ARRAY_DISCRIMINATOR {
        return Err(format!(
            "Orca fixed tick-array discriminator mismatch: got {discriminator:?}"
        ));
    }

    let start_tick_index = read_i32(data, 8, "tick-array start index")?;

    if start_tick_index != expected_start_tick_index {
        return Err(format!(
            concat!(
                "Orca tick-array start-index mismatch: ",
                "expected {}, got {}"
            ),
            expected_start_tick_index, start_tick_index
        ));
    }

    let mut ticks = [TickFacade::default(); TICK_ARRAY_SIZE];
    let mut offset = 12usize;

    for tick in &mut ticks {
        let initialized_raw = *data
            .get(offset)
            .ok_or_else(|| "Orca tick initialized byte missing".to_owned())?;

        let initialized = match initialized_raw {
            0 => false,
            1 => true,
            other => {
                return Err(format!(
                    "Orca tick initialized flag invalid: {other}"
                ));
            }
        };

        offset = checked_advance(offset, 1, "tick initialized")?;

        let liquidity_net = read_i128(data, offset, "tick liquidity_net")?;
        offset = checked_advance(offset, 16, "liquidity_net")?;

        let liquidity_gross = read_u128(data, offset, "tick liquidity_gross")?;
        offset = checked_advance(offset, 16, "liquidity_gross")?;

        let fee_growth_outside_a =
            read_u128(data, offset, "tick fee_growth_outside_a")?;
        offset = checked_advance(offset, 16, "fee_growth_outside_a")?;

        let fee_growth_outside_b =
            read_u128(data, offset, "tick fee_growth_outside_b")?;
        offset = checked_advance(offset, 16, "fee_growth_outside_b")?;

        let reward_growth_0 =
            read_u128(data, offset, "tick reward_growth_outside_0")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_0")?;

        let reward_growth_1 =
            read_u128(data, offset, "tick reward_growth_outside_1")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_1")?;

        let reward_growth_2 =
            read_u128(data, offset, "tick reward_growth_outside_2")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_2")?;

        *tick = TickFacade {
            initialized,
            liquidity_net,
            liquidity_gross,
            fee_growth_outside_a,
            fee_growth_outside_b,
            reward_growths_outside: [
                reward_growth_0,
                reward_growth_1,
                reward_growth_2,
            ],
        };
    }

    let whirlpool_bytes =
        read_array::<32>(data, offset, "tick-array Whirlpool identity")?;

    let decoded_whirlpool = Pubkey::new_from_array(whirlpool_bytes).to_string();

    if decoded_whirlpool != expected_whirlpool {
        return Err(format!(
            concat!(
                "Orca tick-array Whirlpool mismatch: ",
                "expected {}, got {}"
            ),
            expected_whirlpool, decoded_whirlpool
        ));
    }

    offset = checked_advance(offset, 32, "tick-array Whirlpool identity")?;

    if offset != data.len() {
        return Err(format!(
            concat!(
                "Orca fixed tick-array decoder did not consume account: ",
                "consumed={} len={}"
            ),
            offset,
            data.len()
        ));
    }

    Ok(TickArrayFacade {
        start_tick_index,
        ticks,
    })
}

pub fn zeroed_tick_array(start_tick_index: i32) -> TickArrayFacade {
    TickArrayFacade {
        start_tick_index,
        ticks: [TickFacade::default(); TICK_ARRAY_SIZE],
    }
}

pub fn whirlpool_facade(pool: &OrcaWhirlpoolState) -> WhirlpoolFacade {
    WhirlpoolFacade {
        fee_tier_index_seed: pool.fee_tier_index_seed.to_le_bytes(),
        tick_spacing: pool.tick_spacing,
        fee_rate: pool.fee_rate,
        protocol_fee_rate: pool.protocol_fee_rate,
        liquidity: pool.liquidity,
        sqrt_price: pool.sqrt_price,
        tick_current_index: pool.tick_current_index,
        fee_growth_global_a: 0,
        fee_growth_global_b: 0,
        reward_last_updated_timestamp: 0,
        reward_infos: Default::default(),
    }
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_input(
    pool: &OrcaWhirlpoolState,
    input_mint: &str,
    amount_in_raw: u64,
    tick_arrays: [TickArrayFacade; 5],
    timestamp: u64,
    oracle: Option<OracleFacade>,
    transfer_fee_a: Option<TransferFee>,
    transfer_fee_b: Option<TransferFee>,
) -> Result<ExactInSwapQuote, String> {
    if amount_in_raw == 0 {
        return Err("Orca exact-input quote amount must be greater than zero".to_owned());
    }

    let specified_token_a = if input_mint == pool.token_mint_a {
        true
    } else if input_mint == pool.token_mint_b {
        false
    } else {
        return Err(format!(
            "input mint {input_mint} is not part of the Orca Whirlpool"
        ));
    };

    match (pool.is_adaptive_fee(), oracle) {
        (true, None) => {
            return Err(
                "adaptive-fee Orca Whirlpool requires Oracle state".to_owned()
            );
        }
        (false, Some(_)) => {
            return Err(
                "non-adaptive Orca Whirlpool must not receive Oracle state".to_owned()
            );
        }
        _ => {}
    }

    if let Some(oracle_state) = oracle {
        if oracle_state.trade_enable_timestamp > timestamp {
            return Err(format!(
                concat!(
                    "adaptive-fee Orca Whirlpool trading is not enabled yet: ",
                    "trade_enable_timestamp={} quote_timestamp={}"
                ),
                oracle_state.trade_enable_timestamp, timestamp
            ));
        }
    }

    swap_quote_by_input_token(
        amount_in_raw,
        specified_token_a,
        0,
        whirlpool_facade(pool),
        oracle,
        TickArrays::Five(
            tick_arrays[0],
            tick_arrays[1],
            tick_arrays[2],
            tick_arrays[3],
            tick_arrays[4],
        ),
        timestamp,
        transfer_fee_a,
        transfer_fee_b,
    )
    .map_err(|error| format!("Orca authoritative exact-input quote failed: {error:?}"))
}

fn checked_advance(
    offset: usize,
    amount: usize,
    label: &str,
) -> Result<usize, String> {
    offset
        .checked_add(amount)
        .ok_or_else(|| format!("Orca {label} offset overflow"))
}

fn read_i32(data: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    Ok(i32::from_le_bytes(read_array::<4>(
        data, offset, label,
    )?))
}

fn read_i128(data: &[u8], offset: usize, label: &str) -> Result<i128, String> {
    Ok(i128::from_le_bytes(read_array::<16>(
        data, offset, label,
    )?))
}

fn read_u128(data: &[u8], offset: usize, label: &str) -> Result<u128, String> {
    Ok(u128::from_le_bytes(read_array::<16>(
        data, offset, label,
    )?))
}

fn read_array<const N: usize>(
    data: &[u8],
    offset: usize,
    label: &str,
) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("Orca {label} offset overflow"))?;

    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("Orca {label} outside account data"))?;

    <[u8; N]>::try_from(bytes)
        .map_err(|_| format!("Orca {label} had invalid byte length"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_pool() -> OrcaWhirlpoolState {
        OrcaWhirlpoolState {
            whirlpools_config: Pubkey::new_unique().to_string(),
            whirlpool_bump: 255,
            tick_spacing: 64,
            fee_tier_index_seed: 64,
            fee_rate: 3_000,
            protocol_fee_rate: 300,
            liquidity: 1_000_000_000,
            sqrt_price: 18_446_744_073_709_551_616u128,
            tick_current_index: 0,
            token_mint_a: Pubkey::new_unique().to_string(),
            token_vault_a: Pubkey::new_unique().to_string(),
            token_mint_b: Pubkey::new_unique().to_string(),
            token_vault_b: Pubkey::new_unique().to_string(),
        }
    }

    #[test]
    fn official_tick_array_pda_vector_matches() -> Result<(), String> {
        let whirlpool = "2kJmUjxWBwL2NGPBV2PiA5hWtmLCqcKY6reQgkrPtaeS";
        let expected = "8PhPzk7n4wU98Z6XCbVtPai2LtXSxYnfjkmgWuoAU8Zy";

        let actual = tick_array_pda(whirlpool, 0)?;

        assert_eq!(actual, expected);
        Ok(())
    }

    #[test]
    fn bounded_window_matches_orca_five_array_strategy() -> Result<(), String> {
        let pool = sample_pool();

        let indexes = bounded_tick_array_start_indexes(&pool)?;

        let tick_array_size = i32::try_from(TICK_ARRAY_SIZE)
            .map_err(|_| "tick-array size does not fit i32".to_owned())?;

        let width = i32::from(pool.tick_spacing)
            .checked_mul(tick_array_size)
            .ok_or_else(|| "tick-array width overflow".to_owned())?;

        let double_width = width
            .checked_mul(2)
            .ok_or_else(|| "double tick-array width overflow".to_owned())?;

        assert_eq!(indexes[0], 0);
        assert_eq!(indexes[1], width);
        assert_eq!(indexes[2], double_width);
        assert_eq!(indexes[3], -width);
        assert_eq!(indexes[4], -double_width);

        Ok(())
    }

    #[test]
    fn fixed_tick_array_decoder_enforces_identity() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique();
        let mut data = vec![0u8; FIXED_TICK_ARRAY_LEN];

        data[0..8].copy_from_slice(&FIXED_TICK_ARRAY_DISCRIMINATOR);
        data[8..12].copy_from_slice(&0i32.to_le_bytes());

        let whirlpool_offset =
            12 + (TICK_ARRAY_SIZE * TICK_SERIALIZED_LEN);

        data[whirlpool_offset..whirlpool_offset + 32]
            .copy_from_slice(whirlpool.as_ref());

        let decoded =
            decode_fixed_tick_array(&data, &whirlpool.to_string(), 0)?;

        assert_eq!(decoded.start_tick_index, 0);
        assert!(decoded.ticks.iter().all(|tick| !tick.initialized));

        match decode_fixed_tick_array(
            &data,
            &Pubkey::new_unique().to_string(),
            0,
        ) {
            Ok(_) => Err("wrong Whirlpool identity was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("Whirlpool mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn unsupported_tick_array_representation_fails_closed() -> Result<(), String> {
        let data = vec![0u8; 64];

        match decode_fixed_tick_array(
            &data,
            &Pubkey::new_unique().to_string(),
            0,
        ) {
            Ok(_) => Err("unsupported tick-array representation was accepted".to_owned()),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn adaptive_pool_requires_oracle_before_quote() -> Result<(), String> {
        let mut pool = sample_pool();
        pool.fee_tier_index_seed = 32;

        let arrays = [
            zeroed_tick_array(0),
            zeroed_tick_array(5_632),
            zeroed_tick_array(11_264),
            zeroed_tick_array(-5_632),
            zeroed_tick_array(-11_264),
        ];

        let input_mint = pool.token_mint_a.clone();

        match quote_exact_input(
            &pool,
            &input_mint,
            1_000,
            arrays,
            1_000,
            None,
            None,
            None,
        ) {
            Ok(_) => Err("adaptive pool quoted without Oracle".to_owned()),
            Err(error) => {
                assert!(error.contains("requires Oracle"));
                Ok(())
            }
        }
    }
}
