import { env } from "./env";

test("description", () => {
  expect(Object.keys(env).length).toBe(4);
});
