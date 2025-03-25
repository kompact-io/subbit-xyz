import * as lucid from "@lucid-evolution/lucid";

export type BlockfrostConfig = {
  type: "BlockfrostConfig";
  url: string;
  key?: string;
};

export type KupmiosConfig = {
  type: "KupmiosConfig";
  kupoUrl: string;
  ogmiosUrl: string;
};

export type EmulatorConfig = {
  type: "EmulatorConfig";
  accounts: lucid.EmulatorAccount[];
  protocol: lucid.ProtocolParameters;
};

export type ProviderConfig = BlockfrostConfig | KupmiosConfig | EmulatorConfig;

export function mkProvider(config: ProviderConfig) {
  if (config.type === "BlockfrostConfig") {
    return new lucid.Blockfrost(config.url, config.key || "defaultKey");
  } else if (config.type === "KupmiosConfig") {
    return new lucid.Kupmios(config.kupoUrl, config.ogmiosUrl);
  } else if (config.type === "EmulatorConfig") {
    return new lucid.Emulator(config.accounts, config.protocol);
  }
  throw new Error("Not implemented");
}

export function blockfrostDefaultUrl(network: lucid.Network): string {
  const subdomain: Record<string, string> = {
    Mainnet: "cardano-mainnet",
    Preprod: "cardano-preprod",
    Preview: "cardano-preview",
  };
  return `https://${subdomain[network]}.blockfrost.io/api/v0`;
}
