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
      docs = import ./gen-docs.nix { inherit pkgs lib; };
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
          cargo-edit
          cargo-dist
          cargo-release
        ] ++ buildInputs;
      LD_LIBRARY_PATH = lib.makeLibraryPath runtimeDeps;
      PKG_CONFIG_PATH = lib.concatStringsSep ":" [
        "${pkgs.fontconfig.dev}/lib/pkgconfig"
        "${pkgs.libxkbcommon.dev}/lib/pkgconfig"
        "${pkgs.wayland.dev}/lib/pkgconfig"
        "${pkgs.dbus.dev}/lib/pkgconfig"
        "${pkgs.libxcb.dev}/lib/pkgconfig"
      ];
    };
  }
