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
        sssBundle = import ./nix {
          inherit pkgs system crane fenix bundler;
        };
        # OCR-less variant. Same Rust crate but built with
        # `--no-default-features` (sss_cli `ocr` off), so the binary has
        # no sss_ocr code paths AND the bundler skips libonnxruntime +
        # CUDA stack. Distro packages can still recommend the system's
        # `onnxruntime` via per-distro `info.depends` (see release.nix).
        sssBundleNoOcr = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "no-ocr";
        };
        # ROCm/AMD GPU variant. Rebuilds onnxruntime with rocmSupport=true
        # and bundles the rocm-hip-runtime userspace into /opt/sss-rocm/lib/.
        # Hosts AMD users on Linux-x86_64 (RX 6000+ / MI series). Surfaces
        # as `cli-rocm` for ad-hoc builds and `release-sss-rocm` for CI.
        sssBundleRocm = import ./nix {
          inherit pkgs system crane fenix bundler;
          variant = "rocm";
        };
        # GPU-accelerated dev variants. Each one rebuilds the workspace
        # with the matching `sss_cli` feature on AND swaps the bundled
        # onnxruntime for one compiled with the matching EP, so the
        # produced binary's `libonnxruntime.so` actually exposes the
        # provider at runtime. Selected by:
        #   nix build .#cli-cuda
        #   nix build .#cli-cuda-tensorrt
        sssBundleCuda = import ./nix {
          inherit pkgs system crane fenix bundler;
          cudaSupport = true;
          cudaRuntime = true;
        };
        sssBundleCudaTrt = import ./nix {
          inherit pkgs system crane fenix bundler;
          cudaSupport = true;
          tensorrtSupport = true;
          cudaRuntime = true;
        };
      in {
        inherit (sssBundle) apps devShells;
        packages = sssBundle.packages // {
          cli-no-ocr = sssBundleNoOcr.packages.cli;
          release-sss-no-ocr = sssBundleNoOcr.packages.release-sss;
          cli-rocm = sssBundleRocm.packages.cli;
          release-sss-rocm = sssBundleRocm.packages.release-sss;
          cli-cuda = sssBundleCuda.packages.cli;
          cli-cuda-tensorrt = sssBundleCudaTrt.packages.cli;
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
