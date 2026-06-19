+++
title = "Flags"
description = "Complete CLI reference for sss_code."
weight = 20
+++

## Input

<table class="flag-table">
<thead><tr><th>Argument</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>&lt;content&gt;</code></td><td>path / <code>-</code></td><td>—</td><td>Source file path, or <code>-</code> for stdin.</td></tr>
<tr><td><code>--extension / -e</code></td><td>string</td><td>—</td><td>Force language by extension. Required when reading from stdin.</td></tr>
<tr><td><code>--extra-syntaxes</code></td><td>path</td><td>—</td><td>Additional folder of <code>.sublime-syntax</code> files to load.</td></tr>
</tbody>
</table>

## Themes

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--theme</code></td><td>string</td><td><code>base16-ocean.dark</code></td><td>Theme file path or embedded theme name.</td></tr>
<tr><td><code>--vim-theme</code></td><td>string</td><td>—</td><td>Theme from vim highlights. Format: <code>group,bg,fg,style;...</code></td></tr>
<tr><td><code>--list-themes / -L</code></td><td>bool</td><td>false</td><td>List available themes and exit.</td></tr>
<tr><td><code>--list-file-types / -l</code></td><td>bool</td><td>false</td><td>List supported file types and exit.</td></tr>
<tr><td><code>--build-cache</code></td><td>path</td><td>—</td><td>Generate a syntax/theme cache to the given directory and exit.</td></tr>
</tbody>
</table>

## Layout

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--code-background</code></td><td>color/grad/path</td><td><code>#323232</code></td><td><code>#RRGGBBAA</code>, <code>h;c1;c2</code>, <code>v;c1;c2</code>, or a wallpaper image path.</td></tr>
<tr><td><code>--lines</code></td><td>range</td><td><code>..</code></td><td>Crop to a line range, e.g. <code>10..40</code>.</td></tr>
<tr><td><code>--highlight-lines</code></td><td>range</td><td><code>..</code></td><td>Emphasise a sub-range inside the crop.</td></tr>
<tr><td><code>--line-numbers / -n</code></td><td>bool</td><td>true</td><td>Render line numbers in the gutter.</td></tr>
<tr><td><code>--tab-width</code></td><td>u8</td><td>4</td><td>Tabs render as this many spaces.</td></tr>
<tr><td><code>--indent-chars / -i</code></td><td>chars</td><td>—</td><td>Comma-separated indent character set (whitespace markers).</td></tr>
<tr><td><code>--hidden-chars</code></td><td>kv list</td><td>—</td><td>Show hidden chars: <code>space:·,tab:»,eol:¶</code>.</td></tr>
</tbody>
</table>

## Shared

All `[general]` keys from the [config reference](/docs/config-reference/) apply: padding, fonts, window chrome, shadow, background gradient, watermark.
