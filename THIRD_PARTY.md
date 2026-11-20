# Third-party components

Android Device Desk runs the unmodified scrcpy 5.0.1 Windows x64 distribution. Its engine archive is verified against the SHA-256 published by the scrcpy project before being embedded in the release executable. The contents extract into `%LOCALAPPDATA%\AndroidDeviceDesk\engine\scrcpy-win64-v5.0.1`. The scrcpy executable loads its multimedia libraries as separate DLLs from that directory.

The release includes a separate `third-party-sources.zip` download with the upstream source archives below, the applicable build scripts in the scrcpy source tree, platform-tools notices, and license files for the Rust dependencies.

| Component | Upstream source or notices |
| --- | --- |
| scrcpy 5.0.1 | [Source and Apache-2.0 license](https://github.com/Genymobile/scrcpy/tree/v5.0.1) · [Windows release build script](https://github.com/Genymobile/scrcpy/blob/v5.0.1/release/build_windows.sh) |
| FFmpeg 9.0.2 | [Corresponding source](https://ffmpeg.org/releases/ffmpeg-9.0.2.tar.xz) · [Build recipe](https://github.com/Genymobile/scrcpy/blob/v5.0.1/app/deps/ffmpeg.sh) · [License terms](https://ffmpeg.org/legal.html). The bundled DLLs report LGPL 2.1 or later. |
| dav1d 1.5.4 | [Corresponding source and license](https://downloads.videolan.org/pub/videolan/dav1d/1.5.4/dav1d-1.5.4.tar.xz) · [Build recipe](https://github.com/Genymobile/scrcpy/blob/v5.0.1/app/deps/dav1d.sh) |
| libusb 1.0.30 | [Source and LGPL license](https://github.com/libusb/libusb/tree/v1.0.30) · [Build recipe](https://github.com/Genymobile/scrcpy/blob/v5.0.1/app/deps/libusb.sh) |
| SDL 3.4.18 | [Source and zlib license](https://github.com/libsdl-org/SDL/tree/release-3.4.18) · [Build recipe](https://github.com/Genymobile/scrcpy/blob/v5.0.1/app/deps/sdl.sh) |
| ADB, Android SDK Platform-Tools 37.0.1 | [AOSP ADB source](https://android.googlesource.com/platform/packages/modules/adb/) · [Upstream package recipe](https://github.com/Genymobile/scrcpy/blob/v5.0.1/app/deps/adb_windows.sh) · `platform-tools-37.0.1-NOTICE.txt` in the source asset |
| Gnirehtet 2.5.1 | [Source and original release](https://github.com/Genymobile/gnirehtet/tree/v2.5.1) · [Apache-2.0 license](third-party/gnirehtet-LICENSE.txt) |

Rust crate versions are pinned in [Cargo.lock](rust-app/Cargo.lock). The release asset also contains the Windows x64 dependency inventory and the license and notice files provided in those crates.
