# j3Pic Performance Notes

## Image Open Profiling

`profile_open` is a development-only console binary for the image-open path. It does
not run in the normal viewer startup or paint loop.

Default command:

```powershell
cargo run --bin profile_open
```

The default image path is `C:\Users\dolco\Desktop\111.jpg`. A different image can
be supplied explicitly:

```powershell
cargo run --bin profile_open -- C:\Users\dolco\Desktop\111.jpg
```

To avoid including compile time in the measurement, build once and run the binary
directly:

```powershell
cargo build --bin profile_open
.\target\debug\profile_open.exe C:\Users\dolco\Desktop\111.jpg
```

The output table uses:

- `stage`: measured image-open, decode, conversion, app apply, first-render, and
  Win32 paint-preparation boundary.
- `delta_ms`: elapsed time since the previous stage.
- `total_ms`: cumulative elapsed time for the whole profiled flow.

Static preview decode is bracketed by `static.decode_preview_pixels.begin` and
`static.decode_preview_pixels.complete`. For JPEG preview-first loads, the bracket
contains:

- `static.jpeg_preview.open_file_decoder`: reopens the source file through the
  cancellation-aware reader and initializes the scaled JPEG decoder.
- `static.jpeg_preview.scale_decoder`: asks the JPEG decoder to choose its nearest
  supported IDCT scale for the requested preview size.
- `static.jpeg_preview.decode_scaled_pixels`: decodes the scaled JPEG pixel buffer.
- `static.jpeg_preview.sample_to_target`: samples the decoder-scaled buffer into the
  exact preview target and domain pixel buffer.

## Current Baseline

Measured on 2026-05-09 with `C:\Users\dolco\Desktop\111.jpg`, viewport `960x640`,
release binary, after an initial build:

```text
stage                                          delta_ms     total_ms
open.format_detection                             0.015        0.015
open.initial_cancel_check                         0.001        0.015
open.static_full_resolution_cache_reset           0.008        0.024
static.open_file_metadata_decoder                 1.620        1.644
static.read_source_dimensions                     0.001        1.644
static.source_size_policy_check                   0.005        1.650
static.read_exif_orientation                      0.022        1.672
static.build_image_metadata                       0.000        1.672
static.compute_preview_size                       0.005        1.677
static.decode_preview_pixels.begin                0.001        1.677
static.jpeg_preview.open_file_decoder             0.081        1.758
static.jpeg_preview.scale_decoder                 0.087        1.845
static.jpeg_preview.decode_scaled_pixels         41.320       43.165
static.jpeg_preview.sample_to_target              1.361       44.526
static.decode_preview_pixels.complete             0.264       44.791
static.build_loaded_image                         0.000       44.791
static.verify_file_unchanged                      0.126       44.917
open.complete                                     0.013       44.930
app.scan_image_folder                             0.268       45.198
app.replace_loaded_image                          0.009       45.207
app.prepare_first_render                          0.004       45.211
win32.prepare_paint_dib                           1.462       46.673
```

Render summary:

```text
render: source=140x1280 Rgb8, rect=70x640 at 445,0, quality=Balanced
win32_paint_prepare: dib_bytes=716800, source=140x1280 at 0,0, cached=false
```

Current bottleneck:

- The `static.decode_preview_pixels.begin`/`complete` bracket dominates the measured
  open path. Within it, `static.jpeg_preview.decode_scaled_pixels` is the primary
  cost for this JPEG; final target sampling is much smaller.
- `win32.prepare_paint_dib` is a secondary cost. The first paint prepares a
  `140x1280` RGB source as a top-down BGRA DIB. It is not retained in the paint DIB
  cache in this baseline because the cache budget is capped by the final displayed
  area.
- App state replacement and first-render selection are currently negligible for
  this input. The first-render path uses the decoded preview directly and does not
  build the deferred Balanced/HighQuality scaling cache before visibility.

## Linux Ctrl+Pan Paint Measurement

Measured on 2026-06-16 with a temporary release-mode unit probe using a synthetic
`6000x4000` RGB image and a `960x640` visible pan rectangle.

Before the GTK paint-path fix, each paint created a full Cairo ARGB32 surface from
the render pixels. One full `6000x4000` RGB-to-Cairo conversion took `221.405 ms`,
which is too expensive to repeat during Ctrl+left-button image panning.

After the fix, the GTK backend converts only the visible source rectangle and caches
the converted Cairo surface:

```text
full 6000x4000 conversion after direct RGB path      60.457 ms
visible 960x640 source-rect conversion                3.344 ms
first cached 960x640 source-rect conversion           4.960 ms
same cached 960x640 source-rect reuse                 0.001 ms
```

A follow-up measurement on the same date simulated 240 pan steps over the same
image. The paint-cache budget now allows a balanced expanded source rectangle
instead of caching only the exact visible rectangle:

```text
pan cache sequence: steps=240, surface rebuilds=1, total=28.205 ms
```

The remaining release tests avoid timing thresholds. They verify that panned paints
select the visible source rectangle, that the real paint-cache budget reuses nearby
pan rectangles, that expanded cache rectangles grow on both axes, and that clipped
RGB pixels are converted without a full-image RGBA intermediate.

## Regression Tests

Tests intentionally avoid time thresholds. Coverage is structural:

- `src/bin/profile_open.rs` verifies the table columns and cumulative total
  calculation.
- `src/infra.rs` verifies static full decode profiles include a conversion stage
  after pixel decode, and JPEG preview profiles include the scaled preview
  sub-stages in order.
- `src/platform/win32.rs` verifies Win32 paint-preparation profiling reports DIB
  shape and cache status without timing assertions.
- `src/platform/linux.rs` verifies GTK Cairo paint clipping, surface-cache reuse,
  and direct clipped RGB-to-Cairo conversion for pan paints.
