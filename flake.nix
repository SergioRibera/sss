{
  description = "Standar cross compile flake for Rust Lang Projects";

  nixConfig = {
    extra-substituters = [
      "https://cache.sergioribera.rs/main"
      "https://cache.nixos-cuda.org"
    ];
    extra-trusted-public-keys = [
      "main:vFI3N1JP9edRFwBwdk9ebUKTIPWK9R1ECbkdA7Q593M="
      "cache.nixos-cuda.org:74DUi4Ye579gUqzH4ziL9IyiJBlDpMRn9MBN8oNan9M="
    ];
  };

  inputs = {
    nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
    crane.url = "github:ipetkov/crane";
    fenix.url = "github:nix-community/fenix";
    flake-utils.url = "github:numtide/flake-utils";
    nix-bundle-app = {
      url = "github:SergioRibera/nix-bundle-app";
      inputs.nixpkgs.follows = "nixpkgs";
      inputs.flake-utils.follows = "flake-utils";
    };
  };

  outputs = {
    nixpkgs,
    flake-utils,
    nix-bundle-app,
    ...
  } @ inputs: let
      fenix = inputs.fenix.packages;
    in
    # Iterate over Arm, x86 for MacOs 🍎 and Linux 🐧
    (flake-utils.lib.eachDefaultSystem (
      system: let
        pkgs = import nixpkgs {
          inherit system;
          config = {
            allowUnfree = true;
            allowBroken = true;
            allowInsecure = true;
          };
        };
        crane = inputs.crane.mkLib pkgs;
        bundler = nix-bundle-app.lib.mkLib pkgs;
        # Default `system` variant: OCR compiled in (CPU EP), runtime
        # libonnxruntime expected from the distro PM. See `nix/release.nix`
        # for the per-variant naming + recommends model.
        sssBundle = import ./nix {
          inherit pkgs system crane fenix bundler;
        };
        # NVIDIA/CUDA variant: cargo feature `cuda` baked in; distro
        # packages recommend a CUDA-enabled onnxruntime.
        sssBundleNvidia = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "nvidia";
        };
        # ROCm/AMD GPU variant: distro packages recommend a ROCm-enabled
        # onnxruntime + rocm-hip-runtime userspace.
        sssBundleRocm = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "rocm";
        };
        # OCR-stripped variant: built with `--no-default-features`, no
        # sss_ocr code in the binary, no runtime recommendation.
        sssBundleNoOcr = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "noocr";
        };
        # Self-contained dev convenience builds. These flip
        # `bundleRuntime=true` so the produced binary carries
        # libonnxruntime + the matching GPU runtime libs in its RPATH,
        # useful when hacking outside any distro PM (`nix run .#cli-cuda`).
        # NOT exposed as release variants — release matrix uses the
        # variant-named bundles above instead.
        sssBundleCudaDev = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "nvidia";
          bundleRuntime = true;
        };
        sssBundleRocmDev = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "rocm";
          bundleRuntime = true;
        };
      in {
        inherit (sssBundle) apps devShells;
        packages = sssBundle.packages // {
          cli-nvidia = sssBundleNvidia.packages.cli;
          cli-rocm = sssBundleRocm.packages.cli;
          cli-noocr = sssBundleNoOcr.packages.cli;
          release-sss-nvidia = sssBundleNvidia.packages.release-sss;
          release-sss-rocm = sssBundleRocm.packages.release-sss;
          release-sss-noocr = sssBundleNoOcr.packages.release-sss;
          # Dev-only self-contained builds.
          cli-cuda-bundled = sssBundleCudaDev.packages.cli;
          cli-rocm-bundled = sssBundleRocmDev.packages.cli;
        };
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
