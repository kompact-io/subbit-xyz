use std::collections::BTreeMap;

use cardano_sdk::{
    Address, ChangeStrategy, Credential, Input, NetworkId, Output, PlutusData, SlotBound,
    Transaction, Value, address::kind, cbor::FromCbor, transaction::state::ReadyForSigning,
};
use subbit_core::{Constants, Datum, Duration, Hash28, Stage};

use crate::{
    FEE_BUFFER, Lovelace, MIN_ADA_BUFFER, NetworkParameters, Utxos, VALIDATOR,
    fuel::{self, SelectError},
};

pub struct Bounds {
    pub lower: Option<Duration>,
    pub upper: Option<Duration>,
}

pub struct Open {
    delegation: Option<Credential>,
    constants: Constants,
    amount: Lovelace,
}

impl Open {
    pub fn new(delegation: Option<Credential>, constants: Constants, amount: Lovelace) -> Self {
        Self {
            delegation,
            constants,
            amount,
        }
    }

    pub fn delegation(&self) -> &Option<Credential> {
        &self.delegation
    }

    pub fn address(&self, network_id: NetworkId) -> Address<kind::Shelley> {
        crate::validator::address(network_id, self.delegation.as_ref())
    }

    pub fn amount(&self) -> u64 {
        self.amount
    }

    pub fn value(&self) -> Value<u64> {
        match self.constants.currency() {
            subbit_core::Currency::Ada => Value::new(self.amount()),
            subbit_core::Currency::ByHash { .. } => todo!("Not discernable!"),
            subbit_core::Currency::ByClass { hash, name } => {
                let hash: [u8; 28] = hash.clone();
                let name: Vec<u8> = name.clone();
                let amount: u64 = self.amount();
                Value::new(0).with_assets(vec![(hash.into(), vec![(name, amount)])])
            }
        }
    }

    pub fn buffered_amount(&self) -> u64 {
        match self.constants.currency() {
            subbit_core::Currency::Ada => self.amount() + MIN_ADA_BUFFER,
            _ => self.amount(),
        }
    }

    pub fn buffered_value(&self) -> Value<u64> {
        let mut v = self.value();
        v.add(&Value::new(MIN_ADA_BUFFER));
        v
    }

    pub fn constants(&self) -> &Constants {
        &self.constants
    }

    pub fn datum(&self) -> Datum {
        let own_hash = Hash28::try_from(VALIDATOR.hash.as_ref()).expect("impossible");
        let constants = self.constants().clone();
        let subbed = 0;
        let stage = Stage::Opened { constants, subbed };
        Datum { own_hash, stage }
    }

    pub fn output(&self, network_id: NetworkId) -> Output {
        let pd = PlutusData::from_cbor(&minicbor::to_vec(self.datum()).expect("Infallible"))
            .expect("Failed encoding!");
        Output::new(self.address(network_id).into(), self.buffered_value()).with_datum(pd)
    }
}

pub fn tx(
    network_parameters: &NetworkParameters,
    reference_utxo: Option<&(Input, Output)>,
    change_address: Address<kind::Any>,
    channels: &Utxos,
    steppeds: crate::step::Tx,
    opens: Vec<Open>,
    fuel: &Utxos,
) -> Result<Transaction<ReadyForSigning>, TxError> {
    let network_id = network_parameters.network_id;
    let reference_inputs: Vec<_> = reference_utxo.iter().map(|x| x.0.clone()).collect();
    if !steppeds.inputs.is_empty() && reference_inputs.is_empty() {
        return Err(TxError::ScriptMissing);
    }
    // Not strictly required, but works.
    let mut fuel_needed = Value::new(FEE_BUFFER);
    // FIXME :: Put back in
    // fuel_needed.add(stepped...);
    fuel_needed = opens.iter().fold(fuel_needed, |mut acc, x| {
        acc.add(&x.buffered_value());
        acc
    });
    let fuel_inputs = fuel::select(fuel, &fuel_needed)?;
    // FIXME :: Put back in
    // let inputs = steppeds
    //     .inputs()
    //     .iter()
    //     .map(|i| (i.0.clone(), Some(PlutusData::from(i.1.clone()))))
    //     .chain(fuel_inputs.iter().map(|i| (i.clone(), None)))
    //     .collect::<Vec<_>>();
    let inputs: Vec<_> = fuel_inputs.iter().map(|i| (i.clone(), None)).collect();
    // FIXME :: Put back in
    // let outputs: Vec<_> = steppeds
    //     .outputs()
    //     .into_iter()
    //     .chain(opens.iter().map(|o| o.output(network_id)))
    //     .collect();
    let outputs: Vec<Output> = opens.iter().map(|o| o.output(network_id)).collect();

    // FIXME : FEE_BUFFER is not the right value, but its fine for now
    let collaterals: Vec<Input> = fuel_inputs
        .iter()
        .filter(|input| {
            fuel.get(input)
                .is_some_and(|o| o.value().lovelace() >= FEE_BUFFER)
        })
        .take(3)
        .cloned()
        .collect();
    // FIXME
    // let specified_signatories = steppeds.specified_signatories();
    let specified_signatories = vec![];
    // FIXME
    // let bounds = steppeds.bounds();
    let bounds = Bounds {
        lower: None,
        upper: None,
    };

    let to_slot = |d: Duration| network_parameters.protocol_parameters.posix_to_slot(*d);

    let lower_bound = bounds
        .lower
        .map_or(SlotBound::None, |d| SlotBound::Inclusive(to_slot(d)));
    let upper_bound = bounds
        .upper
        .map_or(SlotBound::None, |d| SlotBound::Exclusive(to_slot(d)));

    // FIXME
    // let utxos = steppeds
    //     .utxos()
    //     .iter()
    //     .chain(fuel.iter())
    //     .map(|t| (t.0.clone(), t.1.clone()))
    //     .chain(reference_utxo.iter().map(|i| (i.0.clone(), i.1.clone())))
    //     .collect::<BTreeMap<_, _>>();
    let utxos = fuel
        .iter()
        .map(|t| (t.0.clone(), t.1.clone()))
        .chain(reference_utxo.iter().map(|i| (i.0.clone(), i.1.clone())))
        .collect::<BTreeMap<_, _>>();
    Transaction::build(
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
    .map_err(|e| TxError::BuildError(e.to_string()))
}

#[derive(Debug, Clone, thiserror::Error)]
pub enum TxError {
    /// The tx has a spend, but no reference script provided.
    #[error("Script required but none found")]
    ScriptMissing,
    /// The tx has a spend, but no reference script provided.
    #[error("Failed to build tx {0}")]
    BuildError(String),
    /// The tx has a spend, but no reference script provided.
    #[error("Insufficient fuel to cover requirements")]
    Fuel(#[from] SelectError),
}
