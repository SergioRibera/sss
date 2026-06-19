+++
title = "Examples"
description = "Recipes for diff renders, terminal sessions, README assets, blog snippets."
weight = 40
+++

## Blog snippet

```bash
sss_code \
  --theme base16-tomorrow-night.dark \
  --code-background "v;#1d1f21;#0d0f10" \
  --lines 1..40 \
  src/lib.rs > blog/lib-snippet.png
```

## Diff hunk

Pipe the diff straight in:

```bash
git diff src/main.rs | sss_code -e diff > diff.png
```

## Terminal session

```bash
script -q -c "uptime && ls -la" /tmp/session.log
sss_code -e sh /tmp/session.log > terminal.png
```

## README hero image

Render the first 60 lines of your project's `main.rs` with a wallpaper background:

```bash
sss_code \
  --lines 1..60 \
  --code-background ~/Pictures/wallpapers/abstract.jpg \
  src/main.rs > assets/hero.png
```

## Multi-language gallery

```bash
for f in samples/*; do
  ext="${f##*.}"
  out="gallery/${f##*/}.png"
  sss_code --extension "$ext" --theme base16-ocean.dark "$f" > "$out"
done
```

## Highlight a refactor target

```bash
sss_code \
  --lines 80..160 \
  --highlight-lines 110..125 \
  --theme base16-github.dark \
  src/router.rs > router-hotpath.png
```

The cropped range surrounds the hot path; the highlight range pops it visually.

## Pair with `sss_cli`

`sss_code` renders code; `sss_cli` annotates the result. Pipe one into the other:

```bash
sss_code src/main.rs | sss --area-from-stdin --output annotated.png
```

(Requires `sss >= 0.2`.)
