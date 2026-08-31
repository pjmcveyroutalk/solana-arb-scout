// 1
let pool_data = decode_rpc_account_data(&accounts[0], PUMPSWAP_PROGRAM_ID, "PumpSwap pool")?;

// 2
let data = decode_rpc_account_data(account, PUMPSWAP_PROGRAM_ID, "PumpSwap GlobalConfig")?;

// 3
value => Err(format!("{label} has invalid is_initialized value: {value}")),

// 4
fn read_array<const N: usize>(data: &[u8], offset: usize, label: &str) -> Result<[u8; N], String> {

// 5
_ => Err(format!("PumpSwap {label} invalid bool value: {value}")),

// 6
let error = parse_global_config(&account).expect_err("wrong discriminator must fail");

// 7
assert_eq!(error, "unexpected PumpSwap GlobalConfig discriminator");

// 8
let error = parse_mint_decimals(&data, "test mint").expect_err("mint must fail");

// 9
// The same one-line replacement in the SECOND mint test:
let error = parse_mint_decimals(&data, "test mint").expect_err("mint must fail");

// 10
assert_eq!(error, "test mint has invalid is_initialized value: 2");
