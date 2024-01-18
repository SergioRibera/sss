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
  hash_sss_code = {
    "x86_64-linux" = "0zgn8hrhjrdlxij5mgm8wcnfzigas53asmfwnrnxfxpgd5bi775m";
    "x86_64-darwin" = "1nxykijmp6rpm0s0yxzqx75q3yvsd52p6dssz5wddfjsxpvh2gva";
    "aarch64-darwin" = "14il7r1i0zx62wf8r7y7ayxhml33ycrp1gn0jxzfj5662mmbpgig";
  }."${pkgs.stdenv.hostPlatform.system}";
in
{
  sss = pkgs.stdenv.mkDerivation {
    name = "sss";
    version = "0.1.1";
    buildInputs = with pkgs; [ fontconfig ];
    src = fetchTarball {
      url = "https://github.com/SergioRibera/sss/releases/download/sss_cli/v0.1.1/sss_cli-${platform}.tar.xz";
      sha256 = hash_sss;
    };
    installPhase = ''
      mkdir -p $out/bin
      cp sss $out/bin/
    '';
  };

  sss_code = pkgs.stdenv.mkDerivation {
    name = "sss_code";
    version = "0.1.5";
    buildInputs = with pkgs; [ fontconfig ];
    src = fetchTarball {
      url = "https://github.com/SergioRibera/sss/releases/download/sss_code/v0.1.5/sss_code-${platform}.tar.xz";
      sha256 = hash_sss_code;
    };
    installPhase = ''
      mkdir -p $out/bin
      cp sss_code $out/bin/
    '';
  };
}
