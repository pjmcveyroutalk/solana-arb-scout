use orca_whirlpools_core::TransferFee;

const SPL_TOKEN_PROGRAM_ID: &str = "TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA";
const TOKEN_2022_PROGRAM_ID: &str = "TokenzQdBNbLqP5VEhdkAS6EPFLC1PHnBqCXEpPxuEb";
const CLOCK_SYSVAR_ID: &str = "SysvarC1ock11111111111111111111111111111111";
const SYSVAR_OWNER_ID: &str = "Sysvar1111111111111111111111111111111111111";

const CLOCK_DATA_LEN: usize = 40;
const CLOCK_SLOT_OFFSET: usize = 0;
const CLOCK_EPOCH_OFFSET: usize = 16;
const CLOCK_UNIX_TIMESTAMP_OFFSET: usize = 32;

const TOKEN_2022_MINT_BASE_LEN: usize = 82;
const TOKEN_2022_ACCOUNT_BASE_LEN: usize = 165;
const TOKEN_2022_ACCOUNT_TYPE_OFFSET: usize = TOKEN_2022_ACCOUNT_BASE_LEN;
const TOKEN_2022_MINT_TLV_START: usize = TOKEN_2022_ACCOUNT_TYPE_OFFSET + 1;
const TOKEN_2022_MINT_ACCOUNT_TYPE: u8 = 1;
const TOKEN_2022_TLV_HEADER_LEN: usize = 4;

const EXTENSION_UNINITIALIZED: u16 = 0;
const EXTENSION_TRANSFER_FEE_CONFIG: u16 = 1;
const EXTENSION_TRANSFER_HOOK: u16 = 14;
const EXTENSION_METADATA_POINTER: u16 = 18;
const EXTENSION_TOKEN_METADATA: u16 = 19;

const METADATA_POINTER_LEN: usize = 64;
const TRANSFER_FEE_CONFIG_LEN: usize = 108;
const TRANSFER_FEE_OLDER_EPOCH_OFFSET: usize = 72;
const TRANSFER_FEE_OLDER_MAX_FEE_OFFSET: usize = 80;
const TRANSFER_FEE_OLDER_BPS_OFFSET: usize = 88;
const TRANSFER_FEE_NEWER_EPOCH_OFFSET: usize = 90;
const TRANSFER_FEE_NEWER_MAX_FEE_OFFSET: usize = 98;
const TRANSFER_FEE_NEWER_BPS_OFFSET: usize = 106;
const MAX_FEE_BASIS_POINTS: u16 = 10_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct OrcaQuoteClock {
    pub slot: u64,
    pub epoch: u64,
    pub unix_timestamp: u64,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ParsedTransferFee {
    epoch: u64,
    maximum_fee: u64,
    basis_points: u16,
}

impl ParsedTransferFee {
    fn into_core(self) -> TransferFee {
        TransferFee {
            fee_bps: self.basis_points,
            max_fee: self.maximum_fee,
        }
    }
}

pub fn decode_clock_sysvar(
    account_pubkey: &str,
    account_owner: &str,
    data: &[u8],
) -> Result<OrcaQuoteClock, String> {
    if account_pubkey != CLOCK_SYSVAR_ID {
        return Err(format!(
            "Orca O2 Clock pubkey mismatch: expected {CLOCK_SYSVAR_ID}, got {account_pubkey}"
        ));
    }

    if account_owner != SYSVAR_OWNER_ID {
        return Err(format!(
            "Orca O2 Clock owner mismatch: expected {SYSVAR_OWNER_ID}, got {account_owner}"
        ));
    }

    if data.len() != CLOCK_DATA_LEN {
        return Err(format!(
            "Orca O2 Clock length mismatch: expected {CLOCK_DATA_LEN}, got {}",
            data.len()
        ));
    }

    let unix_timestamp = read_i64(data, CLOCK_UNIX_TIMESTAMP_OFFSET, "Clock unix_timestamp")?;
    let unix_timestamp = u64::try_from(unix_timestamp)
        .map_err(|_| "Orca O2 Clock unix_timestamp is negative".to_owned())?;

    Ok(OrcaQuoteClock {
        slot: read_u64(data, CLOCK_SLOT_OFFSET, "Clock slot")?,
        epoch: read_u64(data, CLOCK_EPOCH_OFFSET, "Clock epoch")?,
        unix_timestamp,
    })
}

pub fn transfer_fee_for_mint(
    account_owner: &str,
    data: &[u8],
    current_epoch: u64,
    label: &str,
) -> Result<Option<TransferFee>, String> {
    match account_owner {
        SPL_TOKEN_PROGRAM_ID => validate_legacy_spl_mint(data, label),
        TOKEN_2022_PROGRAM_ID => {
            current_transfer_fee_for_token_2022_mint(data, current_epoch, label)
        }
        _ => Err(format!(
            "{label} uses unsupported token program {account_owner}"
        )),
    }
}

pub fn current_transfer_fee_for_token_2022_mint(
    data: &[u8],
    current_epoch: u64,
    label: &str,
) -> Result<Option<TransferFee>, String> {
    validate_token_2022_mint_prefix(data, label)?;

    if data.len() == TOKEN_2022_MINT_BASE_LEN {
        return Ok(None);
    }

    let mut offset = TOKEN_2022_MINT_TLV_START;
    let mut transfer_fee = None;
    let mut metadata_pointer_seen = false;
    let mut token_metadata_seen = false;

    loop {
        let remaining = data.len().saturating_sub(offset);
        if remaining < 2 {
            return Ok(transfer_fee);
        }

        let extension_type = read_u16(data, offset, label)?;
        if extension_type == EXTENSION_UNINITIALIZED {
            return Ok(transfer_fee);
        }

        let header_end = offset
            .checked_add(TOKEN_2022_TLV_HEADER_LEN)
            .ok_or_else(|| format!("{label} Token-2022 extension header offset overflow"))?;

        if header_end > data.len() {
            return Err(format!("{label} has truncated Token-2022 extension header"));
        }

        let extension_len = usize::from(read_u16(data, offset + 2, label)?);
        let value_end = header_end
            .checked_add(extension_len)
            .ok_or_else(|| format!("{label} Token-2022 extension value offset overflow"))?;

        if value_end > data.len() {
            return Err(format!(
                "{label} Token-2022 extension type {extension_type} exceeds account data"
            ));
        }

        let extension_value = data
            .get(header_end..value_end)
            .ok_or_else(|| format!("{label} Token-2022 extension value outside account data"))?;

        match extension_type {
            EXTENSION_TRANSFER_FEE_CONFIG => {
                if transfer_fee.is_some() {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 TransferFeeConfig extensions"
                    ));
                }

                transfer_fee = Some(parse_transfer_fee_config(
                    extension_value,
                    current_epoch,
                    label,
                )?);
            }
            EXTENSION_TRANSFER_HOOK => {
                return Err(format!(
                    "{label} uses Token-2022 TransferHook; Orca O2 quote readiness fails closed"
                ));
            }
            EXTENSION_METADATA_POINTER => {
                if metadata_pointer_seen {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 MetadataPointer extensions"
                    ));
                }

                if extension_len != METADATA_POINTER_LEN {
                    return Err(format!(
                        "{label} MetadataPointer length invalid: {extension_len}"
                    ));
                }

                metadata_pointer_seen = true;
            }
            EXTENSION_TOKEN_METADATA => {
                if token_metadata_seen {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 TokenMetadata extensions"
                    ));
                }

                token_metadata_seen = true;
            }
            _ => {
                return Err(format!(
                    "{label} uses unsupported Token-2022 extension type {extension_type}"
                ));
            }
        }

        offset = value_end;
    }
}

fn validate_legacy_spl_mint(data: &[u8], label: &str) -> Result<Option<TransferFee>, String> {
    if data.len() != TOKEN_2022_MINT_BASE_LEN {
        return Err(format!(
            "{label} legacy SPL Mint length mismatch: expected {TOKEN_2022_MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    match data[45] {
        1 => Ok(None),
        0 => Err(format!("{label} is not initialized")),
        value => Err(format!("{label} has invalid is_initialized value: {value}")),
    }
}

fn validate_token_2022_mint_prefix(data: &[u8], label: &str) -> Result<(), String> {
    if data.len() < TOKEN_2022_MINT_BASE_LEN {
        return Err(format!(
            "{label} Token-2022 Mint too short: expected {TOKEN_2022_MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    match data[45] {
        1 => {}
        0 => return Err(format!("{label} is not initialized")),
        value => return Err(format!("{label} has invalid is_initialized value: {value}")),
    }

    if data.len() == TOKEN_2022_MINT_BASE_LEN {
        return Ok(());
    }

    if data.len() < TOKEN_2022_MINT_TLV_START {
        return Err(format!(
            "{label} malformed Token-2022 extension layout: length={}",
            data.len()
        ));
    }

    let padding = data
        .get(TOKEN_2022_MINT_BASE_LEN..TOKEN_2022_ACCOUNT_BASE_LEN)
        .ok_or_else(|| format!("{label} missing Token-2022 mint padding"))?;

    if padding.iter().any(|byte| *byte != 0) {
        return Err(format!("{label} has nonzero Token-2022 mint padding"));
    }

    if data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] != TOKEN_2022_MINT_ACCOUNT_TYPE {
        return Err(format!(
            "{label} invalid Token-2022 account type: {}",
            data[TOKEN_2022_ACCOUNT_TYPE_OFFSET]
        ));
    }

    Ok(())
}

fn parse_transfer_fee_config(
    data: &[u8],
    current_epoch: u64,
    label: &str,
) -> Result<TransferFee, String> {
    if data.len() != TRANSFER_FEE_CONFIG_LEN {
        return Err(format!(
            "{label} TransferFeeConfig length invalid: expected {TRANSFER_FEE_CONFIG_LEN}, got {}",
            data.len()
        ));
    }

    let older = ParsedTransferFee {
        epoch: read_u64(data, TRANSFER_FEE_OLDER_EPOCH_OFFSET, label)?,
        maximum_fee: read_u64(data, TRANSFER_FEE_OLDER_MAX_FEE_OFFSET, label)?,
        basis_points: read_u16(data, TRANSFER_FEE_OLDER_BPS_OFFSET, label)?,
    };

    let newer = ParsedTransferFee {
        epoch: read_u64(data, TRANSFER_FEE_NEWER_EPOCH_OFFSET, label)?,
        maximum_fee: read_u64(data, TRANSFER_FEE_NEWER_MAX_FEE_OFFSET, label)?,
        basis_points: read_u16(data, TRANSFER_FEE_NEWER_BPS_OFFSET, label)?,
    };

    validate_transfer_fee(older, label, "older")?;
    validate_transfer_fee(newer, label, "newer")?;

    let active = if current_epoch >= newer.epoch {
        newer
    } else {
        older
    };

    Ok(active.into_core())
}

fn validate_transfer_fee(fee: ParsedTransferFee, label: &str, which: &str) -> Result<(), String> {
    if fee.basis_points > MAX_FEE_BASIS_POINTS {
        return Err(format!(
            "{label} Token-2022 {which} transfer fee exceeds 10000 bps: {}",
            fee.basis_points
        ));
    }

    Ok(())
}

fn read_u16(data: &[u8], offset: usize, label: &str) -> Result<u16, String> {
    Ok(u16::from_le_bytes(read_array::<2>(data, offset, label)?))
}

fn read_i64(data: &[u8], offset: usize, label: &str) -> Result<i64, String> {
    Ok(i64::from_le_bytes(read_array::<8>(data, offset, label)?))
}

fn read_u64(data: &[u8], offset: usize, label: &str) -> Result<u64, String> {
    Ok(u64::from_le_bytes(read_array::<8>(data, offset, label)?))
}

fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {
    let end = offset
        .checked_add(N)
        .ok_or_else(|| format!("{label} byte offset overflow"))?;
    let bytes = data
        .get(offset..end)
        .ok_or_else(|| format!("{label} bytes outside account data"))?;

    <[u8; N]>::try_from(bytes).map_err(|_| format!("{label} byte slice had invalid length"))
  }
