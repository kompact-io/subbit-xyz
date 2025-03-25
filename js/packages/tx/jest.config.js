/** @type {import('ts-jest').JestConfigWithTsJest} **/
export const testEnvironment = "node";
export const transform = {
  "^.+.tsx?$": ["ts-jest", {}],
  preset: "ts-jest",
};
export const testMatch = ["**/?(*.)test.ts"];
