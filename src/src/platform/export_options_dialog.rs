use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, HANDLE, HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Input::KeyboardAndMouse::EnableWindow;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DialogBoxIndirectParamW, EndDialog, GetClientRect, GetDlgItem, GetPropW,
    GetWindowRect, GetWindowTextLengthW, GetWindowTextW, MessageBoxW, RemovePropW,
    SendDlgItemMessageW, SendMessageW, SetDlgItemTextW, SetPropW, SetWindowPos, BM_GETCHECK,
    BM_SETCHECK, BS_AUTOCHECKBOX, BS_DEFPUSHBUTTON, BS_GROUPBOX, BS_PUSHBUTTON, CBS_DROPDOWNLIST,
    CB_ADDSTRING, CB_ERR, CB_GETCURSEL, CB_SETCURSEL, DLGTEMPLATE, DS_CENTER, DS_MODALFRAME,
    ES_AUTOHSCROLL, HMENU, IDCANCEL, IDOK, MB_ICONWARNING, MB_OK, SWP_NOACTIVATE,
    SWP_NOOWNERZORDER, SWP_NOZORDER, WM_CLOSE, WM_COMMAND, WM_INITDIALOG, WM_NCDESTROY, WM_SETFONT,
    WS_BORDER, WS_CAPTION, WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE,
};

use crate::domain::{
    export_quality_range, export_size_from_height_preserving_aspect,
    export_size_from_width_preserving_aspect, ExportFormat, ImageRotation, ImageSize, UiLanguage,
    MAX_CONFIG_IMAGE_PIXELS,
};
use crate::ui_text;

use super::win32::dpi;

const STATE_PROP_NAME: &str = "j3Pic.ExportOptionsDialogState";
const DIALOG_RESULT_INIT_FAILED: isize = -2;

const ID_FORMAT: i32 = 101;
const ID_QUALITY: i32 = 102;
const ID_ORIGINAL_SIZE: i32 = 103;
const ID_WIDTH: i32 = 104;
const ID_HEIGHT: i32 = 105;
const ID_RESET_SIZE: i32 = 106;
const ID_KEEP_ASPECT: i32 = 107;
const ID_REMOVE_METADATA: i32 = 108;
const ID_ROTATION: i32 = 109;

const EN_CHANGE_CODE: u16 = 0x0300;
const CBN_SELCHANGE_CODE: u16 = 1;
const BST_UNCHECKED_VALUE: WPARAM = 0;
const BST_CHECKED_VALUE: WPARAM = 1;
const DIALOG_TEMPLATE_WIDTH: i16 = 260;
const DIALOG_TEMPLATE_HEIGHT: i16 = 270;
const DIALOG_CLIENT_WIDTH: i32 = 390;
const DIALOG_CLIENT_HEIGHT: i32 = 390;
const DIALOG_FONT_POINT_SIZE: i32 = 9;
const GROUP_X: i32 = 16;
const FILE_GROUP_Y: i32 = 14;
const SIZE_GROUP_Y: i32 = 174;
const GROUP_WIDTH: i32 = 358;
const GROUP_X_PADDING: i32 = 14;
const GROUP_TOP_PADDING: i32 = 25;
const GROUP_ROW_HEIGHT: i32 = 27;
const LABEL_WIDTH: i32 = 96;
const INPUT_WIDTH: i32 = 220;
const EDIT_HEIGHT: i32 = 22;
// Win32 uses the combo box height as the expanded drop-down extent.
const COMBO_HEIGHT: i32 = 160;
const TEMPLATE_BUFFER_BYTES: usize = 256;

const FORMAT_OPTIONS: &[(ExportFormat, &str)] = &[
    (ExportFormat::Png, "PNG"),
    (ExportFormat::Jpeg, "JPEG"),
    (ExportFormat::Bmp, "BMP"),
    (ExportFormat::Webp, "WebP"),
    (ExportFormat::Ico, "ICO"),
];

const ROTATION_OPTIONS: &[(ImageRotation, &str)] = &[
    (ImageRotation::Degrees0, "0도"),
    (ImageRotation::Degrees90, "90도 시계 방향"),
    (ImageRotation::Degrees180, "180도"),
    (ImageRotation::Degrees270, "270도 시계 방향"),
];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportOptionsDialogDefaults {
    original_size: ImageSize,
    default_format: ExportFormat,
    default_quality: u8,
    default_remove_metadata: bool,
    default_rotation: ImageRotation,
}

impl ExportOptionsDialogDefaults {
    pub(crate) fn new(
        original_size: ImageSize,
        default_format: ExportFormat,
        default_quality: u8,
        default_remove_metadata: bool,
    ) -> Self {
        Self {
            original_size,
            default_format,
            default_quality,
            default_remove_metadata,
            default_rotation: ImageRotation::ZERO,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) struct ExportOptionsDialogSelection {
    format: ExportFormat,
    quality: Option<u8>,
    rotation: ImageRotation,
    target_size: Option<ImageSize>,
    remove_metadata: bool,
}

impl ExportOptionsDialogSelection {
    pub(crate) fn format(self) -> ExportFormat {
        self.format
    }

    pub(crate) fn quality(self) -> Option<u8> {
        self.quality
    }

    pub(crate) fn target_size(self) -> Option<ImageSize> {
        self.target_size
    }

    pub(crate) fn rotation(self) -> ImageRotation {
        self.rotation
    }

    pub(crate) fn remove_metadata(self) -> bool {
        self.remove_metadata
    }
}

pub(crate) enum ExportOptionsDialogOutcome {
    Accepted(ExportOptionsDialogSelection),
    Cancelled,
}

#[derive(Debug)]
pub(crate) enum ExportOptionsDialogError {
    ModuleHandle { code: u32 },
    CreateDialog { code: u32 },
    InitializeControls,
}

impl fmt::Display for ExportOptionsDialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleHandle { code } => write!(
                formatter,
                "failed to get module handle for export options dialog: Win32 error {code}"
            ),
            Self::CreateDialog { code } => write!(
                formatter,
                "failed to create export options dialog: Win32 error {code}"
            ),
            Self::InitializeControls => formatter.write_str("failed to initialize export controls"),
        }
    }
}

impl Error for ExportOptionsDialogError {}

pub(crate) fn show_export_options_dialog(
    owner: HWND,
    defaults: ExportOptionsDialogDefaults,
    language: UiLanguage,
) -> Result<ExportOptionsDialogOutcome, ExportOptionsDialogError> {
    let instance = module_instance()?;
    let template = DialogTemplateBuffer::new(ui_text::export_dialog_title(language));
    let mut state = ExportOptionsDialogState {
        defaults,
        language,
        accepted_selection: None,
        init_failed: false,
        ui_font: None,
        syncing_size_fields: false,
        last_size_axis: SizeAxis::Width,
    };

    // SAFETY: template is an aligned in-memory DLGTEMPLATE that outlives the modal call.
    // state is a stack value kept alive for the entire DialogBoxIndirectParamW call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            instance,
            template.as_ptr(),
            owner,
            Some(export_options_dialog_proc),
            (&mut state as *mut ExportOptionsDialogState) as LPARAM,
        )
    };

    if state.init_failed || result == DIALOG_RESULT_INIT_FAILED {
        return Err(ExportOptionsDialogError::InitializeControls);
    }
    if result == -1 {
        return Err(ExportOptionsDialogError::CreateDialog { code: last_error() });
    }
    if result == IDOK as isize {
        if let Some(selection) = state.accepted_selection {
            Ok(ExportOptionsDialogOutcome::Accepted(selection))
        } else {
            Err(ExportOptionsDialogError::InitializeControls)
        }
    } else {
        Ok(ExportOptionsDialogOutcome::Cancelled)
    }
}

struct ExportOptionsDialogState {
    defaults: ExportOptionsDialogDefaults,
    language: UiLanguage,
    accepted_selection: Option<ExportOptionsDialogSelection>,
    init_failed: bool,
    ui_font: Option<dpi::DpiFont>,
    syncing_size_fields: bool,
    last_size_axis: SizeAxis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SizeAxis {
    Width,
    Height,
}

unsafe extern "system" fn export_options_dialog_proc(
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
        WM_COMMAND if handle_dialog_command(hwnd, wparam) => 1,
        WM_COMMAND => 0,
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

    let state_ptr = lparam as *mut ExportOptionsDialogState;
    if !set_dialog_state(hwnd, state_ptr) {
        // SAFETY: state_ptr comes from lparam and is valid during WM_INITDIALOG.
        unsafe {
            (*state_ptr).init_failed = true;
        }
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    }

    initialize_dialog_dpi(hwnd);
    let language = dialog_language(hwnd);
    if !resize_dialog_to_layout(hwnd) || !create_dialog_controls(hwnd, language) {
        if let Some(state) = dialog_state_mut(hwnd) {
            state.init_failed = true;
        }
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    }

    if let Some(state) = dialog_state_mut(hwnd) {
        populate_dialog(hwnd, state.defaults);
    }
}

fn initialize_dialog_dpi(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    let dpi_y = dpi::dpi_y_for_window(hwnd);
    state.ui_font = dpi::DpiFont::new_ui_font(DIALOG_FONT_POINT_SIZE, dpi_y);
}

fn handle_dialog_command(hwnd: HWND, wparam: WPARAM) -> bool {
    let command_id = i32::from(low_word(wparam));
    let notification = high_word(wparam);
    match command_id {
        IDOK => {
            handle_ok(hwnd);
            true
        }
        IDCANCEL => {
            close_dialog(hwnd, IDCANCEL as isize);
            true
        }
        ID_RESET_SIZE => {
            reset_size_fields(hwnd);
            true
        }
        ID_KEEP_ASPECT => {
            if is_checked(hwnd, ID_KEEP_ASPECT) {
                sync_size_fields_preserving_aspect(hwnd, current_size_axis(hwnd));
            }
            true
        }
        ID_WIDTH if notification == EN_CHANGE_CODE => {
            handle_size_edit_changed(hwnd, SizeAxis::Width);
            true
        }
        ID_HEIGHT if notification == EN_CHANGE_CODE => {
            handle_size_edit_changed(hwnd, SizeAxis::Height);
            true
        }
        ID_FORMAT if notification == CBN_SELCHANGE_CODE => {
            handle_format_changed(hwnd);
            true
        }
        ID_ROTATION if notification == CBN_SELCHANGE_CODE => {
            handle_rotation_changed(hwnd);
            true
        }
        _ => false,
    }
}

fn handle_ok(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
        return;
    };

    match read_dialog_selection(hwnd, state.defaults, state.last_size_axis, state.language) {
        Ok(selection) => {
            state.accepted_selection = Some(selection);
            close_dialog(hwnd, IDOK as isize);
        }
        Err(message) => show_warning_message(hwnd, &message),
    }
}

fn create_dialog_controls(hwnd: HWND, language: UiLanguage) -> bool {
    if create_group_box(
        hwnd,
        match language {
            UiLanguage::English => "File",
            UiLanguage::Korean => "파일",
        },
        GROUP_X,
        FILE_GROUP_Y,
        GROUP_WIDTH,
        group_height(4),
    )
    .is_none()
        || !create_group_field(
            hwnd,
            FILE_GROUP_Y,
            0,
            match language {
                UiLanguage::English => "Format",
                UiLanguage::Korean => "포맷",
            },
            ID_FORMAT,
            ControlKind::Combo,
        )
        || !create_group_field(
            hwnd,
            FILE_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "JPEG Quality",
                UiLanguage::Korean => "JPEG 품질",
            },
            ID_QUALITY,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            FILE_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Rotation",
                UiLanguage::Korean => "회전",
            },
            ID_ROTATION,
            ControlKind::Combo,
        )
        || create_checkbox(
            hwnd,
            ID_REMOVE_METADATA,
            match language {
                UiLanguage::English => "Remove Metadata",
                UiLanguage::Korean => "메타데이터 제거",
            },
            GROUP_X + GROUP_X_PADDING,
            group_row_y(FILE_GROUP_Y, 3),
            180,
            EDIT_HEIGHT,
        )
        .is_none()
    {
        return false;
    }

    if create_group_box(
        hwnd,
        match language {
            UiLanguage::English => "Size",
            UiLanguage::Korean => "크기",
        },
        GROUP_X,
        SIZE_GROUP_Y,
        GROUP_WIDTH,
        group_height(4),
    )
    .is_none()
        || create_label_with_id(
            hwnd,
            ID_ORIGINAL_SIZE,
            "",
            GROUP_X + GROUP_X_PADDING,
            group_row_y(SIZE_GROUP_Y, 0) + 3,
            GROUP_WIDTH - GROUP_X_PADDING * 2,
            EDIT_HEIGHT,
        )
        .is_none()
        || !create_group_field(
            hwnd,
            SIZE_GROUP_Y,
            1,
            match language {
                UiLanguage::English => "Width",
                UiLanguage::Korean => "너비",
            },
            ID_WIDTH,
            ControlKind::Edit,
        )
        || !create_group_field(
            hwnd,
            SIZE_GROUP_Y,
            2,
            match language {
                UiLanguage::English => "Height",
                UiLanguage::Korean => "높이",
            },
            ID_HEIGHT,
            ControlKind::Edit,
        )
        || create_checkbox(
            hwnd,
            ID_KEEP_ASPECT,
            match language {
                UiLanguage::English => "Keep Aspect Ratio",
                UiLanguage::Korean => "가로세로 비율 유지",
            },
            GROUP_X + GROUP_X_PADDING,
            group_row_y(SIZE_GROUP_Y, 3),
            180,
            EDIT_HEIGHT,
        )
        .is_none()
        || create_button(
            hwnd,
            ID_RESET_SIZE,
            match language {
                UiLanguage::English => "Original Size",
                UiLanguage::Korean => "원본 크기",
            },
            ControlRect::new(
                GROUP_X + GROUP_WIDTH - GROUP_X_PADDING - 90,
                group_row_y(SIZE_GROUP_Y, 3),
                90,
                24,
            ),
            ButtonKind::Normal,
        )
        .is_none()
    {
        return false;
    }

    let button_y = DIALOG_CLIENT_HEIGHT - 48;
    create_button(
        hwnd,
        IDOK,
        match language {
            UiLanguage::English => "OK",
            UiLanguage::Korean => "확인",
        },
        ControlRect::new(DIALOG_CLIENT_WIDTH - 196, button_y, 84, 28),
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
            ControlRect::new(DIALOG_CLIENT_WIDTH - 102, button_y, 84, 28),
            ButtonKind::Normal,
        )
        .is_some()
        && initialize_combo(hwnd, ID_FORMAT, FORMAT_OPTIONS)
        && initialize_combo_labels(hwnd, ID_ROTATION, ui_text::export_rotation_labels(language))
}

fn populate_dialog(hwnd: HWND, defaults: ExportOptionsDialogDefaults) {
    set_combo_selection(hwnd, ID_FORMAT, format_index(defaults.default_format));
    set_combo_selection(hwnd, ID_ROTATION, rotation_index(defaults.default_rotation));
    set_control_text(hwnd, ID_QUALITY, &defaults.default_quality.to_string());
    reset_size_fields(hwnd);
    set_checkbox(hwnd, ID_KEEP_ASPECT, true);
    set_checkbox(hwnd, ID_REMOVE_METADATA, defaults.default_remove_metadata);
    update_format_dependent_controls(hwnd);
}

fn reset_size_fields(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    let size = effective_original_size(hwnd, state.defaults);
    state.syncing_size_fields = true;
    set_control_text(hwnd, ID_WIDTH, &size.width().to_string());
    set_control_text(hwnd, ID_HEIGHT, &size.height().to_string());
    state.syncing_size_fields = false;
    state.last_size_axis = SizeAxis::Width;
}

fn handle_size_edit_changed(hwnd: HWND, axis: SizeAxis) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    if state.syncing_size_fields {
        return;
    }
    state.last_size_axis = axis;
    if !is_checked(hwnd, ID_KEEP_ASPECT) {
        return;
    }
    sync_size_fields_preserving_aspect(hwnd, axis);
}

fn sync_size_fields_preserving_aspect(hwnd: HWND, axis: SizeAxis) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    if state.syncing_size_fields {
        return;
    }
    let Ok(text) = control_text(
        hwnd,
        match axis {
            SizeAxis::Width => ID_WIDTH,
            SizeAxis::Height => ID_HEIGHT,
        },
    ) else {
        return;
    };
    let Ok(value) = parse_positive_u32(&text, "", state.language) else {
        return;
    };
    let Some(size) = size_preserving_aspect(state.defaults.original_size, axis, value) else {
        return;
    };

    state.last_size_axis = axis;
    state.syncing_size_fields = true;
    match axis {
        SizeAxis::Width => {
            set_control_text(hwnd, ID_HEIGHT, &size.height().to_string());
        }
        SizeAxis::Height => {
            set_control_text(hwnd, ID_WIDTH, &size.width().to_string());
        }
    }
    state.syncing_size_fields = false;
}

fn handle_format_changed(hwnd: HWND) {
    update_format_dependent_controls(hwnd);
    if selected_format(hwnd) != Some(ExportFormat::Ico) {
        reset_size_fields(hwnd);
    }
}

fn update_format_dependent_controls(hwnd: HWND) {
    update_quality_enabled(hwnd);
    update_size_controls_enabled(hwnd);
    update_original_size_label(hwnd);
}

fn update_quality_enabled(hwnd: HWND) {
    let enabled = selected_format(hwnd) == Some(ExportFormat::Jpeg);
    set_control_enabled(hwnd, ID_QUALITY, enabled);
}

fn update_size_controls_enabled(hwnd: HWND) {
    let enabled = selected_format(hwnd) != Some(ExportFormat::Ico);
    for id in [ID_WIDTH, ID_HEIGHT, ID_RESET_SIZE, ID_KEEP_ASPECT] {
        set_control_enabled(hwnd, id, enabled);
    }
}

fn set_control_enabled(hwnd: HWND, id: i32, enabled: bool) {
    // SAFETY: hwnd is the dialog and id identifies a child control when initialization succeeds.
    // EnableWindow tolerates a null hwnd, but the explicit check keeps intent clear.
    let control = unsafe { GetDlgItem(hwnd, id) };
    if control.is_null() {
        return;
    }
    // SAFETY: control is a live child window and the BOOL value only changes user input state.
    unsafe {
        EnableWindow(control, if enabled { 1 } else { 0 });
    }
}

fn handle_rotation_changed(hwnd: HWND) {
    update_original_size_label(hwnd);
    reset_size_fields(hwnd);
}

fn update_original_size_label(hwnd: HWND) {
    let Some(state) = dialog_state_mut(hwnd) else {
        return;
    };
    if selected_format(hwnd) == Some(ExportFormat::Ico) {
        set_control_text(
            hwnd,
            ID_ORIGINAL_SIZE,
            match state.language {
                UiLanguage::English => "ICO sizes: 16, 32, 48, 256 px",
                UiLanguage::Korean => "ICO 크기: 16, 32, 48, 256 px",
            },
        );
        return;
    }
    let size = effective_original_size(hwnd, state.defaults);
    set_control_text(
        hwnd,
        ID_ORIGINAL_SIZE,
        &match state.language {
            UiLanguage::English => format!("Original size: {} x {}", size.width(), size.height()),
            UiLanguage::Korean => format!("원본 크기: {} x {}", size.width(), size.height()),
        },
    );
}

fn effective_original_size(hwnd: HWND, defaults: ExportOptionsDialogDefaults) -> ImageSize {
    defaults
        .original_size
        .with_rotation(selected_rotation(hwnd).unwrap_or(defaults.default_rotation))
}

fn current_size_axis(hwnd: HWND) -> SizeAxis {
    dialog_state_mut(hwnd)
        .map(|state| state.last_size_axis)
        .unwrap_or(SizeAxis::Width)
}

fn read_dialog_selection(
    hwnd: HWND,
    defaults: ExportOptionsDialogDefaults,
    last_size_axis: SizeAxis,
    language: UiLanguage,
) -> Result<ExportOptionsDialogSelection, String> {
    let format = selected_format(hwnd).unwrap_or(defaults.default_format);
    let rotation = selected_rotation(hwnd).unwrap_or(defaults.default_rotation);
    let original_size = defaults.original_size.with_rotation(rotation);
    let quality = if format == ExportFormat::Jpeg {
        Some(read_quality(hwnd, language)?)
    } else {
        None
    };

    let target_size = if format == ExportFormat::Ico {
        None
    } else {
        let size = read_target_size(
            hwnd,
            original_size,
            last_size_axis,
            is_checked(hwnd, ID_KEEP_ASPECT),
            language,
        )?;
        (size != original_size).then_some(size)
    };

    Ok(ExportOptionsDialogSelection {
        format,
        quality,
        rotation,
        target_size,
        remove_metadata: is_checked(hwnd, ID_REMOVE_METADATA),
    })
}

fn read_quality(hwnd: HWND, language: UiLanguage) -> Result<u8, String> {
    let text = control_text(hwnd, ID_QUALITY)?;
    let label = match language {
        UiLanguage::English => "JPEG quality",
        UiLanguage::Korean => "JPEG 품질",
    };
    let value = parse_positive_u32(&text, label, language)?;
    let range = export_quality_range(ExportFormat::Jpeg).ok_or_else(|| {
        match language {
            UiLanguage::English => "Could not determine the JPEG quality range.",
            UiLanguage::Korean => "JPEG 품질 범위를 확인할 수 없습니다.",
        }
        .to_owned()
    })?;
    if value < u32::from(range.min()) || value > u32::from(range.max()) {
        return Err(match language {
            UiLanguage::English => {
                format!(
                    "JPEG quality must be between {} and {}.",
                    range.min(),
                    range.max()
                )
            }
            UiLanguage::Korean => {
                format!(
                    "JPEG 품질은 {}부터 {} 사이여야 합니다.",
                    range.min(),
                    range.max()
                )
            }
        });
    }
    u8::try_from(value).map_err(|_| {
        match language {
            UiLanguage::English => "Could not apply the JPEG quality value.",
            UiLanguage::Korean => "JPEG 품질 값을 적용할 수 없습니다.",
        }
        .to_owned()
    })
}

fn read_target_size(
    hwnd: HWND,
    original_size: ImageSize,
    last_size_axis: SizeAxis,
    keep_aspect: bool,
    language: UiLanguage,
) -> Result<ImageSize, String> {
    let width = parse_positive_u32(
        &control_text(hwnd, ID_WIDTH)?,
        match language {
            UiLanguage::English => "Width",
            UiLanguage::Korean => "너비",
        },
        language,
    )?;
    let height = parse_positive_u32(
        &control_text(hwnd, ID_HEIGHT)?,
        match language {
            UiLanguage::English => "Height",
            UiLanguage::Korean => "높이",
        },
        language,
    )?;
    let size = if keep_aspect {
        let axis_value = match last_size_axis {
            SizeAxis::Width => width,
            SizeAxis::Height => height,
        };
        size_preserving_aspect(original_size, last_size_axis, axis_value).ok_or_else(|| {
            match language {
                UiLanguage::English => "Could not calculate the export size.",
                UiLanguage::Korean => "내보내기 크기를 계산할 수 없습니다.",
            }
            .to_owned()
        })?
    } else {
        ImageSize::new(width, height)
    };
    let pixel_count = size.pixel_count().ok_or_else(|| {
        match language {
            UiLanguage::English => "The export size is too large.",
            UiLanguage::Korean => "내보내기 크기가 너무 큽니다.",
        }
        .to_owned()
    })?;
    if pixel_count > MAX_CONFIG_IMAGE_PIXELS {
        return Err(match language {
            UiLanguage::English => format!(
                "The export size can be at most {} pixels.",
                MAX_CONFIG_IMAGE_PIXELS
            ),
            UiLanguage::Korean => format!(
                "내보내기 크기는 최대 {} 픽셀까지 가능합니다.",
                MAX_CONFIG_IMAGE_PIXELS
            ),
        });
    }
    Ok(size)
}

fn parse_positive_u32(text: &str, label: &str, language: UiLanguage) -> Result<u32, String> {
    let trimmed = text.trim();
    let value = trimmed
        .parse::<u64>()
        .map_err(|_| parse_number_message(label, language))?;
    if value == 0 || value > u64::from(u32::MAX) {
        return Err(parse_number_message(label, language));
    }
    u32::try_from(value).map_err(|_| parse_number_message(label, language))
}

fn parse_number_message(label: &str, language: UiLanguage) -> String {
    match (language, label.is_empty()) {
        (UiLanguage::English, true) => "Value must be an integer of 1 or greater.".to_owned(),
        (UiLanguage::English, false) => {
            format!("{label} must be an integer of 1 or greater.")
        }
        (UiLanguage::Korean, true) => "값은 1 이상의 정수여야 합니다.".to_owned(),
        (UiLanguage::Korean, false) => format!("{label} 값은 1 이상의 정수여야 합니다."),
    }
}

fn size_preserving_aspect(
    original_size: ImageSize,
    axis: SizeAxis,
    value: u32,
) -> Option<ImageSize> {
    match axis {
        SizeAxis::Width => export_size_from_width_preserving_aspect(original_size, value),
        SizeAxis::Height => export_size_from_height_preserving_aspect(original_size, value),
    }
}

fn selected_format(hwnd: HWND) -> Option<ExportFormat> {
    combo_selection(hwnd, ID_FORMAT).and_then(format_for_index)
}

fn selected_rotation(hwnd: HWND) -> Option<ImageRotation> {
    combo_selection(hwnd, ID_ROTATION).and_then(rotation_for_index)
}

fn format_index(format: ExportFormat) -> usize {
    FORMAT_OPTIONS
        .iter()
        .position(|(candidate, _)| *candidate == format)
        .unwrap_or(0)
}

fn rotation_index(rotation: ImageRotation) -> usize {
    ROTATION_OPTIONS
        .iter()
        .position(|(candidate, _)| *candidate == rotation)
        .unwrap_or(0)
}

fn format_for_index(index: usize) -> Option<ExportFormat> {
    FORMAT_OPTIONS.get(index).map(|(format, _)| *format)
}

fn rotation_for_index(index: usize) -> Option<ImageRotation> {
    ROTATION_OPTIONS.get(index).map(|(rotation, _)| *rotation)
}

fn create_group_field(
    parent: HWND,
    group_y: i32,
    row: i32,
    label: &str,
    id: i32,
    kind: ControlKind,
) -> bool {
    let y = group_row_y(group_y, row);
    let label_x = GROUP_X + GROUP_X_PADDING;
    let input_x = label_x + LABEL_WIDTH;
    if create_label(parent, label, label_x, y + 3, LABEL_WIDTH - 4, EDIT_HEIGHT).is_none() {
        return false;
    }
    match kind {
        ControlKind::Combo => create_combo(parent, id, input_x, y, INPUT_WIDTH, COMBO_HEIGHT),
        ControlKind::Edit => create_edit(parent, id, input_x, y, INPUT_WIDTH, EDIT_HEIGHT),
    }
    .is_some()
}

fn group_height(row_count: i32) -> i32 {
    GROUP_TOP_PADDING + GROUP_ROW_HEIGHT * row_count + 12
}

fn group_row_y(group_y: i32, row: i32) -> i32 {
    group_y + GROUP_TOP_PADDING + GROUP_ROW_HEIGHT * row
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
    create_label_with_id(parent, 0, text, x, y, width, height)
}

fn create_label_with_id(
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
        "STATIC",
        text,
        id,
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

fn initialize_combo<T>(hwnd: HWND, id: i32, options: &[(T, &str)]) -> bool {
    for (_, label) in options {
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

fn initialize_combo_labels(hwnd: HWND, id: i32, labels: &[&str]) -> bool {
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
        return Err("내보내기 옵션을 읽을 수 없습니다.".to_owned());
    }

    // SAFETY: control is a live child window and the call only reads its text length.
    let length = unsafe { GetWindowTextLengthW(control) };
    if length < 0 {
        return Err("내보내기 옵션을 읽을 수 없습니다.".to_owned());
    }
    let capacity = length.saturating_add(1);
    let Ok(capacity) = usize::try_from(capacity) else {
        return Err("내보내기 옵션 값이 너무 깁니다.".to_owned());
    };
    let mut buffer = vec![0u16; capacity];
    let max_count = match i32::try_from(buffer.len()) {
        Ok(value) => value,
        Err(_) => return Err("내보내기 옵션 값이 너무 깁니다.".to_owned()),
    };
    // SAFETY: buffer has max_count UTF-16 code units and is writable.
    let copied = unsafe { GetWindowTextW(control, buffer.as_mut_ptr(), max_count) };
    if copied < 0 {
        return Err("내보내기 옵션을 읽을 수 없습니다.".to_owned());
    }
    let Ok(copied) = usize::try_from(copied) else {
        return Err("내보내기 옵션 값이 너무 깁니다.".to_owned());
    };
    Ok(String::from_utf16_lossy(&buffer[..copied]))
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

    // SAFETY: hwnd is the live modal dialog during WM_INITDIALOG.
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

fn set_dialog_state(hwnd: HWND, state: *mut ExportOptionsDialogState) -> bool {
    if state.is_null() {
        return false;
    }
    let name = wide_null(STATE_PROP_NAME);
    // SAFETY: state is valid for the modal dialog lifetime and stored as an opaque handle.
    unsafe { SetPropW(hwnd, name.as_ptr(), state as HANDLE) != 0 }
}

fn dialog_state_mut(hwnd: HWND) -> Option<&'static mut ExportOptionsDialogState> {
    let name = wide_null(STATE_PROP_NAME);
    // SAFETY: The property, when present, is written only by set_dialog_state with this type.
    let handle = unsafe { GetPropW(hwnd, name.as_ptr()) };
    if handle.is_null() {
        None
    } else {
        // SAFETY: DialogBoxIndirectParamW keeps the pointed state alive until the dialog closes.
        Some(unsafe { &mut *(handle as *mut ExportOptionsDialogState) })
    }
}

fn dialog_language(hwnd: HWND) -> UiLanguage {
    dialog_state_mut(hwnd)
        .map(|state| state.language)
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
    let title = wide_null(ui_text::export_dialog_title(dialog_language(hwnd)));
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

fn close_dialog(hwnd: HWND, result: isize) {
    // SAFETY: hwnd is the modal dialog window and result is an application-defined code.
    unsafe {
        EndDialog(hwnd, result);
    }
}

fn module_instance() -> Result<HINSTANCE, ExportOptionsDialogError> {
    // SAFETY: null asks for the current process module handle.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        Err(ExportOptionsDialogError::ModuleHandle { code: last_error() })
    } else {
        Ok(instance)
    }
}

fn low_word(value: WPARAM) -> u16 {
    (value & 0xffff) as u16
}

fn high_word(value: WPARAM) -> u16 {
    ((value >> 16) & 0xffff) as u16
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

    use windows_sys::Win32::UI::WindowsAndMessaging::FindWindowW;

    use super::*;

    #[test]
    #[ignore = "opens the native Win32 modal export options dialog and drives it with window messages"]
    fn export_options_dialog_accepts_valid_changes_from_win32_messages() {
        let (sender, receiver) = mpsc::channel();
        let defaults = ExportOptionsDialogDefaults::new(
            ImageSize::new(400, 200),
            ExportFormat::Png,
            90,
            false,
        );
        let dialog_thread = thread::spawn(move || {
            let outcome = show_export_options_dialog(null_mut(), defaults, UiLanguage::English);
            let _ = sender.send(outcome);
        });

        let hwnd = wait_for_export_options_dialog()
            .unwrap_or_else(|| panic!("export options dialog did not appear before timeout"));

        assert_export_options_dialog_controls_exist(hwnd);
        assert_export_options_dialog_client_size_matches_layout(hwnd);

        assert!(set_combo_selection(hwnd, ID_FORMAT, 1));
        send_dialog_command(hwnd, ID_FORMAT, CBN_SELCHANGE_CODE);
        assert!(set_combo_selection(hwnd, ID_ROTATION, 1));
        send_dialog_command(hwnd, ID_ROTATION, CBN_SELCHANGE_CODE);
        assert!(set_control_text(hwnd, ID_QUALITY, "88"));
        set_checkbox(hwnd, ID_REMOVE_METADATA, true);
        assert!(set_control_text(hwnd, ID_WIDTH, "100"));
        send_dialog_command(hwnd, ID_WIDTH, EN_CHANGE_CODE);

        // SAFETY: hwnd is the live export options dialog and WM_COMMAND/IDOK uses
        // the same path as a user pressing the default OK button.
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, IDOK as WPARAM, 0);
        }

        let outcome = receiver
            .recv_timeout(Duration::from_secs(5))
            .expect("export options dialog should close after OK");
        dialog_thread
            .join()
            .expect("export options dialog thread should finish");

        let ExportOptionsDialogOutcome::Accepted(selection) =
            outcome.expect("dialog should succeed")
        else {
            panic!("dialog should accept valid changes");
        };

        assert_eq!(selection.format(), ExportFormat::Jpeg);
        assert_eq!(selection.quality(), Some(88));
        assert_eq!(selection.rotation(), ImageRotation::Degrees90);
        assert_eq!(selection.target_size(), Some(ImageSize::new(100, 200)));
        assert!(selection.remove_metadata());
    }

    fn wait_for_export_options_dialog() -> Option<HWND> {
        let title = wide_null(ui_text::export_dialog_title(UiLanguage::English));
        let deadline = Instant::now() + Duration::from_secs(5);
        loop {
            // SAFETY: class name is null to match any class; title is null-terminated.
            let hwnd = unsafe { FindWindowW(null(), title.as_ptr()) };
            if !hwnd.is_null() && export_options_dialog_controls_are_ready(hwnd) {
                return Some(hwnd);
            }
            if Instant::now() >= deadline {
                return None;
            }
            thread::sleep(Duration::from_millis(20));
        }
    }

    fn export_options_dialog_controls_are_ready(hwnd: HWND) -> bool {
        // SAFETY: hwnd is a candidate export options dialog; GetDlgItem only looks up a child id.
        !unsafe { GetDlgItem(hwnd, ID_FORMAT) }.is_null()
            && !unsafe { GetDlgItem(hwnd, IDOK) }.is_null()
            && !unsafe { GetDlgItem(hwnd, IDCANCEL) }.is_null()
    }

    fn assert_export_options_dialog_controls_exist(hwnd: HWND) {
        for id in [
            ID_FORMAT,
            ID_QUALITY,
            ID_ORIGINAL_SIZE,
            ID_WIDTH,
            ID_HEIGHT,
            ID_RESET_SIZE,
            ID_KEEP_ASPECT,
            ID_REMOVE_METADATA,
            ID_ROTATION,
            IDOK,
            IDCANCEL,
        ] {
            // SAFETY: hwnd is the live export options dialog; GetDlgItem only looks up a child id.
            let control = unsafe { GetDlgItem(hwnd, id) };
            assert!(
                !control.is_null(),
                "missing export options dialog control {id}"
            );
        }
    }

    fn assert_export_options_dialog_client_size_matches_layout(hwnd: HWND) {
        let dpi_y = dpi::dpi_y_for_window(hwnd);
        let expected_width = dpi::scale_i32_for_dpi(DIALOG_CLIENT_WIDTH, dpi_y);
        let expected_height = dpi::scale_i32_for_dpi(DIALOG_CLIENT_HEIGHT, dpi_y);
        // SAFETY: RECT is a plain Win32 structure that GetClientRect fills.
        let mut client_rect: RECT = unsafe { std::mem::zeroed() };

        // SAFETY: hwnd is the live export options dialog and client_rect is writable storage.
        assert_ne!(unsafe { GetClientRect(hwnd, &mut client_rect) }, 0);
        assert_eq!(client_rect.right - client_rect.left, expected_width);
        assert_eq!(client_rect.bottom - client_rect.top, expected_height);
    }

    fn send_dialog_command(hwnd: HWND, id: i32, notification: u16) {
        let wparam = (id as u16 as WPARAM) | ((notification as WPARAM) << 16);
        // SAFETY: hwnd is the live export options dialog; the command id and
        // notification code mirror Win32 child-control notifications.
        unsafe {
            SendMessageW(hwnd, WM_COMMAND, wparam, 0);
        }
    }
}
