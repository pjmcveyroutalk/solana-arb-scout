use crate::meteora::{
    bin_id_to_bin_array_index, derive_bin_array_pda, next_initialized_bin_array_index,
    MeteoraDlmmFailure, MeteoraDlmmSnapshot, MeteoraSnapshotSource, BIN_ARRAY_MAX_INDEX,
    BIN_ARRAY_MIN_INDEX,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraHydrationTarget {
    pub index: i64,
    pub pubkey: [u8; 32],
    pub bump: u8,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraHydrationPlan {
    pub lb_pair_pubkey: [u8; 32],
    pub swap_for_y: bool,
    pub source: MeteoraSnapshotSource,
    pub targets: Vec<MeteoraHydrationTarget>,
}

impl MeteoraHydrationPlan {
    pub fn ordered_indexes(&self) -> Vec<i64> {
        self.targets.iter().map(|target| target.index).collect()
    }

    pub fn require_source(&self, source: MeteoraSnapshotSource) -> Result<(), MeteoraDlmmFailure> {
        if self.source == source {
            Ok(())
        } else {
            Err(MeteoraDlmmFailure::StaleState)
        }
    }
}

pub fn meteora_initial_hydration_plan(
    snapshot: &MeteoraDlmmSnapshot,
    swap_for_y: bool,
) -> Result<MeteoraHydrationPlan, MeteoraDlmmFailure> {
    plan_from_snapshot(snapshot, swap_for_y, None)
}

pub fn meteora_expand_hydration_plan(
    snapshot: &MeteoraDlmmSnapshot,
    prior_plan: &MeteoraHydrationPlan,
    missing_index: i64,
) -> Result<MeteoraHydrationPlan, MeteoraDlmmFailure> {
    require_new_coherent_generation(snapshot, prior_plan)?;

    let plan = plan_from_snapshot(snapshot, prior_plan.swap_for_y, Some(missing_index))?;
    if plan.targets.last().map(|target| target.index) != Some(missing_index) {
        return Err(MeteoraDlmmFailure::StaleState);
    }

    Ok(plan)
}

fn require_new_coherent_generation(
    snapshot: &MeteoraDlmmSnapshot,
    prior_plan: &MeteoraHydrationPlan,
) -> Result<(), MeteoraDlmmFailure> {
    if snapshot.lb_pair_pubkey() != prior_plan.lb_pair_pubkey {
        return Err(MeteoraDlmmFailure::InvalidLbPair);
    }

    let next_source = snapshot.source();
    if next_source.generation_id <= prior_plan.source.generation_id
        || next_source.source_slot < prior_plan.source.source_slot
    {
        return Err(MeteoraDlmmFailure::StaleState);
    }

    Ok(())
}

fn plan_from_snapshot(
    snapshot: &MeteoraDlmmSnapshot,
    swap_for_y: bool,
    stop_at: Option<i64>,
) -> Result<MeteoraHydrationPlan, MeteoraDlmmFailure> {
    let lb_pair = snapshot.lb_pair();
    let initial_index = bin_id_to_bin_array_index(lb_pair.active_id)?;
    let terminal_index = if swap_for_y {
        BIN_ARRAY_MIN_INDEX
    } else {
        BIN_ARRAY_MAX_INDEX
    };
    let step = if swap_for_y { -1_i64 } else { 1_i64 };
    let mut search_index = initial_index;
    let mut targets = Vec::new();

    loop {
        let next_index = next_initialized_bin_array_index(
            &lb_pair.bin_array_bitmap,
            snapshot.bitmap_extension(),
            search_index,
            swap_for_y,
        )?;

        let Some(index) = next_index else {
            break;
        };

        if let Some(required_index) = stop_at {
            if passed_required_index(index, required_index, swap_for_y) {
                return Err(MeteoraDlmmFailure::StaleState);
            }
        }

        let (pubkey, bump) = derive_bin_array_pda(snapshot.lb_pair_pubkey(), index)?;
        targets.push(MeteoraHydrationTarget {
            index,
            pubkey,
            bump,
        });

        match stop_at {
            None => break,
            Some(required_index) if index == required_index => break,
            Some(_) => {}
        }

        if index == terminal_index {
            break;
        }
        search_index = index
            .checked_add(step)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    }

    if stop_at.is_some() && targets.last().map(|target| target.index) != stop_at {
        return Err(MeteoraDlmmFailure::StaleState);
    }

    Ok(MeteoraHydrationPlan {
        lb_pair_pubkey: snapshot.lb_pair_pubkey(),
        swap_for_y,
        source: snapshot.source(),
        targets,
    })
}

fn passed_required_index(index: i64, required_index: i64, swap_for_y: bool) -> bool {
    if swap_for_y {
        index < required_index
    } else {
        index > required_index
    }
}

#[cfg(test)]
mod tests {
    use super::{meteora_expand_hydration_plan, meteora_initial_hydration_plan};
    use crate::meteora::{
        derive_bin_array_pda, DlmmProtocolProfile, MeteoraBitmapExtensionState,
        MeteoraClockSnapshot, MeteoraDlmmFailure, MeteoraDlmmSnapshot, MeteoraLbPairState,
        MeteoraSnapshotSource, INTERNAL_BITMAP_MIN_INDEX, MAX_BIN_PER_ARRAY,
    };

    const LB_PAIR: [u8; 32] = [7_u8; 32];

    #[test]
    fn initial_plan_selects_only_first_initialized_array() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = test_lb_pair(0);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 0);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 3);
        let snapshot = test_snapshot(lb_pair, None, source(10, 1))?;

        let plan = meteora_initial_hydration_plan(&snapshot, false)?;

        assert_eq!(plan.ordered_indexes(), vec![0]);
        assert_eq!(plan.source, source(10, 1));
        Ok(())
    }

    #[test]
    fn expansion_is_sparse_directional_and_rebuilt_from_new_generation(
    ) -> Result<(), MeteoraDlmmFailure> {
        let mut first = test_lb_pair(0);
        set_internal_bit(&mut first.bin_array_bitmap, 0);
        set_internal_bit(&mut first.bin_array_bitmap, 3);
        let first_snapshot = test_snapshot(first, None, source(10, 1))?;
        let prior = meteora_initial_hydration_plan(&first_snapshot, false)?;

        let mut refreshed = test_lb_pair(0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 2);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 3);
        let refreshed_snapshot = test_snapshot(refreshed, None, source(11, 2))?;
        let expanded = meteora_expand_hydration_plan(&refreshed_snapshot, &prior, 3)?;

        assert_eq!(expanded.ordered_indexes(), vec![0, 2, 3]);
        assert_eq!(expanded.source, source(11, 2));
        Ok(())
    }

    #[test]
    fn downward_expansion_preserves_directional_order() -> Result<(), MeteoraDlmmFailure> {
        let mut first = test_lb_pair(0);
        set_internal_bit(&mut first.bin_array_bitmap, 0);
        set_internal_bit(&mut first.bin_array_bitmap, -4);
        let first_snapshot = test_snapshot(first, None, source(20, 4))?;
        let prior = meteora_initial_hydration_plan(&first_snapshot, true)?;

        let mut refreshed = test_lb_pair(0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, -2);
        set_internal_bit(&mut refreshed.bin_array_bitmap, -4);
        let refreshed_snapshot = test_snapshot(refreshed, None, source(21, 5))?;
        let expanded = meteora_expand_hydration_plan(&refreshed_snapshot, &prior, -4)?;

        assert_eq!(expanded.ordered_indexes(), vec![0, -2, -4]);
        Ok(())
    }

    #[test]
    fn missing_extension_fails_closed_at_positive_seam() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair = test_lb_pair(511 * MAX_BIN_PER_ARRAY);
        let snapshot = test_snapshot(lb_pair, None, source(30, 7))?;

        assert_eq!(
            meteora_initial_hydration_plan(&snapshot, false),
            Err(MeteoraDlmmFailure::BitmapExtensionRequired)
        );
        Ok(())
    }

    #[test]
    fn present_extension_selects_initialized_array_across_seam() -> Result<(), MeteoraDlmmFailure> {
        let lb_pair = test_lb_pair(511 * MAX_BIN_PER_ARRAY);
        let mut extension = test_extension();
        set_positive_extension_bit(&mut extension, 512);
        let snapshot = test_snapshot(lb_pair, Some(extension), source(40, 8))?;
        let plan = meteora_initial_hydration_plan(&snapshot, false)?;

        assert_eq!(plan.ordered_indexes(), vec![512]);
        let (expected_pubkey, expected_bump) = derive_bin_array_pda(LB_PAIR, 512)?;
        assert_eq!(
            plan.targets
                .first()
                .map(|target| (target.pubkey, target.bump)),
            Some((expected_pubkey, expected_bump))
        );
        Ok(())
    }

    #[test]
    fn present_negative_extension_selects_initialized_array_across_seam(
    ) -> Result<(), MeteoraDlmmFailure> {
        let lb_pair = test_lb_pair(-512 * MAX_BIN_PER_ARRAY);
        let mut extension = test_extension();
        set_negative_extension_bit(&mut extension, -513);
        let snapshot = test_snapshot(lb_pair, Some(extension), source(45, 9))?;
        let plan = meteora_initial_hydration_plan(&snapshot, true)?;

        assert_eq!(plan.ordered_indexes(), vec![-513]);
        Ok(())
    }

    #[test]
    fn planner_stops_at_protocol_boundary_without_hydrating_domain(
    ) -> Result<(), MeteoraDlmmFailure> {
        let lb_pair = test_lb_pair(6_655 * MAX_BIN_PER_ARRAY);
        let snapshot = test_snapshot(lb_pair, Some(test_extension()), source(46, 10))?;
        let plan = meteora_initial_hydration_plan(&snapshot, false)?;

        assert!(plan.targets.is_empty());
        Ok(())
    }

    #[test]
    fn expansion_requires_a_new_non_regressing_generation() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = test_lb_pair(0);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 0);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 3);
        let first_snapshot = test_snapshot(lb_pair, None, source(50, 9))?;
        let prior = meteora_initial_hydration_plan(&first_snapshot, false)?;

        let same_generation = test_snapshot(lb_pair, None, source(50, 9))?;
        assert_eq!(
            meteora_expand_hydration_plan(&same_generation, &prior, 3),
            Err(MeteoraDlmmFailure::StaleState)
        );

        let regressed_slot = test_snapshot(lb_pair, None, source(49, 10))?;
        assert_eq!(
            meteora_expand_hydration_plan(&regressed_slot, &prior, 3),
            Err(MeteoraDlmmFailure::StaleState)
        );
        Ok(())
    }

    #[test]
    fn expansion_rejects_target_that_is_no_longer_initialized() -> Result<(), MeteoraDlmmFailure> {
        let mut first = test_lb_pair(0);
        set_internal_bit(&mut first.bin_array_bitmap, 0);
        set_internal_bit(&mut first.bin_array_bitmap, 3);
        let first_snapshot = test_snapshot(first, None, source(60, 11))?;
        let prior = meteora_initial_hydration_plan(&first_snapshot, false)?;

        let mut refreshed = test_lb_pair(0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 0);
        set_internal_bit(&mut refreshed.bin_array_bitmap, 4);
        let refreshed_snapshot = test_snapshot(refreshed, None, source(61, 12))?;

        assert_eq!(
            meteora_expand_hydration_plan(&refreshed_snapshot, &prior, 3),
            Err(MeteoraDlmmFailure::StaleState)
        );
        Ok(())
    }

    #[test]
    fn plan_source_guard_rejects_cross_generation_use() -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = test_lb_pair(0);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 0);
        let snapshot = test_snapshot(lb_pair, None, source(70, 13))?;
        let plan = meteora_initial_hydration_plan(&snapshot, false)?;

        assert_eq!(plan.require_source(source(70, 13)), Ok(()));
        assert_eq!(
            plan.require_source(source(71, 14)),
            Err(MeteoraDlmmFailure::StaleState)
        );
        Ok(())
    }

    fn test_snapshot(
        lb_pair: MeteoraLbPairState,
        extension: Option<MeteoraBitmapExtensionState>,
        source: MeteoraSnapshotSource,
    ) -> Result<MeteoraDlmmSnapshot, MeteoraDlmmFailure> {
        MeteoraDlmmSnapshot::new(
            LB_PAIR,
            lb_pair,
            Vec::new(),
            extension,
            MeteoraClockSnapshot {
                slot: source.source_slot,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            DlmmProtocolProfile::V0_12,
            source,
        )
    }

    fn test_lb_pair(active_id: i32) -> MeteoraLbPairState {
        MeteoraLbPairState {
            base_factor: 1,
            filter_period: 0,
            decay_period: 0,
            reduction_factor: 0,
            variable_fee_control: 0,
            max_volatility_accumulator: 0,
            protocol_share: 0,
            base_fee_power_factor: 0,
            collect_fee_mode: 0,
            volatility_accumulator: 0,
            volatility_reference: 0,
            index_reference: active_id,
            last_update_timestamp: 0,
            active_id,
            bin_step: 1,
            mint_x: [1_u8; 32],
            mint_y: [2_u8; 32],
            bin_array_bitmap: [0_u64; 16],
        }
    }

    fn test_extension() -> MeteoraBitmapExtensionState {
        MeteoraBitmapExtensionState {
            lb_pair: LB_PAIR,
            positive_bin_array_bitmap: [[0_u64; 8]; 12],
            negative_bin_array_bitmap: [[0_u64; 8]; 12],
        }
    }

    fn source(source_slot: u64, generation_id: u64) -> MeteoraSnapshotSource {
        MeteoraSnapshotSource {
            source_slot,
            generation_id,
        }
    }

    fn set_internal_bit(bitmap: &mut [u64; 16], index: i64) {
        let offset = index - INTERNAL_BITMAP_MIN_INDEX;
        let word_index = (offset / 64) as usize;
        let bit_index = (offset % 64) as u32;
        bitmap[word_index] |= 1_u64 << bit_index;
    }

    fn set_positive_extension_bit(extension: &mut MeteoraBitmapExtensionState, index: i64) {
        let offset = index - 512;
        let chunk_index = (offset / 512) as usize;
        let chunk_bit = offset % 512;
        let word_index = (chunk_bit / 64) as usize;
        let bit_index = (chunk_bit % 64) as u32;
        extension.positive_bin_array_bitmap[chunk_index][word_index] |= 1_u64 << bit_index;
    }

    fn set_negative_extension_bit(extension: &mut MeteoraBitmapExtensionState, index: i64) {
        let offset = -513 - index;
        let chunk_index = (offset / 512) as usize;
        let chunk_bit = offset % 512;
        let word_index = (chunk_bit / 64) as usize;
        let bit_index = (chunk_bit % 64) as u32;
        extension.negative_bin_array_bitmap[chunk_index][word_index] |= 1_u64 << bit_index;
    }
}
