{
  description = "Standar cross compile flake for Rust Lang Projects";
  inputs.nixpkgs.url = "github:NixOS/nixpkgs/nixos-unstable";
  outputs =
    { self
    , nixpkgs
    ,
    }:
    let
      # inherit (pkgs) lib;
      genSystems = nixpkgs.lib.genAttrs [
        "x86_64-linux"
        "aarch64-linux"
      ];
      pkgsFor = nixpkgs.legacyPackages;

      # Toolchain
      # toolchain = with fenix.packages.${system};
      #   fromToolchainFile {
      #     file = ./rust-toolchain.toml;
      #     sha256 = "sha256-U2yfueFohJHjif7anmJB5vZbpP7G6bICH4ZsjtufRoU=";
      #   };
      # craneLib = crane.lib.${system}.overrideToolchain toolchain;

      # # src = craneLib.cleanCargoSource (craneLib.path ./.);
      # buildInputs = with pkgs; [
      #   pkg-config
      #   fontconfig

      #   dbus
      #   wayland
      #   wayland-protocols
      #   libxkbcommon
      #   xorg.libXcursor
      #   xorg.libxcb
      #   xorg.libX11
      #   xorg.libXi
      #   xorg.libXrandr
      # ];
    in
    {
      overlays.default = _: prev: {
        sss = prev.callPackage ./crates/sss_cli { };
        sss_code = prev.callPackage ./crates/sss_code { };
      };
      packages = genSystems (system: self.overlays.default null pkgsFor.${system});

      #  inputs.flake-parts.lib.mkFlake {
      #  # nix develop
      #  devShells.default = craneLib.devShell {
      #    packages = with pkgs; [
      #      toolchain
      #      oranda
      #      cargo-dist
      #      cargo-release
      #    ] ++ buildInputs;
      #    LD_LIBRARY_PATH = lib.makeLibraryPath buildInputs;
      #  };
      #};
    };
}
