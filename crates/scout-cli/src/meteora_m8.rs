use crate::meteora::{
    bin_array_index_to_bin_range, bin_id_to_bin_array_index, bin_id_to_bin_array_offset,
    meteora_compute_fee, meteora_compute_fee_from_amount, meteora_exact_in_fill_at_bin,
    meteora_fee_on_input, meteora_split_fee, meteora_update_volatility_accumulator,
    meteora_update_volatility_reference, next_initialized_bin_array_index, MeteoraDlmmFailure,
    MeteoraDlmmSnapshot, MeteoraVolatilityState, BIN_ARRAY_MAX_INDEX, BIN_ARRAY_MIN_INDEX,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraExactInTermination {
    RequestedInputFullyConsumed,
    MissingHydratedBinArray { index: i64 },
    BitmapExtensionRequired,
    LiquiditySearchExhausted,
    ProtocolSearchRangeExceeded,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraExactInTraversalResult {
    pub requested_input: u64,
    pub consumed_input: u64,
    pub unspent_input: u64,
    pub amount_out: u64,
    pub trading_fee: u64,
    pub protocol_fee: u64,
    pub user_fee: u64,
    pub fee_on_input: bool,
    pub ending_active_id: i32,
    pub ending_volatility_state: MeteoraVolatilityState,
    pub touched_bins: Vec<i32>,
    pub touched_bin_arrays: Vec<i64>,
    pub termination: MeteoraExactInTermination,
}

impl MeteoraExactInTraversalResult {
    pub fn operational_failure(&self) -> Option<MeteoraDlmmFailure> {
        match self.termination {
            MeteoraExactInTermination::RequestedInputFullyConsumed => None,
            MeteoraExactInTermination::MissingHydratedBinArray { .. } => {
                Some(MeteoraDlmmFailure::InsufficientHydration)
            }
            MeteoraExactInTermination::BitmapExtensionRequired => {
                Some(MeteoraDlmmFailure::BitmapExtensionRequired)
            }
            MeteoraExactInTermination::LiquiditySearchExhausted => {
                Some(MeteoraDlmmFailure::InsufficientLiquidity)
            }
            MeteoraExactInTermination::ProtocolSearchRangeExceeded => {
                Some(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
            }
        }
    }

    pub fn require_executable_full_fill(&self) -> Result<(), MeteoraDlmmFailure> {
        if self.termination == MeteoraExactInTermination::RequestedInputFullyConsumed
            && self.consumed_input == self.requested_input
            && self.unspent_input == 0
        {
            return Ok(());
        }

        match self.termination {
            MeteoraExactInTermination::MissingHydratedBinArray { .. } => {
                Err(MeteoraDlmmFailure::InsufficientHydration)
            }
            MeteoraExactInTermination::BitmapExtensionRequired => {
                Err(MeteoraDlmmFailure::BitmapExtensionRequired)
            }
            MeteoraExactInTermination::ProtocolSearchRangeExceeded => {
                Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded)
            }
            MeteoraExactInTermination::RequestedInputFullyConsumed
            | MeteoraExactInTermination::LiquiditySearchExhausted => {
                Err(MeteoraDlmmFailure::PartialFillRejected)
            }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct MeteoraExactInBinAccounting {
    input_consumed: u64,
    amount_out: u64,
    trading_fee: u64,
    protocol_fee: u64,
}

pub fn meteora_exact_in_traverse(
    snapshot: &MeteoraDlmmSnapshot,
    requested_input: u64,
    swap_for_y: bool,
    support_limit_order: bool,
) -> Result<MeteoraExactInTraversalResult, MeteoraDlmmFailure> {
    let lb_pair = snapshot.lb_pair();
    let fee_on_input = meteora_fee_on_input(lb_pair.collect_fee_mode, swap_for_y)?;
    let reference_volatility =
        meteora_update_volatility_reference(lb_pair, snapshot.clock().unix_timestamp)?;
    let initial_array_index = match bin_id_to_bin_array_index(lb_pair.active_id) {
        Ok(index) => index,
        Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded) => {
            return finish_traversal(
                requested_input,
                requested_input,
                0,
                0,
                0,
                fee_on_input,
                lb_pair.active_id,
                reference_volatility,
                Vec::new(),
                Vec::new(),
                MeteoraExactInTermination::ProtocolSearchRangeExceeded,
            );
        }
        Err(error) => return Err(error),
    };

    if requested_input == 0 {
        return finish_traversal(
            0,
            0,
            0,
            0,
            0,
            fee_on_input,
            lb_pair.active_id,
            reference_volatility,
            Vec::new(),
            Vec::new(),
            MeteoraExactInTermination::RequestedInputFullyConsumed,
        );
    }

    let array_step = if swap_for_y { -1_i64 } else { 1_i64 };
    let bin_step = if swap_for_y { -1_i32 } else { 1_i32 };
    let terminal_array_index = if swap_for_y {
        BIN_ARRAY_MIN_INDEX
    } else {
        BIN_ARRAY_MAX_INDEX
    };

    let mut amount_left = requested_input;
    let mut amount_out = 0_u64;
    let mut trading_fee = 0_u64;
    let mut protocol_fee = 0_u64;
    let mut ending_active_id = lb_pair.active_id;
    let mut ending_volatility_state = reference_volatility;
    let mut touched_bins = Vec::new();
    let mut touched_bin_arrays = Vec::new();
    let mut search_start_index = initial_array_index;
    let mut first_hydrated_array = true;

    loop {
        let initialized_array_index = match next_initialized_bin_array_index(
            &lb_pair.bin_array_bitmap,
            snapshot.bitmap_extension(),
            search_start_index,
            swap_for_y,
        ) {
            Ok(Some(index)) => index,
            Ok(None) => {
                return finish_traversal(
                    requested_input,
                    amount_left,
                    amount_out,
                    trading_fee,
                    protocol_fee,
                    fee_on_input,
                    ending_active_id,
                    ending_volatility_state,
                    touched_bins,
                    touched_bin_arrays,
                    MeteoraExactInTermination::LiquiditySearchExhausted,
                );
            }
            Err(MeteoraDlmmFailure::BitmapExtensionRequired) => {
                return finish_traversal(
                    requested_input,
                    amount_left,
                    amount_out,
                    trading_fee,
                    protocol_fee,
                    fee_on_input,
                    ending_active_id,
                    ending_volatility_state,
                    touched_bins,
                    touched_bin_arrays,
                    MeteoraExactInTermination::BitmapExtensionRequired,
                );
            }
            Err(MeteoraDlmmFailure::ProtocolSearchRangeExceeded) => {
                return finish_traversal(
                    requested_input,
                    amount_left,
                    amount_out,
                    trading_fee,
                    protocol_fee,
                    fee_on_input,
                    ending_active_id,
                    ending_volatility_state,
                    touched_bins,
                    touched_bin_arrays,
                    MeteoraExactInTermination::ProtocolSearchRangeExceeded,
                );
            }
            Err(error) => return Err(error),
        };

        let bin_array = match snapshot.bin_array_by_index(initialized_array_index)? {
            Some(array) => array,
            None => {
                return finish_traversal(
                    requested_input,
                    amount_left,
                    amount_out,
                    trading_fee,
                    protocol_fee,
                    fee_on_input,
                    ending_active_id,
                    ending_volatility_state,
                    touched_bins,
                    touched_bin_arrays,
                    MeteoraExactInTermination::MissingHydratedBinArray {
                        index: initialized_array_index,
                    },
                );
            }
        };

        touched_bin_arrays.push(initialized_array_index);

        let (lower_bin_id, upper_bin_id) = bin_array_index_to_bin_range(initialized_array_index)?;
        let mut bin_id = if first_hydrated_array && initialized_array_index == initial_array_index {
            lb_pair.active_id
        } else if swap_for_y {
            upper_bin_id
        } else {
            lower_bin_id
        };
        let terminal_bin_id = if swap_for_y {
            lower_bin_id
        } else {
            upper_bin_id
        };

        loop {
            let offset = bin_id_to_bin_array_offset(bin_id)?;
            let bin = bin_array
                .state()
                .bins
                .get(offset)
                .ok_or(MeteoraDlmmFailure::InvalidLayout)?;

            touched_bins.push(bin_id);
            ending_active_id = bin_id;
            ending_volatility_state = meteora_update_volatility_accumulator(
                reference_volatility,
                lb_pair.max_volatility_accumulator,
                bin_id,
            )?;

            let accounting = fill_bin_exact_in(
                lb_pair,
                bin,
                amount_left,
                swap_for_y,
                support_limit_order,
                fee_on_input,
                ending_volatility_state.volatility_accumulator,
            )?;

            amount_left = amount_left
                .checked_sub(accounting.input_consumed)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            amount_out = amount_out
                .checked_add(accounting.amount_out)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            trading_fee = trading_fee
                .checked_add(accounting.trading_fee)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            protocol_fee = protocol_fee
                .checked_add(accounting.protocol_fee)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            if amount_left == 0 {
                return finish_traversal(
                    requested_input,
                    amount_left,
                    amount_out,
                    trading_fee,
                    protocol_fee,
                    fee_on_input,
                    ending_active_id,
                    ending_volatility_state,
                    touched_bins,
                    touched_bin_arrays,
                    MeteoraExactInTermination::RequestedInputFullyConsumed,
                );
            }

            if bin_id == terminal_bin_id {
                break;
            }
            bin_id = bin_id
                .checked_add(bin_step)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
        }

        if initialized_array_index == terminal_array_index {
            return finish_traversal(
                requested_input,
                amount_left,
                amount_out,
                trading_fee,
                protocol_fee,
                fee_on_input,
                ending_active_id,
                ending_volatility_state,
                touched_bins,
                touched_bin_arrays,
                MeteoraExactInTermination::LiquiditySearchExhausted,
            );
        }

        search_start_index = initialized_array_index
            .checked_add(array_step)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
        first_hydrated_array = false;
    }
}

fn fill_bin_exact_in(
    lb_pair: &crate::meteora::MeteoraLbPairState,
    bin: &crate::meteora::MeteoraBin,
    amount_left: u64,
    swap_for_y: bool,
    support_limit_order: bool,
    fee_on_input: bool,
    volatility_accumulator: u32,
) -> Result<MeteoraExactInBinAccounting, MeteoraDlmmFailure> {
    if amount_left == 0 {
        return Ok(MeteoraExactInBinAccounting {
            input_consumed: 0,
            amount_out: 0,
            trading_fee: 0,
            protocol_fee: 0,
        });
    }

    if fee_on_input {
        let all_input_fee =
            meteora_compute_fee_from_amount(lb_pair, volatility_accumulator, amount_left)?;
        let available_swap_input = amount_left
            .checked_sub(all_input_fee)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
        let fill = meteora_exact_in_fill_at_bin(
            bin,
            bin.price,
            available_swap_input,
            swap_for_y,
            support_limit_order,
        )?;

        if fill.amount_in == 0 {
            return Ok(MeteoraExactInBinAccounting {
                input_consumed: 0,
                amount_out: 0,
                trading_fee: 0,
                protocol_fee: 0,
            });
        }

        let (input_consumed, trading_fee) = if fill.amount_left == 0 {
            (amount_left, all_input_fee)
        } else {
            let fee = meteora_compute_fee(lb_pair, volatility_accumulator, fill.amount_in)?;
            let consumed = fill
                .amount_in
                .checked_add(fee)
                .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
            if consumed > amount_left {
                return Err(MeteoraDlmmFailure::ArithmeticOverflow);
            }
            (consumed, fee)
        };
        let fee_split = meteora_split_fee(
            trading_fee,
            lb_pair.protocol_share,
            fill.mm_amount_in,
            fill.amount_in,
        )?;

        return Ok(MeteoraExactInBinAccounting {
            input_consumed,
            amount_out: fill.amount_out,
            trading_fee,
            protocol_fee: fee_split.protocol_fee,
        });
    }

    let fill =
        meteora_exact_in_fill_at_bin(bin, bin.price, amount_left, swap_for_y, support_limit_order)?;
    if fill.amount_in == 0 {
        return Ok(MeteoraExactInBinAccounting {
            input_consumed: 0,
            amount_out: 0,
            trading_fee: 0,
            protocol_fee: 0,
        });
    }

    let trading_fee =
        meteora_compute_fee_from_amount(lb_pair, volatility_accumulator, fill.amount_out)?;
    let amount_out = fill
        .amount_out
        .checked_sub(trading_fee)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let fee_split = meteora_split_fee(
        trading_fee,
        lb_pair.protocol_share,
        fill.mm_amount_in,
        fill.amount_in,
    )?;

    Ok(MeteoraExactInBinAccounting {
        input_consumed: fill.amount_in,
        amount_out,
        trading_fee,
        protocol_fee: fee_split.protocol_fee,
    })
}

#[allow(clippy::too_many_arguments)]
fn finish_traversal(
    requested_input: u64,
    unspent_input: u64,
    amount_out: u64,
    trading_fee: u64,
    protocol_fee: u64,
    fee_on_input: bool,
    ending_active_id: i32,
    ending_volatility_state: MeteoraVolatilityState,
    touched_bins: Vec<i32>,
    touched_bin_arrays: Vec<i64>,
    termination: MeteoraExactInTermination,
) -> Result<MeteoraExactInTraversalResult, MeteoraDlmmFailure> {
    let consumed_input = requested_input
        .checked_sub(unspent_input)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    let user_fee = trading_fee
        .checked_sub(protocol_fee)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    Ok(MeteoraExactInTraversalResult {
        requested_input,
        consumed_input,
        unspent_input,
        amount_out,
        trading_fee,
        protocol_fee,
        user_fee,
        fee_on_input,
        ending_active_id,
        ending_volatility_state,
        touched_bins,
        touched_bin_arrays,
        termination,
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meteora::{
        derive_bin_array_pda, DlmmProtocolProfile, MeteoraBin, MeteoraBinArraySnapshotInput,
        MeteoraBinArrayState, MeteoraBitmapExtensionState, MeteoraClockSnapshot,
        MeteoraInternalBitmap, MeteoraLbPairState, MeteoraSnapshotSource, BIN_ARRAY_VERSION_V3,
        INTERNAL_BITMAP_MIN_INDEX, MAX_BIN_PER_ARRAY, Q64_ONE,
    };

    #[test]
    fn m8_quote_termination_uses_remaining_input_not_original_input(
    ) -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[0] = test_bin(100, 100, false);
        array.state.bins[1] = test_bin(100, 100, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 150, false, true)?;

        assert_eq!(result.consumed_input, 150);
        assert_eq!(result.unspent_input, 0);
        assert_eq!(result.amount_out, 150);
        assert_eq!(result.ending_active_id, 1);
        assert_eq!(result.touched_bins, vec![0, 1]);
        assert_eq!(
            result.termination,
            MeteoraExactInTermination::RequestedInputFullyConsumed
        );
        assert_eq!(result.require_executable_full_fill(), Ok(()));

        Ok(())
    }

    #[test]
    fn m8_fully_consumed_input_does_not_advance_active_bin() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[0] = test_bin(200, 200, false);
        array.state.bins[1] = test_bin(200, 200, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 50, false, true)?;

        assert_eq!(result.consumed_input, 50);
        assert_eq!(result.unspent_input, 0);
        assert_eq!(result.amount_out, 50);
        assert_eq!(result.ending_active_id, 0);
        assert_eq!(result.touched_bins, vec![0]);

        Ok(())
    }

    #[test]
    fn m8_sparse_topology_skips_uninitialized_array_gaps() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 3);

        let mut array_zero = test_bin_array_input(lb_pair_pubkey, 0)?;
        array_zero.state.bins[0] = test_bin(100, 100, false);
        let mut array_three = test_bin_array_input(lb_pair_pubkey, 3)?;
        array_three.state.bins[0] = test_bin(100, 100, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array_zero, array_three])?;

        let result = meteora_exact_in_traverse(&snapshot, 150, false, true)?;

        assert_eq!(result.touched_bin_arrays, vec![0, 3]);
        assert!(result.touched_bins.contains(&0));
        assert!(result.touched_bins.contains(&210));
        assert!(!result
            .touched_bins
            .iter()
            .any(|bin_id| (70..210).contains(bin_id)));
        assert_eq!(result.amount_out, 150);
        assert_eq!(result.unspent_input, 0);

        Ok(())
    }

    #[test]
    fn m8_bitmap_known_missing_array_is_insufficient_hydration() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 3);

        let mut array_zero = test_bin_array_input(lb_pair_pubkey, 0)?;
        array_zero.state.bins[0] = test_bin(100, 100, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array_zero])?;

        let result = meteora_exact_in_traverse(&snapshot, 150, false, true)?;

        assert_eq!(result.consumed_input, 100);
        assert_eq!(result.unspent_input, 50);
        assert_eq!(result.amount_out, 100);
        assert_eq!(result.touched_bin_arrays, vec![0]);
        assert_eq!(
            result.termination,
            MeteoraExactInTermination::MissingHydratedBinArray { index: 3 }
        );
        assert_eq!(
            result.operational_failure(),
            Some(MeteoraDlmmFailure::InsufficientHydration)
        );
        assert_eq!(
            result.require_executable_full_fill(),
            Err(MeteoraDlmmFailure::InsufficientHydration)
        );

        Ok(())
    }

    #[test]
    fn m8_single_sided_wrong_direction_is_liquidity_exhausted() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[0] = test_bin(0, 100, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 50, false, true)?;

        assert_eq!(result.consumed_input, 0);
        assert_eq!(result.unspent_input, 50);
        assert_eq!(result.amount_out, 0);
        assert_eq!(
            result.termination,
            MeteoraExactInTermination::LiquiditySearchExhausted
        );
        assert_eq!(
            result.operational_failure(),
            Some(MeteoraDlmmFailure::InsufficientLiquidity)
        );
        assert_eq!(
            result.require_executable_full_fill(),
            Err(MeteoraDlmmFailure::PartialFillRejected)
        );

        Ok(())
    }

    #[test]
    fn m8_swap_for_y_traverses_downward_and_uses_y_liquidity() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(1);
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[1] = test_bin(200, 100, false);
        array.state.bins[0] = test_bin(200, 100, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 150, true, true)?;

        assert_eq!(result.consumed_input, 150);
        assert_eq!(result.amount_out, 150);
        assert_eq!(result.ending_active_id, 0);
        assert_eq!(result.touched_bins, vec![1, 0]);

        Ok(())
    }

    #[test]
    fn m8_input_fee_is_accounted_before_swap_input() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        lb_pair.base_factor = 1_000;
        lb_pair.bin_step = 10;
        lb_pair.protocol_share = 0;
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);
        let expected_fee = meteora_compute_fee_from_amount(&lb_pair, 0, 10_000)?;

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[0] = test_bin(20_000, 20_000, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 10_000, false, true)?;

        assert_eq!(result.trading_fee, expected_fee);
        assert_eq!(result.protocol_fee, 0);
        assert_eq!(result.user_fee, expected_fee);
        assert_eq!(result.consumed_input, 10_000);
        assert_eq!(result.amount_out, 10_000 - expected_fee);
        assert!(result.fee_on_input);

        Ok(())
    }

    #[test]
    fn m8_only_y_mode_collects_fee_from_y_output() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair_pubkey = [7_u8; 32];
        let mut lb_pair = test_lb_pair(0);
        lb_pair.base_factor = 1_000;
        lb_pair.bin_step = 10;
        lb_pair.collect_fee_mode = 1;
        lb_pair.protocol_share = 0;
        set_internal_bitmap_bit(&mut lb_pair.bin_array_bitmap, 0);
        let expected_fee = meteora_compute_fee_from_amount(&lb_pair, 0, 10_000)?;

        let mut array = test_bin_array_input(lb_pair_pubkey, 0)?;
        array.state.bins[0] = test_bin(20_000, 20_000, false);
        let snapshot = test_snapshot(lb_pair_pubkey, lb_pair, vec![array])?;

        let result = meteora_exact_in_traverse(&snapshot, 10_000, true, true)?;

        assert_eq!(result.trading_fee, expected_fee);
        assert_eq!(result.protocol_fee, 0);
        assert_eq!(result.user_fee, expected_fee);
        assert_eq!(result.consumed_input, 10_000);
        assert_eq!(result.amount_out, 10_000 - expected_fee);
        assert!(!result.fee_on_input);

        Ok(())
    }

    fn test_snapshot(
        lb_pair_pubkey: [u8; 32],
        lb_pair: MeteoraLbPairState,
        bin_arrays: Vec<MeteoraBinArraySnapshotInput>,
    ) -> Result<MeteoraDlmmSnapshot, MeteoraDlmmFailure> {
        MeteoraDlmmSnapshot::new(
            lb_pair_pubkey,
            lb_pair,
            bin_arrays,
            Some(MeteoraBitmapExtensionState {
                lb_pair: lb_pair_pubkey,
                positive_bin_array_bitmap: [[0_u64; 8]; 12],
                negative_bin_array_bitmap: [[0_u64; 8]; 12],
            }),
            MeteoraClockSnapshot {
                slot: 1,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 1_000,
            },
            DlmmProtocolProfile::V0_12,
            MeteoraSnapshotSource {
                source_slot: 1,
                generation_id: 1,
            },
        )
    }

    fn test_lb_pair(active_id: i32) -> MeteoraLbPairState {
        MeteoraLbPairState {
            base_factor: 0,
            filter_period: 10,
            decay_period: 20,
            reduction_factor: 5_000,
            variable_fee_control: 0,
            max_volatility_accumulator: 1_000_000,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 1_000,
            active_id,
            bin_step: 10,
            mint_x: [1_u8; 32],
            mint_y: [2_u8; 32],
            bin_array_bitmap: [0_u64; 16],
        }
    }

    fn test_bin_array_input(
        lb_pair_pubkey: [u8; 32],
        index: i64,
    ) -> Result<MeteoraBinArraySnapshotInput, MeteoraDlmmFailure> {
        let (pubkey, _) = derive_bin_array_pda(lb_pair_pubkey, index)?;
        let bins = vec![test_bin(0, 0, false); MAX_BIN_PER_ARRAY as usize];

        Ok(MeteoraBinArraySnapshotInput {
            pubkey,
            state: MeteoraBinArrayState {
                index,
                version: BIN_ARRAY_VERSION_V3,
                lb_pair: lb_pair_pubkey,
                bins,
            },
        })
    }

    fn test_bin(amount_x: u64, amount_y: u64, ask_side: bool) -> MeteoraBin {
        MeteoraBin {
            amount_x,
            amount_y,
            price: Q64_ONE,
            liquidity_supply: 0,
            fulfilled_order_amount_x: 0,
            fulfilled_order_amount_y: 0,
            limit_order_fee_ask_side: 0,
            limit_order_fee_bid_side: 0,
            fee_amount_x_per_token_stored: 0,
            fee_amount_y_per_token_stored: 0,
            open_order_amount: 0,
            total_processing_order_amount: 0,
            processed_order_remaining_amount: 0,
            order_age: 0,
            limit_order_ask_side: u8::from(ask_side),
        }
    }

    fn set_internal_bitmap_bit(bitmap: &mut MeteoraInternalBitmap, index: i64) {
        let bit_offset = index - INTERNAL_BITMAP_MIN_INDEX;
        let word_index = (bit_offset / 64) as usize;
        let bit_index = (bit_offset % 64) as u32;
        bitmap[word_index] |= 1_u64 << bit_index;
    }
}

