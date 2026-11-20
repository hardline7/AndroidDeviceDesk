# 공개·배포 안내 / Distribution

## 프로젝트 소스

Android Device Desk의 자체 작성 코드, 빌드 스크립트, 문서는 [Apache License 2.0](../LICENSE)을 적용합니다. 배포할 때 라이선스 원문과 [NOTICE](../NOTICE)를 함께 제공하세요. 변경한 파일에는 변경했다는 표시를 남기고, 해당하는 기존 저작권 및 출처 고지를 유지하세요. 저장소의 화면 캡처에 표시된 다른 앱, 상표, 기기 화면의 권리는 해당 권리자에게 있습니다. [Apache-2.0 배포 조항](https://www.apache.org/licenses/LICENSE-2.0#redistribution)

## Windows EXE 릴리스 준비

Windows EXE는 별도 프로세스로 실행하는 scrcpy, ADB와 USB 인터넷 공유용 Gnirehtet을 포함합니다. 구성 요소마다 적용되는 라이선스와 출처를 [THIRD_PARTY.md](../THIRD_PARTY.md)에서 확인할 수 있습니다.

| 준비할 항목 | 확인할 내용 |
| --- | --- |
| 프로젝트 라이선스와 고지 | `LICENSE`, `NOTICE`, `THIRD_PARTY.md`와 번들 구성 요소의 라이선스·고지를 배포 파일과 함께 제공합니다. |
| 런타임과 버전 | `scripts/build.ps1`에 지정된 아카이브 버전과 SHA-256이 실제 빌드에 사용한 파일과 일치하는지 확인합니다. Gnirehtet은 Windows 중계 프로그램과 Android APK를 모두 포함합니다. |
| 대응 소스 | 번들의 FFmpeg·libusb 등 LGPL 구성 요소와 일치하는 소스, 변경 사항, 빌드 스크립트를 EXE와 같은 릴리스에서 내려받을 수 있게 제공합니다. FFmpeg는 정확한 소스 버전과 빌드 옵션을 확인하고 다운로드 페이지에 FFmpeg 사용 및 라이선스를 고지합니다. [FFmpeg 배포 안내](https://ffmpeg.org/legal.html) |
| DLL 교체 | FFmpeg·libusb의 ABI 호환 DLL로 바꾼 후 다시 실행해 교체본이 유지되는지 확인합니다. DLL 교체 방법은 아래에 있습니다. [LGPL 2.1 제6조](https://www.gnu.org/licenses/old-licenses/lgpl-2.1.html#SEC4) |
| ADB와 Platform-Tools | 실제 포함하는 `adb.exe`와 보조 DLL의 출처, 버전, 배포 조건 및 고지를 확인합니다. Android SDK의 오픈소스 구성 요소는 각각의 오픈소스 라이선스가 적용됩니다. 번들에 넣는 파일의 조건이 확인되면 관련 내용을 릴리스 고지에 포함합니다. [Android SDK 약관 제3.4–3.5조](https://developer.android.com/studio/terms) |
| Rust 의존성 | 릴리스에 사용한 `Cargo.lock`을 기준으로 빌드에 포함된 크레이트 및 글꼴의 라이선스·저작권 고지를 모읍니다. |
| EXE와 체크섬 | 새 버전 태그의 소스, EXE, 라이선스·대응 소스 묶음을 동일한 릴리스에 연결하고 공개된 파일의 SHA-256을 확인합니다. |

`v0.1.0`의 EXE와 소스 묶음은 그 릴리스 버전을 기준으로 확인합니다. 위 표는 다음 EXE를 배포하기 위한 기준입니다. [기존 릴리스](https://github.com/hardline7/AndroidDeviceDesk/releases/tag/v0.1.0)

## 호환 DLL 교체

1. 기기 데스크와 실행 중인 미러링 창을 닫습니다.
2. 파일 탐색기에서 `%LOCALAPPDATA%\AndroidDeviceDesk\engine\scrcpy-win64-v5.0.1`을 엽니다. 이 경로는 설치한 엔진 버전에 따라 달라집니다.
3. 기존 DLL을 별도 위치에 복사해 보관합니다. 같은 파일 이름과 Windows x64 ABI를 가진 호환 DLL을 해당 폴더에 넣습니다.
4. 기기 데스크를 다시 열고 미러링을 실행합니다. 같은 버전의 엔진 폴더에 있는 DLL은 파일 크기가 달라도 그대로 사용됩니다.

기본 DLL로 되돌리려면 앱과 미러링을 닫은 뒤 필요한 DLL을 엔진 폴더에서 다른 위치로 옮깁니다. 다음 실행 때 빠진 파일을 내장 아카이브에서 다시 풀어 줍니다.

## English summary

The project's original code, build scripts, and written documentation use [Apache-2.0](../LICENSE). Keep the project [NOTICE](../NOTICE) and applicable upstream notices when distributing copies. The bundled components have their own terms, listed in [THIRD_PARTY.md](../THIRD_PARTY.md).

For each Windows release, record the exact archive versions and hashes, verify the terms for all bundled ADB files, and publish applicable licenses and notices plus corresponding LGPL source and build scripts alongside the EXE. Keep the release tag, executable, hashes, and source archive aligned. [FFmpeg distribution guidance](https://ffmpeg.org/legal.html) · [Android SDK terms](https://developer.android.com/studio/terms)

To replace a DLL, close Device Desk and any standalone mirror windows, back up the original, and put the ABI-compatible Windows x64 DLL with the same file name in `%LOCALAPPDATA%\AndroidDeviceDesk\engine\scrcpy-win64-v5.0.1` or the applicable version folder. Restart Device Desk. Existing DLLs in that version folder are preserved. To restore the bundled version, move the replacement out of the folder before restarting.
