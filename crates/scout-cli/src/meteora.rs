pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

pub const LB_PAIR_ACCOUNT_LEN: usize = 904;
pub const LB_PAIR_DISCRIMINATOR: [u8; 8] = [33, 11, 49, 98, 181, 101, 177, 13];

pub const LB_PAIR_BASE_FACTOR_OFFSET: usize = 8;
pub const LB_PAIR_FILTER_PERIOD_OFFSET: usize = 10;
pub const LB_PAIR_DECAY_PERIOD_OFFSET: usize = 12;
pub const LB_PAIR_REDUCTION_FACTOR_OFFSET: usize = 14;
pub const LB_PAIR_VARIABLE_FEE_CONTROL_OFFSET: usize = 16;
pub const LB_PAIR_MAX_VOLATILITY_ACCUMULATOR_OFFSET: usize = 20;
pub const LB_PAIR_PROTOCOL_SHARE_OFFSET: usize = 32;
pub const LB_PAIR_BASE_FEE_POWER_FACTOR_OFFSET: usize = 34;
pub const LB_PAIR_COLLECT_FEE_MODE_OFFSET: usize = 36;
pub const LB_PAIR_VOLATILITY_ACCUMULATOR_OFFSET: usize = 40;
pub const LB_PAIR_VOLATILITY_REFERENCE_OFFSET: usize = 44;
pub const LB_PAIR_INDEX_REFERENCE_OFFSET: usize = 48;
pub const LB_PAIR_LAST_UPDATE_TIMESTAMP_OFFSET: usize = 56;
pub const LB_PAIR_ACTIVE_ID_OFFSET: usize = 76;
pub const LB_PAIR_BIN_STEP_OFFSET: usize = 80;
pub const LB_PAIR_MINT_X_OFFSET: usize = 88;
pub const LB_PAIR_MINT_Y_OFFSET: usize = 120;

pub const BIN_ARRAY_ACCOUNT_LEN: usize = 10_136;
pub const BIN_ARRAY_DISCRIMINATOR: [u8; 8] = [92, 142, 92, 220, 5, 148, 70, 181];
pub const BIN_ARRAY_INDEX_OFFSET: usize = 8;
pub const BIN_ARRAY_VERSION_OFFSET: usize = 16;
pub const BIN_ARRAY_LB_PAIR_OFFSET: usize = 24;
pub const BIN_ARRAY_FIRST_BIN_OFFSET: usize = 56;
pub const BIN_ARRAY_VERSION_V3: u8 = 3;
pub const MAX_BIN_PER_ARRAY: i32 = 70;
pub const BIN_STRIDE: usize = 144;

pub const BITMAP_EXTENSION_ACCOUNT_LEN: usize = 1_576;
pub const BITMAP_EXTENSION_DISCRIMINATOR: [u8; 8] = [80, 111, 124, 113, 55, 237, 18, 5];

pub const BIN_ARRAY_MIN_INDEX: i64 = -6_656;
pub const BIN_ARRAY_MAX_INDEX: i64 = 6_655;
pub const INTERNAL_BITMAP_MIN_INDEX: i64 = -512;
pub const INTERNAL_BITMAP_MAX_INDEX: i64 = 511;
pub const NEGATIVE_BITMAP_EXTENSION_MAX_INDEX: i64 = -513;
pub const POSITIVE_BITMAP_EXTENSION_MIN_INDEX: i64 = 512;

pub const FEE_PRECISION: u64 = 1_000_000_000;
pub const MAX_FEE_RATE: u64 = 100_000_000;

const BIN_AMOUNT_X_OFFSET: usize = 0;
const BIN_AMOUNT_Y_OFFSET: usize = 8;
const BIN_PRICE_OFFSET: usize = 16;
const BIN_LIQUIDITY_SUPPLY_OFFSET: usize = 32;
const BIN_FULFILLED_ORDER_AMOUNT_X_OFFSET: usize = 48;
const BIN_FULFILLED_ORDER_AMOUNT_Y_OFFSET: usize = 56;
const BIN_LIMIT_ORDER_FEE_ASK_SIDE_OFFSET: usize = 64;
const BIN_LIMIT_ORDER_FEE_BID_SIDE_OFFSET: usize = 72;
const BIN_FEE_AMOUNT_X_PER_TOKEN_STORED_OFFSET: usize = 80;
const BIN_FEE_AMOUNT_Y_PER_TOKEN_STORED_OFFSET: usize = 96;
const BIN_OPEN_ORDER_AMOUNT_OFFSET: usize = 112;
const BIN_TOTAL_PROCESSING_ORDER_AMOUNT_OFFSET: usize = 120;
const BIN_PROCESSED_ORDER_REMAINING_AMOUNT_OFFSET: usize = 128;
const BIN_ORDER_AGE_OFFSET: usize = 136;
const BIN_LIMIT_ORDER_ASK_SIDE_OFFSET: usize = 140;

const BITMAP_EXTENSION_LB_PAIR_OFFSET: usize = 8;
const BITMAP_EXTENSION_POSITIVE_OFFSET: usize = 40;
const BITMAP_EXTENSION_NEGATIVE_OFFSET: usize = 808;
const BITMAP_EXTENSION_CHUNKS: usize = 12;
const BITMAP_EXTENSION_WORDS: usize = 8;

pub type MeteoraBitmapRegion = [[u64; 8]; 12];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DlmmProtocolProfile {
    V0_12,
}

impl DlmmProtocolProfile {
    pub const fn program_id(self) -> &'static str {
        match self {
            Self::V0_12 => METEORA_DLMM_PROGRAM_ID,
        }
    }

    pub const fn accepts_bin_array_version(self, version: u8) -> bool {
        match self {
            Self::V0_12 => version == BIN_ARRAY_VERSION_V3,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraDlmmFailure {
    WrongProgramOwner,
    InvalidAccountDiscriminator,
    InvalidAccountLength,
    InvalidLbPair,
    UnsupportedPool,
    UnsupportedToken,
    UnsupportedTokenExtension,
    InactivePool,
    SwapDisabled,
    StaleState,
    MissingBinArray,
    MissingHydratedBinArray,
    InvalidBinArray,
    UnsupportedBinArrayVersion,
    LegacyOrNonCanonicalBinArray,
    InvalidBitmapExtension,
    BitmapExtensionRequired,
    InvalidLayout,
    UnsupportedCollectFeeMode,
    InsufficientHydration,
    InsufficientLiquidity,
    ArithmeticOverflow,
    PartialFillRejected,
    AccountFootprintExceeded,
    UnsupportedProtocolVersion,
    ProtocolSearchRangeExceeded,
    SimulationFailed,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraLbPairState {
    pub base_factor: u16,
    pub filter_period: u16,
    pub decay_period: u16,
    pub reduction_factor: u16,
    pub variable_fee_control: u32,
    pub max_volatility_accumulator: u32,
    pub protocol_share: u16,
    pub base_fee_power_factor: u8,
    pub collect_fee_mode: u8,
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
    pub last_update_timestamp: i64,
    pub active_id: i32,
    pub bin_step: u16,
    pub mint_x: [u8; 32],
    pub mint_y: [u8; 32],
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraBin {
    pub amount_x: u64,
    pub amount_y: u64,
    pub price: u128,
    pub liquidity_supply: u128,
    pub fulfilled_order_amount_x: u64,
    pub fulfilled_order_amount_y: u64,
    pub limit_order_fee_ask_side: u64,
    pub limit_order_fee_bid_side: u64,
    pub fee_amount_x_per_token_stored: u128,
    pub fee_amount_y_per_token_stored: u128,
    pub open_order_amount: u64,
    pub total_processing_order_amount: u64,
    pub processed_order_remaining_amount: u64,
    pub order_age: u32,
    pub limit_order_ask_side: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraBinArrayState {
    pub index: i64,
    pub version: u8,
    pub lb_pair: [u8; 32],
    pub bins: Vec<MeteoraBin>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraBitmapExtensionState {
    pub lb_pair: [u8; 32],
    pub positive_bin_array_bitmap: MeteoraBitmapRegion,
    pub negative_bin_array_bitmap: MeteoraBitmapRegion,
}

pub fn decode_lb_pair(owner: &str, data: &[u8]) -> Result<MeteoraLbPairState, MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidLbPair;

    validate_account_header(owner, data, LB_PAIR_ACCOUNT_LEN, LB_PAIR_DISCRIMINATOR)?;

    Ok(MeteoraLbPairState {
        base_factor: read_u16(data, LB_PAIR_BASE_FACTOR_OFFSET, invalid)?,
        filter_period: read_u16(data, LB_PAIR_FILTER_PERIOD_OFFSET, invalid)?,
        decay_period: read_u16(data, LB_PAIR_DECAY_PERIOD_OFFSET, invalid)?,
        reduction_factor: read_u16(data, LB_PAIR_REDUCTION_FACTOR_OFFSET, invalid)?,
        variable_fee_control: read_u32(data, LB_PAIR_VARIABLE_FEE_CONTROL_OFFSET, invalid)?,
        max_volatility_accumulator: read_u32(
            data,
            LB_PAIR_MAX_VOLATILITY_ACCUMULATOR_OFFSET,
            invalid,
        )?,
        protocol_share: read_u16(data, LB_PAIR_PROTOCOL_SHARE_OFFSET, invalid)?,
        base_fee_power_factor: read_u8(data, LB_PAIR_BASE_FEE_POWER_FACTOR_OFFSET, invalid)?,
        collect_fee_mode: read_u8(data, LB_PAIR_COLLECT_FEE_MODE_OFFSET, invalid)?,
        volatility_accumulator: read_u32(data, LB_PAIR_VOLATILITY_ACCUMULATOR_OFFSET, invalid)?,
        volatility_reference: read_u32(data, LB_PAIR_VOLATILITY_REFERENCE_OFFSET, invalid)?,
        index_reference: read_i32(data, LB_PAIR_INDEX_REFERENCE_OFFSET, invalid)?,
        last_update_timestamp: read_i64(data, LB_PAIR_LAST_UPDATE_TIMESTAMP_OFFSET, invalid)?,
        active_id: read_i32(data, LB_PAIR_ACTIVE_ID_OFFSET, invalid)?,
        bin_step: read_u16(data, LB_PAIR_BIN_STEP_OFFSET, invalid)?,
        mint_x: read_array::<32>(data, LB_PAIR_MINT_X_OFFSET, invalid)?,
        mint_y: read_array::<32>(data, LB_PAIR_MINT_Y_OFFSET, invalid)?,
    })
}

pub fn decode_bin_array(
    owner: &str,
    data: &[u8],
    profile: DlmmProtocolProfile,
) -> Result<MeteoraBinArrayState, MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidBinArray;

    validate_account_header(owner, data, BIN_ARRAY_ACCOUNT_LEN, BIN_ARRAY_DISCRIMINATOR)?;

    let version = read_u8(data, BIN_ARRAY_VERSION_OFFSET, invalid)?;
    if !profile.accepts_bin_array_version(version) {
        return Err(MeteoraDlmmFailure::UnsupportedBinArrayVersion);
    }

    let mut bins = Vec::with_capacity(MAX_BIN_PER_ARRAY as usize);
    for bin_index in 0..MAX_BIN_PER_ARRAY as usize {
        let relative_offset = bin_index.checked_mul(BIN_STRIDE).ok_or(invalid)?;
        let start = BIN_ARRAY_FIRST_BIN_OFFSET
            .checked_add(relative_offset)
            .ok_or(invalid)?;
        let end = start.checked_add(BIN_STRIDE).ok_or(invalid)?;
        let bin_data = data.get(start..end).ok_or(invalid)?;
        bins.push(decode_bin(bin_data)?);
    }

    Ok(MeteoraBinArrayState {
        index: read_i64(data, BIN_ARRAY_INDEX_OFFSET, invalid)?,
        version,
        lb_pair: read_array::<32>(data, BIN_ARRAY_LB_PAIR_OFFSET, invalid)?,
        bins,
    })
}

pub fn decode_bitmap_extension(
    owner: &str,
    data: &[u8],
) -> Result<MeteoraBitmapExtensionState, MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidBitmapExtension;

    validate_account_header(
        owner,
        data,
        BITMAP_EXTENSION_ACCOUNT_LEN,
        BITMAP_EXTENSION_DISCRIMINATOR,
    )?;

    let mut positive = [[0_u64; BITMAP_EXTENSION_WORDS]; BITMAP_EXTENSION_CHUNKS];
    let mut negative = [[0_u64; BITMAP_EXTENSION_WORDS]; BITMAP_EXTENSION_CHUNKS];

    decode_bitmap_words(data, BITMAP_EXTENSION_POSITIVE_OFFSET, &mut positive)?;
    decode_bitmap_words(data, BITMAP_EXTENSION_NEGATIVE_OFFSET, &mut negative)?;

    Ok(MeteoraBitmapExtensionState {
        lb_pair: read_array::<32>(data, BITMAP_EXTENSION_LB_PAIR_OFFSET, invalid)?,
        positive_bin_array_bitmap: positive,
        negative_bin_array_bitmap: negative,
    })
}

fn decode_bin(data: &[u8]) -> Result<MeteoraBin, MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidBinArray;

    if data.len() != BIN_STRIDE {
        return Err(invalid);
    }

    Ok(MeteoraBin {
        amount_x: read_u64(data, BIN_AMOUNT_X_OFFSET, invalid)?,
        amount_y: read_u64(data, BIN_AMOUNT_Y_OFFSET, invalid)?,
        price: read_u128(data, BIN_PRICE_OFFSET, invalid)?,
        liquidity_supply: read_u128(data, BIN_LIQUIDITY_SUPPLY_OFFSET, invalid)?,
        fulfilled_order_amount_x: read_u64(data, BIN_FULFILLED_ORDER_AMOUNT_X_OFFSET, invalid)?,
        fulfilled_order_amount_y: read_u64(data, BIN_FULFILLED_ORDER_AMOUNT_Y_OFFSET, invalid)?,
        limit_order_fee_ask_side: read_u64(data, BIN_LIMIT_ORDER_FEE_ASK_SIDE_OFFSET, invalid)?,
        limit_order_fee_bid_side: read_u64(data, BIN_LIMIT_ORDER_FEE_BID_SIDE_OFFSET, invalid)?,
        fee_amount_x_per_token_stored: read_u128(
            data,
            BIN_FEE_AMOUNT_X_PER_TOKEN_STORED_OFFSET,
            invalid,
        )?,
        fee_amount_y_per_token_stored: read_u128(
            data,
            BIN_FEE_AMOUNT_Y_PER_TOKEN_STORED_OFFSET,
            invalid,
        )?,
        open_order_amount: read_u64(data, BIN_OPEN_ORDER_AMOUNT_OFFSET, invalid)?,
        total_processing_order_amount: read_u64(
            data,
            BIN_TOTAL_PROCESSING_ORDER_AMOUNT_OFFSET,
            invalid,
        )?,
        processed_order_remaining_amount: read_u64(
            data,
            BIN_PROCESSED_ORDER_REMAINING_AMOUNT_OFFSET,
            invalid,
        )?,
        order_age: read_u32(data, BIN_ORDER_AGE_OFFSET, invalid)?,
        limit_order_ask_side: read_u8(data, BIN_LIMIT_ORDER_ASK_SIDE_OFFSET, invalid)?,
    })
}

fn decode_bitmap_words(
    data: &[u8],
    start_offset: usize,
    output: &mut MeteoraBitmapRegion,
) -> Result<(), MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidBitmapExtension;

    for (chunk_index, chunk) in output.iter_mut().enumerate() {
        for (word_index, word) in chunk.iter_mut().enumerate() {
            let flat_index = chunk_index
                .checked_mul(BITMAP_EXTENSION_WORDS)
                .and_then(|value| value.checked_add(word_index))
                .ok_or(invalid)?;
            let byte_offset = flat_index
                .checked_mul(8)
                .and_then(|value| start_offset.checked_add(value))
                .ok_or(invalid)?;

            *word = read_u64(data, byte_offset, invalid)?;
        }
    }

    Ok(())
}

fn validate_account_header(
    owner: &str,
    data: &[u8],
    expected_len: usize,
    expected_discriminator: [u8; 8],
) -> Result<(), MeteoraDlmmFailure> {
    if owner != METEORA_DLMM_PROGRAM_ID {
        return Err(MeteoraDlmmFailure::WrongProgramOwner);
    }
    if data.len() != expected_len {
        return Err(MeteoraDlmmFailure::InvalidAccountLength);
    }

    let discriminator = read_array::<8>(data, 0, MeteoraDlmmFailure::InvalidAccountDiscriminator)?;
    if discriminator != expected_discriminator {
        return Err(MeteoraDlmmFailure::InvalidAccountDiscriminator);
    }

    Ok(())
}

fn read_u8(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<u8, MeteoraDlmmFailure> {
    data.get(offset).copied().ok_or(failure)
}

fn read_u16(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<u16, MeteoraDlmmFailure> {
    Ok(u16::from_le_bytes(read_array::<2>(data, offset, failure)?))
}

fn read_u32(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<u32, MeteoraDlmmFailure> {
    Ok(u32::from_le_bytes(read_array::<4>(data, offset, failure)?))
}

fn read_i32(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<i32, MeteoraDlmmFailure> {
    Ok(i32::from_le_bytes(read_array::<4>(data, offset, failure)?))
}

fn read_u64(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<u64, MeteoraDlmmFailure> {
    Ok(u64::from_le_bytes(read_array::<8>(data, offset, failure)?))
}

fn read_i64(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<i64, MeteoraDlmmFailure> {
    Ok(i64::from_le_bytes(read_array::<8>(data, offset, failure)?))
}

fn read_u128(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<u128, MeteoraDlmmFailure> {
    Ok(u128::from_le_bytes(read_array::<16>(
        data, offset, failure,
    )?))
}

fn read_array<const N: usize>(
    data: &[u8],
    offset: usize,
    failure: MeteoraDlmmFailure,
) -> Result<[u8; N], MeteoraDlmmFailure> {
    let end = offset.checked_add(N).ok_or(failure)?;
    let bytes = data.get(offset..end).ok_or(failure)?;
    bytes.try_into().map_err(|_| failure)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn v0_12_profile_is_bound_to_public_dlmm_program() {
        assert_eq!(
            DlmmProtocolProfile::V0_12.program_id(),
            "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
        );
    }

    #[test]
    fn v0_12_profile_accepts_only_bin_array_version_three() {
        let profile = DlmmProtocolProfile::V0_12;

        assert!(profile.accepts_bin_array_version(BIN_ARRAY_VERSION_V3));
        assert!(!profile.accepts_bin_array_version(0));
        assert!(!profile.accepts_bin_array_version(2));
        assert!(!profile.accepts_bin_array_version(4));
        assert!(!profile.accepts_bin_array_version(u8::MAX));
    }

    #[test]
    fn account_contract_constants_match_m1_profile() {
        assert_eq!(LB_PAIR_ACCOUNT_LEN, 904);
        assert_eq!(LB_PAIR_DISCRIMINATOR, [33, 11, 49, 98, 181, 101, 177, 13]);

        assert_eq!(BIN_ARRAY_ACCOUNT_LEN, 10_136);
        assert_eq!(BIN_ARRAY_DISCRIMINATOR, [92, 142, 92, 220, 5, 148, 70, 181]);

        assert_eq!(BITMAP_EXTENSION_ACCOUNT_LEN, 1_576);
        assert_eq!(
            BITMAP_EXTENSION_DISCRIMINATOR,
            [80, 111, 124, 113, 55, 237, 18, 5]
        );
    }

    #[test]
    fn protocol_domains_are_explicit_and_bounded() {
        assert_eq!(BIN_ARRAY_VERSION_V3, 3);
        assert_eq!(MAX_BIN_PER_ARRAY, 70);
        assert_eq!(BIN_ARRAY_MIN_INDEX, -6_656);
        assert_eq!(BIN_ARRAY_MAX_INDEX, 6_655);
        assert_eq!(INTERNAL_BITMAP_MIN_INDEX, -512);
        assert_eq!(INTERNAL_BITMAP_MAX_INDEX, 511);
        assert_eq!(NEGATIVE_BITMAP_EXTENSION_MAX_INDEX, -513);
        assert_eq!(POSITIVE_BITMAP_EXTENSION_MIN_INDEX, 512);
    }

    #[test]
    fn fee_constants_match_v0_12_contract() {
        assert_eq!(FEE_PRECISION, 1_000_000_000);
        assert_eq!(MAX_FEE_RATE, 100_000_000);
    }

    #[test]
    fn lb_pair_decoder_reads_locked_m1_fields() {
        let data = valid_lb_pair_bytes();

        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.base_factor),
            Ok(25)
        );
        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.active_id),
            Ok(-321)
        );
        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.mint_x),
            Ok([7_u8; 32])
        );
        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.mint_y),
            Ok([9_u8; 32])
        );
    }

    #[test]
    fn strict_header_validation_rejects_wrong_owner_length_and_discriminator() {
        let data = valid_lb_pair_bytes();

        assert_eq!(
            decode_lb_pair("wrong-owner", &data),
            Err(MeteoraDlmmFailure::WrongProgramOwner)
        );
        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data[..LB_PAIR_ACCOUNT_LEN - 1]),
            Err(MeteoraDlmmFailure::InvalidAccountLength)
        );

        let mut wrong_discriminator = data;
        wrong_discriminator[0] ^= 0xff;

        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &wrong_discriminator),
            Err(MeteoraDlmmFailure::InvalidAccountDiscriminator)
        );
    }

    #[test]
    fn bin_array_decoder_enforces_v3_and_decodes_full_bin_layout() {
        let data = valid_bin_array_bytes();
        let profile = DlmmProtocolProfile::V0_12;

        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, profile).map(|state| state.index),
            Ok(-1)
        );
        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, profile).map(|state| state.bins.len()),
            Ok(70)
        );
        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, profile)
                .map(|state| state.bins.first().map(|bin| bin.amount_x)),
            Ok(Some(11))
        );
        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, profile)
                .map(|state| state.bins.first().map(|bin| bin.price)),
            Ok(Some(33))
        );
        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, profile)
                .map(|state| state.bins.first().map(|bin| bin.limit_order_ask_side)),
            Ok(Some(1))
        );
    }

    #[test]
    fn bin_array_decoder_fails_closed_on_unsupported_version() {
        let mut data = valid_bin_array_bytes();
        data[BIN_ARRAY_VERSION_OFFSET] = 2;

        assert_eq!(
            decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, DlmmProtocolProfile::V0_12),
            Err(MeteoraDlmmFailure::UnsupportedBinArrayVersion)
        );
    }

    #[test]
    fn bitmap_extension_decoder_reads_both_bitmap_regions() {
        let data = valid_bitmap_extension_bytes();

        assert_eq!(
            decode_bitmap_extension(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.lb_pair),
            Ok([5_u8; 32])
        );
        assert_eq!(
            decode_bitmap_extension(METEORA_DLMM_PROGRAM_ID, &data)
                .map(|state| state.positive_bin_array_bitmap[0][0]),
            Ok(0x1122_3344_5566_7788)
        );
        assert_eq!(
            decode_bitmap_extension(METEORA_DLMM_PROGRAM_ID, &data)
                .map(|state| state.negative_bin_array_bitmap[11][7]),
            Ok(0x8877_6655_4433_2211)
        );
    }

    #[test]
    fn bitmap_extension_decoder_rejects_malformed_account() {
        let mut data = valid_bitmap_extension_bytes();
        data[0] ^= 0xff;

        assert_eq!(
            decode_bitmap_extension(METEORA_DLMM_PROGRAM_ID, &data),
            Err(MeteoraDlmmFailure::InvalidAccountDiscriminator)
        );
    }

    fn valid_lb_pair_bytes() -> Vec<u8> {
        let mut data = vec![0_u8; LB_PAIR_ACCOUNT_LEN];

        write_bytes(&mut data, 0, LB_PAIR_DISCRIMINATOR);
        write_bytes(&mut data, LB_PAIR_BASE_FACTOR_OFFSET, 25_u16.to_le_bytes());
        write_bytes(
            &mut data,
            LB_PAIR_FILTER_PERIOD_OFFSET,
            30_u16.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_DECAY_PERIOD_OFFSET,
            600_u16.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_REDUCTION_FACTOR_OFFSET,
            5_000_u16.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_VARIABLE_FEE_CONTROL_OFFSET,
            400_000_u32.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_MAX_VOLATILITY_ACCUMULATOR_OFFSET,
            350_000_u32.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_PROTOCOL_SHARE_OFFSET,
            2_500_u16.to_le_bytes(),
        );
        data[LB_PAIR_BASE_FEE_POWER_FACTOR_OFFSET] = 1;
        data[LB_PAIR_COLLECT_FEE_MODE_OFFSET] = 0;
        write_bytes(
            &mut data,
            LB_PAIR_VOLATILITY_ACCUMULATOR_OFFSET,
            123_u32.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_VOLATILITY_REFERENCE_OFFSET,
            456_u32.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_INDEX_REFERENCE_OFFSET,
            (-12_i32).to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_LAST_UPDATE_TIMESTAMP_OFFSET,
            1_700_000_000_i64.to_le_bytes(),
        );
        write_bytes(
            &mut data,
            LB_PAIR_ACTIVE_ID_OFFSET,
            (-321_i32).to_le_bytes(),
        );
        write_bytes(&mut data, LB_PAIR_BIN_STEP_OFFSET, 10_u16.to_le_bytes());
        write_bytes(&mut data, LB_PAIR_MINT_X_OFFSET, [7_u8; 32]);
        write_bytes(&mut data, LB_PAIR_MINT_Y_OFFSET, [9_u8; 32]);

        data
    }

    fn valid_bin_array_bytes() -> Vec<u8> {
        let mut data = vec![0_u8; BIN_ARRAY_ACCOUNT_LEN];

        write_bytes(&mut data, 0, BIN_ARRAY_DISCRIMINATOR);
        write_bytes(&mut data, BIN_ARRAY_INDEX_OFFSET, (-1_i64).to_le_bytes());
        data[BIN_ARRAY_VERSION_OFFSET] = BIN_ARRAY_VERSION_V3;
        write_bytes(&mut data, BIN_ARRAY_LB_PAIR_OFFSET, [7_u8; 32]);

        let first_bin =
            &mut data[BIN_ARRAY_FIRST_BIN_OFFSET..BIN_ARRAY_FIRST_BIN_OFFSET + BIN_STRIDE];

        write_bytes(first_bin, BIN_AMOUNT_X_OFFSET, 11_u64.to_le_bytes());
        write_bytes(first_bin, BIN_AMOUNT_Y_OFFSET, 22_u64.to_le_bytes());
        write_bytes(first_bin, BIN_PRICE_OFFSET, 33_u128.to_le_bytes());
        write_bytes(
            first_bin,
            BIN_LIQUIDITY_SUPPLY_OFFSET,
            44_u128.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_FULFILLED_ORDER_AMOUNT_X_OFFSET,
            55_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_FULFILLED_ORDER_AMOUNT_Y_OFFSET,
            66_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_LIMIT_ORDER_FEE_ASK_SIDE_OFFSET,
            77_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_LIMIT_ORDER_FEE_BID_SIDE_OFFSET,
            88_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_FEE_AMOUNT_X_PER_TOKEN_STORED_OFFSET,
            99_u128.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_FEE_AMOUNT_Y_PER_TOKEN_STORED_OFFSET,
            111_u128.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_OPEN_ORDER_AMOUNT_OFFSET,
            122_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_TOTAL_PROCESSING_ORDER_AMOUNT_OFFSET,
            133_u64.to_le_bytes(),
        );
        write_bytes(
            first_bin,
            BIN_PROCESSED_ORDER_REMAINING_AMOUNT_OFFSET,
            144_u64.to_le_bytes(),
        );
        write_bytes(first_bin, BIN_ORDER_AGE_OFFSET, 155_u32.to_le_bytes());
        first_bin[BIN_LIMIT_ORDER_ASK_SIDE_OFFSET] = 1;

        data
    }

    fn valid_bitmap_extension_bytes() -> Vec<u8> {
        let mut data = vec![0_u8; BITMAP_EXTENSION_ACCOUNT_LEN];

        write_bytes(&mut data, 0, BITMAP_EXTENSION_DISCRIMINATOR);
        write_bytes(&mut data, BITMAP_EXTENSION_LB_PAIR_OFFSET, [5_u8; 32]);
        write_bytes(
            &mut data,
            BITMAP_EXTENSION_POSITIVE_OFFSET,
            0x1122_3344_5566_7788_u64.to_le_bytes(),
        );

        let last_negative_word_offset = BITMAP_EXTENSION_NEGATIVE_OFFSET
            + (BITMAP_EXTENSION_CHUNKS * BITMAP_EXTENSION_WORDS - 1) * 8;

        write_bytes(
            &mut data,
            last_negative_word_offset,
            0x8877_6655_4433_2211_u64.to_le_bytes(),
        );

        data
    }

    fn write_bytes<const N: usize>(data: &mut [u8], offset: usize, bytes: [u8; N]) {
        data[offset..offset + N].copy_from_slice(&bytes);
    }
}
