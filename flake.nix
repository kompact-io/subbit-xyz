{
  description = "Subbit.xyz : Cardano's featherweight L2";

  inputs = {
    flake-parts.url = "github:hercules-ci/flake-parts";
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    git-hooks-nix = {
      url = "github:cachix/git-hooks.nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    treefmt-nix = {
      url = "github:numtide/treefmt-nix";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-overlay = {
      url = "github:oxalica/rust-overlay";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    rust-flake = {
      url = "github:juspay/rust-flake";
      inputs.rust-overlay.follows = "rust-overlay";
    };
    aiken.url = "github:aiken-lang/aiken";
  };

  outputs = inputs @ {
    self,
    flake-parts,
    ...
  }:
    flake-parts.lib.mkFlake {inherit inputs;}
    {
      imports = [
        inputs.git-hooks-nix.flakeModule
        inputs.treefmt-nix.flakeModule
        inputs.rust-flake.flakeModules.default
        inputs.rust-flake.flakeModules.nixpkgs
      ];
      systems = ["x86_64-linux" "aarch64-darwin"];
      perSystem = {
        lib,
        config,
        pkgs,
        inputs',
        ...
      }: let
        clang-unwrapped = pkgs.llvmPackages_latest.clang-unwrapped;

        gitHash = self.shortRev or self.dirtyShortRev or "unknown";
        devShell = {
          name = "subbit-xyz-shell";
          shellHook = ''
              ${config.pre-commit.installationScript}
            echo 1>&2 "Welcome to the development shell!"
              export RUST_SRC_PATH="${config.rust-project.toolchain}/lib/rustlib/src/rust/library";
          '';
          # Fixed using lib.concatMap and lib.attrValues to safely flatten the lists
          nativeBuildInputs =
            [
              inputs'.aiken.packages.aiken
              pkgs.cbor-diag
              config.treefmt.build.wrapper
              # RUST
              pkgs.openssl
              config.rust-project.toolchain
              clang-unwrapped
              pkgs.cargo-machete
              # PRE-COMMIT
              pkgs.prek
              pkgs.jless
            ]
            ++ lib.concatMap (crate: crate.crane.args.nativeBuildInputs) (lib.attrValues config.rust-project.crates);

          buildInputs =
            [
              pkgs.libiconv
            ]
            ++ lib.concatMap (crate: crate.crane.args.buildInputs) (lib.attrValues config.rust-project.crates);

          CC_wasm32_unknown_unknown = lib.getExe' clang-unwrapped "clang";
        };
      in {
        rust-project = {
          src = lib.fileset.toSource {
            root = ./.;
            fileset = lib.fileset.unions [
              (config.rust-project.crane-lib.fileset.commonCargoSources ./.)
              ./aiken/plutus.json
              ./packages/tx/plutus.json
            ];
          };
          defaults.perCrate.crane.args = {
            GIT_HASH = gitHash;
          };
          crates = {
            "subbit-examples-echo-proxy" = {
              path = ./examples/echo/proxy;
              crane.args.nativeBuildInputs = [
                pkgs.cmake
                pkgs.pkg-config
                pkgs.perl
              ];
            };
          };
        };
        treefmt = {
          projectRootFile = "flake.nix";
          flakeFormatter = true;
          settings.excludes = ["treefmt.toml" ".pre-commit-config.yaml"];
          programs = {
            prettier.enable = true;
            alejandra.enable = true;
            rustfmt.enable = true;
            aiken.enable = true;
            taplo.enable = true;
          };
        };

        pre-commit = let
          nixPrekConfig = ".nix-prek-config.yaml";
          precommitConfig = ".pre-commit-config.yaml";
          treefmtConfig = "treefmt.toml";
        in {
          # clippy checks are failing `nix flake check`
          # However, they come from rust-flakes, and our implicit workspace
          # makes it awkward to turn these off
          check.enable = false;
          settings = {
            package = pkgs.prek;
            configPath = nixPrekConfig;
            hooks = {
              treefmt.enable = true;
              nix-sync = {
                enable = true;
                name = "nix-sync";
                description = "Copy nix-generated prek config to committed ${precommitConfig}. This strips the nixstore dependencies";
                entry = ''
                  sh -c '
                    if [ -f ${nixPrekConfig} ]; then
                      # PRE-COMMIT
                      grep -v "^#" ${nixPrekConfig} | jq ".repos[].hooks[].entry |= gsub(\"/nix/store/[^/]+/bin/\"; \"\")" > ${precommitConfig}
                      git add ${precommitConfig}
                      # TREEFMT
                      sed "s|/nix/store/[^/]*/bin/||g" ${config.treefmt.build.configFile} > treefmt.toml
                      git add ${treefmtConfig}
                    fi
                  '
                '';
                pass_filenames = false;
                always_run = true;
              };
              # Transitive deps mean default clippy ends up using a different cargo.
              my-clippy = {
                enable = true;
                name = "clippy";
                description = "Run clippy";
                entry = "${config.rust-project.toolchain}/bin/cargo-clippy -- --manifest-path Cargo.toml";
                pass_filenames = false;
              };
              cargo-machete = {
                enable = true;
                name = "cargo-machete";
                description = "Check for unused dependencies";
                entry = "${pkgs.cargo-machete}/bin/cargo-machete ./";
                files = "\\.toml$";
                pass_filenames = false;
              };
            };
          };
        };
        devShells = {
          default = pkgs.mkShell devShell;
        };
      };
      flake = {
      };
    };
}
