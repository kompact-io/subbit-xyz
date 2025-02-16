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
        devShells.default = pkgs.mkShell {
          nativeBuildInputs = [
            config.treefmt.build.wrapper
          ];
          shellHook = ''
            ${config.pre-commit.installationScript}
            echo 1>&2 "Welcome to the development shell!"
          '';
          packages = [
            inputs'.aiken.packages.aiken
          ];
        };
      };
      flake = {};
    };
}
