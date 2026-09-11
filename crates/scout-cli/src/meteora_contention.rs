use crate::meteora::{MeteoraDlmmSnapshot, MeteoraSnapshotSource};
use crate::meteora_m13::MeteoraM13ExactInputQuote;
use solana_pubkey::{pubkey, Pubkey};

const METEORA_DLMM_PROGRAM_PUBKEY: Pubkey = pubkey!("LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo");
const BIN_ARRAY_BITMAP_SEED: &[u8] = b"bitmap";
const ORACLE_SEED: &[u8] = b"oracle";

pub const METEORA_CONTENTION_PROVENANCE: &str = concat!(
    "Meteora DLMM deterministic protocol writable subset: lb_pair, optional bitmap_extension, ",
    "reserve_x, reserve_y, oracle, exact quote-touched BinArrays; bound to the exact snapshot ",
    "source slot/generation and quote direction; not the complete future transaction writable ",
    "set; executor-dependent user/user-token accounts, token/memo/event/program accounts, ",
    "host-fee sentinel, and speculative execution-buffer BinArrays are excluded"
);

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MeteoraContentionAccountKind {
    LbPair,
    BinArrayBitmapExtension,
    ReserveX,
    ReserveY,
    Oracle,
    QuoteTouchedBinArray { index: i64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MeteoraContentionAccount {
    pub kind: MeteoraContentionAccountKind,
    pub pubkey: [u8; 32],
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraContentionFootprint {
    pub lb_pair_pubkey: [u8; 32],
    pub source: MeteoraSnapshotSource,
    pub swap_for_y: bool,
    pub requested_input_raw: u64,
    pub accounts: Vec<MeteoraContentionAccount>,
}

impl MeteoraContentionFootprint {
    pub fn account_pubkeys(&self) -> Vec<[u8; 32]> {
        self.accounts.iter().map(|account| account.pubkey).collect()
    }

    pub fn account_pubkeys_base58(&self) -> Vec<String> {
        self.accounts
            .iter()
            .map(|account| bs58::encode(account.pubkey).into_string())
            .collect()
    }

    pub fn provenance(&self) -> &'static str {
        METEORA_CONTENTION_PROVENANCE
    }
}

pub fn meteora_quote_contention_footprint(
    snapshot: &MeteoraDlmmSnapshot,
    quote: &MeteoraM13ExactInputQuote,
) -> Result<MeteoraContentionFootprint, String> {
    validate_quote_binding(snapshot, quote)?;
    validate_touched_bin_arrays(&quote.touched_bin_arrays)?;

    let lb_pair_pubkey = snapshot.lb_pair_pubkey();
    let mut accounts = Vec::new();

    push_unique_account(
        &mut accounts,
        MeteoraContentionAccountKind::LbPair,
        lb_pair_pubkey,
    )?;

    if snapshot.bitmap_extension().is_some() {
        push_unique_account(
            &mut accounts,
            MeteoraContentionAccountKind::BinArrayBitmapExtension,
            derive_bitmap_extension(lb_pair_pubkey),
        )?;
    }

    push_unique_account(
        &mut accounts,
        MeteoraContentionAccountKind::ReserveX,
        derive_reserve_pda(lb_pair_pubkey, snapshot.mint_x())?,
    )?;
    push_unique_account(
        &mut accounts,
        MeteoraContentionAccountKind::ReserveY,
        derive_reserve_pda(lb_pair_pubkey, snapshot.mint_y())?,
    )?;
    push_unique_account(
        &mut accounts,
        MeteoraContentionAccountKind::Oracle,
        derive_oracle_pda(lb_pair_pubkey)?,
    )?;

    for index in &quote.touched_bin_arrays {
        let bin_array = snapshot
            .bin_array_by_index(*index)
            .map_err(|error| {
                format!(
                    "Meteora contention BinArray lookup failed: index={index} error={error:?}"
                )
            })?
            .ok_or_else(|| {
                format!(
                    "Meteora contention quote references an unhydrated BinArray: index={index}"
                )
            })?;

        push_unique_account(
            &mut accounts,
            MeteoraContentionAccountKind::QuoteTouchedBinArray { index: *index },
            bin_array.pubkey(),
        )?;
    }

    Ok(MeteoraContentionFootprint {
        lb_pair_pubkey,
        source: snapshot.source(),
        swap_for_y: quote.swap_for_y,
        requested_input_raw: quote.requested_input_raw,
        accounts,
    })
}

fn validate_quote_binding(
    snapshot: &MeteoraDlmmSnapshot,
    quote: &MeteoraM13ExactInputQuote,
) -> Result<(), String> {
    let source = snapshot.source();

    if quote.source_slot != source.source_slot {
        return Err(format!(
            "Meteora contention quote source-slot mismatch: quote={} snapshot={}",
            quote.source_slot, source.source_slot
        ));
    }

    if quote.generation_id != source.generation_id {
        return Err(format!(
            "Meteora contention quote generation mismatch: quote={} snapshot={}",
            quote.generation_id, source.generation_id
        ));
    }

    if quote.requested_input_raw == 0 {
        return Err("Meteora contention quote requested input must be greater than zero".to_owned());
    }

    if quote.consumed_input_raw != quote.requested_input_raw || quote.unspent_input_raw != 0 {
        return Err(format!(
            concat!(
                "Meteora contention requires an executable full-fill quote: ",
                "requested={} consumed={} unspent={}"
            ),
            quote.requested_input_raw, quote.consumed_input_raw, quote.unspent_input_raw
        ));
    }

    if quote.amount_out_raw == 0 {
        return Err("Meteora contention quote output must be greater than zero".to_owned());
    }

    let mint_x = bs58::encode(snapshot.mint_x()).into_string();
    let mint_y = bs58::encode(snapshot.mint_y()).into_string();
    let (expected_input, expected_output) = if quote.swap_for_y {
        (mint_x.as_str(), mint_y.as_str())
    } else {
        (mint_y.as_str(), mint_x.as_str())
    };

    if quote.input_mint != expected_input || quote.output_mint != expected_output {
        return Err(format!(
            concat!(
                "Meteora contention quote direction mismatch: swap_for_y={} ",
                "input={} expected_input={} output={} expected_output={}"
            ),
            quote.swap_for_y,
            quote.input_mint,
            expected_input,
            quote.output_mint,
            expected_output
        ));
    }

    if quote.touched_bin_arrays.is_empty() {
        return Err("Meteora contention quote touched no BinArrays".to_owned());
    }

    Ok(())
}

fn validate_touched_bin_arrays(indexes: &[i64]) -> Result<(), String> {
    for (position, index) in indexes.iter().enumerate() {
        if indexes[..position].contains(index) {
            return Err(format!(
                "Meteora contention quote contains duplicate BinArray index {index}"
            ));
        }
    }
    Ok(())
}

fn push_unique_account(
    accounts: &mut Vec<MeteoraContentionAccount>,
    kind: MeteoraContentionAccountKind,
    pubkey: [u8; 32],
) -> Result<(), String> {
    if let Some(existing) = accounts.iter().find(|account| account.pubkey == pubkey) {
        return Err(format!(
            "Meteora contention account collision: existing={:?} incoming={kind:?}",
            existing.kind
        ));
    }

    accounts.push(MeteoraContentionAccount { kind, pubkey });
    Ok(())
}

fn derive_reserve_pda(lb_pair: [u8; 32], mint: [u8; 32]) -> Result<[u8; 32], String> {
    Pubkey::try_find_program_address(&[&lb_pair, &mint], &METEORA_DLMM_PROGRAM_PUBKEY)
        .map(|(pubkey, _)| pubkey.to_bytes())
        .ok_or_else(|| "could not derive Meteora reserve PDA for contention footprint".to_owned())
}

fn derive_oracle_pda(lb_pair: [u8; 32]) -> Result<[u8; 32], String> {
    Pubkey::try_find_program_address(&[ORACLE_SEED, &lb_pair], &METEORA_DLMM_PROGRAM_PUBKEY)
        .map(|(pubkey, _)| pubkey.to_bytes())
        .ok_or_else(|| "could not derive Meteora Oracle PDA for contention footprint".to_owned())
}

fn derive_bitmap_extension(lb_pair: [u8; 32]) -> [u8; 32] {
    let (pubkey, _) = Pubkey::find_program_address(
        &[BIN_ARRAY_BITMAP_SEED, &lb_pair],
        &METEORA_DLMM_PROGRAM_PUBKEY,
    );
    pubkey.to_bytes()
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meteora::{
        derive_bin_array_pda, DlmmProtocolProfile, MeteoraBin, MeteoraBinArraySnapshotInput,
        MeteoraBinArrayState, MeteoraBitmapExtensionState, MeteoraClockSnapshot,
        MeteoraInternalBitmap, MeteoraLbPairState, BIN_ARRAY_VERSION_V3,
        INTERNAL_BITMAP_MIN_INDEX,
    };

    const LB_PAIR: [u8; 32] = [7_u8; 32];

    #[test]
    fn deterministic_subset_preserves_protocol_role_order() -> Result<(), String> {
        let source = source(100, 5);
        let snapshot = test_snapshot(&[0, 2, 3], false, source)?;
        let quote = exact_quote(&snapshot, true, vec![0, 2]);

        let footprint = meteora_quote_contention_footprint(&snapshot, &quote)?;

        assert_eq!(footprint.lb_pair_pubkey, LB_PAIR);
        assert_eq!(footprint.source, source);
        assert!(footprint.swap_for_y);
        assert_eq!(footprint.requested_input_raw, 100);
        assert_eq!(
            footprint
                .accounts
                .iter()
                .map(|account| account.kind)
                .collect::<Vec<_>>(),
            vec![
                MeteoraContentionAccountKind::LbPair,
                MeteoraContentionAccountKind::ReserveX,
                MeteoraContentionAccountKind::ReserveY,
                MeteoraContentionAccountKind::Oracle,
                MeteoraContentionAccountKind::QuoteTouchedBinArray { index: 0 },
                MeteoraContentionAccountKind::QuoteTouchedBinArray { index: 2 },
            ]
        );

        assert_eq!(
            footprint.accounts[1].pubkey,
            derive_reserve_pda(LB_PAIR, snapshot.mint_x())?
        );
        assert_eq!(
            footprint.accounts[2].pubkey,
            derive_reserve_pda(LB_PAIR, snapshot.mint_y())?
        );
        assert_eq!(footprint.accounts[3].pubkey, derive_oracle_pda(LB_PAIR)?);
        assert_eq!(
            footprint.accounts[4].pubkey,
            snapshot
                .bin_array_by_index(0)
                .map_err(|error| format!("{error:?}"))?
                .ok_or_else(|| "missing test BinArray 0".to_owned())?
                .pubkey()
        );
        assert_eq!(
            footprint.accounts[5].pubkey,
            snapshot
                .bin_array_by_index(2)
                .map_err(|error| format!("{error:?}"))?
                .ok_or_else(|| "missing test BinArray 2".to_owned())?
                .pubkey()
        );

        let unique = footprint.account_pubkeys();
        for (position, pubkey) in unique.iter().enumerate() {
            assert!(!unique[..position].contains(pubkey));
        }

        assert!(footprint
            .provenance()
            .contains("not the complete future transaction writable set"));
        Ok(())
    }

    #[test]
    fn present_bitmap_extension_is_included_in_canonical_position() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], true, source(101, 6))?;
        let quote = exact_quote(&snapshot, false, vec![0]);

        let footprint = meteora_quote_contention_footprint(&snapshot, &quote)?;

        assert_eq!(
            footprint.accounts[1],
            MeteoraContentionAccount {
                kind: MeteoraContentionAccountKind::BinArrayBitmapExtension,
                pubkey: derive_bitmap_extension(LB_PAIR),
            }
        );
        Ok(())
    }

    #[test]
    fn hydrated_but_untouched_bin_arrays_are_excluded() -> Result<(), String> {
        let snapshot = test_snapshot(&[0, 2, 3], false, source(102, 7))?;
        let quote = exact_quote(&snapshot, true, vec![0]);

        let footprint = meteora_quote_contention_footprint(&snapshot, &quote)?;

        assert!(footprint.accounts.iter().any(|account| {
            account.kind == MeteoraContentionAccountKind::QuoteTouchedBinArray { index: 0 }
        }));
        assert!(!footprint.accounts.iter().any(|account| {
            matches!(
                account.kind,
                MeteoraContentionAccountKind::QuoteTouchedBinArray { index } if index != 0
            )
        }));
        Ok(())
    }

    #[test]
    fn source_slot_mismatch_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(103, 8))?;
        let mut quote = exact_quote(&snapshot, true, vec![0]);
        quote.source_slot = 102;

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "stale source slot must fail",
        )?;
        assert!(error.contains("source-slot mismatch"));
        Ok(())
    }

    #[test]
    fn generation_mismatch_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(104, 9))?;
        let mut quote = exact_quote(&snapshot, true, vec![0]);
        quote.generation_id = 8;

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "stale generation must fail",
        )?;
        assert!(error.contains("generation mismatch"));
        Ok(())
    }

    #[test]
    fn quote_direction_mismatch_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(105, 10))?;
        let mut quote = exact_quote(&snapshot, true, vec![0]);
        quote.input_mint = bs58::encode(snapshot.mint_y()).into_string();

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "direction mismatch must fail",
        )?;
        assert!(error.contains("direction mismatch"));
        Ok(())
    }

    #[test]
    fn partial_fill_quote_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(106, 11))?;
        let mut quote = exact_quote(&snapshot, true, vec![0]);
        quote.consumed_input_raw = 90;
        quote.unspent_input_raw = 10;

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "partial quote must fail",
        )?;
        assert!(error.contains("full-fill"));
        Ok(())
    }

    #[test]
    fn duplicate_touched_bin_array_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(107, 12))?;
        let quote = exact_quote(&snapshot, true, vec![0, 0]);

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "duplicate BinArray must fail",
        )?;
        assert!(error.contains("duplicate BinArray"));
        Ok(())
    }

    #[test]
    fn unhydrated_touched_bin_array_fails_closed() -> Result<(), String> {
        let snapshot = test_snapshot(&[0], false, source(108, 13))?;
        let quote = exact_quote(&snapshot, true, vec![2]);

        let error = rejection(
            meteora_quote_contention_footprint(&snapshot, &quote),
            "unhydrated BinArray must fail",
        )?;
        assert!(error.contains("unhydrated BinArray"));
        Ok(())
    }

    fn rejection<T>(result: Result<T, String>, label: &str) -> Result<String, String> {
        match result {
            Ok(_) => Err(label.to_owned()),
            Err(error) => Ok(error),
        }
    }

    fn exact_quote(
        snapshot: &MeteoraDlmmSnapshot,
        swap_for_y: bool,
        touched_bin_arrays: Vec<i64>,
    ) -> MeteoraM13ExactInputQuote {
        let input_mint = if swap_for_y {
            bs58::encode(snapshot.mint_x()).into_string()
        } else {
            bs58::encode(snapshot.mint_y()).into_string()
        };
        let output_mint = if swap_for_y {
            bs58::encode(snapshot.mint_y()).into_string()
        } else {
            bs58::encode(snapshot.mint_x()).into_string()
        };
        let source = snapshot.source();

        MeteoraM13ExactInputQuote {
            input_mint,
            output_mint,
            requested_input_raw: 100,
            consumed_input_raw: 100,
            unspent_input_raw: 0,
            amount_out_raw: 99,
            trading_fee_raw: 1,
            protocol_fee_raw: 0,
            user_fee_raw: 1,
            fee_on_input: true,
            swap_for_y,
            touched_bin_arrays,
            source_slot: source.source_slot,
            generation_id: source.generation_id,
        }
    }

    fn test_snapshot(
        indexes: &[i64],
        with_extension: bool,
        source: MeteoraSnapshotSource,
    ) -> Result<MeteoraDlmmSnapshot, String> {
        let lb_pair = test_lb_pair();
        let mut bin_arrays = Vec::new();

        for index in indexes {
            let (pubkey, _) =
                derive_bin_array_pda(LB_PAIR, *index).map_err(|error| format!("{error:?}"))?;
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
        .map_err(|error| format!("{error:?}"))
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
