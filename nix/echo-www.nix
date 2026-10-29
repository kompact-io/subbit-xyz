# Builds the echo demo's browser wasm bundle (`examples/echo/www/pkg/`)
# reproducibly, for `subbit-examples-echo-proxy`'s `include_dir!` embed.
#
# Deliberately skips `wasm-pack` the CLI tool: it tries to network-fetch
# a version-matched `wasm-bindgen-cli` at build time, which fails in
# Nix's sandboxed build. Instead this drives the same two steps by hand:
# `cargo build --target wasm32-unknown-unknown`, then `wasm-bindgen-cli`
# (which must be the *exact* same version as the `wasm-bindgen` crate in
# `Cargo.lock` -- see ipetkov/crane#354 -- hence the workspace pins
# `wasm-bindgen = "=0.2.127"` to match nixpkgs' `wasm-bindgen-cli`).
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
    pname = "subbit-stamp-wasm";
    version = "0.0.0";
    cargoExtraArgs = "-p subbit-stamp --no-default-features --features wasm --target wasm32-unknown-unknown";
    doCheck = false;
    CC_wasm32_unknown_unknown = ccWasm32;
    installPhaseCommand = ''
      mkdir -p $out/lib
      cp target/wasm32-unknown-unknown/release/subbit_stamp.wasm $out/lib/
    '';
  };
  cargoArtifacts = craneLib.buildDepsOnly wasmArgs;
  wasmCrate = craneLib.buildPackage (wasmArgs // {inherit cargoArtifacts;});
in
  pkgs.runCommand "echo-www-wasm" {
    nativeBuildInputs = [wasmBindgenCli];
  } ''
    mkdir -p $out/pkg
    wasm-bindgen --target web --out-dir $out/pkg --out-name subbit_stamp \
      ${wasmCrate}/lib/subbit_stamp.wasm
  ''
