{
  description = "Standar cross compile flake for Rust Lang Projects";
  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    flake-utils.url = "github:numtide/flake-utils";
  };

  outputs = {
    nixpkgs,
    flake-utils,
    ...
  } @ inputs: let
      fenix = inputs.fenix.packages;
    in
    # Iterate over Arm, x86 for MacOs 🍎 and Linux 🐧
    (flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = nixpkgs.legacyPackages.${system};
        crane = inputs.crane.mkLib pkgs;
        sssBundle = import ./nix {
          inherit pkgs system crane fenix;
        };
      in {
        inherit (sssBundle) apps packages devShells;
      }
    )) // (flake-utils.lib.eachDefaultSystemPassThrough (system: let
        pkgs = nixpkgs.legacyPackages.${system};
        crane = inputs.crane.mkLib pkgs;
      in {
      # Overlays
      overlays.default = import ./nix/overlay.nix {
        inherit crane fenix;
      };
      # nixosModules
      nixosModules = {
        default = import ./nix/nixos-module.nix {
        inherit crane fenix;
        };
        home-manager = import ./nix/hm-module.nix {
        inherit crane fenix;
        };
      };
    }));
}
