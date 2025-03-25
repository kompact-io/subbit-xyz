# Cli and tests

See the [spec](../../../docs/design/l1-spec.md), and the
[blog post](https://subbit.xyz/posts/building-txs.html) for more context.

## Setup

This is part of multipackage node repo. Usual installation applies.

Txs require ED25519 keys. This repo expect a `.env` file to provide the signing
keys, along with other secrets. See the `.env.example` for expected values.

To generate a new key, there is a helper script from `../kio`

```bash
pnpm run mkKey >> ../jobs/.env
```

Fund the admin wallet from the faucet. To get the address see the `show` command
below. Then request funds from
[here](https://docs.cardano.org/cardano-testnets/tools/faucet).

## cli

The cli exposes a way to run txs and inspect the chain state.

```bash
alias subbit="node ./dist/src/index.js"
```

Getting help is simple

```bash
subbit --help
```

This works also with subcommands. For example

```bash
subbit tx --help
```

To see wallet info

```bash
subbit show wallets
```

### NOTES

#### Patching lucid-evo

https://github.com/Anastasia-Labs/lucid-evolution/issues/534

If this is not fixed, it might have to be done manually
