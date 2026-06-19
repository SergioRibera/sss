#!/usr/bin/env bash
# Fetch latest GitHub release per binary (sss_cli, sss_code) and write a
# classified asset manifest to docs-site/data/releases.json. Used by CI and
# locally for dev preview. Requires `curl` and `jq` only.
#
# Usage:
#   scripts/fetch-releases.sh                 # default output path
#   scripts/fetch-releases.sh path/to/out.json
#   REPO=owner/name scripts/fetch-releases.sh

set -euo pipefail

REPO="${REPO:-SergioRibera/sss}"
OUT="${1:-docs-site/data/releases.json}"
TMP="$(mktemp -t releases.XXXXXX.json)"
trap 'rm -f "$TMP"' EXIT

auth=()
if [ -n "${GH_TOKEN:-}" ]; then
  auth=(-H "Authorization: Bearer $GH_TOKEN")
fi

curl -fsSL "${auth[@]}" \
  -H "Accept: application/vnd.github+json" \
  "https://api.github.com/repos/$REPO/releases?per_page=30" > "$TMP"

jq '
  def filesize(s):
    if   s >= 1073741824 then "\(((s / 1073741824) * 10 | floor) / 10) GB"
    elif s >= 1048576    then "\((s / 1048576) | floor) MB"
    elif s >= 1024       then "\((s / 1024) | floor) KB"
    else "\(s) B" end;

  def os_of(n):
    if   (n | test("\\.deb$|\\.rpm$|\\.AppImage$|\\.pkg\\.tar\\.zst$|-linux\\.|-unknown-linux"))     then "linux"
    elif (n | test("\\.dmg$|-darwin\\.|-apple-darwin|-macos\\.|\\.rb$"))                              then "macos"
    elif (n | test("\\.msi$|-pc-windows|-windows-msvc"))                                              then "windows"
    else "other" end;

  def binary_of(n):
    if   (n | test("sss_code")) then "sss_code"
    elif (n | test("sss_cli"))  then "sss"
    else "" end;

  def arch_of(n):
    if   (n | test("aarch64|arm64")) then "aarch64"
    elif (n | test("x86_64|amd64"))  then "x86_64"
    elif (n | test("i686|i386"))     then "x86"
    elif (n | test("armv7"))         then "armv7"
    else "any" end;

  def format_of(n):
    if   (n | test("\\.deb$"))             then "deb"
    elif (n | test("\\.rpm$"))             then "rpm"
    elif (n | test("\\.AppImage$"))        then "AppImage"
    elif (n | test("\\.pkg\\.tar\\.zst$")) then "Arch"
    elif (n | test("\\.tar\\.xz$"))        then "tar.xz"
    elif (n | test("\\.tar\\.gz$"))        then "tar.gz"
    elif (n | test("\\.tar\\.zst$"))       then "tar.zst"
    elif (n | test("\\.dmg$"))             then "dmg"
    elif (n | test("\\.pkg$"))             then "pkg"
    elif (n | test("\\.msi$"))             then "msi"
    elif (n | test("\\.zip$"))             then "zip"
    elif (n | test("\\.rb$"))              then "brew"
    else "" end;

  # Drop checksum sidecars, manifests, source archives, installer scripts and
  # the brew formula (brew is shown in the package-manager tab as a command).
  def keep:
    (test("\\.sha256$|^dist-manifest|^source\\.|installer\\.(sh|ps1)$|\\.rb$")) | not;

  def latest_for(prefix):
    [ .[] | select(.tag_name | startswith(prefix + "/")) ]
    | sort_by(.published_at) | reverse | first;

  ([ latest_for("sss_cli"), latest_for("sss_code") ] | map(select(. != null))) as $rels
  | {
      generated_at: (now | todate),
      tags: [ $rels[].tag_name ],
      assets: ([ $rels[] | .tag_name as $t | .assets[] | select(.name | keep) | {
        name,
        tag: $t,
        url: .browser_download_url,
        size: .size,
        size_human: filesize(.size),
        os: os_of(.name),
        binary: binary_of(.name),
        arch: arch_of(.name),
        format: format_of(.name)
      } ] | map(select(.os != "other" and .binary != "" and .format != "")))
    }
' "$TMP" > "$OUT"

count=$(jq '.assets | length' "$OUT")
tags=$(jq -r '.tags | join(", ")' "$OUT")
echo "Wrote $count assets ($tags) -> $OUT"
