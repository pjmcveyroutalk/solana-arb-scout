use serde::{Deserialize, Serialize};
use std::time::Duration;

const TOKEN_2022_MINT_BASE_LEN: usize = 82;
const TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET: usize = 165;
const TOKEN_2022_MINT_TLV_START: usize = 166;
const TOKEN_2022_TLV_HEADER_LEN: usize = 4;
const TOKEN_2022_MINT_ACCOUNT_TYPE: u8 = 1;

const TOKEN_2022_TRANSFER_FEE_CONFIG_EXTENSION: u16 = 1;
const TOKEN_2022_METADATA_POINTER_EXTENSION: u16 = 18;
const TOKEN_2022_TOKEN_METADATA_EXTENSION: u16 = 19;
const TOKEN_2022_METADATA_POINTER_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum Venue {
    RaydiumCpmm,
    Meteora,
    Orca,
    PumpSwap,
}

impl Venue {
    pub fn label(self) -> &'static str {
        match self {
            Self::RaydiumCpmm => "raydium_cpmm",
            Self::Meteora => "meteora",
            Self::Orca => "orca",
            Self::PumpSwap => "pumpswap",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum LiquidityModel {
    Cpmm,
    Clmm,
    Dlmm,
}

impl LiquidityModel {
    pub fn label(self) -> &'static str {
        match self {
            Self::Cpmm => "cpmm",
            Self::Clmm => "clmm",
            Self::Dlmm => "dlmm",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TokenProgramKind {
    SplToken,
    Token2022,
}

impl TokenProgramKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::SplToken => "spl_token",
            Self::Token2022 => "token_2022",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum CapabilityState {
    Supported,
    Unsupported,
    RequiresHydration,
}

impl CapabilityState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Supported => "supported",
            Self::Unsupported => "unsupported",
            Self::RequiresHydration => "requires_hydration",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AuxiliaryStateKind {
    None,
    Ticks,
    Bins,
}

impl AuxiliaryStateKind {
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "none",
            Self::Ticks => "ticks",
            Self::Bins => "bins",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum ContentionFootprintState {
    Complete,
    Incomplete,
}

impl ContentionFootprintState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Complete => "complete",
            Self::Incomplete => "incomplete",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub struct AdapterCapabilities {
    pub liquidity_model: LiquidityModel,
    pub exact_input_quote: CapabilityState,
    pub spl_token: CapabilityState,
    pub token_2022: CapabilityState,
    pub transfer_fee: CapabilityState,
    pub auxiliary_state: AuxiliaryStateKind,
    pub contention_footprint: ContentionFootprintState,
}

impl AdapterCapabilities {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "liquidity_model={} ",
                "exact_input_quote={} ",
                "spl_token={} ",
                "token_2022={} ",
                "transfer_fee={} ",
                "auxiliary_state={} ",
                "contention_footprint={}"
            ),
            self.liquidity_model.label(),
            self.exact_input_quote.label(),
            self.spl_token.label(),
            self.token_2022.label(),
            self.transfer_fee.label(),
            self.auxiliary_state.label(),
            self.contention_footprint.label(),
        )
    }
}

pub fn ensure_supported_token_2022_mint_extensions(
    data: &[u8],
    label: &str,
) -> Result<(), String> {
    if data.len() < TOKEN_2022_MINT_BASE_LEN {
        return Err(format!(
            "{label} shorter than Token-2022 Mint base layout: expected at least \
             {TOKEN_2022_MINT_BASE_LEN}, got {}",
            data.len()
        ));
    }

    if data.len() == TOKEN_2022_MINT_BASE_LEN {
        return Ok(());
    }

    if data.len() < TOKEN_2022_MINT_TLV_START {
        return Err(format!(
            "{label} has incomplete Token-2022 mint-extension container: length={}",
            data.len()
        ));
    }

    let padding = data
        .get(TOKEN_2022_MINT_BASE_LEN..TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET)
        .ok_or_else(|| format!("{label} missing Token-2022 mint-extension padding"))?;

    if padding.iter().any(|byte| *byte != 0) {
        return Err(format!(
            "{label} has non-zero Token-2022 mint-extension padding"
        ));
    }

    let account_type = *data
        .get(TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET)
        .ok_or_else(|| format!("{label} missing Token-2022 account type"))?;

    if account_type != TOKEN_2022_MINT_ACCOUNT_TYPE {
        return Err(format!(
            "{label} has unexpected Token-2022 account type: expected \
             {TOKEN_2022_MINT_ACCOUNT_TYPE}, got {account_type}"
        ));
    }

    let mut offset = TOKEN_2022_MINT_TLV_START;
    let mut metadata_pointer_seen = false;
    let mut token_metadata_seen = false;

    while offset < data.len() {
        let remaining = data
            .get(offset..)
            .ok_or_else(|| format!("{label} Token-2022 TLV offset outside mint data"))?;

        if remaining.iter().all(|byte| *byte == 0) {
            return Ok(());
        }

        let header_end = offset
            .checked_add(TOKEN_2022_TLV_HEADER_LEN)
            .ok_or_else(|| format!("{label} Token-2022 TLV header offset overflow"))?;

        let header = data.get(offset..header_end).ok_or_else(|| {
            format!(
                "{label} Token-2022 mint extension ended inside a TLV header at offset {offset}"
            )
        })?;

        let extension_type = u16::from_le_bytes([header[0], header[1]]);
        let extension_len = usize::from(u16::from_le_bytes([header[2], header[3]]));

        if extension_type == 0 {
            return Err(format!(
                "{label} has non-zero data after an uninitialized Token-2022 TLV entry"
            ));
        }

        let value_end = header_end
            .checked_add(extension_len)
            .ok_or_else(|| format!("{label} Token-2022 TLV value offset overflow"))?;

        let _value = data.get(header_end..value_end).ok_or_else(|| {
            format!(
                "{label} Token-2022 extension type {extension_type} declares length \
                 {extension_len} beyond mint data"
            )
        })?;

        match extension_type {
            TOKEN_2022_TRANSFER_FEE_CONFIG_EXTENSION => {
                return Err(format!(
                    "{label} uses Token-2022 TransferFeeConfig; transfer-fee quoting \
                     is not implemented, so quoting fails closed"
                ));
            }
            TOKEN_2022_METADATA_POINTER_EXTENSION => {
                if metadata_pointer_seen {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 MetadataPointer extensions"
                    ));
                }

                if extension_len != TOKEN_2022_METADATA_POINTER_LEN {
                    return Err(format!(
                        "{label} Token-2022 MetadataPointer has unexpected length: \
                         expected {TOKEN_2022_METADATA_POINTER_LEN}, got {extension_len}"
                    ));
                }

                metadata_pointer_seen = true;
            }
            TOKEN_2022_TOKEN_METADATA_EXTENSION => {
                if token_metadata_seen {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 TokenMetadata extensions"
                    ));
                }

                token_metadata_seen = true;
            }
            unsupported => {
                return Err(format!(
                    "{label} uses unsupported Token-2022 mint extension type {unsupported}; \
                     quoting fails closed"
                ));
            }
        }

        offset = value_end;
    }

    Ok(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum PoolTradingState {
    Tradable,
    NotYetOpen,
    SwapDisabled,
}

impl PoolTradingState {
    pub fn label(self) -> &'static str {
        match self {
            Self::Tradable => "tradable",
            Self::NotYetOpen => "not_yet_open",
            Self::SwapDisabled => "swap_disabled",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum QuoteReserveState {
    Unavailable,
    Available {
        token_a_raw: u64,
        token_b_raw: u64,
        source_slot: u64,
    },
}

impl QuoteReserveState {
    pub fn is_available(&self) -> bool {
        matches!(self, Self::Available { .. })
    }

    pub fn label(&self) -> &'static str {
        match self {
            Self::Unavailable => "unavailable",
            Self::Available { .. } => "available",
        }
    }

    pub fn summary(&self) -> String {
        match self {
            Self::Unavailable => "quote_reserves=unavailable".to_owned(),
            Self::Available {
                token_a_raw,
                token_b_raw,
                source_slot,
            } => format!(
                "quote_reserves=available reserve_slot={source_slot} \
                 token_a_raw={token_a_raw} token_b_raw={token_b_raw}"
            ),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedToken {
    pub mint: String,
    pub vault: String,
    pub decimals: u8,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct NormalizedPoolState {
    pub pool_id: String,
    pub venue: Venue,
    pub program_id: String,
    pub source_slot: u64,
    pub token_a: NormalizedToken,
    pub token_b: NormalizedToken,
    pub trading_state: PoolTradingState,
    pub quote_reserves: QuoteReserveState,
    pub account_update_received_at_unix_ms: u64,
    pub normalized_at_unix_ms: u64,
}

impl NormalizedPoolState {
    pub fn summary(&self) -> String {
        format!(
            concat!(
                "venue={} ",
                "pool={} ",
                "program={} ",
                "slot={} ",
                "mint_a={} mint_b={} ",
                "vault_a={} vault_b={} ",
                "decimals_a={} decimals_b={} ",
                "trading_state={} ",
                "{} ",
                "received_at_ms={} normalized_at_ms={}"
            ),
            self.venue.label(),
            self.pool_id,
            self.program_id,
            self.source_slot,
            self.token_a.mint,
            self.token_b.mint,
            self.token_a.vault,
            self.token_b.vault,
            self.token_a.decimals,
            self.token_b.decimals,
            self.trading_state.label(),
            self.quote_reserves.summary(),
            self.account_update_received_at_unix_ms,
            self.normalized_at_unix_ms,
        )
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct LatencyObservation {
    pub account_update_to_state: Duration,
    pub state_to_candidate: Duration,
    pub candidate_to_quotes: Duration,
    pub quotes_to_profit: Duration,
    pub profit_to_hypothetical_ready: Duration,
}

impl LatencyObservation {
    pub fn total_time_to_ready(&self) -> Duration {
        self.account_update_to_state
            + self.state_to_candidate
            + self.candidate_to_quotes
            + self.quotes_to_profit
            + self.profit_to_hypothetical_ready
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn sample_normalized_pool() -> NormalizedPoolState {
        NormalizedPoolState {
            pool_id: "Pool111111111111111111111111111111111111".to_owned(),
            venue: Venue::RaydiumCpmm,
            program_id: "Program111111111111111111111111111111111".to_owned(),
            source_slot: 123_456,
            token_a: NormalizedToken {
                mint: "MintA111111111111111111111111111111111111".to_owned(),
                vault: "VaultA11111111111111111111111111111111111".to_owned(),
                decimals: 9,
            },
            token_b: NormalizedToken {
                mint: "MintB111111111111111111111111111111111111".to_owned(),
                vault: "VaultB11111111111111111111111111111111111".to_owned(),
                decimals: 6,
            },
            trading_state: PoolTradingState::Tradable,
            quote_reserves: QuoteReserveState::Unavailable,
            account_update_received_at_unix_ms: 1_000,
            normalized_at_unix_ms: 1_001,
        }
    }

    fn token_2022_mint_with_extensions(extensions: &[(u16, usize)]) -> Vec<u8> {
        let mut data = vec![0u8; TOKEN_2022_MINT_TLV_START];
        data[TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;

        for (extension_type, extension_len) in extensions {
            data.extend_from_slice(&extension_type.to_le_bytes());

            let encoded_len = u16::try_from(*extension_len)
                .unwrap_or(u16::MAX);

            data.extend_from_slice(&encoded_len.to_le_bytes());
            data.resize(data.len() + *extension_len, 0);
        }

        data
    }

    #[test]
    fn adapter_capabilities_preserve_explicit_truth_states() {
        let capabilities = AdapterCapabilities {
            liquidity_model: LiquidityModel::Cpmm,
            exact_input_quote: CapabilityState::Supported,
            spl_token: CapabilityState::Supported,
            token_2022: CapabilityState::RequiresHydration,
            transfer_fee: CapabilityState::RequiresHydration,
            auxiliary_state: AuxiliaryStateKind::None,
            contention_footprint: ContentionFootprintState::Incomplete,
        };

        assert_eq!(capabilities.liquidity_model, LiquidityModel::Cpmm);
        assert_eq!(capabilities.token_2022, CapabilityState::RequiresHydration);
        assert_eq!(
            capabilities.transfer_fee,
            CapabilityState::RequiresHydration
        );
        assert_eq!(
            capabilities.contention_footprint,
            ContentionFootprintState::Incomplete
        );
    }

    #[test]
    fn adapter_capability_summary_is_deterministic() {
        let capabilities = AdapterCapabilities {
            liquidity_model: LiquidityModel::Clmm,
            exact_input_quote: CapabilityState::Supported,
            spl_token: CapabilityState::Supported,
            token_2022: CapabilityState::Supported,
            transfer_fee: CapabilityState::RequiresHydration,
            auxiliary_state: AuxiliaryStateKind::Ticks,
            contention_footprint: ContentionFootprintState::Complete,
        };

        assert_eq!(
            capabilities.summary(),
            concat!(
                "liquidity_model=clmm ",
                "exact_input_quote=supported ",
                "spl_token=supported ",
                "token_2022=supported ",
                "transfer_fee=requires_hydration ",
                "auxiliary_state=ticks ",
                "contention_footprint=complete"
            )
        );
    }

    #[test]
    fn capability_labels_do_not_collapse_incomplete_into_unsupported() {
        assert_eq!(CapabilityState::Supported.label(), "supported");
        assert_eq!(CapabilityState::Unsupported.label(), "unsupported");
        assert_eq!(
            CapabilityState::RequiresHydration.label(),
            "requires_hydration"
        );
    }

    #[test]
    fn liquidity_models_preserve_distinct_pool_mechanics() {
        assert_eq!(LiquidityModel::Cpmm.label(), "cpmm");
        assert_eq!(LiquidityModel::Clmm.label(), "clmm");
        assert_eq!(LiquidityModel::Dlmm.label(), "dlmm");
        assert_eq!(AuxiliaryStateKind::None.label(), "none");
        assert_eq!(AuxiliaryStateKind::Ticks.label(), "ticks");
        assert_eq!(AuxiliaryStateKind::Bins.label(), "bins");
    }

    #[test]
    fn token_2022_base_mint_without_extensions_is_quote_eligible() -> Result<(), String> {
        let data = vec![0u8; TOKEN_2022_MINT_BASE_LEN];

        ensure_supported_token_2022_mint_extensions(&data, "test mint")
    }

    #[test]
    fn token_2022_metadata_only_mint_is_quote_eligible() -> Result<(), String> {
        let data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_METADATA_POINTER_EXTENSION,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
            (TOKEN_2022_TOKEN_METADATA_EXTENSION, 96),
        ]);

        ensure_supported_token_2022_mint_extensions(&data, "test mint")
    }

    #[test]
    fn token_2022_transfer_fee_config_fails_closed() -> Result<(), String> {
        let data =
            token_2022_mint_with_extensions(&[(TOKEN_2022_TRANSFER_FEE_CONFIG_EXTENSION, 108)]);

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("TransferFeeConfig")),
            "transfer-fee mint must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_unknown_extension_fails_closed() -> Result<(), String> {
        let data = token_2022_mint_with_extensions(&[(14, 64)]);

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("unsupported Token-2022 mint extension")),
            "unknown extension must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_duplicate_metadata_pointer_fails_closed() -> Result<(), String> {
        let data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_METADATA_POINTER_EXTENSION,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
            (
                TOKEN_2022_METADATA_POINTER_EXTENSION,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
        ]);

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("duplicate")),
            "duplicate extension must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_wrong_metadata_pointer_length_fails_closed() -> Result<(), String> {
        let data = token_2022_mint_with_extensions(&[(TOKEN_2022_METADATA_POINTER_EXTENSION, 63)]);

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("unexpected length")),
            "malformed MetadataPointer must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_nonzero_padding_fails_closed() -> Result<(), String> {
        let mut data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_METADATA_POINTER_EXTENSION,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
        ]);
        data[TOKEN_2022_MINT_BASE_LEN] = 1;

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("non-zero")),
            "non-zero mint extension padding must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_wrong_account_type_fails_closed() -> Result<(), String> {
        let mut data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_METADATA_POINTER_EXTENSION,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
        ]);
        data[TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET] = 2;

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("unexpected Token-2022 account type")),
            "wrong Token-2022 account type must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_truncated_tlv_header_fails_closed() -> Result<(), String> {
        let mut data = vec![0u8; TOKEN_2022_MINT_TLV_START];
        data[TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;
        data.extend_from_slice(&TOKEN_2022_METADATA_POINTER_EXTENSION.to_le_bytes());

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("TLV header")),
            "truncated TLV header must fail closed"
        );

        Ok(())
    }

    #[test]
    fn token_2022_truncated_tlv_value_fails_closed() -> Result<(), String> {
        let mut data = vec![0u8; TOKEN_2022_MINT_TLV_START];
        data[TOKEN_2022_MINT_EXTENSION_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;
        data.extend_from_slice(&TOKEN_2022_METADATA_POINTER_EXTENSION.to_le_bytes());
        data.extend_from_slice(&(TOKEN_2022_METADATA_POINTER_LEN as u16).to_le_bytes());
        data.extend_from_slice(&[0u8; 8]);

        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(
            matches!(result, Err(error) if error.contains("beyond mint data")),
            "truncated TLV value must fail closed"
        );

        Ok(())
    }

    #[test]
    fn normalized_pool_preserves_venue_independent_identity() {
        let pool = sample_normalized_pool();

        assert_eq!(pool.venue, Venue::RaydiumCpmm);
        assert_eq!(pool.venue.label(), "raydium_cpmm");
        assert_eq!(pool.source_slot, 123_456);
        assert_eq!(pool.token_a.decimals, 9);
        assert_eq!(pool.token_b.decimals, 6);
        assert_eq!(pool.trading_state, PoolTradingState::Tradable);
    }

    #[test]
    fn normalized_pool_marks_missing_quote_reserves_explicitly() {
        let pool = sample_normalized_pool();

        assert_eq!(pool.quote_reserves, QuoteReserveState::Unavailable);
        assert!(!pool.quote_reserves.is_available());
        assert_eq!(pool.quote_reserves.label(), "unavailable");
        assert_eq!(pool.quote_reserves.summary(), "quote_reserves=unavailable");
    }

    #[test]
    fn available_quote_reserves_preserve_snapshot_provenance() {
        let reserves = QuoteReserveState::Available {
            token_a_raw: 9_964,
            token_b_raw: 19_961,
            source_slot: 123_500,
        };

        assert!(reserves.is_available());
        assert_eq!(reserves.label(), "available");
        assert_eq!(
            reserves.summary(),
            "quote_reserves=available reserve_slot=123500 token_a_raw=9964 token_b_raw=19961"
        );
    }

    #[test]
    fn normalized_pool_summary_exposes_provenance_and_timing() {
        let pool = sample_normalized_pool();
        let summary = pool.summary();

        assert!(summary.contains("venue=raydium_cpmm"));
        assert!(summary.contains("slot=123456"));
        assert!(summary.contains("trading_state=tradable"));
        assert!(summary.contains("quote_reserves=unavailable"));
        assert!(summary.contains("received_at_ms=1000"));
        assert!(summary.contains("normalized_at_ms=1001"));
    }
}
