import { describe, test } from '@jest/globals';
import * as lucid from '@lucid-evolution/lucid';
import * as kio from '@subbit-tx/kio';
import { setup } from './setup.js';
import * as batch from './batch.js';

describe('simple', () => {
	let l: lucid.LucidEvolution;
	let ref: lucid.UTxO;
	beforeAll(async () => {
		l = await kio.mkLucid.mkLucidWithEmulator();
		ref = await setup(l);
	});
	test('job', async () => {
		await batch.job(l, ref);
	});
});
