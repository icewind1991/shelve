{ stdenv
, rustPlatform
, lib
,
}:
let
  inherit (lib.sources) sourceByRegex;
  inherit (builtins) fromTOML readFile;
  src = sourceByRegex ../. [ "Cargo.*" "(src|templates)(/.*)?" ];
  cargoPackage = (fromTOML (readFile ../Cargo.toml)).package;
in
rustPlatform.buildRustPackage rec {
  pname = cargoPackage.name;
  inherit (cargoPackage) version;

  inherit src;

  cargoLock = {
    lockFile = ../Cargo.lock;
  };
}
