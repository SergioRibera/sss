{}:
let
  pkgs = import <nixpkgs> { };
  platform = {
    "x86_64-linux" = "x86_64-unknown-linux-musl";
    "x86_64-darwin" = "x86_64-apple-darwin";
    "aarch64-darwin" = "aarch64-apple-darwin";
  }."${pkgs.stdenv.hostPlatform.system}";
  hash_sss = {
    "x86_64-linux" = "0sbrny0a47hyg8z6266xw77h27slamlqg3kdcrimkn2xrn9341wh";
    "x86_64-darwin" = "00a66gi5l71z0c6xgcswcnlh3m9d5n1hrgkmcgibnr3mxipxgrpm";
    "aarch64-darwin" = "1dw15gjihr898l1apgwjcqx1dk2b227rhcrcq4qclxyd9wyg2861";
  }."${pkgs.stdenv.hostPlatform.system}";
in
pkgs.stdenv.mkDerivation {
  name = "sss";
  version = "0.1.2";
  src = fetchTarball {
    url = "https://github.com/SergioRibera/sss/releases/download/sss_cli/v0.1.2/sss_cli-${platform}.tar.xz";
    sha256 = hash_sss;
  };
  buildInputs = with pkgs; [
    fontconfig
    dbus
    wayland
    wayland-protocols
    libxkbcommon
    xorg.libXcursor
    xorg.libxcb
    xorg.libX11
    xorg.libXi
    xorg.libXrandr
  ];
  installPhase = ''
    mkdir -p $out/bin
    cp sss $out/bin/
  '';
}
