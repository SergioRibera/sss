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
    stdenv ? pkgs.stdenv,
    ...
  }: let
    # fenix: rustup replacement for reproducible builds
    toolchain = fenix.${system}.fromToolchainFile {
      file = ./../rust-toolchain.toml;
      sha256 = "sha256-yMuSb5eQPO/bHv+Bcf/US8LVMbf/G/0MSfiPwBhiPpk=";
    };

    # crane: cargo and artifacts manager
    craneLib = crane.overrideToolchain toolchain;

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
        [ pkgs.pkg-config ]
        ++ lib.optionals stdenv.buildPlatform.isDarwin [
          pkgs.libiconv
        ];
      inherit buildInputs;
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
    devShells.default = craneLib.devShell {
      packages = with pkgs; [
          toolchain
          pkg-config
          oranda
          cargo-dist
          cargo-release
        ] ++ buildInputs;
      PKG_CONFIG_PATH = "${pkgs.fontconfig.dev}/lib/pkgconfig";
    };
  }
