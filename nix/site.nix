{
  pkgs,
  lib,
  configReference ? null,
  releases ? null,
  hero ? null,
}:
let
  # Variable-weight woff2s pinned from the fontsource CDN. Pinned-by-hash so
  # nix builds stay reproducible without dragging in nixpkgs' huge
  # `google-fonts` derivation just for two files.
  manropeWoff2 = pkgs.fetchurl {
    url    = "https://cdn.jsdelivr.net/npm/@fontsource-variable/manrope@5.0.0/files/manrope-latin-wght-normal.woff2";
    sha256 = "1rq984i51yw6rlgjpgc8krz70l5vr7lgm7wzy594drzxvha43ghl";
  };
  jbmonoWoff2 = pkgs.fetchurl {
    url    = "https://cdn.jsdelivr.net/npm/@fontsource-variable/jetbrains-mono@5.0.0/files/jetbrains-mono-latin-wght-normal.woff2";
    sha256 = "10jxg3xg4b9ni3qxphipx1xakg4admbw0c17qyn00hbghh08c66q";
  };
in
pkgs.runCommand "sss-site"
  {
    nativeBuildInputs = [ pkgs.zola pkgs.coreutils ];
    src = ../docs-site;
  }
  ''
    cp -r $src site
    chmod -R u+w site

    # ---- Fonts ----
    mkdir -p site/static/fonts
    cp ${manropeWoff2} site/static/fonts/Manrope-Variable.woff2
    cp ${jbmonoWoff2}  site/static/fonts/JetBrainsMono-Variable.woff2

    # ---- Hero image ----
    ${lib.optionalString (hero != null) ''
      cp ${hero} site/static/img/hero-screenshot.png
    ''}

    # ---- Auto-generated config reference ----
    # gen-docs.nix emits a frontmatter-less .md; splice it under the
    # existing TOML frontmatter in the placeholder so Zola accepts it.
    ${lib.optionalString (configReference != null) ''
      ref=site/content/docs/config-reference.md
      {
        echo "+++"
        echo 'title = "Config reference"'
        echo 'description = "Every config.toml key, generated from the Nix option modules."'
        echo "weight = 50"
        echo "+++"
        echo
        cat ${configReference}
      } > "$ref.new"
      mv "$ref.new" "$ref"
    ''}

    # ---- Latest release manifest ----
    ${lib.optionalString (releases != null) ''
      cp ${releases} site/data/releases.json
    ''}

    cd site
    zola build --output-dir $out
  ''
