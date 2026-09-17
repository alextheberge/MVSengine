# SPDX-License-Identifier: AGPL-3.0-only
#
# Nix flake for mvs-manager. Build: `nix build`. Run: `nix run . -- lint`.
# Dev shell with the full toolchain: `nix develop`.
#
# NOTE: written and reviewed for correctness, but not built-tested in this
# environment (no `nix` binary available here) — verify with `nix build`
# and `nix flake check` before relying on it.
{
  description = "mvs-manager: MVS (ARCH.FEAT.PROT.FIX-CONT) multidimensional versioning CLI";

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = { self, nixpkgs, flake-utils }:
    flake-utils.lib.eachDefaultSystem (system:
      let
        pkgs = import nixpkgs { inherit system; };
        cargoToml = builtins.fromTOML (builtins.readFile ./Cargo.toml);
      in
      {
        packages.default = pkgs.rustPlatform.buildRustPackage {
          pname = "mvs-manager";
          version = cargoToml.package.version;
          src = ./.;
          cargoLock.lockFile = ./Cargo.lock;

          # git is a runtime dependency (migrate/backfill/detect shell out
          # to it), not just a build dependency.
          nativeBuildInputs = [ pkgs.pkg-config ];
          buildInputs = [ pkgs.openssl ] ++ pkgs.lib.optionals pkgs.stdenv.isDarwin [
            pkgs.darwin.apple_sdk.frameworks.Security
            pkgs.darwin.apple_sdk.frameworks.SystemConfiguration
          ];
          propagatedBuildInputs = [ pkgs.git ];

          cargoBuildFlags = [ "--bin" "mvs-manager" ];
          doCheck = false; # the full test suite expects a git checkout, not the Nix sandbox

          meta = with pkgs.lib; {
            description = "Multidimensional (ARCH.FEAT.PROT.FIX-CONT) post-SemVer versioning CLI";
            homepage = "https://github.com/alextheberge/MVSengine";
            license = licenses.agpl3Only;
            mainProgram = "mvs-manager";
          };
        };

        devShells.default = pkgs.mkShell {
          inputsFrom = [ self.packages.${system}.default ];
          packages = [ pkgs.cargo-deny pkgs.rust-analyzer ];
        };
      });
}
