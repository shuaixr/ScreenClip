# ScreenClip

![ScreenClip application icon](assets/icons/icons.png)

> This project is in an early stage of development. Expect frequent changes and occasional rough edges.

ScreenClip is a Rust based screenshot utility built with egui and winit, focused on fast screen capture and annotation workflows, with global hotkey support and a desktop overlay UI.

## Build

### Prerequisites

- Windows 10 or Windows 11
- Rust toolchain (stable), including `cargo`

Install Rust (if needed):

```powershell
winget install Rustlang.Rustup
```

Then, from the project root:

```powershell
cargo build
```

For an optimized build:

```powershell
cargo build --release
```

## Run

From the project root:

```powershell
cargo run
```

Or run the release binary after building:

```powershell
.\target\release\screenclip.exe
```

## Supported Platforms

- Windows: Supported (current target)
- macOS: Planned
- Linux: Planned

Cross-platform support is on the roadmap, but development and testing are currently focused on Windows.
