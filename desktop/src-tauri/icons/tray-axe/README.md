# Tray axe animation

`axe.svg` is the editable axe source, `background.svg` is the fixed white tile,
and `generate.sh` owns the easing angles. The 14 composed `axe-XX.png` files
and the pre-sized `idle.png` brand image are the runtime assets embedded by
`tray_anim.rs`; `axe-preview.gif` is only a review preview because Tauri and
the macOS status-item API decode animated GIFs as a single image.

Regenerate the checked-in assets from this directory:

```bash
bash generate.sh
```

Keep every runtime frame at 44×44 RGBA. Larger frames force Tauri's macOS
backend to perform unnecessary PNG encoding work on the application main
thread and can make the WebView animation stutter.
