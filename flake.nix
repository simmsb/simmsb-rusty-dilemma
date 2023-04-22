{
  description = "things";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixpkgs-unstable";

    crane = {
      url = "github:ipetkov/crane";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    fenix = {
      url = "github:nix-community/fenix";
      inputs.nixpkgs.follows = "nixpkgs";
    };

    parts.url = "github:hercules-ci/flake-parts";
    parts.inputs.nixpkgs-lib.follows = "nixpkgs";
  };

  outputs =
    inputs@{ self
    , nixpkgs
    , crane
    , fenix
    , parts
    , ...
    }:
    parts.lib.mkFlake { inherit inputs; } {
      systems = nixpkgs.lib.systems.flakeExposed;
      imports = [
      ];
      perSystem = { config, pkgs, system, lib, ... }:
        let
          arm-toolchain = fenix.packages.${system}.fromToolchainFile {
            file = ./rust-toolchain.toml;
            sha256 = "sha256-C9yOGqLuqT8wuqyALfKLYHsmSEEN9RjeL7cxsDy7rOM=";
          };
          native-toolchain = fenix.packages.${system}.complete.withComponents [
            "cargo"
            "clippy"
            "rust-src"
            "rustc"
            "rustfmt"
          ];
          toolchain = fenix.packages.${system}.combine [ arm-toolchain native-toolchain ];
          craneLib = (crane.mkLib pkgs).overrideToolchain toolchain;
          package = { path, target ? "thumbv6m-none-eabi", args ? "", profile ? "release" }: craneLib.buildPackage {
            cargoExtraArgs = "--target ${target} ${args}";
            CARGO_PROFILE = profile;
            pname = "rusty-dilemma";
            version = "0.1.0";
            src = lib.cleanSourceWith {
              src = craneLib.path path;
              filter =
                let
                  embeddedStuffFilter = path: _type: builtins.match ".*\\.x$" path != null;
                  embeddedOrCargoStuff = path: type:
                    (embeddedStuffFilter path type) || (craneLib.filterCargoSources path type);
                in
                embeddedOrCargoStuff;
            };
            doCheck = false;
            buildInputs = [
              # Add additional build inputs here
            ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
              # Additional darwin specific inputs can be set here
              pkgs.libiconv
            ];
          };
          bootloader = package { path = ./bootloader; };
          firmware = args: package (args // { path = ./.; });
          elf = pkg: name: pkgs.runCommandLocal "mkelf" { } ''
            mkdir -p $out
            cp ${pkg}/bin/${name} $out/${name}.elf
          '';
          binary = pkg: name: pkgs.runCommandLocal "mkbinary" { buildInputs = [ pkgs.llvm ]; } ''
            mkdir -p $out
            llvm-objcopy -O binary ${pkg}/bin/${name} $out/${name}.bin
          '';
        in
        rec
        {
          devShells.default = pkgs.mkShell {
            inputsFrom = [ (firmware { args = "--lib"; profile = "dev"; }) ];
            nativeBuildInputs = with pkgs; [
              fenix.packages.${system}.rust-analyzer
              cargo-binutils
              cargo-flash
              #probe-run
              picotool
            ];
          };
          packages.default = firmware { args = "--lib"; profile = "dev"; };
          packages.left = firmware { args = "--bin left"; profile = "release"; };
          packages.right = firmware { args = "--bin right"; profile = "release"; };
          packages.bootloader = bootloader;
          packages.binaries = pkgs.symlinkJoin {
            name = "dilemma-binaries";
            paths = [
              (elf packages.left "left")
              (elf packages.right "right")
              (elf packages.bootloader "boot")
              (binary packages.left "left")
              (binary packages.right "right")
              (binary packages.bootloader "boot")
            ];
          };
        };
    };
}
