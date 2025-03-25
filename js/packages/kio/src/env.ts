import "dotenv/config";

export const envLabel = "KOMPACT_";
export const walletLabel = "WALLET_";
export const blockfrostLabel = "BLOCKFROST_";

// Handling secrets

export const env = Object.fromEntries(
  Object.entries(Object(process.env))
    .filter((entry: [string, any]) => entry[0].startsWith(envLabel))
    .map((entry: [string, any]) => [entry[0].slice(envLabel.length), entry[1]]),
);

export const walletNames = Object.keys(env)
  .filter((x) => x.startsWith(walletLabel))
  .map((x) => x.slice(walletLabel.length).toLowerCase());

export function privateKeyKeyName(agent: string) {
  return `${walletLabel}${agent.toString().toUpperCase()}`;
}

export function privateKey(agent: string): string {
  return env[privateKeyKeyName(agent)]!;
}

export function blockfrostKeyKeyName(network: string) {
  return `${blockfrostLabel}${network.toString().toUpperCase()}`;
}

export function blockfrostKey(network: string): string {
  return env[blockfrostKeyKeyName(network)]!;
}
