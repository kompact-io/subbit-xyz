use aiken/collection/dict
use cardano/assets.{Assets}
use subbit/prelude.{Amount}
use subbit/types as t

const script_hash: ByteArray = #"11223344556677889900aabbccddeeff00112233"

const asset_name: ByteArray = #"746f6b656e"

const native: t.Asset = t.Native { hash: script_hash, name: asset_name }

/// #### Amount in
/// 
/// A subbit supports mono-assets. See [funds](../adrs/funds.md). The subbit
/// asset is recorded in the constants. On a subbit input, we are permissive
/// about the value, ie any unanticipated native assets are ignored.
pub fn amount_in(asset: t.Asset, value: Assets) -> Amount {
  when asset is {
    t.Lovelace -> value |> assets.lovelace_of
    t.Native { hash, name } -> value |> assets.quantity_of(hash, name)
  }
}

/// #### Amount out
/// 
/// Continuing outputs value is handled in a stricter manner than subbit inputs. The
/// amount out function is similar to the amount in function, but checks the value
/// contains no additional assets. This is to prevent token spamming attacks.
pub fn amount_out(asset: t.Asset, value: Assets) -> Amount {
  let flat = value |> assets.to_dict |> dict.to_pairs
  when asset is {
    t.Lovelace -> {
      expect [Pair(_, ada)] = flat
      expect [Pair(_, amt)] = ada |> dict.to_pairs
      amt
    }
    t.Native { hash, name } ->
      when flat is {
        [_] -> 0
        [_, Pair(hash_, token)] -> {
          expect hash == hash_
          expect [Pair(name_, amt)] = token |> dict.to_pairs
          expect name == name_
          amt
        }
        _ -> fail @"bad value"
      }
  }
}

test amount_in_lovelace_reads_ada() {
  amount_in(t.Lovelace, assets.from_lovelace(42)) == 42
}

test amount_in_lovelace_ignores_extra_native_assets() {
  let value = assets.merge(
    assets.from_lovelace(42),
    assets.from_asset(script_hash, asset_name, 7),
  )
  amount_in(t.Lovelace, value) == 42
}

test amount_in_native_reads_quantity() {
  amount_in(native, assets.from_asset(script_hash, asset_name, 7)) == 7
}

test amount_in_native_ignores_lovelace() {
  let value = assets.merge(
    assets.from_lovelace(42),
    assets.from_asset(script_hash, asset_name, 7),
  )
  amount_in(native, value) == 7
}

test amount_in_native_absent_is_zero() {
  amount_in(native, assets.from_lovelace(42)) == 0
}

test amount_out_lovelace_reads_ada() {
  amount_out(t.Lovelace, assets.from_lovelace(42)) == 42
}

test amount_out_lovelace_rejects_extra_assets() fail {
  let value = assets.merge(
    assets.from_lovelace(42),
    assets.from_asset(script_hash, asset_name, 7),
  )
  amount_out(t.Lovelace, value) == 42
}

test amount_out_native_reads_quantity_alongside_lovelace() {
  let value = assets.merge(
    assets.from_lovelace(42),
    assets.from_asset(script_hash, asset_name, 7),
  )
  amount_out(native, value) == 7
}

test amount_out_native_absent_is_zero_when_only_lovelace() {
  amount_out(native, assets.from_lovelace(42)) == 0
}

test amount_out_native_rejects_mismatched_asset() fail {
  let other = assets.from_asset(
    #"00112233445566778899aabbccddeeff00112200",
    asset_name,
    7,
  )
  let value = assets.merge(assets.from_lovelace(42), other)
  amount_out(native, value) == 0
}

test amount_out_native_rejects_extra_unrelated_asset() fail {
  let other = assets.from_asset(
    #"00112233445566778899aabbccddeeff00112200",
    asset_name,
    7,
  )
  let this = assets.from_asset(script_hash, asset_name, 7)
  let value = assets.merge(assets.from_lovelace(42), assets.merge(this, other))
  amount_out(native, value) == 7
}
