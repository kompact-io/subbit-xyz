import { describe, test } from '@jest/globals';
import * as kio from '@subbit-tx/kio';
import { setup } from './setup.js';
import { job } from './simple.js';

describe('simple', () => {
	test('simple', async () => {
		const l = await kio.mkLucid.mkLucidWithEmulator();
		await setup(l);
		await job(l);
	});
});
