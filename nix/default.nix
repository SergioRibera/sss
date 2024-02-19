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
    cranix,
    fenix,
    stdenv ? pkgs.stdenv,
    ...
  }: let
    # fenix: rustup replacement for reproducible builds
    # toolchain = fenix.${system}.fromToolchainFile { dir = ./..; };
    toolchain = fenix.${system}.fromToolchainFile {
      file = ./../rust-toolchain.toml;
      sha256 = "sha256-U2yfueFohJHjif7anmJB5vZbpP7G6bICH4ZsjtufRoU=";
    };
    # crane: cargo and artifacts manager
    craneLib = crane.${system}.overrideToolchain toolchain;
    # cranix: extends crane building system with workspace bin building and Mold + Cranelift integrations
    cranixLib = craneLib.overrideScope' (cranix.${system}.craneOverride);

    # buildInputs for SSS
    buildInputs = with pkgs; [
      stdenv.cc.cc.lib
      fontconfig.dev
      libxkbcommon.dev
      xorg.libxcb
      wayland
      wayland-protocols
      xorg.libX11
      xorg.libXcursor
      xorg.libXrandr
      xorg.libXi
    ];

    # Base args, need for build all crate artifacts and caching this for late builds
    deps = {
      nativeBuildInputs = with pkgs;
        [
          pkg-config
          autoPatchelfHook
        ]
        ++ lib.optionals stdenv.buildPlatform.isDarwin [
          pkgs.libiconv
        ]
        ++ lib.optionals stdenv.buildPlatform.isLinux [
          pkgs.libxkbcommon.dev
        ];
      runtimeDependencies = with pkgs;
        lib.optionals stdenv.isLinux [
          wayland
          libxkbcommon
        ];
      inherit buildInputs;
    };

    # Lambda for build packages with cached artifacts
    commonArgs = targetName:
      deps
      // {
        src = lib.cleanSourceWith {
          src = craneLib.path ./..;
          filter = craneLib.filterCargoSources;
        };
        doCheck = false;
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${stdenv.cc.targetPrefix}cc";
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER = "qemu-aarch64";
        HOST_CC = "${stdenv.cc.nativePrefix}cc";
        workspaceTargetName = targetName;
      };

    # Build packages and `nix run` apps
    sss = cranixLib.buildCranixBundle (commonArgs "sss");
    sssCode = cranixLib.buildCranixBundle (commonArgs "sss_code");
    sssLauncher = cranixLib.buildCranixBundle (commonArgs "sss_launcher");
  in {
    # `nix run`
    apps = rec {
      code = sssCode.app;
      cli = sss.app;
      launcher = sssLauncher.app;
      default = cli;
    };
    # `nix build`
    packages = rec {
      code = sssCode.pkg;
      cli = sss.pkg;
      launcher = sssLauncher.pkg;
      default = cli;
    };
    # `nix develop`
    devShells.default = cranixLib.devShell {
      packages = with pkgs;
        [
          toolchain
          oranda
          cargo-dist
          cargo-release
        ]
        ++ buildInputs;
      PKG_CONFIG_PATH = "${pkgs.fontconfig.dev}/lib/pkgconfig";
    };
  }
