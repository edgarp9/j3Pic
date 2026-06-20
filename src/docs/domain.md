# j3Pic Domain

## Core Terms

- Viewer: The native platform application window that owns the current viewing
  session. Windows uses the existing Win32 implementation, and Linux uses the
  GTK4 implementation.
- App icon: The `icon.ico` Windows resource used for the executable shell icon and
  for the viewer window's large and small Win32 icons.
- Viewport: The image content area where pixels are painted. When the status bar is
  visible, this is the client area minus the reserved bottom status bar height.
- Fullscreen state: The Win32 window mode that records whether the viewer is
  borderless on a monitor and, when active, owns the previous window placement and
  style needed to return to windowed mode.
- Image state: The current image slot. It is either empty or one loaded image.
- Loaded image: The decoded image data stored as a domain `PixelImage` with an
  explicit pixel format (`Rgb8`, `Rgba8`, or `Bgra8`), width, height, source format,
  source path, file size metadata, the source file version when it can be identified
  from file length and modified time, and the source file's EXIF orientation when
  one can be read. Static JPEG and other alpha-less 8-bit RGB sources should stay
  `Rgb8` until a boundary specifically requires alpha or a platform DIB layout.
- Image information: The user-visible summary for the current image. It contains
  file name, source pixel resolution, formatted file size, canonical source format
  name, and orientation details when EXIF or manual rotation changes the display
  direction.
- EXIF orientation: The source file metadata value `1` through `8` describing the
  default display transform. It may rotate, flip, or transpose decoded pixels for
  display, but it is never written back to the source file.
- User rotation: The current right-angle rotation requested during this viewing
  session. It is one of `0`, `90`, `180`, and `270` degrees clockwise and is kept
  separate from EXIF orientation.
- Display orientation: The composed transform produced by applying EXIF
  orientation first and user rotation second.
- Display image: The pixel buffer used for painting after applying the current
  display orientation. It preserves the loaded image's pixel format where possible
  and is never written back to the source file.
- Preview image: A downscaled pixel buffer retained for very large images, and for
  JPEG first-view loads when the viewport-sized preview is sufficient and
  meaningfully smaller than the source. JPEG previews remain `Rgb8` when decoded
  through the scaled JPEG preview path.
- First-render path: The first paint after image pixels are replaced prioritizes
  visibility over final scaling quality. It uses the decoded full or preview image
  directly through the platform stretch path and defers Balanced/HighQuality
  software scaling cache creation once, until the render-settle timer runs. After
  that timer resumes scaling-cache rebuilds, the next paint must build or reuse the
  settled cache rather than repeatedly treating every paint as a first render.
- Decode generation: A monotonically increasing session number assigned when a new
  file open or folder navigation request starts. Worker results whose generation no
  longer matches the app state are stale and must be ignored.
- Delayed decode source: A full-resolution or animation-frame decode started from
  an already loaded image. Its result is applied only when generation, path, and
  source file version still match the loaded image.
- Navigation preload: A background decode of the current folder's previous and next
  image targets. Preloaded images are kept as app-owned cache entries and are used
  only if the target is still adjacent to the current image and the source file
  version still matches at navigation time.
- Memory policy: The domain-owned limits for large-image classification, decode
  size, retained full-resolution size, cache entry size, and cache entry count.
- Clipboard image: The current display image copied to the Windows clipboard as
  CF_DIB and CF_DIBV5 DIB payloads. It uses the active display orientation but
  keeps image pixels at their source scale rather than copying the zoomed
  viewport.
- View transform: The current image-to-viewport transform. It owns the view mode,
  manual zoom scale, and manual offset used to compute the destination rectangle.
- Zoom scale: The scale from image pixels to viewport pixels. Manual zoom is clamped
  by the configured zoom bounds and, when zooming out, by the current viewport fit
  floor so a reduced image cannot become smaller than the viewer's fit-to-window
  size. Configured bounds default to 5% through 3200%.
- Zoom status text: The user-visible view scale label. Fit-to-window is displayed
  as `Fit`; actual size and manual zoom are displayed as rounded percentages such
  as `100%` or `320%`.
- Pan gesture: A left-button drag that moves the displayed image while the current
  actual-size or manual-zoom rectangle is larger than the viewport. By default this
  is `Ctrl+left-button drag`. Linux handles this through a GTK drag gesture so the
  app receives cumulative drag offsets rather than only pointer-motion events inside
  the drawing area.
- Window move gesture: A left-button drag that asks Win32 to run the normal top-level
  window move loop. By default this is plain `left-button drag`.
- Interaction settings: The saved mouse shortcut choices for zoom, image folder
  navigation, image panning, and window movement.
- Paint pass: A redraw request handled by the platform layer and reflected in app state.
- Linux paint surface cache: The GTK4 backend converts render pixels into Cairo
  ARGB32 surfaces only for the source rectangle visible in or near the current
  viewport. The converted surface is cached by render key, an expanded source
  rectangle, and scaling quality so repeated pan paints do not reconvert the same
  large image pixels.
- Interactive render settle: Work that was deferred during wheel zoom, keyboard
  view changes, or image panning is resumed only after input has been quiet for the
  configured settle interval. Repeated motion must reschedule the timer so expensive
  scaling cache rebuilds do not run in the middle of a continuous drag.
- Platform backend: The OS-specific UI and system integration layer. Each backend
  must expose the same viewer command model, image-open flow, render/update
  lifecycle, settings flow, export flow, clipboard behavior, drag-and-drop file
  handling, animation timer behavior, and shutdown/config-save behavior through
  the shared `ViewerApp` domain/app contract.

## Current Scope

This stage implements the native Win32 viewer and a Linux GTK4 viewer from the
same codebase. The Windows backend remains the reference behavior. The Linux
backend must provide the same user-visible viewer commands and state transitions
using GTK4 widgets, dialogs, gestures, drawing, clipboard, file dialog, timer, and
drag-and-drop APIs. Platform-specific code owns only native UI/system calls and
message-loop integration; image loading, navigation, render transforms, export
rules, settings parsing, and error classification remain shared Rust app/domain/
infra behavior.

The Windows executable embeds `icon.ico` as resource id `1`. The Win32 platform
layer loads the same resource for the registered window class and sets both large
and small icons on the main viewer window.

Windows builds run as a GUI subsystem executable so launching the viewer from
Explorer does not create a separate console window. Startup failures are still
reported with a native message box, and stderr logging remains best-effort for
terminal-launched runs.

At process startup, the viewer accepts the first non-empty command-line argument as
an initial image path. The path is preserved as an OS string so quoted Windows
paths with spaces or non-Unicode data can be passed through file associations,
shortcuts, terminals, or `Open with`. After the main window is created and sized,
the startup path enters the same image-open flow as `Ctrl+O`; later arguments are
ignored.

Developer profiling for the image-open path is available through the console binary
`profile_open`. It opens one image through the same app and infra load path, records
monotonic stage durations for format detection, file/decoder setup, metadata probing,
pixel decode, pixel-buffer selection or conversion, folder scan, app state
replacement, `app.prepare_first_render`, and Win32 paint DIB preparation, then prints
a compact timing table with `stage`, `delta_ms`, and `total_ms` columns. Static
preview decoding records explicit begin/complete boundaries, and the JPEG scaled
preview path breaks out file/decoder setup, scale selection, scaled pixel decode,
and final target sampling so `static.decode_preview_pixels` bottlenecks can be
attributed without changing the normal viewer path. Measurement
usage and the current `C:\Users\dolco\Desktop\111.jpg` baseline are tracked in
`docs/performance.md`.
`app.prepare_first_render` matches the first-paint policy and does not build
Balanced/HighQuality software scaling caches before the image has been shown. JPEG
RGB paths are expected to avoid the `static.convert_to_rgba8` stage.

The Win32 process enables DPI awareness before creating any native UI. Because
j3Pic is an image viewer where monitor-local pixels matter, it keeps a
per-monitor policy but tries Per-Monitor v1 before Per-Monitor V2 to avoid V2
non-client recalculation during mixed-DPI moves. If those contexts are not
available, it falls back to system-aware and older process-DPI APIs on older
Windows versions. Outside the native
top-level size/move loop, `WM_DPICHANGED` applies the suggested window rectangle,
refreshes the client viewport from Win32, and rebuilds DPI-dependent UI metrics.
While the user is actively dragging or resizing the top-level window, the viewer
does not apply the suggested rectangle because doing so can overwrite the in-flight
system move position on mixed-DPI monitor layouts. DPI-triggered client viewport,
paint, and UI metric updates are deferred until the native size/move loop exits.
Interactive render-settle work is also deferred while the native size/move loop is
active, and invalidation from that path does not synchronously flush `WM_PAINT`.
This keeps cross-monitor dragging from doing cache rebuilds or synchronous repaint
work at the monitor boundary and avoids releasing capture owned by Win32's native
move loop. The status bar and the native settings dialog treat their fixed layout
constants as 96-DPI logical pixels and scale them to the current window DPI.

Supported source formats are `jpg`, `jpeg`, `png`, `bmp`, `gif`, `webp`, `ico`,
`tif`, `tiff`, and `tga`.
Extension checks are case-insensitive. Animated GIF and animated WebP files are loaded
into the same viewer flow as static images: the first frame becomes the current display
buffer, animation metadata is stored in app state, and later frames are decoded on
demand as playback or manual frame commands request them.

Single-file open is triggered with `Ctrl+O` and uses the native Windows file dialog.
The dialog filter exposes only the supported image extensions. If the selected path is
unsupported, inaccessible, not a file, or fails decoding, the app keeps running and
shows a user-facing error message.

The viewer shows a native Win32 context menu from `WM_CONTEXTMENU`, including
open, export, clipboard, view, rotation, fullscreen, and settings commands. Commands
that require a loaded image are unavailable when the image state is empty. The
settings command is always the final menu item and enters the platform boundary
through `open_settings_dialog(hwnd)`, which opens the modal native Win32 settings
dialog titled `j3Pic Settings` or `j3Pic 설정` depending on the selected UI
language.

The export command first opens a modal `Export Options` or `내보내기 옵션`
dialog. It starts with PNG as the selected export format, shows the
display-oriented export source size, lets the user choose another export format
and JPEG quality, lets the user choose whether to remove metadata from the
exported file, lets the user choose an extra right-angle rotation for the
exported file, and lets the user enter a target width or height.
Changing the export rotation updates the source size shown in the dialog and resets
the size fields to that rotated source size. Width and height edits keep the source
aspect ratio while the default-on aspect-ratio checkbox remains checked; clearing it
allows independent width and height edits. Leaving the original dimensions selected
exports without resampling. After the options are accepted, the normal save-file
dialog asks for the destination path using the selected format's extension.
When ICO is selected, the width and height fields are disabled because the export
always writes 16x16, 32x32, 48x48, and 256x256 icon frames.

The settings dialog edits a draft `AppConfig` only. `OK`/`확인` validates the
visible fields, applies the resulting config to the running `ViewerApp`, and
writes it through the normal `config.txt` save path. `Cancel`/`취소` and the
window close button discard the draft without changing the app. `Defaults`/`기본값`
replaces only the dialog draft with values based on `AppConfig::default`; it does
not affect the running viewer until the user later confirms. Numeric parse
failures are reported in a message box and keep the settings dialog open. The
native settings dialog keeps its client area sized to the fixed control layout so
the window does not expose large unused space around the setting groups. The
bottom of the settings dialog includes a project link to `https://github.com/edgarp9`,
opened through the platform's normal external URI handler.

Applying accepted settings updates both future operations and the affected live
viewer state. New image loads use the configured default view mode, decode requests
use the current memory and animation timing policies, render passes use the current
scaling quality and status UI settings, keyboard and wheel zoom use the current zoom
settings, navigation uses the current folder policy, and export suggestions/options
use the current export settings. If an animated image is already loaded, changing
animation autoplay or frame delay settings re-evaluates the loaded playback state so
the next Win32 timer interval reflects the accepted settings without requiring a
reload.

Explorer drag and drop is enabled for the main window. When one or more files are
dropped, the app scans the dropped paths in order and loads the first path with a
supported image extension. If no dropped path is supported, the app keeps the current
image state and reports the supported formats. Linux installs drop targets on the
root GTK widget so drops over either the image area or status area use one path. It
accepts GTK/GDK file-list, single `gio::File`, and string values directly, and reads
URI-list, GNOME/MATE icon-list and copied-files, KDE URI-list, and absolute-path
text payloads through the async MIME stream path so common file managers can use
their native file payload format.

## Error Handling Policy

Recoverable file, decode, folder-scan, export, and configuration failures are carried
as `Result` values to the app or platform boundary. Error values keep their internal
source error for debugging, while user-facing text is produced separately in the
selected UI language.

Image open failures are classified as:

- Unsupported format: the extension or decoded image data is not supported by the
  viewer.
- Corrupt or undecodable file: the decoder cannot interpret the file, including
  truncated image data.
- Permission denied: the file or folder exists but cannot be opened by this user.
- File not found or moved: the path disappeared before or during loading.
- File locked: Windows reports a sharing or lock violation.
- Image too large or out of memory: the file exceeds the memory policy or allocation
  fails.
- Unknown I/O error: a file-system error does not match the known cases above.
- Not a file and canceled decode are separate internal cases.

Each image-open failure also records the internal boundary where it failed:
format detection, file I/O, decoder invocation, pixel conversion, or Win32
rendering. The user-facing message stays short and localized, while debug output
includes the failure stage, category, internal error text, and source chain when
available.

Export save failures are classified as permission denied, missing path, file locked,
encoding failed, invalid image data, image too large or out of memory, or unknown I/O
error. Encoder write failures that wrap an I/O error are classified by the wrapped
I/O cause rather than shown as generic encoding failures.

Normal file-open, drag-and-drop, export, clipboard, and dialog failures are shown with
the existing message-box path. Folder previous/next failures are shown in the bottom
status area instead of a message box so repeated navigation remains lightweight.

The default folder navigation failure policy is `KeepCurrentAndReport`. A failed
previous or next target never replaces the loaded image. By default one command
attempts one target, reports the failure, and keeps the current image visible. When
`auto_skip_failed_navigation` is enabled, the app retries subsequent folder targets
in the same direction up to `max_navigation_attempts_per_command`; if every attempted
target fails or no further target exists, the current image is still kept.

After the current folder snapshot is known and no foreground decode is active, the
platform starts opportunistic background preloads for the previous and next
navigation targets. Foreground opens, navigation decodes, full-resolution decodes,
and animation-frame decodes keep priority and cancel obsolete preload workers. A
preloaded image can satisfy a later previous/next command without starting a new
decode, but stale, resized, retargeted, or changed-file entries are discarded and the
normal navigation decode path is used instead.

## User Configuration

The viewer persists user configuration in a `config.txt` file under the per-user
Windows roaming application data directory, in the app-specific `j3Pic` folder. If
that location is unavailable, the app may fall back to the local application data
directory; if no standard directory is available, configuration loading and saving are
skipped and the app still runs with defaults.

Saved configuration values:

- Last window bounds: normal window `x`, `y`, `width`, and `height`. Fullscreen is
  not restored as fullscreen; closing while fullscreen saves the previous windowed
  bounds when available.
- UI language: `english` or `korean`. `english` is the default. The settings dialog
  exposes this as the first General setting, and newly opened menus/dialogs use the
  selected language after settings are applied.
- Default view mode: `FitToWindow` or `ActualSize`. `FitToWindow` is the default.
  Manual zoom is session state and is corrected to `FitToWindow` if found in config.
- Scaling quality: `Nearest`, `Balanced`, or `HighQuality`. `Balanced` is the
  default. A 100% render still uses nearest-neighbor because no scaling is needed.
- Recent folder: the parent folder of the most recently loaded image, used as the
  next open-file dialog starting folder when present.
- Export default quality: JPEG quality `1` through `100`, default `90`. Numeric
  values outside this range are clamped.
- Animation autoplay: `true` by default. When `false`, newly loaded animated images
  start paused.
- User-adjustable domain settings are persisted in the same `version=1` file as
  optional keys. Missing keys keep the previous behavior exactly: zoom uses
  `0.05` through `32.0` with step factor `1.25`; large-image and cache decisions
  use `ImageMemoryPolicy::DEFAULT`; animation frame delays use `100 ms`, clamped
  between `10 ms` and `60,000 ms`; folder navigation wraps and does not auto-skip
  failed loads; export suggestions keep the source format when supported, append
  `-export`, and flatten JPEG alpha over white; the UI language is English; the
  status bar is shown with detailed text.
- The settings dialog exposes the main user-adjustable values directly. Numeric
  edits are validated on `OK`/`확인` instead of silently corrected: UI language
  defaults to English and allows English or Korean; default view mode defaults to
  `FitToWindow` and allows `FitToWindow` or `ActualSize`; scaling quality defaults
  to `Balanced` and allows `Nearest`, `Balanced`, or `HighQuality`; status bar and
  detailed status text both default to `true`.
  Zoom defaults are `min_zoom_scale=0.05`, `max_zoom_scale=32.0`, and
  `zoom_step_factor=1.25`; the editable ranges are `0.01..=1.0`,
  `1.0..=128.0`, and `1.01..=8.0`, with the minimum zoom not greater than the
  maximum zoom. Large-image and memory dialog fields default to
  `large_image_pixel_threshold=24,000,000`, `max_image_pixels=160,000,000`,
  `preview_max_pixels=8,000,000`, `preview_oversample=2`,
  `full_resolution_request_scale=0.75`, `max_resident_mib=256`,
  `max_cache_entry_mib=128`, and `max_cache_entries=2`. Pixel limits are
  `1..=1,000,000,000`, preview oversample is `1..=8`, full-resolution request
  scale is `0.05..=32.0`, memory MiB values are `1..=4096`, cache entries are
  `0..=64`, the large-image and preview pixel limits must not exceed
  `max_image_pixels`, and the per-entry cache MiB limit must not exceed the total
  cache MiB limit. Animation autoplay defaults to `true`; frame delay defaults
  are `default=100 ms`, `min=10 ms`, and `max=60,000 ms`, each editable within
  `1..=600,000 ms`, with `min <= default <= max`. Navigation defaults are
  `wrap_navigation=true`, `auto_skip_failed_navigation=false`, and
  `max_navigation_attempts_per_command=1`; attempts are editable within
  `1..=100`. Export defaults are policy `png`, JPEG quality `90`, suffix
  `-export`, and JPEG alpha background RGB `255,255,255`; policy allows
  `source`, `png`, `jpeg`, `bmp`, `webp`, or `ico`, JPEG quality is `1..=100`, suffixes
  must be non-empty, at most 64 characters, and avoid Windows-invalid filename
  characters, and RGB components are `0..=255`.
- Zoom settings: `min_zoom_scale`, `max_zoom_scale`, and `zoom_step_factor`.
  Non-finite or out-of-range values are replaced by defaults or clamped to a
  usable range before they are exposed to the app.
- Memory policy settings: `large_image_pixel_threshold`, `max_image_pixels`,
  `preview_max_pixels`, `preview_oversample`, `fallback_viewport_width`,
  `fallback_viewport_height`, `max_transient_decode_mib`,
  `max_full_resolution_mib`, `max_resident_mib`, `max_cache_entry_mib`,
  `max_cache_entries`, `max_animation_metadata_frames`, and
  `full_resolution_request_scale`. `AppConfig` owns these user-facing values and
  produces the domain `ImageMemoryPolicy` used by decode and cache decisions.
- Animation timing settings: `default_frame_delay_ms`, `min_frame_delay_ms`, and
  `max_frame_delay_ms`. The default delay is normalized into the configured
  min/max range.
- Navigation settings: `wrap_navigation`, `auto_skip_failed_navigation`, and
  `max_navigation_attempts_per_command`. Defaults preserve the existing behavior:
  wrapping is enabled, failed files are reported without auto-skip, and one target
  is attempted per command.
- Export settings: `default_export_format_policy` (`source`, `png`, `jpeg`,
  `bmp`, `webp`, or `ico`), `export_filename_suffix`, and
  `jpeg_alpha_background_rgb` as `r,g,b`. Unsafe or empty filename suffixes fall
  back to `-export`.
- Status UI settings: `show_status_bar` and `detailed_status_text`.
- Interaction settings: `zoom_shortcut`, `image_navigation_shortcut`,
  `image_pan_shortcut`, and `window_move_shortcut`. The default mouse shortcuts are
  `ctrl_mouse_wheel` for zoom, `mouse_wheel` for previous/next image navigation,
  `ctrl_left_button_drag` for image panning, and `left_button_drag` for moving the
  viewer window.

Final settings reference. UI paths start from the main viewer context menu
`Settings`/`설정`, which opens the `j3Pic Settings`/`j3Pic 설정` dialog. Values
marked `config.txt only` are persisted for compatibility and advanced policy
tuning but are not exposed as editable Win32 dialog controls.

| Area | Config key(s) | Default | Allowed values or correction | UI path |
| --- | --- | --- | --- | --- |
| Window bounds | `window.x`, `window.y`, `window.width`, `window.height` | none | `x/y`: `-32768..=32767`; `width`: `320..=32767`; `height`: `240..=32767`; invalid partial bounds are ignored | Auto-saved on exit, no settings control |
| UI language | `ui_language` | `english` | `english`/`en` or `korean`/`ko`/`kr`; unknown values fall back to English | `Settings > General > Language` / `설정 > 일반 > 언어` |
| Default view mode | `default_view_mode` | `fit_to_window` | `fit_to_window` or `actual_size`; `manual_zoom` is corrected to `fit_to_window` | `설정 > 일반 > 기본 보기 모드` |
| Scaling quality | `scaling_quality` | `balanced` | `nearest`, `balanced`, `high_quality` | `설정 > 일반 > 스케일링 품질` |
| Recent folder | `recent_folder` | none | Escaped path string; empty path is ignored | Updated after successful image load, no settings control |
| JPEG export quality | `export_default_quality` | `90` | `1..=100`, clamped on config load | `설정 > 내보내기 > JPEG 품질` |
| Animation autoplay | `animation_autoplay` | `true` | `true/false`, plus `1/0` and `yes/no` aliases in config | `설정 > 애니메이션 > 자동재생` |
| Zoom bounds and step | `min_zoom_scale`, `max_zoom_scale`, `zoom_step_factor` | `0.05`, `32.0`, `1.25` | `min`: `0.01..=1.0`; `max`: `1.0..=128.0`; `step`: `1.01..=8.0`; dialog rejects `min > max` | `설정 > 줌` |
| Large image pixel policy | `large_image_pixel_threshold`, `max_image_pixels`, `preview_max_pixels`, `preview_oversample`, `full_resolution_request_scale` | `24,000,000`, `160,000,000`, `8,000,000`, `2`, `0.75` | Pixel limits: `1..=1,000,000,000`; preview oversample: `1..=8`; request scale: `0.05..=32.0`; large and preview limits are capped by max image pixels | `설정 > 대용량 이미지/메모리` |
| Memory cache policy | `max_resident_mib`, `max_cache_entry_mib`, `max_cache_entries` | `256`, `128`, `2` | MiB values: `1..=4096`; cache entries: `0..=64`; per-entry MiB is capped by total resident MiB | `설정 > 대용량 이미지/메모리` |
| Decode and hidden memory policy | `fallback_viewport_width`, `fallback_viewport_height`, `max_transient_decode_mib`, `max_full_resolution_mib`, `max_animation_metadata_frames` | `1920`, `1080`, `640`, `128`, `10,000` | Viewport edges: `1..=16384`; MiB values: `1..=4096`; metadata frames: `1..=1,000,000`; full-resolution MiB is capped by transient decode MiB | `config.txt` only |
| Animation timing | `default_frame_delay_ms`, `min_frame_delay_ms`, `max_frame_delay_ms` | `100`, `10`, `60,000` | Each value `1..=600,000`; dialog requires `min <= default <= max`; config load normalizes into that relationship | `설정 > 애니메이션` |
| Folder navigation | `wrap_navigation`, `auto_skip_failed_navigation`, `max_navigation_attempts_per_command` | `true`, `false`, `1` | Attempts `1..=100`; auto-skip uses the attempt limit, otherwise failed navigation reports and keeps the current image | `설정 > 탐색` |
| Export format policy | `default_export_format_policy` | `png` | `source`, `png`, `jpeg`, `bmp`, `webp`, `ico`; source GIF, TIFF, and TGA fall back to PNG for export | `설정 > 내보내기 > 기본 포맷 정책` |
| Export filename suffix | `export_filename_suffix` | `-export` | Non-empty, at most 64 chars, no `\ / : * ? " < > \|` or control characters; invalid config falls back to default | `설정 > 내보내기 > 파일명 suffix` |
| JPEG alpha background | `jpeg_alpha_background_rgb` | `255,255,255` | `r,g,b`, each component clamped to `0..=255` in config; dialog requires a valid three-component value | `설정 > 내보내기 > JPEG 투명 배경 RGB` |
| Status UI | `show_status_bar`, `detailed_status_text` | `true`, `true` | `true/false`, plus `1/0` and `yes/no` aliases in config | `설정 > 일반 > 상태바 표시`, `설정 > 일반 > 자세한 상태 텍스트` |
| Mouse shortcuts | `zoom_shortcut`, `image_navigation_shortcut`, `image_pan_shortcut`, `window_move_shortcut` | `ctrl_mouse_wheel`, `mouse_wheel`, `ctrl_left_button_drag`, `left_button_drag` | Wheel commands: `mouse_wheel` or `ctrl_mouse_wheel`; drag commands: `left_button_drag` or `ctrl_left_button_drag`; unknown values fall back to defaults | `설정 > 단축키` |

Configuration loading is best-effort. A missing file uses defaults. A malformed or
unsupported-version file is treated as damaged and replaced in memory by defaults.
Individual invalid values in an otherwise parseable file are corrected to defaults or
clamped ranges. Read permission errors do not fail app startup; the app continues with
defaults.

Configuration saving is also best-effort. On shutdown, the app writes a temporary
config file in the same directory and then replaces the real config file. Directory
creation, write, and replace errors are ignored at the app boundary so the viewer can
exit cleanly without turning configuration persistence failures into user-facing
runtime failures.

## Image Export

Image export writes the current display image to a new file through the native Win32
save-file dialog. It is triggered with `Ctrl+S` or `Ctrl+Shift+S`.

Supported export formats are `png`, `jpg/jpeg`, `bmp`, `webp`, and `ico`. Export GIF is not
part of this scope because animated export is intentionally not supported. WebP export
uses the current `image` crate encoder, which is lossless-only in this dependency set,
so quality selection applies to JPEG export only. ICO export writes four transparent
PNG-backed icon frames at 16x16, 32x32, 48x48, and 256x256.

The selected save-dialog filter is the source of truth for the output format. If the
typed file extension does not match the selected format, the path is corrected to that
format's default extension (`jpg`, `png`, `bmp`, `webp`, or `ico`). JPEG keeps either
`jpg` or `jpeg` when one is already present. Suggested export names use the source stem plus
the configured suffix, defaulting to `-export`, to reduce accidental overwrite risk.
Export to the exact original source file path is blocked, and when extension
correction targets an existing file the user must confirm overwrite.

Export starts from the current display-orientation policy: EXIF orientation is applied
first, then the current user rotation is applied. The export options may apply one
additional right-angle rotation (`0`, `90`, `180`, or `270` degrees clockwise) on top
of that display-oriented image before optional resizing and encoding. Zoom, pan, and
viewport scaling are not written. Animated GIF and animated WebP export only the
currently displayed frame. If a very large image is currently preview-backed and the
full-resolution buffer has not been retained, export writes the current preview buffer
rather than forcing a synchronous full-resolution decode.

PNG, BMP, and lossless WebP preserve RGBA pixels. PNG export is losslessly
optimized with oxipng after encoding the temporary file and before replacing the
target file, but color type and bit depth reductions are disabled so the exported
pixel channels remain compatible with the app's PNG export contract. JPEG has no
alpha channel, so transparent pixels are flattened over the configured RGB
background, defaulting to white, before encoding. File creation, write,
permission, disk-space, invalid-buffer, optimizer, and encoder errors are
reported to the user without terminating the app. A successful export updates the
bottom status text with a short saved message when the status bar is enabled.

## Image Information

When an image is loaded, the viewer shows a bottom status bar with:

- File name.
- Source image resolution as `widthxheight`.
- Display resolution after EXIF orientation and user rotation when orientation is
  active.
- EXIF orientation and user rotation labels when either transform is active.
- File size formatted into `B`, `KB`, `MB`, `GB`, or `TB`.
- Canonical format name: `JPEG`, `PNG`, `BMP`, `GIF`, `WebP`, `ICO`, `TIFF`, or `TGA`.
- Current zoom status text: `Fit`, `100%`, or a rounded manual zoom percentage.

The image information text is derived by pure domain functions from loaded image
metadata, source image size, user rotation, the current view transform, and status UI
settings. Detailed text is the default. If no EXIF or user rotation is active, the
compact legacy form is used, for example `photo.jpg | 1920x1080 | 1.5 MB | JPEG`.
If orientation is active, the source and display sizes are shown separately, for
example `source 1920x1080 | display 1080x1920 | EXIF 6`. Simple status text shows
only the file name and zoom label. Long text is rendered by the platform layer with
end ellipsis so it fits the current window width. When no image is loaded, or when
`show_status_bar` is disabled, no status text is shown and the empty state does not
block normal window behavior.
When the status bar is visible, the platform reserves its height outside the image
viewport so fit-to-window rendering is not covered by the status text.

The window title reflects the current image file name. EXIF orientation and user
rotation may be appended separately, but the file name remains the primary title
signal.

## Fullscreen

The viewer supports toggling the top-level Win32 window between windowed mode and
fullscreen mode. Fullscreen is platform window state, separate from image state and
view-transform state.

The Win32 window owns exactly one platform state pointer through `GWLP_USERDATA`.
Attaching, clearing, and style changes are checked with the Win32 last-error
contract because a zero return can mean either success with a previous zero value or
failure. If state attachment fails during creation, creation fails instead of
leaving an unreachable Rust state allocation behind.

Before entering fullscreen, the viewer saves the current Win32 window style and
window placement. It removes the overlapped-window frame style, chooses the monitor
nearest to the current window, and resizes the window to that monitor's full bounds.
On exit, the saved style and placement are restored.

Entering or leaving fullscreen, receiving `WM_DPICHANGED`, opening another image,
navigating, canceling window modes, or destroying the window clears any active pan
gesture and releases mouse capture when the viewer owns one. Fullscreen and DPI
transitions refresh the viewport size from the current client area and constrain the
current view transform to the new viewport. Fit-to-window views refit on the next
paint pass. Manual zoom and actual-size views keep their mode while their offsets
are clamped to valid display bounds.

Fullscreen mode does not change file or image commands. Opening files, drag and
drop, folder previous/next navigation, zoom, actual-size, fit-to-window, and rotation
continue to use the same commands as windowed mode.

Fullscreen keyboard shortcuts:

- `F11`: toggle fullscreen.
- `Alt+Enter`: toggle fullscreen.
- `Esc`: leave fullscreen when fullscreen is active; otherwise exit the viewer.

## Folder Navigation

Whenever an image is opened successfully, the viewer scans the image file's parent
folder and builds a folder image list from regular files with supported extensions:
`jpg`, `jpeg`, `png`, `bmp`, `gif`, `webp`, `ico`, `tif`, `tiff`, and `tga`.
Extension checks are
case-insensitive. The scanner may reuse the most recent same-folder snapshot when
the parent folder's modified timestamp is unchanged and the newly opened image can
be located in that snapshot; if the timestamp is unavailable, changed, or the image
is missing from the snapshot, it performs a fresh scan.
If the user requests previous or next navigation after the image is visible but
before this deferred folder scan has completed, the app records the latest requested
direction and starts that navigation as soon as the matching folder scan result is
applied. Starting another image decode clears that pending navigation.

The folder image list is sorted by file name using a case-insensitive comparison.
Sorting is stable, so entries with equal case-insensitive names keep their scanned
relative order. No natural-sort or locale-specific collation is applied.

The app records the current file's index in that sorted list. Next navigation selects
the following item, and previous navigation selects the preceding item. Navigation is
circular by default: next from the last image selects the first image, and previous
from the first image selects the last image. When `wrap_navigation` is disabled,
navigation at either end is a safe no-op. If the folder contains only the current
image, or the current image cannot be located in the list, previous and next are safe
no-ops.
Current-file lookup first compares the full path. If the scan returns a path with
different casing or a different prefix for the same file name, lookup falls back to a
case-insensitive file-name comparison so Windows path casing does not break previous
and next navigation.

Failed decoding, missing files, locked files, permission failures, and folder-scan
failures during navigation do not replace the current image. The app keeps the
existing image visible and reports the failure in the status area. With the default
policy failed targets are not automatically skipped; with auto-skip enabled, later
targets in the same direction are attempted until one loads or the configured attempt
limit is reached.

## Animation Playback

Animated GIF and animated WebP use the image crate's animation decoders. During initial
load the decoder walks the frame stream to read the frame count, per-frame delay, and
loop policy. The app retains the first frame as the current image buffer and stores the
animation playback state with the loaded image: normalized frame delays, current frame
index, playback state (`Playing`, `Paused`, or `Finished`), loop policy, and completed
loop count. The default state for a multi-frame image is autoplay unless the saved
animation autoplay setting is disabled.

Frame delay values are normalized by domain rules before Win32 timers see them. A delay
of `0` uses the configured default delay, defaulting to `100 ms`; non-zero delays are
clamped to the configured min/max range, defaulting to `10 ms` through `60,000 ms`, to
avoid a busy timer or impractically long timer interval.

Playback timing is owned by the Win32 platform layer and uses `SetTimer`, `KillTimer`,
and `WM_TIMER` with one window-owned animation timer id. The app domain decides the next
frame, loop transition, and playback state with pure functions. The platform kills the
timer before opening a new file, navigating to another file, waiting for an uncached
frame decode, and destroying the window. Destroy also cancels and joins background
decode workers through the decode controller shutdown path before releasing the
window state.

Frame pixels are not retained without bound. The current displayed frame is the loaded
image buffer. Recently replaced frames may be kept in the shared image cache as
`AnimationFrame` slots, subject to the configured resident memory budget, per-entry
limit, and cache-entry count policy. These default to `256 MiB`, `128 MiB`, and two
cache entries. Animation metadata is capped by the configured frame-count policy,
defaulting to `10,000` frame delay entries. When playback or manual stepping needs a
frame that is not cached, the platform starts a background frame decode for that frame
and resumes the timer only after the frame has been applied.

If playback is toggled or a manual frame command is received while a background frame
decode is pending, the pending transition is discarded before the new command is
evaluated. A late worker result for the discarded frame is stale and cannot overwrite
the newer playback state.

Manual frame commands pause playback. `[` steps to the previous frame and clamps at the
first frame. `]` steps to the next frame and clamps at the last frame. `Home` moves to
the first frame. When finite playback reaches its final loop, the animation stays on
the last frame and enters `Finished`; toggling playback from `Finished` restarts at
the first frame.

Shortcut conflict policy:

- Static image: `Space` keeps the legacy next-image command.
- Animated image: `Space` toggles animation playback.
- `P` always maps to animation playback toggle; it is a safe no-op for static images.
- `Right` and `PageDown` remain next-image commands for both static and animated
  images.

Animation frames use the same display path as static images. EXIF orientation, user
rotation, zoom, fit-to-window, actual size, panning, software scaling cache, and
clipboard display-image policy all operate on the current frame buffer through the
existing display-orientation and render-cache logic.

## Pixel Buffer Policy

The domain layer owns image buffer representation through `PixelImage`, not through
image-crate types. `Rgb8` is used for alpha-less 8-bit RGB data, especially JPEG
full-resolution and scaled-preview loads. `Rgba8` is used for animation frames,
alpha-bearing decoded data, and fallback paths where a decoder or resampler already
produces RGBA pixels. `Bgra8` is available as an explicit domain representation for
platform-friendly buffers, but platform APIs still perform their final DIB conversion
at the platform boundary.

The infra layer is the image-crate boundary. It may decode with `image`,
`jpeg-decoder`, or `png`, but it must convert decoder-owned types into domain
`PixelImage` before returning to app code. Image-crate buffer and color types must
not appear in app, domain, or platform-facing API signatures.

App use cases preserve the current pixel format for display orientation and software
scaling when possible. Rotation and EXIF orientation copy pixels using the active
format's bytes-per-pixel stride, so RGB JPEG buffers are not expanded only to rotate
or flip. Software scaling keeps `Rgb8` for RGB sources, keeps `Rgba8` for RGBA
sources, and converts `Bgra8` to `Rgba8` only when the scaler needs an RGBA view.

Conversion boundaries are explicit:

- Rendering converts the display `PixelImage` to a top-down 32-bit BGRA DIB only in
  the Win32 paint boundary. `Rgb8` is swizzled directly to BGRA without alpha
  expansion inside app state; alpha formats are flattened over white for GDI.
- Clipboard copy converts the display `PixelImage` to an opaque 32-bit BGRA DIB at
  the Win32 clipboard boundary. `Rgb8` writes opaque BGRA directly; alpha formats
  are flattened over white.
- Export accepts a domain `PixelImage`. JPEG export writes `Rgb8` directly, while
  alpha formats are flattened to RGB over the configured background. PNG, BMP, WebP,
  and ICO can write compatible RGB/RGBA buffers without exposing image-crate types.
  ICO preserves alpha by converting each frame to RGBA and fitting non-square images
  onto transparent square canvases.
  The app applies any export-only rotation to the display-oriented export pixels
  before resampling. When an export target size is selected, the app resamples those
  pixels before the infra encoder boundary and preserves RGB/RGBA where possible.
  Source metadata is not propagated through the pixel export boundary.
  When the export metadata-removal option is enabled, the infra boundary also strips
  encoder-level metadata containers where applicable, including JPEG APP/COM
  segments, PNG ancillary metadata chunks, and WebP EXIF/XMP/ICCP chunks.
- Animation decode and animation-frame cache remain `Rgba8` because animated GIF and
  WebP frame composition naturally carries alpha-capable frames.

## Large Images and Lazy Loading

The viewer separates the current image state from decode work state. The app owns the
current image, view transform, rotation, cache state, and active decode generation.
The Win32 platform layer owns the decode worker controller, cancellation flags, worker
handles, and the channel used to deliver worker results back to the UI thread.

When opening an image, the worker first validates the extension and file metadata,
then asks the image decoder for dimensions before full pixel decoding. EXIF
orientation is read as metadata during load when the existing decoder can provide
it. If EXIF orientation is absent, malformed, unsupported, or cannot be parsed, the
viewer treats the image as EXIF orientation `1` and continues loading. An image is
classified as large when it exceeds the configured large-image pixel threshold or
when a conservative full-resolution RGBA8-sized buffer would exceed the retained
full-resolution budget. These
defaults are `24,000,000` pixels and `128 MiB`. Images above the configured maximum
pixel count, defaulting to `160,000,000`, or above the transient decode budget,
defaulting to `640 MiB`, are rejected with an image-too-large error instead of
terminating the app.

Large images are decoded on a background thread and reduced to a viewport-oriented
preview buffer before being applied to app state. Decode I/O checks the worker
cancellation flag so superseded work can abort while the decoder is still reading
the source file. Static image decode consumes the decoder-owned image when building
the domain `PixelImage` instead of cloning it, keeping transient memory closer to one decoded
full-size buffer plus the preview target. The preview target is the current viewport
multiplied by the configured oversample factor, defaulting to `2`; when the viewport
is not known, the configured fallback viewport is used, defaulting to `1920x1080`.
Preview buffers are capped by the configured preview pixel budget, defaulting to
`8,000,000` pixels. The source resolution remains part of the loaded image metadata
and is used for status text.
Static JPEG loading also uses this preview target for the first display when the
current viewport is known and the preview has at most half the source pixel count,
even if the image is below the large-image threshold. That path avoids both an
initial full-resolution decode and an RGB-to-RGBA expansion, returning an `Rgb8`
preview through the safe `jpeg-decoder` scaled decode API with default features
disabled so the dependency is limited to JPEG preview IDCT scaling. Full-resolution
JPEG decode remains lazy and uses the existing on-demand replacement flow when zoom
reaches the configured request scale; the replacement buffer is also `Rgb8`.
The EXIF-adjusted display resolution is used for fit-to-window geometry,
actual-size geometry, zoom, and panning. Rendering may stretch the preview buffer
into a rectangle computed from the display-oriented source resolution.

Initial image decode does not attach a viewport-resampled render-ready buffer.
Applying a decoded image invalidates stale render caches and lets the first paint
draw the decoded full or preview buffer directly. The deferred render-settle path
then builds any Balanced/HighQuality scaling cache that still matches the current
generation, viewport, orientation, view mode, and scaling quality. Subsequent zoom,
actual-size, rotation, resize, and full-resolution paths continue to use the normal
display-orientation cache, scaling cache, and lazy full-resolution decode rules.

The original full-resolution pixel buffer is not retained for a large image during
initial display. A full-resolution decode can be requested later when the current
image is still preview-backed, the effective view scale reaches the configured
request threshold, defaulting to `75%`, and the full pixel buffer fits the configured
retained-full-resolution budget, defaulting to `128 MiB`. If the full buffer is too
large, the preview remains the display source and the user can still
zoom, fit-to-window, pan, and navigate.

The decode worker uses a cancellation flag and posts a private `WM_APP` message when
it sends a result through the channel. Starting a new file open or folder navigation
increments the decode generation and cancels the previous worker. Dimension probing,
EXIF probing, and pixel decoding all read through the same cancellation-aware file
boundary, and long preview downscale steps check the flag before and after resampling.
The app applies a worker result only when its generation, decode purpose, requested
source path, and active source path still match the current state. Stale preview,
full-resolution, animation-frame, failure, and cancellation results are ignored and
cannot replace the current image or clear a newer pending decode.

Memory failures, image-too-large failures, and canceled decodes are distinct error
cases. Memory and size failures are reported to the user for the active generation.
Canceled work is normally superseded by a newer generation and is ignored.

## EXIF Orientation and Rotation

The viewer supports EXIF orientation and right-angle user rotation without modifying
the original image file. EXIF orientation is metadata read from the source image;
user rotation is session state for the current loaded image only. Opening a new
file, dropping a file, or moving to another folder image resets user rotation to
`0`, then reads EXIF orientation for the newly loaded image.

EXIF orientation values are interpreted as:

| EXIF | Display transform |
| --- | --- |
| `1` | No transform |
| `2` | Flip horizontally |
| `3` | Rotate 180 degrees |
| `4` | Flip vertically |
| `5` | Rotate 90 degrees clockwise, then flip horizontally |
| `6` | Rotate 90 degrees clockwise |
| `7` | Rotate 270 degrees clockwise, then flip horizontally |
| `8` | Rotate 270 degrees clockwise |

Values outside `1` through `8`, missing EXIF metadata, and EXIF parsing failures
all fall back to `1`. This fallback is not an image loading failure.

`R` rotates the displayed image 90 degrees clockwise. `Shift+R` rotates it 90
degrees counterclockwise. The state always normalizes to one of `0`, `90`, `180`,
or `270`.

The display orientation is calculated as `EXIF orientation + user rotation`: EXIF
orientation is applied first, then the user's session rotation is applied on top.
The display size used by fit-to-window, actual-size, manual zoom, and panning is
the composed display size. EXIF values `5`, `6`, `7`, and `8`, or a user rotation of
`90` or `270` over a non-transposed EXIF orientation, swap width and height.
Horizontal and vertical flips keep width and height but still affect pixel order.

Clipboard copy follows the display-image policy: if a rotation is active, `Ctrl+C`
copies the oriented image in the same direction the viewer displays, including EXIF
flips. The copied image is not scaled by zoom, panning, or viewport size; it is the
full display-oriented image at its pixel dimensions. The clipboard payloads are
top-down 32-bit BGRA DIBs flattened over white for alpha, with CF_DIB using a
BITMAPINFOHEADER and CF_DIBV5 using a BITMAPV5HEADER with explicit color masks.

When user rotation changes while the view mode is `FitToWindow`, the image refits
to the new composed display size. In manual zoom or actual-size states, the current
scale or mode is kept and the offset is clamped so the display-oriented image
remains in the valid display range.

## View Transform and Zoom

The viewer supports three view modes:

- `FitToWindow`: scale the image to fit inside the viewport while preserving aspect
  ratio. The fitted rectangle is recomputed from the current viewport on every paint,
  so resizing the window automatically refits the image.
- `ActualSize`: display image pixels at 100% scale and center the image in the
  viewport. If the actual-size rectangle is larger than the viewport, a pan gesture
  can move it and the transform becomes `ManualZoom` at 100% scale.
- `ManualZoom`: display with an explicit zoom scale clamped to the configured
  min/max zoom range, defaulting to 5% through 3200%, and to the current
  fit-to-window scale when the image is being reduced. For images already smaller
  than the viewport, manual zoom-out stops at 100% instead of shrinking below the
  source size. Manual offset is constrained so zooming or resizing does not move
  the image completely away from the viewport.

By default `Ctrl+mouse wheel` zooms in or out. Keyboard and wheel zoom use the
configured zoom step factor, defaulting to `1.25`. Wheel zoom uses the cursor position
as the anchor, preserving the image point under the cursor when the resulting offset
is within the constrained range. Plain mouse wheel navigates the folder image list by
default: wheel up selects the previous image and wheel down selects the next image.

`Ctrl+left-button` dragging pans by default, and only when the displayed image is
larger than the viewport on at least one axis. Dragging updates the horizontal manual
offset by the mouse delta from the drag start point and the vertical manual offset by
the same mouse delta, so the displayed image follows the drag direction. Each offset
update is clamped by the domain transform rules before paint.
If the displayed image fits inside the viewport on an axis, that axis keeps offset
`0` and remains centered; dragging cannot move it off center. If both axes fit inside
the viewport, panning is disabled.

Offset clamping is calculated from the viewport extent, scaled image extent, and
requested offset. For an oversized axis, the clamped destination origin stays between
the viewport's leading edge and the farthest origin that still covers the trailing
edge. For a fitting axis, the offset is always `0`, so the centered origin is preserved.
Zooming, switching to actual size, switching to fit-to-window, loading a new image, and
resizing the window all leave the active transform in a valid display range.

Keyboard shortcuts are first translated into a central `Command` value, then handled
by the app use case or the platform boundary for native operations such as file
dialogs and fullscreen.

| Shortcut | Command |
| --- | --- |
| `Ctrl+O` | Open image file |
| `Ctrl+S`, `Ctrl+Shift+S` | Export current image |
| `Ctrl+C` | Copy current display image to the Windows clipboard |
| `Right`, `PageDown` | Next image |
| `Space` on a static image | Next image |
| `Space` on an animated image, `P` | Toggle animation playback |
| `[`, `]` | Previous or next animation frame |
| `Home` | First animation frame |
| `Left`, `Backspace`, `PageUp` | Previous image |
| `+`, `=`, numpad `+` | Zoom in around the viewport center |
| `-`, numpad `-` | Zoom out around the viewport center |
| `1` | Actual size |
| `0` | Fit to window |
| `R` | Rotate 90 degrees clockwise |
| `Shift+R` | Rotate 90 degrees counterclockwise |
| `F11`, `Alt+Enter` | Toggle fullscreen |
| `Esc` | Leave fullscreen if active, otherwise exit |
| `Q`, `Alt+F4` | Exit |

The transform calculation takes an image display size as input rather than reading
raw image dimensions directly. EXIF orientation and user rotation update that
display size before the same rectangle calculation runs.

Rendering uses Win32 GDI. The display `PixelImage` is converted at the platform
boundary into a 32-bit DIB-compatible BGRA buffer before `StretchDIBits` paints the
image into the destination rectangle computed by the current view transform. Because
GDI painting is not responsible for rotating or flipping the source image, the app
derives a display-oriented pixel buffer and caches it by current image and display
orientation to avoid repeated orientation work for large images.

When a decoded image, full-resolution replacement, or animation frame is applied, the
platform invalidates stale paint caches and flushes the invalidated window paint
only when the native top-level size/move loop is not active. It does not
synchronously build the software scaling cache on the UI thread before first
paint. The first paint may use the direct GDI path so the new image becomes
visible promptly even when keyboard, wheel, timer, or worker messages continue
arriving; the deferred render-settle timer rebuilds heavier scaling caches
afterward.

Paint invalidation does not request a separate Win32 background erase. The platform
paint pass redraws the complete client area into a compatible memory DC, including
the window background, current image, and status bar, then copies the completed
frame to the window in one `BitBlt`. When the status bar is visible, image paint and
paint-cache clipping use the reserved image content rectangle rather than the full
client rectangle. This keeps zoom, pan, and image replacement from exposing the
intermediate cleared background.

The Win32 paint boundary also keeps one reusable BGRA DIB cache for the current
render source when it fits the configured per-entry cache budget. The cache key
includes the render image identity, display orientation through that render key,
the cached source rect, and the active scaling quality. Paint and prepare paths
cap the DIB cache entry by both the memory policy and the final visible display
area, allowing small quantized scaled buffers while avoiding retained BGRA copies
of a large original source when only a smaller displayed result is useful. Reusing
this conversion avoids reallocating and reblending the same visible or final render
pixels on repeated repaints.
Mouse-wheel zoom and image-pan updates invalidate and synchronously flush the window
paint when outside native size/move so continuous input does not delay repaint until
the message queue becomes idle. During native size/move, the same invalidation path
avoids `UpdateWindow` and lets Win32 deliver paint when the drag/resize loop can
accept it.

## Scaling Quality

The saved scaling quality preference defaults to `Balanced`. The viewer keeps the
native Win32/GDI paint path and chooses an internal `ScalingQuality` for each render:

- `Nearest`: used when the saved preference is nearest-neighbor or the effective
  scale is exactly 100%. It avoids unnecessary software resampling for actual-size
  rendering.
- `Balanced`: the default smooth mode for scaled rendering. Downscaling uses a
  cached software-resampled pixel buffer that preserves RGB/RGBA where possible,
  while upscaling and near-actual-size changes use the direct GDI path with a
  smooth stretch mode.
- `HighQuality`: uses higher-cost resampling for any non-100% scaled render.

The platform layer maps smooth qualities to GDI `HALFTONE` stretch mode and maps
`Nearest` to the nearest-neighbor GDI stretch mode. If software resampling cannot
build a target buffer, rendering falls back to the direct GDI `StretchDIBits` path
for the current display image.

## Scaling Cache

The software scaling cache is separate from the display-orientation cache. Its key
contains the current image revision, display orientation, quantized target size, and
`ScalingQuality`. The target size is bucketed so very small manual zoom changes do
not force a new large image buffer every paint; the cached result may still be
stretched by GDI by a few pixels to the final destination rectangle.

The cache is explicitly invalidated when the loaded image changes, the display
orientation changes, or the window size changes. Manual zoom changes and panning
rely on the cache key: a zoom that stays in the same target-size bucket reuses the
cache, while a meaningful target-size or quality change rebuilds it. Actual-size
rendering does not populate the scaling cache.

The resident image memory budget and single-entry cache cap come from the configured
memory policy and default to `256 MiB` and `128 MiB`. Display-orientation cache,
scaling cache, animation-frame cache, and navigation-preload entries are described
as cache slots and evicted by a domain policy when the total retained image bytes,
cache entry count, or per-entry limit is exceeded. The app can evict caches but not
the current image buffer; if only the current image exceeds the budget, rendering
falls back to the available current buffer and avoids building additional caches.
When the current image has a non-identity EXIF orientation or user rotation, the
display-orientation buffer is required to paint the current image and is accounted
as protected current display state rather than as an optional cache entry. Background
navigation preloads and scaled render caches must not evict it.
