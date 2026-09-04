#![allow(dead_code)]

#[path = "../orca.rs"]
mod orca;

use orca::OrcaWhirlpoolState;
use orca_whirlpools_core::{
    get_tick_array_start_tick_index, swap_quote_by_input_token, AdaptiveFeeConstantsFacade,
    AdaptiveFeeVariablesFacade, ExactInSwapQuote, OracleFacade, TickArrayFacade, TickArrays,
    TickFacade, TransferFee, WhirlpoolFacade, WhirlpoolRewardInfoFacade, TICK_ARRAY_SIZE,
};
use solana_pubkey::Pubkey;
use std::str::FromStr;

const WHIRLPOOL_STATE_LEN: usize = 653;
const WHIRLPOOL_DISCRIMINATOR: [u8; 8] = [63, 149, 209, 12, 225, 128, 99, 9];

const WHIRLPOOL_TICK_SPACING_OFFSET: usize = 41;
const WHIRLPOOL_FEE_TIER_INDEX_SEED_OFFSET: usize = 43;
const WHIRLPOOL_FEE_RATE_OFFSET: usize = 45;
const WHIRLPOOL_PROTOCOL_FEE_RATE_OFFSET: usize = 47;
const WHIRLPOOL_LIQUIDITY_OFFSET: usize = 49;
const WHIRLPOOL_SQRT_PRICE_OFFSET: usize = 65;
const WHIRLPOOL_TICK_CURRENT_INDEX_OFFSET: usize = 81;
const WHIRLPOOL_FEE_GROWTH_GLOBAL_A_OFFSET: usize = 165;
const WHIRLPOOL_FEE_GROWTH_GLOBAL_B_OFFSET: usize = 245;
const WHIRLPOOL_REWARD_LAST_UPDATED_TIMESTAMP_OFFSET: usize = 261;
const WHIRLPOOL_REWARD_INFOS_OFFSET: usize = 269;
const WHIRLPOOL_REWARD_INFO_LEN: usize = 128;
const WHIRLPOOL_REWARD_EMISSIONS_OFFSET: usize = 96;
const WHIRLPOOL_REWARD_GROWTH_OFFSET: usize = 112;
const WHIRLPOOL_REWARD_COUNT: usize = 3;

const ORACLE_STATE_LEN: usize = 254;
const ORACLE_DISCRIMINATOR: [u8; 8] = [139, 194, 131, 179, 140, 179, 229, 244];
const ORACLE_WHIRLPOOL_OFFSET: usize = 8;
const ORACLE_TRADE_ENABLE_TIMESTAMP_OFFSET: usize = 40;
const ORACLE_FILTER_PERIOD_OFFSET: usize = 48;
const ORACLE_DECAY_PERIOD_OFFSET: usize = 50;
const ORACLE_REDUCTION_FACTOR_OFFSET: usize = 52;
const ORACLE_ADAPTIVE_FEE_CONTROL_FACTOR_OFFSET: usize = 54;
const ORACLE_MAX_VOLATILITY_ACCUMULATOR_OFFSET: usize = 58;
const ORACLE_TICK_GROUP_SIZE_OFFSET: usize = 62;
const ORACLE_MAJOR_SWAP_THRESHOLD_TICKS_OFFSET: usize = 64;
const ORACLE_LAST_REFERENCE_UPDATE_TIMESTAMP_OFFSET: usize = 82;
const ORACLE_LAST_MAJOR_SWAP_TIMESTAMP_OFFSET: usize = 90;
const ORACLE_VOLATILITY_REFERENCE_OFFSET: usize = 98;
const ORACLE_TICK_GROUP_INDEX_REFERENCE_OFFSET: usize = 102;
const ORACLE_VOLATILITY_ACCUMULATOR_OFFSET: usize = 106;

const FIXED_TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [0x45, 0x61, 0xbd, 0xbe, 0x6e, 0x07, 0x42, 0xbb];
const TICK_SERIALIZED_LEN: usize = 113;
const FIXED_TICK_ARRAY_LEN: usize = 8 + 4 + (TICK_ARRAY_SIZE * TICK_SERIALIZED_LEN) + 32;

const DYNAMIC_TICK_ARRAY_DISCRIMINATOR: [u8; 8] = [17, 216, 246, 142, 225, 199, 218, 56];
const DYNAMIC_TICK_ARRAY_START_TICK_INDEX_OFFSET: usize = 8;
const DYNAMIC_TICK_ARRAY_WHIRLPOOL_OFFSET: usize = 12;
const DYNAMIC_TICK_ARRAY_BITMAP_OFFSET: usize = 44;
const DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET: usize = 60;
const DYNAMIC_TICK_UNINITIALIZED_LEN: usize = 1;
const DYNAMIC_TICK_INITIALIZED_LEN: usize = 113;
const DYNAMIC_TICK_DATA_LEN: usize = DYNAMIC_TICK_INITIALIZED_LEN - DYNAMIC_TICK_UNINITIALIZED_LEN;
const DYNAMIC_TICK_ARRAY_MIN_LEN: usize =
    DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET + (TICK_ARRAY_SIZE * DYNAMIC_TICK_UNINITIALIZED_LEN);
const DYNAMIC_TICK_ARRAY_MAX_LEN: usize =
    DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET + (TICK_ARRAY_SIZE * DYNAMIC_TICK_INITIALIZED_LEN);

fn main() -> Result<(), String> {
    println!("Orca O2 quote-readiness candidate");
    println!("Read-only. No routing admission, signing, submission, or execution.");
    println!("Authoritative quote core: orca_whirlpools_core 2.1.1");
    println!("Rust contract: 1.80");
    Ok(())
}

pub fn bounded_tick_array_start_indexes(pool: &OrcaWhirlpoolState) -> Result<[i32; 5], String> {
    if pool.tick_spacing == 0 {
        return Err("Orca tick spacing must be greater than zero".to_owned());
    }

    let current = get_tick_array_start_tick_index(pool.tick_current_index, pool.tick_spacing);

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

pub fn tick_array_pda(whirlpool: &str, start_tick_index: i32) -> Result<String, String> {
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

pub fn oracle_pda(whirlpool: &str) -> Result<String, String> {
    let whirlpool_pubkey = Pubkey::from_str(whirlpool)
        .map_err(|error| format!("invalid Orca Whirlpool pubkey: {error}"))?;

    let program_id = Pubkey::from_str(orca::ORCA_WHIRLPOOL_PROGRAM_ID)
        .map_err(|error| format!("invalid Orca program id: {error}"))?;

    let seeds: [&[u8]; 2] = [b"oracle", whirlpool_pubkey.as_ref()];

    let (address, _) = Pubkey::try_find_program_address(&seeds, &program_id)
        .ok_or_else(|| "could not derive Orca Oracle PDA".to_owned())?;

    Ok(address.to_string())
}

pub fn decode_whirlpool_facade(data: &[u8]) -> Result<WhirlpoolFacade, String> {
    if data.len() != WHIRLPOOL_STATE_LEN {
        return Err(format!(
            "Orca Whirlpool account length mismatch: expected {WHIRLPOOL_STATE_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = read_array::<8>(data, 0, "Whirlpool discriminator")?;

    if discriminator != WHIRLPOOL_DISCRIMINATOR {
        return Err(format!(
            "Orca Whirlpool discriminator mismatch: expected {:?}, got {:?}",
            WHIRLPOOL_DISCRIMINATOR, discriminator
        ));
    }

    let fee_tier_index_seed = read_array::<2>(
        data,
        WHIRLPOOL_FEE_TIER_INDEX_SEED_OFFSET,
        "fee_tier_index_seed",
    )?;

    let tick_spacing = read_u16(
        data,
        WHIRLPOOL_TICK_SPACING_OFFSET,
        "Whirlpool tick_spacing",
    )?;
    let fee_rate = read_u16(data, WHIRLPOOL_FEE_RATE_OFFSET, "Whirlpool fee_rate")?;
    let protocol_fee_rate = read_u16(
        data,
        WHIRLPOOL_PROTOCOL_FEE_RATE_OFFSET,
        "Whirlpool protocol_fee_rate",
    )?;
    let liquidity = read_u128(data, WHIRLPOOL_LIQUIDITY_OFFSET, "Whirlpool liquidity")?;
    let sqrt_price = read_u128(data, WHIRLPOOL_SQRT_PRICE_OFFSET, "Whirlpool sqrt_price")?;
    let tick_current_index = read_i32(
        data,
        WHIRLPOOL_TICK_CURRENT_INDEX_OFFSET,
        "Whirlpool tick_current_index",
    )?;
    let fee_growth_global_a = read_u128(
        data,
        WHIRLPOOL_FEE_GROWTH_GLOBAL_A_OFFSET,
        "Whirlpool fee_growth_global_a",
    )?;
    let fee_growth_global_b = read_u128(
        data,
        WHIRLPOOL_FEE_GROWTH_GLOBAL_B_OFFSET,
        "Whirlpool fee_growth_global_b",
    )?;
    let reward_last_updated_timestamp = read_u64(
        data,
        WHIRLPOOL_REWARD_LAST_UPDATED_TIMESTAMP_OFFSET,
        "Whirlpool reward_last_updated_timestamp",
    )?;

    let mut reward_infos = [WhirlpoolRewardInfoFacade::default(); WHIRLPOOL_REWARD_COUNT];

    for (reward_index, reward_info) in reward_infos.iter_mut().enumerate() {
        let reward_stride = reward_index
            .checked_mul(WHIRLPOOL_REWARD_INFO_LEN)
            .ok_or_else(|| "Orca reward-info index overflow".to_owned())?;

        let reward_offset = WHIRLPOOL_REWARD_INFOS_OFFSET
            .checked_add(reward_stride)
            .ok_or_else(|| "Orca reward-info offset overflow".to_owned())?;

        let emissions_offset = reward_offset
            .checked_add(WHIRLPOOL_REWARD_EMISSIONS_OFFSET)
            .ok_or_else(|| "Orca reward emissions offset overflow".to_owned())?;

        let growth_offset = reward_offset
            .checked_add(WHIRLPOOL_REWARD_GROWTH_OFFSET)
            .ok_or_else(|| "Orca reward growth offset overflow".to_owned())?;

        *reward_info = WhirlpoolRewardInfoFacade {
            emissions_per_second_x64: read_u128(
                data,
                emissions_offset,
                "Whirlpool reward emissions_per_second_x64",
            )?,
            growth_global_x64: read_u128(
                data,
                growth_offset,
                "Whirlpool reward growth_global_x64",
            )?,
        };
    }

    Ok(WhirlpoolFacade {
        fee_tier_index_seed,
        tick_spacing,
        fee_rate,
        protocol_fee_rate,
        liquidity,
        sqrt_price,
        tick_current_index,
        fee_growth_global_a,
        fee_growth_global_b,
        reward_last_updated_timestamp,
        reward_infos,
    })
}

pub fn decode_oracle_facade(
    data: &[u8],
    account_owner: &str,
    expected_whirlpool: &str,
) -> Result<OracleFacade, String> {
    if account_owner != orca::ORCA_WHIRLPOOL_PROGRAM_ID {
        return Err(format!(
            "Orca Oracle owner mismatch: expected {}, got {}",
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            account_owner
        ));
    }

    if data.len() != ORACLE_STATE_LEN {
        return Err(format!(
            "Orca Oracle account length mismatch: expected {ORACLE_STATE_LEN}, got {}",
            data.len()
        ));
    }

    let discriminator = read_array::<8>(data, 0, "Oracle discriminator")?;

    if discriminator != ORACLE_DISCRIMINATOR {
        return Err(format!(
            "Orca Oracle discriminator mismatch: expected {:?}, got {:?}",
            ORACLE_DISCRIMINATOR, discriminator
        ));
    }

    let whirlpool_bytes = read_array::<32>(data, ORACLE_WHIRLPOOL_OFFSET, "Oracle Whirlpool")?;
    let decoded_whirlpool = Pubkey::new_from_array(whirlpool_bytes).to_string();

    if decoded_whirlpool != expected_whirlpool {
        return Err(format!(
            "Orca Oracle Whirlpool mismatch: expected {expected_whirlpool}, got {decoded_whirlpool}"
        ));
    }

    Ok(OracleFacade {
        trade_enable_timestamp: read_u64(
            data,
            ORACLE_TRADE_ENABLE_TIMESTAMP_OFFSET,
            "Oracle trade_enable_timestamp",
        )?,
        adaptive_fee_constants: AdaptiveFeeConstantsFacade {
            filter_period: read_u16(data, ORACLE_FILTER_PERIOD_OFFSET, "Oracle filter_period")?,
            decay_period: read_u16(data, ORACLE_DECAY_PERIOD_OFFSET, "Oracle decay_period")?,
            reduction_factor: read_u16(
                data,
                ORACLE_REDUCTION_FACTOR_OFFSET,
                "Oracle reduction_factor",
            )?,
            adaptive_fee_control_factor: read_u32(
                data,
                ORACLE_ADAPTIVE_FEE_CONTROL_FACTOR_OFFSET,
                "Oracle adaptive_fee_control_factor",
            )?,
            max_volatility_accumulator: read_u32(
                data,
                ORACLE_MAX_VOLATILITY_ACCUMULATOR_OFFSET,
                "Oracle max_volatility_accumulator",
            )?,
            tick_group_size: read_u16(
                data,
                ORACLE_TICK_GROUP_SIZE_OFFSET,
                "Oracle tick_group_size",
            )?,
            major_swap_threshold_ticks: read_u16(
                data,
                ORACLE_MAJOR_SWAP_THRESHOLD_TICKS_OFFSET,
                "Oracle major_swap_threshold_ticks",
            )?,
        },
        adaptive_fee_variables: AdaptiveFeeVariablesFacade {
            last_reference_update_timestamp: read_u64(
                data,
                ORACLE_LAST_REFERENCE_UPDATE_TIMESTAMP_OFFSET,
                "Oracle last_reference_update_timestamp",
            )?,
            last_major_swap_timestamp: read_u64(
                data,
                ORACLE_LAST_MAJOR_SWAP_TIMESTAMP_OFFSET,
                "Oracle last_major_swap_timestamp",
            )?,
            volatility_reference: read_u32(
                data,
                ORACLE_VOLATILITY_REFERENCE_OFFSET,
                "Oracle volatility_reference",
            )?,
            tick_group_index_reference: read_i32(
                data,
                ORACLE_TICK_GROUP_INDEX_REFERENCE_OFFSET,
                "Oracle tick_group_index_reference",
            )?,
            volatility_accumulator: read_u32(
                data,
                ORACLE_VOLATILITY_ACCUMULATOR_OFFSET,
                "Oracle volatility_accumulator",
            )?,
        },
    })
}

pub fn verify_whirlpool_facade_matches_pool(
    pool: &OrcaWhirlpoolState,
    facade: &WhirlpoolFacade,
) -> Result<(), String> {
    if facade.tick_spacing != pool.tick_spacing {
        return Err("Orca Whirlpool facade tick_spacing mismatch".to_owned());
    }

    if u16::from_le_bytes(facade.fee_tier_index_seed) != pool.fee_tier_index_seed {
        return Err("Orca Whirlpool facade fee_tier_index_seed mismatch".to_owned());
    }

    if facade.fee_rate != pool.fee_rate {
        return Err("Orca Whirlpool facade fee_rate mismatch".to_owned());
    }

    if facade.protocol_fee_rate != pool.protocol_fee_rate {
        return Err("Orca Whirlpool facade protocol_fee_rate mismatch".to_owned());
    }

    if facade.liquidity != pool.liquidity {
        return Err("Orca Whirlpool facade liquidity mismatch".to_owned());
    }

    if facade.sqrt_price != pool.sqrt_price {
        return Err("Orca Whirlpool facade sqrt_price mismatch".to_owned());
    }

    if facade.tick_current_index != pool.tick_current_index {
        return Err("Orca Whirlpool facade tick_current_index mismatch".to_owned());
    }

    Ok(())
}

pub fn decode_tick_array_account(
    data: &[u8],
    account_owner: &str,
    expected_whirlpool: &str,
    expected_start_tick_index: i32,
) -> Result<TickArrayFacade, String> {
    if account_owner != orca::ORCA_WHIRLPOOL_PROGRAM_ID {
        return Err(format!(
            "Orca tick-array owner mismatch: expected {}, got {}",
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            account_owner
        ));
    }

    let discriminator = read_array::<8>(data, 0, "tick-array discriminator")?;

    if discriminator == FIXED_TICK_ARRAY_DISCRIMINATOR {
        return decode_fixed_tick_array(data, expected_whirlpool, expected_start_tick_index);
    }

    if discriminator == DYNAMIC_TICK_ARRAY_DISCRIMINATOR {
        return decode_dynamic_tick_array(data, expected_whirlpool, expected_start_tick_index);
    }

    Err(format!(
        "unsupported Orca tick-array discriminator: {discriminator:?}"
    ))
}

pub fn decode_fixed_tick_array(
    data: &[u8],
    expected_whirlpool: &str,
    expected_start_tick_index: i32,
) -> Result<TickArrayFacade, String> {
    if data.len() != FIXED_TICK_ARRAY_LEN {
        return Err(format!(
            concat!(
                "unsupported Orca fixed tick-array length: ",
                "expected {}, got {}"
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
                return Err(format!("Orca tick initialized flag invalid: {other}"));
            }
        };

        offset = checked_advance(offset, 1, "tick initialized")?;

        let liquidity_net = read_i128(data, offset, "tick liquidity_net")?;
        offset = checked_advance(offset, 16, "liquidity_net")?;

        let liquidity_gross = read_u128(data, offset, "tick liquidity_gross")?;
        offset = checked_advance(offset, 16, "liquidity_gross")?;

        let fee_growth_outside_a = read_u128(data, offset, "tick fee_growth_outside_a")?;
        offset = checked_advance(offset, 16, "fee_growth_outside_a")?;

        let fee_growth_outside_b = read_u128(data, offset, "tick fee_growth_outside_b")?;
        offset = checked_advance(offset, 16, "fee_growth_outside_b")?;

        let reward_growth_0 = read_u128(data, offset, "tick reward_growth_outside_0")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_0")?;

        let reward_growth_1 = read_u128(data, offset, "tick reward_growth_outside_1")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_1")?;

        let reward_growth_2 = read_u128(data, offset, "tick reward_growth_outside_2")?;
        offset = checked_advance(offset, 16, "reward_growth_outside_2")?;

        *tick = TickFacade {
            initialized,
            liquidity_net,
            liquidity_gross,
            fee_growth_outside_a,
            fee_growth_outside_b,
            reward_growths_outside: [reward_growth_0, reward_growth_1, reward_growth_2],
        };
    }

    let whirlpool_bytes = read_array::<32>(data, offset, "tick-array Whirlpool identity")?;
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

pub fn decode_dynamic_tick_array(
    data: &[u8],
    expected_whirlpool: &str,
    expected_start_tick_index: i32,
) -> Result<TickArrayFacade, String> {
    if !(DYNAMIC_TICK_ARRAY_MIN_LEN..=DYNAMIC_TICK_ARRAY_MAX_LEN).contains(&data.len()) {
        return Err(format!(
            concat!(
                "Orca dynamic tick-array length outside supported bounds: ",
                "min={} max={} got={}"
            ),
            DYNAMIC_TICK_ARRAY_MIN_LEN,
            DYNAMIC_TICK_ARRAY_MAX_LEN,
            data.len()
        ));
    }

    let discriminator = read_array::<8>(data, 0, "dynamic tick-array discriminator")?;

    if discriminator != DYNAMIC_TICK_ARRAY_DISCRIMINATOR {
        return Err(format!(
            "Orca dynamic tick-array discriminator mismatch: got {discriminator:?}"
        ));
    }

    let start_tick_index = read_i32(
        data,
        DYNAMIC_TICK_ARRAY_START_TICK_INDEX_OFFSET,
        "dynamic tick-array start index",
    )?;

    if start_tick_index != expected_start_tick_index {
        return Err(format!(
            concat!(
                "Orca dynamic tick-array start-index mismatch: ",
                "expected {}, got {}"
            ),
            expected_start_tick_index, start_tick_index
        ));
    }

    let whirlpool_bytes = read_array::<32>(
        data,
        DYNAMIC_TICK_ARRAY_WHIRLPOOL_OFFSET,
        "dynamic tick-array Whirlpool identity",
    )?;
    let decoded_whirlpool = Pubkey::new_from_array(whirlpool_bytes).to_string();

    if decoded_whirlpool != expected_whirlpool {
        return Err(format!(
            concat!(
                "Orca dynamic tick-array Whirlpool mismatch: ",
                "expected {}, got {}"
            ),
            expected_whirlpool, decoded_whirlpool
        ));
    }

    let tick_bitmap = read_u128(
        data,
        DYNAMIC_TICK_ARRAY_BITMAP_OFFSET,
        "dynamic tick-array bitmap",
    )?;

    if (tick_bitmap >> TICK_ARRAY_SIZE) != 0 {
        return Err("Orca dynamic tick-array bitmap has bits outside the 88-tick range".to_owned());
    }

    let initialized_tick_count = usize::try_from(tick_bitmap.count_ones())
        .map_err(|_| "Orca dynamic tick count does not fit usize".to_owned())?;

    let initialized_extra = initialized_tick_count
        .checked_mul(DYNAMIC_TICK_DATA_LEN)
        .ok_or_else(|| "Orca dynamic tick-array initialized-size overflow".to_owned())?;

    let expected_len = DYNAMIC_TICK_ARRAY_MIN_LEN
        .checked_add(initialized_extra)
        .ok_or_else(|| "Orca dynamic tick-array expected-length overflow".to_owned())?;

    if data.len() != expected_len {
        return Err(format!(
            concat!(
                "Orca dynamic tick-array bitmap/length mismatch: ",
                "expected {} bytes for {} initialized ticks, got {}"
            ),
            expected_len,
            initialized_tick_count,
            data.len()
        ));
    }

    let mut ticks = [TickFacade::default(); TICK_ARRAY_SIZE];
    let mut offset = DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET;

    for (tick_index, tick) in ticks.iter_mut().enumerate() {
        let tag = *data
            .get(offset)
            .ok_or_else(|| "Orca dynamic tick enum tag missing".to_owned())?;

        let initialized = (tick_bitmap & (1u128 << tick_index)) != 0;

        match (initialized, tag) {
            (false, 0) => {
                offset = checked_advance(offset, 1, "dynamic uninitialized tick")?;
            }
            (true, 1) => {
                offset = checked_advance(offset, 1, "dynamic initialized tick tag")?;

                let liquidity_net = read_i128(data, offset, "dynamic tick liquidity_net")?;
                offset = checked_advance(offset, 16, "dynamic liquidity_net")?;

                let liquidity_gross = read_u128(data, offset, "dynamic tick liquidity_gross")?;
                offset = checked_advance(offset, 16, "dynamic liquidity_gross")?;

                let fee_growth_outside_a =
                    read_u128(data, offset, "dynamic tick fee_growth_outside_a")?;
                offset = checked_advance(offset, 16, "dynamic fee_growth_outside_a")?;

                let fee_growth_outside_b =
                    read_u128(data, offset, "dynamic tick fee_growth_outside_b")?;
                offset = checked_advance(offset, 16, "dynamic fee_growth_outside_b")?;

                let reward_growth_0 =
                    read_u128(data, offset, "dynamic tick reward_growth_outside_0")?;
                offset = checked_advance(offset, 16, "dynamic reward_growth_outside_0")?;

                let reward_growth_1 =
                    read_u128(data, offset, "dynamic tick reward_growth_outside_1")?;
                offset = checked_advance(offset, 16, "dynamic reward_growth_outside_1")?;

                let reward_growth_2 =
                    read_u128(data, offset, "dynamic tick reward_growth_outside_2")?;
                offset = checked_advance(offset, 16, "dynamic reward_growth_outside_2")?;

                *tick = TickFacade {
                    initialized: true,
                    liquidity_net,
                    liquidity_gross,
                    fee_growth_outside_a,
                    fee_growth_outside_b,
                    reward_growths_outside: [reward_growth_0, reward_growth_1, reward_growth_2],
                };
            }
            (false, other) => {
                return Err(format!(
                    concat!(
                        "Orca dynamic tick bitmap/tag mismatch at offset {}: ",
                        "bitmap=uninitialized tag={}"
                    ),
                    tick_index, other
                ));
            }
            (true, other) => {
                return Err(format!(
                    concat!(
                        "Orca dynamic tick bitmap/tag mismatch at offset {}: ",
                        "bitmap=initialized tag={}"
                    ),
                    tick_index, other
                ));
            }
        }
    }

    if offset != data.len() {
        return Err(format!(
            concat!(
                "Orca dynamic tick-array decoder did not consume account: ",
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

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_input(
    pool: &OrcaWhirlpoolState,
    whirlpool: WhirlpoolFacade,
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

    verify_whirlpool_facade_matches_pool(pool, &whirlpool)?;

    let specified_token_a = if input_mint == pool.token_mint_a {
        true
    } else if input_mint == pool.token_mint_b {
        false
    } else {
        return Err(format!(
            "input mint {input_mint} is not part of the Orca Whirlpool"
        ));
    };

    match (whirlpool.is_initialized_with_adaptive_fee(), oracle) {
        (true, None) => {
            return Err("adaptive-fee Orca Whirlpool requires Oracle state".to_owned());
        }
        (false, Some(_)) => {
            return Err("non-adaptive Orca Whirlpool must not receive Oracle state".to_owned());
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
        whirlpool,
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

fn checked_advance(offset: usize, amount: usize, label: &str) -> Result<usize, String> {
    offset
        .checked_add(amount)
        .ok_or_else(|| format!("Orca {label} offset overflow"))
}

fn read_u16(data: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    Ok(u16::from_le_bytes(read_array::<2>(data, offset, label)?))
}

fn read_u32(data: &[u8], offset: usize, label: &str) -> Result<u32, String> {
    Ok(u32::from_le_bytes(read_array::<4>(data, offset, label)?))
}

fn read_i32(data: &[u8], offset: usize, label: &str) -> Result<i32, String> {
    Ok(i32::from_le_bytes(read_array::<4>(data, offset, label)?))
}

fn read_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    Ok(u64::from_le_bytes(read_array::<8>(data, offset, label)?))
}

fn read_i128(data: &[u8], offset: usize, label: &str) -> Result<i128, String> {
    Ok(i128::from_le_bytes(read_array::<16>(data, offset, label)?))
}

fn read_u128(data: &[u8], offset: usize, label: &str) -> Result<u128, String> {
    Ok(u128::from_le_bytes(read_array::<16>(data, offset, label)?))
}

fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("Orca {label} offset overflow"))?;

    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("Orca {label} outside account data"))?;

    <[u8; N]>::try_from(bytes).map_err(|_| format!("Orca {label} had invalid byte length"))
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

    fn sample_whirlpool_account(pool: &OrcaWhirlpoolState) -> Vec<u8> {
        let mut data = vec![0u8; WHIRLPOOL_STATE_LEN];

        data[0..8].copy_from_slice(&WHIRLPOOL_DISCRIMINATOR);
        data[WHIRLPOOL_TICK_SPACING_OFFSET..WHIRLPOOL_TICK_SPACING_OFFSET + 2]
            .copy_from_slice(&pool.tick_spacing.to_le_bytes());
        data[WHIRLPOOL_FEE_TIER_INDEX_SEED_OFFSET..WHIRLPOOL_FEE_TIER_INDEX_SEED_OFFSET + 2]
            .copy_from_slice(&pool.fee_tier_index_seed.to_le_bytes());
        data[WHIRLPOOL_FEE_RATE_OFFSET..WHIRLPOOL_FEE_RATE_OFFSET + 2]
            .copy_from_slice(&pool.fee_rate.to_le_bytes());
        data[WHIRLPOOL_PROTOCOL_FEE_RATE_OFFSET..WHIRLPOOL_PROTOCOL_FEE_RATE_OFFSET + 2]
            .copy_from_slice(&pool.protocol_fee_rate.to_le_bytes());
        data[WHIRLPOOL_LIQUIDITY_OFFSET..WHIRLPOOL_LIQUIDITY_OFFSET + 16]
            .copy_from_slice(&pool.liquidity.to_le_bytes());
        data[WHIRLPOOL_SQRT_PRICE_OFFSET..WHIRLPOOL_SQRT_PRICE_OFFSET + 16]
            .copy_from_slice(&pool.sqrt_price.to_le_bytes());
        data[WHIRLPOOL_TICK_CURRENT_INDEX_OFFSET..WHIRLPOOL_TICK_CURRENT_INDEX_OFFSET + 4]
            .copy_from_slice(&pool.tick_current_index.to_le_bytes());

        data[WHIRLPOOL_FEE_GROWTH_GLOBAL_A_OFFSET..WHIRLPOOL_FEE_GROWTH_GLOBAL_A_OFFSET + 16]
            .copy_from_slice(&111u128.to_le_bytes());
        data[WHIRLPOOL_FEE_GROWTH_GLOBAL_B_OFFSET..WHIRLPOOL_FEE_GROWTH_GLOBAL_B_OFFSET + 16]
            .copy_from_slice(&222u128.to_le_bytes());
        data[WHIRLPOOL_REWARD_LAST_UPDATED_TIMESTAMP_OFFSET
            ..WHIRLPOOL_REWARD_LAST_UPDATED_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&333u64.to_le_bytes());

        for reward_index in 0..WHIRLPOOL_REWARD_COUNT {
            let reward_offset =
                WHIRLPOOL_REWARD_INFOS_OFFSET + (reward_index * WHIRLPOOL_REWARD_INFO_LEN);
            let emissions_offset = reward_offset + WHIRLPOOL_REWARD_EMISSIONS_OFFSET;
            let growth_offset = reward_offset + WHIRLPOOL_REWARD_GROWTH_OFFSET;

            let emissions = 1_000u128 + reward_index as u128;
            let growth = 2_000u128 + reward_index as u128;

            data[emissions_offset..emissions_offset + 16].copy_from_slice(&emissions.to_le_bytes());
            data[growth_offset..growth_offset + 16].copy_from_slice(&growth.to_le_bytes());
        }

        data
    }

    fn sample_oracle_account(
        whirlpool: &str,
        trade_enable_timestamp: u64,
    ) -> Result<Vec<u8>, String> {
        let whirlpool_pubkey = Pubkey::from_str(whirlpool)
            .map_err(|error| format!("invalid test Whirlpool pubkey: {error}"))?;
        let mut data = vec![0u8; ORACLE_STATE_LEN];

        data[0..8].copy_from_slice(&ORACLE_DISCRIMINATOR);
        data[ORACLE_WHIRLPOOL_OFFSET..ORACLE_WHIRLPOOL_OFFSET + 32]
            .copy_from_slice(whirlpool_pubkey.as_ref());
        data[ORACLE_TRADE_ENABLE_TIMESTAMP_OFFSET..ORACLE_TRADE_ENABLE_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&trade_enable_timestamp.to_le_bytes());
        data[ORACLE_FILTER_PERIOD_OFFSET..ORACLE_FILTER_PERIOD_OFFSET + 2]
            .copy_from_slice(&30u16.to_le_bytes());
        data[ORACLE_DECAY_PERIOD_OFFSET..ORACLE_DECAY_PERIOD_OFFSET + 2]
            .copy_from_slice(&120u16.to_le_bytes());
        data[ORACLE_REDUCTION_FACTOR_OFFSET..ORACLE_REDUCTION_FACTOR_OFFSET + 2]
            .copy_from_slice(&5_000u16.to_le_bytes());
        data[ORACLE_ADAPTIVE_FEE_CONTROL_FACTOR_OFFSET
            ..ORACLE_ADAPTIVE_FEE_CONTROL_FACTOR_OFFSET + 4]
            .copy_from_slice(&9_000u32.to_le_bytes());
        data[ORACLE_MAX_VOLATILITY_ACCUMULATOR_OFFSET
            ..ORACLE_MAX_VOLATILITY_ACCUMULATOR_OFFSET + 4]
            .copy_from_slice(&88_000u32.to_le_bytes());
        data[ORACLE_TICK_GROUP_SIZE_OFFSET..ORACLE_TICK_GROUP_SIZE_OFFSET + 2]
            .copy_from_slice(&64u16.to_le_bytes());
        data[ORACLE_MAJOR_SWAP_THRESHOLD_TICKS_OFFSET
            ..ORACLE_MAJOR_SWAP_THRESHOLD_TICKS_OFFSET + 2]
            .copy_from_slice(&32u16.to_le_bytes());
        data[ORACLE_LAST_REFERENCE_UPDATE_TIMESTAMP_OFFSET
            ..ORACLE_LAST_REFERENCE_UPDATE_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&777u64.to_le_bytes());
        data[ORACLE_LAST_MAJOR_SWAP_TIMESTAMP_OFFSET..ORACLE_LAST_MAJOR_SWAP_TIMESTAMP_OFFSET + 8]
            .copy_from_slice(&778u64.to_le_bytes());
        data[ORACLE_VOLATILITY_REFERENCE_OFFSET..ORACLE_VOLATILITY_REFERENCE_OFFSET + 4]
            .copy_from_slice(&123u32.to_le_bytes());
        data[ORACLE_TICK_GROUP_INDEX_REFERENCE_OFFSET
            ..ORACLE_TICK_GROUP_INDEX_REFERENCE_OFFSET + 4]
            .copy_from_slice(&(-7i32).to_le_bytes());
        data[ORACLE_VOLATILITY_ACCUMULATOR_OFFSET..ORACLE_VOLATILITY_ACCUMULATOR_OFFSET + 4]
            .copy_from_slice(&456u32.to_le_bytes());

        Ok(data)
    }

    fn sample_dynamic_tick_array(
        whirlpool: &str,
        start_tick_index: i32,
        initialized_offsets: &[usize],
    ) -> Result<Vec<u8>, String> {
        let whirlpool_pubkey = Pubkey::from_str(whirlpool)
            .map_err(|error| format!("invalid test Whirlpool pubkey: {error}"))?;

        let mut tick_bitmap = 0u128;

        for tick_offset in initialized_offsets {
            if *tick_offset >= TICK_ARRAY_SIZE {
                return Err(format!(
                    "test dynamic tick offset outside array: {tick_offset}"
                ));
            }

            tick_bitmap |= 1u128 << tick_offset;
        }

        let initialized_count = initialized_offsets.len();
        let initialized_extra = initialized_count
            .checked_mul(DYNAMIC_TICK_DATA_LEN)
            .ok_or_else(|| "test dynamic tick-array size overflow".to_owned())?;
        let account_len = DYNAMIC_TICK_ARRAY_MIN_LEN
            .checked_add(initialized_extra)
            .ok_or_else(|| "test dynamic tick-array length overflow".to_owned())?;

        let mut data = vec![0u8; account_len];

        data[0..8].copy_from_slice(&DYNAMIC_TICK_ARRAY_DISCRIMINATOR);
        data[DYNAMIC_TICK_ARRAY_START_TICK_INDEX_OFFSET
            ..DYNAMIC_TICK_ARRAY_START_TICK_INDEX_OFFSET + 4]
            .copy_from_slice(&start_tick_index.to_le_bytes());
        data[DYNAMIC_TICK_ARRAY_WHIRLPOOL_OFFSET..DYNAMIC_TICK_ARRAY_WHIRLPOOL_OFFSET + 32]
            .copy_from_slice(whirlpool_pubkey.as_ref());
        data[DYNAMIC_TICK_ARRAY_BITMAP_OFFSET..DYNAMIC_TICK_ARRAY_BITMAP_OFFSET + 16]
            .copy_from_slice(&tick_bitmap.to_le_bytes());

        let mut offset = DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET;

        for tick_index in 0..TICK_ARRAY_SIZE {
            let initialized = (tick_bitmap & (1u128 << tick_index)) != 0;

            if !initialized {
                data[offset] = 0;
                offset += 1;
                continue;
            }

            data[offset] = 1;
            offset += 1;

            let index_i128 = tick_index as i128;
            let index_u128 = tick_index as u128;

            let liquidity_net = 100i128 + index_i128;
            let liquidity_gross = 200u128 + index_u128;
            let fee_growth_a = 300u128 + index_u128;
            let fee_growth_b = 400u128 + index_u128;
            let reward_0 = 500u128 + index_u128;
            let reward_1 = 600u128 + index_u128;
            let reward_2 = 700u128 + index_u128;

            data[offset..offset + 16].copy_from_slice(&liquidity_net.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&liquidity_gross.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&fee_growth_a.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&fee_growth_b.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&reward_0.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&reward_1.to_le_bytes());
            offset += 16;
            data[offset..offset + 16].copy_from_slice(&reward_2.to_le_bytes());
            offset += 16;
        }

        if offset != data.len() {
            return Err(format!(
                "test dynamic tick-array encoder consumed {offset}, len={}",
                data.len()
            ));
        }

        Ok(data)
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
    fn official_oracle_pda_vector_matches() -> Result<(), String> {
        let whirlpool = "2kJmUjxWBwL2NGPBV2PiA5hWtmLCqcKY6reQgkrPtaeS";
        let expected = "821SHenpVGYY7BCXUzNhs8Xi4grG557fqRw4wzgaPQcS";

        let actual = oracle_pda(whirlpool)?;

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
    fn whirlpool_facade_decoder_preserves_authoritative_quote_state() -> Result<(), String> {
        let pool = sample_pool();
        let data = sample_whirlpool_account(&pool);

        let facade = decode_whirlpool_facade(&data)?;

        assert_eq!(facade.tick_spacing, pool.tick_spacing);
        assert_eq!(
            u16::from_le_bytes(facade.fee_tier_index_seed),
            pool.fee_tier_index_seed
        );
        assert_eq!(facade.fee_rate, pool.fee_rate);
        assert_eq!(facade.protocol_fee_rate, pool.protocol_fee_rate);
        assert_eq!(facade.liquidity, pool.liquidity);
        assert_eq!(facade.sqrt_price, pool.sqrt_price);
        assert_eq!(facade.tick_current_index, pool.tick_current_index);
        assert_eq!(facade.fee_growth_global_a, 111);
        assert_eq!(facade.fee_growth_global_b, 222);
        assert_eq!(facade.reward_last_updated_timestamp, 333);
        assert_eq!(facade.reward_infos[0].emissions_per_second_x64, 1_000);
        assert_eq!(facade.reward_infos[0].growth_global_x64, 2_000);
        assert_eq!(facade.reward_infos[1].emissions_per_second_x64, 1_001);
        assert_eq!(facade.reward_infos[1].growth_global_x64, 2_001);
        assert_eq!(facade.reward_infos[2].emissions_per_second_x64, 1_002);
        assert_eq!(facade.reward_infos[2].growth_global_x64, 2_002);

        verify_whirlpool_facade_matches_pool(&pool, &facade)?;

        Ok(())
    }

    #[test]
    fn oracle_facade_decoder_preserves_authoritative_adaptive_fee_state() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let data = sample_oracle_account(&whirlpool, 900)?;

        let oracle = decode_oracle_facade(&data, orca::ORCA_WHIRLPOOL_PROGRAM_ID, &whirlpool)?;

        assert_eq!(oracle.trade_enable_timestamp, 900);
        assert_eq!(oracle.adaptive_fee_constants.filter_period, 30);
        assert_eq!(oracle.adaptive_fee_constants.decay_period, 120);
        assert_eq!(oracle.adaptive_fee_constants.reduction_factor, 5_000);
        assert_eq!(
            oracle.adaptive_fee_constants.adaptive_fee_control_factor,
            9_000
        );
        assert_eq!(
            oracle.adaptive_fee_constants.max_volatility_accumulator,
            88_000
        );
        assert_eq!(oracle.adaptive_fee_constants.tick_group_size, 64);
        assert_eq!(oracle.adaptive_fee_constants.major_swap_threshold_ticks, 32);
        assert_eq!(
            oracle
                .adaptive_fee_variables
                .last_reference_update_timestamp,
            777
        );
        assert_eq!(oracle.adaptive_fee_variables.last_major_swap_timestamp, 778);
        assert_eq!(oracle.adaptive_fee_variables.volatility_reference, 123);
        assert_eq!(oracle.adaptive_fee_variables.tick_group_index_reference, -7);
        assert_eq!(oracle.adaptive_fee_variables.volatility_accumulator, 456);

        Ok(())
    }

    #[test]
    fn oracle_facade_decoder_fails_closed_on_owner_and_identity_mismatch() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let data = sample_oracle_account(&whirlpool, 900)?;

        match decode_oracle_facade(&data, &Pubkey::new_unique().to_string(), &whirlpool) {
            Ok(_) => {
                return Err("Oracle with wrong owner was accepted".to_owned());
            }
            Err(error) => {
                assert!(error.contains("owner mismatch"));
            }
        }

        match decode_oracle_facade(
            &data,
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            &Pubkey::new_unique().to_string(),
        ) {
            Ok(_) => Err("Oracle with wrong Whirlpool identity was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("Whirlpool mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn whirlpool_facade_decoder_fails_closed_on_wrong_layout() -> Result<(), String> {
        let data = vec![0u8; 64];

        match decode_whirlpool_facade(&data) {
            Ok(_) => Err("invalid Whirlpool layout was accepted".to_owned()),
            Err(_) => Ok(()),
        }
    }

    #[test]
    fn fixed_tick_array_decoder_enforces_identity() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique();
        let mut data = vec![0u8; FIXED_TICK_ARRAY_LEN];

        data[0..8].copy_from_slice(&FIXED_TICK_ARRAY_DISCRIMINATOR);
        data[8..12].copy_from_slice(&0i32.to_le_bytes());

        let whirlpool_offset = 12 + (TICK_ARRAY_SIZE * TICK_SERIALIZED_LEN);

        data[whirlpool_offset..whirlpool_offset + 32].copy_from_slice(whirlpool.as_ref());

        let decoded = decode_fixed_tick_array(&data, &whirlpool.to_string(), 0)?;

        assert_eq!(decoded.start_tick_index, 0);
        assert!(decoded.ticks.iter().all(|tick| !tick.initialized));

        match decode_fixed_tick_array(&data, &Pubkey::new_unique().to_string(), 0) {
            Ok(_) => Err("wrong Whirlpool identity was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("Whirlpool mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn dynamic_tick_array_decoder_preserves_sparse_initialized_ticks() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let data = sample_dynamic_tick_array(&whirlpool, 0, &[1, 86])?;

        assert_eq!(
            data.len(),
            DYNAMIC_TICK_ARRAY_MIN_LEN + (2 * DYNAMIC_TICK_DATA_LEN)
        );

        let decoded =
            decode_tick_array_account(&data, orca::ORCA_WHIRLPOOL_PROGRAM_ID, &whirlpool, 0)?;

        assert_eq!(decoded.start_tick_index, 0);
        assert!(!decoded.ticks[0].initialized);
        assert!(decoded.ticks[1].initialized);
        assert_eq!(decoded.ticks[1].liquidity_net, 101);
        assert_eq!(decoded.ticks[1].liquidity_gross, 201);
        assert_eq!(decoded.ticks[1].fee_growth_outside_a, 301);
        assert_eq!(decoded.ticks[1].fee_growth_outside_b, 401);
        assert_eq!(decoded.ticks[1].reward_growths_outside, [501, 601, 701]);
        assert!(!decoded.ticks[85].initialized);
        assert!(decoded.ticks[86].initialized);
        assert_eq!(decoded.ticks[86].liquidity_net, 186);
        assert_eq!(decoded.ticks[86].liquidity_gross, 286);
        assert_eq!(decoded.ticks[86].reward_growths_outside, [586, 686, 786]);
        assert!(!decoded.ticks[87].initialized);

        Ok(())
    }

    #[test]
    fn dynamic_tick_array_decoder_rejects_bitmap_tag_mismatch() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let mut data = sample_dynamic_tick_array(&whirlpool, 0, &[1])?;

        let tick_one_tag_offset = DYNAMIC_TICK_ARRAY_TICK_DATA_OFFSET + 1;
        data[tick_one_tag_offset] = 0;

        match decode_dynamic_tick_array(&data, &whirlpool, 0) {
            Ok(_) => Err("dynamic tick bitmap/tag mismatch was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("bitmap/tag mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn dynamic_tick_array_decoder_rejects_bitmap_length_mismatch() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let mut data = sample_dynamic_tick_array(&whirlpool, 0, &[2])?;

        let _ = data.pop();

        match decode_dynamic_tick_array(&data, &whirlpool, 0) {
            Ok(_) => Err("dynamic tick bitmap/length mismatch was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("bitmap/length mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn dynamic_tick_array_decoder_rejects_out_of_range_bitmap_bits() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let mut data = sample_dynamic_tick_array(&whirlpool, 0, &[])?;

        let invalid_bitmap = 1u128 << 100;
        data[DYNAMIC_TICK_ARRAY_BITMAP_OFFSET..DYNAMIC_TICK_ARRAY_BITMAP_OFFSET + 16]
            .copy_from_slice(&invalid_bitmap.to_le_bytes());

        match decode_dynamic_tick_array(&data, &whirlpool, 0) {
            Ok(_) => Err("dynamic tick bitmap with out-of-range bit was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("outside the 88-tick range"));
                Ok(())
            }
        }
    }

    #[test]
    fn tick_array_account_decoder_rejects_wrong_owner() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let data = sample_dynamic_tick_array(&whirlpool, 0, &[])?;
        let wrong_owner = Pubkey::new_unique().to_string();

        match decode_tick_array_account(&data, &wrong_owner, &whirlpool, 0) {
            Ok(_) => Err("tick array with wrong owner was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("owner mismatch"));
                Ok(())
            }
        }
    }

    #[test]
    fn tick_array_account_decoder_rejects_unknown_discriminator() -> Result<(), String> {
        let whirlpool = Pubkey::new_unique().to_string();
        let mut data = sample_dynamic_tick_array(&whirlpool, 0, &[])?;

        data[0..8].copy_from_slice(&[9u8; 8]);

        match decode_tick_array_account(&data, orca::ORCA_WHIRLPOOL_PROGRAM_ID, &whirlpool, 0) {
            Ok(_) => Err("unknown tick-array discriminator was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("unsupported Orca tick-array discriminator"));
                Ok(())
            }
        }
    }

    #[test]
    fn adaptive_pool_requires_oracle_before_quote() -> Result<(), String> {
        let mut pool = sample_pool();
        pool.fee_tier_index_seed = 32;

        let whirlpool_data = sample_whirlpool_account(&pool);
        let whirlpool = decode_whirlpool_facade(&whirlpool_data)?;

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
            whirlpool,
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

    #[test]
    fn adaptive_pool_rejects_quote_before_trade_enable_timestamp() -> Result<(), String> {
        let mut pool = sample_pool();
        pool.fee_tier_index_seed = 32;

        let whirlpool_data = sample_whirlpool_account(&pool);
        let whirlpool = decode_whirlpool_facade(&whirlpool_data)?;

        let oracle_whirlpool = Pubkey::new_unique().to_string();
        let oracle_data = sample_oracle_account(&oracle_whirlpool, 2_000)?;
        let oracle = decode_oracle_facade(
            &oracle_data,
            orca::ORCA_WHIRLPOOL_PROGRAM_ID,
            &oracle_whirlpool,
        )?;

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
            whirlpool,
            &input_mint,
            1_000,
            arrays,
            1_000,
            Some(oracle),
            None,
            None,
        ) {
            Ok(_) => Err("adaptive pool quoted before trade enable timestamp".to_owned()),
            Err(error) => {
                assert!(error.contains("trading is not enabled yet"));
                Ok(())
            }
        }
    }
}
