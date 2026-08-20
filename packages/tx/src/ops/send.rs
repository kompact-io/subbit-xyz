use cardano_sdk::{
    Address, ChangeStrategy, Output, ProtocolParameters, Transaction, Value, address,
    transaction::state::ReadyForSigning,
};

use crate::Utxos;

pub fn tx(
    protocol_parameters: &ProtocolParameters,
    // All utxos will be spent
    sender_utxos: &Utxos,
    receivers: Vec<(Address<address::kind::Any>, Value<u64>)>,
    // Anything left over is sent to the change address
    change_address: Address<address::kind::Any>,
) -> anyhow::Result<Transaction<ReadyForSigning>> {
    let outputs = receivers
        .into_iter()
        .map(|(a, v)| Output::new(a, v))
        .collect::<Vec<_>>();
    let inputs = sender_utxos.keys().map(|input| (input.clone(), None));
    Transaction::build(protocol_parameters, sender_utxos, |tx| {
        tx.with_inputs(inputs.to_owned())
            .with_outputs(outputs.to_owned())
            .with_change_strategy(ChangeStrategy::as_last_output(change_address.to_owned()))
            .ok()
    })
}
