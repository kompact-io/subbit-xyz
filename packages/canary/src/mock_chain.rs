//! An in-memory `CardanoConnector`: no real chain, no funded wallet, but
//! every submitted transaction is evaluated for real against the
//! compiled `subbit` validator via `amaru-uplc`/`amaru-plutus`, not just
//! trusted as a Rust-level simulation. Replaces `packages/e2e`'s old
//! fake-indexer shortcut (which never touched `packages/tx` at all).
//!
//! Bridging note: `cardano_sdk::Transaction` wraps `pallas-primitives
//! 0.35`'s `Tx`, while `amaru-plutus`/`amaru-kernel` are pinned to
//! `pallas-primitives 0.34` (confirmed via `Cargo.lock`: both versions
//! are resolved in this workspace). The two are therefore distinct Rust
//! types for the same real ledger CBOR shape, so every submitted
//! transaction is bridged by CBOR round-trip (`cardano_sdk`'s encoder,
//! `amaru_kernel`'s decoder) rather than a zero-cost field reuse.

use std::collections::BTreeMap;
use std::sync::Mutex;

use amaru_kernel::cbor::from_cbor;
use amaru_kernel::{
    Hash, NetworkName, PROTOCOL_VERSION_10, PlutusVersion as UplcPlutusVersion, Slot,
    Transaction as KernelTx, TransactionId, TxInfo, Utxos as PlutusUtxos,
};
use amaru_plutus::script_context::ToScriptArgs;
use amaru_plutus::to_plutus_data::PlutusVersion as DataPlutusVersion;
use amaru_uplc::arena::Arena;
use amaru_uplc::binder::DeBruijn;
use amaru_uplc::term::Term;
use anyhow::{Context, bail};
use cardano_connector::CardanoConnector;
use cardano_sdk::cbor::ToCbor;
use cardano_sdk::transaction::state::ReadyForSigning;
use cardano_sdk::{Credential, Input, Network, Output, ProtocolParameters, Transaction};
use subbit_tx::VALIDATOR;

/// The real validator's flat-encoded bytes, as the blueprint actually
/// stores them: hex of a CBOR-wrapped bytestring (CIP-57's
/// `compiledCode`), not raw flat bytes. `cardano_sdk::PlutusScript`
/// stores whatever was handed to it (see `packages/tx/src/validator.rs`)
/// without unwrapping that wrapper -- it never needed to, since every
/// other consumer (hashing, reference-script upload) only re-wraps or
/// re-hashes those same bytes rather than feeding them to a UPLC
/// evaluator. This is the one place that does, so this is the one place
/// that must strip it first.
fn decode_cbor_bytestring(bytes: &[u8]) -> anyhow::Result<Vec<u8>> {
    let mut decoder = amaru_kernel::cbor::Decoder::new(bytes);
    Ok(decoder.bytes()?.to_vec())
}

/// Real preprod genesis constants, matching
/// `cardano_sdk::ProtocolParameters::preprod()`'s own
/// (`start_time`/`first_shelley_slot`) -- `Tx::build` (via
/// `session.network_parameters()`) computes a submitted tx's own
/// validity-interval slots against *these* constants, so the mock's own
/// notion of "the current slot" has to agree, or amaru's own
/// `TimeRange::new` sees the tx's validity interval as absurdly far
/// from "now" and rejects it (`InvalidValidityInterval(PastTimeHorizon)`
/// -- hit and fixed during implementation).
const PREPROD_START_TIME_SECS: u64 = 1_654_041_600;
const PREPROD_FIRST_SHELLEY_SLOT: u64 = 86_400;

fn now_as_preprod_slot() -> u64 {
    let delta = crate::time::now().as_secs() - PREPROD_START_TIME_SECS;
    let byron_duration = 20 * PREPROD_FIRST_SHELLEY_SLOT;
    PREPROD_FIRST_SHELLEY_SLOT + (delta - byron_duration)
}

pub struct MockChain {
    utxos: Mutex<BTreeMap<Input, Output>>,
}

impl MockChain {
    pub fn new() -> Self {
        Self {
            utxos: Mutex::new(BTreeMap::new()),
        }
    }

    /// Fund an address directly, bypassing validation -- the mock's
    /// equivalent of a real preprod faucet drip.
    pub fn seed(&self, input: Input, output: Output) {
        self.utxos.lock().unwrap().insert(input, output);
    }

    /// The mock's own notion of "the current slot" tracks real
    /// wall-clock time directly (`now_as_preprod_slot`), with no
    /// separate fast-forward: `L1::want` already makes `Elapse` wait
    /// out its own real window regardless of connector (`close_period`
    /// is deliberately short, see `orchestrator::close_period`'s own
    /// doc comment), so there's nothing here that needs skipping.
    fn current_slot(&self) -> u64 {
        now_as_preprod_slot()
    }

    fn matches(output: &Output, payment: &Credential, delegation: Option<&Credential>) -> bool {
        let Some(address) = output.address().as_shelley() else {
            return false;
        };
        if &address.payment() != payment {
            return false;
        }
        match delegation {
            // Per `CardanoConnector::utxos_at`'s own doc comment: if
            // `delegation` is `None`, any matching-payment address
            // should be returned regardless of its own delegation part.
            None => true,
            Some(d) => address.delegation().as_ref() == Some(d),
        }
    }

    /// Evaluate every redeemer in `tx` against the real compiled
    /// validator, then (only on success) apply the transaction's
    /// effects to the in-memory UTxO set.
    fn evaluate_and_apply(&self, tx: &Transaction<ReadyForSigning>) -> anyhow::Result<()> {
        let kernel_tx: KernelTx =
            from_cbor(&tx.to_cbor()).context("decoding submitted tx into amaru's own tx shape")?;

        let utxos = self.utxos.lock().unwrap();
        let mut plutus_utxos: BTreeMap<_, _> = BTreeMap::new();
        for (input, output) in utxos.iter() {
            let kernel_input = from_cbor(&input.to_cbor())
                .context("decoding a tracked input into amaru's own input shape")?;
            let kernel_output = from_cbor(&output.to_cbor())
                .context("decoding a tracked output into amaru's own output shape")?;
            plutus_utxos.insert(kernel_input, kernel_output);
        }
        let plutus_utxos: PlutusUtxos = plutus_utxos.into();

        let slot = Slot::from(self.current_slot());
        let network = NetworkName::Preprod;
        let era_history = network
            .as_era_history()
            .expect("preprod always has an era history");
        let global_parameters = network
            .as_global_parameters()
            .expect("preprod always has global parameters");
        // Reuse `cardano_sdk`'s own tx-id computation (the same one
        // `apply_effects` uses to name this tx's new outputs) rather than
        // recomputing one through amaru's types, so the two stay
        // consistent with each other.
        let tx_id_bytes: [u8; 32] = tx.id().into();
        let tx_id = TransactionId::new(Hash::from(tx_id_bytes));

        let tx_info = TxInfo::new(
            &kernel_tx.body,
            &kernel_tx.witnesses,
            tx_id,
            &plutus_utxos,
            &slot,
            era_history,
            global_parameters,
        )
        .map_err(|e| {
            anyhow::anyhow!("translating submitted tx into a real ScriptContext: {e:?}")
        })?;

        let contexts = tx_info.to_script_contexts();
        if contexts.is_empty() {
            // Nothing to validate (e.g. a plain ref-script upload/teardown
            // with no spend redeemer). Fall through to applying effects.
            drop(utxos);
            return self.apply_effects(tx);
        }

        let arena = Arena::new();
        let script_bytes = decode_cbor_bytestring(VALIDATOR.script.script())
            .context("unwrapping the validator's CIP-57 compiledCode bytestring")?;
        let program = amaru_uplc::flat::decode::<DeBruijn>(
            &arena,
            &script_bytes,
            UplcPlutusVersion::V3,
            PROTOCOL_VERSION_10,
        )
        .context("decoding the validator's flat-encoded UPLC program")?
        .0;

        for (_redeemer_key, ctx, _script) in contexts {
            // This mock chain only ever has the one `subbit` validator in
            // play, so every script context is evaluated against it
            // directly -- no need to dispatch on `_script`'s own hash.
            let args = ctx
                .to_script_args(DataPlutusVersion::<3>)
                .context("serialising ScriptContext to Plutus data")?;
            let mut applied = program;
            for arg in args {
                // `arg` is amaru_kernel's (pallas-shaped) `PlutusData`, a
                // different type from amaru_uplc's own arena-allocated
                // `PlutusData<'a>` term representation -- bridge the same
                // way `packages/conformance`'s `try_into_plutus_data` does,
                // via a CBOR round-trip.
                let mut buf = Vec::new();
                amaru_kernel::cbor::encode(&arg, &mut buf)
                    .context("encoding ScriptContext arg to CBOR")?;
                let data = amaru_uplc::data::PlutusData::from_cbor(&arena, &buf)
                    .map_err(|e| anyhow::anyhow!("decoding ScriptContext arg for UPLC: {e}"))?;
                let term = Term::data(&arena, data);
                applied = applied.apply(&arena, term);
            }
            let result = applied.eval_default(&arena);
            match result.term {
                // Modern Aiken V3 validators are `expect`/`fail`-based and
                // return `Void` (Unit) on success, not a `Bool` -- an
                // explicit `False` is effectively unreachable in that
                // style, but handled below for completeness anyway.
                Ok(Term::Constant(amaru_uplc::constant::Constant::Unit)) => {}
                Ok(Term::Constant(amaru_uplc::constant::Constant::Boolean(true))) => {}
                other => bail!("validator rejected redeemer: {other:?}"),
            }
        }

        drop(utxos);
        self.apply_effects(tx)
    }

    fn apply_effects(&self, tx: &Transaction<ReadyForSigning>) -> anyhow::Result<()> {
        let mut utxos = self.utxos.lock().unwrap();
        for input in tx.inputs() {
            utxos.remove(&input);
        }
        for (index, output) in tx.outputs().enumerate() {
            let input = Input::new(tx.id(), index as u64);
            utxos.insert(input, output);
        }
        Ok(())
    }
}

impl Default for MockChain {
    fn default() -> Self {
        Self::new()
    }
}

impl CardanoConnector for MockChain {
    fn network(&self) -> Network {
        Network::Preprod
    }

    async fn health(&self) -> anyhow::Result<String> {
        Ok("mock".to_string())
    }

    async fn protocol_parameters(&self) -> anyhow::Result<ProtocolParameters> {
        Ok(ProtocolParameters::preprod())
    }

    async fn utxos_at(
        &self,
        payment: &Credential,
        delegation: Option<&Credential>,
    ) -> anyhow::Result<BTreeMap<Input, Output>> {
        let utxos = self.utxos.lock().unwrap();
        Ok(utxos
            .iter()
            .filter(|(_, output)| Self::matches(output, payment, delegation))
            .map(|(i, o)| (i.clone(), o.clone()))
            .collect())
    }

    async fn submit(&self, transaction: &Transaction<ReadyForSigning>) -> anyhow::Result<()> {
        self.evaluate_and_apply(transaction)
    }
}
