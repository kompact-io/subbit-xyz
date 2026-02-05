import * as lucid from '@lucid-evolution/lucid';
import { sumUtxos } from '../lucidExtras.js';

export async function tx(
	l: lucid.LucidEvolution,
	target: string | null = null
): Promise<lucid.TxBuilder> {
	const target_ =
		target == null
			? lucid.credentialToAddress(l.config().network!, lucid.keyHashToCredential('0'.repeat(56)))
			: target;
	const allUtxos = await l.wallet().getUtxos();
	const na = sumUtxos(allUtxos);
	na['lovelace'] = 0n;
	const tx = l.newTx().collectFrom(allUtxos).pay.ToAddress(target_, na);
	return tx;
}
