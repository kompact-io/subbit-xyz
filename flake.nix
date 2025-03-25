{
  description = "Subbit.xyz : Cardano's featherweight L2";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    git-hooks-nix.url = "github:cachix/git-hooks.nix";
    git-hooks-nix.inputs.nixpkgs.follows = "nixpkgs";
    treefmt-nix.url = "github:numtide/treefmt-nix";
    treefmt-nix.inputs.nixpkgs.follows = "nixpkgs";
    aiken.url = "github:waalge/aiken/waalge/fix-nix-build";
  };

  outputs = inputs @ {flake-parts, ...}:
    flake-parts.lib.mkFlake {inherit inputs;}
    {
      imports = [
        inputs.git-hooks-nix.flakeModule
        inputs.treefmt-nix.flakeModule
      ];
      systems = ["x86_64-linux" "aarch64-darwin"];
      perSystem = {
        config,
        self',
        inputs',
        pkgs,
        system,
        ...
      }: {
        treefmt = {
          projectRootFile = "flake.nix";
          flakeFormatter = true;
          programs = {
            prettier = {
              enable = true;
              settings = {
                printWidth = 80;
                proseWrap = "always";
              };
            };
            alejandra.enable = true;
          };
        };
        pre-commit.settings.hooks = {
          treefmt.enable = true;
          aiken = {
            enable = true;
            name = "aiken";
            description = "Run aiken's formatter on ./aik";
            files = "\\.ak";
            entry = "${inputs'.aiken.packages.aiken}/bin/aiken fmt ./aik";
          };
        };
        devShells.default = let
          mk-blueprint =
            pkgs.writeShellScriptBin "mk-blueprint"
            ''
              #!/usr/bin/env bash
              root=$(git rev-parse --show-toplevel)
              aik=$root/aik
              out=$root/js/packages/tx/src/blueprint.ts
              bp=$root/js/node_modules/blueprint-ts/dist/src/index.js
              if [ ! -f $bp ]; then
                echo "blueprint-ts script not found"
                echo "expected ($bp)"
                echo "Maybe it needs to be installed? cd js; pnpm -wD i"
                exit 1
              fi
              aiken build $@ $aik
              node $bp --input $aik/plutus.json --output $out
            '';
        in
          pkgs.mkShell {
            nativeBuildInputs = [
              config.treefmt.build.wrapper
            ];
            shellHook = ''
              ${config.pre-commit.installationScript}
              echo 1>&2 "Welcome to the development shell!"
            '';
            packages = [
              inputs'.aiken.packages.aiken
              pkgs.nodePackages_latest.nodejs
              pkgs.pnpm
              pkgs.typescript-language-server
              mk-blueprint
            ];
          };
      };
      flake = {};
    };
}
