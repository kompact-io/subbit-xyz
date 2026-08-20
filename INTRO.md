# Subbit-rs

A rust re-write of the subbit stack.

TBD: Merge into README.md

## Intro

See [subbit.xyz](https://subbit.xyz).

## Usage and orientation

### cardano-session

Provider needs to ensure Consumer's payments are **backed**. That is, there is a
UTxO from which the payment can be claimed.

To get started we need to be able to connect to the chain and submit txs. A
`cardano-session` allows tx submission by pairing a _cardano connector_ with a
wallet (ie signing key).

Currently only the Blockfrost connector is available (come back soon for more).

```sh
cargo run --bin cardano-session-cli
```

Init a config

```sh
cargo run --bin cardano-session-cli -- init
```

Change cardano connector (blockfrost) variables. Ensure the network aligns with
the configuration. Change the default wallet key. (See below if you need help
generating new keys.)

Get wallet status.

```sh
cargo run --bin cardano-session-cli -- wallet
```

Fund from faucette or otherwise. This concludes the cardano-session setup.

Note that the wallet is stored as plain text. Take care.

### subbit-cli

The subbit-cli wraps a cardano-session with a subbit-tx builder and related
helpers.

```sh
cargo run --bin subbit-cli --help
```

#### Init

Init a config

```sh
cargo run --bin subbit-cli -- init
```

This requires some of the same config from cardano-session. Fill in those bits.

#### Keyring

The keyring is stored in plain text. Take care.

Subbit needs keys. Every subbit channel datum has:

- an `iou` verifying key
- a `consumer` verification key hash
- a `provider` verification key hash

For testing we can generate the required values of a channel:

```sh
cargo run --bin subbit-cli -- keyring generate iou
cargo run --bin subbit-cli -- keyring generate consumer
cargo run --bin subbit-cli -- keyring generate provider
```

See all these with

```sh
cargo run --bin subbit-cli -- keyring generate list
```

You can use the keyring to generate wallet keys (see cardano-session).

#### Session

A subbit-session is a thin wrapper of a cardano-session.

```sh
cargo run --bin subbit-cli -- session
```

Run `status` to see wallet status including whether script was uploaded. It will
also list channels at the relevant addresses. (TODO: at present the script is
automatically uploaded on `status`. This should not be the default behaviour)

Upload the script if not already exists. (Alternatively we can point at one
already uploaded and at tip).

#### Tx

We now have enough info to create a channel.

For that we need to create a transaction with an `open`.

Transaction building is iterative. First a transaction is staged.

```sh
cargo run --bin subbit-cli -- tx stage
```

Then open with

```sh
cargo run --bin subbit-cli --tx open
```

This will launch in `interactive` mode. (TODO: establish non-interactive mode).
Fill in the details using the keyring or otherwise.

Tx building accepts multiple opens (subject to tx limits).

```sh
cargo run --bin subbit-cli -- tx submit
```

Which will (hopefully) see the channels appear on chain.

Channels can now be interacted with.

```sh
cardano run --bin subbit-cli -- tx stage
cardano run --bin subbit-cli -- tx  propose
```

(TODO: The options require a bit of work but should be navigable. Error handling
is ... to be improved.) Multiple proposes (at most one per input) can be
included in a single tx along side opens.

#### IOUs

Use

```sh
cardano run --bin subbit-cli -- iou
```

to generate (valid) IOUs required for `sub` and `settle`.

### subbit-server

Subbit server contains Provider's main L2 logic. It knows almost nothing about
the L1 and relies on other services to liaise with the L1.

It is agnostic as to the framework in which it can be embedded. Some use cases
require tight coupling between the application and cost computation. Others can
be loosely coupled. Subbit-server comes with an axum server wrapper to be run as
an external service.

As with other components, begin by generating an example config

```sh
cardano run --bin subbit-server -- init
```

Note that this uses `subbit-config` which provides an opinionated way to overlay
secrets. The only `key` that may need to be generated and kept secret is the
`mac_key` used in HMAC auth.

### subbit-index

The subbit index informs the server of what accounts have backing.

There are currently two modes: `mock` or `naive`.

- `mock` expects a file as input (see help for the format).
- `naive` reads the chain (thus requires a cardano-connector configured).

The reason `naive` is naive is that it currently has no nuanced handling of
rollbacks or the ability to configure settlement time. A less naive solution
would consider some number of blocks before considering the channel backed.

In addition, it is decoupled from transaction submission. This means that a
naive index may report a channel as backed, even though it has now been settled.
Allowing sufficiently long close periods, and time to settle relative to polling
intervals mitgates this.

Init and edit the config. Then, with the server already running:

```sh
cargo run --bin naive-index -- run
```

Transactions on channels will now result in changes in the state of
subbit-server db.

### subbit-issuer

The issuer runs client side, ie Consumer runs the issuer. It provides their L2
capabilities.

Explore the issuer in the way now established.

```sh
cargo run --bin subbit-issuer -- --help
```

Init and edit the config to align with the channel. This requires only the IOU
key of a channel.

The executable chooses the `cost` mechanic is based of pattern matching on URLs.
(See `UrlLookup`.) The cost mechanic can be configured freely.

## Example: Echo

### No subbit

In our example we have a server

```sh
cargo run --bin echo-server
```

Init a client

```sh
cargo run --bin echo-client -- init
```

And for now edit the config to remove the subbit component, and point at the
server:

```sh
# cat ./echo-client-config.toml
base_url = "http://127.0.0.1:3246"
```

The client can then send (post) a `--data <json>`, for example

```sh
cargo run --bin echo-client -- send --data @example.json
```

So far so good.

### With subbit

Provider now wishes to be remunerated for the service of echoing.

Setup a reverse proxy. Init a config.

```sh
cargo run --bin echo-proxy -- init
```

By default the proxy is listening on the port number of the echo server
_reversed_. It will also be expecting subbit server to up.

Delete the old echo-client config, init a new one. This time fill in the details
with a valid, backed channel. Ensure that the client is now pointing at the
echo-proxy (and _not_ echo-server).

(TODO: in future, grab the cost from the subbit-server via the reverse proxy as
part of initial handshake).

Echo client is now adding a subbit header on each request, and expecting one on
each response. These contain, amongst onther things IOUs, from the client.

Echo proxy is forwarding these to the subbit server before the request reaches
the echo server. If subbit-server deems the ious is good, the request proceeds,
else the request fails.

(TODO: automate the tx building for Provider to claim with IOUs)

## TODOs

- [x] rationalize configs (use figment)
- [x] extend echo client to use issuer.
- [ ] subbit server expose details required for pairing.
- [ ] have a naive tx submitter.
- [ ] documentation outlining the stack.
- [ ] rationalization of core.

## Rationalization of core

What we require is that components that talk over the wire share wire
definitions.

Nothing builds a `Datum` without also submitting a tx? So `Datum` can appear in
`subbit-tx`?

The server was multiple APIs:

- `x` executable `o`: used by issuer
- `a` admin: used by index and tx-submitter.

A more optimal crate arrangement?

```
tx
l1 - sessions? / cli
l2-wire - cli?
issuer
server
client
backing-wire
index
tx-service
conformance
integration
```
