use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, ICC_LINK_CLASS, INITCOMMONCONTROLSEX, NMHDR, NM_CLICK, NM_RETURN,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DialogBoxIndirectParamW, EndDialog, GetClientRect, GetDlgItem, GetPropW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, MessageBoxW, RemovePropW,
    SendDlgItemMessageW, SendMessageW, SetDlgItemTextW, SetPropW, SetWindowPos, BM_GETCHECK,
    BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_GROUPBOX, BS_PUSHBUTTON, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_SETCURSEL, DLGTEMPLATE, DS_CENTER, DS_MODALFRAME,
    ES_AUTOHSCROLL, HMENU, IDCANCEL, IDOK, MB_ICONWARNING, MB_OK, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, SW_SHOWNORMAL, WM_CLOSE, WM_COMMAND, WM_INITDIALOG,
    WM_NCDESTROY, WM_NOTIFY, WM_SETFONT, WS_BORDER, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU,
    WS_TABSTOP, WS_VISIBLE,
};

use crate::domain::{
    validate_export_filename_suffix, AnimationTimingSettings, AppConfig, DefaultExportFormatPolicy,
    ExportFilenameSuffixValidationError, ExportSettings, InteractionSettings, MemoryPolicySettings,
    MouseShortcut, NavigationSettings, RgbColor, ScalingQuality, StatusUiSettings, UiLanguage,
    ViewMode, ZoomSettings, DEFAULT_EXPORT_FILENAME_SUFFIX, MAX_CONFIG_ANIMATION_DELAY_MS,
    MAX_CONFIG_CACHE_ENTRIES, MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE, MAX_CONFIG_IMAGE_PIXELS,
    MAX_CONFIG_MAX_ZOOM_SCALE, MAX_CONFIG_MEMORY_MIB, MAX_CONFIG_MIN_ZOOM_SCALE,
    MAX_CONFIG_PREVIEW_OVERSAMPLE, MAX_CONFIG_ZOOM_STEP_FACTOR, MAX_EXPORT_FILENAME_SUFFIX_CHARS,
    MAX_EXPORT_QUALITY, MIN_CONFIG_ANIMATION_DELAY_MS, MIN_CONFIG_CACHE_ENTRIES,
    MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE, MIN_CONFIG_MAX_ZOOM_SCALE, MIN_CONFIG_MEMORY_MIB,
    MIN_CONFIG_MIN_ZOOM_SCALE, MIN_CONFIG_PIXEL_LIMIT, MIN_CONFIG_PREVIEW_OVERSAMPLE,
    MIN_CONFIG_ZOOM_STEP_FACTOR, MIN_EXPORT_QUALITY,
};
use crate::ui_text;

use super::{win32::dpi, PROJECT_LINK_URL};

const STATE_PROP_NAME: &str = "j3Pic.SettingsDialogState";
const DIALOG_RESULT_INIT_FAILED: isize = -2;

const ID_DEFAULTS: i32 = 100;
const ID_DEFAULT_VIEW_MODE: i32 = 101;
const ID_SCALING_QUALITY: i32 = 102;
const ID_EXPORT_QUALITY: i32 = 103;
const ID_EXPORT_FORMAT_POLICY: i32 = 104;
const ID_EXPORT_SUFFIX: i32 = 105;
const ID_MIN_ZOOM_SCALE: i32 = 106;
const ID_MAX_ZOOM_SCALE: i32 = 107;
const ID_ZOOM_STEP_FACTOR: i32 = 108;
const ID_NAVIGATION_ATTEMPTS: i32 = 109;
const ID_ANIMATION_AUTOPLAY: i32 = 110;
const ID_WRAP_NAVIGATION: i32 = 111;
const ID_AUTO_SKIP_NAVIGATION: i32 = 112;
const ID_SHOW_STATUS_BAR: i32 = 113;
const ID_DETAILED_STATUS_TEXT: i32 = 114;
const ID_LARGE_IMAGE_PIXEL_THRESHOLD: i32 = 115;
const ID_MAX_IMAGE_PIXELS: i32 = 116;
const ID_PREVIEW_MAX_PIXELS: i32 = 117;
const ID_PREVIEW_OVERSAMPLE: i32 = 118;
const ID_FULL_RESOLUTION_REQUEST_SCALE: i32 = 119;
const ID_MAX_RESIDENT_MIB: i32 = 120;
const ID_MAX_CACHE_ENTRY_MIB: i32 = 121;
const ID_MAX_CACHE_ENTRIES: i32 = 122;
const ID_DEFAULT_FRAME_DELAY_MS: i32 = 123;
const ID_MIN_FRAME_DELAY_MS: i32 = 124;
const ID_MAX_FRAME_DELAY_MS: i32 = 125;
const ID_JPEG_ALPHA_BACKGROUND_RGB: i32 = 126;
const ID_ZOOM_SHORTCUT: i32 = 127;
const ID_IMAGE_NAVIGATION_SHORTCUT: i32 = 128;
const ID_IMAGE_PAN_SHORTCUT: i32 = 129;
const ID_WINDOW_MOVE_SHORTCUT: i32 = 130;
const ID_PROJECT_LINK: i32 = 131;
const ID_UI_LANGUAGE: i32 = 132;

const BST_UNCHECKED_VALUE: WPARAM = 0;
const BST_CHECKED_VALUE: WPARAM = 1;

const DIALOG_TEMPLATE_WIDTH: i16 = 420;
const DIALOG_TEMPLATE_HEIGHT: i16 = 330;
const DIALOG_CLIENT_WIDTH: i32 = 780;
const DIALOG_CLIENT_HEIGHT: i32 = 610;
const DIALOG_FONT_POINT_SIZE: i32 = 9;
const STATIC_NOTIFY_STYLE: u32 = 0x0000_0100;
const LEFT_GROUP_X: i32 = 16;
const RIGHT_GROUP_X: i32 = 398;
const GENERAL_GROUP_Y: i32 = 14;
const ZOOM_GROUP_Y: i32 = 181;
const ANIMATION_GROUP_Y: i32 = 298;
const NAVIGATION_GROUP_Y: i32 = 440;
const MEMORY_GROUP_Y: i32 = 14;
const EXPORT_GROUP_Y: i32 = 256;
const SHORTCUT_GROUP_Y: i32 = 407;
const GROUP_WIDTH: i32 = 366;
const GROUP_X_PADDING: i32 = 14;
const GROUP_TOP_PADDING: i32 = 24;
const GROUP_ROW_HEIGHT: i32 = 25;
const LABEL_WIDTH: i32 = 148;
const GROUP_INPUT_WIDTH: i32 = 188;
const EDIT_HEIGHT: i32 = 22;
// Win32 uses the combo box height as the expanded drop-down extent.
const COMBO_HEIGHT: i32 = 180;
const TEMPLATE_BUFFER_BYTES: usize = 256;

pub(crate) enum SettingsDialogOutcome {
    Accepted(Box<AppConfig>),
    Cancelled,
}

#[derive(Debug)]
pub(crate) enum SettingsDialogError {
    ModuleHandle { code: u32 },
    CreateDialog { code: u32 },
    InitializeControls,
}

impl fmt::Display for SettingsDialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleHandle { code } => {
                write!(
                    formatter,
                    "failed to get module handle for settings dialog: Win32 error {code}"
                )
            }
            Self::CreateDialog { code } => {
                write!(
                    formatter,
                    "failed to create settings dialog: Win32 error {code}"
                )
            }
            Self::InitializeControls => {
                formatter.write_str("failed to initialize settings controls")
            }
        }
    }
}

impl Error for SettingsDialogError {}

pub(crate) fn show_settings_dialog(
    owner: HWND,
    initial_config: AppConfig,
) -> Result<SettingsDialogOutcome, SettingsDialogError> {
    let instance = module_instance()?;
    let template = DialogTemplateBuffer::new(ui_text::settings_title(initial_config.ui_language()));
    let mut state = SettingsDialogState {
        draft: initial_config,
        accepted_config: None,
        init_failed: false,
        ui_font: None,
    };

    // SAFETY: template is an aligned in-memory DLGTEMPLATE that outlives the modal call.
    // state is a stack value kept alive for the entire DialogBoxIndirectParamW call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            instance,
            template.as_ptr(),
            owner,
            Some(settings_dialog_proc),
            (&mut state as *mut SettingsDialogState) as LPARAM,
        )
    };

    if state.init_failed || result == DIALOG_RESULT_INIT_FAILED {
        return Err(SettingsDialogError::InitializeControls);
    }
    if result == -1 {
        return Err(SettingsDialogError::CreateDialog { code: last_error() });
    }
    if result == IDOK as isize {
        if let Some(config) = state.accepted_config {
            Ok(SettingsDialogOutcome::Accepted(Box::new(config)))
        } else {
            Err(SettingsDialogError::InitializeControls)
        }
    } else {
        Ok(SettingsDialogOutcome::Cancelled)
    }
}

struct SettingsDialogState {
    draft: AppConfig,
    accepted_config: Option<AppConfig>,
    init_failed: bool,
    ui_font: Option<dpi::DpiFont>,
}

unsafe extern "system" fn settings_dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            handle_init_dialog(hwnd, lparam);
            1
        }
        WM_COMMAND if handle_dialog_command(hwnd, low_word(wparam)) => 1,
        WM_COMMAND => 0,
        WM_NOTIFY if handle_dialog_notify(hwnd, lparam) => 1,
        WM_NOTIFY => 0,
        WM_CLOSE => {
            close_dialog(hwnd, IDCANCEL as isize);
            1
        }
        WM_NCDESTROY => {
            remove_dialog_state(hwnd);
            0
        }
        _ => 0,
    }
}

fn handle_init_dialog(hwnd: HWND, lparam: LPARAM) {
    if lparam == 0 {
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    }

    let state_ptr = lparam as *mut SettingsDialogState;
    if !set_dialog_state(hwnd, state_ptr) {
        // SAFETY: state_ptr comes from lparam and is valid during WM_INITDIALOG.
        unsafe {
            (*state_ptr).init_failed = true;
        }
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    }

    initialize_dialog_dpi(hwnd);

    let language = dialog_state_mut(hwnd)
        .map(|state| state.draft.ui_language())
        .unwrap_or_default();
    if !resize_dialog_to_layout(hwnd) || !create_dialog_controls(hwnd, language) {
        if let Some(state) = dialog_state_mut(hwnd) {
            state.init_failed = true;
        }
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    }

    if let Some(state) = dialog_state_mut(hwnd) {
        populate_dialog(hwnd, &state.draft);
    }
}

fn initialize_dialog_dpi(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    let dpi_y = dpi::dpi_y_for_window(hwnd);
    state.ui_font = dpi::DpiFont::new_ui_font(DIALOG_FONT_POINT_SIZE, dpi_y);
}

fn handle_dialog_command(hwnd: HWND, command_id: u16) -> bool {
    match i32::from(command_id) {
        IDOK => {
            handle_ok(hwnd);
            true
        }
        IDCANCEL => {
            close_dialog(hwnd, IDCANCEL as isize);
            true
        }
        ID_DEFAULTS => {
            if let Some(state) = dialog_state_mut(hwnd) {
                state.draft = AppConfig::default();
                populate_dialog(hwnd, &state.draft);
            }
            true
        }
        ID_PROJECT_LINK => {
            open_project_link(hwnd);
            true
        }
        _ => false,
    }
}

fn handle_dialog_notify(hwnd: HWND, lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }

    // SAFETY: WM_NOTIFY provides a pointer to an NMHDR-compatible structure
    // for the duration of the message dispatch.
    let header = unsafe { &*(lparam as *const NMHDR) };
    if header.idFrom == ID_PROJECT_LINK as usize
        && (header.code == NM_CLICK || header.code == NM_RETURN)
    {
        open_project_link(hwnd);
        true
    } else {
        false
    }
}

fn handle_ok(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    };

    match read_dialog_config(hwnd, &state.draft) {
        Ok(config) => {
            state.accepted_config = Some(config);
            close_dialog(hwnd, IDOK as isize);
        }
        Err(message) => show_warning_message(hwnd, &message),
    }
}

fn create_dialog_controls(hwnd: HWND, language: UiLanguage) -> bool {
    let groups = ui_text::settings_group_titles(language);
    if !create_settings_group(hwnd, groups[0], LEFT_GROUP_X, GENERAL_GROUP_Y, 5)
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            GENERAL_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Language",
                UiLanguage::Korean => "언어",
            },
            ID_UI_LANGUAGE,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            GENERAL_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Default View Mode",
                UiLanguage::Korean => "기본 보기 모드",
            },
            ID_DEFAULT_VIEW_MODE,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            GENERAL_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Scaling Quality",
                UiLanguage::Korean => "스케일링 품질",
            },
            ID_SCALING_QUALITY,
            ControlKind::Combo,
        )
        || !create_group_checkbox(
            hwnd,
            LEFT_GROUP_X,
            GENERAL_GROUP_Y,
            3,
            ID_SHOW_STATUS_BAR,
            match language {
                UiLanguage::English => "Show Status Bar",
                UiLanguage::Korean => "상태바 표시",
            },
        )
        || !create_group_checkbox(
            hwnd,
            LEFT_GROUP_X,
            GENERAL_GROUP_Y,
            4,
            ID_DETAILED_STATUS_TEXT,
            match language {
                UiLanguage::English => "Detailed Status Text",
                UiLanguage::Korean => "자세한 상태 텍스트",
            },
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[1], LEFT_GROUP_X, ZOOM_GROUP_Y, 3)
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ZOOM_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Minimum Zoom",
                UiLanguage::Korean => "최소 줌",
            },
            ID_MIN_ZOOM_SCALE,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ZOOM_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Maximum Zoom",
                UiLanguage::Korean => "최대 줌",
            },
            ID_MAX_ZOOM_SCALE,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ZOOM_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Zoom Step Factor",
                UiLanguage::Korean => "줌 단계 배율",
            },
            ID_ZOOM_STEP_FACTOR,
            ControlKind::Edit,
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[2], LEFT_GROUP_X, ANIMATION_GROUP_Y, 4)
        || !create_group_checkbox(
            hwnd,
            LEFT_GROUP_X,
            ANIMATION_GROUP_Y,
            0,
            ID_ANIMATION_AUTOPLAY,
            match language {
                UiLanguage::English => "Autoplay",
                UiLanguage::Korean => "자동재생",
            },
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ANIMATION_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Default Frame Delay (ms)",
                UiLanguage::Korean => "기본 프레임 지연(ms)",
            },
            ID_DEFAULT_FRAME_DELAY_MS,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ANIMATION_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Minimum Frame Delay (ms)",
                UiLanguage::Korean => "최소 프레임 지연(ms)",
            },
            ID_MIN_FRAME_DELAY_MS,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            ANIMATION_GROUP_Y,
            3,
            match language {
                UiLanguage::English => "Maximum Frame Delay (ms)",
                UiLanguage::Korean => "최대 프레임 지연(ms)",
            },
            ID_MAX_FRAME_DELAY_MS,
            ControlKind::Edit,
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[3], LEFT_GROUP_X, NAVIGATION_GROUP_Y, 3)
        || !create_group_checkbox(
            hwnd,
            LEFT_GROUP_X,
            NAVIGATION_GROUP_Y,
            0,
            ID_WRAP_NAVIGATION,
            match language {
                UiLanguage::English => "Wrap Navigation",
                UiLanguage::Korean => "순환 이동",
            },
        )
        || !create_group_checkbox(
            hwnd,
            LEFT_GROUP_X,
            NAVIGATION_GROUP_Y,
            1,
            ID_AUTO_SKIP_NAVIGATION,
            match language {
                UiLanguage::English => "Auto-skip Failed Files",
                UiLanguage::Korean => "실패 파일 자동 스킵",
            },
        )
        || !create_group_field(
            hwnd,
            LEFT_GROUP_X,
            NAVIGATION_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Maximum Attempts",
                UiLanguage::Korean => "최대 시도 횟수",
            },
            ID_NAVIGATION_ATTEMPTS,
            ControlKind::Edit,
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[4], RIGHT_GROUP_X, MEMORY_GROUP_Y, 8)
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Large Pixel Threshold",
                UiLanguage::Korean => "대용량 픽셀 기준",
            },
            ID_LARGE_IMAGE_PIXEL_THRESHOLD,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Maximum Image Pixels",
                UiLanguage::Korean => "최대 이미지 픽셀",
            },
            ID_MAX_IMAGE_PIXELS,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Preview Maximum Pixels",
                UiLanguage::Korean => "프리뷰 최대 픽셀",
            },
            ID_PREVIEW_MAX_PIXELS,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            3,
            match language {
                UiLanguage::English => "Preview Oversample",
                UiLanguage::Korean => "프리뷰 배율",
            },
            ID_PREVIEW_OVERSAMPLE,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            4,
            match language {
                UiLanguage::English => "Full-res Request Scale",
                UiLanguage::Korean => "전체 해상도 요청 배율",
            },
            ID_FULL_RESOLUTION_REQUEST_SCALE,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            5,
            match language {
                UiLanguage::English => "Total Cache (MiB)",
                UiLanguage::Korean => "캐시 총량(MiB)",
            },
            ID_MAX_RESIDENT_MIB,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            6,
            match language {
                UiLanguage::English => "Entry Cache Limit (MiB)",
                UiLanguage::Korean => "캐시 항목 한도(MiB)",
            },
            ID_MAX_CACHE_ENTRY_MIB,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            MEMORY_GROUP_Y,
            7,
            match language {
                UiLanguage::English => "Cache Entries",
                UiLanguage::Korean => "캐시 항목 수",
            },
            ID_MAX_CACHE_ENTRIES,
            ControlKind::Edit,
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[5], RIGHT_GROUP_X, EXPORT_GROUP_Y, 4)
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            EXPORT_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Default Format Policy",
                UiLanguage::Korean => "기본 포맷 정책",
            },
            ID_EXPORT_FORMAT_POLICY,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            EXPORT_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "JPEG Quality",
                UiLanguage::Korean => "JPEG 품질",
            },
            ID_EXPORT_QUALITY,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            EXPORT_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Filename Suffix",
                UiLanguage::Korean => "파일명 suffix",
            },
            ID_EXPORT_SUFFIX,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            EXPORT_GROUP_Y,
            3,
            match language {
                UiLanguage::English => "JPEG Alpha RGB",
                UiLanguage::Korean => "JPEG 투명 배경 RGB",
            },
            ID_JPEG_ALPHA_BACKGROUND_RGB,
            ControlKind::Edit,
        )
    {
        return false;
    }

    if !create_settings_group(hwnd, groups[6], RIGHT_GROUP_X, SHORTCUT_GROUP_Y, 4)
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            SHORTCUT_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Zoom",
                UiLanguage::Korean => "확대/축소",
            },
            ID_ZOOM_SHORTCUT,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            SHORTCUT_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Previous/Next Image",
                UiLanguage::Korean => "이전/다음 이미지",
            },
            ID_IMAGE_NAVIGATION_SHORTCUT,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            SHORTCUT_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Image Pan",
                UiLanguage::Korean => "이미지 이동",
            },
            ID_IMAGE_PAN_SHORTCUT,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            RIGHT_GROUP_X,
            SHORTCUT_GROUP_Y,
            3,
            match language {
                UiLanguage::English => "Window Move",
                UiLanguage::Korean => "창 이동",
            },
            ID_WINDOW_MOVE_SHORTCUT,
            ControlKind::Combo,
        )
    {
        return false;
    }

    let button_y = DIALOG_CLIENT_HEIGHT - 52;
    create_button(
        hwnd,
        ID_DEFAULTS,
        match language {
            UiLanguage::English => "Defaults",
            UiLanguage::Korean => "기본값",
        },
        ControlRect::new(LEFT_GROUP_X, button_y, 86, 28),
        ButtonKind::Normal,
    )
    .is_some()
        && create_project_link(
            hwnd,
            ControlRect::new(LEFT_GROUP_X + 104, button_y + 5, 340, 22),
        )
        .is_some()
        && create_button(
            hwnd,
            IDOK,
            match language {
                UiLanguage::English => "OK",
                UiLanguage::Korean => "확인",
            },
            ControlRect::new(DIALOG_CLIENT_WIDTH - 204, button_y, 86, 28),
            ButtonKind::Default,
        )
        .is_some()
        && create_button(
            hwnd,
            IDCANCEL,
            match language {
                UiLanguage::English => "Cancel",
                UiLanguage::Korean => "취소",
            },
            ControlRect::new(DIALOG_CLIENT_WIDTH - 108, button_y, 86, 28),
            ButtonKind::Normal,
        )
        .is_some()
        && initialize_combo(hwnd, ID_UI_LANGUAGE, ui_text::UI_LANGUAGE_LABELS)
        && initialize_combo(
            hwnd,
            ID_DEFAULT_VIEW_MODE,
            ui_text::settings_view_mode_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_SCALING_QUALITY,
            ui_text::settings_scaling_quality_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_EXPORT_FORMAT_POLICY,
            ui_text::settings_export_policy_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_ZOOM_SHORTCUT,
            ui_text::settings_wheel_shortcut_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_IMAGE_NAVIGATION_SHORTCUT,
            ui_text::settings_wheel_shortcut_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_IMAGE_PAN_SHORTCUT,
            ui_text::settings_drag_shortcut_labels(language),
        )
        && initialize_combo(
            hwnd,
            ID_WINDOW_MOVE_SHORTCUT,
            ui_text::settings_drag_shortcut_labels(language),
        )
}

fn create_settings_group(parent: HWND, title: &str, x: i32, y: i32, row_count: i32) -> bool {
    create_group_box(parent, title, x, y, GROUP_WIDTH, group_height(row_count)).is_some()
}

fn create_group_field(
    parent: HWND,
    group_x: i32,
    group_y: i32,
    row: i32,
    label: &str,
    id: i32,
    kind: ControlKind,
) -> bool {
    let y = group_row_y(group_y, row);
    let label_x = group_x + GROUP_X_PADDING;
    let input_x = label_x + LABEL_WIDTH;
    if create_label(parent, label, label_x, y + 3, LABEL_WIDTH - 4, EDIT_HEIGHT).is_none() {
        return false;
    }

    match kind {
        ControlKind::Combo => create_combo(parent, id, input_x, y, GROUP_INPUT_WIDTH, COMBO_HEIGHT),
        ControlKind::Edit => create_edit(parent, id, input_x, y, GROUP_INPUT_WIDTH, EDIT_HEIGHT),
    }
    .is_some()
}

fn create_group_checkbox(
    parent: HWND,
    group_x: i32,
    group_y: i32,
    row: i32,
    id: i32,
    label: &str,
) -> bool {
    create_checkbox(
        parent,
        id,
        label,
        group_x + GROUP_X_PADDING,
        group_row_y(group_y, row),
        GROUP_WIDTH - GROUP_X_PADDING * 2,
        EDIT_HEIGHT,
    )
    .is_some()
}

fn group_height(row_count: i32) -> i32 {
    GROUP_TOP_PADDING + GROUP_ROW_HEIGHT * row_count + 12
}

fn group_row_y(group_y: i32, row: i32) -> i32 {
    group_y + GROUP_TOP_PADDING + GROUP_ROW_HEIGHT * row
}

fn resize_dialog_to_layout(hwnd: HWND) -> bool {
    let Some((window_rect, client_rect)) = dialog_rects(hwnd) else {
        return false;
    };

    let dpi_y = dpi::dpi_y_for_window(hwnd);
    let non_client_width = rect_width(&window_rect).saturating_sub(rect_width(&client_rect));
    let non_client_height = rect_height(&window_rect).saturating_sub(rect_height(&client_rect));
    let client_width = dpi::scale_i32_for_dpi(DIALOG_CLIENT_WIDTH, dpi_y);
    let client_height = dpi::scale_i32_for_dpi(DIALOG_CLIENT_HEIGHT, dpi_y);
    let window_width = client_width.saturating_add(non_client_width);
    let window_height = client_height.saturating_add(non_client_height);
    let center_x = window_rect.left + rect_width(&window_rect) / 2;
    let center_y = window_rect.top + rect_height(&window_rect) / 2;

    // SAFETY: hwnd is the live modal dialog during WM_INITDIALOG. The size keeps the
    // client area matched to the fixed-pixel child control layout.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            center_x - window_width / 2,
            center_y - window_height / 2,
            window_width,
            window_height,
            SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
        ) != 0
    }
}

fn dialog_rects(hwnd: HWND) -> Option<(RECT, RECT)> {
    let mut window_rect = empty_rect();
    let mut client_rect = empty_rect();

    // SAFETY: RECT values are valid writable storage and hwnd is a live dialog window.
    let has_window_rect = unsafe { GetWindowRect(hwnd, &mut window_rect) != 0 };
    if !has_window_rect {
        return None;
    }

    // SAFETY: RECT values are valid writable storage and hwnd is a live dialog window.
    let has_client_rect = unsafe { GetClientRect(hwnd, &mut client_rect) != 0 };
    if !has_client_rect {
        return None;
    }

    Some((window_rect, client_rect))
}

fn empty_rect() -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: 0,
        bottom: 0,
    }
}

fn rect_width(rect: &RECT) -> i32 {
    rect.right.saturating_sub(rect.left).max(0)
}

fn rect_height(rect: &RECT) -> i32 {
    rect.bottom.saturating_sub(rect.top).max(0)
}

#[derive(Clone, Copy)]
enum ControlKind {
    Combo,
    Edit,
}

#[derive(Clone, Copy)]
enum ButtonKind {
    Default,
    Normal,
}

fn populate_dialog(hwnd: HWND, config: &AppConfig) {
    set_combo_selection(
        hwnd,
        ID_UI_LANGUAGE,
        ui_text::ui_language_index(config.ui_language()),
    );
    set_combo_selection(
        hwnd,
        ID_DEFAULT_VIEW_MODE,
        view_mode_index(config.default_view_mode()),
    );
    set_combo_selection(
        hwnd,
        ID_SCALING_QUALITY,
        scaling_quality_index(config.scaling_quality()),
    );
    set_control_text(
        hwnd,
        ID_EXPORT_QUALITY,
        &config.export_default_quality().to_string(),
    );
    set_combo_selection(
        hwnd,
        ID_EXPORT_FORMAT_POLICY,
        export_policy_index(config.export_settings().default_export_format_policy()),
    );
    set_control_text(
        hwnd,
        ID_EXPORT_SUFFIX,
        config.export_settings().export_filename_suffix(),
    );
    set_control_text(
        hwnd,
        ID_JPEG_ALPHA_BACKGROUND_RGB,
        &rgb_color_text(config.export_settings().jpeg_alpha_background_rgb()),
    );

    let zoom = config.zoom_settings();
    set_control_text(
        hwnd,
        ID_MIN_ZOOM_SCALE,
        &format!("{}", zoom.min_zoom_scale()),
    );
    set_control_text(
        hwnd,
        ID_MAX_ZOOM_SCALE,
        &format!("{}", zoom.max_zoom_scale()),
    );
    set_control_text(
        hwnd,
        ID_ZOOM_STEP_FACTOR,
        &format!("{}", zoom.zoom_step_factor()),
    );

    let memory = config.memory_policy_settings();
    set_control_text(
        hwnd,
        ID_LARGE_IMAGE_PIXEL_THRESHOLD,
        &memory.large_image_pixel_threshold().to_string(),
    );
    set_control_text(
        hwnd,
        ID_MAX_IMAGE_PIXELS,
        &memory.max_image_pixels().to_string(),
    );
    set_control_text(
        hwnd,
        ID_PREVIEW_MAX_PIXELS,
        &memory.preview_max_pixels().to_string(),
    );
    set_control_text(
        hwnd,
        ID_PREVIEW_OVERSAMPLE,
        &memory.preview_oversample().to_string(),
    );
    set_control_text(
        hwnd,
        ID_FULL_RESOLUTION_REQUEST_SCALE,
        &format!("{}", memory.full_resolution_request_scale()),
    );
    set_control_text(
        hwnd,
        ID_MAX_RESIDENT_MIB,
        &memory.max_resident_mib().to_string(),
    );
    set_control_text(
        hwnd,
        ID_MAX_CACHE_ENTRY_MIB,
        &memory.max_cache_entry_mib().to_string(),
    );
    set_control_text(
        hwnd,
        ID_MAX_CACHE_ENTRIES,
        &memory.max_cache_entries().to_string(),
    );

    let animation_timing = config.animation_timing_settings();
    set_control_text(
        hwnd,
        ID_DEFAULT_FRAME_DELAY_MS,
        &animation_timing.default_frame_delay_ms().to_string(),
    );
    set_control_text(
        hwnd,
        ID_MIN_FRAME_DELAY_MS,
        &animation_timing.min_frame_delay_ms().to_string(),
    );
    set_control_text(
        hwnd,
        ID_MAX_FRAME_DELAY_MS,
        &animation_timing.max_frame_delay_ms().to_string(),
    );

    let navigation = config.navigation_settings();
    set_control_text(
        hwnd,
        ID_NAVIGATION_ATTEMPTS,
        &navigation.max_navigation_attempts_per_command().to_string(),
    );
    set_checkbox(hwnd, ID_ANIMATION_AUTOPLAY, config.animation_autoplay());
    set_checkbox(hwnd, ID_WRAP_NAVIGATION, navigation.wrap_navigation());
    set_checkbox(
        hwnd,
        ID_AUTO_SKIP_NAVIGATION,
        navigation.auto_skip_failed_navigation(),
    );

    let status_ui = config.status_ui_settings();
    set_checkbox(hwnd, ID_SHOW_STATUS_BAR, status_ui.show_status_bar());
    set_checkbox(
        hwnd,
        ID_DETAILED_STATUS_TEXT,
        status_ui.detailed_status_text(),
    );

    let interaction = config.interaction_settings();
    set_combo_selection(
        hwnd,
        ID_ZOOM_SHORTCUT,
        wheel_shortcut_index(interaction.zoom_shortcut()),
    );
    set_combo_selection(
        hwnd,
        ID_IMAGE_NAVIGATION_SHORTCUT,
        wheel_shortcut_index(interaction.image_navigation_shortcut()),
    );
    set_combo_selection(
        hwnd,
        ID_IMAGE_PAN_SHORTCUT,
        drag_shortcut_index(interaction.image_pan_shortcut()),
    );
    set_combo_selection(
        hwnd,
        ID_WINDOW_MOVE_SHORTCUT,
        drag_shortcut_index(interaction.window_move_shortcut()),
    );
}

fn read_dialog_config(hwnd: HWND, base: &AppConfig) -> Result<AppConfig, String> {
    let mut config = base.clone();
    config.set_ui_language(read_ui_language(hwnd)?);
    config.set_default_view_mode(read_view_mode(hwnd)?);
    config.set_scaling_quality(read_scaling_quality(hwnd)?);
    config.set_export_default_quality(read_export_quality(hwnd)?);
    config.set_animation_autoplay(is_checked(hwnd, ID_ANIMATION_AUTOPLAY));

    config.set_export_settings(read_export_settings(hwnd, config.export_settings())?);
    config.set_zoom_settings(read_zoom_settings(hwnd)?);
    config.set_memory_policy_settings(read_memory_policy_settings(
        hwnd,
        config.memory_policy_settings(),
    )?);
    config.set_animation_timing_settings(read_animation_timing_settings(hwnd)?);
    config.set_navigation_settings(read_navigation_settings(hwnd)?);
    config.set_status_ui_settings(read_status_ui_settings(hwnd));
    config.set_interaction_settings(read_interaction_settings(hwnd)?);

    Ok(config)
}

fn read_ui_language(hwnd: HWND) -> Result<UiLanguage, String> {
    combo_selection(hwnd, ID_UI_LANGUAGE)
        .and_then(ui_text::ui_language_from_index)
        .ok_or_else(|| match dialog_language(hwnd) {
            UiLanguage::English => "Select a UI language.".to_owned(),
            UiLanguage::Korean => "UI 언어를 선택할 수 없습니다.".to_owned(),
        })
}

fn read_view_mode(hwnd: HWND) -> Result<ViewMode, String> {
    match combo_selection(hwnd, ID_DEFAULT_VIEW_MODE) {
        Some(0) => Ok(ViewMode::FitToWindow),
        Some(1) => Ok(ViewMode::ActualSize),
        _ => Err("기본 보기 값을 선택할 수 없습니다.".to_owned()),
    }
}

fn read_scaling_quality(hwnd: HWND) -> Result<ScalingQuality, String> {
    match combo_selection(hwnd, ID_SCALING_QUALITY) {
        Some(0) => Ok(ScalingQuality::Nearest),
        Some(1) => Ok(ScalingQuality::Balanced),
        Some(2) => Ok(ScalingQuality::HighQuality),
        _ => Err("스케일링 품질 값을 선택할 수 없습니다.".to_owned()),
    }
}

fn read_export_policy(hwnd: HWND) -> Result<DefaultExportFormatPolicy, String> {
    match combo_selection(hwnd, ID_EXPORT_FORMAT_POLICY) {
        Some(0) => Ok(DefaultExportFormatPolicy::Source),
        Some(1) => Ok(DefaultExportFormatPolicy::Png),
        Some(2) => Ok(DefaultExportFormatPolicy::Jpeg),
        Some(3) => Ok(DefaultExportFormatPolicy::Bmp),
        Some(4) => Ok(DefaultExportFormatPolicy::Webp),
        Some(5) => Ok(DefaultExportFormatPolicy::Ico),
        _ => Err("내보내기 형식 값을 선택할 수 없습니다.".to_owned()),
    }
}

fn read_export_quality(hwnd: HWND) -> Result<u8, String> {
    parse_u8_range_field(
        hwnd,
        ID_EXPORT_QUALITY,
        "JPEG 품질",
        MIN_EXPORT_QUALITY,
        MAX_EXPORT_QUALITY,
    )
}

fn read_export_settings(hwnd: HWND, base: &ExportSettings) -> Result<ExportSettings, String> {
    let mut export = base.clone();
    export.set_default_export_format_policy(read_export_policy(hwnd)?);
    export.set_export_filename_suffix(read_export_suffix(hwnd)?);
    export.set_jpeg_alpha_background_rgb(read_rgb_color_field(
        hwnd,
        ID_JPEG_ALPHA_BACKGROUND_RGB,
        "JPEG 투명 배경 RGB",
    )?);
    Ok(export)
}

fn read_zoom_settings(hwnd: HWND) -> Result<ZoomSettings, String> {
    let min_zoom_scale = parse_f64_range_field(
        hwnd,
        ID_MIN_ZOOM_SCALE,
        "최소 줌",
        MIN_CONFIG_MIN_ZOOM_SCALE,
        MAX_CONFIG_MIN_ZOOM_SCALE,
    )?;
    let max_zoom_scale = parse_f64_range_field(
        hwnd,
        ID_MAX_ZOOM_SCALE,
        "최대 줌",
        MIN_CONFIG_MAX_ZOOM_SCALE,
        MAX_CONFIG_MAX_ZOOM_SCALE,
    )?;
    if min_zoom_scale > max_zoom_scale {
        return Err("최소 줌 값은 최대 줌 값보다 클 수 없습니다.".to_owned());
    }
    let zoom_step_factor = parse_f64_range_field(
        hwnd,
        ID_ZOOM_STEP_FACTOR,
        "줌 단계 배율",
        MIN_CONFIG_ZOOM_STEP_FACTOR,
        MAX_CONFIG_ZOOM_STEP_FACTOR,
    )?;

    Ok(ZoomSettings::new(
        min_zoom_scale,
        max_zoom_scale,
        zoom_step_factor,
    ))
}

fn read_memory_policy_settings(
    hwnd: HWND,
    base: MemoryPolicySettings,
) -> Result<MemoryPolicySettings, String> {
    let large_image_pixel_threshold = parse_u64_range_field(
        hwnd,
        ID_LARGE_IMAGE_PIXEL_THRESHOLD,
        "대용량 픽셀 기준",
        MIN_CONFIG_PIXEL_LIMIT,
        MAX_CONFIG_IMAGE_PIXELS,
    )?;
    let max_image_pixels = parse_u64_range_field(
        hwnd,
        ID_MAX_IMAGE_PIXELS,
        "최대 이미지 픽셀",
        MIN_CONFIG_PIXEL_LIMIT,
        MAX_CONFIG_IMAGE_PIXELS,
    )?;
    if large_image_pixel_threshold > max_image_pixels {
        return Err("대용량 픽셀 기준 값은 최대 이미지 픽셀 값보다 클 수 없습니다.".to_owned());
    }

    let preview_max_pixels = parse_u64_range_field(
        hwnd,
        ID_PREVIEW_MAX_PIXELS,
        "프리뷰 최대 픽셀",
        MIN_CONFIG_PIXEL_LIMIT,
        MAX_CONFIG_IMAGE_PIXELS,
    )?;
    if preview_max_pixels > max_image_pixels {
        return Err("프리뷰 최대 픽셀 값은 최대 이미지 픽셀 값보다 클 수 없습니다.".to_owned());
    }

    let preview_oversample = parse_u32_range_field(
        hwnd,
        ID_PREVIEW_OVERSAMPLE,
        "프리뷰 배율",
        MIN_CONFIG_PREVIEW_OVERSAMPLE,
        MAX_CONFIG_PREVIEW_OVERSAMPLE,
    )?;
    let full_resolution_request_scale = parse_f64_range_field(
        hwnd,
        ID_FULL_RESOLUTION_REQUEST_SCALE,
        "전체 해상도 요청 배율",
        MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
        MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
    )?;
    let max_resident_mib = parse_u32_range_field(
        hwnd,
        ID_MAX_RESIDENT_MIB,
        "캐시 총량(MiB)",
        MIN_CONFIG_MEMORY_MIB,
        MAX_CONFIG_MEMORY_MIB,
    )?;
    let max_cache_entry_mib = parse_u32_range_field(
        hwnd,
        ID_MAX_CACHE_ENTRY_MIB,
        "캐시 항목 한도(MiB)",
        MIN_CONFIG_MEMORY_MIB,
        MAX_CONFIG_MEMORY_MIB,
    )?;
    if max_cache_entry_mib > max_resident_mib {
        return Err("캐시 항목 한도(MiB)는 캐시 총량(MiB)보다 클 수 없습니다.".to_owned());
    }
    let max_cache_entries = parse_usize_range_field(
        hwnd,
        ID_MAX_CACHE_ENTRIES,
        "캐시 항목 수",
        MIN_CONFIG_CACHE_ENTRIES,
        MAX_CONFIG_CACHE_ENTRIES,
    )?;

    let mut memory = base;
    memory.set_max_image_pixels(max_image_pixels);
    memory.set_large_image_pixel_threshold(large_image_pixel_threshold);
    memory.set_preview_max_pixels(preview_max_pixels);
    memory.set_preview_oversample(preview_oversample);
    memory.set_full_resolution_request_scale(full_resolution_request_scale);
    memory.set_max_resident_mib(max_resident_mib);
    memory.set_max_cache_entry_mib(max_cache_entry_mib);
    memory.set_max_cache_entries(max_cache_entries);
    Ok(memory)
}

fn read_animation_timing_settings(hwnd: HWND) -> Result<AnimationTimingSettings, String> {
    let default_frame_delay_ms = parse_u32_range_field(
        hwnd,
        ID_DEFAULT_FRAME_DELAY_MS,
        "기본 프레임 지연",
        MIN_CONFIG_ANIMATION_DELAY_MS,
        MAX_CONFIG_ANIMATION_DELAY_MS,
    )?;
    let min_frame_delay_ms = parse_u32_range_field(
        hwnd,
        ID_MIN_FRAME_DELAY_MS,
        "최소 프레임 지연",
        MIN_CONFIG_ANIMATION_DELAY_MS,
        MAX_CONFIG_ANIMATION_DELAY_MS,
    )?;
    let max_frame_delay_ms = parse_u32_range_field(
        hwnd,
        ID_MAX_FRAME_DELAY_MS,
        "최대 프레임 지연",
        MIN_CONFIG_ANIMATION_DELAY_MS,
        MAX_CONFIG_ANIMATION_DELAY_MS,
    )?;

    if min_frame_delay_ms > max_frame_delay_ms {
        return Err("최소 프레임 지연 값은 최대 프레임 지연 값보다 클 수 없습니다.".to_owned());
    }
    if default_frame_delay_ms < min_frame_delay_ms || default_frame_delay_ms > max_frame_delay_ms {
        return Err(
            "기본 프레임 지연 값은 최소/최대 프레임 지연 범위 안에 있어야 합니다.".to_owned(),
        );
    }

    let mut timing = AnimationTimingSettings::default();
    timing.set_min_frame_delay_ms(min_frame_delay_ms);
    timing.set_max_frame_delay_ms(max_frame_delay_ms);
    timing.set_default_frame_delay_ms(default_frame_delay_ms);
    Ok(timing)
}

fn read_navigation_settings(hwnd: HWND) -> Result<NavigationSettings, String> {
    let mut navigation = NavigationSettings::default();
    navigation.set_wrap_navigation(is_checked(hwnd, ID_WRAP_NAVIGATION));
    navigation.set_auto_skip_failed_navigation(is_checked(hwnd, ID_AUTO_SKIP_NAVIGATION));
    navigation.set_max_navigation_attempts_per_command(parse_usize_range_field(
        hwnd,
        ID_NAVIGATION_ATTEMPTS,
        "최대 시도 횟수",
        1,
        100,
    )?);
    Ok(navigation)
}

fn read_status_ui_settings(hwnd: HWND) -> StatusUiSettings {
    let mut status_ui = StatusUiSettings::default();
    status_ui.set_show_status_bar(is_checked(hwnd, ID_SHOW_STATUS_BAR));
    status_ui.set_detailed_status_text(is_checked(hwnd, ID_DETAILED_STATUS_TEXT));
    status_ui
}

fn read_interaction_settings(hwnd: HWND) -> Result<InteractionSettings, String> {
    let mut interaction = InteractionSettings::default();
    interaction.set_zoom_shortcut(read_wheel_shortcut(hwnd, ID_ZOOM_SHORTCUT, "확대/축소")?);
    interaction.set_image_navigation_shortcut(read_wheel_shortcut(
        hwnd,
        ID_IMAGE_NAVIGATION_SHORTCUT,
        "이전/다음 이미지",
    )?);
    interaction.set_image_pan_shortcut(read_drag_shortcut(
        hwnd,
        ID_IMAGE_PAN_SHORTCUT,
        "이미지 이동",
    )?);
    interaction.set_window_move_shortcut(read_drag_shortcut(
        hwnd,
        ID_WINDOW_MOVE_SHORTCUT,
        "창 이동",
    )?);
    Ok(interaction)
}

fn read_wheel_shortcut(hwnd: HWND, id: i32, label: &str) -> Result<MouseShortcut, String> {
    match combo_selection(hwnd, id) {
        Some(0) => Ok(MouseShortcut::MouseWheel),
        Some(1) => Ok(MouseShortcut::CtrlMouseWheel),
        _ => Err(format!("{label} 단축키 값을 선택할 수 없습니다.")),
    }
}

fn read_drag_shortcut(hwnd: HWND, id: i32, label: &str) -> Result<MouseShortcut, String> {
    match combo_selection(hwnd, id) {
        Some(0) => Ok(MouseShortcut::LeftButtonDrag),
        Some(1) => Ok(MouseShortcut::CtrlLeftButtonDrag),
        _ => Err(format!("{label} 단축키 값을 선택할 수 없습니다.")),
    }
}

fn read_export_suffix(hwnd: HWND) -> Result<String, String> {
    let suffix = control_text(hwnd, ID_EXPORT_SUFFIX)?;
    match validate_export_filename_suffix(&suffix) {
        Ok(suffix) => Ok(suffix.to_owned()),
        Err(ExportFilenameSuffixValidationError::Empty) => Err(format!(
            "파일명 suffix 값은 비워 둘 수 없습니다. 기본값은 {DEFAULT_EXPORT_FILENAME_SUFFIX}입니다."
        )),
        Err(ExportFilenameSuffixValidationError::TooLong) => Err(format!(
            "파일명 suffix 값은 최대 {MAX_EXPORT_FILENAME_SUFFIX_CHARS}자까지 입력할 수 있습니다."
        )),
        Err(ExportFilenameSuffixValidationError::InvalidCharacter) => Err(
            "파일명 suffix 값에는 \\ / : * ? \" < > | 또는 제어 문자를 사용할 수 없습니다."
                .to_owned(),
        ),
    }
}

fn read_rgb_color_field(hwnd: HWND, id: i32, label: &str) -> Result<RgbColor, String> {
    let text = control_text(hwnd, id)?;
    let mut components = text.split(',');
    let red = parse_rgb_component(components.next(), label, "R")?;
    let green = parse_rgb_component(components.next(), label, "G")?;
    let blue = parse_rgb_component(components.next(), label, "B")?;
    if components.next().is_some() {
        return Err(format!("{label} 값은 R,G,B 형식으로 입력해야 합니다."));
    }
    Ok(RgbColor::new(red, green, blue))
}

fn parse_f64_field(hwnd: HWND, id: i32, label: &str) -> Result<f64, String> {
    let text = control_text(hwnd, id)?;
    let value = text
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label} 값은 숫자로 입력해야 합니다."))?;
    if value.is_finite() {
        Ok(value)
    } else {
        Err(format!("{label} 값은 유한한 숫자로 입력해야 합니다."))
    }
}

fn parse_f64_range_field(
    hwnd: HWND,
    id: i32,
    label: &str,
    min: f64,
    max: f64,
) -> Result<f64, String> {
    let value = parse_f64_field(hwnd, id, label)?;
    if value < min || value > max {
        Err(format!("{label} 값은 {min}부터 {max} 사이여야 합니다."))
    } else {
        Ok(value)
    }
}

fn parse_u64_range_field(
    hwnd: HWND,
    id: i32,
    label: &str,
    min: u64,
    max: u64,
) -> Result<u64, String> {
    let value = parse_i128_field(hwnd, id, label)?;
    if value < i128::from(min) || value > i128::from(max) {
        return Err(format!("{label} 값은 {min}부터 {max} 사이여야 합니다."));
    }
    u64::try_from(value).map_err(|_| format!("{label} 값을 적용할 수 없습니다."))
}

fn parse_u32_range_field(
    hwnd: HWND,
    id: i32,
    label: &str,
    min: u32,
    max: u32,
) -> Result<u32, String> {
    let value = parse_i128_field(hwnd, id, label)?;
    if value < i128::from(min) || value > i128::from(max) {
        return Err(format!("{label} 값은 {min}부터 {max} 사이여야 합니다."));
    }
    u32::try_from(value).map_err(|_| format!("{label} 값을 적용할 수 없습니다."))
}

fn parse_u8_range_field(hwnd: HWND, id: i32, label: &str, min: u8, max: u8) -> Result<u8, String> {
    let value = parse_i128_field(hwnd, id, label)?;
    if value < i128::from(min) || value > i128::from(max) {
        return Err(format!("{label} 값은 {min}부터 {max} 사이여야 합니다."));
    }
    u8::try_from(value).map_err(|_| format!("{label} 값을 적용할 수 없습니다."))
}

fn parse_usize_range_field(
    hwnd: HWND,
    id: i32,
    label: &str,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let value = parse_i128_field(hwnd, id, label)?;
    if value < min as i128 || value > max as i128 {
        return Err(format!("{label} 값은 {min}부터 {max} 사이여야 합니다."));
    }
    usize::try_from(value).map_err(|_| format!("{label} 값을 적용할 수 없습니다."))
}

fn parse_i128_field(hwnd: HWND, id: i32, label: &str) -> Result<i128, String> {
    let text = control_text(hwnd, id)?;
    text.trim()
        .parse::<i128>()
        .map_err(|_| format!("{label} 값은 정수로 입력해야 합니다."))
}

fn parse_rgb_component(
    value: Option<&str>,
    label: &str,
    component_label: &str,
) -> Result<u8, String> {
    let Some(value) = value else {
        return Err(format!("{label} 값은 R,G,B 형식으로 입력해야 합니다."));
    };
    let value = value
        .trim()
        .parse::<i128>()
        .map_err(|_| format!("{label}의 {component_label} 값은 정수로 입력해야 합니다."))?;
    if value < 0 || value > i128::from(u8::MAX) {
        return Err(format!(
            "{label}의 {component_label} 값은 0부터 255 사이여야 합니다."
        ));
    }
    u8::try_from(value).map_err(|_| format!("{label}의 {component_label} 값을 적용할 수 없습니다."))
}

fn rgb_color_text(color: RgbColor) -> String {
    format!("{},{},{}", color.red(), color.green(), color.blue())
}

fn view_mode_index(view_mode: ViewMode) -> usize {
    match view_mode {
        ViewMode::FitToWindow | ViewMode::ManualZoom => 0,
        ViewMode::ActualSize => 1,
    }
}

fn scaling_quality_index(quality: ScalingQuality) -> usize {
    match quality {
        ScalingQuality::Nearest => 0,
        ScalingQuality::Balanced => 1,
        ScalingQuality::HighQuality => 2,
    }
}

fn export_policy_index(policy: DefaultExportFormatPolicy) -> usize {
    match policy {
        DefaultExportFormatPolicy::Source => 0,
        DefaultExportFormatPolicy::Png => 1,
        DefaultExportFormatPolicy::Jpeg => 2,
        DefaultExportFormatPolicy::Bmp => 3,
        DefaultExportFormatPolicy::Webp => 4,
        DefaultExportFormatPolicy::Ico => 5,
    }
}

fn wheel_shortcut_index(shortcut: MouseShortcut) -> usize {
    match shortcut {
        MouseShortcut::MouseWheel | MouseShortcut::LeftButtonDrag => 0,
        MouseShortcut::CtrlMouseWheel | MouseShortcut::CtrlLeftButtonDrag => 1,
    }
}

fn drag_shortcut_index(shortcut: MouseShortcut) -> usize {
    match shortcut {
        MouseShortcut::LeftButtonDrag | MouseShortcut::MouseWheel => 0,
        MouseShortcut::CtrlLeftButtonDrag | MouseShortcut::CtrlMouseWheel => 1,
    }
}

fn create_group_box(
    parent: HWND,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Option<HWND> {
    create_child_control(
        parent,
        "BUTTON",
        text,
        0,
        WS_CHILD | WS_VISIBLE | BS_GROUPBOX as u32,
        ControlRect::new(x, y, width, height),
    )
}

fn create_label(parent: HWND, text: &str, x: i32, y: i32, width: i32, height: i32) -> Option<HWND> {
    create_child_control(
        parent,
        "STATIC",
        text,
        0,
        WS_CHILD | WS_VISIBLE,
        ControlRect::new(x, y, width, height),
    )
}

fn create_edit(parent: HWND, id: i32, x: i32, y: i32, width: i32, height: i32) -> Option<HWND> {
    create_child_control(
        parent,
        "EDIT",
        "",
        id,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | WS_BORDER | ES_AUTOHSCROLL as u32,
        ControlRect::new(x, y, width, height),
    )
}

fn create_combo(parent: HWND, id: i32, x: i32, y: i32, width: i32, height: i32) -> Option<HWND> {
    create_child_control(
        parent,
        "COMBOBOX",
        "",
        id,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | CBS_DROPDOWNLIST as u32,
        ControlRect::new(x, y, width, height),
    )
}

fn create_checkbox(
    parent: HWND,
    id: i32,
    text: &str,
    x: i32,
    y: i32,
    width: i32,
    height: i32,
) -> Option<HWND> {
    create_child_control(
        parent,
        "BUTTON",
        text,
        id,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_AUTOCHECKBOX as u32,
        ControlRect::new(x, y, width, height),
    )
}

fn create_project_link(parent: HWND, rect: ControlRect) -> Option<HWND> {
    if init_link_controls() {
        let link_text = format!("<A HREF=\"{PROJECT_LINK_URL}\">{PROJECT_LINK_URL}</A>");
        if let Some(control) = create_child_control(
            parent,
            "SysLink",
            &link_text,
            ID_PROJECT_LINK,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            rect,
        ) {
            return Some(control);
        }
    }

    create_child_control(
        parent,
        "STATIC",
        PROJECT_LINK_URL,
        ID_PROJECT_LINK,
        WS_CHILD | WS_VISIBLE | STATIC_NOTIFY_STYLE,
        rect,
    )
}

fn init_link_controls() -> bool {
    let Ok(size) = u32::try_from(std::mem::size_of::<INITCOMMONCONTROLSEX>()) else {
        return false;
    };
    let controls = INITCOMMONCONTROLSEX {
        dwSize: size,
        dwICC: ICC_LINK_CLASS,
    };
    // SAFETY: controls points to a valid INITCOMMONCONTROLSEX with the documented size.
    unsafe { InitCommonControlsEx(&controls) != 0 }
}

fn create_button(
    parent: HWND,
    id: i32,
    text: &str,
    rect: ControlRect,
    kind: ButtonKind,
) -> Option<HWND> {
    let style = match kind {
        ButtonKind::Default => BS_DEFPUSHBUTTON,
        ButtonKind::Normal => BS_PUSHBUTTON,
    };
    create_child_control(
        parent,
        "BUTTON",
        text,
        id,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | style as u32,
        rect,
    )
}

#[derive(Clone, Copy)]
struct ControlRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ControlRect {
    fn new(x: i32, y: i32, width: i32, height: i32) -> Self {
        Self {
            x,
            y,
            width,
            height,
        }
    }

    fn scale_for_dpi(self, dpi_y: i32) -> Self {
        Self {
            x: dpi::scale_i32_for_dpi(self.x, dpi_y),
            y: dpi::scale_i32_for_dpi(self.y, dpi_y),
            width: dpi::scale_i32_for_dpi(self.width, dpi_y),
            height: dpi::scale_i32_for_dpi(self.height, dpi_y),
        }
    }
}

fn create_child_control(
    parent: HWND,
    class_name: &str,
    text: &str,
    id: i32,
    style: u32,
    rect: ControlRect,
) -> Option<HWND> {
    let class_name = wide_null(class_name);
    let text = wide_null(text);
    let rect = rect.scale_for_dpi(dpi::dpi_y_for_window(parent));
    // SAFETY: class_name and text are null-terminated UTF-16 buffers valid for the call.
    // parent is the dialog receiving child controls, and the child id is passed as hmenu.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name.as_ptr(),
            text.as_ptr(),
            style,
            rect.x,
            rect.y,
            rect.width,
            rect.height,
            parent,
            id as usize as HMENU,
            null_mut(),
            null(),
        )
    };
    if hwnd.is_null() {
        None
    } else {
        apply_dialog_font(parent, hwnd);
        Some(hwnd)
    }
}

fn apply_dialog_font(parent: HWND, hwnd: HWND) {
    let Some(font) = dialog_font(parent) else {
        return;
    };

    // SAFETY: hwnd is a live control and font is owned by the dialog state for the modal lifetime.
    unsafe {
        SendMessageW(hwnd, WM_SETFONT, font as WPARAM, 1);
    }
}

fn dialog_font(hwnd: HWND) -> Option<windows_sys::Win32::Graphics::Gdi::HFONT> {
    dialog_state_mut(hwnd)
        .and_then(|state| state.ui_font.as_ref())
        .map(|font| font.handle())
}

fn initialize_combo(hwnd: HWND, id: i32, labels: &[&str]) -> bool {
    for label in labels {
        let label = wide_null(label);
        // SAFETY: label is a null-terminated UTF-16 buffer valid for the call.
        let result =
            unsafe { SendDlgItemMessageW(hwnd, id, CB_ADDSTRING, 0, label.as_ptr() as LPARAM) };
        if result == CB_ERR as isize {
            return false;
        }
    }
    set_combo_selection(hwnd, id, 0)
}

fn set_combo_selection(hwnd: HWND, id: i32, index: usize) -> bool {
    // SAFETY: hwnd is the dialog and id identifies a combo box created during initialization.
    unsafe { SendDlgItemMessageW(hwnd, id, CB_SETCURSEL, index, 0) != CB_ERR as isize }
}

fn combo_selection(hwnd: HWND, id: i32) -> Option<usize> {
    // SAFETY: hwnd is the dialog and id identifies a combo box created during initialization.
    let result = unsafe { SendDlgItemMessageW(hwnd, id, CB_GETCURSEL, 0, 0) };
    if result == CB_ERR as isize {
        None
    } else {
        usize::try_from(result).ok()
    }
}

fn set_checkbox(hwnd: HWND, id: i32, checked: bool) {
    let value = if checked {
        BST_CHECKED_VALUE
    } else {
        BST_UNCHECKED_VALUE
    };
    // SAFETY: hwnd is the dialog and id identifies a checkbox button.
    unsafe {
        SendDlgItemMessageW(hwnd, id, BM_SETCHECK, value, 0);
    }
}

fn is_checked(hwnd: HWND, id: i32) -> bool {
    // SAFETY: hwnd is the dialog and id identifies a checkbox button.
    unsafe { SendDlgItemMessageW(hwnd, id, BM_GETCHECK, 0, 0) == BST_CHECKED_VALUE as isize }
}

fn set_control_text(hwnd: HWND, id: i32, text: &str) -> bool {
    let text = wide_null(text);
    // SAFETY: text is a null-terminated UTF-16 buffer valid for the call.
    unsafe { SetDlgItemTextW(hwnd, id, text.as_ptr()) != 0 }
}

fn control_text(hwnd: HWND, id: i32) -> Result<String, String> {
    // SAFETY: hwnd is the dialog and id is a child control id.
    let control = unsafe { GetDlgItem(hwnd, id) };
    if control.is_null() {
        return Err("설정 값을 읽을 수 없습니다.".to_owned());
    }

    // SAFETY: control is a live child window and the call only reads its text length.
    let length = unsafe { GetWindowTextLengthW(control) };
    if length < 0 {
        return Err("설정 값을 읽을 수 없습니다.".to_owned());
    }
    let capacity = length.saturating_add(1);
    let Ok(capacity) = usize::try_from(capacity) else {
        return Err("설정 값이 너무 깁니다.".to_owned());
    };
    let mut buffer = vec![0u16; capacity];
    let max_count = match i32::try_from(buffer.len()) {
        Ok(value) => value,
        Err(_) => return Err("설정 값이 너무 깁니다.".to_owned()),
    };
    // SAFETY: buffer has max_count UTF-16 code units and is writable.
    let copied = unsafe { GetWindowTextW(control, buffer.as_mut_ptr(), max_count) };
    if copied < 0 {
        return Err("설정 값을 읽을 수 없습니다.".to_owned());
    }
    let Ok(copied) = usize::try_from(copied) else {
        return Err("설정 값을 읽을 수 없습니다.".to_owned());
    };
    Ok(String::from_utf16_lossy(&buffer[..copied]))
}

fn set_dialog_state(hwnd: HWND, state: *mut SettingsDialogState) -> bool {
    if state.is_null() {
        return false;
    }
    let name = wide_null(STATE_PROP_NAME);
    // SAFETY: state is valid for the modal dialog lifetime and stored as an opaque handle.
    unsafe { SetPropW(hwnd, name.as_ptr(), state as HANDLE) != 0 }
}

fn dialog_state_mut(hwnd: HWND) -> Option<&'static mut SettingsDialogState> {
    let name = wide_null(STATE_PROP_NAME);
    // SAFETY: The property, when present, is written only by set_dialog_state with this type.
    let handle = unsafe { GetPropW(hwnd, name.as_ptr()) };
    if handle.is_null() {
        None
    } else {
        // SAFETY: DialogBoxIndirectParamW keeps the pointed state alive until the dialog closes.
        Some(unsafe { &mut *(handle as *mut SettingsDialogState) })
    }
}

fn dialog_language(hwnd: HWND) -> UiLanguage {
    dialog_state_mut(hwnd)
        .map(|state| state.draft.ui_language())
        .unwrap_or_default()
}

fn remove_dialog_state(hwnd: HWND) {
    let name = wide_null(STATE_PROP_NAME);
    // SAFETY: Removing a missing property is harmless and returns null.
    unsafe {
        let _ = RemovePropW(hwnd, name.as_ptr());
    }
}

fn show_warning_message(hwnd: HWND, message: &str) {
    let message = wide_null(message);
    let title = wide_null(ui_text::settings_title(dialog_language(hwnd)));
    // SAFETY: message and title are null-terminated UTF-16 buffers valid for the call.
    unsafe {
        MessageBoxW(
            hwnd,
            message.as_ptr(),
            title.as_ptr(),
            MB_OK | MB_ICONWARNING,
        );
    }
}

fn open_project_link(hwnd: HWND) {
    let operation = wide_null("open");
    let url = wide_null(PROJECT_LINK_URL);
    // SAFETY: operation and url are null-terminated UTF-16 buffers valid for the call.
    let result = unsafe {
        ShellExecuteW(
            hwnd,
            operation.as_ptr(),
            url.as_ptr(),
            null(),
            null(),
            SW_SHOWNORMAL,
        )
    };
    if result as isize <= 32 {
        show_warning_message(hwnd, "링크를 열 수 없습니다.");
    }
}

fn close_dialog(hwnd: HWND, result: isize) {
    // SAFETY: hwnd is the modal dialog window and result is an application-defined code.
    unsafe {
        EndDialog(hwnd, result);
    }
}

fn module_instance() -> Result<HINSTANCE, SettingsDialogError> {
    // SAFETY: null asks for the current process module handle.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        Err(SettingsDialogError::ModuleHandle { code: last_error() })
    } else {
        Ok(instance)
    }
}

fn low_word(value: WPARAM) -> u16 {
    (value & 0xffff) as u16
}

fn last_error() -> u32 {
    // SAFETY: GetLastError reads the calling thread's Win32 error code.
    unsafe { GetLastError() }
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

#[repr(C, align(4))]
struct DialogTemplateBuffer {
    bytes: [u8; TEMPLATE_BUFFER_BYTES],
}

impl DialogTemplateBuffer {
    fn new(title: &str) -> Self {
        let mut buffer = Self {
            bytes: [0; TEMPLATE_BUFFER_BYTES],
        };
        let mut writer = TemplateWriter {
            bytes: &mut buffer.bytes,
            offset: 0,
        };
        writer.write_u32(
            WS_POPUP | WS_CAPTION | WS_SYSMENU | DS_MODALFRAME as u32 | DS_CENTER as u32,
        );
        writer.write_u32(0);
        writer.write_u16(0);
        writer.write_i16(0);
        writer.write_i16(0);
        writer.write_i16(DIALOG_TEMPLATE_WIDTH);
        writer.write_i16(DIALOG_TEMPLATE_HEIGHT);
        writer.write_u16(0);
        writer.write_u16(0);
        writer.write_wide_null(title);
        writer.align_dword();
        buffer
    }

    fn as_ptr(&self) -> *const DLGTEMPLATE {
        self.bytes.as_ptr().cast()
    }
}

struct TemplateWriter<'a> {
    bytes: &'a mut [u8],
    offset: usize,
}

impl TemplateWriter<'_> {
    fn write_u32(&mut self, value: u32) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_i16(&mut self, value: i16) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_u16(&mut self, value: u16) {
        self.write_bytes(&value.to_le_bytes());
    }

    fn write_wide_null(&mut self, value: &str) {
        for unit in OsStr::new(value).encode_wide().chain(iter::once(0)) {
            self.write_u16(unit);
        }
    }

    fn align_dword(&mut self) {
        while !self.offset.is_multiple_of(4) {
            self.write_bytes(&[0]);
        }
    }

    fn write_bytes(&mut self, bytes: &[u8]) {
        let end = self.offset.saturating_add(bytes.len());
        if end <= self.bytes.len() {
            self.bytes[self.offset..end].copy_from_slice(bytes);
            self.offset = end;
        }
    }
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;
    use std::thread;
    use std::time::{Duration, Instant};

    use crate::domain::{RgbColor, ScalingQuality, ViewMode};
    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    use super::*;

    #[test]
    #[ignore = "opens the native Win32 modal settings dialog and drives it with window messages"]
    fn settings_dialog_accepts_valid_changes_from_win32_messages() {
        let (sender, receiver) = mpsc::channel();
        let initial_config = AppConfig::default();
        let dialog_thread = thread::spawn(move || {
            let outcome = show_settings_dialog(null_mut(), initial_config);
            let _ = sender.send(outcome);
        });

        let hwnd = wait_for_settings_dialog()
            .unwrap_or_else(|| panic!("settings dialog did not appear before timeout"));

        assert_settings_dialog_controls_exist(hwnd);
        assert_settings_dialog_client_size_matches_layout(hwnd);

        assert!(set_combo_selection(hwnd, ID_DEFAULT_VIEW_MODE, 1));
        assert!(set_combo_selection(hwnd, ID_SCALING_QUALITY, 2));
        assert!(set_control_text(hwnd, ID_EXPORT_QUALITY, "77"));
        assert!(set_control_text(hwnd, ID_EXPORT_SUFFIX, "_checked"));
        assert!(set_control_text(
            hwnd,
            ID_JPEG_ALPHA_BACKGROUND_RGB,
            "12,34,56"
        ));
        set_checkbox(hwnd, ID_SHOW_STATUS_BAR, false);

        // SAFETY: hwnd is the live settings dialog and WM_COMMAND/IDOK uses the same
        // path as a user pressing the default OK button.
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, IDOK as WPARAM, 0);
        }

        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("settings dialog should close after OK");
        dialog_thread
            .join()
            .expect("settings dialog thread should finish");

        let SettingsDialogOutcome::Accepted(config) = outcome.expect("dialog should succeed")
        else {
            panic!("dialog should accept valid changes");
        };

        assert_eq!(config.default_view_mode(), ViewMode::ActualSize);
        assert_eq!(config.scaling_quality(), ScalingQuality::HighQuality);
        assert_eq!(config.export_default_quality(), 77);
        assert!(!config.status_ui_settings().show_status_bar());
        assert_eq!(
            config.export_settings().export_filename_suffix(),
            "_checked"
        );
        assert_eq!(
            config.export_settings().jpeg_alpha_background_rgb(),
            RgbColor::new(12, 34, 56)
        );
    }

    fn wait_for_settings_dialog() -> Option<HWND> {
        let title = wide_null(ui_text::settings_title(UiLanguage::English));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: class name is null to match any class; title is null-terminated.
            let hwnd = unsafe { FindWindowW(null(), title.as_ptr()) };
            if !hwnd.is_null() && settings_dialog_controls_are_ready(hwnd) {
                return Some(hwnd);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn settings_dialog_controls_are_ready(hwnd: HWND) -> bool {
        // SAFETY: hwnd is a candidate settings dialog; GetDlgItem only looks up a child id.
        !unsafe { GetDlgItem(hwnd, ID_DEFAULTS) }.is_null()
            && !unsafe { GetDlgItem(hwnd, IDOK) }.is_null()
            && !unsafe { GetDlgItem(hwnd, IDCANCEL) }.is_null()
    }

    fn assert_settings_dialog_controls_exist(hwnd: HWND) {
        for id in [
            ID_DEFAULTS,
            ID_DEFAULT_VIEW_MODE,
            ID_SCALING_QUALITY,
            ID_EXPORT_QUALITY,
            ID_EXPORT_FORMAT_POLICY,
            ID_EXPORT_SUFFIX,
            ID_MIN_ZOOM_SCALE,
            ID_MAX_ZOOM_SCALE,
            ID_ZOOM_STEP_FACTOR,
            ID_NAVIGATION_ATTEMPTS,
            ID_ANIMATION_AUTOPLAY,
            ID_WRAP_NAVIGATION,
            ID_AUTO_SKIP_NAVIGATION,
            ID_SHOW_STATUS_BAR,
            ID_DETAILED_STATUS_TEXT,
            ID_LARGE_IMAGE_PIXEL_THRESHOLD,
            ID_MAX_IMAGE_PIXELS,
            ID_PREVIEW_MAX_PIXELS,
            ID_PREVIEW_OVERSAMPLE,
            ID_FULL_RESOLUTION_REQUEST_SCALE,
            ID_MAX_RESIDENT_MIB,
            ID_MAX_CACHE_ENTRY_MIB,
            ID_MAX_CACHE_ENTRIES,
            ID_DEFAULT_FRAME_DELAY_MS,
            ID_MIN_FRAME_DELAY_MS,
            ID_MAX_FRAME_DELAY_MS,
            ID_JPEG_ALPHA_BACKGROUND_RGB,
            ID_ZOOM_SHORTCUT,
            ID_IMAGE_NAVIGATION_SHORTCUT,
            ID_IMAGE_PAN_SHORTCUT,
            ID_WINDOW_MOVE_SHORTCUT,
            ID_PROJECT_LINK,
            IDOK,
            IDCANCEL,
        ] {
            // SAFETY: hwnd is the live settings dialog; GetDlgItem only looks up a child id.
            let control = unsafe { GetDlgItem(hwnd, id) };
            assert!(!control.is_null(), "missing settings dialog control {id}");
        }
    }

    fn assert_settings_dialog_client_size_matches_layout(hwnd: HWND) {
        let dpi_y = dpi::dpi_y_for_window(hwnd);
        let expected_width = dpi::scale_i32_for_dpi(DIALOG_CLIENT_WIDTH, dpi_y);
        let expected_height = dpi::scale_i32_for_dpi(DIALOG_CLIENT_HEIGHT, dpi_y);
        // SAFETY: RECT is a plain Win32 structure that GetClientRect fills.
        let mut client_rect: RECT = unsafe { std::mem::zeroed() };

        // SAFETY: hwnd is the live settings dialog and client_rect is writable storage.
        assert_ne!(unsafe { GetClientRect(hwnd, &mut client_rect) }, 0);
        assert_eq!(client_rect.right - client_rect.left, expected_width);
        assert_eq!(client_rect.bottom - client_rect.top, expected_height);
    }
}
