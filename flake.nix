{
  description = "Standar cross compile flake for Rust Lang Projects";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    cranix.url = "github:Lemin-n/cranix/2af6b2e71577bb8836b10e28f3267f2c5342a8fd";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    self,
    nixpkgs,
    flake-utils,
    ...
  } @ inputs:
  # Iterate over Arm, x86 for MacOs 🍎 and Linux 🐧
    flake-utils.lib.eachSystem (flake-utils.lib.defaultSystems) (
      system: let
        sssBundle = import ./nix {
          inherit system;
          pkgs = nixpkgs.legacyPackages.${system};
          crane = inputs.crane.lib;
          cranix = inputs.cranix.lib;
          fenix = inputs.fenix.packages;
        };
      in {
        inherit (sssBundle) apps packages devShells;
      }
    )
    // {
      # Overlays
      overlays.default = import ./nix/overlay.nix {
        crane = inputs.crane.lib;
        cranix = inputs.cranix.lib;
        fenix = inputs.fenix.packages;
      };
      # nixosModules
      nixosModules = {
        default = import ./nix/nixos-module.nix {
          crane = inputs.crane.lib;
          cranix = inputs.cranix.lib;
          fenix = inputs.fenix.packages;
        };
        home-manager = import ./nix/hm-module.nix {
          crane = inputs.crane.lib;
          cranix = inputs.cranix.lib;
          fenix = inputs.fenix.packages;
        };
      };
    };
}
