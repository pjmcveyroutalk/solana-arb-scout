use crate::rpc_transport;
use crate::runtime_quote::MeteoraRuntimeQuoteState;
use crate::ws_transport;
use futures_util::StreamExt;
use reqwest::Client;
use scout_cli::meteora::{
    bin_array_bitmap_bit, bin_id_to_bin_array_index, derive_bin_array_pda,
    MeteoraBitmapExtensionState, MeteoraDlmmSnapshot, MeteoraInternalBitmap, BIN_ARRAY_MAX_INDEX,
    BIN_ARRAY_MIN_INDEX, INTERNAL_BITMAP_MAX_INDEX, INTERNAL_BITMAP_MIN_INDEX,
};
use scout_cli::meteora_live::{
    meteora_base_hydration_account_pubkeys,
    meteora_program_subscribe_request as live_program_subscribe_request,
    meteora_quote_hydration_account_pubkeys, parse_meteora_base_hydration_response,
    parse_meteora_program_notification, parse_meteora_quote_hydration_response,
    MeteoraLiveObservation,
};
use scout_cli::meteora_m13::MeteoraM13PreparedQuote;
use serde_json::{json, Value};
use std::collections::{BTreeMap, BTreeSet};
use std::time::{Duration, Instant};
use tokio_tungstenite::tungstenite::Message;

const MAX_BIN_ARRAYS_PER_DIRECTION: usize = 3;
const BASE_HYDRATION_REQUEST_ID: u64 = 61;
const QUOTE_HYDRATION_REQUEST_ID: u64 = 62;

pub fn program_subscribe_request() -> Value {
    live_program_subscribe_request()
}

pub async fn observe_and_prepare<S>(
    rpc_client: &Client,
    rpc_url: &str,
    reader: &mut S,
    max_observations: usize,
    observation_timeout: Duration,
) -> Result<BTreeMap<String, MeteoraRuntimeQuoteState>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    if max_observations == 0 {
        return Err("Meteora runtime observation bound must be greater than zero".to_owned());
    }
    if observation_timeout.is_zero() {
        return Err("Meteora runtime observation timeout must be greater than zero".to_owned());
    }

    println!("\nVenue adapter: Meteora DLMM");

    let mut prepared = BTreeMap::new();
    let mut observed = 0usize;
    let mut next_generation_id = 1u64;
    let observation_window_started = Instant::now();

    while observed < max_observations {
        let remaining = observation_timeout.saturating_sub(observation_window_started.elapsed());
        if remaining.is_zero() {
            break;
        }

        let Some(payload) = ws_transport::next_json_message_optional(reader, remaining).await?
        else {
            break;
        };
        let trigger_received_at_unix_ms = crate::unix_time_ms_now()?;

        let observation = match parse_meteora_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("meteora_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "meteora_observation: pool={} slot={} version={}",
            observation.pubkey, observation.slot, observation.admission.version
        );

        let base_generation_id = next_generation_id;
        let quote_generation_id = base_generation_id
            .checked_add(1)
            .ok_or_else(|| "Meteora runtime generation id overflow".to_owned())?;
        next_generation_id = quote_generation_id
            .checked_add(1)
            .ok_or_else(|| "Meteora runtime generation id overflow".to_owned())?;

        match hydrate_observation(
            rpc_client,
            rpc_url,
            &observation,
            trigger_received_at_unix_ms,
            base_generation_id,
            quote_generation_id,
        )
        .await
        {
            Ok(runtime) => {
                let pool_id = runtime.normalized.pool_id.clone();
                let source_slot = runtime.normalized.source_slot;

                let replace = should_replace_runtime_state(
                    source_slot,
                    prepared
                        .get(&pool_id)
                        .map(|existing: &MeteoraRuntimeQuoteState| existing.normalized.source_slot),
                );

                if replace {
                    println!(
                        "meteora_runtime_prepared: pool={} source_slot={} generation_id={}",
                        pool_id,
                        source_slot,
                        runtime.snapshot.source().generation_id
                    );
                    prepared.insert(pool_id, runtime);
                }
            }
            Err(error) => {
                println!(
                    "meteora_hydration_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("meteora_live_observation_count={observed}");
    println!("meteora_live_eligible_count={}", prepared.len());
    println!(
        "meteora_live_observation_elapsed_ms={}",
        observation_window_started.elapsed().as_millis()
    );

    if prepared.is_empty() {
        println!(concat!(
            "meteora_production_admission_unavailable: ",
            "no bounded runtime-ready Meteora pool observed"
        ));
    } else {
        println!("READ-ONLY METEORA PRODUCTION ADMISSION PASS");
    }

    Ok(prepared)
}

async fn hydrate_observation(
    rpc_client: &Client,
    rpc_url: &str,
    observation: &MeteoraLiveObservation,
    trigger_received_at_unix_ms: u64,
    base_generation_id: u64,
    quote_generation_id: u64,
) -> Result<MeteoraRuntimeQuoteState, String> {
    let base_pubkeys = meteora_base_hydration_account_pubkeys(observation)?;
    let base_payload = fetch_multiple_accounts(
        rpc_client,
        rpc_url,
        BASE_HYDRATION_REQUEST_ID,
        &base_pubkeys,
        observation.slot,
        "Meteora runtime base hydration",
    )
    .await?;

    let base_hydrated_at_unix_ms = crate::unix_time_ms_now()?;
    let base = parse_meteora_base_hydration_response(
        observation,
        &base_payload,
        base_generation_id,
        trigger_received_at_unix_ms,
        base_hydrated_at_unix_ms,
    )?;

    let base_source_slot = base.snapshot.source().source_slot;
    let bin_array_pubkeys = certified_directional_bin_array_pubkeys(&base.snapshot)?;
    let quote_pubkeys = meteora_quote_hydration_account_pubkeys(observation, &bin_array_pubkeys)?;
    let quote_payload = fetch_multiple_accounts(
        rpc_client,
        rpc_url,
        QUOTE_HYDRATION_REQUEST_ID,
        &quote_pubkeys,
        base_source_slot,
        "Meteora runtime quote hydration",
    )
    .await?;

    let quote_hydrated_at_unix_ms = crate::unix_time_ms_now()?;
    let prepared = parse_meteora_quote_hydration_response(
        observation,
        &bin_array_pubkeys,
        &quote_payload,
        quote_generation_id,
        trigger_received_at_unix_ms,
        quote_hydrated_at_unix_ms,
    )?;

    let quote_plan = certified_directional_bin_array_pubkeys(&prepared.snapshot)?;
    if quote_plan != bin_array_pubkeys {
        return Err(format!(
            concat!(
                "Meteora runtime BinArray plan changed between base and quote hydration: ",
                "base={:?} quote={:?}"
            ),
            bin_array_pubkeys, quote_plan
        ));
    }

    let snapshot_source_slot = prepared.snapshot.source().source_slot;
    if prepared.normalized.source_slot != snapshot_source_slot {
        return Err(format!(
            concat!(
                "Meteora runtime normalized/snapshot source-slot mismatch: ",
                "normalized={} snapshot={}"
            ),
            prepared.normalized.source_slot, snapshot_source_slot
        ));
    }

    MeteoraM13PreparedQuote::from_snapshot(&prepared.normalized, &prepared.snapshot)?;

    Ok(MeteoraRuntimeQuoteState {
        normalized: prepared.normalized,
        snapshot: prepared.snapshot,
    })
}

fn certified_directional_bin_array_pubkeys(
    snapshot: &MeteoraDlmmSnapshot,
) -> Result<Vec<String>, String> {
    certified_directional_bin_array_pubkeys_from_parts(
        snapshot.lb_pair_pubkey(),
        snapshot.lb_pair().active_id,
        &snapshot.lb_pair().bin_array_bitmap,
        snapshot.bitmap_extension(),
    )
}

fn certified_directional_bin_array_pubkeys_from_parts(
    lb_pair_pubkey: [u8; 32],
    active_id: i32,
    bin_array_bitmap: &MeteoraInternalBitmap,
    bitmap_extension: Option<&MeteoraBitmapExtensionState>,
) -> Result<Vec<String>, String> {
    let start_index = bin_id_to_bin_array_index(active_id)
        .map_err(|error| format!("Meteora runtime active BinArray index failed: {error:?}"))?;
    let mut ordered = Vec::new();
    let mut seen = BTreeSet::new();

    for swap_for_y in [true, false] {
        let terminal = if swap_for_y {
            BIN_ARRAY_MIN_INDEX
        } else {
            BIN_ARRAY_MAX_INDEX
        };
        let step = if swap_for_y { -1_i64 } else { 1_i64 };
        let mut index = start_index;
        let mut taken = 0usize;

        while taken < MAX_BIN_ARRAYS_PER_DIRECTION {
            let outside_internal =
                !(INTERNAL_BITMAP_MIN_INDEX..=INTERNAL_BITMAP_MAX_INDEX).contains(&index);

            if outside_internal && bitmap_extension.is_none() {
                break;
            }

            let initialized = bin_array_bitmap_bit(bin_array_bitmap, bitmap_extension, index)
                .map_err(|error| {
                    format!(
                        concat!(
                            "Meteora runtime certified BinArray search failed: ",
                            "direction={} index={} error={:?}"
                        ),
                        direction_label(swap_for_y),
                        index,
                        error
                    )
                })?;

            if initialized {
                let (pubkey, _) = derive_bin_array_pda(lb_pair_pubkey, index).map_err(|error| {
                    format!("Meteora runtime BinArray PDA derivation failed: {error:?}")
                })?;
                let encoded = bs58::encode(pubkey).into_string();

                if seen.insert(encoded.clone()) {
                    ordered.push(encoded);
                }

                taken = taken
                    .checked_add(1)
                    .ok_or_else(|| "Meteora runtime BinArray take counter overflow".to_owned())?;
            }

            if index == terminal {
                break;
            }

            index = index.checked_add(step).ok_or_else(|| {
                format!(
                    concat!(
                        "Meteora runtime directional BinArray index overflow: ",
                        "direction={} index={}"
                    ),
                    direction_label(swap_for_y),
                    index
                )
            })?;
        }
    }

    if ordered.is_empty() {
        return Err("Meteora runtime pool has no initialized BinArrays".to_owned());
    }

    Ok(ordered)
}

async fn fetch_multiple_accounts(
    rpc_client: &Client,
    rpc_url: &str,
    request_id: u64,
    pubkeys: &[String],
    min_context_slot: u64,
    label: &str,
) -> Result<Value, String> {
    if pubkeys.is_empty() {
        return Err(format!("{label} account list must not be empty"));
    }

    let request = json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getMultipleAccounts",
        "params": [
            pubkeys,
            {
                "commitment": "processed",
                "encoding": "base64",
                "minContextSlot": min_context_slot
            }
        ]
    });

    let started_at = Instant::now();

    println!(
        "rpc_request_start: label={} id={} method=getMultipleAccounts account_count={}",
        label,
        request_id,
        pubkeys.len()
    );

    let payload = rpc_transport::post_json(rpc_client, rpc_url, &request, label)
        .await
        .map_err(|error| {
            format!(
                "{label} RPC request failed after {} ms: {error}",
                started_at.elapsed().as_millis()
            )
        })?;

    println!(
        "rpc_request_finish: label={} id={} elapsed_ms={} rpc_error={}",
        label,
        request_id,
        started_at.elapsed().as_millis(),
        payload.get("error").is_some()
    );

    Ok(payload)
}

fn should_replace_runtime_state(source_slot: u64, existing_source_slot: Option<u64>) -> bool {
    match existing_source_slot {
        Some(existing_source_slot) => source_slot >= existing_source_slot,
        None => true,
    }
}

fn direction_label(swap_for_y: bool) -> &'static str {
    if swap_for_y {
        "x_to_y"
    } else {
        "y_to_x"
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use scout_cli::meteora::{
        DlmmProtocolProfile, MeteoraClockSnapshot, MeteoraDlmmFailure, MeteoraLbPairState,
        MeteoraSnapshotSource, MAX_BIN_PER_ARRAY,
    };
    use scout_core::{
        NormalizedPoolState, NormalizedToken, PoolTradingState, QuoteReserveState, Venue,
    };

    const LB_PAIR: [u8; 32] = [7_u8; 32];

    #[test]
    fn runtime_subscription_reuses_certified_meteora_live_request() {
        let request = program_subscribe_request();

        assert_eq!(
            request.get("method").and_then(Value::as_str),
            Some("programSubscribe")
        );
        assert_eq!(request.get("id").and_then(Value::as_u64), Some(60));
        assert_eq!(
            request.pointer("/params/0").and_then(Value::as_str),
            Some(scout_cli::meteora::METEORA_DLMM_PROGRAM_ID)
        );
    }

    #[test]
    fn certified_selector_collects_three_initialized_arrays_per_direction(
    ) -> Result<(), MeteoraDlmmFailure> {
        let mut lb_pair = test_lb_pair(0);
        for index in [-3_i64, -2, -1, 0, 1, 2, 3] {
            set_internal_bit(&mut lb_pair.bin_array_bitmap, index);
        }
        let snapshot = test_snapshot(lb_pair, None)?;

        let selected = certified_directional_bin_array_pubkeys(&snapshot)
            .map_err(|_| MeteoraDlmmFailure::InvalidLayout)?;
        let expected_indexes = [0_i64, -1, -2, 1, 2];
        let expected = expected_indexes
            .into_iter()
            .map(|index| {
                derive_bin_array_pda(LB_PAIR, index)
                    .map(|(pubkey, _)| bs58::encode(pubkey).into_string())
            })
            .collect::<Result<Vec<_>, _>>()?;

        assert_eq!(selected, expected);
        Ok(())
    }

    #[test]
    fn certified_selector_stops_cleanly_without_bitmap_extension() -> Result<(), MeteoraDlmmFailure>
    {
        let mut lb_pair = test_lb_pair(511 * MAX_BIN_PER_ARRAY);
        set_internal_bit(&mut lb_pair.bin_array_bitmap, 511);
        let snapshot = test_snapshot(lb_pair, None)?;

        let selected = certified_directional_bin_array_pubkeys(&snapshot)
            .map_err(|_| MeteoraDlmmFailure::InvalidLayout)?;
        let (pubkey, _) = derive_bin_array_pda(LB_PAIR, 511)?;

        assert_eq!(selected, vec![bs58::encode(pubkey).into_string()]);
        Ok(())
    }

    #[test]
    fn newer_source_slot_replaces_existing_runtime_state() {
        assert!(should_replace_runtime_state(101, Some(100)));
        assert!(should_replace_runtime_state(100, Some(100)));
        assert!(!should_replace_runtime_state(99, Some(100)));
        assert!(should_replace_runtime_state(1, None));
    }

    #[test]
    fn prepared_meteora_runtime_state_produces_quote_readiness() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(test_lb_pair(0), None)?;
        let normalized = test_normalized_pool(&snapshot);
        let mut meteora_runtime = BTreeMap::new();
        meteora_runtime.insert(
            normalized.pool_id.clone(),
            MeteoraRuntimeQuoteState {
                normalized: test_normalized_pool(&snapshot),
                snapshot,
            },
        );

        let readiness = crate::runtime_quote::readiness_for_pool_with_meteora(
            &normalized,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &meteora_runtime,
        );

        assert!(readiness.is_some());
        Ok(())
    }

    #[test]
    fn missing_meteora_runtime_state_fails_readiness_closed() -> Result<(), MeteoraDlmmFailure> {
        let snapshot = test_snapshot(test_lb_pair(0), None)?;
        let normalized = test_normalized_pool(&snapshot);

        let readiness = crate::runtime_quote::readiness_for_pool_with_meteora(
            &normalized,
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
            &BTreeMap::new(),
        );

        assert!(readiness.is_none());
        Ok(())
    }

    fn test_snapshot(
        lb_pair: MeteoraLbPairState,
        extension: Option<MeteoraBitmapExtensionState>,
    ) -> Result<MeteoraDlmmSnapshot, MeteoraDlmmFailure> {
        MeteoraDlmmSnapshot::new(
            LB_PAIR,
            lb_pair,
            Vec::new(),
            extension,
            MeteoraClockSnapshot {
                slot: 100,
                epoch_start_timestamp: 0,
                epoch: 0,
                leader_schedule_epoch: 0,
                unix_timestamp: 0,
            },
            DlmmProtocolProfile::V0_12,
            MeteoraSnapshotSource {
                source_slot: 100,
                generation_id: 1,
            },
        )
    }

    fn test_normalized_pool(snapshot: &MeteoraDlmmSnapshot) -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: bs58::encode(snapshot.lb_pair_pubkey()).into_string(),
            venue: Venue::Meteora,
            program_id: scout_cli::meteora::METEORA_DLMM_PROGRAM_ID.to_owned(),
            source_slot: snapshot.source().source_slot,
            token_a: NormalizedToken {
                mint: bs58::encode(snapshot.mint_x()).into_string(),
                vault: bs58::encode([3_u8; 32]).into_string(),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: bs58::encode(snapshot.mint_y()).into_string(),
                vault: bs58::encode([4_u8; 32]).into_string(),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Unavailable,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
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

    fn set_internal_bit(bitmap: &mut [u64; 16], index: i64) {
        let offset = index - INTERNAL_BITMAP_MIN_INDEX;
        let word_index = (offset / 64) as usize;
        let bit_index = (offset % 64) as u32;
        bitmap[word_index] |= 1_u64 << bit_index;
    }
}
