#![allow(dead_code)]

use orca_whirlpools_core::TransferFee;

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

fn main() -> Result<(), String> {
    println!("Orca O2 Token-2022 quote-semantics candidate");
    println!("Read-only. Current-epoch transfer fees only. TransferHook fails closed.");
    println!("Rust contract: 1.80");
    Ok(())
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
                        "{label} Token-2022 MetadataPointer has invalid length: expected \
                         {METADATA_POINTER_LEN}, got {extension_len}"
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

fn validate_token_2022_mint_prefix(data: &[u8], label: &str) -> Result<(), String> {
    if data.len() < TOKEN_2022_MINT_BASE_LEN {
        return Err(format!(
            "{label} shorter than Token-2022 Mint base layout: expected at least \
             {TOKEN_2022_MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    match data[45] {
        1 => {}
        0 => return Err(format!("{label} is not initialized")),
        value => {
            return Err(format!("{label} has invalid is_initialized value: {value}"));
        }
    }

    if data.len() == TOKEN_2022_MINT_BASE_LEN {
        return Ok(());
    }

    if data.len() < TOKEN_2022_MINT_TLV_START {
        return Err(format!(
            "{label} has malformed Token-2022 extension layout: length={} expected either \
             {TOKEN_2022_MINT_BASE_LEN} or at least {TOKEN_2022_MINT_TLV_START}",
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
            "{label} has invalid Token-2022 account type: expected \
             {TOKEN_2022_MINT_ACCOUNT_TYPE}, got {}",
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
            "{label} Token-2022 TransferFeeConfig has invalid length: expected \
             {TRANSFER_FEE_CONFIG_LEN}, got {}",
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

#[cfg(test)]
mod tests {
    use super::*;

    fn mint_with_extensions(extensions: &[(u16, Vec<u8>)]) -> Result<Vec<u8>, String> {
        let extension_bytes = extensions.iter().try_fold(0usize, |total, (_, value)| {
            total
                .checked_add(TOKEN_2022_TLV_HEADER_LEN)
                .and_then(|next| next.checked_add(value.len()))
                .ok_or_else(|| "test Token-2022 extension size overflow".to_owned())
        })?;

        let total_len = TOKEN_2022_MINT_TLV_START
            .checked_add(extension_bytes)
            .ok_or_else(|| "test Token-2022 mint size overflow".to_owned())?;

        let mut data = vec![0u8; total_len];
        data[45] = 1;
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;

        let mut offset = TOKEN_2022_MINT_TLV_START;

        for (extension_type, value) in extensions {
            let value_len = u16::try_from(value.len())
                .map_err(|_| "test Token-2022 extension value too large".to_owned())?;

            data[offset..offset + 2].copy_from_slice(&extension_type.to_le_bytes());
            data[offset + 2..offset + 4].copy_from_slice(&value_len.to_le_bytes());

            offset += TOKEN_2022_TLV_HEADER_LEN;

            data[offset..offset + value.len()].copy_from_slice(value);
            offset += value.len();
        }

        Ok(data)
    }

    fn transfer_fee_config(
        older_epoch: u64,
        older_max_fee: u64,
        older_bps: u16,
        newer_epoch: u64,
        newer_max_fee: u64,
        newer_bps: u16,
    ) -> Vec<u8> {
        let mut data = vec![0u8; TRANSFER_FEE_CONFIG_LEN];

        data[TRANSFER_FEE_OLDER_EPOCH_OFFSET..TRANSFER_FEE_OLDER_EPOCH_OFFSET + 8]
            .copy_from_slice(&older_epoch.to_le_bytes());
        data[TRANSFER_FEE_OLDER_MAX_FEE_OFFSET..TRANSFER_FEE_OLDER_MAX_FEE_OFFSET + 8]
            .copy_from_slice(&older_max_fee.to_le_bytes());
        data[TRANSFER_FEE_OLDER_BPS_OFFSET..TRANSFER_FEE_OLDER_BPS_OFFSET + 2]
            .copy_from_slice(&older_bps.to_le_bytes());

        data[TRANSFER_FEE_NEWER_EPOCH_OFFSET..TRANSFER_FEE_NEWER_EPOCH_OFFSET + 8]
            .copy_from_slice(&newer_epoch.to_le_bytes());
        data[TRANSFER_FEE_NEWER_MAX_FEE_OFFSET..TRANSFER_FEE_NEWER_MAX_FEE_OFFSET + 8]
            .copy_from_slice(&newer_max_fee.to_le_bytes());
        data[TRANSFER_FEE_NEWER_BPS_OFFSET..TRANSFER_FEE_NEWER_BPS_OFFSET + 2]
            .copy_from_slice(&newer_bps.to_le_bytes());

        data
    }

    #[test]
    fn plain_token_2022_mint_has_no_transfer_fee() -> Result<(), String> {
        let mut data = vec![0u8; TOKEN_2022_MINT_BASE_LEN];
        data[45] = 1;

        let fee = current_transfer_fee_for_token_2022_mint(&data, 99, "test mint")?;

        assert!(fee.is_none());
        Ok(())
    }

    #[test]
    fn current_epoch_before_newer_epoch_selects_older_fee() -> Result<(), String> {
        let config = transfer_fee_config(1, 10, 100, 100, 5_000, 1);
        let data = mint_with_extensions(&[(EXTENSION_TRANSFER_FEE_CONFIG, config)])?;

        let fee = current_transfer_fee_for_token_2022_mint(&data, 99, "test mint")?
            .ok_or_else(|| "expected transfer fee".to_owned())?;

        assert_eq!(fee.fee_bps, 100);
        assert_eq!(fee.max_fee, 10);
        Ok(())
    }

    #[test]
    fn current_epoch_at_newer_epoch_selects_newer_fee() -> Result<(), String> {
        let config = transfer_fee_config(1, 10, 100, 100, 5_000, 1);
        let data = mint_with_extensions(&[(EXTENSION_TRANSFER_FEE_CONFIG, config)])?;

        let fee = current_transfer_fee_for_token_2022_mint(&data, 100, "test mint")?
            .ok_or_else(|| "expected transfer fee".to_owned())?;

        assert_eq!(fee.fee_bps, 1);
        assert_eq!(fee.max_fee, 5_000);
        Ok(())
    }

    #[test]
    fn metadata_extensions_can_coexist_with_transfer_fee() -> Result<(), String> {
        let config = transfer_fee_config(1, 10, 100, 100, 5_000, 1);

        let data = mint_with_extensions(&[
            (EXTENSION_METADATA_POINTER, vec![0u8; METADATA_POINTER_LEN]),
            (EXTENSION_TRANSFER_FEE_CONFIG, config),
            (EXTENSION_TOKEN_METADATA, vec![7u8; 12]),
        ])?;

        let fee = current_transfer_fee_for_token_2022_mint(&data, 100, "test mint")?
            .ok_or_else(|| "expected transfer fee".to_owned())?;

        assert_eq!(fee.fee_bps, 1);
        assert_eq!(fee.max_fee, 5_000);
        Ok(())
    }

    #[test]
    fn transfer_hook_fails_closed() -> Result<(), String> {
        let data = mint_with_extensions(&[(EXTENSION_TRANSFER_HOOK, vec![0u8; 64])])?;

        match current_transfer_fee_for_token_2022_mint(&data, 100, "test mint") {
            Ok(_) => Err("TransferHook mint was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("TransferHook"));
                Ok(())
            }
        }
    }

    #[test]
    fn malformed_transfer_fee_length_fails_closed() -> Result<(), String> {
        let data = mint_with_extensions(&[(EXTENSION_TRANSFER_FEE_CONFIG, vec![0u8; 107])])?;

        match current_transfer_fee_for_token_2022_mint(&data, 100, "test mint") {
            Ok(_) => Err("malformed TransferFeeConfig was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("invalid length"));
                Ok(())
            }
        }
    }

    #[test]
    fn transfer_fee_above_basis_point_limit_fails_closed() -> Result<(), String> {
        let config = transfer_fee_config(1, 10, 10_001, 100, 5_000, 1);
        let data = mint_with_extensions(&[(EXTENSION_TRANSFER_FEE_CONFIG, config)])?;

        match current_transfer_fee_for_token_2022_mint(&data, 99, "test mint") {
            Ok(_) => Err("invalid transfer-fee bps was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("exceeds 10000 bps"));
                Ok(())
            }
        }
    }

    #[test]
    fn unsupported_extension_fails_closed() -> Result<(), String> {
        let data = mint_with_extensions(&[(3u16, vec![0u8; 32])])?;

        match current_transfer_fee_for_token_2022_mint(&data, 100, "test mint") {
            Ok(_) => Err("unsupported Token-2022 extension was accepted".to_owned()),
            Err(error) => {
                assert!(error.contains("unsupported Token-2022 extension type"));
                Ok(())
            }
        }
    }
}
