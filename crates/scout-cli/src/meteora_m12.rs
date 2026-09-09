use crate::meteora::{MeteoraDlmmFailure, MeteoraSnapshotSource};
use crate::meteora_m10::serialize_swap2_empty_remaining_accounts;
use crate::meteora_m11::{
    MeteoraExecutionAccountKind, MeteoraExecutionAccountPlan, MeteoraPlannedAccount,
};
use base64::{engine::general_purpose::STANDARD as BASE64_STANDARD, Engine as _};
use serde_json::{json, Value};

const VERSIONED_MESSAGE_V0_PREFIX: u8 = 0x80;
const SIGNATURE_BYTES: usize = 64;
pub const SOLANA_MAX_SERIALIZED_TRANSACTION_BYTES: usize = 1_232;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraValidatedAddressLookupTable {
    pub account_key: [u8; 32],
    pub addresses: Vec<[u8; 32]>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraV0AddressTableLookup {
    pub account_key: [u8; 32],
    pub writable_indexes: Vec<u8>,
    pub readonly_indexes: Vec<u8>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraCompiledV0Swap2 {
    pub source: MeteoraSnapshotSource,
    pub static_account_keys: Vec<[u8; 32]>,
    pub loaded_writable_accounts: Vec<[u8; 32]>,
    pub loaded_readonly_accounts: Vec<[u8; 32]>,
    pub address_table_lookup: Option<MeteoraV0AddressTableLookup>,
    pub program_id_index: u8,
    pub instruction_account_indices: Vec<u8>,
    pub message_bytes: Vec<u8>,
    pub simulation_transaction_bytes: Vec<u8>,
}

impl MeteoraCompiledV0Swap2 {
    pub fn message_len(&self) -> usize {
        self.message_bytes.len()
    }

    pub fn transaction_len(&self) -> usize {
        self.simulation_transaction_bytes.len()
    }

    pub fn simulation_transaction_base64(&self) -> String {
        BASE64_STANDARD.encode(&self.simulation_transaction_bytes)
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraSimulationOutcome {
    pub units_consumed: Option<u64>,
    pub logs: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct UniqueAccount {
    pubkey: [u8; 32],
    is_signer: bool,
    is_writable: bool,
    is_invoked: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct LookupApplication {
    static_accounts: Vec<UniqueAccount>,
    loaded_writable: Vec<UniqueAccount>,
    loaded_readonly: Vec<UniqueAccount>,
    lookup: Option<MeteoraV0AddressTableLookup>,
}

pub fn compile_meteora_swap2_v0(
    plan: &MeteoraExecutionAccountPlan,
    amount_in: u64,
    min_amount_out: u64,
    recent_blockhash: [u8; 32],
    lookup_table: Option<&MeteoraValidatedAddressLookupTable>,
) -> Result<MeteoraCompiledV0Swap2, MeteoraDlmmFailure> {
    validate_m11_plan_shape(plan)?;

    let ordered_accounts = plan.ordered_accounts();
    let canonical_unique_accounts = canonical_unique_accounts(&ordered_accounts)?;
    let lookup_application = apply_optional_lookup_table(&canonical_unique_accounts, lookup_table)?;
    let combined_account_keys = combined_account_keys(
        &lookup_application.static_accounts,
        &lookup_application.loaded_writable,
        &lookup_application.loaded_readonly,
    );
    if combined_account_keys.len() > usize::from(u8::MAX) + 1 {
        return Err(MeteoraDlmmFailure::AccountFootprintExceeded);
    }

    let program_pubkey = plan
        .fixed_accounts
        .iter()
        .find(|account| matches!(account.kind, MeteoraExecutionAccountKind::Program))
        .map(|account| account.pubkey)
        .ok_or(MeteoraDlmmFailure::InvalidLayout)?;
    let program_id_index = account_index(&combined_account_keys, program_pubkey)?;

    let mut instruction_account_indices = Vec::with_capacity(ordered_accounts.len());
    for account in &ordered_accounts {
        instruction_account_indices.push(account_index(&combined_account_keys, account.pubkey)?);
    }

    let required_signatures = lookup_application
        .static_accounts
        .iter()
        .filter(|account| account.is_signer)
        .count();
    let readonly_signed = lookup_application
        .static_accounts
        .iter()
        .filter(|account| account.is_signer && !account.is_writable)
        .count();
    let readonly_unsigned = lookup_application
        .static_accounts
        .iter()
        .filter(|account| !account.is_signer && !account.is_writable)
        .count();

    let required_signatures = u8::try_from(required_signatures)
        .map_err(|_| MeteoraDlmmFailure::AccountFootprintExceeded)?;
    let readonly_signed =
        u8::try_from(readonly_signed).map_err(|_| MeteoraDlmmFailure::AccountFootprintExceeded)?;
    let readonly_unsigned = u8::try_from(readonly_unsigned)
        .map_err(|_| MeteoraDlmmFailure::AccountFootprintExceeded)?;

    let instruction_data = serialize_swap2_empty_remaining_accounts(amount_in, min_amount_out);
    let message_bytes = serialize_v0_message(
        required_signatures,
        readonly_signed,
        readonly_unsigned,
        &lookup_application.static_accounts,
        recent_blockhash,
        program_id_index,
        &instruction_account_indices,
        &instruction_data,
        lookup_application.lookup.as_ref(),
    );

    let simulation_transaction_bytes =
        serialize_unsigned_simulation_transaction(required_signatures, &message_bytes)?;
    if simulation_transaction_bytes.len() > SOLANA_MAX_SERIALIZED_TRANSACTION_BYTES {
        return Err(MeteoraDlmmFailure::AccountFootprintExceeded);
    }

    Ok(MeteoraCompiledV0Swap2 {
        source: plan.source,
        static_account_keys: lookup_application
            .static_accounts
            .iter()
            .map(|account| account.pubkey)
            .collect(),
        loaded_writable_accounts: lookup_application
            .loaded_writable
            .iter()
            .map(|account| account.pubkey)
            .collect(),
        loaded_readonly_accounts: lookup_application
            .loaded_readonly
            .iter()
            .map(|account| account.pubkey)
            .collect(),
        address_table_lookup: lookup_application.lookup,
        program_id_index,
        instruction_account_indices,
        message_bytes,
        simulation_transaction_bytes,
    })
}

pub fn meteora_latest_blockhash_request(request_id: u64, min_context_slot: u64) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "getLatestBlockhash",
        "params": [
            {
                "commitment": "processed",
                "minContextSlot": min_context_slot
            }
        ]
    })
}

pub fn parse_meteora_latest_blockhash_response(
    payload: &Value,
) -> Result<[u8; 32], MeteoraDlmmFailure> {
    if payload.get("error").is_some() {
        return Err(MeteoraDlmmFailure::SimulationFailed);
    }

    let encoded = payload
        .get("result")
        .and_then(|result| result.get("value"))
        .and_then(|value| value.get("blockhash"))
        .and_then(Value::as_str)
        .ok_or(MeteoraDlmmFailure::SimulationFailed)?;
    let decoded = bs58::decode(encoded)
        .into_vec()
        .map_err(|_| MeteoraDlmmFailure::SimulationFailed)?;
    if decoded.len() != 32 {
        return Err(MeteoraDlmmFailure::SimulationFailed);
    }

    let mut blockhash = [0_u8; 32];
    blockhash.copy_from_slice(&decoded);
    Ok(blockhash)
}

pub fn meteora_simulate_transaction_request(
    request_id: u64,
    compiled: &MeteoraCompiledV0Swap2,
) -> Value {
    json!({
        "jsonrpc": "2.0",
        "id": request_id,
        "method": "simulateTransaction",
        "params": [
            compiled.simulation_transaction_base64(),
            {
                "commitment": "processed",
                "encoding": "base64",
                "sigVerify": false,
                "replaceRecentBlockhash": false,
                "minContextSlot": compiled.source.source_slot
            }
        ]
    })
}

pub fn parse_meteora_simulation_response(
    payload: &Value,
) -> Result<MeteoraSimulationOutcome, MeteoraDlmmFailure> {
    if payload.get("error").is_some() {
        return Err(MeteoraDlmmFailure::SimulationFailed);
    }

    let value = payload
        .get("result")
        .and_then(|result| result.get("value"))
        .ok_or(MeteoraDlmmFailure::SimulationFailed)?;
    if value.get("err").is_some_and(|error| !error.is_null()) {
        return Err(MeteoraDlmmFailure::SimulationFailed);
    }

    let units_consumed = match value.get("unitsConsumed") {
        Some(units) if !units.is_null() => {
            Some(units.as_u64().ok_or(MeteoraDlmmFailure::SimulationFailed)?)
        }
        _ => None,
    };

    let logs = match value.get("logs") {
        Some(Value::Array(entries)) => {
            let mut logs = Vec::with_capacity(entries.len());
            for entry in entries {
                let log = entry.as_str().ok_or(MeteoraDlmmFailure::SimulationFailed)?;
                logs.push(log.to_owned());
            }
            logs
        }
        Some(Value::Null) | None => Vec::new(),
        Some(_) => return Err(MeteoraDlmmFailure::SimulationFailed),
    };

    Ok(MeteoraSimulationOutcome {
        units_consumed,
        logs,
    })
}

fn validate_m11_plan_shape(plan: &MeteoraExecutionAccountPlan) -> Result<(), MeteoraDlmmFailure> {
    if plan.fixed_accounts.len() != 16 {
        return Err(MeteoraDlmmFailure::InvalidLayout);
    }

    for (position, account) in plan.fixed_accounts.iter().enumerate() {
        if !fixed_account_kind_matches(position, account.kind)
            || !fixed_account_flags_match(position, *account)
        {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }

    for account in &plan.remaining_accounts {
        if !matches!(
            account.kind,
            MeteoraExecutionAccountKind::BinArrayQuoteRequired { .. }
                | MeteoraExecutionAccountKind::BinArrayExecutionBuffer { .. }
        ) || account.is_signer
            || !account.is_writable
        {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }

    if plan.ordered_accounts().len() > usize::from(u8::MAX) + 1 {
        return Err(MeteoraDlmmFailure::AccountFootprintExceeded);
    }

    Ok(())
}

fn fixed_account_kind_matches(position: usize, kind: MeteoraExecutionAccountKind) -> bool {
    match position {
        0 => matches!(kind, MeteoraExecutionAccountKind::LbPair),
        1 => matches!(kind, MeteoraExecutionAccountKind::BinArrayBitmapExtension),
        2 => matches!(kind, MeteoraExecutionAccountKind::ReserveX),
        3 => matches!(kind, MeteoraExecutionAccountKind::ReserveY),
        4 => matches!(kind, MeteoraExecutionAccountKind::TokenXMint),
        5 => matches!(kind, MeteoraExecutionAccountKind::TokenYMint),
        6 => matches!(kind, MeteoraExecutionAccountKind::TokenXProgram),
        7 => matches!(kind, MeteoraExecutionAccountKind::TokenYProgram),
        8 => matches!(kind, MeteoraExecutionAccountKind::User),
        9 => matches!(kind, MeteoraExecutionAccountKind::UserTokenIn),
        10 => matches!(kind, MeteoraExecutionAccountKind::UserTokenOut),
        11 => matches!(kind, MeteoraExecutionAccountKind::Oracle),
        12 => matches!(kind, MeteoraExecutionAccountKind::HostFeeIn),
        13 => matches!(kind, MeteoraExecutionAccountKind::EventAuthority),
        14 => matches!(kind, MeteoraExecutionAccountKind::Program),
        15 => matches!(kind, MeteoraExecutionAccountKind::MemoProgram),
        _ => false,
    }
}

fn fixed_account_flags_match(position: usize, account: MeteoraPlannedAccount) -> bool {
    match position {
        0 | 1 | 2 | 3 | 9 | 10 | 11 | 12 => !account.is_signer && account.is_writable,
        4 | 5 | 6 | 7 | 13 | 14 | 15 => !account.is_signer && !account.is_writable,
        8 => account.is_signer && !account.is_writable,
        _ => false,
    }
}

fn canonical_unique_accounts(
    ordered_accounts: &[MeteoraPlannedAccount],
) -> Result<Vec<UniqueAccount>, MeteoraDlmmFailure> {
    let mut unique = Vec::<UniqueAccount>::new();

    for account in ordered_accounts {
        let is_invoked = matches!(account.kind, MeteoraExecutionAccountKind::Program);
        let is_fee_payer = matches!(account.kind, MeteoraExecutionAccountKind::User);
        if let Some(existing) = unique
            .iter_mut()
            .find(|existing| existing.pubkey == account.pubkey)
        {
            existing.is_signer |= account.is_signer || is_fee_payer;
            existing.is_writable |= account.is_writable || is_fee_payer;
            existing.is_invoked |= is_invoked;
        } else {
            unique.push(UniqueAccount {
                pubkey: account.pubkey,
                is_signer: account.is_signer || is_fee_payer,
                is_writable: account.is_writable || is_fee_payer,
                is_invoked,
            });
        }
    }

    if unique.len() > usize::from(u8::MAX) + 1 {
        return Err(MeteoraDlmmFailure::AccountFootprintExceeded);
    }

    let mut canonical = Vec::with_capacity(unique.len());
    append_account_class(&unique, &mut canonical, true, true);
    append_account_class(&unique, &mut canonical, true, false);
    append_account_class(&unique, &mut canonical, false, true);
    append_account_class(&unique, &mut canonical, false, false);
    Ok(canonical)
}

fn append_account_class(
    unique: &[UniqueAccount],
    canonical: &mut Vec<UniqueAccount>,
    is_signer: bool,
    is_writable: bool,
) {
    canonical.extend(
        unique
            .iter()
            .copied()
            .filter(|account| account.is_signer == is_signer && account.is_writable == is_writable),
    );
}

fn apply_optional_lookup_table(
    canonical_accounts: &[UniqueAccount],
    lookup_table: Option<&MeteoraValidatedAddressLookupTable>,
) -> Result<LookupApplication, MeteoraDlmmFailure> {
    let Some(table) = lookup_table else {
        return Ok(LookupApplication {
            static_accounts: canonical_accounts.to_vec(),
            loaded_writable: Vec::new(),
            loaded_readonly: Vec::new(),
            lookup: None,
        });
    };

    validate_lookup_table(table)?;

    let mut static_accounts = Vec::with_capacity(canonical_accounts.len());
    let mut loaded_writable = Vec::new();
    let mut loaded_readonly = Vec::new();
    let mut writable_indexes = Vec::new();
    let mut readonly_indexes = Vec::new();

    for account in canonical_accounts {
        if account.is_signer || account.is_invoked {
            static_accounts.push(*account);
            continue;
        }

        let Some(table_position) = table
            .addresses
            .iter()
            .position(|address| *address == account.pubkey)
        else {
            static_accounts.push(*account);
            continue;
        };
        let table_index = u8::try_from(table_position)
            .map_err(|_| MeteoraDlmmFailure::AccountFootprintExceeded)?;

        if account.is_writable {
            loaded_writable.push(*account);
            writable_indexes.push(table_index);
        } else {
            loaded_readonly.push(*account);
            readonly_indexes.push(table_index);
        }
    }

    let lookup = if writable_indexes.is_empty() && readonly_indexes.is_empty() {
        None
    } else {
        Some(MeteoraV0AddressTableLookup {
            account_key: table.account_key,
            writable_indexes,
            readonly_indexes,
        })
    };

    Ok(LookupApplication {
        static_accounts,
        loaded_writable,
        loaded_readonly,
        lookup,
    })
}

fn validate_lookup_table(
    table: &MeteoraValidatedAddressLookupTable,
) -> Result<(), MeteoraDlmmFailure> {
    if table.addresses.len() > usize::from(u8::MAX) + 1 {
        return Err(MeteoraDlmmFailure::AccountFootprintExceeded);
    }

    for (position, address) in table.addresses.iter().enumerate() {
        if table.addresses[..position].contains(address) {
            return Err(MeteoraDlmmFailure::InvalidLayout);
        }
    }

    Ok(())
}

fn combined_account_keys(
    static_accounts: &[UniqueAccount],
    loaded_writable: &[UniqueAccount],
    loaded_readonly: &[UniqueAccount],
) -> Vec<[u8; 32]> {
    static_accounts
        .iter()
        .chain(loaded_writable.iter())
        .chain(loaded_readonly.iter())
        .map(|account| account.pubkey)
        .collect()
}

fn account_index(
    combined_account_keys: &[[u8; 32]],
    pubkey: [u8; 32],
) -> Result<u8, MeteoraDlmmFailure> {
    let position = combined_account_keys
        .iter()
        .position(|candidate| *candidate == pubkey)
        .ok_or(MeteoraDlmmFailure::InvalidLayout)?;

    u8::try_from(position).map_err(|_| MeteoraDlmmFailure::AccountFootprintExceeded)
}

#[allow(clippy::too_many_arguments)]
fn serialize_v0_message(
    required_signatures: u8,
    readonly_signed: u8,
    readonly_unsigned: u8,
    static_accounts: &[UniqueAccount],
    recent_blockhash: [u8; 32],
    program_id_index: u8,
    instruction_account_indices: &[u8],
    instruction_data: &[u8],
    lookup: Option<&MeteoraV0AddressTableLookup>,
) -> Vec<u8> {
    let mut message = Vec::new();
    message.push(VERSIONED_MESSAGE_V0_PREFIX);
    message.push(required_signatures);
    message.push(readonly_signed);
    message.push(readonly_unsigned);

    append_shortvec(&mut message, static_accounts.len());
    for account in static_accounts {
        message.extend_from_slice(&account.pubkey);
    }

    message.extend_from_slice(&recent_blockhash);

    append_shortvec(&mut message, 1);
    message.push(program_id_index);
    append_shortvec(&mut message, instruction_account_indices.len());
    message.extend_from_slice(instruction_account_indices);
    append_shortvec(&mut message, instruction_data.len());
    message.extend_from_slice(instruction_data);

    match lookup {
        Some(lookup) => {
            append_shortvec(&mut message, 1);
            message.extend_from_slice(&lookup.account_key);
            append_shortvec(&mut message, lookup.writable_indexes.len());
            message.extend_from_slice(&lookup.writable_indexes);
            append_shortvec(&mut message, lookup.readonly_indexes.len());
            message.extend_from_slice(&lookup.readonly_indexes);
        }
        None => append_shortvec(&mut message, 0),
    }

    message
}

fn serialize_unsigned_simulation_transaction(
    required_signatures: u8,
    message_bytes: &[u8],
) -> Result<Vec<u8>, MeteoraDlmmFailure> {
    let signature_count = usize::from(required_signatures);
    let signature_bytes = signature_count
        .checked_mul(SIGNATURE_BYTES)
        .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?;

    let mut transaction = Vec::with_capacity(
        message_bytes
            .len()
            .checked_add(signature_bytes)
            .and_then(|len| len.checked_add(3))
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?,
    );
    append_shortvec(&mut transaction, signature_count);
    transaction.resize(
        transaction
            .len()
            .checked_add(signature_bytes)
            .ok_or(MeteoraDlmmFailure::ArithmeticOverflow)?,
        0,
    );
    transaction.extend_from_slice(message_bytes);
    Ok(transaction)
}

fn append_shortvec(output: &mut Vec<u8>, mut value: usize) {
    loop {
        let mut byte = (value & 0x7f) as u8;
        value >>= 7;
        if value != 0 {
            byte |= 0x80;
        }
        output.push(byte);
        if value == 0 {
            break;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::meteora::{MeteoraDlmmFailure, MeteoraSnapshotSource};
    use crate::meteora_m11::{
        MeteoraAccountProvenance, MeteoraExecutionAccountKind, MeteoraExecutionAccountPlan,
        MeteoraPlannedAccount,
    };
    use serde_json::json;

    fn key(seed: u8) -> [u8; 32] {
        [seed; 32]
    }

    fn planned(
        kind: MeteoraExecutionAccountKind,
        pubkey: [u8; 32],
        is_signer: bool,
        is_writable: bool,
    ) -> MeteoraPlannedAccount {
        MeteoraPlannedAccount {
            kind,
            pubkey,
            is_signer,
            is_writable,
            provenance: MeteoraAccountProvenance::ExecutionProvided,
        }
    }

    fn canonical_test_plan() -> MeteoraExecutionAccountPlan {
        let dlmm = key(200);
        let token_program = key(70);

        MeteoraExecutionAccountPlan {
            lb_pair_pubkey: key(1),
            swap_for_y: true,
            source: MeteoraSnapshotSource {
                source_slot: 123,
                generation_id: 9,
            },
            fixed_accounts: vec![
                planned(MeteoraExecutionAccountKind::LbPair, key(1), false, true),
                planned(
                    MeteoraExecutionAccountKind::BinArrayBitmapExtension,
                    dlmm,
                    false,
                    true,
                ),
                planned(MeteoraExecutionAccountKind::ReserveX, key(3), false, true),
                planned(MeteoraExecutionAccountKind::ReserveY, key(4), false, true),
                planned(
                    MeteoraExecutionAccountKind::TokenXMint,
                    key(5),
                    false,
                    false,
                ),
                planned(
                    MeteoraExecutionAccountKind::TokenYMint,
                    key(6),
                    false,
                    false,
                ),
                planned(
                    MeteoraExecutionAccountKind::TokenXProgram,
                    token_program,
                    false,
                    false,
                ),
                planned(
                    MeteoraExecutionAccountKind::TokenYProgram,
                    token_program,
                    false,
                    false,
                ),
                planned(MeteoraExecutionAccountKind::User, key(8), true, false),
                planned(
                    MeteoraExecutionAccountKind::UserTokenIn,
                    key(9),
                    false,
                    true,
                ),
                planned(
                    MeteoraExecutionAccountKind::UserTokenOut,
                    key(10),
                    false,
                    true,
                ),
                planned(MeteoraExecutionAccountKind::Oracle, key(11), false, true),
                planned(MeteoraExecutionAccountKind::HostFeeIn, dlmm, false, true),
                planned(
                    MeteoraExecutionAccountKind::EventAuthority,
                    key(13),
                    false,
                    false,
                ),
                planned(MeteoraExecutionAccountKind::Program, dlmm, false, false),
                planned(
                    MeteoraExecutionAccountKind::MemoProgram,
                    key(15),
                    false,
                    false,
                ),
            ],
            remaining_accounts: vec![
                planned(
                    MeteoraExecutionAccountKind::BinArrayQuoteRequired { index: 4 },
                    key(16),
                    false,
                    true,
                ),
                planned(
                    MeteoraExecutionAccountKind::BinArrayExecutionBuffer { index: 5 },
                    key(17),
                    false,
                    true,
                ),
            ],
            contention_writable_accounts: Vec::new(),
        }
    }

    #[test]
    fn no_alt_compiles_deterministic_unsigned_v0_simulation_transaction(
    ) -> Result<(), MeteoraDlmmFailure> {
        let plan = canonical_test_plan();
        let compiled = compile_meteora_swap2_v0(&plan, 1, 2, key(99), None)?;

        assert_eq!(compiled.simulation_transaction_bytes[0], 1);
        assert_eq!(
            &compiled.simulation_transaction_bytes[1..65],
            &[0_u8; SIGNATURE_BYTES]
        );
        assert_eq!(
            compiled.simulation_transaction_bytes[65],
            VERSIONED_MESSAGE_V0_PREFIX
        );
        assert_eq!(compiled.message_bytes[0], VERSIONED_MESSAGE_V0_PREFIX);
        assert_eq!(compiled.message_bytes[1], 1);
        assert_eq!(compiled.message_bytes[2], 0);
        assert!(compiled.address_table_lookup.is_none());
        assert!(compiled.transaction_len() <= SOLANA_MAX_SERIALIZED_TRANSACTION_BYTES);
        Ok(())
    }

    #[test]
    fn instruction_indices_preserve_m11_account_order_and_intentional_duplicates(
    ) -> Result<(), MeteoraDlmmFailure> {
        let plan = canonical_test_plan();
        let compiled = compile_meteora_swap2_v0(&plan, 1, 2, key(99), None)?;

        assert_eq!(
            compiled.instruction_account_indices.len(),
            plan.ordered_accounts().len()
        );
        assert_eq!(
            compiled.instruction_account_indices[1],
            compiled.instruction_account_indices[12]
        );
        assert_eq!(
            compiled.instruction_account_indices[12],
            compiled.instruction_account_indices[14]
        );
        assert_eq!(
            compiled.instruction_account_indices[6],
            compiled.instruction_account_indices[7]
        );
        assert_eq!(
            compiled.instruction_account_indices[14],
            compiled.program_id_index
        );
        Ok(())
    }

    #[test]
    fn optional_alt_moves_only_eligible_non_signer_non_program_accounts(
    ) -> Result<(), MeteoraDlmmFailure> {
        let plan = canonical_test_plan();
        let table = MeteoraValidatedAddressLookupTable {
            account_key: key(220),
            addresses: vec![key(16), key(5), key(8), key(200)],
        };
        let compiled = compile_meteora_swap2_v0(&plan, 1, 2, key(99), Some(&table))?;
        let lookup = compiled
            .address_table_lookup
            .as_ref()
            .ok_or(MeteoraDlmmFailure::InvalidLayout)?;

        assert_eq!(lookup.writable_indexes, vec![0]);
        assert_eq!(lookup.readonly_indexes, vec![1]);
        assert_eq!(compiled.loaded_writable_accounts, vec![key(16)]);
        assert_eq!(compiled.loaded_readonly_accounts, vec![key(5)]);
        assert!(compiled.static_account_keys.contains(&key(8)));
        assert!(compiled.static_account_keys.contains(&key(200)));
        Ok(())
    }

    #[test]
    fn duplicate_lookup_table_addresses_fail_closed() {
        let plan = canonical_test_plan();
        let table = MeteoraValidatedAddressLookupTable {
            account_key: key(220),
            addresses: vec![key(16), key(16)],
        };

        assert_eq!(
            compile_meteora_swap2_v0(&plan, 1, 2, key(99), Some(&table)),
            Err(MeteoraDlmmFailure::InvalidLayout)
        );
    }

    #[test]
    fn malformed_m11_fixed_account_order_fails_closed() {
        let mut plan = canonical_test_plan();
        plan.fixed_accounts.swap(0, 1);

        assert_eq!(
            compile_meteora_swap2_v0(&plan, 1, 2, key(99), None),
            Err(MeteoraDlmmFailure::InvalidLayout)
        );
    }

    #[test]
    fn oversized_v0_transaction_fails_closed() {
        let mut plan = canonical_test_plan();
        plan.remaining_accounts.clear();

        for index in 0_u8..48 {
            plan.remaining_accounts.push(planned(
                MeteoraExecutionAccountKind::BinArrayExecutionBuffer {
                    index: i64::from(index),
                },
                key(index.saturating_add(30)),
                false,
                true,
            ));
        }

        assert_eq!(
            compile_meteora_swap2_v0(&plan, 1, 2, key(99), None),
            Err(MeteoraDlmmFailure::AccountFootprintExceeded)
        );
    }

    #[test]
    fn latest_blockhash_request_preserves_snapshot_floor() {
        let request = meteora_latest_blockhash_request(41, 123);

        assert_eq!(request["method"], "getLatestBlockhash");
        assert_eq!(request["params"][0]["commitment"], "processed");
        assert_eq!(request["params"][0]["minContextSlot"], 123);
    }

    #[test]
    fn latest_blockhash_parser_requires_exact_32_byte_base58_value() {
        let encoded = bs58::encode(key(77)).into_string();
        let payload = json!({
            "result": {
                "value": {
                    "blockhash": encoded
                }
            }
        });

        assert_eq!(
            parse_meteora_latest_blockhash_response(&payload),
            Ok(key(77))
        );
        assert_eq!(
            parse_meteora_latest_blockhash_response(&json!({
                "result": {
                    "value": {
                        "blockhash": "not-base58-!"
                    }
                }
            })),
            Err(MeteoraDlmmFailure::SimulationFailed)
        );
    }

    #[test]
    fn simulation_request_is_unsigned_read_only_and_slot_bounded() -> Result<(), MeteoraDlmmFailure>
    {
        let plan = canonical_test_plan();
        let compiled = compile_meteora_swap2_v0(&plan, 1, 2, key(99), None)?;
        let request = meteora_simulate_transaction_request(42, &compiled);

        assert_eq!(request["method"], "simulateTransaction");
        assert_eq!(request["params"][1]["encoding"], "base64");
        assert_eq!(request["params"][1]["sigVerify"], false);
        assert_eq!(request["params"][1]["replaceRecentBlockhash"], false);
        assert_eq!(request["params"][1]["minContextSlot"], 123);
        Ok(())
    }

    #[test]
    fn simulation_units_are_observed_when_present() {
        let payload = json!({
            "result": {
                "value": {
                    "err": null,
                    "logs": ["Program log: ok"],
                    "unitsConsumed": 88_123
                }
            }
        });

        assert_eq!(
            parse_meteora_simulation_response(&payload),
            Ok(MeteoraSimulationOutcome {
                units_consumed: Some(88_123),
                logs: vec!["Program log: ok".to_owned()],
            })
        );
    }

    #[test]
    fn missing_simulation_units_remain_unknown_not_zero() {
        let payload = json!({
            "result": {
                "value": {
                    "err": null,
                    "logs": []
                }
            }
        });

        assert_eq!(
            parse_meteora_simulation_response(&payload),
            Ok(MeteoraSimulationOutcome {
                units_consumed: None,
                logs: Vec::new(),
            })
        );
    }

    #[test]
    fn simulation_program_error_fails_closed() {
        let payload = json!({
            "result": {
                "value": {
                    "err": {
                        "InstructionError": [0, "Custom"]
                    },
                    "logs": [],
                    "unitsConsumed": 10
                }
            }
        });

        assert_eq!(
            parse_meteora_simulation_response(&payload),
            Err(MeteoraDlmmFailure::SimulationFailed)
        );
    }
}

