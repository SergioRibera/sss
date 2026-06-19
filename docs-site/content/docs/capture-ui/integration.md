+++
title = "Integration"
description = "Driving the selector from another GUI, an async runtime, or a daemon."
weight = 30
+++

## Thread model

`Selector::run()` blocks the calling thread and drives an `egui` + `wgpu` event loop. It must run on the **main thread** on macOS (winit requirement) and any thread on Linux.

If your app already has an event loop, spawn `sss_capture_ui` in a fresh process or a dedicated thread.

## From a Tauri app

Tauri owns the main thread on macOS, so call `Selector` in a worker thread on Linux/Windows, and spawn a sidecar process on macOS:

```rust
#[tauri::command]
async fn pick_region() -> Result<PathBuf, String> {
    let path = tauri::async_runtime::spawn_blocking(|| {
        let out = Selector::builder()
            .mode(SelectorMode::Area)
            .build()
            .map_err(|e| e.to_string())?
            .run()
            .map_err(|e| e.to_string())?;

        let path = std::env::temp_dir().join("pick.png");
        out.write_png(&path).map_err(|e| e.to_string())?;
        Ok::<_, String>(path)
    })
    .await
    .map_err(|e| e.to_string())??;

    Ok(path)
}
```

## From a daemon (long-running process)

A long-running process that opens a fresh selector on demand:

```rust
loop {
    wait_for_trigger().await;
    let outcome = std::thread::scope(|s| {
        s.spawn(|| Selector::builder().mode(SelectorMode::Area).build()?.run())
            .join()
            .unwrap()
    })?;
    push_to_history(outcome);
}
```

The selector consumes its own thread; the outer loop stays clean.

## Sharing state between captures

The selector reads `remember_last_selection` from `UiConfig`. Set it and persist the area between runs:

```rust
let mut config = UiConfig::default();
config.remember_last_selection = true;
```

The next `run()` invocation will pre-seed the previous area selection. The shared state lives in `$XDG_DATA_HOME/sss/last-selection.json`.

## Custom OCR backend

Plug a non-default OCR engine — say, an offline ML model running in WebGPU, or a cloud API:

```rust
let ocr: OcrPipeline = Box::new(|image: Image| {
    let (tx, rx) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let boxes = my_ocr_backend(image.as_rgba_bytes());
        for tb in boxes { let _ = tx.send(tb); }
    });
    rx
});

Selector::builder().ocr_pipeline(ocr).build()?.run()?;
```

The overlay subscribes to the channel; text boxes appear as they arrive (streamable; doesn't block selection).

## Cancellation

`PostAction::Cancelled` means the user hit `Esc` or closed the overlay. `Outcome.selection` is still populated (the last hovered region) for debugging, but you should not write it anywhere.

## Errors

`SelectorError` covers backend init failures (no Wayland compositor / no display), wgpu adapter unavailable, and config parse errors. The variants are `#[non_exhaustive]` — handle the catch-all in your code.
