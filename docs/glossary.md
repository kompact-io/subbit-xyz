# Subbit.xyz Glossary

## About

A simple way to collect terms in one place that we use across the project.

This is exclusively for terms that are used in way with distinct or precise
meaning, not shared by an established context. For example, it includes 'subbit'
and 'account', but not 'utxo' ...accept we do for style guidelines purposes.

Insert a new term in its alphabetic order. Prefer:

- lower case by default although upper case is allowed.
- verbs in their infinitive (without 'to')

In each entry, link the first occurrence of mentioned terms with relative
anchors. Assume that the anchor ref is header with all punctuation and spaces
replaced by single hyphen characters `-`.

(This glossary format was originally developed for
[cardano-lightning](https://github.com/cardano-lightning/cardano-lightning/blob/main/docs/glossary.md),
with which Subbit.xyz share much in common.)

## Terms

### account

The [funds](#funds) of a [subbit](#subbit) attributed to one of its
[participants](#participant). Typically this is represented by a single integer,
since channels are mono-asset.

### add

A [step](#step) on an [opened](#opened) [subbit](#subbit) by the
[consumer](#consumer) that adds funds of one of the [accounts](#account). Note
that this step does not change the stage.

### amount

The preferred term for an integer representing an amount of [funds](#funds). It
is preferred over alternatives such as `value` and `quantity`.

### bit

In relation to service, a bit of service is quanta of service for which the
consumer recompenses the provider with an [iou](#iou). How much each bit costs
is described in the [tos](#tos).

### close

A [step](#step) that changes the [stage](#stage) from [opened](#opened) to
[closed](#closed). It is performed by the [consumer](#consumer) who wishes to
end the [subbit](#subbit).

### close period

The time period that must pass between [close](#close) and a [drain](#drain).
The provider must check the L1 at time intervals smaller than the closer period
not to miss the opportunity to do a final [sub](#sub).

### closed

The second [stage](#stage). It occurs after a [close](#close) step. The
participants are no longer transacting off-chain (at least for long).

### consumer

One of the [subbit](#subbit) [participant](#participant)s, the other being the
[provider](#provider). The consumer is consuming the service, and paying for it
via Subbit.xyz. The consumer sends [iou](#iou)s to the provider for each
[bit](#bit) of service. The consumer does the [step](#step)s [open](#open),
[add](#add), [close](#close), [end](#end) and [expire](#expire).

### currency

In relation to a [subbit](#subbit), the asset class which is being exchanged.
The default currency is ada.

### eol

"End of life" [step](#step)s of a [subbit](#subbit).
These are: [close](#close), [settle](#settle), [expire](#expire), [end](#end).

### end

A terminal [step](#step) in which the [consumer](#consumer) recovers the
remaining funds of a [settled](#settled) [subbit](#subbit).

### expire

A terminal [step](#step) in which the [consumer](#consumer) recovers the
remaining funds of a [closed](#closed) [subbit](#subbit) in which the
[close period](#close-period) has expired. This may happen if the provider has
failed to [settle](#settle).

### funds

The preferred term for amount of assets in the subbit that are locked as
collateral on the L1. Use the term 'funds' over alternatives such as 'value',
'assets', 'tokens', _etc_.

### L1

Shorthand for layer one ie the Cardano blockchain. It can also be used to refer
to the part of the Subbit.xyz protocol that takes place on the Cardano
blockchain, such as subbit utxos and txs that step channels.

### L2

Shorthand for layer two, also called 'off-chain transacting'. Characterised by
simply 'not L1', it includes messages passed between subbit participants such as
[iou](#iou)s.

### lifecycle

In relation to a [subbit](#subbit), it is the collection of [stage](#stage)s
that are arrived at be a sequence of [step](#step)s.

(This term is include mainly to document that the preferred form is as a single
word.)

### iou

The key data object exchanged on the L2 in relation to demonstrating funds owed.
It is attached to each [bit](#bit) of service. It is used on the L1 to prove
what funds are owed, and can be subbed.

(More accurately, it may not be explicitly be exchanged on the L2, but is certainly derivable from whatever is.)

### iou body

The data object derived from an iou and a subbit id that forms the body that is signed, and verified against.

### open

A [step](#step) that [stages](#staged) [subbit](#subbit) as [opened](#opened).
The [consumer](#consumer) performing an open locks their initial funds and
indicates the credentials of the other participant.

### opened

The main [stage](#stage) of [subbit](#subbit). While the subbit is at this
stage, the [participants](#participant) are transacting off-chain.

### participant

Anyone using Subbit.xyz. In relation to a [subbit](#subbit), there are two
participants: a [consumer](#consumer) and a [provider](#provider).

### partner

In relation to a [subbit](#subbit), a synonym for [participant](#participant).
This is the preferred term from related projects such as bitcoin lightning.

### provider

One of the [subbit](#subbit) [participant](#participant)s, the other being the
[consumer](#consumer). The provider is providing the service, and is recompensed
for it via Subbit.xyz. The provider receives [iou](#iou)s from the consumer for
[bit](#bit) of service. The provider does the [step](#step) [sub](#sub), and
[settle](#settle).

### settle

A [step](#step) done by the [provider](#provider) to a [closed](#closed)
[subbit](#subbit). It is akin to [sub](#sub), but is effectively the ultimate
sub. The result is [settled](#settled) subbit.

### settled

From this [stage](#stage), the only eligible [step](#step) is a [drain](#drain).

### signing key

Ed25519 signing key. This is the preferred term over 'secret key' or 'private
key'.

### stage

A [subbit](#subbit) stage relates to it's L1 state. A subbit (that
[staged](#staged)), begins in a [opened](#opened) then later [steps](#step) to a
[closed](#closed) stage.

### staged

A [subbit](#subbit) is staged if it there is utxo that represents it on tip.
That is to say, it is staged if there is a utxo at tip representing its current
[stage](#stage). See also [unstaged](#unstaged)

### step

A Cardano transaction that either spends and/or outputs a utxo representing a
[subbit](#subbit) is said to step the subbit.

The term is used both for a specific step, and to mean a "type of step". For
example, we may say:

- "this tx steps this subbit" or
- "add is a step"

The steps are: [open](#open), [add](#add), [sub](#sub), [close](#close),
[settle](#settle), [expire](#expire), [end](#end).

### sub

A [step](#step) on an [opened](#opened) [subbit](#subbit) by the
[provider](#provider) that subs funds they are owed, and attested for by an
[iou](#iou). Note that this step does not change the stage.

### subbit

The barefaced re-branding of the more usual term "channel".

### tos

Cribbed from "terms of service", is a machine and human readable document that
describes to the [consumer](#consumer) how much the [provider](#provider)'s
service costs at the level of [bit](#bit)s.

### tx

Permissible shorthand for transaction. Plural: txs.

### unstaged

Any terminal [step](#step) ceases the subbit. A [subbit](#subbit) that is no
longer [staged](#staged) is ceased.

### utxo

Our preferred style of shorthand for unspent transaction output.

### verification key

Ed25519 verification key. This is the preferred term over 'public key'.
