let
  inherit
    (builtins)
    currentSystem
    fromJSON
    readFile
    ;
  getFlake = name:
    with (fromJSON (readFile ../flake.lock)).nodes.${name}.locked; {
      inherit rev;
      outPath = fetchTarball {
        url = "https://github.com/${owner}/${repo}/archive/${rev}.tar.gz";
        sha256 = narHash;
      };
    };
in
  {
    system ? currentSystem,
    pkgs ? import (getFlake "nixpkgs") {localSystem = {inherit system;};},
    lib ? pkgs.lib,
    crane,
    fenix,
    bundler ? null,
    stdenv ? pkgs.stdenv,
    # Release variant selector. Drives the bundle name, which cargo features
    # land in the binary, and which onnxruntime package the distro PM is
    # told to install. NONE of the variants bundle libonnxruntime or GPU
    # runtime libs anymore — the binary uses `ORT_PREFER_DYNAMIC_LINK=1`
    # and dlopens libonnxruntime.so from the system loader path at runtime,
    # so the user's distro package is the one in play. See `nix/release.nix`
    # for the per-distro `Recommends` declarations.
    #
    # Values:
    #   "system" — default. OCR on; no GPU cargo features; depends on the
    #              distro's `onnxruntime` (CPU) package at runtime.
    #   "nvidia" — OCR on + `cuda` cargo feature; depends on a CUDA-enabled
    #              onnxruntime (distro-specific package name).
    #   "rocm"   — OCR on; no cargo feature toggles (ROCm is an
    #              onnxruntime build flag, not a cargo flag); depends on a
    #              ROCm-enabled onnxruntime. Linux-only.
    #   "noocr"  — OCR compiled out entirely; binary has no sss_ocr code.
    variant ? "system",
    # Derived flags. Callers can override individually for ad-hoc builds
    # (e.g. `cli-cuda-bundled` style dev derivations with bundleRuntime=true).
    ocrSupport ? (variant != "noocr"),
    cudaSupport ? (ocrSupport && variant == "nvidia"),
    coreMLSupport ? (ocrSupport && stdenv.hostPlatform.isDarwin),
    directMLSupport ? (ocrSupport && stdenv.hostPlatform.isWindows),
    tensorrtSupport ? false,
    openvinoSupport ? false,
    # When false, libonnxruntime + CUDA/ROCm libs are NOT added to
    # `runtimeDeps` (autoPatchelfHook leaves them out of RPATH). The binary
    # still has ORT bindings compiled in via `ORT_PREFER_DYNAMIC_LINK=1`
    # and resolves `libonnxruntime.so` from the system loader cache at
    # runtime. Distro packages declare the right onnxruntime variant as a
    # `Recommends` so a standard `apt install sss` / `pacman -S sss-bin`
    # also pulls in the runtime.
    #
    # Set true only for self-contained dev builds you want to run outside
    # any distro PM (`nix build .#cli` for hacking, AppImage-like usage).
    # When true, `cudaRuntime`/`rocmRuntime` light up by default so the
    # bundled libonnxruntime gets a matching build.
    bundleRuntime ? false,
    cudaRuntime ? (bundleRuntime && cudaSupport),
    rocmRuntime ? (bundleRuntime && variant == "rocm"),
    ...
  }: let
    # When `ocrSupport=false` every OCR knob collapses to off — the
    # cargo build skips sss_ocr entirely and the bundler stops carrying
    # libonnxruntime + the CUDA stack into the artifact.
    realCudaSupport = ocrSupport && cudaSupport;
    realCudaRuntime = ocrSupport && cudaRuntime;
    realRocmRuntime = ocrSupport && rocmRuntime;
    realTensorrtSupport = ocrSupport && tensorrtSupport;
    realCoreMLSupport = ocrSupport && coreMLSupport;
    realDirectMLSupport = ocrSupport && directMLSupport;
    realOpenvinoSupport = ocrSupport && openvinoSupport;
    # Swap the system onnxruntime for one with GPU EPs compiled in. We
    # only override when the runtime flag is set; the cargo feature on
    # its own doesn't need a new lib (the stock CPU build still exposes
    # all the binding entry points; calls to CUDA EP just fall back).
    onnxruntime =
      if realCudaRuntime || realRocmRuntime
      then
        pkgs.onnxruntime.override (lib.filterAttrs (_: v: v != null) {
          cudaSupport = if realCudaRuntime then true else null;
          rocmSupport = if realRocmRuntime then true else null;
        })
      else pkgs.onnxruntime;
    # Cargo features handed to crane via `cargoExtraArgs`. Order is the
    # auto-pick preference: GPU EPs first, CPU implicit at the tail.
    # `ocr` is the master switch; when off we pass `--no-default-features`
    # below and the EP list collapses to empty.
    gpuCargoFeatures = lib.concatStringsSep "," (
      lib.optionals realCudaSupport ["cuda"]
      ++ lib.optionals realTensorrtSupport ["tensorrt"]
      ++ lib.optionals realCoreMLSupport ["coreml"]
      ++ lib.optionals realDirectMLSupport ["directml"]
      ++ lib.optionals realOpenvinoSupport ["openvino"]
    );
    # fenix: rustup replacement for reproducible builds
    toolchain = fenix.${system}.fromToolchainFile {
      file = ./../rust-toolchain.toml;
      sha256 = "sha256-gh/xTkxKHL4eiRXzWv8KP7vfjSk61Iq48x47BEDFgfk=";
    };

    # crane: cargo and artifacts manager
    craneLib = crane.overrideToolchain toolchain;

    # buildInputs for SSS — the runtime / linker dependencies the
    # workspace's crates pull in on Linux.
    #
    # * wayland (libwayland-client.so.0)
    #     wayland-client / wayland-protocols / wayland-protocols-wlr /
    #     wayland-cursor link against this. Required at build time so
    #     rustc can find -lwayland-client *and* at run time so the
    #     dynamic linker can resolve it. Note that several transitive
    #     deps in the tree (wayland-sys, xkbcommon-dl,
    #     yeslogic-fontconfig-sys) use `dlib`/`libloading` to dlopen
    #     these .so files at runtime — those calls do NOT show up as
    #     DT_NEEDED entries, so autoPatchelfHook alone can't infer the
    #     rpath. We declare them explicitly in `runtimeDependencies`
    #     below so the rpath gets baked into the produced binaries.
    # * libxkbcommon: used by winit / arboard fallbacks on Wayland.
    # * libxcb / libX11: x11rb (sss_capture's X11 backend)
    #     loads libxcb; arboard's X11 path needs libX11 + libXi.
    # * libxkbcommon-x11: linked alongside libxkbcommon when xkb's
    #     X11 helpers are pulled in indirectly by winit.
    # * fontconfig: ab_glyph itself is pure Rust, but other crates
    #     in the dep tree call into fontconfig for system-font
    #     discovery — keep it around for the broader workspace.
    # * libxcursor: wayland-cursor is pure Rust today, but some
    #     installations still pull in libxcursor through xkbcommon's
    #     transitive deps; cheap to include.
    # * dbus / openssl: the dbus crate is vendored, but the
    #     screencast portal path inside sss_capture talks to
    #     org.freedesktop.portal.Desktop over D-Bus and benefits
    #     from having libdbus headers visible.
    buildInputs = with pkgs; [
      fontconfig.dev
      freetype
      libxkbcommon.dev
      libxkbcommon
      libxcb
      libx11
      libxi
      libxcursor
      libxrandr
      libxcb.dev
      wayland
      wayland-protocols
      wayland-scanner
      dbus.dev
    ] ++ lib.optionals ocrSupport [
      # onnxruntime is a BUILD-TIME dep regardless of `bundleRuntime`: the
      # ort crate needs the headers + a stub libonnxruntime.so to link
      # against (`ORT_LIB_LOCATION` points here). The bundle-vs-system
      # split is enforced in `runtimeDeps` below — that's the list
      # autoPatchelfHook walks to bake the RPATH.
      onnxruntime
    ];

    # Libraries that the workspace loads via dlopen at runtime. These
    # are not present as DT_NEEDED in the produced ELF (libloading
    # resolves them by SONAME at runtime), so autoPatchelfHook has no
    # way of inferring them — we hand them in explicitly and the hook
    # appends each one's `/lib` to the binary's RPATH.
    #
    # libonnxruntime + the GPU runtime libs are gated on `bundleRuntime`.
    # Default-off: the produced bundle does NOT carry libonnxruntime.so,
    # and the binary dlopens it from the system loader cache at runtime
    # (distro PM installs the right onnxruntime variant via the
    # per-distro `Recommends` declared in `release.nix`).
    runtimeDeps = with pkgs; [
      wayland
      libxkbcommon
      fontconfig.lib
      freetype
      libxcb
      libx11
      libxi
      libxcursor
      libxrandr
      dbus.lib
      stdenv.cc.cc.lib
      vulkan-loader
      libglvnd
      mesa
    ] ++ lib.optionals (ocrSupport && bundleRuntime) [
      onnxruntime
    ] ++ lib.optionals realCudaRuntime [
      # CUDA runtime libraries that onnxruntime dlopens. Pulled from the
      # same cudaPackages set the override picks; ship them so the binary
      # finds libcudart / libcudnn etc. at runtime.
      pkgs.cudaPackages.cudatoolkit
      pkgs.cudaPackages.cudnn
      pkgs.cudaPackages.libcublas
    ];

    # Base args, need for build all crate artifacts and caching this for late builds
    commonArgs = {
      src = ./..;
      doCheck = false;
      nativeBuildInputs =
        [] ++ lib.optionals stdenv.hostPlatform.isLinux [
          pkgs.autoPatchelfHook
          pkgs.pkg-config
          pkgs.wayland-scanner
          pkgs.stdenv.cc.cc.lib
        ] ++ lib.optionals stdenv.buildPlatform.isDarwin [
          pkgs.libiconv
        ];
        runtimeDependencies = lib.optionals stdenv.hostPlatform.isLinux runtimeDeps;
      inherit buildInputs;
    } // lib.optionalAttrs ocrSupport {
      # ort-sys needs to find the system onnxruntime instead of trying
      # to fetch a prebuilt blob (which fails in Nix's sandbox).
      # `ORT_PREFER_DYNAMIC_LINK=1` keeps the linker in dynamic-link mode;
      # `ORT_LIB_LOCATION` points it at the nixpkgs build for header /
      # stub discovery. The binary ends up with a DT_NEEDED entry for
      # `libonnxruntime.so.1` — that's intentional, but for the no-bundle
      # release variants we strip the matching RPATH entry in `postFixup`
      # below so the loader falls through to the system path.
      ORT_LIB_LOCATION = "${onnxruntime}/lib";
      ORT_PREFER_DYNAMIC_LINK = "1";
    } // lib.optionalAttrs (ocrSupport && gpuCargoFeatures != "") {
      cargoExtraArgs = "--features ${gpuCargoFeatures}";
    } // lib.optionalAttrs (!ocrSupport) {
      # No `ocr` feature → kill the default features that pull sss_ocr.
      cargoExtraArgs = "--no-default-features";
    } // lib.optionalAttrs (stdenv.hostPlatform.isLinux && ocrSupport && !bundleRuntime) {
      # Strip the bundled onnxruntime / CUDA / ROCm store paths from the
      # final binary's RPATH. autoPatchelfHook (which runs IN fixupPhase
      # via postFixupHooks) bakes them in because the binary has a
      # DT_NEEDED for `libonnxruntime.so.1`. The no-bundle release model
      # wants the distro PM's `onnxruntime` package to provide the .so
      # via the system loader cache instead, so we run a custom phase
      # AFTER fixup (via `postPhases`) to drop the matching RPATH entry.
      # DT_NEEDED is preserved so ld.so still tries to load the lib —
      # just from `/etc/ld.so.cache` rather than the embedded RPATH.
      postPhases = [ "stripBundledRuntimeRpath" ];
      stripBundledRuntimeRpath = ''
        # nix store paths look like `/nix/store/<hash>-<name>-<ver>/lib`.
        # Match `<name>` against the ML runtime list. The `-` before the
        # name is the store-path separator, NOT a slash.
        for binary in $out/bin/*; do
          [ -f "$binary" ] || continue
          old=$(patchelf --print-rpath "$binary" 2>/dev/null || true)
          [ -n "$old" ] || continue
          new=$(printf '%s' "$old" \
            | tr ':' '\n' \
            | grep -vE -- '-(onnxruntime|cuda[-_]|cudnn|cublas|libcublas|libcudart|rocm|hip|miopen|hsa-runtime|nccl|nvjitlink|cufft|curand|cusolver|cusparse|libnpp|cudatoolkit)' \
            || true)
          new=$(printf '%s' "$new" | tr '\n' ':' | sed 's/:$//')
          if [ "$old" != "$new" ]; then
            patchelf --set-rpath "$new" "$binary"
            echo "  stripped ML runtime from RPATH: $binary"
          fi
        done
      '';
    };

    # Static docs site (Zola) — wired so `nix build .#site` produces a
    # publishable `public/` derivation. configReference is auto-generated;
    # releases.json defaults to an empty stub when no release fetched.
    site = import ./site.nix {
      inherit pkgs lib;
      configReference = import ./gen-docs.nix { inherit pkgs lib; };
      # Hero image is injected by CI from the latest release artifacts;
      # local `nix build .#site` produces a placeholder.
    };

    # sss artifacts
    sssDeps = craneLib.buildDepsOnly commonArgs;

    # Lambda for build packages with cached artifacts
    packageArgs = targetName:
      commonArgs
      // {
        cargoArtifacts = sssDeps;
        workspaceTargetName = targetName;
      };

    genBuild = name:  rec {
      pkg = craneLib.buildPackage (packageArgs name);
      app = {
        type = "app";
        program = "${pkg}${pkg.passthru.exePath or "/bin/${pkg.pname or pkg.name}"}";
      };
    };
    # Build packages and `nix run` apps
    sss = genBuild "sss";
    sssCode = genBuild "sss_code";

    # Per-binary release bundles via nix-bundle-app — only attached when
    # the flake was evaluated with `bundler` (the workspace's flake.nix
    # always supplies it; downstream consumers using ./nix directly may
    # not). Each `release-*` is a directory containing every (target,
    # format) artifact the current Nix `system` can produce + install.sh /
    # install.ps1 / INSTALL.md / SHA256SUMS, ready to upload as a GitHub
    # release.
    releaseBundles =
      if bundler == null
      then {}
      else import ./release.nix {
        inherit pkgs lib system bundler craneLib commonArgs variant;
        sssPkg = sss.pkg;
        sssCodePkg = sssCode.pkg;
      };
  in {
    # `nix run`
    apps = rec {
      code = sssCode.app;
      cli = sss.app;
      default = cli;
    };
    # `nix build`
    packages = rec {
      code = sssCode.pkg;
      cli = sss.pkg;
      docs = import ./gen-docs.nix { inherit pkgs lib; };
      inherit site;
      default = cli;
    } // releaseBundles;
    # `nix develop`
    #
    # The dev shell needs LD_LIBRARY_PATH wired up so binaries built
    # *inside* the shell can resolve libwayland-client.so.0 and friends
    # at runtime. crane.devShell propagates `buildInputs` to the build
    # but not necessarily to the runtime linker path on every system,
    # so we set it explicitly. PKG_CONFIG_PATH ensures pkg-config finds
    # every .pc the workspace links against.
    #
    # GPU note: when `cudaSupport` is on the dev shell pulls in the
    # CUDA-enabled onnxruntime (heavy first build, cached after) AND the
    # CUDA runtime libs (cudatoolkit / cudnn / cublas) so `cargo run`
    # inside the shell can actually drive CUDA — the `cli` package
    # itself stays CPU-only unless built as `cli-cuda`. NixOS users get
    # `libcuda.so` via `/run/opengl-driver/lib`; on other distros the
    # NVIDIA driver provides it through the system loader cache.
    devShells.default = let
      devOnnxruntime =
        if realCudaSupport
        then pkgs.onnxruntime.override { cudaSupport = true; }
        else onnxruntime;
      cudaDevDeps = lib.optionals realCudaSupport [
        pkgs.cudaPackages.cudatoolkit
        pkgs.cudaPackages.cudnn
        pkgs.cudaPackages.libcublas
      ];
      cudaDevLibPath = lib.optionals (realCudaSupport && stdenv.hostPlatform.isLinux) [
        # NVIDIA proprietary driver lives outside the Nix store. On NixOS
        # the open-source loader symlinks the driver into this prefix;
        # adding it to LD_LIBRARY_PATH lets `libcuda.so.1` resolve.
        "/run/opengl-driver/lib"
      ];
      ocrLibPath = lib.optionals ocrSupport [ devOnnxruntime ];
    in craneLib.devShell ({
      packages = with pkgs; [
          toolchain
          pkg-config
          cargo-edit
          claude-code
          cargo-release
        ] ++ buildInputs ++ cudaDevDeps;
      LD_LIBRARY_PATH = lib.concatStringsSep ":" (
        [ (lib.makeLibraryPath (runtimeDeps ++ cudaDevDeps ++ ocrLibPath)) ]
        ++ cudaDevLibPath
      );
      PKG_CONFIG_PATH = lib.concatStringsSep ":" [
        "${pkgs.fontconfig.dev}/lib/pkgconfig"
        "${pkgs.libxkbcommon.dev}/lib/pkgconfig"
        "${pkgs.wayland.dev}/lib/pkgconfig"
        "${pkgs.dbus.dev}/lib/pkgconfig"
        "${pkgs.libxcb.dev}/lib/pkgconfig"
      ];
    } // lib.optionalAttrs ocrSupport {
      # See `commonArgs` above — sss_ocr needs onnxruntime via ort's
      # dynamic-link path, and the same env vars must be visible to
      # `cargo build` inside the dev shell. Point at the CUDA-enabled
      # build when the host is set up for it.
      ORT_LIB_LOCATION = "${devOnnxruntime}/lib";
      ORT_PREFER_DYNAMIC_LINK = "1";
    });
  }
