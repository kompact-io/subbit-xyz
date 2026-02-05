import { env } from './env.js';

test('description', () => {
	expect(Object.keys(env).length).toBe(6);
});
