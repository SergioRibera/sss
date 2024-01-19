{}:
let
  pkgs = import <nixpkgs> { };
  platform = {
    "x86_64-linux" = "x86_64-unknown-linux-gnu";
    "x86_64-darwin" = "x86_64-apple-darwin";
    "aarch64-darwin" = "aarch64-apple-darwin";
  }."${pkgs.stdenv.hostPlatform.system}";
  hash_sss = {
    "x86_64-linux" = "179p06qm5c9vw8kdqzmy0grffwl23044anhrinj8k2clbaacb00z";
    "x86_64-darwin" = "146c5laxhh2h523cizbck9cwshqdb2nkj91m2mzrqilv5gpam3yl";
    "aarch64-darwin" = "12v9ay0b70rxbdch62k39kpcdlxc8nlmsic5f61q8rkzxcpdyh8n";
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
