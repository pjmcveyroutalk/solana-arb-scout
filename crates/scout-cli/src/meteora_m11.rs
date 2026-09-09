use crate::meteora::{MeteoraDlmmFailure, MeteoraDlmmSnapshot, MeteoraSnapshotSource};
use crate::meteora_m8::MeteoraExactInTraversalResult;
use crate::meteora_m9::MeteoraHydrationPlan;
use solana_pubkey::{pubkey, Pubkey};

const METEORA_DLMM_PROGRAM_PUBKEY: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const SPL_MEMO_PROGRAM_PUBKEY: Pubkey = pubkey!("MemoSq4gqABAXKb96qnH8TysNcWxMyWCqXgDLGmfcHr");
const SPL_TOKEN_PROGRAM_PUBKEY: Pubkey = pubkey!("TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA");
const TOKEN_2022_PROGRAM_PUBKEY: Pubkey = pubkey!("TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb");
const BIN_ARRAY_BITMAP_SEED: &[u8] = b"bitmap";
const EVENT_AUTHORITY_SEED: &[u8] = b"__event_authority";

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraAccountProvenance {
    Derived,
    HydratedValidated,
    ExecutionProvided,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraExecutionAccountKind {
    LbPair,
    BinArrayBitmapExtension,
    ReserveX,
    ReserveY,
    UserTokenIn,
    UserTokenOut,
    TokenXMint,
    TokenYMint,
    Oracle,
    HostFeeIn,
    User,
    TokenXProgram,
    TokenYProgram,
    MemoProgram,
    EventAuthority,
    Program,
    BinArrayQuoteRequired { index: i64 },
    BinArrayExecutionBuffer { index: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraPlannedAccount {
    pub kind: MeteoraExecutionAccountKind,
    pub pubkey: [u8; 32],
    pub is_signer: bool,
    pub is_writable: bool,
    pub provenance: MeteoraAccountProvenance,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraExecutionProvidedAccounts {
    pub reserve_x: [u8; 32],
    pub reserve_y: [u8; 32],
    pub user_token_in: [u8; 32],
    pub user_token_out: [u8; 32],
    pub oracle: [u8; 32],
    pub user: [u8; 32],
    pub token_x_program: [u8; 32],
    pub token_y_program: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraExecutionAccountPlan {
    pub lb_pair_pubkey: [u8; 32],
    pub swap_for_y: bool,
    pub source: MeteoraSnapshotSource,
    pub fixed_accounts: Vec<MeteoraPlannedAccount>,
    pub remaining_accounts: Vec<MeteoraPlannedAccount>,
    pub contention_writable_accounts: Vec<[u8; 32]>,
}

impl MeteoraExecutionAccountPlan {
    pub fn ordered_accounts(&self) -> Vec<MeteoraPlannedAccount> {
        self.fixed_accounts
            .iter()
            .chain(self.remaining_accounts.iter())
            .copied()
            .collect()
    }

    pub fn ordered_pubkeys(&self) -> Vec<[u8; 32]> {
        self.fixed_accounts
            .iter()
            .chain(self.remaining_accounts.iter())
            .map(|account| account.pubkey)
            .collect()
    }
}

pub fn meteora_execution_account_plan(
    snapshot: &MeteoraDlmmSnapshot,
    hydration_plan: &MeteoraHydrationPlan,
    quote: &MeteoraExactInTraversalResult,
    swap_for_y: bool,
    provided: MeteoraExecutionProvidedAccounts,
    execution_buffer_indexes: &[i64],
) -> Result<MeteoraExecutionAccountPlan, MeteoraDlmmFailure> {
    quote.require_executable_full_fill()?;
    validate_snapshot_and_hydration(snapshot, hydration_plan, swap_for_y)?;
    validate_execution_provided_accounts(snapshot, provided)?;

    let remaining_accounts =
        build_remaining_accounts(snapshot, hydration_plan, quote, execution_buffer_indexes)?;
    let fixed_accounts = build_fixed_accounts(snapshot, provided);

    validate_duplicate_account_construction(&fixed_accounts, &remaining_accounts)?;
    let contention_writable_accounts =
        build_contention_writable_accounts(&fixed_accounts, &remaining_accounts);

    Ok(MeteoraExecutionAccountPlan {
        lb_pair_pubkey: snapshot.lb_pair_pubkey(),
        swap_for_y,
        source: snapshot.source(),
        fixed_accounts,
        remaining_accounts,
        contention_writable_accounts,
    })
}

fn validate_snapshot_and_hydration(
    snapshot: &MeteoraDlmmSnapshot,
    hydration_plan: &MeteoraHydrationPlan,
    swap_for_y: bool,
) -> Result<(), MeteoraDlmmFailure> {
    if hydration_plan.lb_pair_pubkey != snapshot.lb_pair_pubkey() {
        return Err(MeteoraDlmmFailure::InvalidLbPair);
    }
    if hydration_plan.swap_for_y != swap_for_y {
        return Err(MeteoraDlmmFailure::InvalidLayout);
    }
    hydration_plan.require_source(snapshot.source())?;
    Ok(())
}

fn validate_execution_provided_accounts(
    snapshot: &MeteoraDlmmSnapshot,
    provided: MeteoraExecutionProvidedAccounts,
) -> Result<(), MeteoraDlmmFailure> {
    if snapshot.mint_x() == snapshot.mint_y()
        || provided.reserve_x == provided.reserve_y
        || provided.user_token_in == provided.user_token_out
    {
        return Err(MeteoraDlmmFailure::InvalidLayout);
    }

    validate_token_program(provided.token_x_program)?;
    validate_token_program(provided.token_y_program)?;
    Ok(())
}

fn validate_token_program(program: [u8; 32]) -> Result<(), MeteoraDlmmFailure> {
    let pubkey = Pubkey::new_from_array(program);
    if pubkey == SPL_TOKEN_PROGRAM_PUBKEY || pubkey == TOKEN_2022_PROGRAM_PUBKEY {
        Ok(())
    } else {
        Err(MeteoraDlmmFailure::UnsupportedToken)
    }
}

fn build_fixed_accounts(
    snapshot: &MeteoraDlmmSnapshot,
    provided: MeteoraExecutionProvidedAccounts,
) -> Vec<MeteoraPlannedAccount> {
    let dlmm_program = METEORA_DLMM_PROGRAM_PUBKEY.to_bytes();
    let bitmap_extension = if snapshot.bitmap_extension().is_some() {
        derive_bitmap_extension(snapshot.lb_pair_pubkey()).0
    } else {
        dlmm_program
    };
    let event_authority = derive_event_authority().0;

    vec![
        planned_account(
            MeteoraExecutionAccountKind::LbPair,
            snapshot.lb_pair_pubkey(),
            false,
            true,
            MeteoraAccountProvenance::HydratedValidated,
        ),
        planned_account(
            MeteoraExecutionAccountKind::BinArrayBitmapExtension,
            bitmap_extension,
            false,
            true,
            if snapshot.bitmap_extension().is_some() {
                MeteoraAccountProvenance::HydratedValidated
            } else {
                MeteoraAccountProvenance::Derived
            },
        ),
        planned_account(
            MeteoraExecutionAccountKind::ReserveX,
            provided.reserve_x,
            false,
            true,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::ReserveY,
            provided.reserve_y,
            false,
            true,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::TokenXMint,
            snapshot.mint_x(),
            false,
            false,
            MeteoraAccountProvenance::HydratedValidated,
        ),
        planned_account(
            MeteoraExecutionAccountKind::TokenYMint,
            snapshot.mint_y(),
            false,
            false,
            MeteoraAccountProvenance::HydratedValidated,
        ),
        planned_account(
            MeteoraExecutionAccountKind::TokenXProgram,
            provided.token_x_program,
            false,
            false,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::TokenYProgram,
            provided.token_y_program,
            false,
            false,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::User,
            provided.user,
            true,
            false,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::UserTokenIn,
            provided.user_token_in,
            false,
            true,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::UserTokenOut,
            provided.user_token_out,
            false,
            true,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::Oracle,
            provided.oracle,
            false,
            true,
            MeteoraAccountProvenance::ExecutionProvided,
        ),
        planned_account(
            MeteoraExecutionAccountKind::HostFeeIn,
            dlmm_program,
            false,
            true,
            MeteoraAccountProvenance::Derived,
        ),
        planned_account(
            MeteoraExecutionAccountKind::EventAuthority,
            event_authority,
            false,
            false,
            MeteoraAccountProvenance::Derived,
        ),
        planned_account(
            MeteoraExecutionAccountKind::Program,
            dlmm_program,
            false,
            false,
            MeteoraAccountProvenance::Derived,
        ),
        planned_account(
            MeteoraExecutionAccountKind::MemoProgram,
            SPL_MEMO_PROGRAM_PUBKEY.to_bytes(),
            false,
            false,
            MeteoraAccountProvenance::Derived,
        ),
    ]
}

fn build_remaining_accounts(
    snapshot: &MeteoraDlmmSnapshot,
    hydration_plan: &MeteoraHydrationPlan,
    quote: &MeteoraExactInTraversalResult,
    execution_buffer_indexes: &[i64],
) -> Result<Vec<MeteoraPlannedAccount>, MeteoraDlmmFailure> {
    validate_index_list_unique(&quote.touched_bin_arrays)?;
    validate_index_list_unique(execution_buffer_indexes)?;

    for index in execution_buffer_indexes {
        if quote.touched_bin_arrays.contains(index) {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }

    let selected_count = quote
        .touched_bin_arrays
        .len()
        .checked_add(execution_buffer_indexes.len())
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
    if selected_count > hydration_plan.targets.len() {
        return Err(MeteoraDlmmFailure::InsufficientHydration);
    }

    for (position, required_index) in quote.touched_bin_arrays.iter().enumerate() {
        if hydration_plan.targets[position].index != *required_index {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }
    for (offset, buffer_index) in execution_buffer_indexes.iter().enumerate() {
        let position = quote
            .touched_bin_arrays
            .len()
            .checked_add(offset)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;
        if hydration_plan.targets[position].index != *buffer_index {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }

    let mut remaining_accounts = Vec::with_capacity(selected_count);
    for (position, target) in hydration_plan
        .targets
        .iter()
        .take(selected_count)
        .enumerate()
    {
        let hydrated = snapshot
            .bin_array_by_index(target.index)?
            .ok_or(MeteoraDlmmFailure::InsufficientHydration)?;
        if target.pubkey != hydrated.pubkey() {
            return Err(MeteoraDlmmFailure::InvalidBinArray);
        }

        let kind = if position < quote.touched_bin_arrays.len() {
            MeteoraExecutionAccountKind::BinArrayQuoteRequired {
                index: target.index,
            }
        } else {
            MeteoraExecutionAccountKind::BinArrayExecutionBuffer {
                index: target.index,
            }
        };
        remaining_accounts.push(planned_account(
            kind,
            target.pubkey,
            false,
            true,
            MeteoraAccountProvenance::HydratedValidated,
        ));
    }

    validate_required_order(&remaining_accounts, &quote.touched_bin_arrays)?;
    Ok(remaining_accounts)
}

fn validate_required_order(
    remaining_accounts: &[MeteoraPlannedAccount],
    required_indexes: &[i64],
) -> Result<(), MeteoraDlmmFailure> {
    let actual_required: Vec<i64> = remaining_accounts
        .iter()
        .filter_map(|account| match account.kind {
            MeteoraExecutionAccountKind::BinArrayQuoteRequired { index } => Some(index),
            _ => None,
        })
        .collect();

    if actual_required == required_indexes {
        Ok(())
    } else {
        Err(MeteoraDlmmFailure::InvalidLayout)
    }
}

fn validate_index_list_unique(indexes: &[i64]) -> Result<(), MeteoraDlmmFailure> {
    for (position, index) in indexes.iter().enumerate() {
        if indexes[..position].contains(index) {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }
    Ok(())
}

fn validate_duplicate_account_construction(
    fixed_accounts: &[MeteoraPlannedAccount],
    remaining_accounts: &[MeteoraPlannedAccount],
) -> Result<(), MeteoraDlmmFailure> {
    let ordered: Vec<MeteoraPlannedAccount> = fixed_accounts
        .iter()
        .chain(remaining_accounts.iter())
        .copied()
        .collect();
    let dlmm_program = METEORA_DLMM_PROGRAM_PUBKEY.to_bytes();

    for left in 0..ordered.len() {
        for right in (left + 1)..ordered.len() {
            if ordered[left].pubkey != ordered[right].pubkey {
                continue;
            }
            if ordered[left].pubkey == dlmm_program {
                continue;
            }
            if !ordered[left].is_writable && !ordered[right].is_writable {
                continue;
            }
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }
    Ok(())
}

fn build_contention_writable_accounts(
    fixed_accounts: &[MeteoraPlannedAccount],
    remaining_accounts: &[MeteoraPlannedAccount],
) -> Vec<[u8; 32]> {
    let dlmm_program = METEORA_DLMM_PROGRAM_PUBKEY.to_bytes();
    let mut contention = Vec::new();

    for account in fixed_accounts.iter().chain(remaining_accounts.iter()) {
        if !account.is_writable || account.pubkey == dlmm_program {
            continue;
        }
        if !contention.contains(&account.pubkey) {
            contention.push(account.pubkey);
        }
    }
    contention
}

fn planned_account(
    kind: MeteoraExecutionAccountKind,
    pubkey: [u8; 32],
    is_signer: bool,
    is_writable: bool,
    provenance: MeteoraAccountProvenance,
) -> MeteoraPlannedAccount {
    MeteoraPlannedAccount {
        kind,
        pubkey,
        is_signer,
        is_writable,
        provenance,
    }
}

fn derive_bitmap_extension(lb_pair: [u8; 32]) -> ([u8; 32], u8) {
    let lb_pair = Pubkey::new_from_array(lb_pair);
    let (pubkey, bump) = Pubkey::find_program_address(
        &[BIN_ARRAY_BITMAP_SEED, lb_pair.as_ref()],
        &METEORA_DLMM_PROGRAM_PUBKEY,
    );
    (pubkey.to_bytes(), bump)
}

fn derive_event_authority() -> ([u8; 32], u8) {
    let (pubkey, bump) =
        Pubkey::find_program_address(&[EVENT_AUTHORITY_SEED], &METEORA_DLMM_PROGRAM_PUBKEY);
    (pubkey.to_bytes(), bump)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meteora::{
        derive_bin_array_pda, DlmmProtocolProfile, MeteoraBin, MeteoraBinArraySnapshotInput,
        MeteoraBinArrayState, MeteoraBitmapExtensionState, MeteoraClockSnapshot,
        MeteoraInternalBitmap, MeteoraLbPairState, MeteoraSnapshotSource, MeteoraVolatilityState,
        BIN_ARRAY_VERSION_V3, INTERNAL_BITMAP_MIN_INDEX,
    };
    use crate::meteora_m8::{MeteoraExactInTermination, MeteoraExactInTraversalResult};
    use crate::meteora_m9::{MeteoraHydrationPlan, MeteoraHydrationTarget};

    const LB_PAIR: [u8; 32] = [7_u8; 32];

    #[test]
    fn canonical_fixed_shape_uses_sentinels_and_official_order() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], false, source(100, 5))?;
        let hydration = hydration_plan(&[0], source(100, 5), false)?;
        let quote = executable_quote(vec![0]);
        let provided = provided_accounts();

        let plan =
            meteora_execution_account_plan(&snapshot, &hydration, &quote, false, provided, &[])?;

        let kinds: Vec<MeteoraExecutionAccountKind> = plan
            .fixed_accounts
            .iter()
            .map(|account| account.kind)
            .collect();
        assert_eq!(
            kinds,
            vec![
                MeteoraExecutionAccountKind::LbPair,
                MeteoraExecutionAccountKind::BinArrayBitmapExtension,
                MeteoraExecutionAccountKind::ReserveX,
                MeteoraExecutionAccountKind::ReserveY,
                MeteoraExecutionAccountKind::TokenXMint,
                MeteoraExecutionAccountKind::TokenYMint,
                MeteoraExecutionAccountKind::TokenXProgram,
                MeteoraExecutionAccountKind::TokenYProgram,
                MeteoraExecutionAccountKind::User,
                MeteoraExecutionAccountKind::UserTokenIn,
                MeteoraExecutionAccountKind::UserTokenOut,
                MeteoraExecutionAccountKind::Oracle,
                MeteoraExecutionAccountKind::HostFeeIn,
                MeteoraExecutionAccountKind::EventAuthority,
                MeteoraExecutionAccountKind::Program,
                MeteoraExecutionAccountKind::MemoProgram,
            ]
        );
        assert_eq!(
            plan.fixed_accounts[1].pubkey,
            METEORA_DLMM_PROGRAM_PUBKEY.to_bytes()
        );
        assert_eq!(
            plan.fixed_accounts[12].pubkey,
            METEORA_DLMM_PROGRAM_PUBKEY.to_bytes()
        );
        Ok(())
    }

    #[test]
    fn present_bitmap_extension_uses_canonical_derived_account() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], true, source(101, 6))?;
        let hydration = hydration_plan(&[0], source(101, 6), false)?;
        let quote = executable_quote(vec![0]);

        let plan = meteora_execution_account_plan(
            &snapshot,
            &hydration,
            &quote,
            false,
            provided_accounts(),
            &[],
        )?;

        assert_eq!(
            plan.fixed_accounts[1].pubkey,
            derive_bitmap_extension(LB_PAIR).0
        );
        assert_eq!(
            plan.fixed_accounts[1].provenance,
            MeteoraAccountProvenance::HydratedValidated
        );
        Ok(())
    }

    #[test]
    fn quote_arrays_and_buffers_preserve_m9_order() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0, 2, 3], false, source(102, 7))?;
        let hydration = hydration_plan(&[0, 2, 3], source(102, 7), false)?;
        let quote = executable_quote(vec![0, 2]);

        let plan = meteora_execution_account_plan(
            &snapshot,
            &hydration,
            &quote,
            false,
            provided_accounts(),
            &[3],
        )?;

        assert_eq!(
            plan.remaining_accounts
                .iter()
                .map(|account| account.kind)
                .collect::<Vec<_>>(),
            vec![
                MeteoraExecutionAccountKind::BinArrayQuoteRequired { index: 0 },
                MeteoraExecutionAccountKind::BinArrayQuoteRequired { index: 2 },
                MeteoraExecutionAccountKind::BinArrayExecutionBuffer { index: 3 },
            ]
        );
        Ok(())
    }

    #[test]
    fn stale_hydration_generation_fails_closed() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], false, source(103, 8))?;
        let hydration = hydration_plan(&[0], source(102, 7), false)?;
        let quote = executable_quote(vec![0]);

        assert_eq!(
            meteora_execution_account_plan(
                &snapshot,
                &hydration,
                &quote,
                false,
                provided_accounts(),
                &[],
            ),
            Err(MeteoraDlmmFailure::StaleState)
        );
        Ok(())
    }

    #[test]
    fn unhydrated_buffer_fails_closed() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], false, source(104, 9))?;
        let hydration = hydration_plan(&[0], source(104, 9), false)?;
        let quote = executable_quote(vec![0]);

        assert_eq!(
            meteora_execution_account_plan(
                &snapshot,
                &hydration,
                &quote,
                false,
                provided_accounts(),
                &[2],
            ),
            Err(MeteoraDlmmFailure::InsufficientHydration)
        );
        Ok(())
    }

    #[test]
    fn duplicate_writable_execution_accounts_fail_closed() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], false, source(105, 10))?;
        let hydration = hydration_plan(&[0], source(105, 10), false)?;
        let quote = executable_quote(vec![0]);
        let mut provided = provided_accounts();
        provided.oracle = provided.reserve_x;

        assert_eq!(
            meteora_execution_account_plan(&snapshot, &hydration, &quote, false, provided, &[],),
            Err(MeteoraDlmmFailure::InvalidLayout)
        );
        Ok(())
    }

    #[test]
    fn contention_is_unique_and_excludes_program_sentinels() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(&[0], false, source(106, 11))?;
        let hydration = hydration_plan(&[0], source(106, 11), false)?;
        let quote = executable_quote(vec![0]);

        let plan = meteora_execution_account_plan(
            &snapshot,
            &hydration,
            &quote,
            false,
            provided_accounts(),
            &[],
        )?;

        assert!(!plan
            .contention_writable_accounts
            .contains(&METEORA_DLMM_PROGRAM_PUBKEY.to_bytes()));
        assert_eq!(
            plan.contention_writable_accounts
                .iter()
                .filter(|pubkey| **pubkey == LB_PAIR)
                .count(),
            1
        );
        Ok(())
    }

    fn provided_accounts() -> MeteoraExecutionProvidedAccounts {
        MeteoraExecutionProvidedAccounts {
            reserve_x: [11_u8; 32],
            reserve_y: [12_u8; 32],
            user_token_in: [13_u8; 32],
            user_token_out: [14_u8; 32],
            oracle: [15_u8; 32],
            user: [16_u8; 32],
            token_x_program: SPL_TOKEN_PROGRAM_PUBKEY.to_bytes(),
            token_y_program: SPL_TOKEN_PROGRAM_PUBKEY.to_bytes(),
        }
    }

    fn executable_quote(touched_bin_arrays: Vec<i64>) -> MeteoraExactInTraversalResult {
        MeteoraExactInTraversalResult {
            requested_input: 100,
            consumed_input: 100,
            unspent_input: 0,
            amount_out: 99,
            trading_fee: 1,
            protocol_fee: 0,
            user_fee: 1,
            fee_on_input: true,
            ending_active_id: 0,
            ending_volatility_state: MeteoraVolatilityState {
                volatility_accumulator: 0,
                volatility_reference: 0,
                index_reference: 0,
            },
            touched_bins: vec![0],
            touched_bin_arrays,
            termination: MeteoraExactInTermination::RequestedInputFullyConsumed,
        }
    }

    fn hydration_plan(
        indexes: &[i64],
        source: MeteoraSnapshotSource,
        swap_for_y: bool,
    ) -> Result<MeteoraHydrationPlan, MeteoraDlmmFailure> {
        let mut targets = Vec::new();
        for index in indexes {
            let (pubkey, bump) = derive_bin_array_pda(LB_PAIR, *index)?;
            targets.push(MeteoraHydrationTarget {
                index: *index,
                pubkey,
                bump,
            });
        }
        Ok(MeteoraHydrationPlan {
            lb_pair_pubkey: LB_PAIR,
            swap_for_y,
            source,
            targets,
        })
    }

    fn test_snapshot(
        indexes: &[i64],
        with_extension: bool,
        source: MeteoraSnapshotSource,
    ) -> Result<MeteoraDlmmSnapshot, MeteoraDlmmFailure> {
        let lb_pair = test_lb_pair();
        let mut bin_arrays = Vec::new();
        for index in indexes {
            let (pubkey, _) = derive_bin_array_pda(LB_PAIR, *index)?;
            bin_arrays.push(MeteoraBinArraySnapshotInput {
                pubkey,
                state: MeteoraBinArrayState {
                    index: *index,
                    version: BIN_ARRAY_VERSION_V3,
                    lb_pair: LB_PAIR,
                    bins: vec![zero_bin(); 70],
                },
            });
        }
        let extension = with_extension.then_some(MeteoraBitmapExtensionState {
            lb_pair: LB_PAIR,
            positive_bin_array_bitmap: [[0_u64; 8]; 12],
            negative_bin_array_bitmap: [[0_u64; 8]; 12],
        });

        MeteoraDlmmSnapshot::new(
            LB_PAIR,
            lb_pair,
            bin_arrays,
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

    fn test_lb_pair() -> MeteoraLbPairState {
        let mut bitmap: MeteoraInternalBitmap = [0_u64; 16];
        let offset = -INTERNAL_BITMAP_MIN_INDEX;
        bitmap[(offset / 64) as usize] |= 1_u64 << (offset % 64) as u32;

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
            index_reference: 0,
            last_update_timestamp: 0,
            active_id: 0,
            bin_step: 1,
            mint_x: [1_u8; 32],
            mint_y: [2_u8; 32],
            bin_array_bitmap: bitmap,
        }
    }

    fn zero_bin() -> MeteoraBin {
        MeteoraBin {
            amount_x: 0,
            amount_y: 0,
            price: 1_u128 << 64,
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
            limit_order_ask_side: 0,
        }
    }

    fn source(source_slot: u64, generation_id: u64) -> MeteoraSnapshotSource {
        MeteoraSnapshotSource {
            source_slot,
            generation_id,
        }
    }
}

