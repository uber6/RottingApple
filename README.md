# RottingApple

PC-to-Apple TV AirPlay screen mirroring for **Windows** and **Linux**.

RottingApple acts as an AirPlay 2 **sender**: it discovers Apple TVs on your LAN, pairs with a PIN, captures your desktop, encodes H.264 video, and streams it to the TV.

## Features

- mDNS discovery of Apple TVs (`_airplay._tcp`)
- Legacy AirPlay `pair-setup-pin` pairing with credential persistence
- Screen mirroring (mirror mode)
- Linux capture via X11 (Wayland via XWayland)
- Windows capture via DXGI Desktop Duplication
- Virtual-display-only capture for extend-like workflows (experimental)
- Test mode with synthetic video (no display server required)
- Experimental audio mirroring hook (AAC-ELD pipeline stub)

## Requirements

### Linux

- Rust 1.85+ (edition 2024)
- X11 display server (or XWayland on Wayland)
- Network access to Apple TV on the same subnet

### Windows

- Rust 1.85+
- Windows 10/11 with DXGI-capable GPU
- Apple TV on the same network
- `openh264-2.6.0-win64.dll` and `fpsap-helper.exe` next to `rottingapple.exe` (built by `scripts/build-windows.sh`)

### Apple TV

- AirPlay enabled: **Settings → AirPlay and HomeKit**
- Note the 4-digit PIN when pairing (displayed on TV after pairing starts)

## Build

```bash
cargo build --release
```

Binary: `target/release/rottingapple`

### Cross-compile for Windows (from Linux/WSL)

Install MinGW and the Rust Windows target, then build:

```bash
# One-time setup (Ubuntu/Debian/WSL)
sudo apt install mingw-w64
rustup target add x86_64-pc-windows-gnu

# Build
./scripts/build-windows.sh
```

Output: `target/x86_64-pc-windows-gnu/release/rottingapple.exe`

Copy that `.exe` to Windows and run it in PowerShell or cmd. This build uses the **DXGI** capture backend and mirrors the **Windows desktop** (not the WSL Linux environment).

```powershell
.\rottingapple.exe discover
.\rottingapple.exe mirror --target 192.168.1.50
```

The MinGW-linked binary requires no MSVC runtime. Windows may prompt for firewall access on first run.


## Usage

```bash
# Discover Apple TVs on the LAN
rottingapple discover

# Pair (first time — enter PIN when prompted after it appears on TV)
rottingapple pair --target 192.168.1.50

# Pair non-interactively (scripts / CI)
rottingapple pair --target 192.168.1.50 --pin 1234

# Mirror primary display
rottingapple mirror --target 192.168.1.50

# Mirror with options (software OpenH264 encoder)
rottingapple mirror --target appletv.local --width 1920 --height 1080 --fps 30 --hwaccel none

# Test mode (synthetic pattern, no screen capture)
rottingapple mirror --target 192.168.1.50 --test

# Extend-like mode: capture only virtual displays (experimental)
rottingapple mirror --target 192.168.1.50 --virtual-display --display 1
```

### Credentials

Pairing credentials are stored at:

```
~/.config/rottingapple/credentials.json
```

On Linux/macOS the config directory is created with mode `0700` and the credentials file with mode `0600`.

Override with `--creds /path/to/creds.json`.

### Debug logging

Optional developer trace logs (disabled by default):

```bash
ROTTINGAPPLE_DEBUG_LOG=1 rottingapple mirror --target 192.168.1.50 --test
```

## Architecture

```
crates/
  rotten-core/       Config, device types, errors
  rotten-discovery/  mDNS browse + manual target resolve
  rotten-crypto/     SRP, ChaCha20-Poly1305, FairPlay SAP
  rotten-pairing/    Legacy pair-setup-pin (+ experimental HAP stub)
  rotten-protocol/   HTTP mirror setup + RTSP session
  rotten-video/      H.264 encode (OpenH264 software), frame pacing, encrypted stream
  rotten-capture/    X11 (Linux) / DXGI (Windows) backends
  rotten-app/        CLI binary
```

## Extend display (virtual monitor)

Native AirPlay extend is not available from non-Apple senders. For extend-like behavior:

1. Install a virtual display driver (e.g. [Virtual Display Driver](https://github.com/VirtualDrivers/Virtual-Display-Driver) on Windows).
2. Configure it as an extended desktop in OS display settings.
3. Run `rottingapple mirror --virtual-display --display <index>`.

## Limitations

- Pairing uses legacy `pair-setup-pin`, not full HomeKit Accessory Protocol pairing.
- FairPlay `fp-setup` requires `fpsap-helper` (GPL-3.0) as a separate binary; see `THIRD_PARTY_NOTICES.md`.
- Video encoding uses **OpenH264 software only**; `--hwaccel nvenc` / `vaapi` are not implemented yet.
- Audio mirroring is experimental.
- Corporate networks blocking mDNS require `--target <ip>`.

## License

MIT OR Apache-2.0 for the RottingApple application code.

Third-party components (OpenH264, Playfair, fpsap-helper) have separate licenses — see [THIRD_PARTY_NOTICES.md](THIRD_PARTY_NOTICES.md).
