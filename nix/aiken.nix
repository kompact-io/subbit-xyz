# Bridges `aiken/aiken.lock` to Nix, the same job `importCargoLock`/
# crane's `vendorCargoDeps` does for `Cargo.lock`: no flake input, no
# separate Nix-side lock, no hand-typed table. Just `aiken.lock` itself,
# read directly, the same way `Cargo.lock` is for Rust.
#
# This only works because every dependency here is pinned to a full
# git commit SHA, not a tag: `aiken packages upgrade --version <sha>
# <package>` writes that SHA into both `aiken.toml` and
# `aiken.lock` (confirmed directly from `aiken packages upgrade
# --help`: "a git commit hash, a tag or a branch name" are all
# accepted). A commit SHA is the one thing `builtins.fetchGit` trusts
# without a separate content hash (see the Nix manual on `fetchGit`'s
# `rev`) -- it's already a content-derived identity, the way a tag or
# branch name never is. So this fetches exactly the way crane fetches a
# git dependency from `Cargo.lock` (confirmed against the real
# derivation graph: `deriver: null`, fetched at evaluation time, never
# a declared flake input) -- `aiken.lock`'s `version` just needs to be
# a commit, which is now enforced by keeping every dependency pinned
# that way rather than by a tag.
{
  lib,
  pkgs,
  aikenBin,
  projectDir,
  lockFile,
}: let
  lock = builtins.fromTOML (builtins.readFile lockFile);

  dirName = pkg: builtins.replaceStrings ["/"] ["-"] pkg.name;

  isCommitSha = rev: builtins.match "[0-9a-f]{40}" rev != null;

  resolvePkg = pkg:
    if !(isCommitSha pkg.version)
    then
      throw ''
        ${toString lockFile} pins "${pkg.name}" at "${pkg.version}", which isn't
        a full 40-character commit SHA -- Nix can't fetch it without a
        separate content hash. Pin it to a commit instead:
          aiken packages upgrade --version <commit-sha> ${pkg.name}
        (then `aiken check` once, with network, to refresh the lock).
      ''
    else {
      name = dirName pkg;
      source = builtins.fetchGit {
        url = "https://github.com/${pkg.name}.git";
        rev = pkg.version;
      };
    };

  resolved = map resolvePkg lock.packages;

  packagesToml =
    pkgs.writeText "packages.toml"
    (lib.concatMapStringsSep "\n\n" (pkg: ''
        [[packages]]
        name = "${pkg.name}"
        version = "${pkg.version}"
        source = "${pkg.source}"
      '')
      lock.packages);

  # Shared by both checks below: a writable copy of the project with
  # its dependencies' local package cache populated, exactly what
  # `aiken build`/`aiken check` otherwise populate themselves against
  # the network.
  setup = ''
    cp -r ${projectDir} ./aiken
    chmod -R u+w ./aiken
    mkdir -p ./aiken/build/packages
    ${lib.concatMapStringsSep "\n" (r: ''
        cp -r ${r.source} ./aiken/build/packages/${r.name}
      '')
      resolved}
    cp ${packagesToml} ./aiken/build/packages/packages.toml
    chmod -R u+w ./aiken/build/packages
  '';
in {
  # Validates every `aiken.lock` entry is pinned to a commit SHA, with
  # no network access and no `aiken` invocation. Catches a
  # tag-pinned dependency here, fast, rather than only once `check`
  # (below) tries to fetch it.
  lockIsNixCompatible =
    pkgs.runCommand "aiken-lock-is-nix-compatible" {
      # Forces every `resolvePkg` call above.
      resolvedNames = builtins.toJSON (map (r: r.name) resolved);
    } ''
      echo "$resolvedNames" > $out
    '';

  check =
    pkgs.runCommand "aiken-check" {
      nativeBuildInputs = [aikenBin];
    } ''
      ${setup}
      cd ./aiken
      aiken check
      touch $out
    '';

  # `aiken build` regenerates `plutus.json` from the validator sources;
  # nothing re-derives it from `plutus.json` itself (`subbit-tx` just
  # reads the committed file). So a validator change that isn't
  # followed by a real `aiken build` + commit leaves a stale blueprint
  # sitting there, silently -- this catches exactly that, by rebuilding
  # in the sandbox and diffing against what's actually committed.
  plutusJsonIsUpToDate =
    pkgs.runCommand "aiken-plutus-json-is-up-to-date" {
      nativeBuildInputs = [aikenBin];
    } ''
      ${setup}
      cp ./aiken/plutus.json ./committed-plutus.json
      (cd ./aiken && aiken build)

      # `aiken build`'s own output has no trailing newline; the
      # committed file does (whatever last touched it added one) --
      # normalize both to exactly one before comparing, so that's not
      # mistaken for a real drift.
      normalize() { printf '%s\n' "$(cat "$1")"; }
      if ! diff -u <(normalize ./committed-plutus.json) <(normalize ./aiken/plutus.json); then
        echo "error: aiken/plutus.json is out of date with the validator sources." >&2
        echo "Run 'aiken build' in aiken/ and commit the regenerated plutus.json." >&2
        exit 1
      fi
      touch $out
    '';
}
