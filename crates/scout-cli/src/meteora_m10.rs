pub const SWAP2_DISCRIMINATOR: [u8; 8] = [65, 75, 63, 76, 235, 91, 91, 136];

pub const SWAP2_EMPTY_REMAINING_ACCOUNTS_DATA_LEN: usize = 28;

pub fn serialize_swap2_empty_remaining_accounts(
    amount_in: u64,
    min_amount_out: u64,
) -> [u8; SWAP2_EMPTY_REMAINING_ACCOUNTS_DATA_LEN] {
    let mut data = [0_u8; SWAP2_EMPTY_REMAINING_ACCOUNTS_DATA_LEN];

    data[..8].copy_from_slice(&SWAP2_DISCRIMINATOR);
    data[8..16].copy_from_slice(&amount_in.to_le_bytes());
    data[16..24].copy_from_slice(&min_amount_out.to_le_bytes());

    data
}

#[cfg(test)]
mod tests {
    use super::{
        serialize_swap2_empty_remaining_accounts, SWAP2_DISCRIMINATOR,
        SWAP2_EMPTY_REMAINING_ACCOUNTS_DATA_LEN,
    };

    #[test]
    fn discriminator_matches_official_swap2_discriminator() {
        assert_eq!(SWAP2_DISCRIMINATOR, [65, 75, 63, 76, 235, 91, 91, 136]);
    }

    #[test]
    fn amount_one_min_out_two_matches_official_empty_slices_payload() {
        let data = serialize_swap2_empty_remaining_accounts(1, 2);

        assert_eq!(
            data,
            [
                65, 75, 63, 76, 235, 91, 91, 136, 1, 0, 0, 0, 0, 0, 0, 0, 2, 0, 0, 0, 0, 0, 0, 0,
                0, 0, 0, 0,
            ]
        );
    }

    #[test]
    fn payload_has_exact_empty_remaining_accounts_length() {
        let data = serialize_swap2_empty_remaining_accounts(1, 2);

        assert_eq!(data.len(), SWAP2_EMPTY_REMAINING_ACCOUNTS_DATA_LEN);
        assert_eq!(data.len(), 28);
    }

    #[test]
    fn zero_boundary_serializes_exactly() {
        let data = serialize_swap2_empty_remaining_accounts(0, 0);

        assert_eq!(&data[..8], &SWAP2_DISCRIMINATOR);
        assert_eq!(&data[8..16], &[0_u8; 8]);
        assert_eq!(&data[16..24], &[0_u8; 8]);
        assert_eq!(&data[24..28], &[0_u8; 4]);
    }

    #[test]
    fn u64_max_boundary_serializes_little_endian() {
        let data = serialize_swap2_empty_remaining_accounts(u64::MAX, u64::MAX);

        assert_eq!(&data[8..16], &[0xff_u8; 8]);
        assert_eq!(&data[16..24], &[0xff_u8; 8]);
        assert_eq!(&data[24..28], &[0_u8; 4]);
    }

    #[test]
    fn representative_values_prove_little_endian_argument_order() {
        let data =
            serialize_swap2_empty_remaining_accounts(0x0102_0304_0506_0708, 0x1112_1314_1516_1718);

        assert_eq!(
            &data[8..16],
            &[0x08, 0x07, 0x06, 0x05, 0x04, 0x03, 0x02, 0x01]
        );
        assert_eq!(
            &data[16..24],
            &[0x18, 0x17, 0x16, 0x15, 0x14, 0x13, 0x12, 0x11]
        );
        assert_eq!(&data[24..28], &[0_u8; 4]);
    }
}
