# Index

```sample
| cardano data source | <--> | index | --> | subbit-gate |
```

The index service pulls data from a cardano data source and reports Channel **lineage** to the subbit gate.
From this, the gate chooses a Backing for the Channel, and also records the _safe to settle_ Utxos.

## Design assumptions

- Index service is treated as stateless, although may cache data as an optimization.
- The data source: Sees all relevant txs; Fully resolves txs.
  Thus from a tx we can establish "continuing" output from input-output ordering.
- The index service is polling data a cadence much higher than the close period.

## Design considerations

There are two ways the index report can be wrong about a Backing:

- Reporting it exists when it doesn't
- Reporting it doesn't exist when it does

The former is very bad - Provider is exposed to losing funds.
The latter is bad - Consumer does not receive service, but no funds at risk.

Therefore on failure the index should report `NoBacking` and require intervention, not silent stall.

Moreover the gate should cease serving if it has not heard from the index service.

## Lineage

A sequence of TXOs:

```sample
txo -> txo -> txo
```

Each transition is a Tx.

Txs can step many channels.

## Validation

From the set of inputs or outputs.

For each tx:

Filter on payment cred of inputs and outputs

If there is only one input then it may be a mutual tx.
If there is more than one channel input, then it cannot be a mutual tx.

For single input test for non continuity of stake credential or constants or value or script.
If non-continuity, then assume mutual tx, and treat the input lineage a Ended.
Otherwise treat continuation of lineage despite not necessarily having validator assurance.

For multiple input case we can assume "normal" validator has been executed.
Any of the following sanity check failures should result in all lineages being deemed Ended.
This is a safty fallback.

- For each input, the datum is wellformed `(own_hash, constants, stage)`.
- Align with corresponding outputs, accounting for termination of channels:
  - Allow skip output if `stage = Closed{ elapse_at }` and `elapse_at` < `now`
  - Ensure skipt output if `stage = Settled`
- Verify well-formedness of continuing Outputs
- New outputs may be ill-formed, but ignore.

## Data

### Ingestion

- We are targeting `floe` as our backend.
- In future we may migrate to speaking `n2n` directly.

Floe speaks a variant of utxo-rpc, and supports grpc, http-json, http-cbor.
Floe racks the last chunk of the chain: this the tip, and some amount of depth.

It provides a query `SearchUtxos` by payment credential.
Really, its TxOs rather than UTxOs because it can return spent as well as unspent.
Subbit index service requires to some amount of depth to reconstruct lineage reliably.
Assumption: depth provided is greater than that which is considered "settled" (ie unrollbackable).

The default query is TxOs at payment credential equal to the subbit validator.

### Process

Under normal operation, partition by `(constants, stake_key)` will be sufficient to construct lineage.

Additional work is required to distinguish mimics:

1. Partition on transitive relation induced by `spent_by`.
2. Tie break by ordering.

The service verifies that a TxO is well-formed.

- payment credential
- value is ada + maybe currency
- datum
  - own hash
  - constants: provider hash, long enough close period, currency
  - stage: well formed
- no script

If a line contains a TxO that violates this, then it implies that this cannot be
the output of a (normally) validated tx.
We infer that this is either a "bad" open or a "bad" mutual.
The violating item, and its parents are dropped from the line.

### Output

We should output in two modes:

- Snapshot: Update all. Keytags missing implies terminated.
- Partial: Update some. Keytags missing does not imply terminated.

For now we implement only Snapshot mode.

```rust
struct Output {
    tip : Point,
    channels : Vec<Channel>, stake_credential , [lines],
}

/// Validator enforces that under a normal (non-mutual) spend, then the following
/// are equal between an input and its continuing output
struct ConstantsExt {
    constants: Constants,
    delegation: Option<Credential>,
}

struct Channel {
    constants_ext : ConstantsExt,
    lines : Vec<Line>,
}

struct Line(Vec<Item>)

struct Item {
    created_at : Point,
    txo_ref : OutputReference,

}

/// A variant of channel stage relevant to Backing.
enum Stagish {
    // Potential Channel backing
    Opened { amount : u64, subbed : u64 },
    // Channel (line) closed.
    Closed,
    // Channel (line) terminated in way other than close (ie mutual or data corruption/ assumption violation).
    Terminated,
}
```

### Downstream

An empty line is then treated as if `Terminated`.
A channel with no lines is treated as `Terminated`.
Re-activation will require intervention.

Gate chooses open line (ie not closed or terminated), item of settled depth for backing,
with greatest redeemable funds.
If there are none, then backing is `None`.

All closed `txo_ref` are recorded as `safe_to_settle`. Their lines are excluded as backing.
