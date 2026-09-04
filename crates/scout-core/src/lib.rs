use serde::{Deserialize, Serialize};
use std::time::Duration;

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

const TOKEN_2022_MINT_BASE_LEN: usize = 82;
const TOKEN_2022_MINT_INITIALIZED_OFFSET: usize = 45;
const TOKEN_2022_ACCOUNT_BASE_LEN: usize = 165;
const TOKEN_2022_ACCOUNT_TYPE_OFFSET: usize = TOKEN_2022_ACCOUNT_BASE_LEN;
const TOKEN_2022_MINT_TLV_START: usize = TOKEN_2022_ACCOUNT_TYPE_OFFSET + 1;
const TOKEN_2022_MINT_ACCOUNT_TYPE: u8 = 1;
const TOKEN_2022_TLV_HEADER_LEN: usize = 4;
const TOKEN_2022_EXTENSION_UNINITIALIZED: u16 = 0;
const TOKEN_2022_EXTENSION_TRANSFER_FEE_CONFIG: u16 = 1;
const TOKEN_2022_EXTENSION_METADATA_POINTER: u16 = 18;
const TOKEN_2022_EXTENSION_TOKEN_METADATA: u16 = 19;
const TOKEN_2022_METADATA_POINTER_LEN: u16 = 64;

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

    match data[TOKEN_2022_MINT_INITIALIZED_OFFSET] {
        1 => {}
        0 => return Err(format!("{label} is not initialized")),
        value => {
            return Err(format!(
                "{label} has invalid is_initialized value: {value}"
            ));
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

    let mut offset = TOKEN_2022_MINT_TLV_START;
    let mut metadata_pointer_seen = false;
    let mut token_metadata_seen = false;

    loop {
        let remaining = data.len().saturating_sub(offset);
        if remaining < 2 {
            return Ok(());
        }

        let type_end = offset
            .checked_add(2)
            .ok_or_else(|| format!("{label} Token-2022 extension type offset overflow"))?;
        let extension_type_bytes = data
            .get(offset..type_end)
            .ok_or_else(|| format!("{label} Token-2022 extension type outside account data"))?;
        let extension_type = u16::from_le_bytes(
            <[u8; 2]>::try_from(extension_type_bytes)
                .map_err(|_| format!("{label} Token-2022 extension type had invalid length"))?,
        );

        if extension_type == TOKEN_2022_EXTENSION_UNINITIALIZED {
            return Ok(());
        }

        let header_end = offset
            .checked_add(TOKEN_2022_TLV_HEADER_LEN)
            .ok_or_else(|| format!("{label} Token-2022 extension header offset overflow"))?;

        if header_end > data.len() {
            return Err(format!("{label} has truncated Token-2022 extension header"));
        }

        let length_bytes = data
            .get(type_end..header_end)
            .ok_or_else(|| format!("{label} Token-2022 extension length outside account data"))?;
        let extension_len = usize::from(u16::from_le_bytes(
            <[u8; 2]>::try_from(length_bytes)
                .map_err(|_| format!("{label} Token-2022 extension length had invalid size"))?,
        ));
        let value_end = header_end
            .checked_add(extension_len)
            .ok_or_else(|| format!("{label} Token-2022 extension value offset overflow"))?;

        if value_end > data.len() {
            return Err(format!(
                "{label} Token-2022 extension type {extension_type} exceeds account data"
            ));
        }

        match extension_type {
            TOKEN_2022_EXTENSION_TRANSFER_FEE_CONFIG => {
                return Err(format!(
                    "{label} uses Token-2022 TransferFeeConfig; transfer-fee quoting is not \
                     supported"
                ));
            }
            TOKEN_2022_EXTENSION_METADATA_POINTER => {
                if metadata_pointer_seen {
                    return Err(format!(
                        "{label} contains duplicate Token-2022 MetadataPointer extensions"
                    ));
                }
                if extension_len != usize::from(TOKEN_2022_METADATA_POINTER_LEN) {
                    return Err(format!(
                        "{label} Token-2022 MetadataPointer has invalid length: expected \
                         {TOKEN_2022_METADATA_POINTER_LEN}, got {extension_len}"
                    ));
                }
                metadata_pointer_seen = true;
            }
            TOKEN_2022_EXTENSION_TOKEN_METADATA => {
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

    fn token_2022_mint_with_extensions(extensions: &[(u16, u16)]) -> Vec<u8> {
        let mut data = vec![0u8; TOKEN_2022_MINT_TLV_START];
        data[TOKEN_2022_MINT_INITIALIZED_OFFSET] = 1;
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;

        for (extension_type, extension_len) in extensions {
            data.extend_from_slice(&extension_type.to_le_bytes());
            data.extend_from_slice(&extension_len.to_le_bytes());
            data.resize(data.len() + usize::from(*extension_len), 0);
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

    #[test]
    fn token_2022_plain_mint_is_eligible() {
        let mut data = vec![0u8; TOKEN_2022_MINT_BASE_LEN];
        data[TOKEN_2022_MINT_INITIALIZED_OFFSET] = 1;

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_ok());
    }

    #[test]
    fn token_2022_metadata_only_extensions_are_eligible() {
        let data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_EXTENSION_METADATA_POINTER,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
            (TOKEN_2022_EXTENSION_TOKEN_METADATA, 8),
        ]);

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_ok());
    }

    #[test]
    fn token_2022_transfer_fee_config_fails_closed() {
        let data = token_2022_mint_with_extensions(&[(
            TOKEN_2022_EXTENSION_TRANSFER_FEE_CONFIG,
            8,
        )]);
        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(matches!(result, Err(error) if error.contains("TransferFeeConfig")));
    }

    #[test]
    fn token_2022_unknown_extension_fails_closed() {
        let data = token_2022_mint_with_extensions(&[(14, 4)]);
        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(matches!(
            result,
            Err(error) if error.contains("unsupported Token-2022 extension")
        ));
    }

    #[test]
    fn token_2022_duplicate_metadata_extension_fails_closed() {
        let data = token_2022_mint_with_extensions(&[
            (
                TOKEN_2022_EXTENSION_METADATA_POINTER,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
            (
                TOKEN_2022_EXTENSION_METADATA_POINTER,
                TOKEN_2022_METADATA_POINTER_LEN,
            ),
        ]);
        let result = ensure_supported_token_2022_mint_extensions(&data, "test mint");

        assert!(matches!(result, Err(error) if error.contains("duplicate")));
    }

    #[test]
    fn token_2022_nonzero_mint_padding_fails_closed() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data[TOKEN_2022_MINT_BASE_LEN] = 1;

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_err());
    }

    #[test]
    fn token_2022_wrong_account_type_fails_closed() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] = 2;

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_err());
    }

    #[test]
    fn token_2022_truncated_header_fails_closed() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data.extend_from_slice(&TOKEN_2022_EXTENSION_METADATA_POINTER.to_le_bytes());
        data.push(64);

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_err());
    }

    #[test]
    fn token_2022_truncated_value_fails_closed() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data.extend_from_slice(&TOKEN_2022_EXTENSION_METADATA_POINTER.to_le_bytes());
        data.extend_from_slice(&TOKEN_2022_METADATA_POINTER_LEN.to_le_bytes());
        data.extend_from_slice(&[0u8; 3]);

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_err());
    }

    #[test]
    fn token_2022_uninitialized_type_terminates_tlv_iteration() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data.extend_from_slice(&TOKEN_2022_EXTENSION_UNINITIALIZED.to_le_bytes());
        data.extend_from_slice(&[9u8; 3]);

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_ok());
    }

    #[test]
    fn token_2022_single_byte_tail_matches_upstream_iteration_boundary() {
        let mut data = token_2022_mint_with_extensions(&[]);
        data.push(9);

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_ok());
    }

    #[test]
    fn token_2022_uninitialized_mint_fails_closed() {
        let data = vec![0u8; TOKEN_2022_MINT_BASE_LEN];

        assert!(ensure_supported_token_2022_mint_extensions(&data, "test mint").is_err());
    }
}
