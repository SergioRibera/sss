# Per-binary `bundler.release` outputs. Each CI runner builds the slice of
# the global target matrix that its `system` can produce natively. The
# publish job downloads every slice and concatenates the install scripts /
# INSTALL.md sections.
#
# Two binaries ship from this workspace:
#   - `sss`      (sss_cli  — GUI selector + CLI). v0.1.x.
#   - `sss_code` (sss_code — pure CLI, code → png renderer). v0.2.x.
#
# Targets: linux x86_64 / aarch64 + macOS x86_64 / aarch64. Windows is
# explicitly not built.
{
  pkgs,
  lib,
  system,
  bundler,
  craneLib,
  commonArgs,
  sssPkg,
  sssCodePkg,
  # Release variant. Drives package naming + description + per-distro
  # depends. Values: "full" | "no-ocr" | "rocm". Mirrors the same arg
  # in `nix/default.nix`; this file only consumes the naming side.
  variant ? "full",
}:
let
  cliCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_cli/Cargo.toml);
  codeCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_code/Cargo.toml);
  sssVersion = cliCargo.package.version;
  sssCodeVersion = codeCargo.package.version;

  repo = "SergioRibera/sss";
  maintainer = "Sergio Ribera <sergioalejandroriberacosta@gmail.com>";

  ocrSupport = variant != "no-ocr";
  rocmVariant = variant == "rocm";

  # Package name swaps per variant. Drives bundle filename, install prefix
  # (`/opt/<name>/bin/sss`), bundleId, AUR pkg, and brew formula filename.
  # Binary basename stays `sss` regardless — the bundler's symlink
  # fallback in `_common-linux.nix` walks the `/bin` contents when
  # `meta.name` doesn't match a binary name.
  sssPkgName = {
    "full" = "sss";
    "no-ocr" = "sss-no-ocr";
    "rocm" = "sss-rocm";
  }.${variant} or (throw "release.nix: unknown variant '${variant}'");

  # Shared info defaults; per-binary specifics override.
  baseInfo = {
    inherit maintainer;
    homepage = "https://github.com/${repo}";
    license = "MIT";
  };

  # Per-distro package names for the libonnxruntime runtime. The
  # full-OCR variant doesn't need to declare these (we bundle the lib
  # directly into `/opt/sss/lib/`); the `no-ocr` variant lists them as
  # `Recommends` / `optdepends` so users who DO want OCR can install
  # their distro's package without rebuilding.
  ocrRuntimeDepends = {
    # Debian/Ubuntu ship libonnxruntime starting from late-2024 unstable;
    # the soname-based binary `libonnxruntime1.20` is the most stable
    # alternation. Fallback to plain `libonnxruntime` for derivatives
    # that namespace differently.
    deb = [ "libonnxruntime1.20 | libonnxruntime" ];
    # Fedora ships `onnxruntime` directly; openSUSE Tumbleweed too.
    rpm = [ "onnxruntime" ];
    # AUR has `onnxruntime` (CPU) + `onnxruntime-cuda` for GPU users.
    archlinux = [ "onnxruntime" ];
    # Homebrew ships `onnxruntime` as a formula.
    brew = [ "onnxruntime" ];
  };

  # ROCm runtime userspace components users need on the host: the binary
  # bundles libonnxruntime + the rocm runtime libs, but the kernel-level
  # AMDGPU driver + ROCm userspace loader (libamdhip64 / libhsa-runtime)
  # ship with the distro. We advertise them as Recommends so a user
  # installing the `sss-rocm` package gets nudged towards the right deps.
  rocmRuntimeDepends = {
    deb = [ "libamdhip64-5 | libamdhip64" "rocm-smi" ];
    rpm = [ "rocm-hip-runtime" "rocm-smi" ];
    archlinux = [ "rocm-hip-runtime" "rocm-smi-lib" ];
  };

  variantSummary = {
    "full" = "Take pretty screenshots of your screen with annotations";
    "no-ocr" = "Take pretty screenshots of your screen with annotations (no OCR)";
    "rocm" = "Take pretty screenshots of your screen with annotations (ROCm/AMD GPU)";
  };

  variantLongDescription = {
    "full" = ''
      sss (Super ScreenShot) — interactive screen-region selector with a
      built-in annotation overlay (shapes, arrows, text, blur, pipette).
      Native Wayland (wlr-layer-shell) + X11 backends; wgpu-accelerated
      preview canvas; exports to PNG / clipboard.
    '';
    "no-ocr" = ''
      sss (Super ScreenShot) — OCR-less variant. Same selector + annotation
      toolkit as the full `sss` package, minus the on-device OCR pipeline
      and its libonnxruntime / CUDA runtime payload. Install your distro's
      `onnxruntime` package and pick the full `sss` build if you want OCR
      recognition over selections.
    '';
    "rocm" = ''
      sss (Super ScreenShot) — ROCm/AMD GPU variant. Same selector +
      annotation toolkit as the full build, with the OCR pipeline routed
      through onnxruntime's ROCm execution provider. Requires an AMD GPU
      with ROCm support (RX 6000+ / MI series). Distro-shipped ROCm
      userspace + AMDGPU kernel driver must be installed on the host.
    '';
  };

  sssInfo = baseInfo // {
    name = sssPkgName;
    version = sssVersion;
    summary = variantSummary.${variant};
    description = variantSummary.${variant};
    longDescription = variantLongDescription.${variant};
    bundleId = "rs.sergioribera.${sssPkgName}";
    downloadUrl =
      "https://github.com/${repo}/releases/download/v${sssVersion}";
    desktopEntries = [
      {
        name = sssPkgName;
        exec = "/opt/${sssPkgName}/bin/sss";
        comment = variantSummary.${variant};
        categories = [ "Graphics" "Utility" ];
      }
    ];
  } // lib.optionalAttrs (!ocrSupport) {
    # Recommend the system's onnxruntime package on each linux distro
    # AND advertise it as an optional brew dependency. Without these
    # the no-ocr binary still runs (OCR code paths are compiled out) —
    # they only kick in if the user ALSO installs the full sss package
    # alongside this one and wants the system loader to find a shared
    # libonnxruntime for it.
    depends = {
      debRecommends = ocrRuntimeDepends.deb;
      rpmRecommends = ocrRuntimeDepends.rpm;
      archlinuxOptional = ocrRuntimeDepends.archlinux;
      brew = ocrRuntimeDepends.brew;
    };
  } // lib.optionalAttrs rocmVariant {
    # ROCm needs the AMDGPU driver + the rocm-hip-runtime userspace from
    # the host distro to resolve `libamdhip64.so` / kernel devices. We
    # ship libonnxruntime + the rocm-enabled EP in the bundle.
    depends = {
      debRecommends = rocmRuntimeDepends.deb;
      rpmRecommends = rocmRuntimeDepends.rpm;
      archlinuxOptional = rocmRuntimeDepends.archlinux;
    };
  };

  sssCodeInfo = baseInfo // {
    name = "sss_code";
    version = sssCodeVersion;
    summary = "Take pretty screenshots of your code";
    description = "Take pretty screenshots of your code";
    longDescription = ''
      sss_code — render source files into beautiful PNG screenshots with
      syntax highlighting (powered by syntect), themes and configurable
      backgrounds. CLI-only; no GUI dependencies.
    '';
    bundleId = "rs.sergioribera.sss_code";
    downloadUrl =
      "https://github.com/${repo}/releases/download/sss_code/v${sssCodeVersion}";
  };

  # Slice the global matrix down to what the current Nix system can build.
  # CI invokes `nix build .#release-sss` (and `.#release-sss_code`) on each
  # runner; each runner only emits its slice. The publish job stitches
  # every slice into one release.
  linuxFormats = [ "deb" "rpm" "archlinux" "tar.gz" "tar.zst" "appimage" ];
  darwinFormats = [ "tar.gz" "tar.zst" "pkg" "dmg" "brew" ];

  # Cross-arch slices are intentionally omitted: native runners per arch
  # sidestep the `outputHashes` plumbing that `pkgsCross.*.rustPlatform`
  # needs for this workspace's git-based crates (winit/egui forks +
  # mouse_position).

  # Build the matrix entry set for a given binary, gated on what the
  # current `system` can produce. Each runner sees only its own slice.
  matrixFor = { drvName, hostDrv }:
    let
      linuxX86 = lib.optionalAttrs (system == "x86_64-linux") {
        "x86_64-linux" = {
          drv = hostDrv;
          formats = linuxFormats;
        };
      };
      linuxArmHost = lib.optionalAttrs (system == "aarch64-linux") {
        "aarch64-linux" = {
          drv = hostDrv;
          formats = linuxFormats;
        };
      };
      darwinX86 = lib.optionalAttrs (system == "x86_64-darwin") {
        "x86_64-darwin" = {
          drv = hostDrv;
          formats = darwinFormats;
        };
      };
      darwinArm = lib.optionalAttrs (system == "aarch64-darwin") {
        "aarch64-darwin" = {
          drv = hostDrv;
          formats = darwinFormats;
        };
      };
    in
      linuxX86 // linuxArmHost // darwinX86 // darwinArm;

  # ROCm bundles are linux-x86_64 only: ROCm has no Apple Silicon / Darwin
  # support and AMD's aarch64 server stack isn't a realistic distribution
  # target for this app. On any other host the matrix collapses to empty
  # and `releaseOrSkip` emits a NOTES placeholder.
  rocmEligible = system == "x86_64-linux";

  sssMatrix =
    if rocmVariant && !rocmEligible
    then {}
    else matrixFor {
      drvName = "sss";
      hostDrv = sssPkg;
    };
  sssCodeMatrix = matrixFor {
    drvName = "sss_code";
    hostDrv = sssCodePkg;
  };

  # Don't call bundler.release with an empty matrix — emit a placeholder.
  releaseOrSkip = info: matrix:
    if matrix == {}
    then pkgs.runCommand "${info.name}-${info.version}-release-empty" {} ''
      mkdir -p $out
      echo "No release artifacts producible on system=${system} for ${info.name}." > $out/NOTES.md
    ''
    else bundler.release {
      inherit info matrix;
      releaseUrl = info.downloadUrl;
      installScripts = true;
    };
in
{
  release-sss = releaseOrSkip sssInfo sssMatrix;
  release-sss_code = releaseOrSkip sssCodeInfo sssCodeMatrix;
}
