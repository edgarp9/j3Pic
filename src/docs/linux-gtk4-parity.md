# Linux GTK4 기능 동등성 기록

검증일: 2026-06-16

## 범위

Windows Win32 백엔드를 기준 동작으로 두고 Linux 전용 GTK4 백엔드를 추가했다. 두
백엔드는 같은 `ViewerApp` app 계층과 `domain`/`infra` 규칙을 사용하고, 플랫폼별
코드는 창, 입력, 다이얼로그, 렌더링 surface, 클립보드, drag-and-drop, 타이머, 종료
저장만 담당한다.

## Windows 기준 기능과 Linux 구현 상태

| Windows 기준 동작 | Linux GTK4 구현 상태 |
| --- | --- |
| 네이티브 메인 창 생성, 제목 동기화, 하단 상태바 표시/숨김 | 구현. `gtk::ApplicationWindow`, `DrawingArea`, `Label`로 구성하고 app title/status text를 공유 규칙으로 갱신한다. 첫 실행 기본 크기는 Win32와 같은 `960x640`이고, status bar는 Win32의 28 logical px, 좌우 10px padding, 단일 행 끝 말줄임 정책을 따른다. |
| 앱/창 아이콘 | 구현. Windows는 `icon.ico` 리소스를 유지하고, Linux는 같은 `icon.ico`를 GTK texture로 디코드해 GDK toplevel icon list에 전달한다. |
| 첫 명령줄 인자를 시작 이미지로 열기 | 구현. Linux `run_with_args`도 Windows와 같은 startup parsing 후 GTK idle에서 동일 image-open flow를 시작한다. GTK 런타임에는 앱 이름만 전달해 `GApplication`이 이미지 경로를 별도 file-open 인자로 재해석하지 않게 한다. GTK application은 `NON_UNIQUE`로 실행해 Windows처럼 이미지 인자를 가진 새 실행이 기존 Linux 인스턴스에 흡수되어 무시되지 않게 한다. |
| `Ctrl+O` 이미지 열기, 지원 포맷 필터 | 구현. GTK4 `FileDialog`와 `FileFilter`를 사용하며 지원 확장자와 필터 표시명 `Supported Images (*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga)`를 Windows와 동일하게 맞춘다. |
| 이미지 drag-and-drop, 지원 포맷 첫 항목 선택 | 구현. root GTK widget에 drop target을 설치한다. `gdk::FileList`, `gio::File`, string 값은 direct `DropTarget`으로 받고, `text/uri-list`, `x-special/gnome-copied-files`, GNOME/MATE icon-list, KDE URI-list, plain text 절대 경로 payload는 async MIME stream으로 읽어 공유 `first_supported_image_path` 규칙으로 처리한다. 지원 이미지가 없거나 drop 값을 읽지 못하면 Win32 `HDROP` 경로처럼 사용자 메시지를 표시한다. |
| 키보드 명령: 열기, 내보내기, 클립보드, 이전/다음, Space contextual, 애니메이션 재생/프레임, zoom, actual/fit, 회전, fullscreen, quit | 구현. GTK key event를 공통 `KeyInput`/`Command` 매핑에 연결한다. |
| 우클릭 컨텍스트 메뉴와 이미지 필요 항목 비활성화 | 구현. Windows 메뉴와 같은 항목 순서로 GTK popover 메뉴를 만들고, 항목 선택 즉시 popover를 닫은 뒤 command를 실행한다. |
| 마우스 휠 zoom 또는 폴더 이동, 설정 기반 modifier | 구현. 공통 `InteractionSettings`를 읽어 zoom/navigation을 구분한다. zoom anchor는 GTK scroll 이벤트 좌표를 우선 사용하고, discrete wheel step 크기는 Win32 `WM_MOUSEWHEEL` delta처럼 zoom factor에 반영한다. |
| 좌클릭 이미지 pan, 창 이동 gesture, 설정 기반 modifier | 구현. pan은 GTK drag gesture의 누적 offset을 `ViewerApp` pan lifecycle에 전달하고, 창 이동은 GTK/GDK toplevel move 요청으로 위임한다. |
| 창 resize, paint, 첫 렌더 지연 캐시 정책 | 구현. GTK draw handler에서 `render_rgba8_for_paint`를 호출해 Win32 첫-paint 정책과 같은 캐시 지연 규칙을 쓴다. |
| fullscreen 진입/해제, `Esc` 처리 | 구현. GTK fullscreen/unfullscreen과 공통 command를 연결한다. 전환 시 active pan을 끝내고, fullscreen 중 종료하면 진입 직전 windowed size를 저장한다. |
| 초기 decode, full-resolution decode, stale result 무시, folder scan, navigation preload | 구현. Win32 worker controller와 같은 generation/file-version 모델을 GTK poll timer로 연결한다. in-flight decode worker 한도는 Win32와 같은 3, navigation preload worker 한도는 같은 2로 맞춘다. |
| animated GIF/WebP timer, frame decode, cache/prefetch | 구현. GTK timeout을 사용하고 animation frame decode는 공통 infra/cache 경로를 쓴다. |
| 내보내기 옵션 dialog, save dialog, background export worker | 구현. GTK dialog로 Win32와 같은 `PNG, JPEG, BMP, WebP, ICO` format 순서, file/size 그룹, JPEG 품질, metadata 제거, 회전, 원본 크기 reset, aspect 유지, ICO 크기/비활성화, export pixel-count 상한을 제공한다. 저장은 공통 export worker로 수행한다. 기본 export 포맷 정책, 저장 필터 표시명(`PNG image (*.png)` 등), 원본 파일과 같은 기존 경로 저장 차단, 기존 파일 overwrite 확인, 확장자 보정 후 기존 파일 overwrite 확인도 Win32 기준을 따른다. overwrite 확인은 Win32 `MB_YESNO`처럼 `예`가 첫 버튼/기본 응답이다. GTK file/save dialog가 취소가 아닌 오류로 실패하면 Win32처럼 사용자 메시지를 표시하고 내부 오류를 stderr에 남긴다. export 진행 중 재시도 메시지도 Win32와 같은 busy 안내를 사용한다. |
| 현재 display image 클립보드 복사 | 구현. Linux는 현재 display pixels를 Win32 DIB paint/clipboard boundary와 같은 흰색 배경 합성 정책으로 불투명 GDK texture에 만든 뒤 clipboard에 등록한다. |
| 설정 dialog draft, 기본값, 취소/닫기 discard, 확인 적용/저장, validation | 구현. GTK dialog가 Win32 설정창과 같은 고정 그룹 레이아웃(`일반`, `줌`, `애니메이션`, `탐색`, `대용량 이미지/메모리`, `내보내기`, `단축키`)과 label 순서를 사용하고, 공통 `AppConfig` validation 범위를 따른다. validation 실패는 Win32처럼 warning dialog로 표시하며, 정수 필드는 앞뒤 공백을 trim한 뒤 파싱한다. |
| 설정 저장/로드 | 구현. Windows와 Linux 모두 실행파일과 같은 디렉터리에 실행파일 stem 기반 `.toml` 파일을 사용한다. 예를 들어 `j3pic.exe` 또는 `j3pic` 옆의 `j3pic.toml`이다. 실행파일 경로를 확인할 수 없으면 로드는 기본값으로 계속하고 종료 저장은 건너뛴다. Linux config replace는 temp file sync 후 rename하고 parent directory를 fsync해 Windows `MOVEFILE_WRITE_THROUGH` 기준에 맞춘다. |
| 종료 시 worker/timer 정리와 config 저장 | 구현. GTK close request에서 timers/workers를 정리하고 config snapshot을 저장한다. 진행 중인 export worker는 Win32처럼 UI thread를 막지 않고 별도 joiner로 완료를 기다린 뒤 GTK 창 종료를 재개한다. |
| 오류 메시지 표시 | 구현. GTK modal message dialog로 사용자 메시지를 표시하고 내부 원인, category, source chain은 stderr에 남긴다. |
| 큰 이미지 preview/full-resolution/memory policy | 구현. app/domain/infra 공통 경로를 그대로 사용한다. |

## 메뉴 항목별 검증 기록

| 메뉴 | 기능 | Windows 동작 | Linux 기존 동작 | 문제 여부 | 원인 | 수정 내용 | 재검증 결과 |
| --- | --- | --- | --- | --- | --- | --- | --- |
| 우클릭 메뉴 | `열기...` | 이미지 유무와 무관하게 `OpenImage`를 실행하고 native open dialog를 연다. 지원 필터는 `Supported Images (*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga)`로 표시된다. | 같은 `Command::OpenImage`에 연결되어 있었고 suffix 순서는 같았지만 GTK 필터 표시명이 `이미지 파일`로 Windows와 달랐다. | 있음 | Linux open dialog filter label이 Windows 기준 패턴 문자열을 공유하지 않았다. | Windows와 같은 필터 표시명과 suffix 패턴 문자열을 Linux 상수/테스트로 고정했다. | 메뉴 순서/command 매핑은 `context_menu_matches_win32_reference_order`, 필터 suffix/표시명은 `open_filter_suffixes_match_win32_reference_order`로 확인. 실제 native file dialog 조작은 수동 항목. |
| 우클릭 메뉴 | `내보내기...` | 이미지가 있을 때만 활성화하고 export option dialog, save dialog, background worker 흐름을 시작한다. 숫자 입력은 앞뒤 공백을 무시하고, save dialog는 제안 파일명을 표시하며, 기존 선택 경로와 확장자 보정 후 최종 경로를 각각 확인한다. | 메뉴 command는 같았지만 Linux export option UI의 format 순서, ICO/resize/reset, size cap, overwrite, busy 메시지가 Win32와 달랐다. 추가 점검에서 숫자 입력 공백 처리, 비UTF-8 제안 파일명 표시, 선택 경로 overwrite prompt도 Win32와 달랐다. | 있음 | GTK dialog가 Win32 export option dialog의 field state/update 규칙을 일부만 복제했고, save dialog 후속 확인도 최종 경로 위주로만 처리했다. | format 순서 `PNG,JPEG,BMP,WebP,ICO`, reset/aspect/ICO 비활성화, size cap, 숫자 입력 trim, 비UTF-8 파일명 lossy 표시, 선택 경로/보정 경로 overwrite 확인, busy 메시지를 Win32 기준으로 보정했다. | `context_menu_matches_win32_reference_order`, `export_format_indices_match_export_dialog_order`, `export_dialog_size_values_follow_win32_resize_rules`, `export_dialog_size_values_reject_oversized_non_ico_exports`, `numeric_entry_text_parsing_trims_like_win32_dialogs`, `gtk_initial_file_name_keeps_lossy_non_utf8_names`, `platform::tests::export_file_selection_keeps_selected_and_corrected_paths` 통과. |
| 우클릭 메뉴 | `내보내기...` 저장 dialog 세부 동작 | save dialog 필터는 `PNG image (*.png)`, `JPEG image (*.jpg;*.jpeg)`처럼 패턴을 포함하고, `CommDlgExtendedError()!=0`이면 저장 dialog 오류를 표시한다. 확장자 보정 overwrite 확인은 `예/아니요`에서 `예`가 기본이다. | Linux 저장 필터명이 `PNG 파일`처럼 패턴 없이 표시됐고, GTK save dialog 오류를 취소처럼 무시했으며, overwrite 확인은 `아니요/예`에서 `아니요`가 기본이었다. | 있음 | GTK 저장 dialog helper가 Win32 filter label/pattern과 MessageBox button policy를 공유하지 않았고, GTK future의 cancel/error를 구분하지 않았다. | Linux 저장 필터 label/pattern을 Win32와 같은 매핑으로 고정하고, `gtk::DialogError::{Cancelled,Dismissed}` 및 `gio::IOErrorEnum::Cancelled`만 취소로 무시하게 했다. overwrite 버튼 순서와 기본 응답을 Win32 기준으로 변경했다. | `export_filter_labels_match_win32_reference_patterns`, `gtk_file_dialog_cancel_errors_are_not_reported_as_failures`, `export_overwrite_confirmation_defaults_to_win32_yes_button` 통과. |
| 우클릭 메뉴 | `클립보드에 복사` | 이미지가 있을 때만 활성화하고 현재 display image를 흰색 배경에 합성한 불투명 DIB로 복사한다. | 메뉴 command는 같았지만 Linux clipboard texture는 alpha를 그대로 보존할 수 있었다. | 있음 | GDK texture 생성 경계가 Win32 DIB/clipboard boundary의 alpha flatten 정책을 따르지 않았다. | Linux clipboard용 bytes를 흰색 배경 합성 후 alpha 255로 등록하도록 변경했다. | `clipboard_texture_pixels_are_flattened_over_white` 통과. 실제 paste target별 확인은 수동 항목. |
| 우클릭 메뉴 | `실제 크기` | 이미지가 있을 때만 활성화하고 `ActualSize`를 공통 app command로 처리한다. | 같은 command에 연결되어 있었다. | 없음 | 해당 없음 | 기존 구현 유지. | 메뉴 command 매핑 및 공통 `app_command_path_handles_view_commands`, `loaded_image_render_stays_visible_after_zoom_pan_rotate_and_resize` 통과. |
| 우클릭 메뉴 | `창에 맞춤` | 이미지가 있을 때만 활성화하고 `FitToWindow`를 공통 app command로 처리한다. | 같은 command에 연결되어 있었다. | 없음 | 해당 없음 | 기존 구현 유지. | 메뉴 command 매핑 및 공통 view command 테스트 통과. |
| 우클릭 메뉴 | `시계 방향 회전` | 이미지가 있을 때만 활성화하고 user rotation을 시계 방향으로 적용한다. | 같은 command에 연결되어 있었다. | 없음 | 해당 없음 | 기존 구현 유지. | 메뉴 command 매핑 및 rotation/cache 관련 공통 테스트 통과. |
| 우클릭 메뉴 | `반시계 방향 회전` | 이미지가 있을 때만 활성화하고 user rotation을 반시계 방향으로 적용한다. | 같은 command에 연결되어 있었다. | 없음 | 해당 없음 | 기존 구현 유지. | 메뉴 command 매핑 및 rotation/cache 관련 공통 테스트 통과. |
| 우클릭 메뉴 | `전체 화면` | 이미지 유무와 무관하게 fullscreen을 toggle하고 전환 중 active pan을 끝내며 종료 시 이전 windowed size를 저장한다. | command는 같았지만 Linux는 fullscreen 전환의 pan 정리와 fullscreen 종료 저장 size 정책이 Win32와 달랐다. | 있음 | GTK fullscreen/unfullscreen 호출만 있고 Win32의 restore/windowed bounds 정책을 별도로 보존하지 않았다. | 전환 전 active pan cancel, 진입 직전 windowed bounds 저장, fullscreen 종료 저장 시 windowed size 사용으로 보정했다. | `fullscreen_save_uses_windowed_size_and_preserves_existing_position` 통과. 실제 compositor/window-manager 동작은 수동 항목. |
| 우클릭 메뉴 | `설정...` | 이미지 유무와 무관하게 `j3Pic 설정` modal dialog를 열고 확인 시 config를 적용/저장한다. | dialog는 열렸지만 Linux UI가 Win32 고정 그룹 레이아웃/label 순서와 달랐다. | 있음 | GTK 설정창이 Notebook 기반으로 구현되어 Win32 그룹 구조를 따르지 않았다. | Win32와 같은 그룹 제목, label, combo item 순서를 쓰는 고정 grid layout으로 변경했다. | `settings_dialog_labels_match_win32_group_layout`, `startup_config_skips_destroy_save_without_config_directory` 통과. 실제 modal field 조작은 수동 항목. |
| 우클릭 메뉴 | `설정...` validation | 잘못된 숫자나 범위 오류는 warning message box로 표시하고 dialog를 열린 채 유지한다. Windows 정수 파서는 앞뒤 공백을 무시한다. | Linux validation 메시지는 error icon으로 표시됐고, 일부 `usize` 필드는 ` 2 ` 같은 공백 포함 입력을 거부할 수 있었다. | 있음 | validation 경고와 실행 오류가 같은 GTK error helper를 사용했고, `usize` parser만 trim을 빠뜨렸다. | validation helper를 warning으로 분리하고 `usize` text parser도 trim하도록 보정했다. | `numeric_entry_text_parsing_trims_like_win32_dialogs` 통과. 실제 modal icon 표시는 수동 항목. |
| 공통 메뉴 동작 | 이미지 필요 항목 비활성화, 메뉴 해제, 키보드 호출 | export/clipboard/view/rotation은 이미지 없을 때 disabled, open/fullscreen/settings는 활성화한다. Win32 `WM_CONTEXTMENU`는 마우스 우클릭과 키보드 context menu 호출 모두 처리하고, 키보드 호출은 client 중앙에 메뉴를 띄운다. | 같은 `requires_image` 개념은 있었고 메뉴 항목 클릭 후 popover가 닫히지 않을 수 있었다. GTK는 버튼 3 click만 처리해 `Menu` 키/`Shift+F10` 호출이 빠져 있었고, popover가 닫힌 뒤 parent/reference 정리 경로도 명확하지 않았다. | 있음 | GTK popover button handler가 command만 실행하고 popdown을 호출하지 않았다. 또한 Win32 `WM_CONTEXTMENU`의 keyboard invocation에 대응하는 GTK key handling이 없었다. | 항목 선택 즉시 popover를 닫은 뒤 command 실행으로 변경했다. `Menu` 키와 `Shift+F10`은 drawing area 중앙에 같은 context menu를 띄우며, popover close 시 parent를 끊고 button closure는 weak popover 참조만 보관하도록 변경했다. | `context_menu_matches_win32_reference_order`, `gtk_context_menu_keys_match_windows_keyboard_invocation`으로 requires-image flag와 keyboard invocation 판정을 확인. 실제 GTK popover 닫힘/키보드 선택 조작은 수동 항목. |
| 공통 메뉴/파일 동작 | open/save native dialog 오류 | 사용자가 취소한 경우만 조용히 반환하고, native dialog 생성/실행 실패는 사용자 메시지를 표시한다. | Linux는 GTK open/save future의 모든 `Err`를 취소처럼 무시했다. | 있음 | GTK cancel/dismiss와 backend/portal failure가 같은 `Err` 경로로 합쳐져 있었다. | cancel/dismiss만 무시하고 그 외 오류는 Win32와 같은 “파일 열기/저장 대화상자를 열 수 없습니다.” 메시지로 표시한다. | `gtk_file_dialog_cancel_errors_are_not_reported_as_failures` 통과. 실제 portal failure 재현은 수동 항목. |
| 시작/공통 이벤트 | 시작 이미지 로드와 단축키/메뉴 이벤트 | Windows는 창 상태가 Win32 window data에 보관되어 startup open, 키/메뉴/close handler가 창 생애주기 동안 같은 `ViewerApp`에 접근한다. | GTK 창은 표시됐지만 `GtkViewer`가 `activate` 반환 뒤 drop되어 weak handler가 upgrade되지 않았다. 시작 이미지가 열리지 않고 이후 이벤트가 실제 app 상태에 도달하지 않았다. | 있음 | `ApplicationWindow`는 GTK가 소유하지만 Rust `Rc<GtkViewer>`를 강하게 보관하는 수명 owner가 없었다. 모든 signal/idle closure는 weak reference만 들고 있었다. | GTK application 실행 동안 active viewer slot에 `Rc<GtkViewer>`를 보관하고 shutdown 때 해제하도록 변경했다. | 실제 GUI 스모크에서 `cargo run --bin j3pic -- smoke.png` 시작 제목 `smoke.png - j3Pic`, F11 fullscreen, Escape 복귀, `q` 종료를 확인했다. |
| 시작/공통 이벤트 | 이미지 인자를 가진 반복 실행 | Windows는 실행마다 새 창과 startup image open을 수행한다. | Linux `GApplication` 기본 단일 인스턴스 모델에서는 기존 인스턴스가 있으면 새 실행의 image 인자가 전달되지 않고 무시될 수 있었다. | 있음 | GTK application을 unique 기본값으로 실행하면서 실제 `run_with_args`에는 앱 이름만 넘겨 image path가 remote activate로 전달될 경로가 없었다. | GTK application flags를 `NON_UNIQUE`로 바꿔 Windows처럼 각 실행이 독립 창과 startup image path를 소유하게 했다. | `gtk_application_is_non_unique_like_win32_launches` 통과. 실제 두 프로세스 동시 실행 스모크는 수동 항목. |
| Drag-and-drop | 파일 매니저 drop payload 협상 | 지원 이미지가 없는 drop은 오류 메시지를 표시하고, 지원 이미지가 있으면 첫 지원 경로를 연다. | 일부 Linux 파일 매니저 drop이 열리지 않았다. | 있음 | 기존 구현은 drawing area에 여러 drop controller를 따로 붙이고 `text/uri-list`를 `String` 값으로만 읽었다. 실제 파일 매니저는 root child hierarchy의 다른 영역으로 drop하거나 MIME 스트림(`text/uri-list`, icon-list, KDE URI-list) 또는 GDK file-list를 광고할 수 있어 controller 선택과 값 역직렬화가 어긋날 수 있었다. | root widget에 direct `FileList`/`gio::File`/string `DropTarget`과 async text MIME `DropTargetAsync`를 설치한다. GDK/GIO 파일 값과 MIME stream은 같은 path-selection 규칙으로 정규화한다. | `uri_list_drop_uses_first_supported_image_path`, `gnome_copied_files_drop_text_uses_first_supported_file_uri`, `plain_text_drop_uses_absolute_supported_path`, `icon_list_drop_text_accepts_uri_or_path_token`, `gdk_file_list_drop_uses_first_supported_image_path`, `gio_file_drop_uses_supported_image_path` 통과. 실제 파일 관리자별 drag source는 수동 항목. |
| 설정 저장 | 종료/설정 확인 후 config replace | Windows config replace는 `MOVEFILE_WRITE_THROUGH`로 디렉터리 엔트리 내구성을 높인다. | Linux는 temp file sync 후 rename만 수행했다. | 있음 | non-Windows replace 경로가 단순 rename으로 끝나 parent directory fsync가 없었다. | Linux config replace 후 parent directory를 fsync한다. export replace는 기존 atomic rename 정책을 유지한다. | `config_replace_renames_file_and_syncs_parent_directory` 통과. |
| 종료/공통 이벤트 | `Q`/`Alt+F4` 종료와 설정 저장 | Windows는 quit command가 close path로 들어가 worker/timer 정리와 window bounds 저장 후 종료한다. | GTK viewer 수명 보정 후 `q` 종료가 close request로 재진입할 때 `RefCell already borrowed` panic이 발생했다. | 있음 | `save_window_bounds`의 `if let` 조건에서 잡은 `self.app.borrow()` temporary가 본문까지 살아 있는 상태로 `borrow_mut()`를 호출했다. | 저장할 bounds를 먼저 값으로 계산해 immutable borrow를 끝낸 뒤 mutable borrow로 config에 반영하도록 분리했다. | 실제 GUI 스모크에서 `q` 종료 후 실행파일 옆 `j3pic.toml` 저장을 확인했다. |

## 남은 플랫폼 차이

- Linux GTK4/Wayland에서는 일반 앱이 창의 절대 화면 좌표를 안정적으로 읽거나 복원할 수 없다. 현재 Linux는 window width/height를 저장하고, x/y는 기존 설정값을 유지하거나 없으면 `0,0`을 쓴다. Windows는 기존 Win32 좌표 저장/복원을 유지한다.
- Linux clipboard는 GTK/GDK clipboard ownership API를 사용하므로 Win32 `OpenClipboard`/`SetClipboardData`처럼 set 호출의 즉시 실패를 모두 관찰하지 못한다. 현재는 clipboard texture 생성 전 오류만 사용자 메시지로 표시하고, backend별 paste target 검증은 수동 항목으로 둔다.
- Linux pan gesture는 GTK event controller 흐름을 사용한다. Win32 `SetCapture`와 완전히 같은 pointer capture semantics인지, 포인터가 drawing area 밖으로 나간 상태의 compositor별 동작은 수동 확인이 필요하다.
- Linux desktop launcher/task switcher 아이콘은 compositor와 desktop 파일/아이콘 테마 설치 상태의 영향을 받는다. 코드에서는 같은 `icon.ico`를 GDK toplevel icon list로 제공하지만, 배포 패키지용 `.desktop` 파일과 테마 아이콘 설치는 아직 추가하지 않았다.
- Linux GUI 조작은 현재 세션에서 전체 수동 비교를 완료하지 않았다. 네이티브 GTK 창 생성, 시작 이미지 로드, fullscreen/windowed 전환, `q` 종료와 config 저장 스모크는 수행했고, 상세 native file dialog, drag-and-drop source, clipboard paste target은 문서의 수동 점검 항목으로 남긴다.
- Windows 교차 `cargo check --target x86_64-pc-windows-gnu`는 로컬에 MinGW/Windows C toolchain과 Windows sysroot가 없어 `libdeflate-sys`/Windows resource 단계에서 중단된다. 코드 변경 전제상 Windows 전용 소스는 조건부 의존성으로 유지되지만, 이 Linux 환경에서는 전체 Windows 타깃 컴파일을 끝까지 검증하지 못했다.

## 검증 결과

```text
cargo fmt --check
OK

cargo check
OK

cargo test
OK. lib tests: 308 passed, 1 ignored. profile_open tests: 2 passed. main/doc tests: 0 tests.

cargo test platform::linux::tests
OK. 31 passed. 메뉴 순서, GTK key/modifier에서 Windows shortcut command로 이어지는 통합 매핑, keyboard context menu invocation, application non-unique 실행 정책, file dialog cancel/error 분리, decode/folder scan/preload worker 한도, 마우스 shortcut, GTK scroll delta에서 Windows wheel step 의미로 이어지는 zoom/navigation 계산, open/export filter suffix/label 순서, export format/resize/size cap/live aspect/숫자 trim/기본 파일명/overwrite 기본 응답, settings combo index 순서, fullscreen 저장 크기, 설정 label, clipboard alpha 합성, URI drop, 아이콘, export shutdown 회귀를 포함한다.

cargo test config_replace_renames_file_and_syncs_parent_directory
OK. 1 passed. Linux config replace 후 parent directory fsync 경로를 확인했다.

cargo test platform::tests
3 passed; 0 failed

timeout 3s cargo run --quiet --bin j3pic --
GTK 창 시작 후 timeout 종료. GTK 설정/AT-SPI 경고만 출력되고 앱 시작 오류는 없었다.

GDK_BACKEND=x11 GSETTINGS_BACKEND=memory cargo run --quiet --bin j3pic -- <temp>/smoke.png
OK. ImageMagick `convert`로 만든 64x48 PNG 기준으로 GTK 주 창 `960x640`, title `smoke.png - j3Pic`, F11 후 `_NET_WM_STATE_FULLSCREEN`, Escape 후 fullscreen state 제거, `q` 종료, 실행파일 옆 config 저장을 확인했다. stderr에는 현재 세션의 AT-SPI bus 경고만 출력됐다.

GDK_BACKEND=x11 GSETTINGS_BACKEND=memory cargo run --quiet --bin j3pic -- <temp>/smoke.png + xdotool
OK. `Menu`, `Escape`, `Shift+F10`, `Escape` 후 포커스가 viewer로 돌아오고, `R`/`Shift+R` 제목 변경, F11/Escape fullscreen 왕복, `q` 종료와 실행파일 옆 config 저장을 확인했다. stderr에는 현재 세션의 AT-SPI bus 경고만 출력됐다.

GDK_BACKEND=x11 GSETTINGS_BACKEND=memory <temp1>/j3pic -- <temp>/first.png
GDK_BACKEND=x11 GSETTINGS_BACKEND=memory <temp2>/j3pic -- <temp>/second.png
OK. `NON_UNIQUE` 실행 정책으로 `first.png - j3Pic`, `second.png - j3Pic` 두 창이 동시에 뜨고, 각 임시 실행파일 디렉터리에 별도 config 저장을 확인했다. stderr에는 현재 세션의 AT-SPI bus 경고만 출력됐다.

cargo clippy --all-targets --all-features -- -D warnings
실패. 이번 Linux GTK4 변경에서 새로 생긴 lint는 정리했지만, 기존 app/domain/infra 전역 lint 부채(`too_many_arguments`, `question_mark`, `large_enum_variant`, `unnecessary_map_or`, `io_other_error`)가 남아 있음.

WINDRES=llvm-windres-19 AR_x86_64_pc_windows_gnu=llvm-ar-19 CC_x86_64_pc_windows_gnu='clang-19 --target=x86_64-w64-windows-gnu' cargo check --target x86_64-pc-windows-gnu
중단: Windows sysroot/header가 없어 libdeflate-sys가 `string.h`를 찾지 못하고, Windows resource/archive toolchain도 완전하지 않음.

cargo test startup
7 passed; 0 failed

cargo check
OK
```

## Linux 수동 점검 항목

1. `cargo run --bin j3pic -- [image-path]`로 시작 이미지가 열리고 title/status가 파일 정보와 일치하는지 확인한다.
2. `Ctrl+O`로 `jpg`, `jpeg`, `png`, `bmp`, `gif`, `webp`, `ico`, `tif`, `tiff`, `tga` 중 여러 포맷을 열어 오류 처리와 최근 폴더 초기값을 확인한다.
3. 우클릭 메뉴 항목 순서와 이미지가 없을 때 이미지 필요 명령 비활성화를 확인한다.
4. `+`, `-`, `1`, `0`, `R`, `Shift+R`, `Right`, `Left`, `PageUp`, `PageDown`, `Backspace`, `Space`, `P`, `[`, `]`, `Home`, `F11`, `Esc`, `Q`, `Alt+Enter`, `Alt+F4`를 Windows 기준 command 표와 비교한다.
5. 설정에서 zoom/navigation/pan/window-move mouse shortcut을 바꾼 뒤 휠과 좌클릭 drag가 새 설정을 따르는지 확인한다.
6. 큰 이미지를 열고 첫 표시, actual-size 전환, full-resolution 반영, pan/zoom 중 캐시 rebuild 지연이 표시를 깨지 않는지 확인한다.
7. animated GIF/WebP를 열고 autoplay, `P`, `[`, `]`, `Home`, frame delay 설정 반영을 확인한다.
8. 내보내기 옵션 dialog에서 format/quality/remove metadata/rotation/resize/aspect/ICO 비활성화가 Windows와 같은 규칙인지 확인한다.
9. alpha PNG를 JPEG로 내보내고 JPEG alpha background RGB 설정이 반영되는지 확인한다.
10. 설정 dialog에서 `기본값`, `취소`, 창 닫기, 잘못된 숫자/RGB/suffix validation, `확인` 후 즉시 저장과 재시작 복원을 확인한다.
11. 파일 manager에서 여러 파일을 drag-and-drop해 지원 포맷 첫 항목이 열리고, 모두 미지원이면 오류 메시지가 표시되는지 확인한다.
12. fullscreen 상태에서 open/drop/navigation/settings/export가 동작하고 `Esc`로 windowed 상태로 돌아오는지 확인한다.
