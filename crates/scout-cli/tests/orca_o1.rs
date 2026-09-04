#[path = "../src/orca.rs"]
mod orca;

#[test]
fn orca_o1_seam_is_compiled_by_the_canonical_test_suite() {
    assert_eq!(
        orca::ORCA_WHIRLPOOL_PROGRAM_ID,
        "whirLbMiicVdio4qvUfM5KAg6Ct8VwpYzGff3uctyCc"
    );

    let request = orca::program_subscribe_request();

    assert_eq!(
        request
            .pointer("/params/0")
            .and_then(serde_json::Value::as_str),
        Some(orca::ORCA_WHIRLPOOL_PROGRAM_ID)
    );
}
