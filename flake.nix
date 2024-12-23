{
  inputs = {
    nixpkgs.url = "nixpkgs/nixos-24.11";
    flakelight = {
      url = "github:nix-community/flakelight";
      inputs.nixpkgs.follows = "nixpkgs";
    };
    mill-scale = {
      url = "github:icewind1991/mill-scale";
      inputs.flakelight.follows = "flakelight";
    };
  };
  outputs = {mill-scale, ...}:
    mill-scale ./. {
      extraPaths = [
        ./templates
      ];
      nixosModules = {outputs, ...}: {
        default = {
          pkgs,
          config,
          lib,
          ...
        }: {
          imports = [./nix/module.nix];
          config = lib.mkIf config.services.shelve.enable {
            nixpkgs.overlays = [outputs.overlays.default];
            services.shelve.package = lib.mkDefault pkgs.shelve;
          };
        };
      };
    };
}
