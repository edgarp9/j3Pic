use std::error::Error;
use std::ffi::OsStr;
use std::fmt;
use std::iter;
use std::os::windows::ffi::OsStrExt;
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{GetLastError, HINSTANCE, HWND, LPARAM, RECT, WPARAM};
use windows_sys::Win32::Graphics::Gdi::{GetStockObject, DEFAULT_GUI_FONT};
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::UI::Controls::{
    InitCommonControlsEx, EM_SETLIMITTEXT, ICC_LINK_CLASS, INITCOMMONCONTROLSEX, NMHDR, NM_CLICK,
    NM_RETURN,
};
use windows_sys::Win32::UI::Shell::ShellExecuteW;
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DialogBoxIndirectParamW, EndDialog, GetClientRect, GetWindow, GetWindowRect,
    MessageBoxW, SetWindowPos, SetWindowTextW, BS_DEFPUSHBUTTON, DLGTEMPLATE, DS_CENTER,
    DS_MODALFRAME, ES_AUTOVSCROLL, ES_MULTILINE, ES_READONLY, ES_WANTRETURN, GW_OWNER, HMENU,
    IDCANCEL, IDOK, MB_ICONWARNING, MB_OK, SWP_NOACTIVATE, SWP_NOOWNERZORDER, SWP_NOZORDER,
    SW_SHOWNORMAL, WM_COMMAND, WM_INITDIALOG, WM_NOTIFY, WM_SETFONT, WS_BORDER, WS_CAPTION,
    WS_CHILD, WS_POPUP, WS_SYSMENU, WS_TABSTOP, WS_VISIBLE, WS_VSCROLL,
};

use crate::about;
use crate::domain::UiLanguage;

use super::{win32::dpi, PROJECT_LINK_URL};

const DIALOG_RESULT_INIT_FAILED: isize = -2;
const ID_LICENSE_TEXT: i32 = 201;
const ID_VERSION_LABEL: i32 = 202;
const ID_SOURCE_LINK: i32 = 203;
const ID_LICENSES_LABEL: i32 = 204;

const DIALOG_TEMPLATE_WIDTH: i16 = 420;
const DIALOG_TEMPLATE_HEIGHT: i16 = 320;
const DIALOG_CLIENT_WIDTH: i32 = 450;
const DIALOG_CLIENT_HEIGHT: i32 = 400;
const MARGIN: i32 = 16;
const VERSION_LABEL_HEIGHT: i32 = 24;
const SOURCE_LINK_HEIGHT: i32 = 24;
const LABEL_HEIGHT: i32 = 22;
const BUTTON_WIDTH: i32 = 86;
const BUTTON_HEIGHT: i32 = 26;
const BUTTON_TOP_MARGIN: i32 = 12;
const TEMPLATE_BUFFER_BYTES: usize = 256;
const STATIC_NOTIFY_STYLE: u32 = 0x0000_0100;

#[derive(Debug)]
pub(crate) enum AboutDialogError {
    ModuleHandle { code: u32 },
    CreateDialog { code: u32 },
    InitializeControls,
}

impl fmt::Display for AboutDialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleHandle { code } => {
                write!(
                    formatter,
                    "failed to get module handle for about dialog: Win32 error {code}"
                )
            }
            Self::CreateDialog { code } => {
                write!(
                    formatter,
                    "failed to create about dialog: Win32 error {code}"
                )
            }
            Self::InitializeControls => formatter.write_str("failed to initialize about controls"),
        }
    }
}

impl Error for AboutDialogError {}

pub(crate) fn show_about_dialog(owner: HWND, language: UiLanguage) -> Result<(), AboutDialogError> {
    let instance = module_instance()?;
    let template = DialogTemplateBuffer::new(about::about_title(language));
    let text = about::about_license_text(language).replace('\n', "\r\n");
    let state = AboutDialogState { language, text };

    // SAFETY: template and state live for the entire modal call.
    let result = unsafe {
        DialogBoxIndirectParamW(
            instance,
            template.as_ptr(),
            owner,
            Some(about_dialog_proc),
            (&state as *const AboutDialogState) as LPARAM,
        )
    };

    if result == DIALOG_RESULT_INIT_FAILED {
        Err(AboutDialogError::InitializeControls)
    } else if result == -1 {
        Err(AboutDialogError::CreateDialog { code: last_error() })
    } else {
        Ok(())
    }
}

struct AboutDialogState {
    language: UiLanguage,
    text: String,
}

unsafe extern "system" fn about_dialog_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> isize {
    match message {
        WM_INITDIALOG => {
            if initialize_about_dialog(hwnd, lparam) {
                1
            } else {
                close_dialog(hwnd, DIALOG_RESULT_INIT_FAILED);
                1
            }
        }
        WM_COMMAND if low_word(wparam) == ID_SOURCE_LINK as u16 => {
            open_source_link(hwnd);
            1
        }
        WM_COMMAND if low_word(wparam) == IDOK as u16 || low_word(wparam) == IDCANCEL as u16 => {
            close_dialog(hwnd, IDOK as isize);
            1
        }
        WM_NOTIFY if handle_about_notify(hwnd, lparam) => 1,
        _ => 0,
    }
}

fn initialize_about_dialog(hwnd: HWND, lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }
    let state = unsafe { &*(lparam as *const AboutDialogState) };
    resize_dialog_to_layout(hwnd);
    create_about_controls(hwnd, state)
}

fn resize_dialog_to_layout(hwnd: HWND) {
    let dpi_y = dpi::dpi_y_for_window(hwnd);
    let desired_width = dpi::scale_i32_for_dpi(DIALOG_CLIENT_WIDTH, dpi_y);
    let desired_height = dpi::scale_i32_for_dpi(DIALOG_CLIENT_HEIGHT, dpi_y);
    // SAFETY: RECT is a plain Win32 structure filled by the following API calls.
    let mut client_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: RECT is a plain Win32 structure filled by the following API calls.
    let mut window_rect: RECT = unsafe { std::mem::zeroed() };

    // SAFETY: hwnd is the live dialog and both RECT values are writable storage.
    let ok = unsafe {
        GetClientRect(hwnd, &mut client_rect) != 0 && GetWindowRect(hwnd, &mut window_rect) != 0
    };
    if !ok {
        return;
    }

    let frame_width =
        (window_rect.right - window_rect.left) - (client_rect.right - client_rect.left);
    let frame_height =
        (window_rect.bottom - window_rect.top) - (client_rect.bottom - client_rect.top);
    let window_width = desired_width + frame_width;
    let window_height = desired_height + frame_height;
    let (x, y) = dialog_position_centered_on_owner(hwnd, window_rect, window_width, window_height);

    // SAFETY: hwnd is live; x/y/size are calculated from current dialog and owner rectangles.
    unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            x,
            y,
            window_width,
            window_height,
            SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_NOACTIVATE,
        );
    }
}

fn dialog_position_centered_on_owner(
    hwnd: HWND,
    fallback_rect: RECT,
    window_width: i32,
    window_height: i32,
) -> (i32, i32) {
    // SAFETY: RECT is a plain Win32 structure filled by GetWindowRect below.
    let mut owner_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is the live dialog; GW_OWNER reads its owner handle.
    let owner = unsafe { GetWindow(hwnd, GW_OWNER) };
    let target_rect = if !owner.is_null() {
        // SAFETY: owner is a live top-level owner window while the modal dialog is initializing.
        let has_owner_rect = unsafe { GetWindowRect(owner, &mut owner_rect) != 0 };
        if has_owner_rect {
            owner_rect
        } else {
            fallback_rect
        }
    } else {
        fallback_rect
    };

    centered_window_position(target_rect, window_width, window_height)
}

fn centered_window_position(
    target_rect: RECT,
    window_width: i32,
    window_height: i32,
) -> (i32, i32) {
    let target_width = target_rect.right - target_rect.left;
    let target_height = target_rect.bottom - target_rect.top;

    (
        target_rect.left + (target_width - window_width) / 2,
        target_rect.top + (target_height - window_height) / 2,
    )
}

fn create_about_controls(hwnd: HWND, state: &AboutDialogState) -> bool {
    let dpi_y = dpi::dpi_y_for_window(hwnd);
    let width = DIALOG_CLIENT_WIDTH;
    let height = DIALOG_CLIENT_HEIGHT;
    let content_width = width - MARGIN * 2;
    let source_link_top = MARGIN + VERSION_LABEL_HEIGHT;
    let licenses_label_top = source_link_top + SOURCE_LINK_HEIGHT;
    let edit_top = licenses_label_top + LABEL_HEIGHT;
    let edit_height = height - edit_top - BUTTON_TOP_MARGIN - BUTTON_HEIGHT - MARGIN;
    let button_x = width - MARGIN - BUTTON_WIDTH;
    let button_y = height - MARGIN - BUTTON_HEIGHT;

    let Some(_) = create_child_control(
        hwnd,
        "STATIC",
        &about::version_label(),
        ID_VERSION_LABEL,
        WS_CHILD | WS_VISIBLE,
        ControlRect::new(MARGIN, MARGIN, content_width, VERSION_LABEL_HEIGHT).scale_for_dpi(dpi_y),
    ) else {
        return false;
    };
    let Some(_) = create_source_link(
        hwnd,
        state.language,
        ControlRect::new(MARGIN, source_link_top, content_width, SOURCE_LINK_HEIGHT)
            .scale_for_dpi(dpi_y),
    ) else {
        return false;
    };
    let Some(_) = create_child_control(
        hwnd,
        "STATIC",
        about::licenses_label(state.language),
        ID_LICENSES_LABEL,
        WS_CHILD | WS_VISIBLE,
        ControlRect::new(MARGIN, licenses_label_top, content_width, LABEL_HEIGHT)
            .scale_for_dpi(dpi_y),
    ) else {
        return false;
    };
    let Some(edit) = create_child_control(
        hwnd,
        "EDIT",
        "",
        ID_LICENSE_TEXT,
        WS_CHILD
            | WS_VISIBLE
            | WS_TABSTOP
            | WS_BORDER
            | WS_VSCROLL
            | ES_MULTILINE as u32
            | ES_AUTOVSCROLL as u32
            | ES_READONLY as u32
            | ES_WANTRETURN as u32,
        ControlRect::new(MARGIN, edit_top, content_width, edit_height).scale_for_dpi(dpi_y),
    ) else {
        return false;
    };
    let Some(_) = create_child_control(
        hwnd,
        "BUTTON",
        match state.language {
            UiLanguage::English => "OK",
            UiLanguage::Korean => "확인",
        },
        IDOK,
        WS_CHILD | WS_VISIBLE | WS_TABSTOP | BS_DEFPUSHBUTTON as u32,
        ControlRect::new(button_x, button_y, BUTTON_WIDTH, BUTTON_HEIGHT).scale_for_dpi(dpi_y),
    ) else {
        return false;
    };

    let limit = state.text.encode_utf16().count().saturating_add(1);
    // SAFETY: edit is a live EDIT control; text is a null-terminated UTF-16 buffer.
    unsafe {
        windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(edit, EM_SETLIMITTEXT, limit, 0);
    }
    set_window_text(edit, &state.text)
}

fn create_source_link(parent: HWND, language: UiLanguage, rect: ControlRect) -> Option<HWND> {
    if init_link_controls() {
        let link_text = format!(
            "{} <A HREF=\"{PROJECT_LINK_URL}\">{PROJECT_LINK_URL}</A>",
            about::source_code_label(language)
        );
        if let Some(control) = create_child_control(
            parent,
            "SysLink",
            &link_text,
            ID_SOURCE_LINK,
            WS_CHILD | WS_VISIBLE | WS_TABSTOP,
            rect,
        ) {
            return Some(control);
        }
    }

    create_child_control(
        parent,
        "STATIC",
        &format!("{} {PROJECT_LINK_URL}", about::source_code_label(language)),
        ID_SOURCE_LINK,
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

fn handle_about_notify(hwnd: HWND, lparam: LPARAM) -> bool {
    if lparam == 0 {
        return false;
    }

    // SAFETY: WM_NOTIFY provides an NMHDR-compatible pointer for this dispatch.
    let header = unsafe { &*(lparam as *const NMHDR) };
    if header.idFrom == ID_SOURCE_LINK as usize
        && (header.code == NM_CLICK || header.code == NM_RETURN)
    {
        open_source_link(hwnd);
        true
    } else {
        false
    }
}

fn open_source_link(hwnd: HWND) {
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
        let message = wide_null("Could not open the source code link.");
        let title = wide_null("j3Pic");
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
    // SAFETY: class_name and text are null-terminated UTF-16 buffers valid for the call.
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
        apply_dialog_font(hwnd);
        Some(hwnd)
    }
}

fn apply_dialog_font(hwnd: HWND) {
    // SAFETY: DEFAULT_GUI_FONT is a process-owned stock object and remains valid.
    let font = unsafe { GetStockObject(DEFAULT_GUI_FONT) };
    if !font.is_null() {
        // SAFETY: hwnd is a live control and font is a stock GUI font handle.
        unsafe {
            windows_sys::Win32::UI::WindowsAndMessaging::SendMessageW(
                hwnd,
                WM_SETFONT,
                font as WPARAM,
                1,
            );
        }
    }
}

fn set_window_text(hwnd: HWND, text: &str) -> bool {
    let text = wide_null(text);
    // SAFETY: hwnd is live and text is null-terminated.
    unsafe { SetWindowTextW(hwnd, text.as_ptr()) != 0 }
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

fn close_dialog(hwnd: HWND, result: isize) {
    // SAFETY: hwnd is the modal dialog window and result is application-defined.
    unsafe {
        EndDialog(hwnd, result);
    }
}

fn module_instance() -> Result<HINSTANCE, AboutDialogError> {
    // SAFETY: null asks for the current process module handle.
    let instance = unsafe { GetModuleHandleW(null()) };
    if instance.is_null() {
        Err(AboutDialogError::ModuleHandle { code: last_error() })
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
