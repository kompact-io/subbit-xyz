use aiken/collection/dict
use cardano/assets.{Value}
use subbit/prelude.{Amount}
use subbit/types as t

/// #### Amount in
/// 
/// A subbit supports mono-assets. See [funds](../adrs/funds.md). The subbit
/// currency is recorded in the constants. On a subbit input, we are permissive
/// about the value, ie any unanticipated native assets are ignored.
pub fn amount_in(currency: t.Currency, value: Value) -> Amount {
  when currency is {
    t.Ada -> value |> assets.lovelace_of
    t.Asset { hash, name } -> value |> assets.quantity_of(hash, name)
  }
}

/// #### Amount out
/// 
/// Continuing outputs value is handled in a stricter manner than subbit inputs. The
/// amount out function is similar to the amount in function, but checks the value
/// contains no additional assets. This is to prevent token spamming attacks.
pub fn amount_out(currency: t.Currency, value: Value) -> Amount {
  let flat = value |> assets.to_dict |> dict.to_pairs
  when currency is {
    t.Ada -> {
      expect [Pair(_, ada)] = flat
      expect [Pair(_, amt)] = ada |> dict.to_pairs
      amt
    }
    t.Asset { hash, name } ->
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
