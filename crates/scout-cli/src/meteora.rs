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
}
