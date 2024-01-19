{}:
let
  pkgs = import <nixpkgs> { };
  platform = {
    "x86_64-linux" = "x86_64-unknown-linux-gnu";
    "x86_64-darwin" = "x86_64-apple-darwin";
    "aarch64-darwin" = "aarch64-apple-darwin";
  }."${pkgs.stdenv.hostPlatform.system}";
  hash_sss_code = {
    "x86_64-linux" = "1gs9614mdsdx5w9q2cvkdj34jh86jn68xs9l8xvc4rvarp20z4rh";
    "x86_64-darwin" = "14i3ycyq4pcw2sp8hlai4l37xmvf9m865sw19030x6aq86137npg";
    "aarch64-darwin" = "1s4dic01dfapx3j6m2ihk7clcpwiiv5kfnyzrk4fdhz81qyj84xg";
  }."${pkgs.stdenv.hostPlatform.system}";
in
pkgs.stdenv.mkDerivation {
    name = "sss_code";
    version = "0.1.6";
    src = fetchTarball {
      url = "https://github.com/SergioRibera/sss/releases/download/sss_code/v0.1.6/sss_code-${platform}.tar.xz";
      sha256 = hash_sss_code;
    };
    buildInputs = with pkgs; [ fontconfig ];
    installPhase = ''
      mkdir -p $out/bin
      cp sss_code $out/bin/
    '';
}
