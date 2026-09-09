use crate::meteora::MeteoraDlmmSnapshot;
use crate::meteora_m8::meteora_exact_in_traverse;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct MeteoraM13ExactInputQuote {
    pub input_mint: String,
    pub output_mint: String,
    pub requested_input_raw: u64,
    pub consumed_input_raw: u64,
    pub unspent_input_raw: u64,
    pub amount_out_raw: u64,
    pub trading_fee_raw: u64,
    pub protocol_fee_raw: u64,
    pub user_fee_raw: u64,
    pub fee_on_input: bool,
    pub swap_for_y: bool,
    pub touched_bin_arrays: Vec<i64>,
    pub source_slot: u64,
    pub generation_id: u64,
}

pub fn meteora_m13_pair(snapshot: &MeteoraDlmmSnapshot) -> (String, String) {
    (
        bs58::encode(snapshot.mint_x()).into_string(),
        bs58::encode(snapshot.mint_y()).into_string(),
    )
}

pub fn meteora_m13_quote_exact_input(
    snapshot: &MeteoraDlmmSnapshot,
    input_mint: &str,
    amount_in_raw: u64,
) -> Result<MeteoraM13ExactInputQuote, String> {
    if amount_in_raw == 0 {
        return Err("Meteora exact-input quote amount must be greater than zero".to_owned());
    }

    let input_bytes = decode_meteora_mint(input_mint)?;
    let mint_x = snapshot.mint_x();
    let mint_y = snapshot.mint_y();
    let (swap_for_y, output_bytes) = meteora_m13_direction(input_bytes, mint_x, mint_y)?;
    let output_mint = bs58::encode(output_bytes).into_string();

    let traversal = meteora_exact_in_traverse(snapshot, amount_in_raw, swap_for_y, true)
        .map_err(|error| {
            format!(
                "Meteora authoritative exact-input quote failed: input_mint={input_mint} \
                 amount_in_raw={amount_in_raw} error={error:?}"
            )
        })?;

    traversal.require_executable_full_fill().map_err(|error| {
        format!(
            "Meteora exact-input quote is not executable as a full arb leg: \
             input_mint={input_mint} amount_in_raw={amount_in_raw} \
             consumed_input_raw={} unspent_input_raw={} termination={:?} error={error:?}",
            traversal.consumed_input, traversal.unspent_input, traversal.termination
        )
    })?;

    if traversal.amount_out == 0 {
        return Err(format!(
            "Meteora authoritative exact-input quote produced zero output: \
             input_mint={input_mint} amount_in_raw={amount_in_raw}"
        ));
    }

    let source = snapshot.source();

    Ok(MeteoraM13ExactInputQuote {
        input_mint: input_mint.to_owned(),
        output_mint,
        requested_input_raw: traversal.requested_input,
        consumed_input_raw: traversal.consumed_input,
        unspent_input_raw: traversal.unspent_input,
        amount_out_raw: traversal.amount_out,
        trading_fee_raw: traversal.trading_fee,
        protocol_fee_raw: traversal.protocol_fee,
        user_fee_raw: traversal.user_fee,
        fee_on_input: traversal.fee_on_input,
        swap_for_y,
        touched_bin_arrays: traversal.touched_bin_arrays,
        source_slot: source.source_slot,
        generation_id: source.generation_id,
    })
}

fn decode_meteora_mint(input_mint: &str) -> Result<[u8; 32], String> {
    let decoded = bs58::decode(input_mint)
        .into_vec()
        .map_err(|error| format!("invalid Meteora input mint {input_mint}: {error}"))?;
    let decoded_len = decoded.len();

    decoded.try_into().map_err(|_| {
        format!(
            "invalid Meteora input mint length: mint={input_mint} decoded_len={decoded_len}"
        )
    })
}

fn meteora_m13_direction(
    input_mint: [u8; 32],
    mint_x: [u8; 32],
    mint_y: [u8; 32],
) -> Result<(bool, [u8; 32]), String> {
    if input_mint == mint_x {
        Ok((true, mint_y))
    } else if input_mint == mint_y {
        Ok((false, mint_x))
    } else {
        Err("input mint is not part of the Meteora DLMM pair".to_owned())
    }
}

#[cfg(test)]
mod tests {
    use super::{decode_meteora_mint, meteora_m13_direction};

    #[test]
    fn mint_x_input_maps_to_swap_for_y_and_mint_y_output() -> Result<(), String> {
        let mint_x = [1_u8; 32];
        let mint_y = [2_u8; 32];

        let (swap_for_y, output_mint) = meteora_m13_direction(mint_x, mint_x, mint_y)?;

        assert!(swap_for_y);
        assert_eq!(output_mint, mint_y);
        Ok(())
    }

    #[test]
    fn mint_y_input_maps_to_swap_for_x_and_mint_x_output() -> Result<(), String> {
        let mint_x = [1_u8; 32];
        let mint_y = [2_u8; 32];

        let (swap_for_y, output_mint) = meteora_m13_direction(mint_y, mint_x, mint_y)?;

        assert!(!swap_for_y);
        assert_eq!(output_mint, mint_x);
        Ok(())
    }

    #[test]
    fn unrelated_input_mint_fails_closed() {
        let result = meteora_m13_direction([3_u8; 32], [1_u8; 32], [2_u8; 32]);

        assert!(matches!(
            result,
            Err(error) if error.contains("not part of the Meteora DLMM pair")
        ));
    }

    #[test]
    fn base58_mint_decode_requires_exactly_32_bytes() {
        let short_mint = bs58::encode([7_u8; 31]).into_string();
        let result = decode_meteora_mint(&short_mint);

        assert!(matches!(
            result,
            Err(error) if error.contains("decoded_len=31")
        ));
    }

    #[test]
    fn base58_mint_decode_round_trips_32_bytes() -> Result<(), String> {
        let expected = [9_u8; 32];
        let encoded = bs58::encode(expected).into_string();

        assert_eq!(decode_meteora_mint(&encoded)?, expected);
        Ok(())
    }
}

