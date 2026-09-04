#![allow(dead_code)]

#[path = "orca-o2-token2022-candidate.rs"]
mod token2022;

use orca_whirlpools_core::{
    swap_quote_by_input_token, ExactInSwapQuote, OracleFacade, TickArrays, WhirlpoolFacade,
};

fn main() -> Result<(), String> {
    println!("Orca O2 Token-2022 quote integration candidate");
    println!("Read-only. Current-epoch transfer fees feed authoritative Orca quote core.");
    println!("Exact input/output fee rounding and max-fee parity are deterministic.");
    println!("TransferHook and unsupported extensions fail closed before quoting.");
    println!("Rust contract: 1.80");
    Ok(())
}

#[derive(Debug, Clone, Copy)]
pub enum MintFeeSource<'a> {
    SplToken,
    Token2022(&'a [u8]),
}

pub fn current_transfer_fee(
    source: MintFeeSource<'_>,
    current_epoch: u64,
    label: &str,
) -> Result<Option<orca_whirlpools_core::TransferFee>, String> {
    match source {
        MintFeeSource::SplToken => Ok(None),
        MintFeeSource::Token2022(data) => {
            token2022::current_transfer_fee_for_token_2022_mint(data, current_epoch, label)
        }
    }
}

#[allow(clippy::too_many_arguments)]
pub fn quote_exact_input_with_mint_fees(
    token_in: u64,
    specified_token_a: bool,
    slippage_tolerance_bps: u16,
    whirlpool: WhirlpoolFacade,
    oracle: Option<OracleFacade>,
    tick_arrays: TickArrays,
    timestamp: u64,
    current_epoch: u64,
    token_a_fee_source: MintFeeSource<'_>,
    token_b_fee_source: MintFeeSource<'_>,
) -> Result<ExactInSwapQuote, String> {
    if token_in == 0 {
        return Err("Orca exact-input quote amount must be greater than zero".to_owned());
    }

    let transfer_fee_a =
        current_transfer_fee(token_a_fee_source, current_epoch, "Orca token A mint")?;
    let transfer_fee_b =
        current_transfer_fee(token_b_fee_source, current_epoch, "Orca token B mint")?;

    swap_quote_by_input_token(
        token_in,
        specified_token_a,
        slippage_tolerance_bps,
        whirlpool,
        oracle,
        tick_arrays,
        timestamp,
        transfer_fee_a,
        transfer_fee_b,
    )
    .map_err(|error| format!("Orca authoritative exact-input quote failed: {error:?}"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use orca_whirlpools_core::{TickArrayFacade, TickFacade, TICK_ARRAY_SIZE};

    const TOKEN_2022_MINT_BASE_LEN: usize = 82;
    const TOKEN_2022_ACCOUNT_BASE_LEN: usize = 165;
    const TOKEN_2022_ACCOUNT_TYPE_OFFSET: usize = TOKEN_2022_ACCOUNT_BASE_LEN;
    const TOKEN_2022_MINT_TLV_START: usize = TOKEN_2022_ACCOUNT_TYPE_OFFSET + 1;
    const TOKEN_2022_MINT_ACCOUNT_TYPE: u8 = 1;
    const TOKEN_2022_TLV_HEADER_LEN: usize = 4;

    const EXTENSION_TRANSFER_FEE_CONFIG: u16 = 1;
    const EXTENSION_TRANSFER_HOOK: u16 = 14;

    const TRANSFER_FEE_CONFIG_LEN: usize = 108;
    const TRANSFER_FEE_OLDER_EPOCH_OFFSET: usize = 72;
    const TRANSFER_FEE_OLDER_MAX_FEE_OFFSET: usize = 80;
    const TRANSFER_FEE_OLDER_BPS_OFFSET: usize = 88;
    const TRANSFER_FEE_NEWER_EPOCH_OFFSET: usize = 90;
    const TRANSFER_FEE_NEWER_MAX_FEE_OFFSET: usize = 98;
    const TRANSFER_FEE_NEWER_BPS_OFFSET: usize = 106;

    const BPS_DENOMINATOR: u128 = 10_000;
    const TEST_TIMESTAMP: u64 = 1_700_000_000;
    const TEST_EPOCH: u64 = 100;

    fn fixture_whirlpool() -> WhirlpoolFacade {
        WhirlpoolFacade {
            tick_current_index: 0,
            fee_rate: 3_000,
            liquidity: 100_000_000,
            sqrt_price: 1u128 << 64,
            fee_tier_index_seed: [2, 0],
            tick_spacing: 2,
            ..WhirlpoolFacade::default()
        }
    }

    fn fixture_tick(positive_liquidity_net: bool) -> TickFacade {
        let liquidity_net = if positive_liquidity_net {
            1_000
        } else {
            -1_000
        };

        TickFacade {
            initialized: true,
            liquidity_net,
            ..TickFacade::default()
        }
    }

    fn fixture_tick_array(start_tick_index: i32) -> TickArrayFacade {
        TickArrayFacade {
            start_tick_index,
            ticks: [fixture_tick(start_tick_index < 0); TICK_ARRAY_SIZE],
        }
    }

    fn fixture_tick_arrays() -> TickArrays {
        [
            fixture_tick_array(0),
            fixture_tick_array(176),
            fixture_tick_array(352),
            fixture_tick_array(-176),
            fixture_tick_array(-352),
        ]
        .into()
    }

    fn mint_with_extension(extension_type: u16, value: &[u8]) -> Result<Vec<u8>, String> {
        let value_len = u16::try_from(value.len())
            .map_err(|_| "test Token-2022 extension value too large".to_owned())?;

        let total_len = TOKEN_2022_MINT_TLV_START
            .checked_add(TOKEN_2022_TLV_HEADER_LEN)
            .and_then(|len| len.checked_add(value.len()))
            .ok_or_else(|| "test Token-2022 mint size overflow".to_owned())?;

        let mut data = vec![0u8; total_len];
        data[45] = 1;
        data[TOKEN_2022_ACCOUNT_TYPE_OFFSET] = TOKEN_2022_MINT_ACCOUNT_TYPE;

        data[TOKEN_2022_MINT_TLV_START..TOKEN_2022_MINT_TLV_START + 2]
            .copy_from_slice(&extension_type.to_le_bytes());

        data[TOKEN_2022_MINT_TLV_START + 2..TOKEN_2022_MINT_TLV_START + 4]
            .copy_from_slice(&value_len.to_le_bytes());

        let value_start = TOKEN_2022_MINT_TLV_START + TOKEN_2022_TLV_HEADER_LEN;
        data[value_start..value_start + value.len()].copy_from_slice(value);

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

    fn transfer_fee_mint(fee_bps: u16, max_fee: u64) -> Result<Vec<u8>, String> {
        let config = transfer_fee_config(1, max_fee, fee_bps, 200, max_fee, fee_bps);
        mint_with_extension(EXTENSION_TRANSFER_FEE_CONFIG, &config)
    }

    fn plain_token_2022_mint() -> Vec<u8> {
        let mut data = vec![0u8; TOKEN_2022_MINT_BASE_LEN];
        data[45] = 1;
        data
    }

    fn reference_apply_transfer_fee(
        amount: u64,
        fee_bps: u16,
        max_fee: u64,
    ) -> Result<u64, String> {
        if fee_bps > 10_000 {
            return Err("reference transfer fee exceeds 10000 bps".to_owned());
        }

        if fee_bps == 0 || amount == 0 {
            return Ok(amount);
        }

        let numerator = u128::from(amount)
            .checked_mul(u128::from(fee_bps))
            .ok_or_else(|| "reference transfer-fee numerator overflow".to_owned())?;

        let adjusted = numerator
            .checked_add(BPS_DENOMINATOR - 1)
            .ok_or_else(|| "reference transfer-fee rounding overflow".to_owned())?;

        let raw_fee = adjusted / BPS_DENOMINATOR;

        let raw_fee = u64::try_from(raw_fee)
            .map_err(|_| "reference transfer fee does not fit u64".to_owned())?;

        let fee = raw_fee.min(max_fee);

        amount
            .checked_sub(fee)
            .ok_or_else(|| "reference transfer fee exceeds amount".to_owned())
    }

    fn baseline_quote(token_in: u64, specified_token_a: bool) -> Result<ExactInSwapQuote, String> {
        quote_exact_input_with_mint_fees(
            token_in,
            specified_token_a,
            0,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            TEST_EPOCH,
            MintFeeSource::SplToken,
            MintFeeSource::SplToken,
        )
    }

    fn assert_input_fee_parity(specified_token_a: bool) -> Result<(), String> {
        let gross_input = 10_001;
        let fee_bps = 100;
        let max_fee = u64::MAX;
        let net_input = reference_apply_transfer_fee(gross_input, fee_bps, max_fee)?;
        let fee_mint = transfer_fee_mint(fee_bps, max_fee)?;

        let expected = baseline_quote(net_input, specified_token_a)?;

        let actual = if specified_token_a {
            quote_exact_input_with_mint_fees(
                gross_input,
                true,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::Token2022(&fee_mint),
                MintFeeSource::SplToken,
            )?
        } else {
            quote_exact_input_with_mint_fees(
                gross_input,
                false,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::SplToken,
                MintFeeSource::Token2022(&fee_mint),
            )?
        };

        assert_eq!(net_input, 9_900);
        assert_eq!(actual.token_est_out, expected.token_est_out);
        assert_eq!(actual.trade_fee, expected.trade_fee);
        Ok(())
    }

    fn assert_output_fee_parity(specified_token_a: bool) -> Result<(), String> {
        let gross_input = 10_000;
        let fee_bps = 100;
        let max_fee = u64::MAX;
        let fee_mint = transfer_fee_mint(fee_bps, max_fee)?;

        let baseline = baseline_quote(gross_input, specified_token_a)?;

        let expected_output =
            reference_apply_transfer_fee(baseline.token_est_out, fee_bps, max_fee)?;

        let actual = if specified_token_a {
            quote_exact_input_with_mint_fees(
                gross_input,
                true,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::SplToken,
                MintFeeSource::Token2022(&fee_mint),
            )?
        } else {
            quote_exact_input_with_mint_fees(
                gross_input,
                false,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::Token2022(&fee_mint),
                MintFeeSource::SplToken,
            )?
        };

        assert_eq!(actual.token_est_out, expected_output);
        assert_eq!(actual.trade_fee, baseline.trade_fee);
        Ok(())
    }

    fn assert_both_fee_parity(specified_token_a: bool) -> Result<(), String> {
        let gross_input = 10_001;
        let input_fee_bps = 100;
        let output_fee_bps = 250;
        let input_max_fee = u64::MAX;
        let output_max_fee = u64::MAX;

        let net_input = reference_apply_transfer_fee(gross_input, input_fee_bps, input_max_fee)?;

        let input_fee_mint = transfer_fee_mint(input_fee_bps, input_max_fee)?;
        let output_fee_mint = transfer_fee_mint(output_fee_bps, output_max_fee)?;

        let baseline = baseline_quote(net_input, specified_token_a)?;

        let expected_output =
            reference_apply_transfer_fee(baseline.token_est_out, output_fee_bps, output_max_fee)?;

        let actual = if specified_token_a {
            quote_exact_input_with_mint_fees(
                gross_input,
                true,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::Token2022(&input_fee_mint),
                MintFeeSource::Token2022(&output_fee_mint),
            )?
        } else {
            quote_exact_input_with_mint_fees(
                gross_input,
                false,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::Token2022(&output_fee_mint),
                MintFeeSource::Token2022(&input_fee_mint),
            )?
        };

        assert_eq!(actual.token_est_out, expected_output);
        assert_eq!(actual.trade_fee, baseline.trade_fee);
        Ok(())
    }

    fn assert_max_fee_cap_parity(specified_token_a: bool) -> Result<(), String> {
        let gross_input = 10_000;
        let fee_bps = 5_000;
        let max_fee = 7;

        let net_input = reference_apply_transfer_fee(gross_input, fee_bps, max_fee)?;
        assert_eq!(net_input, 9_993);

        let fee_mint = transfer_fee_mint(fee_bps, max_fee)?;
        let expected = baseline_quote(net_input, specified_token_a)?;

        let actual = if specified_token_a {
            quote_exact_input_with_mint_fees(
                gross_input,
                true,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::Token2022(&fee_mint),
                MintFeeSource::SplToken,
            )?
        } else {
            quote_exact_input_with_mint_fees(
                gross_input,
                false,
                0,
                fixture_whirlpool(),
                None,
                fixture_tick_arrays(),
                TEST_TIMESTAMP,
                TEST_EPOCH,
                MintFeeSource::SplToken,
                MintFeeSource::Token2022(&fee_mint),
            )?
        };

        assert_eq!(actual.token_est_out, expected.token_est_out);
        assert_eq!(actual.trade_fee, expected.trade_fee);
        Ok(())
    }

    #[test]
    fn spl_tokens_preserve_authoritative_baseline_quote() -> Result<(), String> {
        let quote = quote_exact_input_with_mint_fees(
            1_000,
            true,
            1_000,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            TEST_EPOCH,
            MintFeeSource::SplToken,
            MintFeeSource::SplToken,
        )?;

        assert_eq!(quote.token_in, 1_000);
        assert_eq!(quote.token_est_out, 996);
        assert_eq!(quote.token_min_out, 896);
        assert_eq!(quote.trade_fee, 3);
        Ok(())
    }

    #[test]
    fn plain_token_2022_mints_match_spl_baseline() -> Result<(), String> {
        let token_a = plain_token_2022_mint();
        let token_b = plain_token_2022_mint();

        let quote = quote_exact_input_with_mint_fees(
            1_000,
            true,
            1_000,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            TEST_EPOCH,
            MintFeeSource::Token2022(&token_a),
            MintFeeSource::Token2022(&token_b),
        )?;

        assert_eq!(quote.token_in, 1_000);
        assert_eq!(quote.token_est_out, 996);
        assert_eq!(quote.token_min_out, 896);
        assert_eq!(quote.trade_fee, 3);
        Ok(())
    }

    #[test]
    fn a_to_b_input_fee_matches_independent_rounding_reference() -> Result<(), String> {
        assert_input_fee_parity(true)
    }

    #[test]
    fn b_to_a_input_fee_matches_independent_rounding_reference() -> Result<(), String> {
        assert_input_fee_parity(false)
    }

    #[test]
    fn a_to_b_output_fee_matches_independent_rounding_reference() -> Result<(), String> {
        assert_output_fee_parity(true)
    }

    #[test]
    fn b_to_a_output_fee_matches_independent_rounding_reference() -> Result<(), String> {
        assert_output_fee_parity(false)
    }

    #[test]
    fn a_to_b_both_fees_match_independent_reference() -> Result<(), String> {
        assert_both_fee_parity(true)
    }

    #[test]
    fn b_to_a_both_fees_match_independent_reference() -> Result<(), String> {
        assert_both_fee_parity(false)
    }

    #[test]
    fn a_to_b_max_fee_cap_matches_independent_reference() -> Result<(), String> {
        assert_max_fee_cap_parity(true)
    }

    #[test]
    fn b_to_a_max_fee_cap_matches_independent_reference() -> Result<(), String> {
        assert_max_fee_cap_parity(false)
    }

    #[test]
    fn current_epoch_changes_quote_when_newer_fee_activates() -> Result<(), String> {
        let config = transfer_fee_config(1, u64::MAX, 10, 100, u64::MAX, 500);
        let token_a = mint_with_extension(EXTENSION_TRANSFER_FEE_CONFIG, &config)?;

        let before = quote_exact_input_with_mint_fees(
            10_000,
            true,
            0,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            99,
            MintFeeSource::Token2022(&token_a),
            MintFeeSource::SplToken,
        )?;

        let after = quote_exact_input_with_mint_fees(
            10_000,
            true,
            0,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            100,
            MintFeeSource::Token2022(&token_a),
            MintFeeSource::SplToken,
        )?;

        assert!(after.token_est_out < before.token_est_out);
        Ok(())
    }

    #[test]
    fn transfer_hook_fails_before_authoritative_quote() -> Result<(), String> {
        let token_a = mint_with_extension(EXTENSION_TRANSFER_HOOK, &[0u8; 64])?;

        match quote_exact_input_with_mint_fees(
            1_000,
            true,
            0,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            TEST_EPOCH,
            MintFeeSource::Token2022(&token_a),
            MintFeeSource::SplToken,
        ) {
            Ok(_) => Err("TransferHook reached authoritative quote core".to_owned()),
            Err(error) => {
                assert!(error.contains("TransferHook"));
                Ok(())
            }
        }
    }

    #[test]
    fn unsupported_token_2022_extension_fails_before_quote() -> Result<(), String> {
        let token_a = mint_with_extension(3, &[0u8; 32])?;

        match quote_exact_input_with_mint_fees(
            1_000,
            true,
            0,
            fixture_whirlpool(),
            None,
            fixture_tick_arrays(),
            TEST_TIMESTAMP,
            TEST_EPOCH,
            MintFeeSource::Token2022(&token_a),
            MintFeeSource::SplToken,
        ) {
            Ok(_) => Err("unsupported Token-2022 extension reached quote core".to_owned()),
            Err(error) => {
                assert!(error.contains("unsupported Token-2022 extension type"));
                Ok(())
            }
        }
    }
}
