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
      sha256 = "sha256-3jVIIf5XPnUU1CRaTyAiO0XHVbJl12MSx3eucTXCjtE=";
    };
    # crane: cargo and artifacts manager
    craneLib = crane.${system}.overrideToolchain toolchain;
    # cranix: extends crane building system with workspace bin building and Mold + Cranelift integrations
    cranixLib = craneLib.overrideScope (cranix.${system}.craneOverride);

    # buildInputs for SSS
    buildInputs = with pkgs; [
      fontconfig.dev
      libxkbcommon.dev
      xorg.libxcb
    ];

    # Base args, need for build all crate artifacts and caching this for late builds
    commonArgs = {
      src = ./..;
      doCheck = false;
      nativeBuildInputs =
        [pkgs.pkg-config]
        ++ lib.optionals stdenv.buildPlatform.isDarwin [
          pkgs.libiconv
        ];
      inherit buildInputs;
    };

    # sss artifacts
    sssDeps = cranixLib.buildCranixDepsOnly commonArgs;

    # Lambda for build packages with cached artifacts
    packageArgs = targetName:
      commonArgs
      // {
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_LINKER = "${stdenv.cc.targetPrefix}cc";
        CARGO_TARGET_AARCH64_UNKNOWN_LINUX_GNU_RUNNER = "qemu-aarch64";
        HOST_CC = "${stdenv.cc.nativePrefix}cc";
        cargoArtifacts = sssDeps;
        workspaceTargetName = targetName;
      };

    # Build packages and `nix run` apps
    sss = cranixLib.buildCranixBundle (packageArgs "sss");
    sssCode = cranixLib.buildCranixBundle (packageArgs "sss_code");
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
      default = cli;
    };
    # `nix develop`
    devShells.default = cranixLib.devShell {
      packages = with pkgs;
        [
          toolchain
          pkg-config
          oranda
          cargo-dist
          cargo-release
        ]
        ++ buildInputs;
      PKG_CONFIG_PATH = "${pkgs.fontconfig.dev}/lib/pkgconfig";
    };
  }
