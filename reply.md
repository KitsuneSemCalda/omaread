egui + typst would work, but it won't save much RAM, because the GPU stack is what uses the memory. Swapping vello's GPU backend for its CPU backend (`anyrender_vello_cpu`) is a three-line change and gives most of the savings.

## Measurements

These are release builds on this machine. **One caveat:** the NVIDIA driver was updated without a reboot ("Driver/library version mismatch"). Vulkan can't create a device, so the current app can only run on wgpu's GL backend, which here goes through Mesa's software renderer. That makes the "current" row worse than your real setup. After a reboot, rerun it before trusting the size of the gap.

| | Library | Prose (Postman) | Géron ch.9 |
|---|---|---|---|
| Current: wgpu + vello, software GL | 575 MB | 437 MB | 620 MB |
| Layout engine alone (`--check`, no GPU) | – | – | 110 MB peak, whole book |
| **vello CPU renderer + softbuffer** | 228 MB | **90 MB** | 189 MB |
| typst alone (one 5,255-word chapter, 8 pages) | – | 26 MB, 33 MB after rasterizing a page | – |
| Empty egui window (glow) | 176 MB | – | – |
| egui showing one typst page | – | 205 MB | – |

## egui + typst

**What typst does well:**
- Very little RAM.
- Excellent typography. The test page came out with Knuth-Plass justification and clean Spanish hyphenation, both built in.
- Literata's bold worked.
- It paginates natively, so `paginate.rs` and the `hyphenation` crate would go away.

**What it costs:**
- **Slow re-typesetting.** About 310 ms per chapter for every compile, and changing the font size means a full recompile. blitz does a chapter load plus pagination in 23–42 ms.
- **Growing memory.** typst's cache took RSS from 26 to 54 MB after three compiles, and `comemo::evict` didn't give it back.
- **egui still needs a GPU backend,** and that window is the 176 MB figure. The RAM you'd save in layout goes straight back into the windowing stack.
- **It's a rewrite of most of the 9.8k lines.** You'd need:
  - an XHTML → typst converter;
  - your own selection and hit-testing to replace parley's `Cursor`/`Selection`;
  - a way to map CFI character offsets back from typst's source spans;
  - highlight and link geometry;
  - the library grid, HUD and contents view rebuilt in egui.
- **It goes against CONTEXT.md.** §2 says "one layout engine, one paint path", and the reason §14 gives for rejecting Iced applies equally to egui.

## The cheaper option

These three lines were tested in a scratch copy; the repo is unchanged:

```toml
anyrender_vello_cpu = { version = "=0.8.1", features = ["softbuffer_window_renderer"] }
```
```rust
use anyrender_vello_cpu::VelloCpuWindowRenderer as VelloWindowRenderer; // main.rs
use anyrender_vello_cpu::VelloCpuImageRenderer as VelloImageRenderer;  // shot.rs
```

It compiled without other changes and renders text correctly. Reading frames take 7–14 ms (up to ~40 ms for Géron's images). It also removes the wgpu teardown crash workaround described in CONTEXT.md and the dependency on the GPU driver, which is the part that's broken right now.

The one regression is library frames, at about 370 ms. In 0.8.1, `anyrender_vello_cpu` reconverts every cover image on every frame: it has a `convert_image_cached` function but doesn't call it. That's also the likely reason the library uses more RAM than reading. The fix is either a newer anyrender (which means a blitz upgrade) or vendoring that one file. Downscaling covers at import, already noted in `net::CoverProvider`, would help as well.

**Next steps:** reboot, measure the current Vulkan build for a fair baseline, then decide on the swap. I can apply it to the repo and fix the image caching if you want.

Two scratch builds are still on disk: `target/cpu-proto/` in the repo (about 1 GB, gitignored, holds the CPU-renderer binary) and `/tmp/claude-1000/-home-daniel-Projects-omaread/a417229d-5998-453e-813c-f84cb096194a/scratchpad/proto` (1.3 GB, the typst + egui prototype). Delete either whenever you like.