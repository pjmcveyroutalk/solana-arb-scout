#!/usr/bin/env bash
set -euo pipefail

readonly FIXTURE_PATH="${1:-artifacts/m14-meteora/frozen-mainnet.json}"
readonly SDK_REPOSITORY="https://github.com/MeteoraAg/dlmm-sdk.git"
readonly SDK_COMMIT="576919e3e4368e542c402f000b4264724f7f23ec"
readonly SDK_TOOLCHAIN="1.85.0"
readonly SPL_TOKEN_PROGRAM_ID="TokenkegQfeZyiNwAJbNbGKPFXCWuBvf9Ss623VQ5DA"
readonly DLMM_PROGRAM_ID="LBUZKhRxPF3XUpBCjp4YzTKgLccjZhTSDM9YuVaPwxo"
readonly SYSVAR_OWNER_ID="Sysvar1111111111111111111111111111111111111"

for command_name in git python3 rustup; do
  if ! command -v "${command_name}" >/dev/null 2>&1; then
    echo "Meteora M14 reference requires ${command_name}." >&2
    exit 1
  fi
done

if [[ ! -f "${FIXTURE_PATH}" ]]; then
  echo "Meteora M14 fixture not found: ${FIXTURE_PATH}" >&2
  exit 1
fi

readonly WORK_DIR="$(mktemp -d)"
trap 'rm -rf "$WORK_DIR"' EXIT
readonly SDK_DIR="${WORK_DIR}/dlmm-sdk"
readonly MANIFEST_PATH="${WORK_DIR}/reference-input.json"
readonly REFERENCE_OUTPUT="${WORK_DIR}/reference-output.log"

python3 - "$FIXTURE_PATH" "$WORK_DIR" "$MANIFEST_PATH" \
  "$SDK_COMMIT" "$SPL_TOKEN_PROGRAM_ID" "$DLMM_PROGRAM_ID" "$SYSVAR_OWNER_ID" <<'PY'
import base64
import json
import pathlib
import sys

fixture_path = pathlib.Path(sys.argv[1])
work_dir = pathlib.Path(sys.argv[2])
manifest_path = pathlib.Path(sys.argv[3])
expected_commit = sys.argv[4]
spl_token_program = sys.argv[5]
dlmm_program = sys.argv[6]
sysvar_owner = sys.argv[7]

fixture = json.loads(fixture_path.read_text(encoding="utf-8"))

def require(condition, message):
    if not condition:
        raise SystemExit(f"METEORA M14 DIFFERENTIAL HARD STOP: {message}")

require(fixture.get("schema") == "scout.meteora.m14.frozen-mainnet.v1", "unexpected fixture schema")
require(fixture.get("scope") == "read-only differential certification", "unexpected fixture scope")
reference = fixture.get("reference") or {}
require(reference.get("commit") == expected_commit, "fixture reference commit mismatch")
require(reference.get("function") == "commons::quote::quote_exact_in", "fixture reference function mismatch")
require(reference.get("rust_toolchain") == "1.85.0", "fixture reference toolchain mismatch")
require(fixture.get("scout_toolchain") == "1.80.0", "fixture Scout toolchain mismatch")
require(fixture.get("token_x_program") == spl_token_program, "token X is not legacy SPL Token")
require(fixture.get("token_y_program") == spl_token_program, "token Y is not legacy SPL Token")

payload = fixture.get("frozen_rpc_payload") or {}
result = payload.get("result") or {}
context = result.get("context") or {}
accounts = result.get("value")
pubkeys = fixture.get("frozen_account_pubkeys")
bin_pubkeys = fixture.get("bin_array_pubkeys")

require(isinstance(accounts, list), "frozen account payload is missing")
require(isinstance(pubkeys, list), "frozen account pubkeys are missing")
require(isinstance(bin_pubkeys, list), "BinArray pubkeys are missing")
require(len(accounts) == len(pubkeys), "frozen account/pubkey length mismatch")
require(len(accounts) == 5 + len(bin_pubkeys), "unexpected frozen account count")
require(pubkeys[0] == fixture.get("pool"), "pool pubkey is not first frozen account")
require(pubkeys[1] == fixture.get("mint_x"), "mint X is not second frozen account")
require(pubkeys[2] == fixture.get("mint_y"), "mint Y is not third frozen account")
require(pubkeys[5:] == bin_pubkeys, "frozen BinArray account order mismatch")

source_slot = fixture.get("source_slot")
base_source_slot = fixture.get("base_source_slot")
trigger_slot = fixture.get("trigger_slot")
require(isinstance(source_slot, int), "source slot missing")
require(context.get("slot") == source_slot, "frozen RPC context slot/source slot mismatch")
require(isinstance(base_source_slot, int) and source_slot >= base_source_slot, "frozen slot regressed below base slot")
require(isinstance(trigger_slot, int) and base_source_slot >= trigger_slot, "base slot regressed below trigger slot")

processed = []
for index, (pubkey, account) in enumerate(zip(pubkeys, accounts)):
    if account is None:
        require(index == 4, f"required frozen account {index} is missing")
        processed.append({"pubkey": pubkey, "account": None})
        continue

    require(isinstance(account, dict), f"frozen account {index} is malformed")
    data = account.get("data")
    require(isinstance(data, list) and len(data) == 2 and data[1] == "base64", f"account {index} is not base64")
    raw = base64.b64decode(data[0], validate=True)
    data_path = work_dir / f"account-{index}.bin"
    data_path.write_bytes(raw)
    processed.append({
        "pubkey": pubkey,
        "account": {
            "lamports": account.get("lamports"),
            "owner": account.get("owner"),
            "executable": account.get("executable"),
            "rent_epoch": account.get("rentEpoch"),
            "data_path": str(data_path),
        },
    })

require(processed[0]["account"]["owner"] == dlmm_program, "LB pair owner mismatch")
require(processed[1]["account"]["owner"] == spl_token_program, "mint X owner mismatch")
require(processed[2]["account"]["owner"] == spl_token_program, "mint Y owner mismatch")
require(processed[3]["account"]["owner"] == sysvar_owner, "Clock owner mismatch")
if processed[4]["account"] is not None:
    require(processed[4]["account"]["owner"] == dlmm_program, "bitmap extension owner mismatch")
for index in range(5, len(processed)):
    require(processed[index]["account"]["owner"] == dlmm_program, f"BinArray owner mismatch at index {index}")

manifest = {
    "pool": fixture["pool"],
    "source_slot": source_slot,
    "quote_amount_raw": fixture["quote_amount_raw"],
    "mint_x": fixture["mint_x"],
    "mint_y": fixture["mint_y"],
    "bin_array_pubkeys": bin_pubkeys,
    "accounts": processed,
}
manifest_path.write_text(json.dumps(manifest, separators=(",", ":")), encoding="utf-8")
PY

git init -q "$SDK_DIR"
git -C "$SDK_DIR" remote add origin "$SDK_REPOSITORY"
git -C "$SDK_DIR" fetch -q --depth 1 origin "$SDK_COMMIT"
git -C "$SDK_DIR" checkout -q --detach FETCH_HEAD

ACTUAL_COMMIT="$(git -C "$SDK_DIR" rev-parse HEAD)"
if [[ "$ACTUAL_COMMIT" != "$SDK_COMMIT" ]]; then
  echo "METEORA M14 DIFFERENTIAL HARD STOP: pinned SDK checkout mismatch" >&2
  exit 1
fi

mkdir -p "$SDK_DIR/commons/examples"
cat > "$SDK_DIR/commons/examples/scout_m14_reference.rs" <<'RS'
use anchor_client::solana_sdk::{account::Account, clock::Clock, pubkey::Pubkey};
use commons::dlmm::accounts::{BinArray, BinArrayBitmapExtension, LbPair};
use commons::{
    derive_bin_array_bitmap_extension, derive_bin_array_pda, get_bin_array_pubkeys_for_swap,
    pod_read_unaligned_skip_disc, quote_exact_in,
};
use serde_json::{json, Value};
use std::collections::HashMap;
use std::env;
use std::fs;
use std::str::FromStr;

fn main() -> anyhow::Result<()> {
    let manifest_path = env::args()
        .nth(1)
        .ok_or_else(|| anyhow::anyhow!("missing M14 reference manifest path"))?;
    let manifest: Value = serde_json::from_slice(&fs::read(manifest_path)?)?;

    let pool = parse_pubkey(required_str(&manifest, "pool")?)?;
    let mint_x = parse_pubkey(required_str(&manifest, "mint_x")?)?;
    let mint_y = parse_pubkey(required_str(&manifest, "mint_y")?)?;
    let quote_amount_raw = required_u64(&manifest, "quote_amount_raw")?;
    let accounts = manifest["accounts"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("manifest accounts missing"))?;

    anyhow::ensure!(accounts.len() >= 6, "manifest account set is incomplete");

    let pool_account = account_at(accounts, 0)?;
    anyhow::ensure!(pool_account.owner == commons::dlmm::ID, "LB pair owner mismatch");
    let lb_pair: LbPair = pod_read_unaligned_skip_disc(&pool_account.data)?;
    anyhow::ensure!(lb_pair.token_x_mint == mint_x, "LB pair mint X mismatch");
    anyhow::ensure!(lb_pair.token_y_mint == mint_y, "LB pair mint Y mismatch");

    let mint_x_account = account_at(accounts, 1)?;
    let mint_y_account = account_at(accounts, 2)?;
    let clock_account = account_at(accounts, 3)?;
    anyhow::ensure!(
        mint_x_account.owner == anchor_spl::token::spl_token::ID,
        "mint X is not legacy SPL Token"
    );
    anyhow::ensure!(
        mint_y_account.owner == anchor_spl::token::spl_token::ID,
        "mint Y is not legacy SPL Token"
    );
    let clock: Clock = bincode::deserialize(clock_account.data.as_ref())?;

    let bitmap_pubkey = parse_pubkey(account_pubkey(accounts, 4)?)?;
    let expected_bitmap = derive_bin_array_bitmap_extension(pool).0;
    anyhow::ensure!(bitmap_pubkey == expected_bitmap, "bitmap extension PDA mismatch");

    let bitmap_account = optional_account_at(accounts, 4)?;
    let bitmap_extension = match bitmap_account.as_ref() {
        Some(account) => {
            anyhow::ensure!(account.owner == commons::dlmm::ID, "bitmap owner mismatch");
            Some(pod_read_unaligned_skip_disc::<BinArrayBitmapExtension>(&account.data)?)
        }
        None => None,
    };

    let expected_bin_pubkeys = manifest["bin_array_pubkeys"]
        .as_array()
        .ok_or_else(|| anyhow::anyhow!("manifest BinArray pubkeys missing"))?
        .iter()
        .map(|value| {
            value
                .as_str()
                .ok_or_else(|| anyhow::anyhow!("invalid BinArray pubkey"))
                .and_then(parse_pubkey)
        })
        .collect::<anyhow::Result<Vec<_>>>()?;

    let left = get_bin_array_pubkeys_for_swap(
        pool,
        &lb_pair,
        bitmap_extension.as_ref(),
        true,
        3,
    )?;
    let right = get_bin_array_pubkeys_for_swap(
        pool,
        &lb_pair,
        bitmap_extension.as_ref(),
        false,
        3,
    )?;
    let mut official_order = Vec::new();
    for pubkey in left.into_iter().chain(right.into_iter()) {
        if !official_order.contains(&pubkey) {
            official_order.push(pubkey);
        }
    }
    anyhow::ensure!(
        official_order == expected_bin_pubkeys,
        "official bounded BinArray plan differs from frozen Scout plan"
    );

    let mut bin_arrays = HashMap::new();
    for index in 5..accounts.len() {
        let pubkey = parse_pubkey(account_pubkey(accounts, index)?)?;
        let account = account_at(accounts, index)?;
        anyhow::ensure!(account.owner == commons::dlmm::ID, "BinArray owner mismatch");
        let bin_array: BinArray = pod_read_unaligned_skip_disc(&account.data)?;
        let canonical = derive_bin_array_pda(pool, bin_array.index).0;
        anyhow::ensure!(pubkey == canonical, "BinArray PDA mismatch");
        bin_arrays.insert(pubkey, bin_array);
    }

    let x_to_y = quote_exact_in(
        pool,
        &lb_pair,
        quote_amount_raw,
        true,
        bin_arrays.clone(),
        bitmap_extension.as_ref(),
        &clock,
        &mint_x_account,
        &mint_y_account,
    )?;
    let y_to_x = quote_exact_in(
        pool,
        &lb_pair,
        quote_amount_raw,
        false,
        bin_arrays,
        bitmap_extension.as_ref(),
        &clock,
        &mint_x_account,
        &mint_y_account,
    )?;

    println!(
        "SCOUT_M14_REFERENCE_JSON={}",
        json!({
            "x_to_y": {
                "amount_out_raw": x_to_y.amount_out,
                "trading_fee_raw": x_to_y.fee,
                "protocol_fee_raw": x_to_y.protocol_fee,
            },
            "y_to_x": {
                "amount_out_raw": y_to_x.amount_out,
                "trading_fee_raw": y_to_x.fee,
                "protocol_fee_raw": y_to_x.protocol_fee,
            }
        })
    );

    Ok(())
}

fn parse_pubkey(value: &str) -> anyhow::Result<Pubkey> {
    Pubkey::from_str(value).map_err(Into::into)
}

fn required_str<'a>(value: &'a Value, key: &str) -> anyhow::Result<&'a str> {
    value[key]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("manifest field {key} missing"))
}

fn required_u64(value: &Value, key: &str) -> anyhow::Result<u64> {
    value[key]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("manifest field {key} missing"))
}

fn account_pubkey(accounts: &[Value], index: usize) -> anyhow::Result<&str> {
    accounts
        .get(index)
        .and_then(|entry| entry["pubkey"].as_str())
        .ok_or_else(|| anyhow::anyhow!("account pubkey missing at index {index}"))
}

fn optional_account_at(accounts: &[Value], index: usize) -> anyhow::Result<Option<Account>> {
    let entry = accounts
        .get(index)
        .ok_or_else(|| anyhow::anyhow!("account missing at index {index}"))?;
    if entry["account"].is_null() {
        return Ok(None);
    }
    account_from_value(&entry["account"]).map(Some)
}

fn account_at(accounts: &[Value], index: usize) -> anyhow::Result<Account> {
    optional_account_at(accounts, index)?
        .ok_or_else(|| anyhow::anyhow!("required account missing at index {index}"))
}

fn account_from_value(value: &Value) -> anyhow::Result<Account> {
    let lamports = value["lamports"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("account lamports missing"))?;
    let owner = value["owner"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("account owner missing"))?;
    let executable = value["executable"]
        .as_bool()
        .ok_or_else(|| anyhow::anyhow!("account executable flag missing"))?;
    let rent_epoch = value["rent_epoch"]
        .as_u64()
        .ok_or_else(|| anyhow::anyhow!("account rent epoch missing"))?;
    let data_path = value["data_path"]
        .as_str()
        .ok_or_else(|| anyhow::anyhow!("account data path missing"))?;

    Ok(Account {
        lamports,
        data: fs::read(data_path)?,
        owner: parse_pubkey(owner)?,
        executable,
        rent_epoch,
    })
}
RS

rustup toolchain install "$SDK_TOOLCHAIN" --profile minimal

(
  cd "$SDK_DIR"
  timeout 420s rustup run "$SDK_TOOLCHAIN" cargo run --locked -q -p commons \
    --example scout_m14_reference -- "$MANIFEST_PATH"
) | tee "$REFERENCE_OUTPUT"

python3 - "$FIXTURE_PATH" "$REFERENCE_OUTPUT" <<'PY'
import json
import pathlib
import sys

fixture = json.loads(pathlib.Path(sys.argv[1]).read_text(encoding="utf-8"))
lines = pathlib.Path(sys.argv[2]).read_text(encoding="utf-8").splitlines()
prefix = "SCOUT_M14_REFERENCE_JSON="
reference_lines = [line for line in lines if line.startswith(prefix)]
if len(reference_lines) != 1:
    raise SystemExit("METEORA M14 DIFFERENTIAL HARD STOP: reference output marker missing or duplicated")
reference = json.loads(reference_lines[0][len(prefix):])

for direction in ("x_to_y", "y_to_x"):
    scout = fixture["scout_quotes"][direction]
    official = reference[direction]
    if scout["requested_input_raw"] != fixture["quote_amount_raw"]:
        raise SystemExit(f"METEORA M14 DIFFERENTIAL HARD STOP: {direction} requested input mismatch")
    if scout["consumed_input_raw"] != fixture["quote_amount_raw"] or scout["unspent_input_raw"] != 0:
        raise SystemExit(f"METEORA M14 DIFFERENTIAL HARD STOP: {direction} is not a full fill")
    for field in ("amount_out_raw", "trading_fee_raw", "protocol_fee_raw"):
        if scout[field] != official[field]:
            raise SystemExit(
                f"METEORA M14 DIFFERENTIAL HARD STOP: {direction} {field} "
                f"Scout={scout[field]} official={official[field]}"
            )

print("READ-ONLY METEORA M14 DIFFERENTIAL PASS")
PY
