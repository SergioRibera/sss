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
    # GPU execution-provider knobs for sss_ocr. Two independent things:
    #
    # 1. **Cargo feature** (`cudaSupport` / `coreMLSupport` / ...): compiles
    #    the EP bindings into the produced binary. Cheap (no SDK build),
    #    just toggles Rust code paths. The binary advertises support for
    #    that EP at runtime; if `libonnxruntime` doesn't actually ship it,
    #    ORT silently falls back to CPU.
    #
    # 2. **Lib swap** (`cudaRuntime` / `rocmRuntime`): rebuilds `onnxruntime`
    #    with the EP fused into the `.so` itself. EXPENSIVE — pulls in the
    #    CUDA toolkit, builds onnxruntime from source, easily hours of
    #    compile time and gigabytes of store.
    #
    # Defaults flip ONLY the cargo features per platform, so a stock
    # `nix build .#cli` is fast and the resulting binary recognises the
    # right EP for its host. Users wanting the full GPU lib pick the
    # purpose-built variants (`cli-cuda`, `cli-cuda-tensorrt`, `cli-rocm`)
    # which set the runtime flag in addition.
    # Master OCR toggle. When false the workspace builds with
    # `--no-default-features` (sss_cli `ocr` feature off), the produced
    # binary has zero references to sss_ocr / onnxruntime, and we skip
    # bundling libonnxruntime + the CUDA stack. Distro packages can still
    # offer OCR via the system's onnxruntime — declared per-distro in
    # `nix/release.nix` `info.depends` as a recommendation.
    ocrSupport ? true,
    cudaSupport ? (ocrSupport && stdenv.hostPlatform.isLinux && stdenv.hostPlatform.isx86_64),
    coreMLSupport ? (ocrSupport && stdenv.hostPlatform.isDarwin),
    directMLSupport ? (ocrSupport && stdenv.hostPlatform.isWindows),
    tensorrtSupport ? false,
    openvinoSupport ? false,
    # Default to swapping libonnxruntime for the CUDA-enabled build
    # whenever the cargo feature is on. Without this the binary
    # advertises CUDA but the .so doesn't ship the EP, so ORT logs
    # "CUDA execution provider is not enabled in this build" and falls
    # back to CPU silently. The heavy paths come from
    # `cache.nixos-cuda.org` (declared in flake.nix `nixConfig`); if the
    # substituter misses, builds fall back to compiling onnxruntime from
    # source — set `cudaRuntime = false` explicitly to skip.
    cudaRuntime ? cudaSupport,
    rocmRuntime ? false,
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
      # onnxruntime for sss_ocr — the ort crate dlopens libonnxruntime
      # at runtime when `ORT_PREFER_DYNAMIC_LINK=1` is set (see below).
      # Substituted with a CUDA / ROCm build when those flags are on.
      onnxruntime
    ];

    # Libraries that the workspace loads via dlopen at runtime. These
    # are not present as DT_NEEDED in the produced ELF (libloading
    # resolves them by SONAME at runtime), so autoPatchelfHook has no
    # way of inferring them — we hand them in explicitly and the hook
    # appends each one's `/lib` to the binary's RPATH.
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
    ] ++ lib.optionals ocrSupport [
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
      # `ORT_PREFER_DYNAMIC_LINK=1` flips ort to dlopen-at-runtime mode;
      # `ORT_LIB_LOCATION` points it at the nixpkgs build.
      ORT_LIB_LOCATION = "${onnxruntime}/lib";
      ORT_PREFER_DYNAMIC_LINK = "1";
    } // lib.optionalAttrs (ocrSupport && gpuCargoFeatures != "") {
      cargoExtraArgs = "--features ${gpuCargoFeatures}";
    } // lib.optionalAttrs (!ocrSupport) {
      # No `ocr` feature → kill the default features that pull sss_ocr.
      cargoExtraArgs = "--no-default-features";
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
        inherit pkgs lib system bundler craneLib commonArgs ocrSupport;
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
