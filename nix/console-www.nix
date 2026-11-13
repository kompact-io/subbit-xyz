# Builds `subbit-console`'s wasm bundle (gated by `--target
# wasm32-unknown-unknown` itself, not a Cargo feature -- see
# `src/ui.rs`'s own doc, plan-subbit-console.md sections A/E-J) for
# `packages/console/www/`.
# Same hand-rolled crane + `wasm-bindgen-cli` pattern as
# `nix/echo-www.nix` (see its own doc for why `wasm-pack` is avoided).
#
# Just one package now, not two: `subbit-tx` no longer has its own
# `wasm` feature at all -- it was absorbed into `subbit-console`'s own
# `Basket` (a direct Rust dependency on `subbit-tx`, not JSON at a wasm
# boundary), so a native wrapper and the browser share one core instead
# of `subbit-tx` growing a second, browser-only implementation. See
# `packages/console/src/basket.rs`'s own module doc.
#
# REVISED: no second bundle, no new flake input needed after all.
# `floe-client` is now a direct, bare (`default-features = false`)
# dependency of `subbit-console` itself -- its own `wasm` feature
# (which pins a conflicting `wasm-bindgen`) is never enabled; Console
# supplies its own `Backend` impl over `fetch` instead
# (`packages/console/src/wasm/transport.rs`), under *our* pinned
# `wasm-bindgen`/`web-sys`. One bundle, one `Cargo.lock`.
{
  lib,
  pkgs,
  craneLib,
  src,
  wasmBindgenCli,
  ccWasm32,
}: let
  wasmArgs = {
    inherit src;
    pname = "subbit-console-wasm";
    version = "0.0.0";
    cargoExtraArgs = "-p subbit-console --no-default-features --target wasm32-unknown-unknown";
    doCheck = false;
    CC_wasm32_unknown_unknown = ccWasm32;
    installPhaseCommand = ''
      mkdir -p $out/lib
      cp target/wasm32-unknown-unknown/release/subbit_console.wasm $out/lib/
    '';
  };
  cargoArtifacts = craneLib.buildDepsOnly wasmArgs;
  consoleWasm = craneLib.buildPackage (wasmArgs // {inherit cargoArtifacts;});
in {
  inherit consoleWasm;
  combined =
    pkgs.runCommand "console-www-wasm" {
      nativeBuildInputs = [wasmBindgenCli];
    } ''
      mkdir -p $out/subbit-console
      wasm-bindgen --target web --out-dir $out/subbit-console --out-name subbit_console \
        ${consoleWasm}/lib/subbit_console.wasm
    '';
}
