# j3Pic

A lightweight native Windows image viewer built in Rust for quick image opening,
navigation, viewing, and export.

## Project Status

j3Pic was created with AI assistance using an in-house tool. It is usable for
personal workflows, but the test coverage is still limited. Please treat the
project as experimental and verify behavior with your own images before relying
on it for important work.

## Features

- Open common image formats: JPEG, PNG, BMP, GIF, WebP, ICO, TIFF, and TGA.
- Navigate images in the current folder with keyboard and mouse shortcuts.
- View static images and animated GIF/WebP files.
- Respect EXIF orientation without modifying the original file.
- Rotate the current view in 90-degree steps.
- Zoom, fit to window, show actual size, and pan large images.
- Copy the display-oriented image to the Windows clipboard.
- Export images as PNG, JPEG, BMP, WebP, or ICO.
- Resize, rotate, and remove metadata during export.
- Configure language, zoom behavior, animation, navigation, export defaults,
  memory limits, and shortcuts.
- Use a native Win32 UI with a context menu, settings dialog, export options
  dialog, drag-and-drop, and fullscreen support.

## Build

Requirements:

- Rust toolchain
- Windows development environment capable of building Rust Win32 applications

Build from the Rust project directory:

```powershell
cd src
cargo build --release
```

The executable is created under `src/target/release/`.

You can also use the helper script:

```powershell
cd src
python build_release.py
```

## Run

Run without arguments to open an empty viewer:

```powershell
.\target\release\j3pic.exe
```

Run with an image path to open it at startup:

```powershell
.\target\release\j3pic.exe C:\path\to\image.jpg
```

## Shortcuts

| Shortcut | Action |
| --- | --- |
| `Ctrl+O` | Open image |
| `Ctrl+S` or `Ctrl+Shift+S` | Export image |
| `Ctrl+C` | Copy image to clipboard |
| `Right`, `PageDown`, mouse wheel | Next image |
| `Left`, `Backspace`, `PageUp` | Previous image |
| `+` / `-` | Zoom in / out |
| `1` | Actual size |
| `0` | Fit to window |
| `R` | Rotate clockwise |
| `Shift+R` | Rotate counterclockwise |
| `F11` or `Alt+Enter` | Toggle fullscreen |
| `Esc` | Leave fullscreen, or exit when not fullscreen |
| `Q` or `Alt+F4` | Exit |

Mouse and keyboard behavior can be changed in the settings dialog.

## Configuration

j3Pic stores user settings in the per-user Windows application data folder under
the `j3Pic` directory. If the configuration file is missing or invalid, the app
falls back to default settings.

## License

This project is licensed under the GNU General Public License v3.0. See
[LICENSE](LICENSE) for details.

## Notices and Thanks

This project uses icons from [Google Fonts Icons](https://fonts.google.com/icons).
Google Material Symbols and Icons are made available under the
[Apache License 2.0](https://www.apache.org/licenses/LICENSE-2.0).

Thank you to Google and the Material Symbols and Icons contributors for making
these icon resources available.
