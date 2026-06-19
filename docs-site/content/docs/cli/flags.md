+++
title = "Flags"
description = "Complete CLI reference for sss — every flag, type, and default."
weight = 20
+++

## Capture targeting

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--current</code></td><td>bool</td><td>false</td><td>Capture the monitor under the cursor right now.</td></tr>
<tr><td><code>--show-cursor</code></td><td>bool</td><td>false</td><td>Composite the mouse cursor into the frame.</td></tr>
<tr><td><code>--screen</code></td><td>bool</td><td>false</td><td>Open the monitor selector (or capture current with <code>--current</code>).</td></tr>
<tr><td><code>--screen-id &lt;id&gt;</code></td><td>name/id</td><td>—</td><td>Pick a screen directly. Omit value to open the picker.</td></tr>
<tr><td><code>--area &lt;spec&gt;</code></td><td>"X,Y WxH"</td><td>—</td><td>Pick an area. Omit value to open the interactive selector.</td></tr>
<tr><td><code>--window &lt;spec&gt;</code></td><td>id/title</td><td>—</td><td>Pick a window by id or title substring. Omit value to open the picker.</td></tr>
<tr><td><code>--interactive</code></td><td>bool</td><td>false</td><td>Force the interactive selector even when a target was specified.</td></tr>
</tbody>
</table>

## Annotation overlay

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--no-toolbar</code></td><td>bool</td><td>false</td><td>Hide the annotation toolbar (slurp-mode behaviour).</td></tr>
<tr><td><code>--remember-last-selection</code></td><td>bool</td><td>false</td><td>Persist the last selected area and pre-seed it on next run.</td></tr>
</tbody>
</table>

## Backend selection

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--capture-backend</code></td><td>enum</td><td><code>auto</code></td><td>One of <code>auto</code>, <code>wayland</code>, <code>portal</code>, <code>x11</code>, <code>windows</code>, <code>macos</code>.</td></tr>
</tbody>
</table>

## OCR

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--ocr</code></td><td>bool</td><td>true</td><td>Enable or disable the OCR overlay.</td></tr>
<tr><td><code>--ocr-gpu</code></td><td>enum</td><td><code>auto</code></td><td>OCR execution provider: <code>auto</code>, <code>cpu</code>, <code>cuda</code>, <code>tensorrt</code>, <code>coreml</code>, <code>directml</code>, <code>openvino</code>, <code>webgpu</code>.</td></tr>
</tbody>
</table>

## Misc

<table class="flag-table">
<thead><tr><th>Flag</th><th>Type</th><th>Default</th><th>Description</th></tr></thead>
<tbody>
<tr><td><code>--verbose / -v</code></td><td>bool</td><td>false</td><td>Bump log level to <code>info</code>.</td></tr>
<tr><td><code>--config &lt;path&gt;</code></td><td>path</td><td>XDG</td><td>Override the default config file location.</td></tr>
</tbody>
</table>

{{ callout(kind="info", body="Use `sss --help` for the version bundled with your binary. Flags are stable within a minor release.") }}
