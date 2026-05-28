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
}:
let
  cliCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_cli/Cargo.toml);
  codeCargo = builtins.fromTOML (builtins.readFile ./../crates/sss_code/Cargo.toml);
  sssVersion = cliCargo.package.version;
  sssCodeVersion = codeCargo.package.version;

  repo = "SergioRibera/sss";
  maintainer = "Sergio Ribera <sergioalejandroriberacosta@gmail.com>";

  # Shared info defaults; per-binary specifics override.
  baseInfo = {
    inherit maintainer;
    homepage = "https://github.com/${repo}";
    license = "MIT";
  };

  sssInfo = baseInfo // {
    name = "sss";
    version = sssVersion;
    summary = "Take pretty screenshots of your screen with annotations";
    description = "Take pretty screenshots of your screen with annotations";
    longDescription = ''
      sss (Super ScreenShot) — interactive screen-region selector with a
      built-in annotation overlay (shapes, arrows, text, blur, pipette).
      Native Wayland (wlr-layer-shell) + X11 backends; wgpu-accelerated
      preview canvas; exports to PNG / clipboard.
    '';
    bundleId = "rs.sergioribera.sss";
    downloadUrl =
      "https://github.com/${repo}/releases/download/v${sssVersion}";
    desktopEntries = [
      {
        name = "sss";
        exec = "/opt/sss/bin/sss";
        comment = "Take pretty screenshots of your screen";
        categories = [ "Graphics" "Utility" ];
      }
    ];
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

  sssMatrix = matrixFor {
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
