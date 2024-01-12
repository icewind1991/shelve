{
  stdenv,
  rustPlatform,
  lib,
}: let
  inherit (lib.sources) sourceByRegex;
  src = sourceByRegex ./. ["Cargo.*" "(src|templates)(/.*)?"];
in
  rustPlatform.buildRustPackage rec {
    pname = "shelve";
    version = "0.1.0";

    inherit src;

    cargoLock = {
      lockFile = ./Cargo.lock;
    };
  }
