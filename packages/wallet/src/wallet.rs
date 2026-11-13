//! `Wallet`: the surface anything that wants to build and submit a
//! transaction depends on -- address, UTXOs, signing, submission --
//! regardless of whether keys live in-process (`wallet::embedded::
//! Embedded`, this crate's only implementor) or in a browser extension
//! (a future `Cip30`, the "complement" `plan-embedded-wallet.md` names
//! but deliberately does not build here -- it's inherently wasm/JS-
//! dependent and belongs wherever it's actually wired up, most likely
//! `subbit-console`'s own wasm layer). Backend choice is a build-time
//! decision, so this stays a plain `trait` with `impl Future` returns
//! rather than paying for `dyn`-safety nobody needs.
//!
//! Ported in shape (not as a dependency) from the user's own
//! `cardano-lightning/konduit` repo, `packages/cardano/wallet/src/
//! wallet.rs` -- same `cardano-sdk` commit this workspace already pins,
//! so the types below are the exact ones already used throughout this
//! workspace's own tx-building code, not a reinterpretation.
//!
//! **One deliberate deviation from that reference**: `utxos`/`submit`
//! take `&mut self` here, not `&self`. The reference's `Wallet` is
//! shaped around CIP-30's JS calls (inherently `&self`-compatible --
//! calling a JS method never requires an exclusive Rust borrow).
//! `Embedded` instead drives I/O through `floe_client::Backend`, whose
//! own methods are `&mut self` (a short-lived, single-connection
//! client, not a shareable handle) -- forcing `Wallet::utxos`/`submit`
//! to stay `&self` here would mean giving `Embedded` an interior-
//! mutability cell purely to paper over a signature mismatch, for no
//! real benefit. `network_id`/`change_address`/`sign_tx` need no I/O
//! through a mutable backend at all (pure derivation/local signing), so
//! they stay `&self`, matching the reference exactly.

pub mod embedded;
pub mod witness;

use std::collections::{BTreeMap, BTreeSet};
use std::future::Future;

use cardano_sdk::{
    Address, Hash, Input, NetworkId, Output, Signature, Transaction, Value, VerificationKey,
    address::kind, transaction::state,
};

pub trait Wallet {
    type Error: std::error::Error + Send + Sync + 'static;

    fn network_id(&self) -> impl Future<Output = Result<NetworkId, Self::Error>>;

    /// This wallet's one address (single-address wallet, by design --
    /// see `plan-embedded-wallet.md`'s scope note). Analogous to CIP-30's
    /// `api.getChangeAddress()`.
    fn change_address(&self) -> impl Future<Output = Result<Address<kind::Any>, Self::Error>>;

    /// Analogous to CIP-30's `api.getUtxos(value, paginate)` -- pagination
    /// is not honored, but `value`, when given, is: `None` means "the
    /// wallet can't cover it."
    fn utxos(
        &mut self,
        value: Option<Value<u64>>,
    ) -> impl Future<Output = Result<Option<BTreeMap<Input, Output>>, Self::Error>>;

    /// Analogous to CIP-30's `api.signTx(tx, partialSign: true)` --
    /// always a single witness, never a complete signed transaction
    /// (the caller attaches it via `Transaction::add_witness`).
    fn sign_tx(
        &self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> impl Future<Output = Result<(VerificationKey, Signature), Self::Error>>;

    /// Analogous to CIP-30's `api.submitTx(tx: cbor<transaction>):
    /// Promise<hash32>`.
    fn submit(
        &mut self,
        tx: &Transaction<state::ReadyForSigning>,
    ) -> impl Future<Output = Result<Hash<32>, Self::Error>>;
}

/// Polls `wallet.utxos(None)` until a utxo originating from `hash`
/// appears. Exponential backoff, capped attempts: a real preprod
/// indexer lag is usually a handful of seconds, not longer. Shared by
/// every caller that submits a tx through a `Wallet` and needs to wait
/// for it to actually land before doing anything that depends on its
/// effect (`subbit-console`'s own multi-batch `submit`, `subbit-canary`'s
/// `Chain`).
///
/// `sleep` is caller-supplied rather than a direct `tokio::time::sleep`
/// call: this crate has no opinion on async runtime, and `subbit-
/// console`'s own copy of this logic has to work compiled either way
/// (native tokio, or wasm's `setTimeout`) -- see its own `sleep`
/// module's doc comment. A native-only caller (`subbit-canary`) can
/// just pass `tokio::time::sleep` directly.
///
#[derive(Debug, thiserror::Error)]
pub enum WaitForLandingError<E: std::error::Error> {
    #[error("polling for {1} to land: {0}")]
    Utxos(E, Hash<32>),
    #[error("tx {0} still hasn't landed after {1} polls -- giving up")]
    TimedOut(Hash<32>, u32),
}

#[derive(Debug, thiserror::Error)]
pub enum WaitForSpentError<E: std::error::Error> {
    #[error("polling for tx {1}'s inputs to be spent: {0}")]
    Utxos(E, Hash<32>),
    #[error("tx {0} still has unspent inputs after {1} polls -- giving up")]
    TimedOut(Hash<32>, u32),
}

/// How long a caller is willing to wait for a tx to land.
///
/// The default is deliberately much larger than an indexer's usual handful of
/// seconds: submitting a tx to preprod and having it take longer than any fixed
/// short window is normal variance, not a dead transaction. A caller that knows
/// it is on a cleanup path rather than a latency-sensitive one (e.g. a wallet
/// sweep at the end of a run) can opt into `patient`.
#[derive(Debug, Clone, Copy)]
pub struct LandingPolicy {
    pub max_attempts: u32,
    pub initial_wait: std::time::Duration,
    pub max_wait: std::time::Duration,
}

impl LandingPolicy {
    /// Roughly 18 minutes before giving up (2 + 4 + 8 + 16 + 30 * 35 seconds).
    pub const DEFAULT: Self = Self {
        max_attempts: 40,
        initial_wait: std::time::Duration::from_secs(2),
        max_wait: std::time::Duration::from_secs(30),
    };

    /// For cleanup/non-latency-sensitive paths: roughly 38 minutes.
    pub const PATIENT: Self = Self {
        max_attempts: 80,
        initial_wait: std::time::Duration::from_secs(2),
        max_wait: std::time::Duration::from_secs(30),
    };
}

impl Default for LandingPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

/// A `utxos` error here is a hard failure, not treated as "not landed
/// yet": if the backend (Floe) can't answer the question right now,
/// that's a real problem for the caller to see, not something to
/// quietly paper over by assuming "no" and retrying -- a backend
/// returning a genuine error should fail loudly, not be swallowed into
/// a longer wait.
pub async fn wait_for_landing<W, S, F>(
    wallet: &mut W,
    hash: Hash<32>,
    sleep: S,
) -> Result<(), WaitForLandingError<W::Error>>
where
    W: Wallet,
    S: FnMut(std::time::Duration) -> F,
    F: Future<Output = ()>,
{
    wait_for_landing_with(wallet, hash, sleep, LandingPolicy::default()).await
}

/// `wait_for_landing` with an explicit patience policy -- see
/// `LandingPolicy`'s own doc.
pub async fn wait_for_landing_with<W, S, F>(
    wallet: &mut W,
    hash: Hash<32>,
    mut sleep: S,
    policy: LandingPolicy,
) -> Result<(), WaitForLandingError<W::Error>>
where
    W: Wallet,
    S: FnMut(std::time::Duration) -> F,
    F: Future<Output = ()>,
{
    let mut wait = policy.initial_wait;
    for attempt in 0..policy.max_attempts {
        let utxos = wallet
            .utxos(None)
            .await
            .map_err(|e| WaitForLandingError::Utxos(e, hash))?;
        if utxos.is_some_and(|utxos| utxos.keys().any(|input| input.transaction_id() == hash)) {
            return Ok(());
        }
        if attempt + 1 >= policy.max_attempts {
            return Err(WaitForLandingError::TimedOut(hash, policy.max_attempts));
        }
        sleep(wait).await;
        wait = (wait * 2).min(policy.max_wait);
    }
    unreachable!("loop above always returns or errors by its last iteration")
}

/// Waits until every input in `inputs` is gone from `wallet.utxos(None)`.
///
/// Unlike [`wait_for_landing_with`], this does not require the tx to create
/// a utxo at the *same* wallet -- a sweep sends everything to another
/// address, so no utxo carrying its own hash will ever appear there. Waiting
/// for the spent inputs to disappear is therefore the only landing signal a
/// sweep can observe. Requiring two consecutive absent observations guards
/// against a transiently incomplete search result being mistaken for a
/// confirmed spend.
pub async fn wait_for_spent_with<W, S, F>(
    wallet: &mut W,
    hash: Hash<32>,
    inputs: &BTreeSet<Input>,
    mut sleep: S,
    policy: LandingPolicy,
) -> Result<(), WaitForSpentError<W::Error>>
where
    W: Wallet,
    S: FnMut(std::time::Duration) -> F,
    F: Future<Output = ()>,
{
    const SPENT_CONFIRMATIONS: u8 = 2;

    let mut wait = policy.initial_wait;
    let mut missing = 0u8;
    for attempt in 0..policy.max_attempts {
        let utxos = wallet
            .utxos(None)
            .await
            .map_err(|e| WaitForSpentError::Utxos(e, hash))?;
        let any_live = utxos.is_some_and(|utxos| inputs.iter().any(|i| utxos.contains_key(i)));
        if any_live {
            missing = 0;
        } else {
            missing = missing.saturating_add(1);
            if missing >= SPENT_CONFIRMATIONS {
                return Ok(());
            }
        }
        if attempt + 1 >= policy.max_attempts {
            return Err(WaitForSpentError::TimedOut(hash, policy.max_attempts));
        }
        sleep(wait).await;
        wait = (wait * 2).min(policy.max_wait);
    }
    unreachable!("loop above always returns or errors by its last iteration")
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::{Arc, Mutex};

    fn test_hash() -> Hash<32> {
        Hash::from([9u8; 32])
    }

    fn test_output() -> Output {
        let address: Address<kind::Any> = Address::new(
            NetworkId::from(cardano_sdk::Network::Preprod),
            cardano_sdk::Credential::from_key(Hash::<28>::from([1u8; 28])),
        )
        .into();
        Output::new(address, Value::new(1_000_000))
    }

    #[derive(Debug)]
    struct TestWallet {
        polls: Arc<Mutex<usize>>,
        land_on_poll: usize,
        disappear_on_poll: Option<usize>,
        hash: Hash<32>,
    }

    impl Wallet for TestWallet {
        type Error = std::io::Error;

        fn network_id(&self) -> impl Future<Output = Result<NetworkId, Self::Error>> {
            std::future::ready(Ok(NetworkId::from(cardano_sdk::Network::Preprod)))
        }

        fn change_address(&self) -> impl Future<Output = Result<Address<kind::Any>, Self::Error>> {
            let address: Address<kind::Any> = Address::new(
                NetworkId::from(cardano_sdk::Network::Preprod),
                cardano_sdk::Credential::from_key(Hash::<28>::from([1u8; 28])),
            )
            .into();
            std::future::ready(Ok(address))
        }

        fn utxos(
            &mut self,
            _value: Option<Value<u64>>,
        ) -> impl Future<Output = Result<Option<BTreeMap<Input, Output>>, Self::Error>> {
            let poll = {
                let mut polls = self.polls.lock().unwrap();
                *polls += 1;
                *polls
            };
            let present = match self.disappear_on_poll {
                Some(disappear_on) => poll < disappear_on,
                None => poll >= self.land_on_poll,
            };
            let utxos = if present {
                BTreeMap::from([(Input::new(self.hash, 0), test_output())])
            } else {
                BTreeMap::new()
            };
            std::future::ready(Ok(Some(utxos)))
        }

        fn sign_tx(
            &self,
            _tx: &Transaction<state::ReadyForSigning>,
        ) -> impl Future<Output = Result<(VerificationKey, Signature), Self::Error>> {
            std::future::ready(Err(std::io::Error::other("unused in tests")))
        }

        fn submit(
            &mut self,
            _tx: &Transaction<state::ReadyForSigning>,
        ) -> impl Future<Output = Result<Hash<32>, Self::Error>> {
            std::future::ready(Err(std::io::Error::other("unused in tests")))
        }
    }

    fn no_sleep(_: std::time::Duration) -> std::future::Ready<()> {
        std::future::ready(())
    }

    #[tokio::test]
    async fn a_patient_policy_polls_past_the_old_default_budget() {
        let polls = Arc::new(Mutex::new(0));
        let mut wallet = TestWallet {
            polls: polls.clone(),
            land_on_poll: 25,
            disappear_on_poll: None,
            hash: test_hash(),
        };
        let patient = LandingPolicy {
            max_attempts: 40,
            ..LandingPolicy::DEFAULT
        };
        wait_for_landing_with(&mut wallet, test_hash(), no_sleep, patient)
            .await
            .expect("a tx landing on the 25th poll must be seen by a 40-poll policy");
        assert_eq!(*polls.lock().unwrap(), 25);
    }

    #[tokio::test]
    async fn the_default_policy_gives_up_at_its_own_budget() {
        let polls = Arc::new(Mutex::new(0));
        let mut wallet = TestWallet {
            polls: polls.clone(),
            land_on_poll: usize::MAX,
            disappear_on_poll: None,
            hash: test_hash(),
        };
        let result =
            wait_for_landing_with(&mut wallet, test_hash(), no_sleep, LandingPolicy::DEFAULT).await;
        assert!(matches!(
            result,
            Err(WaitForLandingError::TimedOut(_, attempts)) if attempts == LandingPolicy::DEFAULT.max_attempts
        ));
        assert_eq!(
            *polls.lock().unwrap(),
            LandingPolicy::DEFAULT.max_attempts as usize
        );
    }

    #[tokio::test]
    async fn a_sweep_lands_when_its_inputs_disappear_even_with_no_utxo_at_the_source() {
        let polls = Arc::new(Mutex::new(0));
        let hash = test_hash();
        let mut wallet = TestWallet {
            polls: polls.clone(),
            land_on_poll: usize::MAX,
            disappear_on_poll: Some(2),
            hash,
        };
        let inputs = BTreeSet::from([Input::new(hash, 0)]);
        wait_for_spent_with(&mut wallet, hash, &inputs, no_sleep, LandingPolicy::DEFAULT)
            .await
            .expect("a sweep whose inputs are gone must be treated as landed");
        // Poll 1 still sees the input; polls 2 and 3 confirm it gone.
        assert_eq!(*polls.lock().unwrap(), 3);
    }
}
