# Per-binary `bundler.release` outputs. Each CI runner builds the slice of
# the global target matrix that its `system` can produce natively. The
# publish job downloads every slice and concatenates the install scripts /
# INSTALL.md sections.
#
# Two binaries ship from this workspace:
#   - `sss`      (sss_cli  — GUI selector + CLI). v0.1.x.
#   - `sss_code` (sss_code — pure CLI, code → png renderer). v0.2.x.
#
# `sss` ships in four AUR-equivalent variants, none of which bundle
# libonnxruntime / GPU runtime libs into the artifact (see `default.nix`
# `bundleRuntime ? false`). Each variant declares the matching system
# onnxruntime package as a per-distro recommendation, so a regular
# `apt install sss` / `pacman -S sss-bin` pulls in the runtime via the
# distro PM:
#
#   variant   → AUR pkg          → system onnxruntime pkg expected
#   "system"  → sss-bin          → onnxruntime (CPU)
#   "nvidia"  → sss-nvidia-bin   → onnxruntime-cuda / equivalent
#   "rocm"    → sss-rocm-bin     → onnxruntime-rocm  / equivalent
#   "noocr"   → sss-noocr-bin    → (none — OCR compiled out)
#
# Targets: linux x86_64 / aarch64 + macOS x86_64 / aarch64. Windows is
# explicitly not built. AppImage is also dropped — by design the app
# depends on a system onnxruntime, AppImage's portability promise no
# longer fits.
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
  # `Recommends`. Values: "system" | "nvidia" | "rocm" | "noocr".
  # Mirrors the same arg in `nix/default.nix`; this file only consumes
  # the naming + metadata side.
  variant ? "system",
}:
let
  cliCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_cli/Cargo.toml);
  codeCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_code/Cargo.toml);
  sssVersion = cliCargo.package.version;
  sssCodeVersion = codeCargo.package.version;

  repo = "SergioRibera/sss";
  maintainer = "Sergio Ribera <sergioalejandroriberacosta@gmail.com>";

  ocrSupport = variant != "noocr";

  # Package name per variant. Drives bundle filename, install prefix
  # (`/opt/<name>/bin/sss`), bundleId, AUR pkg, and brew formula filename.
  # Binary basename stays `sss` regardless — the bundler's symlink
  # fallback in `_common-linux.nix` walks the `/bin` contents when
  # `meta.name` doesn't match a binary name.
  sssPkgName = {
    "system" = "sss";
    "nvidia" = "sss-nvidia";
    "rocm" = "sss-rocm";
    "noocr" = "sss-noocr";
  }.${variant} or (throw "release.nix: unknown variant '${variant}'");

  # Shared info defaults; per-binary specifics override.
  baseInfo = {
    inherit maintainer;
    homepage = "https://github.com/${repo}";
    license = "MIT";
    # Ship a slim bundle: no `/opt/<name>/lib/` copy, no RPATH rewrite
    # to `$ORIGIN/../lib`. Runtime libs (fontconfig, libwayland, libxcb,
    # libxkbcommon, dbus, openssl, onnxruntime, …) resolve through the
    # target distro's loader. The per-variant `info.depends.<distro>`
    # entries declare them so `apt install sss` / `dnf install sss` /
    # `pacman -U sss-bin` pulls them in alongside the binary.
    bundleLibs = false;
  };

  # Per-distro onnxruntime package candidates. The names are realistic for
  # each ecosystem at the time of writing:
  #
  # CPU-only:
  #   * deb (Debian unstable, Ubuntu 25.04+): `libonnxruntime1.20` (with
  #     a plain `libonnxruntime` alternation for derivatives that namespace
  #     differently).
  #   * rpm (Fedora 40+, openSUSE Tumbleweed): `onnxruntime`.
  #   * archlinux (community): `onnxruntime`.
  #   * brew: `onnxruntime`.
  #
  # CUDA:
  #   * deb/rpm: NVIDIA's `libonnxruntime-gpu` debs from their own apt
  #     archive — no clean repo-level name to pin. Best-effort.
  #   * archlinux (AUR): `onnxruntime-cuda`.
  #
  # ROCm:
  #   * deb/rpm: no first-party repo packaging today; users hit AUR-like
  #     overlays or build from source. We list the AMDGPU userspace as
  #     a sibling Recommends so at least the loader/devices are present.
  #   * archlinux (AUR): `onnxruntime-rocm`.
  onnxruntimePackages = {
    cpu = {
      deb = [ "libonnxruntime1.20 | libonnxruntime" ];
      rpm = [ "onnxruntime" ];
      archlinux = [ "onnxruntime" ];
      brew = [ "onnxruntime" ];
    };
    cuda = {
      # Vendor name varies (`libonnxruntime-gpu`, `libonnxruntime1.20-gpu`,
      # etc). Use a wide alternation; the AUR side has a concrete package.
      deb = [ "libonnxruntime-gpu | libonnxruntime1.20-gpu | libonnxruntime" ];
      # RPM `Recommends:` does not support `|` alternation. Fedora/RHEL
      # ships a single `onnxruntime` package (CPU only); CUDA builds come
      # from NVIDIA's own repos with no canonical RPM name. List the one
      # name the distro repo can resolve and leave CUDA detection to the
      # binary's runtime EP probe (it falls back to CPU EP gracefully).
      rpm = [ "onnxruntime" ];
      archlinux = [ "onnxruntime-cuda" ];
    };
    rocm = {
      deb = [ "libonnxruntime-rocm | libonnxruntime" "libamdhip64-5 | libamdhip64" ];
      # See cuda.rpm note above re: `|` not being valid in RPM Recommends.
      rpm = [ "onnxruntime" "rocm-hip-runtime" ];
      archlinux = [ "onnxruntime-rocm" "rocm-hip-runtime" ];
    };
  };

  variantSummary = {
    "system" = "Take pretty screenshots of your screen with annotations + OCR";
    "nvidia" = "Take pretty screenshots of your screen with annotations + CUDA OCR";
    "rocm" = "Take pretty screenshots of your screen with annotations + ROCm OCR";
    "noocr" = "Take pretty screenshots of your screen with annotations (no OCR)";
  };

  variantLongDescription = {
    "system" = ''
      sss (Super ScreenShot) — interactive screen-region selector with a
      built-in annotation overlay (shapes, arrows, text, blur, pipette)
      and an on-device OCR pipeline. Native Wayland (wlr-layer-shell) +
      X11 backends; wgpu-accelerated preview canvas; exports to PNG /
      clipboard. The OCR pipeline dlopens libonnxruntime at runtime, so
      this package recommends the distro's CPU `onnxruntime` build.
    '';
    "nvidia" = ''
      sss (Super ScreenShot) — NVIDIA/CUDA variant. Same selector +
      annotation toolkit as `sss`, with the OCR pipeline compiled to
      register the CUDA execution provider. Recommends a CUDA-enabled
      onnxruntime build from the distro; falls back to CPU EP if the
      installed onnxruntime does not expose CUDA at runtime.
    '';
    "rocm" = ''
      sss (Super ScreenShot) — ROCm/AMD GPU variant. Same selector +
      annotation toolkit as `sss`, designed to drive a ROCm-enabled
      onnxruntime build from the distro. Requires an AMD GPU with ROCm
      support (RX 6000+ / MI series) and the AMDGPU kernel driver +
      `rocm-hip-runtime` userspace installed alongside.
    '';
    "noocr" = ''
      sss (Super ScreenShot) — OCR-less variant. Same selector +
      annotation toolkit, minus the on-device OCR pipeline; no
      libonnxruntime runtime dependency. Pick this if you don't need
      text recognition over selections and want the smallest install.
    '';
  };

  # Per-variant `info.depends` mapping. The bundler honours these keys
  # to populate the appropriate field in each per-distro manifest:
  #   * debRecommends      → Debian/Ubuntu  `Recommends`
  #   * rpmRecommends      → Fedora/openSUSE `Recommends`
  #   * archlinuxOptional  → Arch Linux PKGBUILD `optdepends`
  #   * brew               → Homebrew formula `depends_on`
  variantDepends = {
    "system" = {
      depends = {
        debRecommends = onnxruntimePackages.cpu.deb;
        rpmRecommends = onnxruntimePackages.cpu.rpm;
        archlinuxOptional = onnxruntimePackages.cpu.archlinux;
        brew = onnxruntimePackages.cpu.brew;
      };
    };
    "nvidia" = {
      depends = {
        debRecommends = onnxruntimePackages.cuda.deb;
        rpmRecommends = onnxruntimePackages.cuda.rpm;
        archlinuxOptional = onnxruntimePackages.cuda.archlinux;
      };
    };
    "rocm" = {
      depends = {
        debRecommends = onnxruntimePackages.rocm.deb;
        rpmRecommends = onnxruntimePackages.rocm.rpm;
        archlinuxOptional = onnxruntimePackages.rocm.archlinux;
      };
    };
    "noocr" = {
      # No runtime to recommend.
    };
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
  } // variantDepends.${variant};

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
  # CI invokes `nix build .#release-sss[-variant]` (and `.#release-sss_code`)
  # on each runner; each runner only emits its slice. The publish job
  # stitches every slice into one release.
  #
  # AppImage dropped: by design the binary expects a distro-installed
  # `libonnxruntime.so`, which breaks AppImage's portability promise.
  linuxFormats = [ "deb" "rpm" "archlinux" "tar.gz" "tar.zst" ];
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
      # x86_64-darwin intentionally absent: nixpkgs deprecating (26.05 is
      # the last supported release) + GitHub macos-13 runners queue
      # forever on free-tier quotas. aarch64-darwin covers all modern Mac
      # hardware. Re-introducing requires either a self-hosted runner or
      # paid larger macos-13 image.
      darwinArm = lib.optionalAttrs (system == "aarch64-darwin") {
        "aarch64-darwin" = {
          drv = hostDrv;
          formats = darwinFormats;
        };
      };
    in
      linuxX86 // linuxArmHost // darwinArm;

  # Per-variant slice eligibility. `system` + `noocr` ship on every
  # platform we build for. `nvidia` is Linux-only (no CUDA stack on
  # macOS). `rocm` is linux-x86_64 only — ROCm's aarch64 support is
  # niche server-side and Apple platforms have no ROCm story at all.
  variantSliceEligible = {
    "system" = true;
    "noocr" = true;
    "nvidia" = lib.hasSuffix "-linux" system;
    "rocm" = system == "x86_64-linux";
  }.${variant};

  sssMatrix =
    if !variantSliceEligible
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
