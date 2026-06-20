# Win32 검증 기록

검증일: 2026-05-05

## 자동 확인

다음 항목은 `target/debug/j3pic.exe`를 실제 실행한 뒤 PowerShell P/Invoke 하네스로
확인했다. 앱은 임시 `APPDATA`/`LOCALAPPDATA`를 주입해 사용자 설정을 격리했고,
하네스 종료 후 임시 설정 디렉터리를 삭제했다.

- 기본 창 생성: class `j3pic.viewer.window`, title `j3Pic`.
- `WM_CREATE`: 생성 직후 title 동기화와 `DragAcceptFiles` 경로까지 도달.
- `WM_SIZE`: `SetWindowPos(120, 120, 900, 650)` 후 client size `878x594`.
- drag-and-drop 수신 설정: `WS_EX_ACCEPTFILES` 설정 확인.
- fullscreen enter: `WM_KEYDOWN/F11` 전송 후 `WS_OVERLAPPEDWINDOW` style 제거 확인.
- fullscreen exit: `WM_KEYDOWN/Esc` 전송 후 windowed style 복원 확인.
- 정상 종료: `WM_CLOSE` 후 프로세스가 5초 안에 종료.
- 종료 시 `j3Pic/config.txt` 저장 확인.

확인된 스모크 값:

```text
StyleFullscreen       = 0x14000000
StyleRestored         = 0x14CF0000
ClientSize            = 878x594
AcceptDrops           = True
ConfigSaved           = True
```

2026-05-05 검증 세션은 foreground window가 없는 비대화형 컨텍스트로
`SetForegroundWindow`가 실패한다. 그래서 실제 키보드 입력으로 common file dialog를
여는 `Ctrl+O` 자동화와 Explorer가 소유한 실제 drag source 자동화는 신뢰 가능한
자동 결과로 사용하지 않았다. 대신 foreground가 필요 없는 `SendMessage` 기반 키
입력(`F11`, `Esc`)과 단위 테스트의 중앙 `Command` 매핑, 같은 프로세스에서 만든
유효한 `HDROP` 단위 테스트로 해당 경로를 검증했다.

전역 Windows 클립보드는 다음 ignored 테스트를 별도 실행해 CF_DIB/CF_DIBV5 등록과
읽기 경로를 확인했다.

```text
cargo test clipboard_payloads_can_be_registered_and_read_back_from_win32_clipboard -- --ignored
1 passed
```

## 2026-05-06 멀티모니터 이동 회귀 확인

사용자 보고 증상: DPI가 다른 멀티모니터 환경에서 제목 표시줄을 마우스로 끌어 다른
모니터로 이동할 때, 창이 모니터 경계 근처에서 이전 위치로 되돌아가 다른 모니터로
옮겨지지 않았다.

재현 환경은 4개 모니터였고, 주요 재현 경로는 primary 144 DPI 모니터에서 아래쪽
96 DPI 모니터로 창을 드래그하는 흐름이다. PowerShell P/Invoke 하네스가 실제
`target/debug/j3pic.exe`를 임시 `APPDATA`/`LOCALAPPDATA`로 실행한 뒤 `SendInput`으로
caption drag를 재현했다.

수정 전 PMv2 DPI aware 상태에서 추적한 결과:

```text
before rect=(120,120)-(1020,770) size=900x650
step-50 rect=(372,862)-(1272,1512) size=900x650
step-60 rect=(120,120)-(1020,770) size=900x650
trace: WM_DPICHANGED suggested=(573,1015)-(1173,1448) size=600x433
trace: WM_DPICHANGED suggested=(120,120)-(1020,770) size=900x650
```

원인: 앱이 per-monitor DPI v2를 요청해 native system move loop 도중 `WM_DPICHANGED`
전환이 발생했다. 혼합 DPI/비직사각 모니터 배치에서 이 전환이 기존 DPI 좌표계로
되돌아가는 suggested rectangle을 만들었고, 사용자가 계속 끄는 중인 최상위 창 위치가
원래 모니터 위치로 복원되었다. 앱은 custom top-level move loop를 소유하지 않으므로,
창 이동 안정성을 위해 system DPI aware로 고정했다.

수정 후 확인 값:

```text
process DPI awareness = 1 (system DPI aware)
before rect=(120,120)-(1020,770) size=900x650
step-50 rect=(372,862)-(1272,1512) size=900x650
step-60 rect=(573,1015)-(1173,1448) size=600x433
step-80 rect=(674,1312)-(1274,1745) size=600x433
after rect=(674,1312)-(1274,1745) size=600x433
exit=0
```

창은 더 이상 원래 모니터 위치로 복귀하지 않고 대상 모니터에 남는다. 하네스는 종료 전
`WM_CLOSE`를 보내고 프로세스 종료와 임시 설정 디렉터리 삭제까지 확인했다.

## 2026-05-09 Per-Monitor DPI 유지 수정

2026-05-08 DPI awareness 적용으로 Per-Monitor V2 경로가 다시 활성화되면서
2026-05-06의 system-DPI aware 완화책은 더 이상 현재 동작이 아니게 되었다. 이번
수정은 DPI awareness를 낮추지 않고, native top-level size/move loop와
`WM_DPICHANGED` 처리를 분리한다.

재분석한 원인:

- native move loop 중 `WM_DPICHANGED`가 들어오면 기존 handler가 suggested rectangle을
  즉시 `SetWindowPos`로 적용했다. 혼합 DPI/비직사각 모니터 배치에서는 이 suggested
  rectangle이 사용자가 드래그 중인 위치를 오래된 모니터 좌표로 덮어쓸 수 있다.
- 같은 handler가 `end_pan_and_release_capture`를 호출했다. native move loop 중에도
  capture owner가 viewer HWND일 수 있으므로, 앱이 소유한 image-pan capture가 아닌
  Win32 이동 루프의 capture까지 해제할 수 있었다.
- client-area window move gesture는 `WM_NCLBUTTONDOWN/HTCAPTION`을 합성하면서
  `lParam=0`을 전달했다. `WM_NCLBUTTONDOWN`의 `lParam`은 screen cursor coordinate이므로,
  합성 메시지에도 현재 cursor 좌표를 전달해야 한다.

수정 후 정책:

- `WM_ENTERSIZEMOVE`/`WM_EXITSIZEMOVE`로 native size/move loop 상태를 추적한다.
- 해당 루프 중 `WM_DPICHANGED`는 suggested rectangle, DPI-dependent UI metrics,
  실제 client rect 기반 viewport, repaint를 즉시 적용하지 않고 루프 종료 시 한 번만
  갱신한다. 이 경로는 모니터 경계에서 반복되는 synchronous paint를 피하기 위한
  부드러운 이동 정책이기도 하다.
- native size/move loop 중에는 Win32가 소유한 capture를 해제하지 않는다.
- client-area window move gesture가 `WM_NCLBUTTONDOWN`을 보낼 때 현재 screen cursor
  coordinate을 packed `lParam`으로 전달한다.

현재 Codex 세션의 모니터 4개는 모두 96 DPI라 혼합 DPI 경계의 물리 재현은 수행할 수
없었다. 대신 상태 전이 단위 테스트와 실제 Win32 실행 스모크로 런타임 경로를 확인했다.

```text
cargo fmt --check
OK

cargo check
OK

cargo test
300 passed; 0 failed; 2 ignored

WIN32_MOVE_SMOOTH_SMOKE before=(120,120)-(1020,770) after=(355,414)-(1255,1064) drag_ms=657 exit=0
```

## 단위 테스트 확인

다음 순수 상태 전환은 Rust 단위 테스트로 확인했다.

- 키 입력에서 중앙 `Command` 매핑.
- static/animation 이미지에서 `Space`의 contextual command 해석.
- wheel zoom과 같은 view transform 계산의 anchor 유지.
- 설정 적용 후 keyboard zoom과 wheel-equivalent zoom이 같은 configured
  `zoom_step_factor`, min/max clamp를 사용하는지.
- Win32 mouse wheel delta가 configured step factor로 zoom factor를 계산하는지.
- 설정 적용 후 새 이미지 로드가 `FitToWindow`/`ActualSize` 기본 보기 모드를
  사용하는지.
- 설정 적용 후 `Nearest`/`Balanced`/`HighQuality` 스케일링 품질이 render path와
  software scaling cache 선택에 반영되는지.
- 설정 적용 후 현재 로드된 animation의 autoplay와 normalized frame delay가
  timer interval에 즉시 반영되는지.
- 설정 적용 후 memory policy가 preview-backed decode 및 full-resolution decode
  요청 판단에 반영되는지.
- 설정 적용 후 navigation wrap/auto-skip/attempt limit이 기존 folder flow에
  반영되는지.
- export default format policy, JPEG quality, suffix, JPEG alpha background가 app
  export defaults/options와 실제 JPEG 저장 결과에 반영되는지.
- status bar hide/simple/detailed 설정이 `image_info_text()` 렌더링 입력에
  반영되는지.
- pan 시작, 이동, 종료 상태 전환.
- zoom 및 새 decode 시작 시 active pan state 해제.
- resize/rotation/image replacement 시 cache invalidation과 offset clamp.
- app state에서 resize 후 manual zoom offset이 새 viewport를 덮도록 clamp되는지.
- Win32 `HDROP` path list에서 지원 이미지 확장자의 첫 항목을 선택하는지.
- Win32 decode worker controller가 같은 generation의 실행 중인 full-resolution worker만 중복 방지하고, unjoined initial worker는 full-resolution 시작을 막지 않는지.

## 수동 재현 절차

다음 항목은 Windows shell이 소유한 실제 입력 또는 Explorer drag source가 필요하므로 수동 절차로 확인한다.

1. `target/debug/j3pic.exe`를 실행한다.
2. `Ctrl+O`로 `png`, `jpg`, `gif`, `ico`, `tiff`, `tga` 중 하나를 연다.
3. `+`, `-`, `1`, `0`, `R`, `Shift+R`, `Right`, `Left`, `F11`, `Esc`, `Q`가 문서의 command 표와 일치하는지 확인한다.
4. 큰 이미지를 연 뒤 `1`을 눌러 actual size로 바꾸고, 좌클릭 드래그 중 이미지가 이동하며 버튼을 놓으면 drag가 끝나는지 확인한다.
5. 마우스 휠 위/아래로 확대/축소되고 status text의 zoom percent가 바뀌는지 확인한다.
6. Explorer에서 지원 이미지 파일을 창 위로 drag-and-drop하고 이미지가 로드되는지 확인한다.
7. fullscreen 진입 후 이미지를 열거나 drop해도 창이 유지되고, `Esc`로 원래 위치와 크기로 복원되는지 확인한다.

설정창 통합 확인은 실제 modal dialog 조작이 필요하므로 다음 절차로 확인한다.

1. `target/debug/j3pic.exe`를 실행하고 서로 다른 크기의 PNG/JPEG 2개, animated GIF
   1개, alpha가 있는 PNG 1개, 중간에 손상 파일이 있는 이미지 폴더를 준비한다.
2. 첫 이미지를 연 뒤 context menu `설정...`에서 기본 보기 모드를 `실제 크기`로
   바꾸고 확인한다. 다음 이미지를 열었을 때 100% actual-size 상태로 시작하는지
   본다. 다시 `창에 맞춤`으로 바꾼 뒤 다음 로드가 fit 상태인지 본다.
3. 스케일링 품질을 `가장 가까운 픽셀`, `균형`, `고품질`로 바꾸며 축소 표시에서
   nearest는 픽셀 경계가 거칠고 smooth 품질은 부드럽게 표시되는지 본다.
4. 최소 줌 `0.5`, 최대 줌 `2.0`, 단계 `2.0`을 적용한다. `+`/`-`와 마우스 휠이
   모두 50%에서 200% 사이로만 움직이고 한 단계가 2배/0.5배인지 본다.
5. animated GIF를 연 상태에서 자동재생을 끄면 타이머가 멈추고, 켜면 다시 진행하는지
   본다. frame delay min/default/max를 크게 바꾸면 진행 속도가 그 값에 맞게 변하는지 본다.
6. 대용량 이미지/메모리 값을 낮춰 preview decode가 선택되는 조건을 만들고, actual
   size 진입 시 full-resolution 요청 threshold를 낮추거나 높였을 때 재디코드 여부가
   달라지는지 본다.
7. 순환 이동을 끄면 폴더 끝에서 next/previous가 no-op인지 본다. 실패 파일 자동
   스킵과 시도 횟수를 켜면 손상 파일을 건너뛰고 다음 이미지로 이동하는지 본다.
8. 내보내기 기본 포맷을 JPEG, 품질을 100, suffix를 `_check`, JPEG 투명 배경 RGB를
   `12,34,56`으로 설정한다. alpha PNG의 export dialog 제안 이름/필터와 저장된 JPEG의
   배경색이 설정과 일치하는지 본다.
9. 상태바 표시를 끄면 하단 상태줄이 사라지는지, 다시 켜고 자세한 상태 텍스트를
   끄면 파일명과 줌만 표시되는지 본다.

현재 비대화형 Codex 세션에서는 foreground window를 안정적으로 소유할 수 없어 위
modal settings dialog 절차를 실제 마우스/키보드 입력으로 완료하지 않았다. 관찰된
자동 결과는 단위 테스트와 foreground가 필요 없는 Win32 message/PInvoke smoke에
한정한다.

주의: 다른 프로세스에서 임의로 `GlobalAlloc`한 값을 `WM_DROPFILES`의 `HDROP`으로 보내는 방식은 유효한 Explorer drop 재현이 아니다. `HDROP`은 수신 프로세스에서 `DragQueryFileW`가 해석할 수 있는 shell drop handle이어야 하므로, 이 방식은 자동화 근거로 사용하지 않는다.

## 2026-05-09 Native size/move 렌더 정착 지연

추가 보고 증상: 모니터 사이로 창을 이동할 때 위치 복귀 문제는 없더라도 이동이
순간적으로 끊긴다.

재분석한 원인:

- 기존 수정은 native size/move loop 중 `WM_DPICHANGED`의 suggested rectangle 적용과
  DPI-triggered viewport refresh를 미뤘지만, 렌더 정착 타이머 경로는 별도로 남아
  있었다.
- 첫 paint나 resize가 `ViewerApp`의 deferred scaling cache rebuild 상태를 만들면
  `INTERACTIVE_RENDER_SETTLE_TIMER_ID`가 설정된다. 이 타이머는 Win32의 modal
  size/move loop 중에도 배달될 수 있으므로, 큰 이미지에서는 UI 스레드가 이동 중에
  software scaling cache와 paint DIB cache를 재생성할 수 있었다.
- 같은 경로의 invalidation은 `UpdateWindow`로 동기 `WM_PAINT`를 강제했다. mixed-DPI
  모니터 경계처럼 size/move loop가 바쁜 순간에는 이 동기 paint가 드래그 지연으로
  보일 수 있다.

수정 후 정책:

- native size/move loop 진입 시 이미 예약된 render-settle timer를 취소한다.
- loop 중 render-settle이 필요해지면 타이머를 다시 걸지 않고, 종료 시 실행할 pending
  상태만 기록한다.
- loop 중 interactive/image-content invalidation은 `InvalidateRect`만 호출하고
  `UpdateWindow`로 즉시 flush하지 않는다.
- loop 종료 후 DPI refresh가 필요하면 기존처럼 viewport/UI metrics를 한 번 갱신하고,
  render-settle pending이 있으면 loop 밖에서 타이머 또는 fallback 경로로 처리한다.

현재 Codex 세션의 모니터는 4개 모두 96 DPI였다.

```text
\\.\DISPLAY1 primary rect=(0,0)-(1280,800) dpi=96x96
\\.\DISPLAY2 rect=(714,1200)-(1914,3120) dpi=96x96
\\.\DISPLAY3 rect=(1914,1398)-(3962,2678) dpi=96x96
\\.\DISPLAY4 rect=(1920,-762)-(4114,472) dpi=96x96
```

비대화형 세션에서 `SendInput` 기반 실제 drag smoke는 foreground/input 소유권 문제로
창 위치가 바뀌지 않아 behavioral 근거로 사용하지 않았다. 대신 native size/move 상태
전이를 단위 테스트로 고정하고 전체 회귀 테스트를 수행했다.

```text
cargo fmt --check
OK

cargo check
OK

cargo test size_move_dpi_state -- --nocapture
3 passed

cargo test
303 passed; 0 failed; 2 ignored
```

## 2026-05-11 DPI awareness fallback 순서 정리

`windows-dpi-awareness-prompt.md`와 native size/move 가이드를 함께 재검토했다.
j3Pic은 이미지 뷰어라 모니터별 DPI 반영이 필요하므로 system-DPI aware로 낮추지 않고
per-monitor 정책을 유지한다. 다만 mixed-DPI 이동 중 Per-Monitor V2의 non-client
자동 재계산이 끊김을 키울 수 있으므로 프로세스 DPI awareness 요청 순서를
Per-Monitor v1 우선으로 바꿨다.

수정 후 순서:

- `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE)`
- `SetProcessDpiAwarenessContext(PER_MONITOR_AWARE_V2)`
- `SetProcessDpiAwarenessContext(SYSTEM_AWARE)`
- `SetProcessDpiAwareness(PROCESS_PER_MONITOR_DPI_AWARE)`
- `SetProcessDpiAwareness(PROCESS_SYSTEM_DPI_AWARE)`
- `SetProcessDPIAware()`

`SetProcessDpiAwarenessContext`와 `SetProcessDpiAwareness`는 실패 시 다음 fallback을
시도한다. `shcore` fallback은 `HRESULT` 성공 여부를 확인해 실패한 per-monitor 요청에서
멈추지 않게 했다. 한국어 UI의 기본 Win32 font 생성은 `Malgun Gothic`을 사용한다.

검증:

```text
cargo fmt --check
OK

cargo check
OK

cargo test process_dpi_awareness -- --nocapture
1 passed

cargo test
307 passed; 0 failed; 2 ignored
```

현재 세션에서는 mixed-DPI 물리 모니터 이동을 다시 수행하지 못했다. 자동 검증은 fallback
순서 단위 테스트, 기존 native size/move 상태 테스트, 전체 Rust 테스트에 한정한다.
