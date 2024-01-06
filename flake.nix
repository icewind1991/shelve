{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-23.11";
    flake-utils.url = "github:numtide/flake-utils";
    naersk.url = "github:nix-community/naersk";
    naersk.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.url = "github:oxalica/rust-overlay";
    rust-overlay.inputs.nixpkgs.follows = "nixpkgs";
    rust-overlay.inputs.flake-utils.follows = "flake-utils";
    cross-naersk.url = "github:icewind1991/cross-naersk";
    cross-naersk.inputs.nixpkgs.follows = "nixpkgs";
    cross-naersk.inputs.naersk.follows = "naersk";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    naersk,
    rust-overlay,
    cross-naersk,
  }:
    flake-utils.lib.eachDefaultSystem (
      system: let
        overlays = [
          (import rust-overlay)
          (import ./overlay.nix)
        ];
        pkgs = (import nixpkgs) {
          inherit system overlays;
        };
        inherit (pkgs) lib callPackage rust-bin mkShell;
        inherit (lib.sources) sourceByRegex;
        inherit (builtins) fromTOML readFile map;

        msrv = (fromTOML (readFile ./Cargo.toml)).package.rust-version;
        toolchain = rust-bin.stable.latest.default;
        msrvToolchain = rust-bin.stable."${msrv}".default;

        naersk' = callPackage naersk {
          rustc = toolchain;
          cargo = toolchain;
        };
        msrvNaersk = callPackage naersk {
          rustc = msrvToolchain;
          cargo = msrvToolchain;
        };
        cross-naersk' = pkgs.callPackage cross-naersk {inherit naersk;};

        buildMatrix = targets: {
          include =
            map (target: {
              inherit target;
              artifactSuffix = cross-naersk'.execSufficForTarget target;
            })
            targets;
        };

        hostTarget = pkgs.hostPlatform.config;
        targets = [
          "x86_64-unknown-linux-musl"
          "x86_64-pc-windows-gnu"
          hostTarget
        ];
        releaseTargets = lib.lists.remove hostTarget targets;

        src = sourceByRegex ./. ["Cargo.*" "(src|templates)(/.*)?"];
        nearskOpt = {
          pname = "shelve";
          root = src;
        };
      in rec {
        packages =
          lib.attrsets.genAttrs targets (target:
            (cross-naersk'.buildPackage target) nearskOpt)
          // {
            shelve = pkgs.shelve;
            check = naersk'.buildPackage (nearskOpt
              // {
                mode = "check";
              });
            clippy = naersk'.buildPackage (nearskOpt
              // {
                mode = "clippy";
              });
            msrv = msrvNaersk.buildPackage (nearskOpt
              // {
                mode = "check";
              });
            docker = callPackage ./docker.nix {};
            default = pkgs.shelve;
          };
        apps.default = packages.default;

        matrix = buildMatrix targets;
        releaseMatrix = buildMatrix releaseTargets;

        devShells.default = mkShell {
          nativeBuildInputs = with pkgs; [toolchain bacon cargo-msrv];
        };
      }
    )
    // {
      overlays.default = import ./overlay.nix;
      nixosModules.default = {
        pkgs,
        config,
        lib,
        ...
      }: {
        imports = [./module.nix];
        config = lib.mkIf config.services.shelve.enable {
          nixpkgs.overlays = [self.overlays.default];
          services.shelve.package = lib.mkDefault pkgs.shelve;
        };
      };
    };
}
