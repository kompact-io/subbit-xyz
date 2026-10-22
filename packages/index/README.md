# Index

Roles and responsibilities

The service reports Channel **lineage**.
From this, the server chooses a Backing for the Channel,
and also records the _safe to settle_ Utxos.

## Design assumptions

Index service is treated as stateless, although may cache data as an optimization.

The data source:

- Sees all relevant txs
- Fully resolves txs

Thus from a tx we can establish "continuing" output from input-output ordering.

## Design considerations

There are two ways the index report can be wrong about a Backing:

- Reporting it exists when it doesn't
- Reporting it doesn't exist when it does

The former is very bad - Provider is exposed to losing funds.
The latter is bad - Consumer does not receive service, but no funds at risk.
Thus the on any failure, the index should report `NoBacking` and require
intervention, not silent stall.

Likewise the server should cease serving if it has not heard from the index service.

## Lineage

A sequence of TXOs:

```sample
txo -> txo -> txo
```

Each transition is a Tx.

Txs can step many channels.

## Validation

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

Each tx is:

- `[ (input, option<output>) ]` - spends
- `[ output ]` - new

- `{ keytag : [ lineage ] }`
- `lineage = { constants, stake_credential, inner}`
- `inner =  [ { block_height, stage, amount } ]`

The service verifies that a TxO is well-formed.

- payment credential
- value is ada + maybe currency
- datum
  - own hash
  - constants: provider hash, long enough close period, currency
  - stage: well formed
- no script

Index hands over lineage: Server decides which to choose.
All other lines are "safe to settle".

For

Cases we must accommodate:

- spends on a UTxO
- rollbacks
- mimics

What happens if the service reports:

- a safe to settle,
- then there is a rollback

Either the Tx appears again or it does not. On the former case, it will be safe to settle.
In the latter it is unsettle-able.

A notion of continuity in the case of a mutual

Service should report : Current backing, and identify Output References that are _safe to
settle_.
A TxO is _safe to settle_ if it is not on the lineage of the current Backing.

Payload = { <keytag> : { backing , safe_to_settle : [ txo_ref ] } }
backing = { reserve, subbed }

What does safe to settle mean? And what can go wrong?

Under normal operation, there is a tx consuming the channel, and outputing a Closed UTxO.
The correspondence is unambiguous.

A tx creates relationships between Inputs and Outputs.
A parent (input) and child (output) relationship is established by:

_"Did the validator identify the child as the continuing output of the parent?"_

A lineage is a sequence of parent-child relationships of TxOs.

What is a transparent and robust way to establish relationships?
Rerunning the validator code?

For each keytag, there is a vec of lineages.
In normal operation, this vec has one element,
but we must accommodate the case of more than one.

If a lineage has an element in closed stage, then it is closed.
Otherwise is is open.

TxO is **settled** if it is beyond a certain blocked depth `D`.
The depth `D` is at providers discression.
The backing of lineage is decided by

t PLACEHOLDER : u8 = 0;
