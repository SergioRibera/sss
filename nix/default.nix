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
      sha256 = "sha256-Qxt8XAuaUR2OMdKbN4u8dBJOhSHxS+uS06Wl9+flVEk=";
    };

    # crane: cargo and artifacts manager
    craneLib = crane.overrideToolchain toolchain;

    # buildInputs for SSS — the runtime / linker dependencies the
    # workspace's crates pull in on Linux.
    #
    # * wayland (libwayland-client.so.0)
    #     wayland-client / wayland-protocols / wayland-protocols-wlr /
    #     wayland-cursor all link against this dynamically via
    #     DT_NEEDED (the dlopen feature is deliberately off — see the
    #     comment in crates/sss_capture_ui/Cargo.toml). Required at
    #     build time so rustc can find -lwayland-client *and* at run
    #     time so the dynamic linker can resolve it.
    # * libxkbcommon: used by winit / arboard fallbacks on Wayland.
    # * xorg.libxcb / xorg.libX11: x11rb (sss_capture's X11 backend)
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
      libxkbcommon.dev
      libxkbcommon
      xorg.libxcb
      xorg.libX11
      xorg.libXi
      xorg.libXcursor
      xorg.libXrandr
      xorg.libxcb.dev
      wayland
      wayland-protocols
      wayland-scanner
      dbus.dev
    ];

    # Base args, need for build all crate artifacts and caching this for late builds
    commonArgs = {
      src = ./..;
      doCheck = false;
      nativeBuildInputs =
        [ pkgs.pkg-config pkgs.wayland-scanner ]
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
    #
    # The dev shell needs LD_LIBRARY_PATH wired up so binaries built
    # *inside* the shell can resolve libwayland-client.so.0 and friends
    # at runtime. crane.devShell propagates `buildInputs` to the build
    # but not necessarily to the runtime linker path on every system,
    # so we set it explicitly. PKG_CONFIG_PATH ensures pkg-config finds
    # every .pc the workspace links against.
    devShells.default = craneLib.devShell {
      packages = with pkgs; [
          toolchain
          pkg-config
          oranda
          cargo-dist
          cargo-release
        ] ++ buildInputs;
      LD_LIBRARY_PATH = lib.makeLibraryPath (with pkgs; [
        wayland
        libxkbcommon
        xorg.libxcb
        xorg.libX11
        xorg.libXi
        xorg.libXcursor
        xorg.libXrandr
        fontconfig
        dbus
        stdenv.cc.cc.lib
      ]);
      PKG_CONFIG_PATH = lib.concatStringsSep ":" [
        "${pkgs.fontconfig.dev}/lib/pkgconfig"
        "${pkgs.libxkbcommon.dev}/lib/pkgconfig"
        "${pkgs.wayland.dev}/lib/pkgconfig"
        "${pkgs.dbus.dev}/lib/pkgconfig"
        "${pkgs.xorg.libxcb.dev}/lib/pkgconfig"
      ];
    };
  }
