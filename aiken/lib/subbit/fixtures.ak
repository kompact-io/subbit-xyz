//// Shared fixtures for unit and tx-esque tests across the `subbit` package.
////
//// Signatures below were produced out-of-band (Aiken cannot sign, only
//// verify) against the exact wire format used by `subbit/iou.verify`:
//// `cbor.serialise((tag, amount))`, i.e. an indefinite CBOR array of the
//// tag bytestring followed by the amount uint.

use cardano/address.{Address, Script}
use cardano/assets.{Assets}
use cardano/transaction.{InlineDatum, Input, Output, OutputReference}
use subbit/types as t

pub const own_hash: ByteArray = #"00112233445566778899aabbccddeeff0011223344"

pub const other_hash: ByteArray = #"ffeeddccbbaa99887766554433221100ffeedd0011"

pub const consumer_vkh: ByteArray = #"c0"

pub const provider_vkh: ByteArray = #"90"

pub const close_period: Int = 100

pub const tag: ByteArray = #"7375626269742d746573742d746167"

/// Public key for which `sig_for_1000` is a valid signature over
/// `(tag, 1000)`.
pub const iou_key: ByteArray =
  #"8a88e3dd7409f195fd52db2d3cba5d72ca6709bf1d94121bf3748801b40f6f5c"

pub const owed_1000: Int = 1000

pub const sig_for_1000: ByteArray =
  #"104e39b968bb917eff7ed097c3b0760d04cdf13cc54f4b89f4d0dc844baf4ea481af64210d615fe6ebff9e2626f177b7063af6a90980e6ffaceced6f4804fd07"

/// A different, unrelated 32-byte key: the public half of the "other-key"
/// fixture, which never signs `(tag, owed_1000)`.
pub const other_iou_key: ByteArray =
  #"8139770ea87d175f56a35466c34c7ecccb8d8a91b4ee37a25df60f5b8fc9b394"

/// A signature that is well-formed (64 bytes) but does not verify against
/// `iou_key`/`tag`/`owed_1000`.
pub const bad_sig: ByteArray =
  #"00000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000000"

pub const constants: t.Constants =
  (tag, t.Lovelace, iou_key, consumer_vkh, provider_vkh, close_period)

pub fn own_cred() {
  Script(own_hash)
}

pub fn own_address() {
  Address { payment_credential: own_cred(), stake_credential: None }
}

pub fn datum(stage: t.Stage) -> t.Datum {
  (own_hash, constants, stage)
}

pub fn datum_with(these_constants: t.Constants, stage: t.Stage) -> t.Datum {
  (own_hash, these_constants, stage)
}

pub fn output_with(
  these_constants: t.Constants,
  stage: t.Stage,
  value: Assets,
) -> Output {
  Output {
    address: own_address(),
    value,
    datum: InlineDatum(datum_with(these_constants, stage)),
    reference_script: None,
  }
}

pub fn input_with(
  oref: OutputReference,
  these_constants: t.Constants,
  stage: t.Stage,
  value: Assets,
) -> Input {
  Input {
    output_reference: oref,
    output: output_with(these_constants, stage, value),
  }
}

pub fn own_output(stage: t.Stage, value: Assets) -> Output {
  output_with(constants, stage, value)
}

pub fn own_input(oref: OutputReference, stage: t.Stage, value: Assets) -> Input {
  Input { output_reference: oref, output: own_output(stage, value) }
}

pub fn oref(idx: Int) -> OutputReference {
  OutputReference { transaction_id: #"00", output_index: idx }
}

/// #### Bench claims
///
/// A small, fixed pool of distinct (consumer, provider, iou key/signature)
/// fixtures used to give benches some spread across N simultaneous
/// `Sub` claims, without needing to generate N distinct ed25519 keypairs.
/// Claims are meant to be cycled (`claim_at(i % n_bench_claims)`) when N
/// exceeds the pool size; the signature-verification cost this models is
/// the same regardless of key/message content, so reuse doesn't bias the
/// measurement — only the size of `extra_signatories`/the ordered vkh list
/// does, which this keeps genuinely distinct up to `n_bench_claims`.
pub type BenchClaim {
  constants: t.Constants,
  owed: Int,
  sig: ByteArray,
}

pub const n_bench_claims: Int = 8

pub fn claim_at(i: Int) -> BenchClaim {
  when i % n_bench_claims is {
    0 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d30",
          t.Lovelace,
          #"2bc2800b3316e009209ffd757dab19ccf0ae84bc7ae90654e1e81712d270f653",
          #"c000",
          #"9000",
          close_period,
        ),
        owed: 1000,
        sig: #"55b0a485179dcedea5f9fad5529b6b726dc378481daba2d24b581a38523e072ce15eacc46e844ba12c6d5edb24ea097debcb6071f93a184e43bec6e5ce3d9d07",
      }
    1 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d31",
          t.Lovelace,
          #"d62f016a1efd1e4fdf793eb42cd84471e1ba9f0cf04d1287b5cc71f616287cb8",
          #"c001",
          #"9001",
          close_period,
        ),
        owed: 1000,
        sig: #"3b09ef1890322cacad6486b046d8cb4fec7f1ae433377d9f65401b5a369a1908315ee74f0dff449198fd3b686525fcbaf6749577d68860d0cd70d89fc24e760f",
      }
    2 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d32",
          t.Lovelace,
          #"34b4d9043156cb6dcf0beb0a2949b7559c940d2bcb6dbe8c53a9b30278e3a746",
          #"c002",
          #"9002",
          close_period,
        ),
        owed: 1000,
        sig: #"8d3081fae6f7ab9dc98f2ef1bc721b2f006347235f3789b737fbabc3ca0dcbe0fb21c04863dfd27e89bae64ba6edb80af30ce5decac4e58852c4c2aeb9a7d102",
      }
    3 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d33",
          t.Lovelace,
          #"12a41592c8b7c17d4059e7b29b61e8ff96c7415f2f803348f2f017e05b9ea1da",
          #"c003",
          #"9003",
          close_period,
        ),
        owed: 1000,
        sig: #"b680b223272f5dcd7d01fdaca8c6154ea7b48530528aab591f49f50c395ee4d7a988be54c3d97f114abb94d2ac6a4d69c54da9aca02bb74fbfe05e6c9e46cc04",
      }
    4 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d34",
          t.Lovelace,
          #"54b0d81d0fa7d00e4a7d600dfaba6f2b22035b22fe335e17edf5f9aa5bb05074",
          #"c004",
          #"9004",
          close_period,
        ),
        owed: 1000,
        sig: #"fb480c140cdf76321276e5a66fa70a44dfa3975c648f6ae4f880d8ba4ad48b2f44809b4c5b67c8f7109c746ef45e8f8c40c6aae78939b84ff125a1638480a900",
      }
    5 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d35",
          t.Lovelace,
          #"e34d3a01b6112e1429ead61668405f4ef4be4f8853abee5079d28ac6c13ffdd0",
          #"c005",
          #"9005",
          close_period,
        ),
        owed: 1000,
        sig: #"f5145c674ae5596fd7743452a6b7b18dac8aa8061447194a8233444a91adc874eb4fd71f9edc949127b74dc1d5079e06edbfd7ec004d7347835657c319c21404",
      }
    6 ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d36",
          t.Lovelace,
          #"750826c281a4f691a41ffb03ef1b4287568ebfc52ec5cca89d953e5a0093abd5",
          #"c006",
          #"9006",
          close_period,
        ),
        owed: 1000,
        sig: #"29235e0dc67700fc5af85b94cfa0a38eb557a4924bd763c6ba6a76115a10bd8675c350911949c16341af1e298cf0be19c032b993fb44d397ed4fc7583966010c",
      }
    _ ->
      BenchClaim {
        constants: (
          #"7375626269742d62656e63682d7461672d37",
          t.Lovelace,
          #"8320a51977d8c38ca8a4927c670df5821e449761945e15e9efb26a1509d230ea",
          #"c007",
          #"9007",
          close_period,
        ),
        owed: 1000,
        sig: #"ed643111e705cf3fcd1a1b0d932916dca664ea5c1b98943c1b64ce46dd1ad59400072f192e74273e5802919ddfee0205077b41ac84c0f3473399129830cb3b02",
      }
  }
}

/// Every provider verification-key hash appearing across `claim_at`'s pool,
/// for seeding `extra_signatories` regardless of how many claims (`N`) a
/// scenario actually uses.
pub const bench_providers: List<ByteArray> =
  [#"9000", #"9001", #"9002", #"9003", #"9004", #"9005", #"9006", #"9007"]
