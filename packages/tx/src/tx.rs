use std::collections::BTreeMap;

use cardano_sdk::{
    Address, ChangeStrategy, Hash, Input, Output, SlotBound, Transaction, Value, address::kind,
    transaction::state::ReadyForSigning,
};

use subbit_core::Duration;
use subbit_core::{Hash28, Redeemer, Step, Tag};

use crate::{Bounds, Open, validator};
use crate::{
    Channel, FEE_BUFFER, NetworkParameters, Utxos,
    channel::FromOutputError,
    fuel,
    step::{Want, Will},
};

#[derive(Debug, Clone, thiserror::Error)]
pub enum Error {
    #[error("no channel utxo for this input")]
    MissingInput,
    #[error(transparent)]
    Channel(#[from] FromOutputError),
    #[error(transparent)]
    Step(#[from] crate::step::Error),
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum BuildError {
    #[error("reference script required when stepping channels")]
    MissingReference,
    #[error(transparent)]
    Fuel(#[from] fuel::SelectError),
    #[error("cannot balance tx")]
    Balancing,
}

#[derive(Debug, Clone)]
#[cfg_attr(feature = "serde", derive(serde::Serialize, serde::Deserialize))]
pub struct Tx {
    /// Raw utxos — needed unmodified for the final transaction builder
    /// (addresses, values, everything `Channel` doesn't retain).
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    utxos: BTreeMap<Input, Output>,
    /// Parsed once at construction; pre-step state, never mutated afterward.
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    channels: BTreeMap<Input, Channel>,
    /// Cached intents against existing channels.
    #[cfg_attr(feature = "serde", serde(with = "as_pairs"))]
    wills: BTreeMap<Input, Will>,
    /// Cached intents to open new channels, keyed by the channel's own `Tag`.
    opens: BTreeMap<Tag, Open>,
}

impl Tx {
    pub fn new(utxos: BTreeMap<Input, Output>) -> Result<Self, Error> {
        let channels = utxos
            .iter()
            .map(|(input, output)| Ok((input.clone(), Channel::try_from(output)?)))
            .collect::<Result<_, FromOutputError>>()?;
        Ok(Self {
            utxos,
            channels,
            wills: BTreeMap::new(),
            opens: BTreeMap::new(),
        })
    }

    /// Like `new`, but drops utxos that fail to parse as a `Channel`,
    /// and further restricts to channels for which `keep` returns true.
    pub fn filtered(utxos: BTreeMap<Input, Output>, keep: impl Fn(&Channel) -> bool) -> Self {
        let mut kept_utxos = BTreeMap::new();
        let mut channels = BTreeMap::new();

        for (input, output) in utxos {
            if let Ok(c) = Channel::try_from(&output)
                && keep(&c)
            {
                channels.insert(input.clone(), c);
                kept_utxos.insert(input, output);
            }
        }

        Self {
            utxos: kept_utxos,
            channels,
            wills: BTreeMap::new(),
            opens: BTreeMap::new(),
        }
    }

    // --- Wills: intents against existing channels ---

    pub fn channels(&self) -> &BTreeMap<Input, Channel> {
        &self.channels
    }

    /// Registers a `Want` for `input` against its parsed pre-step `Channel`.
    /// Overwrites any existing intent already cached for the same input.
    pub fn propose(&mut self, input: Input, want: Want) -> Result<(), Error> {
        let channel = self
            .channels
            .get(&input)
            .ok_or(Error::MissingInput)?
            .clone();
        let will = channel.try_will(want).map_err(|boxed| boxed.1)?;
        self.wills.insert(input, will);
        Ok(())
    }

    pub fn drop_intent(&mut self, input: &Input) -> Option<Will> {
        self.wills.remove(input)
    }

    pub fn drop_all_intents(&mut self) {
        self.wills.clear();
    }

    // --- Opens: intents to create new channels ---

    pub fn opens(&self) -> &BTreeMap<Tag, Open> {
        &self.opens
    }

    /// Caches an open, keyed by its channel's own `Tag`. Replaces any
    /// existing open already cached under the same `Tag`.
    pub fn add_open(&mut self, open: Open) {
        let tag = open.channel().constants().tag().clone();
        self.opens.insert(tag, open);
    }

    pub fn drop_open(&mut self, tag: &Tag) -> Option<Open> {
        self.opens.remove(tag)
    }

    pub fn drop_all_opens(&mut self) {
        self.opens.clear();
    }

    // --- Assembly ---

    pub fn inputs(&self) -> Vec<(Input, Redeemer)> {
        self.wills
            .keys()
            .enumerate()
            .map(|(i, input)| (input.clone(), self.redeemer(i)))
            .collect()
    }

    fn redeemer(&self, index: usize) -> Redeemer {
        if index == 0 {
            Redeemer::Main(self.steps())
        } else {
            Redeemer::Defer
        }
    }

    pub fn steps(&self) -> Vec<Step> {
        self.wills.values().map(Will::to_step).collect()
    }

    /// Continuing outputs (from `wills`) followed by new-channel outputs
    /// (from `opens`) — matching the ordering documented on `Will`.
    pub fn outputs(&self, network_id: cardano_sdk::NetworkId) -> Vec<Output> {
        let cont = self.wills.iter().filter_map(|(input, will)| match will {
            Will::Cont {
                output: channel, ..
            } => {
                let pre_output = self.utxos.get(input)?;
                Some(
                    Output::new(pre_output.address().clone(), channel.buffered_value()).with_datum(
                        {
                            let bytes = minicbor::to_vec(channel.datum()).expect("encode");
                            minicbor_old::decode(&bytes)
                                .expect("valid cbor round-trips into PlutusData")
                        },
                    ),
                )
            }
            Will::Eol { .. } => None,
        });
        let opens = self.opens.values().map(|open| {
            Output::new(
                validator::address(network_id, open.delegation()).into(),
                open.channel().buffered_value(),
            )
            .with_datum({
                let value = open.channel().datum();
                let bytes = minicbor::to_vec(value).expect("encode");
                minicbor_old::decode(&bytes).expect("valid cbor round-trips into PlutusData")
            })
        });
        cont.chain(opens).collect()
    }

    pub fn signers(&self) -> Vec<Hash28> {
        let mut signers: Vec<_> = self
            .wills
            .iter()
            .filter_map(|(input, will)| {
                let constants = self.channels.get(input)?.constants();
                Some(will.signer(constants))
            })
            .collect();
        signers.sort();
        signers.dedup();
        signers
    }

    pub fn bounds(&self) -> Bounds {
        self.wills.values().fold(Bounds::default(), |bounds, will| {
            bounds.intersect(&will.bounds())
        })
    }

    /// This channel-batch's own utxo set — the caller chains this with fuel
    /// and reference-input utxos before resolving the full transaction.
    pub fn utxos(&self) -> &BTreeMap<Input, Output> {
        &self.utxos
    }

    /// Assembles and balances the transaction for every cached `will`/`open`
    /// intent. On failure, `self` is left completely unchanged — nothing is
    /// cleared until a `Transaction` is actually produced. On success, all
    /// cached intents (`wills` and `opens`) are dropped, since they've now
    /// been consumed into a concrete transaction.
    pub fn build(
        &mut self,
        network_parameters: &NetworkParameters,
        reference_utxo: Option<&(Input, Output)>,
        change_address: Address<kind::Any>,
        fuel: &Utxos,
    ) -> Result<Transaction<ReadyForSigning>, BuildError> {
        let network_id = network_parameters.network_id;

        let reference_inputs: Vec<_> = reference_utxo.iter().map(|x| x.0.clone()).collect();
        if !self.wills.is_empty() && reference_inputs.is_empty() {
            return Err(BuildError::MissingReference);
        }
        let spent_value = self
            .wills
            .keys()
            .filter_map(|input| self.utxos.get(input))
            .fold(Value::new(0), |mut acc, output| {
                acc.add(output.value());
                acc
            });
        let produced_value =
            self.outputs(network_id)
                .iter()
                .fold(Value::new(0), |mut acc, output| {
                    acc.add(output.value());
                    acc
                });

        // Per-unit shortfall (floored at 0): whatever `produced` needs beyond
        // `spent` — covers both the lovelace fee/min-ada gap and any native
        // asset an `Open` locks up with no corresponding spent input.
        let mut target = fuel::saturating_sub_value(&produced_value, &spent_value);
        let target_lovelace = target.lovelace() + FEE_BUFFER;
        target.with_lovelace(target_lovelace);
        let fuel_inputs = fuel::select(fuel, &target)?;

        let inputs: Vec<_> = self
            .inputs()
            .into_iter()
            .map(|(input, redeemer)| {
                (
                    input,
                    Some({
                        let bytes = minicbor::to_vec(redeemer).expect("encode");
                        minicbor_old::decode(&bytes)
                            .expect("valid cbor round-trips into PlutusData")
                    }),
                )
            })
            .chain(fuel_inputs.iter().map(|i| (i.clone(), None)))
            .collect();
        let outputs = self.outputs(network_id);
        let collaterals = fuel_inputs.clone();
        let specified_signatories = self
            .signers()
            .iter()
            .map(|x| Hash::<28>::from(<[u8; 28]>::from(*x)))
            .collect::<Vec<_>>();
        let bounds = self.bounds();

        let to_slot = |d: Duration| network_parameters.protocol_parameters.posix_to_slot(*d);
        let lower_bound = bounds
            .lower
            .map_or(SlotBound::None, |d| SlotBound::Inclusive(to_slot(d)));
        let upper_bound = bounds
            .upper
            .map_or(SlotBound::None, |d| SlotBound::Exclusive(to_slot(d)));

        let utxos = self
            .utxos
            .iter()
            .chain(fuel.iter())
            .map(|(i, o)| (i.clone(), o.clone()))
            .chain(reference_utxo.iter().map(|i| (i.0.clone(), i.1.clone())))
            .collect::<BTreeMap<_, _>>();

        let tx = Transaction::build(
            &network_parameters.protocol_parameters,
            &utxos,
            |transaction| {
                transaction
                    .with_inputs(inputs.clone())
                    .with_collaterals(collaterals.clone())
                    .with_reference_inputs(reference_inputs.clone())
                    .with_outputs(outputs.clone())
                    .with_specified_signatories(specified_signatories.clone())
                    .with_validity_interval(lower_bound, upper_bound)
                    .with_change_strategy(ChangeStrategy::as_last_output(change_address.clone()))
                    .ok()
            },
        )
        .map_err(|err| {
            println!("{:?}", err);
            BuildError::Balancing
        })?;

        self.wills.clear();
        self.opens.clear();

        Ok(tx)
    }
}

#[cfg(feature = "serde")]
mod as_pairs {
    use serde::{Deserialize, Deserializer, Serialize, Serializer};
    use std::collections::BTreeMap;

    pub fn serialize<K, V, S>(map: &BTreeMap<K, V>, s: S) -> Result<S::Ok, S::Error>
    where
        K: Serialize + Ord,
        V: Serialize,
        S: Serializer,
    {
        map.iter().collect::<Vec<_>>().serialize(s)
    }

    pub fn deserialize<'de, K, V, D>(d: D) -> Result<BTreeMap<K, V>, D::Error>
    where
        K: Deserialize<'de> + Ord,
        V: Deserialize<'de>,
        D: Deserializer<'de>,
    {
        Ok(Vec::<(K, V)>::deserialize(d)?.into_iter().collect())
    }
}
