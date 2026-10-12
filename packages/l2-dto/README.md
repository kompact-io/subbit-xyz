# l2-dto

The core part of subbit is agnostic to the transport.
Http is the primary target, but sneakernet will also work.

This crate provides the wire format for both server and client.
Components are common as much as is pragmatic: `Request`, `Commit`, `Terms`, `Response` live at
the crate root; a dialect re-exports what it needs unchanged, and only defines its own type where
its own wire shape actually differs.

Subbit has its native dialect, `vanilla`, and `x402` (v2).

## Adding a dialect

Not all dialects need the same API. Every dialect has its own `request`/`response` cycle; beyond
that, a dialect only gets its own `require`/`discover`-shaped type if its wire protocol actually
has a separate challenge/discovery step (x402 does; vanilla, today, doesn't).

`vanilla` (`./src/dialect/vanilla.rs`) is the minimal case: every submodule is a straight
re-export of a crate-root type, nothing wrapped. `x402` (`./src/dialect/x402.rs`) is the fuller
case: its own `require`/`request`/`response` wrap the crate-root types to carry the scheme's own
fields (`accepted`, `extra`, `extensions`, ...). Start from whichever is the closer fit.
