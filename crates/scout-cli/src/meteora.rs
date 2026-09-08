use ethnum::U256;
use solana_pubkey::{pubkey, Pubkey};

pub const METEORA_DLMM_PROGRAM_ID: &str = "LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo";

const METEORA_DLMM_PROGRAM_PUBKEY: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const BIN_ARRAY_SEED: &[u8] = b"bin_array";

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
pub const LB_PAIR_BIN_ARRAY_BITMAP_OFFSET: usize = 584;

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

const BASIS_POINT_MAX: u64 = 10_000;
const VARIABLE_FEE_DENOMINATOR: u128 = 100_000_000_000;
const Q64_SCALE_OFFSET: u8 = 64;
const Q64_MAX_EXPONENTIAL: u32 = 0x80000;
pub const Q64_ONE: u128 = 1_u128 << Q64_SCALE_OFFSET;

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
const INTERNAL_BITMAP_WORDS: usize = 16;
const BITMAP_WORD_BITS: i64 = 64;
const EXTENSION_BITMAP_BITS: i64 = 512;

pub type MeteoraInternalBitmap = [u64; 16];
pub type MeteoraBitmapRegion = [[u64; 8]; 12];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraBinArrayBitmapRegion {
    Internal,
    NegativeExtension,
    PositiveExtension,
}

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
    pub bin_array_bitmap: MeteoraInternalBitmap,
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

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraClockSnapshot {
    pub slot: u64,
    pub epoch_start_timestamp: i64,
    pub epoch: u64,
    pub leader_schedule_epoch: u64,
    pub unix_timestamp: i64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraSnapshotSource {
    pub source_slot: u64,
    pub generation_id: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraBinArraySnapshotInput {
    pub pubkey: [u8; 32],
    pub state: MeteoraBinArrayState,
}

#[derive(Debug, PartialEq, Eq)]
pub struct MeteoraValidatedBinArray {
    pubkey: [u8; 32],
    state: MeteoraBinArrayState,
}

impl MeteoraValidatedBinArray {
    pub fn pubkey(&self) -> [u8; 32] {
        self.pubkey
    }

    pub fn index(&self) -> i64 {
        self.state.index
    }

    pub fn state(&self) -> &MeteoraBinArrayState {
        &self.state
    }
}

#[derive(Debug, PartialEq, Eq)]
pub struct MeteoraDlmmSnapshot {
    lb_pair_pubkey: [u8; 32],
    lb_pair: MeteoraLbPairState,
    bin_arrays: Vec<MeteoraValidatedBinArray>,
    bitmap_extension: Option<MeteoraBitmapExtensionState>,
    mint_x: [u8; 32],
    mint_y: [u8; 32],
    clock: MeteoraClockSnapshot,
    profile: DlmmProtocolProfile,
    source: MeteoraSnapshotSource,
}

impl MeteoraDlmmSnapshot {
    pub fn new(
        lb_pair_pubkey: [u8; 32],
        lb_pair: MeteoraLbPairState,
        bin_arrays: Vec<MeteoraBinArraySnapshotInput>,
        bitmap_extension: Option<MeteoraBitmapExtensionState>,
        clock: MeteoraClockSnapshot,
        profile: DlmmProtocolProfile,
        source: MeteoraSnapshotSource,
    ) -> Result<Self, MeteoraDlmmFailure> {
        if let Some(extension) = bitmap_extension.as_ref() {
            if extension.lb_pair != lb_pair_pubkey {
                return Err(MeteoraDlmmFailure::InvalidBitmapExtension);
            }
        }

        let mut validated_bin_arrays: Vec<MeteoraValidatedBinArray> =
            Vec::with_capacity(bin_arrays.len());

        for input in bin_arrays {
            validate_bin_array_index(input.state.index)?;

            if !profile.accepts_bin_array_version(input.state.version) {
                return Err(MeteoraDlmmFailure::UnsupportedBinArrayVersion);
            }
            if input.state.lb_pair != lb_pair_pubkey {
                return Err(MeteoraDlmmFailure::InvalidBinArray);
            }
            if input.state.bins.len() != MAX_BIN_PER_ARRAY as usize {
                return Err(MeteoraDlmmFailure::InvalidBinArray);
            }

            let (canonical_pubkey, _) = derive_bin_array_pda(lb_pair_pubkey, input.state.index)?;
            if input.pubkey != canonical_pubkey {
                return Err(MeteoraDlmmFailure::LegacyOrNonCanonicalBinArray);
            }
            for existing in &validated_bin_arrays {
                if existing.index() == input.state.index {
                    return Err(MeteoraDlmmFailure::InvalidLayout);
                }
            }

            validated_bin_arrays.push(MeteoraValidatedBinArray {
                pubkey: input.pubkey,
                state: input.state,
            });
        }

        Ok(Self {
            lb_pair_pubkey,
            mint_x: lb_pair.mint_x,
            mint_y: lb_pair.mint_y,
            lb_pair,
            bin_arrays: validated_bin_arrays,
            bitmap_extension,
            clock,
            profile,
            source,
        })
    }

    pub fn lb_pair_pubkey(&self) -> [u8; 32] {
        self.lb_pair_pubkey
    }

    pub fn lb_pair(&self) -> &MeteoraLbPairState {
        &self.lb_pair
    }

    pub fn bin_arrays(&self) -> &[MeteoraValidatedBinArray] {
        &self.bin_arrays
    }

    pub fn bin_array_by_index(
        &self,
        index: i64,
    ) -> Result<Option<&MeteoraValidatedBinArray>, MeteoraDlmmFailure> {
        validate_bin_array_index(index)?;
        Ok(self.bin_arrays.iter().find(|array| array.index() == index))
    }

    pub fn bitmap_extension(&self) -> Option<&MeteoraBitmapExtensionState> {
        self.bitmap_extension.as_ref()
    }

    pub fn mint_x(&self) -> [u8; 32] {
        self.mint_x
    }

    pub fn mint_y(&self) -> [u8; 32] {
        self.mint_y
    }

    pub fn clock(&self) -> MeteoraClockSnapshot {
        self.clock
    }

    pub fn profile(&self) -> DlmmProtocolProfile {
        self.profile
    }

    pub fn source(&self) -> MeteoraSnapshotSource {
        self.source
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraRounding {
    Up,
    Down,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraCollectFeeMode {
    InputOnly,
    OnlyY,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraVolatilityState {
    pub volatility_accumulator: u32,
    pub volatility_reference: u32,
    pub index_reference: i32,
}

pub fn meteora_mul_div(
    x: u128,
    y: u128,
    denominator: u128,
    rounding: MeteoraRounding,
) -> Result<u128, MeteoraDlmmFailure> {
    if denominator == 0 {
        return Err(MeteoraDlmmFailure::ArithmeticOverflow);
    }

    let denominator = U256::from(denominator);
    let product = U256::from(x)
        .checked_mul(U256::from(y))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let mut quotient = product / denominator;
    let remainder = product % denominator;

    if rounding == MeteoraRounding::Up && remainder != U256::ZERO {
        quotient = quotient
            .checked_add(U256::ONE)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    }
    u128::try_from(quotient).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_amount_out(
    amount_in: u64,
    price: u128,
    swap_for_y: bool,
    rounding: MeteoraRounding,
) -> Result<u64, MeteoraDlmmFailure> {
    let amount_out = if swap_for_y {
        meteora_mul_shr(price, u128::from(amount_in), Q64_SCALE_OFFSET, rounding)?
    } else {
        meteora_shl_div(u128::from(amount_in), price, Q64_SCALE_OFFSET, rounding)?
    };

    u64::try_from(amount_out).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_amount_in(
    amount_out: u64,
    price: u128,
    swap_for_y: bool,
    rounding: MeteoraRounding,
) -> Result<u64, MeteoraDlmmFailure> {
    let amount_in = if swap_for_y {
        meteora_shl_div(u128::from(amount_out), price, Q64_SCALE_OFFSET, rounding)?
    } else {
        meteora_mul_shr(u128::from(amount_out), price, Q64_SCALE_OFFSET, rounding)?
    };

    u64::try_from(amount_in).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_q64_price_from_bin_id(
    bin_id: i32,
    bin_step: u16,
) -> Result<u128, MeteoraDlmmFailure> {
    let bps = u128::from(bin_step)
        .checked_shl(u32::from(Q64_SCALE_OFFSET))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .checked_div(u128::from(BASIS_POINT_MAX))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let base = Q64_ONE
        .checked_add(bps)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    meteora_q64_pow(base, bin_id)
}

pub fn meteora_base_fee_rate(lb_pair: &MeteoraLbPairState) -> Result<u128, MeteoraDlmmFailure> {
    let power = 10_u128
        .checked_pow(u32::from(lb_pair.base_fee_power_factor))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    u128::from(lb_pair.base_factor)
        .checked_mul(u128::from(lb_pair.bin_step))
        .and_then(|value| value.checked_mul(10))
        .and_then(|value| value.checked_mul(power))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_variable_fee_rate(
    lb_pair: &MeteoraLbPairState,
    volatility_accumulator: u32,
) -> Result<u128, MeteoraDlmmFailure> {
    if lb_pair.variable_fee_control == 0 {
        return Ok(0);
    }

    let volatility_times_bin_step = u128::from(volatility_accumulator)
        .checked_mul(u128::from(lb_pair.bin_step))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let square = volatility_times_bin_step
        .checked_mul(volatility_times_bin_step)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let variable_fee = u128::from(lb_pair.variable_fee_control)
        .checked_mul(square)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    variable_fee
        .checked_add(VARIABLE_FEE_DENOMINATOR - 1)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .checked_div(VARIABLE_FEE_DENOMINATOR)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_total_fee_rate(
    lb_pair: &MeteoraLbPairState,
    volatility_accumulator: u32,
) -> Result<u128, MeteoraDlmmFailure> {
    let total = meteora_base_fee_rate(lb_pair)?
        .checked_add(meteora_variable_fee_rate(lb_pair, volatility_accumulator)?)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    Ok(total.min(u128::from(MAX_FEE_RATE)))
}

pub fn meteora_compute_fee(
    lb_pair: &MeteoraLbPairState,
    volatility_accumulator: u32,
    amount: u64,
) -> Result<u64, MeteoraDlmmFailure> {
    let total_fee_rate = meteora_total_fee_rate(lb_pair, volatility_accumulator)?;
    let denominator = u128::from(FEE_PRECISION)
        .checked_sub(total_fee_rate)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    if denominator == 0 {
        return Err(MeteoraDlmmFailure::ArithmeticOverflow);
    }

    let numerator = u128::from(amount)
        .checked_mul(total_fee_rate)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .checked_add(denominator - 1)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let fee = numerator
        .checked_div(denominator)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    u64::try_from(fee).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_compute_fee_from_amount(
    lb_pair: &MeteoraLbPairState,
    volatility_accumulator: u32,
    amount_with_fees: u64,
) -> Result<u64, MeteoraDlmmFailure> {
    let total_fee_rate = meteora_total_fee_rate(lb_pair, volatility_accumulator)?;
    let numerator = u128::from(amount_with_fees)
        .checked_mul(total_fee_rate)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .checked_add(u128::from(FEE_PRECISION - 1))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let fee = numerator
        .checked_div(u128::from(FEE_PRECISION))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    u64::try_from(fee).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_protocol_fee_amount(
    lb_pair: &MeteoraLbPairState,
    fee_amount: u64,
) -> Result<u64, MeteoraDlmmFailure> {
    let protocol_fee = u128::from(fee_amount)
        .checked_mul(u128::from(lb_pair.protocol_share))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .checked_div(u128::from(BASIS_POINT_MAX))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    u64::try_from(protocol_fee).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn meteora_update_volatility_reference(
    lb_pair: &MeteoraLbPairState,
    current_timestamp: i64,
) -> Result<MeteoraVolatilityState, MeteoraDlmmFailure> {
    let elapsed = current_timestamp
        .checked_sub(lb_pair.last_update_timestamp)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let mut volatility_reference = lb_pair.volatility_reference;
    let mut index_reference = lb_pair.index_reference;

    if elapsed >= i64::from(lb_pair.filter_period) {
        index_reference = lb_pair.active_id;

        if elapsed < i64::from(lb_pair.decay_period) {
            let decayed_reference = u64::from(lb_pair.volatility_accumulator)
                .checked_mul(u64::from(lb_pair.reduction_factor))
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
                .checked_div(BASIS_POINT_MAX)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            volatility_reference = u32::try_from(decayed_reference)
                .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
        } else {
            volatility_reference = 0;
        }
    }

    Ok(MeteoraVolatilityState {
        volatility_accumulator: lb_pair.volatility_accumulator,
        volatility_reference,
        index_reference,
    })
}

pub fn meteora_update_volatility_accumulator(
    state: MeteoraVolatilityState,
    max_volatility_accumulator: u32,
    active_id: i32,
) -> Result<MeteoraVolatilityState, MeteoraDlmmFailure> {
    let delta_id = i64::from(state.index_reference)
        .checked_sub(i64::from(active_id))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .unsigned_abs();
    let volatility_accumulator = u64::from(state.volatility_reference)
        .checked_add(
            delta_id
                .checked_mul(BASIS_POINT_MAX)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?,
        )
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
        .min(u64::from(max_volatility_accumulator));
    let volatility_accumulator = u32::try_from(volatility_accumulator)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;

    Ok(MeteoraVolatilityState {
        volatility_accumulator,
        ..state
    })
}

pub fn meteora_collect_fee_mode(value: u8) -> Result<MeteoraCollectFeeMode, MeteoraDlmmFailure> {
    match value {
        0 => Ok(MeteoraCollectFeeMode::InputOnly),
        1 => Ok(MeteoraCollectFeeMode::OnlyY),
        _ => Err(MeteoraDlmmFailure::UnsupportedCollectFeeMode),
    }
}

pub fn meteora_fee_on_input(
    collect_fee_mode: u8,
    swap_for_y: bool,
) -> Result<bool, MeteoraDlmmFailure> {
    match meteora_collect_fee_mode(collect_fee_mode)? {
        MeteoraCollectFeeMode::InputOnly => Ok(true),
        MeteoraCollectFeeMode::OnlyY => Ok(!swap_for_y),
    }
}

fn meteora_mul_shr(
    x: u128,
    y: u128,
    offset: u8,
    rounding: MeteoraRounding,
) -> Result<u128, MeteoraDlmmFailure> {
    let denominator = 1_u128
        .checked_shl(u32::from(offset))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    meteora_mul_div(x, y, denominator, rounding)
}

fn meteora_shl_div(
    x: u128,
    y: u128,
    offset: u8,
    rounding: MeteoraRounding,
) -> Result<u128, MeteoraDlmmFailure> {
    let scale = 1_u128
        .checked_shl(u32::from(offset))
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    meteora_mul_div(x, scale, y, rounding)
}

fn meteora_q64_pow(base: u128, exponent: i32) -> Result<u128, MeteoraDlmmFailure> {
    let mut invert = exponent.is_negative();

    if exponent == 0 {
        return Ok(Q64_ONE);
    }

    let exponent = exponent.unsigned_abs();
    if exponent >= Q64_MAX_EXPONENTIAL {
        return Err(MeteoraDlmmFailure::ArithmeticOverflow);
    }

    let mut squared_base = base;
    let mut result = Q64_ONE;

    if squared_base >= result {
        squared_base = u128::MAX
            .checked_div(squared_base)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
        invert = !invert;
    }

    let mut bit = 1_u32;
    while bit <= 0x40000 {
        if exponent & bit != 0 {
            result = result
                .checked_mul(squared_base)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
                >> Q64_SCALE_OFFSET;
        }

        if bit < 0x40000 {
            squared_base = squared_base
                .checked_mul(squared_base)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
                >> Q64_SCALE_OFFSET;
        }

        bit <<= 1;
    }

    if result == 0 {
        return Err(MeteoraDlmmFailure::ArithmeticOverflow);
    }
    if invert {
        result = u128::MAX
            .checked_div(result)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    }

    Ok(result)
}

pub fn bin_id_to_bin_array_index(bin_id: i32) -> Result<i64, MeteoraDlmmFailure> {
    let quotient = bin_id / MAX_BIN_PER_ARRAY;
    let remainder = bin_id % MAX_BIN_PER_ARRAY;
    let index = if bin_id < 0 && remainder != 0 {
        quotient
            .checked_sub(1)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
    } else {
        quotient
    };
    let index = i64::from(index);

    validate_bin_array_index(index)?;
    Ok(index)
}

pub fn bin_array_index_to_bin_range(index: i64) -> Result<(i32, i32), MeteoraDlmmFailure> {
    validate_bin_array_index(index)?;

    let bin_array_width = i64::from(MAX_BIN_PER_ARRAY);
    let lower = index
        .checked_mul(bin_array_width)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let upper = lower
        .checked_add(bin_array_width - 1)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    Ok((
        i32::try_from(lower).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?,
        i32::try_from(upper).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?,
    ))
}

pub fn bin_id_to_bin_array_offset(bin_id: i32) -> Result<usize, MeteoraDlmmFailure> {
    let index = bin_id_to_bin_array_index(bin_id)?;
    let (lower, _) = bin_array_index_to_bin_range(index)?;
    let offset = bin_id
        .checked_sub(lower)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    usize::try_from(offset).map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)
}

pub fn bin_array_bitmap_region(
    index: i64,
) -> Result<MeteoraBinArrayBitmapRegion, MeteoraDlmmFailure> {
    validate_bin_array_index(index)?;

    if index <= NEGATIVE_BITMAP_EXTENSION_MAX_INDEX {
        Ok(MeteoraBinArrayBitmapRegion::NegativeExtension)
    } else if index <= INTERNAL_BITMAP_MAX_INDEX {
        Ok(MeteoraBinArrayBitmapRegion::Internal)
    } else {
        Ok(MeteoraBinArrayBitmapRegion::PositiveExtension)
    }
}

pub fn derive_bin_array_pda(
    lb_pair: [u8; 32],
    index: i64,
) -> Result<([u8; 32], u8), MeteoraDlmmFailure> {
    validate_bin_array_index(index)?;

    let lb_pair = Pubkey::new_from_array(lb_pair);
    let index_bytes = index.to_le_bytes();

    Pubkey::try_find_program_address(
        &[BIN_ARRAY_SEED, lb_pair.as_ref(), &index_bytes],
        &METEORA_DLMM_PROGRAM_PUBKEY,
    )
    .map(|(pubkey, bump)| (pubkey.to_bytes(), bump))
    .ok_or(MeteoraDlmmFailure::InvalidLayout)
}

pub fn bin_array_bitmap_bit(
    internal_bitmap: &MeteoraInternalBitmap,
    bitmap_extension: Option<&MeteoraBitmapExtensionState>,
    index: i64,
) -> Result<bool, MeteoraDlmmFailure> {
    match bin_array_bitmap_region(index)? {
        MeteoraBinArrayBitmapRegion::Internal => internal_bitmap_bit(internal_bitmap, index),
        MeteoraBinArrayBitmapRegion::NegativeExtension => {
            let extension = bitmap_extension.ok_or(MeteoraDlmmFailure::BitmapExtensionRequired)?;
            extension_bitmap_bit(&extension.negative_bin_array_bitmap, index, false)
        }
        MeteoraBinArrayBitmapRegion::PositiveExtension => {
            let extension = bitmap_extension.ok_or(MeteoraDlmmFailure::BitmapExtensionRequired)?;
            extension_bitmap_bit(&extension.positive_bin_array_bitmap, index, true)
        }
    }
}

pub fn next_initialized_bin_array_index(
    internal_bitmap: &MeteoraInternalBitmap,
    bitmap_extension: Option<&MeteoraBitmapExtensionState>,
    start_index: i64,
    swap_for_y: bool,
) -> Result<Option<i64>, MeteoraDlmmFailure> {
    validate_bin_array_index(start_index)?;

    let step = if swap_for_y { -1_i64 } else { 1_i64 };
    let terminal = if swap_for_y {
        BIN_ARRAY_MIN_INDEX
    } else {
        BIN_ARRAY_MAX_INDEX
    };
    let mut index = start_index;

    loop {
        if bin_array_bitmap_bit(internal_bitmap, bitmap_extension, index)? {
            return Ok(Some(index));
        }
        if index == terminal {
            return Ok(None);
        }
        index = index
            .checked_add(step)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    }
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
        bin_array_bitmap: decode_internal_bitmap(data)?,
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

fn decode_internal_bitmap(data: &[u8]) -> Result<MeteoraInternalBitmap, MeteoraDlmmFailure> {
    let invalid = MeteoraDlmmFailure::InvalidLbPair;
    let mut bitmap = [0_u64; INTERNAL_BITMAP_WORDS];

    for (word_index, word) in bitmap.iter_mut().enumerate() {
        let byte_offset = word_index
            .checked_mul(8)
            .and_then(|value| LB_PAIR_BIN_ARRAY_BITMAP_OFFSET.checked_add(value))
            .ok_or(invalid)?;
        *word = read_u64(data, byte_offset, invalid)?;
    }

    Ok(bitmap)
}

fn internal_bitmap_bit(
    bitmap: &MeteoraInternalBitmap,
    index: i64,
) -> Result<bool, MeteoraDlmmFailure> {
    if !(INTERNAL_BITMAP_MIN_INDEX..=INTERNAL_BITMAP_MAX_INDEX).contains(&index) {
        return Err(MeteoraDlmmFailure::InvalidLayout);
    }

    let bit_offset = index
        .checked_sub(INTERNAL_BITMAP_MIN_INDEX)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let word_index = usize::try_from(bit_offset / BITMAP_WORD_BITS)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
    let bit_index = u32::try_from(bit_offset % BITMAP_WORD_BITS)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
    let word = bitmap
        .get(word_index)
        .copied()
        .ok_or(MeteoraDlmmFailure::InvalidLayout)?;

    Ok((word & (1_u64 << bit_index)) != 0)
}

fn extension_bitmap_bit(
    bitmap: &MeteoraBitmapRegion,
    index: i64,
    positive: bool,
) -> Result<bool, MeteoraDlmmFailure> {
    let logical_offset = if positive {
        index
            .checked_sub(POSITIVE_BITMAP_EXTENSION_MIN_INDEX)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
    } else {
        NEGATIVE_BITMAP_EXTENSION_MAX_INDEX
            .checked_sub(index)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?
    };

    if logical_offset < 0 {
        return Err(MeteoraDlmmFailure::InvalidLayout);
    }

    let chunk_index = usize::try_from(logical_offset / EXTENSION_BITMAP_BITS)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
    let chunk_bit = logical_offset % EXTENSION_BITMAP_BITS;
    let word_index = usize::try_from(chunk_bit / BITMAP_WORD_BITS)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
    let bit_index = u32::try_from(chunk_bit % BITMAP_WORD_BITS)
        .map_err(|_| MeteoraDlmmFailure::ArithmeticOverflow)?;
    let word = bitmap
        .get(chunk_index)
        .and_then(|chunk| chunk.get(word_index))
        .copied()
        .ok_or(MeteoraDlmmFailure::InvalidLayout)?;

    Ok((word & (1_u64 << bit_index)) != 0)
}

fn validate_bin_array_index(index: i64) -> Result<(), MeteoraDlmmFailure> {
    if !(BIN_ARRAY_MIN_INDEX..=BIN_ARRAY_MAX_INDEX).contains(&index) {
        return Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded);
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
    fn m3_bin_mapping_uses_floor_division_for_negative_ids() {
        assert_eq!(bin_id_to_bin_array_index(-1), Ok(-1));
        assert_eq!(bin_id_to_bin_array_index(-70), Ok(-1));
        assert_eq!(bin_id_to_bin_array_index(-71), Ok(-2));
        assert_eq!(bin_id_to_bin_array_index(0), Ok(0));
        assert_eq!(bin_id_to_bin_array_index(69), Ok(0));
        assert_eq!(bin_id_to_bin_array_index(70), Ok(1));
    }

    #[test]
    fn m3_bin_ranges_and_offsets_are_canonical() {
        assert_eq!(bin_array_index_to_bin_range(-2), Ok((-140, -71)));
        assert_eq!(bin_array_index_to_bin_range(-1), Ok((-70, -1)));
        assert_eq!(bin_array_index_to_bin_range(0), Ok((0, 69)));
        assert_eq!(bin_array_index_to_bin_range(1), Ok((70, 139)));

        assert_eq!(bin_id_to_bin_array_offset(-71), Ok(69));
        assert_eq!(bin_id_to_bin_array_offset(-70), Ok(0));
        assert_eq!(bin_id_to_bin_array_offset(-1), Ok(69));
        assert_eq!(bin_id_to_bin_array_offset(0), Ok(0));
        assert_eq!(bin_id_to_bin_array_offset(69), Ok(69));
    }

    #[test]
    fn m3_bitmap_regions_are_bounded_without_traversal() {
        assert_eq!(
            bin_array_bitmap_region(BIN_ARRAY_MIN_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::NegativeExtension)
        );
        assert_eq!(
            bin_array_bitmap_region(NEGATIVE_BITMAP_EXTENSION_MAX_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::NegativeExtension)
        );
        assert_eq!(
            bin_array_bitmap_region(INTERNAL_BITMAP_MIN_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::Internal)
        );
        assert_eq!(
            bin_array_bitmap_region(INTERNAL_BITMAP_MAX_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::Internal)
        );
        assert_eq!(
            bin_array_bitmap_region(POSITIVE_BITMAP_EXTENSION_MIN_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::PositiveExtension)
        );
        assert_eq!(
            bin_array_bitmap_region(BIN_ARRAY_MAX_INDEX),
            Ok(MeteoraBinArrayBitmapRegion::PositiveExtension)
        );
        assert_eq!(
            bin_array_bitmap_region(BIN_ARRAY_MIN_INDEX - 1),
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
        );
        assert_eq!(
            bin_array_bitmap_region(BIN_ARRAY_MAX_INDEX + 1),
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
        );
    }

    #[test]
    fn m3_bin_mapping_rejects_ids_outside_protocol_search_range() {
        assert_eq!(
            bin_id_to_bin_array_index(-465_921),
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
        );
        assert_eq!(
            bin_id_to_bin_array_index(465_920),
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
        );
    }

    #[test]
    fn m3_bin_array_pda_vectors_match_pinned_meteora_contract() {
        let lb_pair = [7_u8; 32];
        let lb_pair_pubkey = Pubkey::new_from_array(lb_pair);
        let vectors = [
            (-6_656_i64, 254_u8),
            (-1_025_i64, 254_u8),
            (-1_024_i64, 255_u8),
            (-514_i64, 255_u8),
            (-1_i64, 250_u8),
            (0_i64, 251_u8),
            (513_i64, 255_u8),
            (1_023_i64, 253_u8),
            (1_024_i64, 254_u8),
            (6_655_i64, 255_u8),
        ];

        for (index, expected_bump) in vectors {
            let index_bytes = index.to_le_bytes();
            let expected = Pubkey::try_find_program_address(
                &[b"bin_array", lb_pair_pubkey.as_ref(), &index_bytes],
                &pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"),
            )
            .map(|(pubkey, bump)| (pubkey.to_bytes(), bump));

            assert_eq!(derive_bin_array_pda(lb_pair, index).ok(), expected);
            assert_eq!(
                derive_bin_array_pda(lb_pair, index).map(|(_, bump)| bump),
                Ok(expected_bump)
            );
        }
    }

    #[test]
    fn m4_internal_bitmap_bit_mapping_matches_meteora_layout() {
        let mut bitmap = [0_u64; INTERNAL_BITMAP_WORDS];
        set_internal_bitmap_bit(&mut bitmap, -512);
        set_internal_bitmap_bit(&mut bitmap, -1);
        set_internal_bitmap_bit(&mut bitmap, 0);
        set_internal_bitmap_bit(&mut bitmap, 511);

        assert_eq!(bin_array_bitmap_bit(&bitmap, None, -512), Ok(true));
        assert_eq!(bin_array_bitmap_bit(&bitmap, None, -1), Ok(true));
        assert_eq!(bin_array_bitmap_bit(&bitmap, None, 0), Ok(true));
        assert_eq!(bin_array_bitmap_bit(&bitmap, None, 511), Ok(true));
        assert_eq!(bin_array_bitmap_bit(&bitmap, None, 1), Ok(false));
    }

    #[test]
    fn m4_extension_bitmap_bit_mapping_matches_positive_and_negative_chunks() {
        let internal = [0_u64; INTERNAL_BITMAP_WORDS];
        let mut extension = empty_bitmap_extension();
        set_extension_bitmap_bit(&mut extension.positive_bin_array_bitmap, 512, true);
        set_extension_bitmap_bit(&mut extension.positive_bin_array_bitmap, 1_023, true);
        set_extension_bitmap_bit(&mut extension.positive_bin_array_bitmap, 1_024, true);
        set_extension_bitmap_bit(&mut extension.positive_bin_array_bitmap, 6_655, true);
        set_extension_bitmap_bit(&mut extension.negative_bin_array_bitmap, -513, false);
        set_extension_bitmap_bit(&mut extension.negative_bin_array_bitmap, -1_024, false);
        set_extension_bitmap_bit(&mut extension.negative_bin_array_bitmap, -1_025, false);
        set_extension_bitmap_bit(&mut extension.negative_bin_array_bitmap, -6_656, false);

        for index in [512_i64, 1_023, 1_024, 6_655, -513, -1_024, -1_025, -6_656] {
            assert_eq!(
                bin_array_bitmap_bit(&internal, Some(&extension), index),
                Ok(true)
            );
        }
    }

    #[test]
    fn m4_extension_access_fails_closed_when_extension_is_missing() {
        let internal = [0_u64; INTERNAL_BITMAP_WORDS];

        assert_eq!(
            bin_array_bitmap_bit(&internal, None, -513),
            Err(MeteoraDlmmFailure::BitmapExtensionRequired)
        );
        assert_eq!(
            bin_array_bitmap_bit(&internal, None, 512),
            Err(MeteoraDlmmFailure::BitmapExtensionRequired)
        );
    }

    #[test]
    fn m4_search_is_inclusive_and_directional_across_internal_bitmap() {
        let mut internal = [0_u64; INTERNAL_BITMAP_WORDS];
        set_internal_bitmap_bit(&mut internal, -2);
        set_internal_bitmap_bit(&mut internal, 3);

        assert_eq!(
            next_initialized_bin_array_index(&internal, None, 0, true),
            Ok(Some(-2))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, None, 0, false),
            Ok(Some(3))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, None, -2, true),
            Ok(Some(-2))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, None, 3, false),
            Ok(Some(3))
        );
    }

    #[test]
    fn m4_search_crosses_internal_and_extension_boundaries() {
        let mut internal = [0_u64; INTERNAL_BITMAP_WORDS];
        let mut extension = empty_bitmap_extension();
        set_internal_bitmap_bit(&mut internal, -510);
        set_internal_bitmap_bit(&mut internal, 509);
        set_extension_bitmap_bit(&mut extension.negative_bin_array_bitmap, -514, false);
        set_extension_bitmap_bit(&mut extension.positive_bin_array_bitmap, 513, true);

        assert_eq!(
            next_initialized_bin_array_index(&internal, Some(&extension), -511, true),
            Ok(Some(-514))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, Some(&extension), 510, false),
            Ok(Some(513))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, Some(&extension), -510, false),
            Ok(Some(-510))
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, Some(&extension), 509, true),
            Ok(Some(509))
        );
    }

    #[test]
    fn m4_search_requires_extension_only_when_traversal_enters_extension_range() {
        let internal = [0_u64; INTERNAL_BITMAP_WORDS];

        assert_eq!(
            next_initialized_bin_array_index(&internal, None, 511, false),
            Err(MeteoraDlmmFailure::BitmapExtensionRequired)
        );
        assert_eq!(
            next_initialized_bin_array_index(&internal, None, -512, true),
            Err(MeteoraDlmmFailure::BitmapExtensionRequired)
        );
    }

    #[test]
    fn m4_search_returns_none_at_protocol_edge_when_no_initialized_array_exists() {
        let internal = [0_u64; INTERNAL_BITMAP_WORDS];
        let extension = empty_bitmap_extension();

        assert_eq!(
            next_initialized_bin_array_index(
                &internal,
                Some(&extension),
                BIN_ARRAY_MIN_INDEX,
                true,
            ),
            Ok(None)
        );
        assert_eq!(
            next_initialized_bin_array_index(
                &internal,
                Some(&extension),
                BIN_ARRAY_MAX_INDEX,
                false,
            ),
            Ok(None)
        );
    }

    #[test]
    fn m6_q64_price_matches_frozen_vectors() -> Result<(), MeteoraDlmmFailure> {
        assert_eq!(meteora_q64_price_from_bin_id(0, 25)?, Q64_ONE);
        assert_eq!(
            meteora_q64_price_from_bin_id(1, 25)?,
            18_492_860_933_893_825_495
        );
        assert_eq!(
            meteora_q64_price_from_bin_id(-1, 25)?,
            18_400_742_218_164_141_262
        );
        assert_eq!(
            meteora_q64_price_from_bin_id(10, 25)?,
            18_913_135_561_744_016_868
        );
        assert_eq!(
            meteora_q64_price_from_bin_id(-10, 25)?,
            17_991_853_641_087_124_277
        );

        Ok(())
    }

    #[test]
    fn m6_q64_price_rejects_unsupported_exponent() {
        assert_eq!(
            meteora_q64_price_from_bin_id(0x80000, 1),
            Err(MeteoraDlmmFailure::ArithmeticOverflow)
        );
        assert_eq!(
            meteora_q64_price_from_bin_id(-0x80000, 1),
            Err(MeteoraDlmmFailure::ArithmeticOverflow)
        );
    }

    #[test]
    fn m6_wide_mul_div_preserves_rounding() -> Result<(), MeteoraDlmmFailure> {
        let down = meteora_mul_div(u128::MAX, 2, 7, MeteoraRounding::Down)?;
        let up = meteora_mul_div(u128::MAX, 2, 7, MeteoraRounding::Up)?;

        assert_eq!(down, 97_223_533_405_982_418_132_392_744_980_505_203_272);
        assert_eq!(up, 97_223_533_405_982_418_132_392_744_980_505_203_273);
        assert_eq!(
            meteora_mul_div(1, 1, 0, MeteoraRounding::Down),
            Err(MeteoraDlmmFailure::ArithmeticOverflow)
        );

        Ok(())
    }

    #[test]
    fn m6_directional_amount_math_uses_protocol_rounding() -> Result<(), MeteoraDlmmFailure> {
        let price = meteora_q64_price_from_bin_id(1, 25)?;

        let out_y = meteora_amount_out(1_000, price, true, MeteoraRounding::Down)?;
        let required_x = meteora_amount_in(out_y, price, true, MeteoraRounding::Up)?;
        let out_x = meteora_amount_out(1_000, price, false, MeteoraRounding::Down)?;
        let required_y = meteora_amount_in(out_x, price, false, MeteoraRounding::Up)?;

        assert_eq!(out_y, 1_002);
        assert_eq!(required_x, 1_000);
        assert_eq!(out_x, 997);
        assert_eq!(required_y, 1000);

        Ok(())
    }

    #[test]
    fn m6_fee_rates_match_frozen_vector() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.base_factor = 5_000;
        lb_pair.bin_step = 25;
        lb_pair.base_fee_power_factor = 0;
        lb_pair.variable_fee_control = 1_000;

        assert_eq!(meteora_base_fee_rate(&lb_pair)?, 1_250_000);
        assert_eq!(meteora_variable_fee_rate(&lb_pair, 300_000)?, 562_500);
        assert_eq!(meteora_total_fee_rate(&lb_pair, 300_000)?, 1_812_500);

        Ok(())
    }

    #[test]
    fn m6_total_fee_rate_is_capped() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.base_factor = u16::MAX;
        lb_pair.bin_step = u16::MAX;
        lb_pair.base_fee_power_factor = 4;
        lb_pair.variable_fee_control = 0;

        assert_eq!(
            meteora_total_fee_rate(&lb_pair, 0)?,
            u128::from(MAX_FEE_RATE)
        );

        Ok(())
    }

    #[test]
    fn m6_fee_amounts_preserve_ceiling_and_protocol_floor() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.base_factor = 5_000;
        lb_pair.bin_step = 25;
        lb_pair.base_fee_power_factor = 0;
        lb_pair.variable_fee_control = 1_000;
        lb_pair.protocol_share = 2_500;

        assert_eq!(meteora_compute_fee(&lb_pair, 300_000, 1_000_000)?, 1_816);
        assert_eq!(
            meteora_compute_fee_from_amount(&lb_pair, 300_000, 1_000_000)?,
            1_813
        );
        assert_eq!(meteora_protocol_fee_amount(&lb_pair, 12_345)?, 3_086);

        Ok(())
    }

    #[test]
    fn m6_dynamic_fee_boundaries_use_reduction_ratio() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.filter_period = 10;
        lb_pair.decay_period = 20;
        lb_pair.reduction_factor = 5_000;
        lb_pair.volatility_accumulator = 300_000;
        lb_pair.volatility_reference = 300_000;
        lb_pair.index_reference = 7;
        lb_pair.active_id = 7;
        lb_pair.last_update_timestamp = 1_000;
        lb_pair.max_volatility_accumulator = 1_000_000;

        let before = meteora_update_volatility_reference(&lb_pair, 1_009)?;
        let at_filter = meteora_update_volatility_reference(&lb_pair, 1_010)?;
        let inside_decay = meteora_update_volatility_reference(&lb_pair, 1_019)?;
        let at_decay = meteora_update_volatility_reference(&lb_pair, 1_020)?;
        let after_decay = meteora_update_volatility_reference(&lb_pair, 1_021)?;

        assert_eq!(before.volatility_reference, 300_000);
        assert_eq!(at_filter.volatility_reference, 150_000);
        assert_eq!(inside_decay.volatility_reference, 150_000);
        assert_eq!(at_decay.volatility_reference, 0);
        assert_eq!(after_decay.volatility_reference, 0);
        assert_ne!(at_filter.volatility_reference, 295_000);

        Ok(())
    }

    #[test]
    fn m6_volatility_accumulator_tracks_delta_and_cap() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.filter_period = 100;
        lb_pair.decay_period = 200;
        lb_pair.volatility_reference = 50_000;
        lb_pair.index_reference = 10;
        lb_pair.last_update_timestamp = 1_000;
        lb_pair.max_volatility_accumulator = 75_000;

        let reference_state = meteora_update_volatility_reference(&lb_pair, 1_050)?;
        let state = meteora_update_volatility_accumulator(
            reference_state,
            lb_pair.max_volatility_accumulator,
            0,
        )?;

        assert_eq!(state.volatility_reference, 50_000);
        assert_eq!(state.index_reference, 10);
        assert_eq!(state.volatility_accumulator, 75_000);

        Ok(())
    }

    #[test]
    fn m6_volatility_accumulator_reuses_one_reference_state() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        lb_pair.filter_period = 10;
        lb_pair.decay_period = 20;
        lb_pair.reduction_factor = 5_000;
        lb_pair.volatility_accumulator = 300_000;
        lb_pair.volatility_reference = 300_000;
        lb_pair.index_reference = 5;
        lb_pair.active_id = 7;
        lb_pair.last_update_timestamp = 1_000;
        lb_pair.max_volatility_accumulator = 1_000_000;

        let reference_state = meteora_update_volatility_reference(&lb_pair, 1_010)?;
        let at_nine = meteora_update_volatility_accumulator(reference_state, 1_000_000, 9)?;
        let at_ten = meteora_update_volatility_accumulator(reference_state, 1_000_000, 10)?;

        assert_eq!(reference_state.index_reference, 7);
        assert_eq!(reference_state.volatility_reference, 150_000);
        assert_eq!(at_nine.volatility_accumulator, 170_000);
        assert_eq!(at_ten.volatility_accumulator, 180_000);

        Ok(())
    }

    #[test]
    fn m6_collect_fee_mode_is_directional_and_fail_closed() -> Result<(), MeteoraDlmmFailure> {
        assert_eq!(
            meteora_collect_fee_mode(0)?,
            MeteoraCollectFeeMode::InputOnly
        );
        assert_eq!(meteora_collect_fee_mode(1)?, MeteoraCollectFeeMode::OnlyY);
        assert!(meteora_fee_on_input(0, true)?);
        assert!(meteora_fee_on_input(0, false)?);
        assert!(!meteora_fee_on_input(1, true)?);
        assert!(meteora_fee_on_input(1, false)?);
        assert_eq!(
            meteora_collect_fee_mode(2),
            Err(MeteoraDlmmFailure::UnsupportedCollectFeeMode)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_preserves_order_and_index_lookup() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let inputs = vec![
            bin_array_snapshot_input(lb_pair_pubkey, 2)?,
            bin_array_snapshot_input(lb_pair_pubkey, -1)?,
            bin_array_snapshot_input(lb_pair_pubkey, 0)?,
        ];
        let snapshot = MeteoraDlmmSnapshot::new(
            lb_pair_pubkey,
            lb_pair,
            inputs,
            None,
            test_clock(),
            DlmmProtocolProfile::V0_12,
            test_source(),
        )?;

        let indexes = snapshot
            .bin_arrays()
            .iter()
            .map(MeteoraValidatedBinArray::index)
            .collect::<Vec<_>>();

        assert_eq!(indexes, vec![2, -1, 0]);
        let found_index = snapshot
            .bin_array_by_index(-1)?
            .map(MeteoraValidatedBinArray::index);
        assert_eq!(found_index, Some(-1));
        assert_eq!(snapshot.bin_array_by_index(1), Ok(None));
        assert_eq!(
            snapshot.bin_array_by_index(BIN_ARRAY_MAX_INDEX + 1),
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_rejects_bin_array_from_another_lb_pair() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let mut input = bin_array_snapshot_input(lb_pair_pubkey, -1)?;
        input.state.lb_pair = [8_u8; 32];

        assert_eq!(
            MeteoraDlmmSnapshot::new(
                lb_pair_pubkey,
                lb_pair,
                vec![input],
                None,
                test_clock(),
                DlmmProtocolProfile::V0_12,
                test_source(),
            ),
            Err(MeteoraDlmmFailure::InvalidBinArray)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_rejects_malformed_bin_count() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let mut input = bin_array_snapshot_input(lb_pair_pubkey, -1)?;
        assert!(input.state.bins.pop().is_some());

        assert_eq!(
            MeteoraDlmmSnapshot::new(
                lb_pair_pubkey,
                lb_pair,
                vec![input],
                None,
                test_clock(),
                DlmmProtocolProfile::V0_12,
                test_source(),
            ),
            Err(MeteoraDlmmFailure::InvalidBinArray)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_rejects_noncanonical_bin_array_pubkey() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let mut input = bin_array_snapshot_input(lb_pair_pubkey, -1)?;
        input.pubkey[0] ^= 1;

        assert_eq!(
            MeteoraDlmmSnapshot::new(
                lb_pair_pubkey,
                lb_pair,
                vec![input],
                None,
                test_clock(),
                DlmmProtocolProfile::V0_12,
                test_source(),
            ),
            Err(MeteoraDlmmFailure::LegacyOrNonCanonicalBinArray)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_rejects_duplicate_indexes() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let input = bin_array_snapshot_input(lb_pair_pubkey, 3)?;

        assert_eq!(
            MeteoraDlmmSnapshot::new(
                lb_pair_pubkey,
                lb_pair,
                vec![input.clone(), input],
                None,
                test_clock(),
                DlmmProtocolProfile::V0_12,
                test_source(),
            ),
            Err(MeteoraDlmmFailure::InvalidLayout)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_rejects_wrong_bitmap_extension() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let extension = MeteoraBitmapExtensionState {
            lb_pair: [8_u8; 32],
            positive_bin_array_bitmap: [[0_u64; 8]; 12],
            negative_bin_array_bitmap: [[0_u64; 8]; 12],
        };

        assert_eq!(
            MeteoraDlmmSnapshot::new(
                lb_pair_pubkey,
                lb_pair,
                Vec::new(),
                Some(extension),
                test_clock(),
                DlmmProtocolProfile::V0_12,
                test_source(),
            ),
            Err(MeteoraDlmmFailure::InvalidBitmapExtension)
        );

        Ok(())
    }

    #[test]
    fn m5_snapshot_retains_metadata_and_inputs() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let lb_pair = decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &valid_lb_pair_bytes())?;
        let snapshot = MeteoraDlmmSnapshot::new(
            lb_pair_pubkey,
            lb_pair,
            Vec::new(),
            None,
            test_clock(),
            DlmmProtocolProfile::V0_12,
            test_source(),
        )?;

        assert_eq!(snapshot.lb_pair_pubkey(), lb_pair_pubkey);
        assert_eq!(snapshot.mint_x(), [7_u8; 32]);
        assert_eq!(snapshot.mint_y(), [9_u8; 32]);
        assert_eq!(snapshot.clock(), test_clock());
        assert_eq!(snapshot.profile(), DlmmProtocolProfile::V0_12);
        assert_eq!(snapshot.source(), test_source());
        assert!(snapshot.bitmap_extension().is_none());

        Ok(())
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
        assert_eq!(
            decode_lb_pair(METEORA_DLMM_PROGRAM_ID, &data).map(|state| state.bin_array_bitmap[0]),
            Ok(0x0123_4567_89ab_cdef)
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
        write_bytes(
            &mut data,
            LB_PAIR_BIN_ARRAY_BITMAP_OFFSET,
            0x0123_4567_89ab_cdef_u64.to_le_bytes(),
        );

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

    fn bin_array_snapshot_input(
        lb_pair_pubkey: [u8; 32],
        index: i64,
    ) -> Result<MeteoraBinArraySnapshotInput, MeteoraDlmmFailure> {
        let mut data = valid_bin_array_bytes();
        write_bytes(&mut data, BIN_ARRAY_INDEX_OFFSET, index.to_le_bytes());
        write_bytes(&mut data, BIN_ARRAY_LB_PAIR_OFFSET, lb_pair_pubkey);
        let state = decode_bin_array(METEORA_DLMM_PROGRAM_ID, &data, DlmmProtocolProfile::V0_12)?;
        let (pubkey, _) = derive_bin_array_pda(lb_pair_pubkey, index)?;

        Ok(MeteoraBinArraySnapshotInput { pubkey, state })
    }

    fn test_clock() -> MeteoraClockSnapshot {
        MeteoraClockSnapshot {
            slot: 42_000,
            epoch_start_timestamp: 1_700_000_000,
            epoch: 500,
            leader_schedule_epoch: 501,
            unix_timestamp: 1_700_000_123,
        }
    }

    fn test_source() -> MeteoraSnapshotSource {
        MeteoraSnapshotSource {
            source_slot: 42_010,
            generation_id: 9,
        }
    }

    fn empty_bitmap_extension() -> MeteoraBitmapExtensionState {
        MeteoraBitmapExtensionState {
            lb_pair: [0_u8; 32],
            positive_bin_array_bitmap: [[0_u64; 8]; 12],
            negative_bin_array_bitmap: [[0_u64; 8]; 12],
        }
    }

    fn set_internal_bitmap_bit(bitmap: &mut MeteoraInternalBitmap, index: i64) {
        let bit_offset = index - INTERNAL_BITMAP_MIN_INDEX;
        let word_index = (bit_offset / BITMAP_WORD_BITS) as usize;
        let bit_index = (bit_offset % BITMAP_WORD_BITS) as u32;
        bitmap[word_index] |= 1_u64 << bit_index;
    }

    fn set_extension_bitmap_bit(bitmap: &mut MeteoraBitmapRegion, index: i64, positive: bool) {
        let logical_offset = if positive {
            index - POSITIVE_BITMAP_EXTENSION_MIN_INDEX
        } else {
            NEGATIVE_BITMAP_EXTENSION_MAX_INDEX - index
        };
        let chunk_index = (logical_offset / EXTENSION_BITMAP_BITS) as usize;
        let chunk_bit = logical_offset % EXTENSION_BITMAP_BITS;
        let word_index = (chunk_bit / BITMAP_WORD_BITS) as usize;
        let bit_index = (chunk_bit % BITMAP_WORD_BITS) as u32;
        bitmap[chunk_index][word_index] |= 1_u64 << bit_index;
    }

    fn write_bytes<const N: usize>(data: &mut [u8], offset: usize, bytes: [u8; N]) {
        data[offset..offset + N].copy_from_slice(&bytes);
    }
}
