use crate::{orca, orca_live, ws_transport};
use futures_util::StreamExt;
use reqwest::Client;
use std::collections::BTreeMap;
use tokio::time::Duration;
use tokio_tungstenite::tungstenite::Message;

const MAX_ORCA_OBSERVATIONS: usize = 10;
const ORCA_OBSERVATION_TIMEOUT: Duration = Duration::from_secs(15);

pub async fn observe_and_prepare<S>(
    rpc_client: &Client,
    rpc_url: &str,
    reader: &mut S,
) -> Result<BTreeMap<String, orca_live::PreparedOrca>, String>
where
    S: StreamExt<Item = Result<Message, tokio_tungstenite::tungstenite::Error>> + Unpin,
{
    println!("\nVenue adapter: Orca Whirlpool");

    let mut prepared_by_pool = BTreeMap::new();
    let mut observed = 0usize;
    let mut anchor_candidates = 0usize;

    while observed < MAX_ORCA_OBSERVATIONS && prepared_by_pool.is_empty() {
        let Some(payload) =
            ws_transport::next_json_message_optional(reader, ORCA_OBSERVATION_TIMEOUT).await?
        else {
            break;
        };

        let observation = match orca::parse_program_notification(&payload) {
            Ok(Some(observation)) => observation,
            Ok(None) => continue,
            Err(error) => {
                println!("orca_observation_rejected: {error}");
                continue;
            }
        };

        observed += 1;

        println!(
            "orca_observation: pool={} slot={} {}",
            observation.pubkey,
            observation.slot,
            observation.pool_state.summary()
        );

        let Some((anchor_mint, intermediate_mint)) =
            orca_live::anchor_pair(&observation.pool_state)
        else {
            continue;
        };

        anchor_candidates += 1;

        let hydration_targets = orca::hydration_account_pubkeys(&observation);
        println!(
            "orca_base_hydration_targets: pool={} mint_a={} mint_b={}",
            hydration_targets[0], hydration_targets[1], hydration_targets[2]
        );

        if observation.pool_state.is_adaptive_fee() {
            println!(
                concat!(
                    "orca_preparation_rejected: pool={} reason=",
                    "adaptive-fee pool is not admitted by current production O2 preparation"
                ),
                observation.pubkey
            );
            continue;
        }

        match orca_live::prepare_orca(
            rpc_client,
            rpc_url,
            &observation,
            anchor_mint,
            intermediate_mint,
        )
        .await
        {
            Ok(prepared) => {
                println!(
                    "orca_production_ready: pool={} slot={} anchor={} intermediate={}",
                    prepared.normalized.pool_id,
                    prepared.normalized.source_slot,
                    prepared.anchor_mint,
                    prepared.intermediate_mint
                );

                prepared_by_pool.insert(prepared.normalized.pool_id.clone(), prepared);
            }
            Err(error) => {
                println!(
                    "orca_preparation_rejected: pool={} reason={error}",
                    observation.pubkey
                );
            }
        }
    }

    println!("orca_live_observation_count={observed}");
    println!("orca_live_anchor_candidate_count={anchor_candidates}");
    println!("orca_live_eligible_count={}", prepared_by_pool.len());

    if prepared_by_pool.is_empty() {
        println!("orca_production_admission_unavailable: no bounded O2-ready Orca pool observed");
    } else {
        println!("READ-ONLY ORCA PRODUCTION ADMISSION PASS");
    }

    Ok(prepared_by_pool)
}
