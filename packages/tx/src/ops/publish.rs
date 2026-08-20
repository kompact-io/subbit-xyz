use cardano_sdk::{
    Address, ChangeStrategy, Output, PlutusScript, ProtocolParameters, Transaction, address,
    transaction::state::ReadyForSigning,
};

use crate::Utxos;

pub fn tx(
    protocol_parameters: &ProtocolParameters,
    utxos: &Utxos,
    script: PlutusScript,
    host_address: Address<address::kind::Any>,
    change_address: Address<address::kind::Any>,
) -> anyhow::Result<Transaction<ReadyForSigning>> {
    let outputs = vec![Output::to(host_address).with_plutus_script(script)];

    let inputs = utxos.keys().map(|input| (input.clone(), None));
    Transaction::build(protocol_parameters, utxos, |tx| {
        tx.with_inputs(inputs.to_owned())
            .with_outputs(outputs.to_owned())
            .with_change_strategy(ChangeStrategy::as_last_output(change_address.to_owned()))
            .ok()
    })
}
