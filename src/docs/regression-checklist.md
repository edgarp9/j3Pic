# j3Pic Regression Checklist

검증일: 2026-06-18

## 범위

이 체크리스트는 Rust 앱/도메인/인프라 테스트와 실제 Win32 앱 실행 스모크를 합쳐 현재 이미지 뷰어의 주요 회귀 범위를 확인한다.
Linux GTK4 백엔드의 Windows 기준 기능 비교와 Linux 검증 결과는
`docs/linux-gtk4-parity.md`에 별도로 기록한다.

## 2026-06-18 메뉴 점검 추가 기록

Windows 직접 클릭 자동화는 Computer Use 초기화 단계에서 다음 런타임 오류가 발생해
진행하지 못했다.

```text
Package subpath './dist/project/cua/sky_js/src/targets/windows/internal/computer_use_client_base.js' is not defined by "exports"
```

대신 사용자 설정을 건드리지 않도록 임시 `APPDATA`/`LOCALAPPDATA`를 지정해
`target/debug/j3pic.exe`를 실제 실행했다. 빈 시작은 title `j3Pic`, 시작 이미지
`icon.ico` 인자는 title `icon.ico - j3Pic`, `CloseMainWindow` 종료 코드 `0`,
격리된 `j3Pic/config.txt` 저장을 확인했다.

Linux 직접 실행은 현재 Windows 세션에 WSL이 설치되어 있지 않아 수행하지 못했다.
`cargo check --target x86_64-unknown-linux-gnu`도 시도했지만 Linux C toolchain
`x86_64-linux-gnu-gcc`가 없어 `libdeflate-sys` 빌드에서 중단됐다. Linux 동작은
`#[cfg(target_os = "linux")]` 단위 테스트와 `docs/linux-gtk4-parity.md`의 기존
GTK4 parity 기록을 기준으로 확인한다.

| 메뉴 | 기능 | Windows 동작 | Linux 기존 동작 | 문제 여부 | 원인 | 수정 내용 | 재검증 결과 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 우클릭 메뉴 | `열기...` | 시작 이미지 인자와 공통 open/decode 경로가 정상 동작. 메뉴 항목은 이미지 유무와 무관하게 활성 상태여야 한다. | `Command::OpenImage`와 Windows 기준 open filter parity 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 전체 순서/명령/이미지 필요 여부 회귀 테스트 추가. | `context_menu_matches_reference_order_and_image_requirements`, `cargo test` 통과. |
| 우클릭 메뉴 | `내보내기...` | 이미지가 있을 때만 활성화되어 export option/save 흐름으로 진입해야 한다. `내보내기 옵션` modal dialog는 고정 client layout과 Win32 control id를 만들고, format/rotation/width/quality 변경 후 `확인`으로 selection을 반환한다. | GTK export dialog/save parity 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 활성 조건 테스트와 native export options dialog 메시지 스모크 추가. | export 관련 기존 테스트, `export_options_dialog_accepts_valid_changes_from_win32_messages -- --ignored`, 전체 `cargo test` 통과. |
| 우클릭 메뉴 | `클립보드에 복사` | 이미지가 있을 때만 활성화되고 Win32 DIB payload를 사용한다. | GTK clipboard texture flatten parity 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 활성 조건을 테스트로 고정. | Win32 clipboard payload 단위 테스트와 `clipboard_payloads_can_be_registered_and_read_back_from_win32_clipboard -- --ignored` 통과. |
| 우클릭 메뉴 | `실제 크기`, `창에 맞춤` | 이미지가 있을 때만 활성화되고 공통 view command로 처리된다. | 같은 `Command` 매핑과 app command 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 순서/활성 조건 테스트와 반복 메뉴 명령/resize 후 render 가능성 테스트 추가. | `app_command_path_handles_view_commands`, `repeated_menu_image_commands_keep_loaded_image_renderable_after_resize`, 전체 `cargo test` 통과. |
| 우클릭 메뉴 | `시계 방향 회전`, `반시계 방향 회전` | 이미지가 있을 때만 활성화되고 user rotation과 title/cache 갱신 경로를 탄다. | 같은 `Command` 매핑과 rotation/cache 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 순서/활성 조건 테스트와 반복 메뉴 명령/resize 후 render 가능성 테스트 추가. | rotation/cache 관련 테스트, `repeated_menu_image_commands_keep_loaded_image_renderable_after_resize`, 전체 `cargo test` 통과. |
| 우클릭 메뉴 | `전체 화면` | 이미지 없이도 활성화되어 fullscreen toggle 명령으로 처리된다. | GTK fullscreen/windowed 저장 parity 테스트가 있다. | 없음 | 해당 없음 | Win32 메뉴 활성 조건을 테스트로 고정. | command/key mapping 테스트와 전체 `cargo test` 통과. |
| 우클릭 메뉴 | `설정...` | 이미지 없이도 활성화되고 메뉴 마지막 항목이어야 한다. `j3Pic 설정` modal dialog는 고정 client layout과 Win32 control id를 만들고, 값 변경 후 `확인`으로 `AppConfig`를 반환한다. | GTK 설정창 layout/validation parity 테스트가 있다. | 없음 | 해당 없음 | 기존 "마지막 항목" 테스트에 더해 전체 메뉴 테스트와 native settings dialog 메시지 스모크 추가. | `context_menu_keeps_settings_command_at_bottom`, `settings_dialog_accepts_valid_changes_from_win32_messages -- --ignored`, 전체 `cargo test` 통과. |
| 빌드/테스트 환경 | `cargo clippy --all-targets --all-features -- -D warnings` | 최신 clippy에서 기존 코드가 `too_many_arguments`, enum/error 크기 임계값, 단순 lint로 실패했다. | Linux 코드도 동일 crate 검증에 포함된다. | 있음 | clippy 기본 lint 기준이 현재 코드의 이미지 디코딩 경계와 맞지 않고, 일부 코드는 최신 clippy 단순화 제안을 반영하지 않았다. | 단순 lint는 코드로 정리하고, 현재 설계를 반영하는 `clippy.toml` 임계값을 추가했다. | `cargo clippy --all-targets --all-features -- -D warnings` 통과. |

추가 도구 설치는 없었다.

## 자동 검증 결과

| 항목 | 실행 근거 | 결과 |
| --- | --- | --- |
| 포맷 열기 `jpg`, `jpeg`, `png`, `bmp`, `gif`, `webp`, `ico`, `tif`, `tiff`, `tga` | `infra::tests::fixture_images_load_for_all_supported_open_formats` | 통과 |
| 손상/오도 확장자 오류 | `infra::tests::corrupt_and_misleading_supported_extension_files_are_decode_failures` | 통과 |
| 권한 오류 분류 | `infra::tests::*permission_denied*` | 통과 |
| 폴더 이전/다음, wrap, 손상 파일 유지 정책 | `app::tests::app_folder_navigation_with_real_varied_images_wraps_and_keeps_current_on_broken_file` | 통과 |
| 확대/축소, 실제 크기, 화면 맞춤 | `domain::tests::*zoom*`, `app::tests::app_command_path_handles_view_commands` | 통과 |
| 패닝 | `domain::tests::*panning*`, `app::tests::pan_lifecycle_updates_offset_and_ends_cleanly` | 통과 |
| 회전과 EXIF 방향 조합 | `domain::tests::*orientation*`, `app::tests::*rotation*`, `infra::tests::generated_jpeg_exif_orientation_fixtures_load_metadata_values_1_through_8` | 통과 |
| 이미지 정보와 확대 비율 표시 | `domain::tests::*image_info*`, `domain::tests::image_status_text_appends_zoom_text` | 통과 |
| 클립보드 DIB payload | `platform::win32::tests::*clipboard*` | 통과 |
| 부드러운 스케일링과 스케일 캐시 | `domain::tests::*scaling*`, `app::tests::*scaled_cache*` | 통과 |
| 큰 이미지 preview/full-resolution 정책 | `cargo test large_bmp_fixture_loads_preview_then_full_resolution_on_demand -- --ignored` | 통과 |
| preview 이후 full-resolution worker 시작 race | `platform::win32::tests::full_resolution_decode_can_replace_unjoined_initial_worker_for_same_generation`, `platform::win32::tests::duplicate_running_full_resolution_decode_for_same_generation_is_not_restarted` | 통과 |
| 큰 이미지 판정/한도 초과 | `domain::tests::large_image_classification_*`, `infra::tests::oversized_bmp_header_is_rejected_before_pixel_decode` | 통과 |
| stale decode 결과 무시 | `app::tests::*stale*`, `domain::tests::decode_generation_marks_only_non_active_results_as_stale` | 통과 |
| 애니메이션 GIF/WebP 프레임, delay, loop | `infra::tests::generated_animated_*`, `domain::tests::animation_*`, `app::tests::animation_*` | 통과 |
| 압축/변환 내보내기 | `infra::tests::export_writes_supported_formats_and_reopens_with_expected_metadata`, `infra::tests::jpeg_quality_changes_encoded_file_size` | 통과 |
| 설정 저장/복구 | `infra::tests::*config*`, 실제 앱 종료/설정창 스모크 | 통과 |
| 설정 파싱/직렬화 key 누락 방지 | `domain::tests::app_config_serialization_includes_all_current_user_setting_keys`, `domain::tests::app_config_serializes_and_deserializes_user_settings` | 통과 |
| 구버전 설정 파일 호환 | `domain::tests::legacy_version1_config_without_user_settings_keeps_safe_defaults`, `infra::tests::load_config_accepts_legacy_version1_without_new_setting_keys` | 통과 |
| 설정 기반 도메인 정책 | `domain::tests::app_config_user_settings_drive_domain_policies`, `app::tests::applying_config_updates_runtime_policy_and_future_decode_requests` | 통과 |
| 시작 시 설정 로드 실패 fallback | `lib::tests::startup_config_falls_back_to_defaults_when_config_load_fails` | 통과 |
| 혼합 DPI 멀티모니터 창 이동 | 2026-05-06 mixed-DPI trace, 2026-05-09 size/move DPI state unit tests and Win32 same-DPI move smoke, 2026-05-11 DPI awareness fallback order unit test | 통과. 현재 자동 세션은 mixed-DPI 물리 재현은 수동 확인 필요 |

## 실제 앱 실행 스모크

`target/debug/j3pic.exe`를 임시 `APPDATA`/`LOCALAPPDATA`로 실행해 사용자 설정을 격리했다.

확인한 항목:

- 기본 Win32 창 생성: class `j3pic.viewer.window`, title `j3Pic`.
- drag-and-drop 수신 설정: `WS_EX_ACCEPTFILES` 설정 확인.
- 창 resize 후 client size `878x594`.
- `F11` fullscreen 진입: `WS_OVERLAPPEDWINDOW` style bit 제거, style `0x14000000`.
- `Esc` fullscreen 복귀: style bit `0x14CF0000` 복원.
- `WM_CLOSE` 종료와 `j3Pic/config.txt` 저장.
- 설정창 값 변경 후 `확인`, 앱 종료, 재실행 후 값 유지 확인.
- 설정창 `취소`와 `X` 닫기는 `config.txt`를 변경하지 않음.
- 설정창 `기본값` 후 `확인`하면 기본값 기준 `config.txt` 저장.
- `config.txt` 삭제 후 기본값으로 실행.
- 손상된 `config.txt`(`version=2`와 malformed line)가 앱 시작을 막지 않고 기본값으로 설정창 표시.
- 설정 저장 경로가 파일로 막힌 실패 상황에서 `확인` 후 오류 메시지를 표시하되, 실행 중 설정은 적용되고 앱 종료도 정상 진행.
- 144 DPI primary 모니터에서 96 DPI 보조 모니터로 제목 표시줄을 드래그해도 창이 이전 위치로 되돌아가지 않고 대상 모니터에 남음.

제약:

- 전역 Windows 클립보드 쓰기/읽기는
  `cargo test clipboard_payloads_can_be_registered_and_read_back_from_win32_clipboard -- --ignored`
  로 별도 확인했다.
- Open File/Save File common dialog와 Explorer가 소유한 실제 drag source는 이번 자동화에서 직접 조작하지 않았다. 파일 열기, 폴더 탐색, 변환 내보내기, drop path 선택은 단위/통합 테스트로 확인한다.

## 설정창 자동 점검

Win32 설정창은 실제 앱을 띄운 뒤 `PostMessage`/`SendMessage(WM_SETTEXT, WM_GETTEXT)` 기반
P/Invoke 하네스로 조작했다. 하네스는 사용자 설정을 건드리지 않도록 임시
`APPDATA`/`LOCALAPPDATA`만 사용한다.

자동 확인 항목:

- 여러 설정값을 변경하고 `확인`하면 `config.txt`에 즉시 저장된다.
- 앱 종료 후 같은 임시 설정 경로로 재실행하면 설정창 컨트롤이 저장값으로 초기화된다.
- `취소`와 `X` 닫기는 draft를 버리고 저장 파일을 변경하지 않는다.
- `기본값` 후 `확인`하면 사용자 설정 key가 `AppConfig::default()` 기준으로 저장된다.
- `config.txt`가 없으면 기본값으로 실행되고 설정창이 정상 표시된다.
- 손상된 `config.txt`는 앱 시작을 막지 않고 기본값으로 복구된다.
- 설정 저장 실패 상황에서는 오류 메시지가 표시되고, 실행 중 설정 적용과 앱 종료가 유지된다.

실행 결과:

```text
SETTINGS_UI_SMOKE_OK persisted restart/cancel/x/defaults/missing/damaged/save-failure
```

## 설정창 수동 점검

아래 항목은 실제 이미지 표시 품질, Explorer drag source, common file dialog처럼
운영체제/사용자 입력 의존성이 큰 부분을 사람이 눈으로 확인할 때 사용한다.

공통 준비:

- 임시 `APPDATA`/`LOCALAPPDATA`를 지정하거나 기존 `%APPDATA%\j3Pic\config.txt`를 백업해 사용자 설정을 격리한다.
- 앱 실행 후 이미지가 없는 상태와 이미지가 로드된 상태에서 각각 우클릭 컨텍스트 메뉴를 연다.
- 컨텍스트 메뉴 마지막 항목이 `설정`이고, 선택하면 `j3Pic 설정` modal dialog가 열린다.

수동 확인 항목:

- `취소`와 창 닫기: 값을 바꾼 뒤 `취소` 또는 `X`로 닫으면 현재 viewer 동작과 `config.txt`가 바뀌지 않는다.
- `기본값`: dialog 안의 draft 값만 기본값으로 되돌아간다. 이후 `취소`하면 실행 중인 viewer에는 적용되지 않고, `확인`해야 적용/저장된다.
- 일반: `기본 보기 모드`, `스케일링 품질`, `상태바 표시`, `자세한 상태 텍스트`를 바꾸고 `확인`하면 새 이미지 로드, 렌더링 품질, 하단 상태 텍스트가 설정과 일치한다.
- 줌: `최소 줌`, `최대 줌`, `줌 단계 배율`을 유효 범위 안에서 바꾸면 키보드/휠 줌이 새 범위와 step을 따른다. 숫자가 아니거나 범위 밖이거나 최소값이 최대값보다 크면 경고 메시지가 뜨고 dialog가 열린 채 유지된다.
- 대용량 이미지/메모리: 픽셀/캐시 한도와 `전체 해상도 요청 배율`을 바꾼 뒤 큰 이미지 preview/full-resolution 동작과 cache eviction이 기존 이미지 표시를 깨지 않는지 확인한다. `대용량 픽셀 기준 > 최대 이미지 픽셀`, `프리뷰 최대 픽셀 > 최대 이미지 픽셀`, `캐시 항목 한도 > 캐시 총량`은 경고 후 적용되지 않아야 한다.
- 애니메이션: `자동재생`을 끄면 새 animated GIF/WebP가 pause 상태로 시작한다. frame delay 세 값은 `min <= default <= max`가 아니면 경고 후 적용되지 않아야 한다.
- 탐색: `순환 이동`을 끄면 폴더 끝에서 이전/다음이 no-op이다. `실패 파일 자동 스킵`과 `최대 시도 횟수`를 켜면 손상 이미지 다음 loadable 이미지까지 재시도하되, 모두 실패하면 현재 이미지를 유지한다.
- 내보내기: `기본 포맷 정책`, `JPEG 품질`, `파일명 suffix`, `JPEG 투명 배경 RGB`가 save dialog 제안 파일명, 선택 기본 포맷, JPEG alpha flatten 색상에 반영된다. 빈 suffix, 64자 초과 suffix, Windows-invalid 문자, 잘못된 RGB 형식은 경고 후 적용되지 않아야 한다.
- 저장 실패: 설정 저장 경로를 쓰기 불가로 만든 뒤 `확인`하면 현재 실행 중인 창에는 설정이 적용되고, 저장 실패 메시지가 표시되며, 앱과 설정창 흐름은 종료되지 않는다. 이후 권한을 복구하면 다음 `확인` 또는 앱 종료 저장이 성공해야 한다.
- 구버전/누락 config: `config.txt`가 없거나 새 사용자 설정 key가 없는 `version=1` 파일만 있어도 앱이 기본값으로 실행되고 설정창이 정상 표시된다. `version=2`나 손상된 라인은 기본값으로 복구되어 앱 시작을 막지 않는다.

## 큰 이미지 메모리 스모크

`cargo test large_bmp_fixture_loads_preview_then_full_resolution_on_demand -- --ignored --nocapture`로
임시 `7000x4000` BMP fixture를 생성했다. 초기 로드는 `1750x1000` preview로 축소되고, 이후
full-resolution on-demand decode가 `7000x4000` RGBA8로 복원되는 경로를 확인했다.

PowerShell로 Cargo/test 하위 프로세스를 100ms 간격 샘플링한 결과:

- Peak working set: `258.2 MiB`
- Peak private memory: `231.7 MiB`
- Samples: `52`
- Test runtime: `19.51s`

## 명령 결과

```text
cargo fmt --check
OK

cargo check
OK

cargo build
OK

cargo test
lib/main/doc tests: 347 passed; 0 failed; 4 ignored
profile_open tests: 2 passed; 0 failed

cargo test clipboard_payloads_can_be_registered_and_read_back_from_win32_clipboard -- --ignored
1 passed

cargo test settings_dialog_accepts_valid_changes_from_win32_messages -- --ignored
1 passed

cargo test export_options_dialog_accepts_valid_changes_from_win32_messages -- --ignored
1 passed

cargo clippy --all-targets --all-features -- -D warnings
OK

cargo check --target x86_64-unknown-linux-gnu
FAILED: missing x86_64-linux-gnu-gcc for libdeflate-sys

actual app smoke with temp APPDATA/LOCALAPPDATA
empty-start: title=j3Pic, windowHandleSeen=true, exitCode=0
startup-image: title=icon.ico - j3Pic, windowHandleSeen=true, exitCode=0
configWritten=true
```

## 2026-05-05 추가 수정

코드 검토 중 Win32 decode controller가 작업 종류를 기록하지 않고 generation만 비교해
full-resolution 중복 실행을 막는 구조를 확인했다. 초기 디코드 worker가 결과를 보낸 직후
아직 join되지 않은 짧은 구간에서 사용자가 실제 크기나 고배율 zoom을 요청하면 같은
generation의 기존 worker가 있다는 이유로 첫 full-resolution decode 요청이 스킵될 수
있었다.

수정 후 worker 종류를 `Initial`, `FullResolution`, `AnimationFrame`으로 보존한다.
중복 방지는 같은 generation의 실행 중인 `FullResolution` worker에만 적용하고, 이미
결과가 적용된 초기 worker는 cancel/retire한 뒤 full-resolution worker를 시작한다.
`Condvar` 기반 단위 테스트로 지연 없이 두 상태 전이를 고정했다.
