use std::error::Error;
use std::ffi::{c_void, OsStr, OsString};
use std::fmt;
use std::iter;
use std::os::windows::ffi::{OsStrExt, OsStringExt};
use std::path::{Path, PathBuf};
use std::ptr::{null, null_mut};

use windows_sys::Win32::Foundation::{
    GetLastError, GlobalFree, SetLastError, ERROR_CLASS_ALREADY_EXISTS, ERROR_SUCCESS, HGLOBAL,
    HINSTANCE, HWND, LPARAM, LRESULT, POINT, RECT, WPARAM,
};
use windows_sys::Win32::Graphics::Gdi::{
    BeginPaint, BitBlt, CreateCompatibleBitmap, CreateCompatibleDC, DeleteDC, DeleteObject,
    DrawTextW, EndPaint, FillRect, GetDeviceCaps, GetMonitorInfoW, GetSysColor, GetSysColorBrush,
    InvalidateRect, MonitorFromWindow, ScreenToClient, SelectObject, SetBkMode, SetBrushOrgEx,
    SetStretchBltMode, SetTextColor, StretchDIBits, UpdateWindow, BITMAPINFO, BITSPIXEL, BI_RGB,
    COLORONCOLOR, COLOR_3DFACE, COLOR_WINDOW, COLOR_WINDOWTEXT, DIB_RGB_COLORS, DT_END_ELLIPSIS,
    DT_LEFT, DT_NOPREFIX, DT_SINGLELINE, DT_VCENTER, HALFTONE, HBITMAP, HDC, HGDIOBJ, MONITORINFO,
    MONITOR_DEFAULTTONEAREST, PAINTSTRUCT, PLANES, SRCCOPY, TRANSPARENT,
};
use windows_sys::Win32::System::DataExchange::{
    CloseClipboard, EmptyClipboard, OpenClipboard, SetClipboardData,
};
use windows_sys::Win32::System::Diagnostics::Debug::OutputDebugStringW;
use windows_sys::Win32::System::LibraryLoader::GetModuleHandleW;
use windows_sys::Win32::System::Memory::{GlobalAlloc, GlobalLock, GlobalUnlock, GMEM_MOVEABLE};
use windows_sys::Win32::UI::Controls::Dialogs::{
    CommDlgExtendedError, GetOpenFileNameW, GetSaveFileNameW, OFN_EXPLORER, OFN_FILEMUSTEXIST,
    OFN_HIDEREADONLY, OFN_NOCHANGEDIR, OFN_OVERWRITEPROMPT, OFN_PATHMUSTEXIST, OPENFILENAMEW,
};
use windows_sys::Win32::UI::Input::KeyboardAndMouse::{
    GetCapture, GetKeyState, ReleaseCapture, SetCapture, VK_BACK, VK_CONTROL, VK_ESCAPE, VK_F11,
    VK_F4, VK_HOME, VK_LEFT, VK_MENU, VK_NEXT, VK_PRIOR, VK_RETURN, VK_RIGHT, VK_SHIFT, VK_SPACE,
};
use windows_sys::Win32::UI::Shell::{DragAcceptFiles, DragFinish, DragQueryFileW, HDROP};
use windows_sys::Win32::UI::WindowsAndMessaging::{
    CreateWindowExW, DefWindowProcW, DestroyWindow, DispatchMessageW, GetClientRect, GetCursorPos,
    GetMessageW, GetSystemMetrics, GetWindowLongPtrW, GetWindowPlacement, GetWindowThreadProcessId,
    KillTimer, LoadCursorW, LoadImageW, MessageBoxW, PostMessageW, PostQuitMessage,
    PostThreadMessageW, RegisterClassW, SendMessageW, SetTimer, SetWindowLongPtrW,
    SetWindowPlacement, SetWindowPos, SetWindowTextW, ShowWindow, TranslateMessage, CREATESTRUCTW,
    CS_HREDRAW, CS_VREDRAW, CW_USEDEFAULT, GWLP_USERDATA, GWL_STYLE, HICON, HTCAPTION, ICON_BIG,
    ICON_SMALL, ICON_SMALL2, IDC_ARROW, IDYES, IMAGE_ICON, LR_DEFAULTCOLOR, LR_SHARED,
    MB_ICONERROR, MB_ICONWARNING, MB_OK, MB_YESNO, MSG, SM_CXICON, SM_CXSMICON, SM_CYICON,
    SM_CYSMICON, SWP_FRAMECHANGED, SWP_NOMOVE, SWP_NOOWNERZORDER, SWP_NOSIZE, SWP_NOZORDER,
    SW_SHOW, WINDOWPLACEMENT, WM_APP, WM_CANCELMODE, WM_CAPTURECHANGED, WM_COMMAND, WM_CONTEXTMENU,
    WM_CREATE, WM_DESTROY, WM_DPICHANGED, WM_DROPFILES, WM_ENTERSIZEMOVE, WM_ERASEBKGND,
    WM_EXITSIZEMOVE, WM_KEYDOWN, WM_LBUTTONDOWN, WM_LBUTTONUP, WM_MOUSEMOVE, WM_MOUSEWHEEL,
    WM_NCCREATE, WM_NCDESTROY, WM_NCLBUTTONDOWN, WM_PAINT, WM_QUIT, WM_SETICON, WM_SIZE,
    WM_SYSKEYDOWN, WM_TIMER, WNDCLASSW, WS_OVERLAPPEDWINDOW,
};

use super::about_dialog::show_about_dialog;
use super::export_options_dialog::{
    show_export_options_dialog, ExportOptionsDialogDefaults, ExportOptionsDialogOutcome,
};
use super::settings_dialog::{show_settings_dialog, SettingsDialogOutcome};
use super::{
    corrected_export_overwrite_message, corrected_export_path_requires_overwrite_confirmation,
    paths_refer_to_same_existing_file, same_source_export_message, ExportFileSelection,
};

use crate::app::{
    AnimationFrameDecodeRequest, AnimationFrameOutcome, AppCommandOutcome, DecodeApplyOutcome,
    DecodeFailurePresentation, ImageDecodeRequest, ImagePreloadRequest, NavigationStartOutcome,
    RenderImage, RenderImageCacheKey, ViewerApp, ViewerAppError,
};
#[cfg(test)]
use crate::domain::Rgba8Image;
use crate::domain::{
    command_for_key_input_with_context, export_format_default_extension,
    export_path_with_format_extension, AnimationCommand, Command, CommandContext, DecodeGeneration,
    ExportFormat, ExportOptions, ImageFileVersion, ImageFolder, ImageLoadFailureStage,
    ImageNavigationDirection, ImageOrientation, ImageSize, KeyCode, KeyInput, KeyModifiers,
    MouseShortcut, PixelImage, ScalingQuality, SupportedImageFormat, UiLanguage, ViewportPoint,
    ViewportSize, WindowBounds,
};
use crate::infra::{
    cached_animation_frame_pixels_for_loaded_image, save_app_config, AnimationFramePixels,
    LoadImageError, ScanImageFolderError,
};

pub(crate) mod dpi;

use self::decode_worker::{
    DecodeController, DecodeNotificationOutcome, DecodeStartFailure, DecodeWorkerMessage,
};
#[cfg(test)]
use self::decode_worker::{
    DecodeWorker, DecodeWorkerKind, FolderScanPermit, PendingDecodeRequest,
    MAX_IN_FLIGHT_DECODE_WORKERS, MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS,
};
use self::export_worker::{
    ExportController, ExportShutdownOutcome, ExportStartOutcome, ExportWorkerMessage,
};

const WINDOW_CLASS_NAME: &str = "j3pic.viewer.window";
const APP_ICON_RESOURCE_ID: u16 = 1;
const INITIAL_WIDTH: i32 = 960;
const INITIAL_HEIGHT: i32 = 640;
const OPEN_FILE_BUFFER_CHARS: usize = 32_768;
const OPEN_FILE_FILTER_PATTERNS: &str =
    "*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga";
const DROP_QUERY_FILE_COUNT: u32 = u32::MAX;
const MAX_DROPPED_FILE_PATHS_TO_SCAN: u32 = 20_000;
const DROP_EXTENSION_ASCII_BUFFER_CHARS: usize = 16;
const DIB_BYTES_PER_PIXEL: usize = 4;
const BITMAPINFOHEADER_SIZE: usize = 40;
// windows-sys exposes the clipboard functions used here, but not this legacy
// clipboard format constant. Win32 defines CF_DIB as 8.
const CF_DIB_FORMAT: u32 = 8;
const WHEEL_DELTA_UNITS: f64 = 120.0;
const STATUS_BAR_HEIGHT: i32 = 28;
const STATUS_TEXT_HORIZONTAL_PADDING: i32 = 10;
const STATUS_FONT_POINT_SIZE: i32 = 9;
const KEY_0: u16 = b'0' as u16;
const KEY_1: u16 = b'1' as u16;
const KEY_C: u16 = b'C' as u16;
const KEY_O: u16 = b'O' as u16;
const KEY_P: u16 = b'P' as u16;
const KEY_Q: u16 = b'Q' as u16;
const KEY_R: u16 = b'R' as u16;
const KEY_S: u16 = b'S' as u16;
const VK_ADD_KEY: u16 = 0x6b;
const VK_SUBTRACT_KEY: u16 = 0x6d;
const VK_OEM_PLUS_KEY: u16 = 0xbb;
const VK_OEM_MINUS_KEY: u16 = 0xbd;
const VK_OEM_4_KEY: u16 = 0xdb;
const VK_OEM_6_KEY: u16 = 0xdd;
const WM_IMAGE_DECODED: u32 = WM_APP + 1;
const WM_IMAGE_EXPORTED: u32 = WM_APP + 2;
const WM_OPEN_STARTUP_IMAGE: u32 = WM_APP + 3;
const ANIMATION_TIMER_ID: usize = 1;
const ANIMATION_DEBUG_LOG_ENV: &str = "J3PIC_ANIMATION_DEBUG";
const DECODE_NOTIFICATION_TIMER_ID: usize = 2;
const EXPORT_NOTIFICATION_TIMER_ID: usize = 3;
const INTERACTIVE_RENDER_SETTLE_TIMER_ID: usize = 4;
const DECODE_NOTIFICATION_TIMER_INTERVAL_MS: u32 = 10;
const EXPORT_NOTIFICATION_TIMER_INTERVAL_MS: u32 = 10;
const INTERACTIVE_RENDER_SETTLE_TIMER_INTERVAL_MS: u32 = 50;
const UI_THREAD_QUIT_POST_ATTEMPTS: usize = 3;
const UI_THREAD_QUIT_POST_RETRY_DELAY_MS: u64 = 10;

pub fn run_native_viewer(
    app: ViewerApp,
    save_config_on_destroy: bool,
    startup_image_path: Option<PathBuf>,
) -> Result<i32, Win32Error> {
    dpi::enable_process_dpi_awareness();

    let instance = module_instance()?;
    let class_name = wide_null(WINDOW_CLASS_NAME);
    register_window_class(instance, class_name.as_ptr())?;

    let title = wide_null(app.title());
    let hwnd = create_main_window(
        instance,
        class_name.as_ptr(),
        title.as_ptr(),
        app,
        save_config_on_destroy,
        startup_image_path,
    )?;

    // SAFETY: hwnd is a live top-level window returned by CreateWindowExW.
    unsafe {
        ShowWindow(hwnd, SW_SHOW);
        UpdateWindow(hwnd);
    }

    message_loop()
}

pub fn show_startup_error_message(message: &str) {
    dpi::enable_process_dpi_awareness();
    show_error_message(null_mut(), message);
}

#[derive(Debug)]
pub enum Win32Error {
    ModuleHandle { code: u32 },
    RegisterClass { code: u32 },
    CreateWindow { code: u32 },
    MessageLoop { code: u32 },
}

impl fmt::Display for Win32Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ModuleHandle { code } => {
                write!(formatter, "failed to get module handle: Win32 error {code}")
            }
            Self::RegisterClass { code } => {
                write!(
                    formatter,
                    "failed to register window class: Win32 error {code}"
                )
            }
            Self::CreateWindow { code } => {
                write!(
                    formatter,
                    "failed to create main window: Win32 error {code}"
                )
            }
            Self::MessageLoop { code } => {
                write!(formatter, "message loop failed: Win32 error {code}")
            }
        }
    }
}

impl Error for Win32Error {}

struct WindowCreationContext {
    app: Option<ViewerApp>,
    save_config_on_destroy: bool,
    startup_image_path: Option<PathBuf>,
    attached_to_hwnd: bool,
}

struct WindowState {
    app: ViewerApp,
    save_config_on_destroy: bool,
    startup_image_path: Option<PathBuf>,
    decoder: DecodeController,
    exporter: ExportController,
    fullscreen: FullscreenState,
    ui_metrics: WindowUiMetrics,
    size_move_dpi: SizeMoveDpiState,
    paint_cache: PaintDibCache,
    paint_buffer: ReusableCompatiblePaintBuffer,
}

struct WindowUiMetrics {
    status_bar_height: i32,
    status_text_horizontal_padding: i32,
    status_font: Option<dpi::DpiFont>,
}

impl WindowUiMetrics {
    fn for_window(hwnd: HWND) -> Self {
        let dpi_y = dpi::dpi_y_for_window(hwnd);
        Self {
            status_bar_height: dpi::scale_i32_for_dpi(STATUS_BAR_HEIGHT, dpi_y).max(1),
            status_text_horizontal_padding: dpi::scale_i32_for_dpi(
                STATUS_TEXT_HORIZONTAL_PADDING,
                dpi_y,
            )
            .max(0),
            status_font: dpi::DpiFont::new_ui_font(STATUS_FONT_POINT_SIZE, dpi_y),
        }
    }
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SizeMoveDpiState {
    in_size_move_loop: bool,
    dpi_changed_during_size_move: bool,
    render_settle_pending_during_size_move: bool,
}

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
struct SizeMoveExitOutcome {
    dpi_changed: bool,
    render_settle_pending: bool,
}

impl SizeMoveDpiState {
    fn enter_size_move(&mut self) {
        self.in_size_move_loop = true;
        self.dpi_changed_during_size_move = false;
        self.render_settle_pending_during_size_move = false;
    }

    fn is_in_size_move_loop(self) -> bool {
        self.in_size_move_loop
    }

    fn should_apply_suggested_rect_for_dpi_change(&mut self) -> bool {
        if self.in_size_move_loop {
            self.dpi_changed_during_size_move = true;
            false
        } else {
            true
        }
    }

    fn should_defer_view_refresh(self) -> bool {
        self.in_size_move_loop && self.dpi_changed_during_size_move
    }

    fn defer_render_settle_until_exit(&mut self) -> bool {
        if !self.in_size_move_loop {
            return false;
        }

        self.render_settle_pending_during_size_move = true;
        true
    }

    fn exit_size_move(&mut self) -> SizeMoveExitOutcome {
        let outcome = SizeMoveExitOutcome {
            dpi_changed: self.dpi_changed_during_size_move,
            render_settle_pending: self.render_settle_pending_during_size_move,
        };
        self.in_size_move_loop = false;
        self.dpi_changed_during_size_move = false;
        self.render_settle_pending_during_size_move = false;
        outcome
    }
}

mod context_menu {
    use std::ptr::null;

    use windows_sys::Win32::Foundation::{HWND, LPARAM, POINT, RECT};
    use windows_sys::Win32::Graphics::Gdi::ClientToScreen;
    use windows_sys::Win32::UI::WindowsAndMessaging::{
        AppendMenuW, CreatePopupMenu, DestroyMenu, GetClientRect, PostMessageW,
        SetForegroundWindow, TrackPopupMenu, HMENU, MF_GRAYED, MF_SEPARATOR, MF_STRING,
        TPM_NONOTIFY, TPM_RETURNCMD, TPM_RIGHTBUTTON, WM_NULL,
    };

    use crate::domain::{Command, UiLanguage};

    use super::{signed_high_word, signed_low_word, wide_null};

    const CONTEXT_MENU_ID_OPEN_IMAGE: usize = 0x7101;
    const CONTEXT_MENU_ID_EXPORT_IMAGE: usize = 0x7102;
    const CONTEXT_MENU_ID_COPY_IMAGE: usize = 0x7103;
    const CONTEXT_MENU_ID_ACTUAL_SIZE: usize = 0x7104;
    const CONTEXT_MENU_ID_FIT_TO_WINDOW: usize = 0x7105;
    const CONTEXT_MENU_ID_ROTATE_CLOCKWISE: usize = 0x7106;
    const CONTEXT_MENU_ID_ROTATE_COUNTER_CLOCKWISE: usize = 0x7107;
    const CONTEXT_MENU_ID_TOGGLE_FULLSCREEN: usize = 0x7108;
    pub(super) const CONTEXT_MENU_ID_OPEN_SETTINGS: usize = 0x7109;
    pub(super) const CONTEXT_MENU_ID_OPEN_ABOUT: usize = 0x710a;

    #[derive(Clone, Copy)]
    pub(super) struct ContextMenuCommand {
        pub(super) id: usize,
        pub(super) command: Command,
        pub(super) requires_image: bool,
    }

    #[derive(Clone, Copy)]
    pub(super) enum ContextMenuEntry {
        Command(ContextMenuCommand),
        Separator,
    }

    pub(super) const CONTEXT_MENU_ENTRIES: &[ContextMenuEntry] = &[
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_OPEN_IMAGE,
            command: Command::OpenImage,
            requires_image: false,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_EXPORT_IMAGE,
            command: Command::ExportImage,
            requires_image: true,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_COPY_IMAGE,
            command: Command::CopyImageToClipboard,
            requires_image: true,
        }),
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_ACTUAL_SIZE,
            command: Command::ActualSize,
            requires_image: true,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_FIT_TO_WINDOW,
            command: Command::FitToWindow,
            requires_image: true,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_ROTATE_CLOCKWISE,
            command: Command::RotateClockwise,
            requires_image: true,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_ROTATE_COUNTER_CLOCKWISE,
            command: Command::RotateCounterClockwise,
            requires_image: true,
        }),
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_TOGGLE_FULLSCREEN,
            command: Command::ToggleFullscreen,
            requires_image: false,
        }),
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_OPEN_ABOUT,
            command: Command::OpenAbout,
            requires_image: false,
        }),
        ContextMenuEntry::Command(ContextMenuCommand {
            id: CONTEXT_MENU_ID_OPEN_SETTINGS,
            command: Command::OpenSettings,
            requires_image: false,
        }),
    ];

    pub(super) enum ContextMenuResult {
        Selected(Command),
        NoSelection,
        CreateFailed,
    }

    pub(super) fn show(
        hwnd: HWND,
        lparam: LPARAM,
        language: UiLanguage,
        has_image: impl FnOnce() -> bool,
    ) -> ContextMenuResult {
        let Some(point) = screen_point(hwnd, lparam) else {
            return ContextMenuResult::NoSelection;
        };
        let Some(menu) = create(language, has_image) else {
            return ContextMenuResult::CreateFailed;
        };

        // SAFETY: hwnd is the top-level owner of this popup. TrackPopupMenu expects
        // the owner to be foreground so mouse/keyboard selection reliably returns.
        unsafe {
            SetForegroundWindow(hwnd);
        }

        // SAFETY: menu is a live popup menu created for this call, hwnd owns the popup,
        // and point is in screen coordinates as required by TrackPopupMenu.
        let selected_id = unsafe {
            TrackPopupMenu(
                menu,
                TPM_RIGHTBUTTON | TPM_RETURNCMD | TPM_NONOTIFY,
                point.x,
                point.y,
                0,
                hwnd,
                null(),
            )
        };
        // SAFETY: menu was created by CreatePopupMenu and is no longer used after
        // TrackPopupMenu returns.
        unsafe {
            DestroyMenu(menu);
            let _ = PostMessageW(hwnd, WM_NULL, 0, 0);
        }

        if selected_id == 0 {
            return ContextMenuResult::NoSelection;
        }
        command_from_id(selected_id as usize)
            .map(ContextMenuResult::Selected)
            .unwrap_or(ContextMenuResult::NoSelection)
    }

    fn screen_point(hwnd: HWND, lparam: LPARAM) -> Option<POINT> {
        if lparam == -1 {
            return client_center_screen_point(hwnd);
        }

        Some(POINT {
            x: signed_low_word(lparam),
            y: signed_high_word(lparam),
        })
    }

    fn client_center_screen_point(hwnd: HWND) -> Option<POINT> {
        // SAFETY: RECT is a plain Win32 structure that GetClientRect fills.
        let mut client_rect: RECT = unsafe { std::mem::zeroed() };
        // SAFETY: hwnd is live and client_rect is valid writable storage.
        if unsafe { GetClientRect(hwnd, &mut client_rect) } == 0 {
            return None;
        }

        let mut point = POINT {
            x: client_rect.left + client_rect.right.saturating_sub(client_rect.left) / 2,
            y: client_rect.top + client_rect.bottom.saturating_sub(client_rect.top) / 2,
        };
        // SAFETY: point starts in hwnd client coordinates and is valid writable storage.
        if unsafe { ClientToScreen(hwnd, &mut point) } == 0 {
            None
        } else {
            Some(point)
        }
    }

    fn create(language: UiLanguage, has_image: impl FnOnce() -> bool) -> Option<HMENU> {
        // SAFETY: CreatePopupMenu creates a detached menu handle owned by this function
        // until it is returned to the caller or destroyed on failure.
        let menu = unsafe { CreatePopupMenu() };
        if menu.is_null() {
            return None;
        }

        let has_image = has_image();
        for entry in CONTEXT_MENU_ENTRIES {
            let appended = match entry {
                ContextMenuEntry::Command(item) => append_command(menu, *item, language, has_image),
                ContextMenuEntry::Separator => append_separator(menu),
            };
            if !appended {
                // SAFETY: menu is owned by this function and must be released on append failure.
                unsafe {
                    DestroyMenu(menu);
                }
                return None;
            }
        }

        Some(menu)
    }

    fn append_command(
        menu: HMENU,
        item: ContextMenuCommand,
        language: UiLanguage,
        has_image: bool,
    ) -> bool {
        let label = wide_null(crate::ui_text::context_menu_label(language, item.command));
        let flags = if item.requires_image && !has_image {
            MF_STRING | MF_GRAYED
        } else {
            MF_STRING
        };

        // SAFETY: menu is a live popup menu, label is a null-terminated UTF-16 buffer
        // valid for the duration of the call, and item.id is a viewer-owned menu id.
        unsafe { AppendMenuW(menu, flags, item.id, label.as_ptr()) != 0 }
    }

    fn append_separator(menu: HMENU) -> bool {
        // SAFETY: menu is a live popup menu and separators ignore the id and text pointer.
        unsafe { AppendMenuW(menu, MF_SEPARATOR, 0, null()) != 0 }
    }

    pub(super) fn command_from_id(id: usize) -> Option<Command> {
        CONTEXT_MENU_ENTRIES.iter().find_map(|entry| match entry {
            ContextMenuEntry::Command(item) if item.id == id => Some(item.command),
            ContextMenuEntry::Command(_) | ContextMenuEntry::Separator => None,
        })
    }
}

mod decode_worker {
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::{self, JoinHandle};

    use windows_sys::Win32::Foundation::HWND;

    use crate::app::{
        AnimationFrameDecodeRequest, ImageDecodePurpose, ImageDecodeRequest, ImagePreloadRequest,
        ViewerAppError,
    };
    use crate::domain::{
        DecodeGeneration, ImageFileVersion, ImageFolder, ImageMemoryPolicy, ImageSize, PixelImage,
        SupportedImageFormat, ViewportSize,
    };
    use crate::infra::{
        animation_frame_prefetch_for_loaded_image_covers,
        load_animation_frame_for_view_with_prefetch_and_file_version,
        load_full_resolution_image_with_file_version, load_image_file_for_view_with_timing,
        preload_image_file_for_view_with_timing, scan_image_folder_for_file_with_cancellation,
        AnimationFramePixels, LoadImageError, ScanImageFolderError,
    };

    use super::{debug_output_line, notify_decode_worker_messages};

    // Bounds decode threads when canceled workers are slow to observe cancellation.
    pub(super) const MAX_IN_FLIGHT_DECODE_WORKERS: usize = 3;
    // Folder scans continue after the initial image is decoded. Keep their I/O
    // bounded separately so slow canceled scans do not accumulate without
    // blocking foreground decode startup behind them.
    pub(super) const MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS: usize = MAX_IN_FLIGHT_DECODE_WORKERS;
    const MAX_NAVIGATION_PRELOAD_WORKERS: usize = 2;
    const NO_FOLLOW_UP_ANIMATION_FRAME: usize = usize::MAX;

    pub(super) struct DecodeController {
        sender: mpsc::Sender<DecodeWorkerMessage>,
        receiver: mpsc::Receiver<DecodeWorkerMessage>,
        folder_scan_worker_count: Arc<AtomicUsize>,
        pub(super) active_worker: Option<DecodeWorker>,
        pub(super) retired_workers: Vec<DecodeWorker>,
        pub(super) folder_scan_workers: Vec<DecodeWorker>,
        navigation_preload_workers: Vec<NavigationPreloadWorker>,
        pub(super) pending_decode: Option<PendingDecodeRequest>,
    }

    pub(super) struct DecodeWorker {
        pub(super) generation: DecodeGeneration,
        pub(super) kind: DecodeWorkerKind,
        pub(super) cancel: Arc<AtomicBool>,
        pub(super) handle: JoinHandle<()>,
        pub(super) animation_frame: Option<ActiveAnimationFrameDecode>,
    }

    struct NavigationPreloadWorker {
        request: ImagePreloadRequest,
        cancel: Arc<AtomicBool>,
        handle: JoinHandle<()>,
    }

    pub(super) struct ActiveAnimationFrameDecode {
        path: PathBuf,
        file_version: ImageFileVersion,
        format: SupportedImageFormat,
        source_size: ImageSize,
        frame_index: usize,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
        follow_up_frame_index: Arc<AtomicUsize>,
    }

    pub(super) enum DecodeWorkerMessage {
        Initial {
            generation: DecodeGeneration,
            result: Result<(crate::domain::LoadedImage, ImageFolder), ViewerAppError>,
        },
        InitialDecodeCompleted {
            generation: DecodeGeneration,
        },
        FolderScanned {
            generation: DecodeGeneration,
            path: PathBuf,
            result: Result<ImageFolder, ScanImageFolderError>,
        },
        FolderScanSkipped {
            generation: DecodeGeneration,
            path: PathBuf,
        },
        FullResolution {
            generation: DecodeGeneration,
            file_version: Option<ImageFileVersion>,
            result: Result<PixelImage, LoadImageError>,
        },
        AnimationFrame {
            generation: DecodeGeneration,
            path: PathBuf,
            file_version: Option<ImageFileVersion>,
            frame_index: usize,
            result: Result<AnimationFramePixels, LoadImageError>,
        },
        NavigationPreload {
            request: ImagePreloadRequest,
            result: Result<crate::domain::LoadedImage, LoadImageError>,
        },
    }

    pub(super) enum PendingDecodeRequest {
        Initial {
            hwnd_value: isize,
            request: ImageDecodeRequest,
        },
        FullResolution {
            hwnd_value: isize,
            request: ImageDecodeRequest,
        },
        AnimationFrame {
            hwnd_value: isize,
            request: AnimationFrameDecodeRequest,
        },
    }

    pub(super) enum DecodeStartFailure {
        Initial {
            generation: DecodeGeneration,
            error: ViewerAppError,
        },
        FullResolution {
            generation: DecodeGeneration,
            file_version: Option<ImageFileVersion>,
            error: ViewerAppError,
        },
        AnimationFrame {
            generation: DecodeGeneration,
            path: PathBuf,
            file_version: ImageFileVersion,
            frame_index: usize,
            error: ViewerAppError,
        },
    }

    enum ActiveAnimationFrameRequest {
        InFlight {
            follow_up_frame_index: Arc<AtomicUsize>,
        },
        FollowUp {
            follow_up_frame_index: Arc<AtomicUsize>,
        },
    }

    impl DecodeStartFailure {
        fn into_error(self) -> ViewerAppError {
            match self {
                Self::Initial { error, .. }
                | Self::FullResolution { error, .. }
                | Self::AnimationFrame { error, .. } => error,
            }
        }
    }

    pub(super) struct DecodeControllerDrain {
        pub(super) messages: Vec<DecodeWorkerMessage>,
        pub(super) start_failures: Vec<DecodeStartFailure>,
    }

    impl PendingDecodeRequest {
        fn spawn_worker(
            self,
            sender: mpsc::Sender<DecodeWorkerMessage>,
            folder_scan_worker_count: &Arc<AtomicUsize>,
        ) -> Result<DecodeWorker, DecodeStartFailure> {
            match self {
                Self::Initial {
                    hwnd_value,
                    request,
                } => {
                    let generation = request.generation();
                    spawn_decode_worker(
                        hwnd_value as HWND,
                        sender,
                        request,
                        ImageDecodeWorkerKind::Initial,
                        Arc::clone(folder_scan_worker_count),
                    )
                    .map_err(|error| DecodeStartFailure::Initial { generation, error })
                }
                Self::FullResolution {
                    hwnd_value,
                    request,
                } => {
                    let generation = request.generation();
                    let file_version = request.file_version();
                    spawn_decode_worker(
                        hwnd_value as HWND,
                        sender,
                        request,
                        ImageDecodeWorkerKind::FullResolution,
                        Arc::clone(folder_scan_worker_count),
                    )
                    .map_err(|error| DecodeStartFailure::FullResolution {
                        generation,
                        file_version,
                        error,
                    })
                }
                Self::AnimationFrame {
                    hwnd_value,
                    request,
                } => {
                    let generation = request.generation();
                    let path = request.path().to_path_buf();
                    let file_version = request.file_version();
                    let frame_index = request.frame_index();
                    spawn_animation_frame_decode_worker(hwnd_value as HWND, sender, request)
                        .map_err(|error| DecodeStartFailure::AnimationFrame {
                            generation,
                            path,
                            file_version,
                            frame_index,
                            error,
                        })
                }
            }
        }
    }

    #[derive(Debug, PartialEq, Eq)]
    pub(super) enum DecodeNotificationOutcome {
        Posted,
        TimerFallback { post_error: u32 },
        SendFallback { post_error: u32, timer_error: u32 },
    }

    impl DecodeController {
        pub(super) fn new() -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                sender,
                receiver,
                folder_scan_worker_count: Arc::new(AtomicUsize::new(0)),
                active_worker: None,
                retired_workers: Vec::new(),
                folder_scan_workers: Vec::new(),
                navigation_preload_workers: Vec::new(),
                pending_decode: None,
            }
        }

        pub(super) fn start_initial_decode(
            &mut self,
            hwnd: HWND,
            request: ImageDecodeRequest,
        ) -> Result<(), ViewerAppError> {
            self.join_finished_workers();
            self.start_decode_or_queue(PendingDecodeRequest::Initial {
                hwnd_value: hwnd as isize,
                request,
            })
        }

        pub(super) fn start_full_resolution_decode(
            &mut self,
            hwnd: HWND,
            request: ImageDecodeRequest,
        ) -> Result<(), ViewerAppError> {
            self.join_finished_workers();
            if self.active_worker.as_ref().is_some_and(|worker| {
                !worker.handle.is_finished()
                    && worker.generation == request.generation()
                    && worker.kind == DecodeWorkerKind::FullResolution
            }) {
                return Ok(());
            }

            self.start_decode_or_queue(PendingDecodeRequest::FullResolution {
                hwnd_value: hwnd as isize,
                request,
            })
        }

        pub(super) fn start_animation_frame_decode(
            &mut self,
            hwnd: HWND,
            request: AnimationFrameDecodeRequest,
        ) -> Result<(), ViewerAppError> {
            self.join_finished_workers();
            match self.active_animation_frame_request(&request) {
                Some(ActiveAnimationFrameRequest::InFlight {
                    follow_up_frame_index,
                }) => {
                    self.pending_decode = None;
                    follow_up_frame_index.store(NO_FOLLOW_UP_ANIMATION_FRAME, Ordering::Release);
                    self.cancel_folder_scan_workers();
                    return Ok(());
                }
                Some(ActiveAnimationFrameRequest::FollowUp {
                    follow_up_frame_index,
                }) => {
                    follow_up_frame_index.store(request.frame_index(), Ordering::Release);
                    self.queue_pending_animation_frame_decode_without_cancel(hwnd, request);
                    return Ok(());
                }
                None => {}
            }
            self.start_decode_or_queue(PendingDecodeRequest::AnimationFrame {
                hwnd_value: hwnd as isize,
                request,
            })
        }

        pub(super) fn start_navigation_preloads(
            &mut self,
            hwnd: HWND,
            requests: Vec<ImagePreloadRequest>,
        ) {
            self.join_finished_workers();
            self.cancel_obsolete_navigation_preloads(&requests);
            for request in requests {
                if self.navigation_preload_workers.iter().any(|worker| {
                    worker.request == request && !worker.cancel.load(Ordering::Acquire)
                }) {
                    continue;
                }
                if self.navigation_preload_workers.len() >= MAX_NAVIGATION_PRELOAD_WORKERS {
                    break;
                }
                match spawn_navigation_preload_worker(hwnd, self.sender.clone(), request) {
                    Ok(worker) => self.navigation_preload_workers.push(worker),
                    Err(error) => {
                        debug_output_line(&format!(
                            "[j3Pic] navigation preload worker start failed: {error}"
                        ));
                    }
                }
            }
        }

        pub(super) fn drain_messages(&mut self) -> DecodeControllerDrain {
            self.join_finished_workers();
            let mut start_failures = Vec::new();
            if let Some(failure) = self.start_pending_decode_if_possible() {
                start_failures.push(failure);
            }
            let mut messages = Vec::new();
            while let Ok(message) = self.receiver.try_recv() {
                match message {
                    DecodeWorkerMessage::InitialDecodeCompleted { generation } => {
                        self.release_initial_worker_for_folder_scan(generation);
                    }
                    DecodeWorkerMessage::AnimationFrame {
                        generation,
                        path,
                        file_version,
                        frame_index,
                        result,
                    } => {
                        self.clear_pending_animation_frame_decode(
                            generation,
                            &path,
                            file_version,
                            frame_index,
                        );
                        messages.push(DecodeWorkerMessage::AnimationFrame {
                            generation,
                            path,
                            file_version,
                            frame_index,
                            result,
                        });
                    }
                    message => messages.push(message),
                }
            }
            self.join_finished_workers();
            if let Some(failure) = self.start_pending_decode_if_possible() {
                start_failures.push(failure);
            }
            DecodeControllerDrain {
                messages,
                start_failures,
            }
        }

        fn start_decode_or_queue(
            &mut self,
            pending: PendingDecodeRequest,
        ) -> Result<(), ViewerAppError> {
            self.pending_decode = None;
            self.cancel_folder_scan_workers();
            self.cancel_navigation_preload_workers();
            if self.can_start_replacement_worker() {
                self.cancel_active_worker();
                let worker = pending
                    .spawn_worker(self.sender.clone(), &self.folder_scan_worker_count)
                    .map_err(DecodeStartFailure::into_error)?;
                self.active_worker = Some(worker);
            } else {
                self.queue_pending_decode(pending);
            }
            Ok(())
        }

        fn active_animation_frame_request(
            &self,
            request: &AnimationFrameDecodeRequest,
        ) -> Option<ActiveAnimationFrameRequest> {
            let worker = self.active_worker.as_ref()?;
            if worker.handle.is_finished() || worker.cancel.load(Ordering::Acquire) {
                return None;
            }
            let active = worker.animation_frame.as_ref()?;
            if worker.generation != request.generation()
                || active.path != request.path()
                || active.file_version != request.file_version()
                || active.format != request.format()
                || active.source_size != request.source_size()
                || active.viewport != request.viewport()
                || active.memory_policy != request.memory_policy()
            {
                return None;
            }

            let requested_frame_index = request.frame_index();
            let follow_up_frame_index = Arc::clone(&active.follow_up_frame_index);
            if requested_frame_index == active.frame_index {
                return Some(ActiveAnimationFrameRequest::InFlight {
                    follow_up_frame_index,
                });
            }
            if requested_frame_index == NO_FOLLOW_UP_ANIMATION_FRAME {
                return None;
            }
            if animation_frame_prefetch_for_loaded_image_covers(
                &active.path,
                active.file_version,
                active.format,
                active.source_size,
                active.frame_index,
                requested_frame_index,
                active.viewport,
                active.memory_policy,
            ) {
                return Some(ActiveAnimationFrameRequest::FollowUp {
                    follow_up_frame_index,
                });
            }

            None
        }

        fn queue_pending_animation_frame_decode_without_cancel(
            &mut self,
            hwnd: HWND,
            request: AnimationFrameDecodeRequest,
        ) {
            self.pending_decode = Some(PendingDecodeRequest::AnimationFrame {
                hwnd_value: hwnd as isize,
                request,
            });
            self.cancel_folder_scan_workers();
            self.cancel_navigation_preload_workers();
        }

        fn clear_pending_animation_frame_decode(
            &mut self,
            generation: DecodeGeneration,
            path: &Path,
            file_version: Option<ImageFileVersion>,
            frame_index: usize,
        ) {
            let should_clear = self.pending_decode.as_ref().is_some_and(|pending| {
                matches!(
                    pending,
                    PendingDecodeRequest::AnimationFrame { request, .. }
                        if request.generation() == generation
                            && request.path() == path
                            && Some(request.file_version()) == file_version
                            && request.frame_index() == frame_index
                )
            });
            if should_clear {
                self.pending_decode = None;
            }
        }

        fn start_pending_decode_if_possible(&mut self) -> Option<DecodeStartFailure> {
            if !self.can_start_pending_worker() {
                return None;
            }
            let pending = self.pending_decode.take()?;
            self.cancel_folder_scan_workers();
            self.cancel_navigation_preload_workers();
            match pending.spawn_worker(self.sender.clone(), &self.folder_scan_worker_count) {
                Ok(worker) => {
                    self.active_worker = Some(worker);
                    None
                }
                Err(failure) => Some(failure),
            }
        }

        fn queue_pending_decode(&mut self, pending: PendingDecodeRequest) {
            self.pending_decode = Some(pending);
            if self.active_worker.is_none() {
                return;
            }
            if self.inactive_worker_count() < MAX_IN_FLIGHT_DECODE_WORKERS {
                self.cancel_active_worker();
            } else if let Some(worker) = self.active_worker.as_ref() {
                worker.cancel.store(true, Ordering::Release);
            }
        }

        fn can_start_replacement_worker(&self) -> bool {
            self.in_flight_worker_count() < MAX_IN_FLIGHT_DECODE_WORKERS
        }

        fn can_start_pending_worker(&self) -> bool {
            self.active_worker.is_none()
                && self.inactive_worker_count() < MAX_IN_FLIGHT_DECODE_WORKERS
        }

        fn in_flight_worker_count(&self) -> usize {
            self.inactive_worker_count() + usize::from(self.active_worker.is_some())
        }

        fn inactive_worker_count(&self) -> usize {
            self.retired_workers.len()
        }

        fn cancel_active_worker(&mut self) {
            if let Some(worker) = self.active_worker.take() {
                worker.cancel.store(true, Ordering::Release);
                self.retired_workers.push(worker);
            }
        }

        fn release_initial_worker_for_folder_scan(&mut self, generation: DecodeGeneration) {
            if self.active_worker.as_ref().is_some_and(|worker| {
                worker.generation == generation && worker.kind == DecodeWorkerKind::Initial
            }) {
                if let Some(worker) = self.active_worker.take() {
                    self.folder_scan_workers.push(worker);
                }
                return;
            }

            if let Some(index) = self.retired_workers.iter().position(|worker| {
                worker.generation == generation && worker.kind == DecodeWorkerKind::Initial
            }) {
                let worker = self.retired_workers.swap_remove(index);
                self.folder_scan_workers.push(worker);
            }
        }

        fn cancel_folder_scan_workers(&mut self) {
            for worker in &self.folder_scan_workers {
                worker.cancel.store(true, Ordering::Release);
            }
        }

        fn cancel_obsolete_navigation_preloads(&mut self, requests: &[ImagePreloadRequest]) {
            for worker in &self.navigation_preload_workers {
                if !requests.iter().any(|request| request == &worker.request) {
                    worker.cancel.store(true, Ordering::Release);
                }
            }
        }

        fn cancel_navigation_preload_workers(&mut self) {
            for worker in &self.navigation_preload_workers {
                worker.cancel.store(true, Ordering::Release);
            }
        }

        pub(super) fn shutdown(&mut self) {
            self.pending_decode = None;
            self.cancel_active_worker();
            // WM_DESTROY runs on the UI thread. Dropping handles detaches any slow
            // decoder/file I/O workers after cancellation instead of blocking teardown.
            for worker in self.retired_workers.drain(..) {
                worker.cancel.store(true, Ordering::Release);
            }
            for worker in self.folder_scan_workers.drain(..) {
                worker.cancel.store(true, Ordering::Release);
            }
            for worker in self.navigation_preload_workers.drain(..) {
                worker.cancel.store(true, Ordering::Release);
            }
        }

        fn join_finished_workers(&mut self) {
            if self
                .active_worker
                .as_ref()
                .is_some_and(|worker| worker.handle.is_finished())
            {
                if let Some(worker) = self.active_worker.take() {
                    let _ = worker.handle.join();
                }
            }

            Self::join_finished_worker_list(&mut self.retired_workers);
            Self::join_finished_worker_list(&mut self.folder_scan_workers);
            Self::join_finished_navigation_preload_workers(&mut self.navigation_preload_workers);
        }

        fn join_finished_worker_list(workers: &mut Vec<DecodeWorker>) {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].handle.is_finished() {
                    let worker = workers.swap_remove(index);
                    let _ = worker.handle.join();
                } else {
                    index += 1;
                }
            }
        }

        fn join_finished_navigation_preload_workers(workers: &mut Vec<NavigationPreloadWorker>) {
            let mut index = 0;
            while index < workers.len() {
                if workers[index].handle.is_finished() {
                    let worker = workers.swap_remove(index);
                    let _ = worker.handle.join();
                } else {
                    index += 1;
                }
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(super) enum DecodeWorkerKind {
        Initial,
        FullResolution,
        AnimationFrame,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ImageDecodeWorkerKind {
        Initial,
        FullResolution,
    }

    impl ImageDecodeWorkerKind {
        fn worker_kind(self) -> DecodeWorkerKind {
            match self {
                Self::Initial => DecodeWorkerKind::Initial,
                Self::FullResolution => DecodeWorkerKind::FullResolution,
            }
        }
    }

    pub(super) struct FolderScanPermit {
        active_count: Arc<AtomicUsize>,
    }

    impl FolderScanPermit {
        pub(super) fn try_acquire(active_count: &Arc<AtomicUsize>) -> Option<Self> {
            let mut current = active_count.load(Ordering::Acquire);
            loop {
                if current >= MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS {
                    return None;
                }
                match active_count.compare_exchange_weak(
                    current,
                    current + 1,
                    Ordering::AcqRel,
                    Ordering::Acquire,
                ) {
                    Ok(_) => {
                        return Some(Self {
                            active_count: Arc::clone(active_count),
                        });
                    }
                    Err(observed) => current = observed,
                }
            }
        }
    }

    impl Drop for FolderScanPermit {
        fn drop(&mut self) {
            let previous = self.active_count.fetch_sub(1, Ordering::AcqRel);
            debug_assert!(previous > 0);
        }
    }

    fn spawn_decode_worker(
        hwnd: HWND,
        sender: mpsc::Sender<DecodeWorkerMessage>,
        request: ImageDecodeRequest,
        kind: ImageDecodeWorkerKind,
        folder_scan_worker_count: Arc<AtomicUsize>,
    ) -> Result<DecodeWorker, ViewerAppError> {
        let generation = request.generation();
        let path = request.path().to_path_buf();
        let file_version = request.file_version();
        let error_path = path.clone();
        let viewport = request.viewport();
        let memory_policy = request.memory_policy();
        let animation_timing = request.animation_timing();
        let should_scan_folder =
            !matches!(request.purpose(), ImageDecodePurpose::FolderNavigation(_));
        let worker_kind = kind.worker_kind();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let hwnd_value = hwnd as isize;
        let handle = thread::Builder::new()
            .spawn(move || {
                match kind {
                    ImageDecodeWorkerKind::Initial => {
                        let result = match load_image_file_for_view_with_timing(
                            &path,
                            viewport,
                            memory_policy,
                            animation_timing,
                            Some(worker_cancel.as_ref()),
                        ) {
                            Ok(image) => {
                                let image_path = image.metadata().path().to_path_buf();
                                let folder = if should_scan_folder {
                                    ImageFolder::from_paths(&image_path, [image_path.clone()])
                                } else {
                                    ImageFolder::empty()
                                };
                                Ok((image, folder))
                            }
                            Err(error) => Err(ViewerAppError::from(error)),
                        };
                        let scan_path = result
                            .as_ref()
                            .ok()
                            .map(|(image, _)| image.metadata().path().to_path_buf());
                        let initial_sent = send_decode_worker_message(
                            &sender,
                            hwnd_value,
                            DecodeWorkerMessage::Initial { generation, result },
                        );
                        if initial_sent && should_scan_folder {
                            if let Some(scan_path) = scan_path {
                                let scan_released = send_decode_worker_message(
                                    &sender,
                                    hwnd_value,
                                    DecodeWorkerMessage::InitialDecodeCompleted { generation },
                                );
                                if scan_released {
                                    let result = if worker_cancel.load(Ordering::Acquire) {
                                        None
                                    } else if let Some(_permit) =
                                        FolderScanPermit::try_acquire(&folder_scan_worker_count)
                                    {
                                        scan_image_folder_for_file_with_cancellation(
                                            &scan_path,
                                            worker_cancel.as_ref(),
                                        )
                                    } else {
                                        None
                                    };
                                    let message = match result {
                                        Some(result) => DecodeWorkerMessage::FolderScanned {
                                            generation,
                                            path: scan_path,
                                            result,
                                        },
                                        None => DecodeWorkerMessage::FolderScanSkipped {
                                            generation,
                                            path: scan_path,
                                        },
                                    };
                                    send_decode_worker_message(&sender, hwnd_value, message);
                                }
                            }
                        }
                    }
                    ImageDecodeWorkerKind::FullResolution => {
                        let result = load_full_resolution_image_with_file_version(
                            &path,
                            memory_policy,
                            Some(worker_cancel.as_ref()),
                        )
                        .map(|(rgba8, decoded_file_version)| (decoded_file_version, rgba8));
                        let (file_version, result) = match result {
                            Ok((decoded_file_version, rgba8)) => (decoded_file_version, Ok(rgba8)),
                            Err(error) => (file_version, Err(error)),
                        };
                        send_decode_worker_message(
                            &sender,
                            hwnd_value,
                            DecodeWorkerMessage::FullResolution {
                                generation,
                                file_version,
                                result,
                            },
                        );
                    }
                };
            })
            .map_err(|source| ViewerAppError::DecodeWorkerStart {
                path: error_path,
                source,
            })?;

        Ok(DecodeWorker {
            generation,
            kind: worker_kind,
            cancel,
            handle,
            animation_frame: None,
        })
    }

    fn spawn_navigation_preload_worker(
        hwnd: HWND,
        sender: mpsc::Sender<DecodeWorkerMessage>,
        request: ImagePreloadRequest,
    ) -> Result<NavigationPreloadWorker, ViewerAppError> {
        let error_path = request.path().to_path_buf();
        let path = request.path().to_path_buf();
        let viewport = request.viewport();
        let memory_policy = request.memory_policy();
        let animation_timing = request.animation_timing();
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let worker_request = request.clone();
        let hwnd_value = hwnd as isize;
        let handle = thread::Builder::new()
            .spawn(move || {
                let result = preload_image_file_for_view_with_timing(
                    &path,
                    viewport,
                    memory_policy,
                    animation_timing,
                    Some(worker_cancel.as_ref()),
                );
                if worker_cancel.load(Ordering::Acquire) {
                    return;
                }
                send_decode_worker_message(
                    &sender,
                    hwnd_value,
                    DecodeWorkerMessage::NavigationPreload {
                        request: worker_request,
                        result,
                    },
                );
            })
            .map_err(|source| ViewerAppError::DecodeWorkerStart {
                path: error_path,
                source,
            })?;

        Ok(NavigationPreloadWorker {
            request,
            cancel,
            handle,
        })
    }

    fn send_decode_worker_message(
        sender: &mpsc::Sender<DecodeWorkerMessage>,
        hwnd_value: isize,
        message: DecodeWorkerMessage,
    ) -> bool {
        if sender.send(message).is_err() {
            return false;
        }
        notify_decode_worker_messages(hwnd_value as HWND);
        true
    }

    fn spawn_animation_frame_decode_worker(
        hwnd: HWND,
        sender: mpsc::Sender<DecodeWorkerMessage>,
        request: AnimationFrameDecodeRequest,
    ) -> Result<DecodeWorker, ViewerAppError> {
        let generation = request.generation();
        let frame_index = request.frame_index();
        let path = request.path().to_path_buf();
        let file_version = request.file_version();
        let error_path = path.clone();
        let viewport = request.viewport();
        let memory_policy = request.memory_policy();
        let format = request.format();
        let source_size = request.source_size();
        let follow_up_frame_index = Arc::new(AtomicUsize::new(NO_FOLLOW_UP_ANIMATION_FRAME));
        let worker_follow_up_frame_index = Arc::clone(&follow_up_frame_index);
        let active_animation_frame = ActiveAnimationFrameDecode {
            path: path.clone(),
            file_version,
            format,
            source_size,
            frame_index,
            viewport,
            memory_policy,
            follow_up_frame_index,
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let hwnd_value = hwnd as isize;
        let handle = thread::Builder::new()
            .spawn(move || {
                let mut delivered = false;
                let result = load_animation_frame_for_view_with_prefetch_and_file_version(
                    &path,
                    frame_index,
                    viewport,
                    memory_policy,
                    |decoded_file_version, frame| {
                        delivered = true;
                        send_decode_worker_message(
                            &sender,
                            hwnd_value,
                            DecodeWorkerMessage::AnimationFrame {
                                generation,
                                path: path.clone(),
                                file_version: decoded_file_version,
                                frame_index,
                                result: Ok(frame),
                            },
                        )
                    },
                    |decoded_file_version, prefetched_frame_index, frame| {
                        if worker_follow_up_frame_index.load(Ordering::Acquire)
                            != prefetched_frame_index
                        {
                            return true;
                        }
                        let sent = send_decode_worker_message(
                            &sender,
                            hwnd_value,
                            DecodeWorkerMessage::AnimationFrame {
                                generation,
                                path: path.clone(),
                                file_version: decoded_file_version,
                                frame_index: prefetched_frame_index,
                                result: Ok(frame),
                            },
                        );
                        if sent {
                            let _ = worker_follow_up_frame_index.compare_exchange(
                                prefetched_frame_index,
                                NO_FOLLOW_UP_ANIMATION_FRAME,
                                Ordering::AcqRel,
                                Ordering::Acquire,
                            );
                        }
                        sent
                    },
                    Some(worker_cancel.as_ref()),
                );
                if !delivered {
                    let result: Result<AnimationFramePixels, LoadImageError> = match result {
                        Ok(()) => Err(LoadImageError::AnimationFrameUnavailable {
                            path: path.clone(),
                            frame_index,
                        }),
                        Err(error) => Err(error),
                    };
                    send_decode_worker_message(
                        &sender,
                        hwnd_value,
                        DecodeWorkerMessage::AnimationFrame {
                            generation,
                            path,
                            file_version: Some(file_version),
                            frame_index,
                            result,
                        },
                    );
                }
                if delivered {
                    notify_decode_worker_messages(hwnd_value as HWND);
                }
            })
            .map_err(|source| ViewerAppError::DecodeWorkerStart {
                path: error_path,
                source,
            })?;

        Ok(DecodeWorker {
            generation,
            kind: DecodeWorkerKind::AnimationFrame,
            cancel,
            handle,
            animation_frame: Some(active_animation_frame),
        })
    }
}

mod export_worker {
    use std::path::PathBuf;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{mpsc, Arc};
    use std::thread::{self, JoinHandle};

    use windows_sys::Win32::Foundation::HWND;

    use crate::app::{ImageExportRequest, ViewerAppError};
    use crate::domain::ExportOptions;

    use super::notify_export_worker_messages;

    const MAX_IN_FLIGHT_EXPORT_WORKERS: usize = 1;

    pub(super) struct ExportController {
        sender: mpsc::Sender<ExportWorkerMessage>,
        receiver: mpsc::Receiver<ExportWorkerMessage>,
        workers: Vec<ExportWorker>,
        notifications_enabled: Arc<AtomicBool>,
    }

    struct ExportWorker {
        handle: JoinHandle<()>,
    }

    pub(super) enum ExportWorkerMessage {
        Completed {
            path: PathBuf,
            options: ExportOptions,
            quality: Option<u8>,
            result: Result<(), ViewerAppError>,
        },
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ExportStartOutcome {
        Started,
        Busy,
    }

    #[derive(Clone, Copy, Debug, PartialEq, Eq)]
    pub(super) enum ExportShutdownOutcome {
        Complete,
        WaitingForWorkers,
    }

    impl ExportController {
        pub(super) fn new() -> Self {
            let (sender, receiver) = mpsc::channel();
            Self {
                sender,
                receiver,
                workers: Vec::new(),
                notifications_enabled: Arc::new(AtomicBool::new(true)),
            }
        }

        pub(super) fn start_export(
            &mut self,
            hwnd: HWND,
            build_request: impl FnOnce() -> Result<ImageExportRequest, ViewerAppError>,
            quality: Option<u8>,
        ) -> Result<ExportStartOutcome, ViewerAppError> {
            let sender = self.sender.clone();
            let notifications_enabled = Arc::clone(&self.notifications_enabled);
            self.try_start_export_worker(|| {
                let request = build_request()?;
                spawn_export_worker(hwnd, sender, notifications_enabled, request, quality)
            })
        }

        pub(super) fn drain_messages(&mut self) -> Vec<ExportWorkerMessage> {
            self.join_finished_workers();
            let mut messages = Vec::new();
            while let Ok(message) = self.receiver.try_recv() {
                messages.push(message);
            }
            self.join_finished_workers();
            messages
        }

        pub(super) fn shutdown(
            &mut self,
            on_workers_joined: impl FnOnce() + Send + 'static,
        ) -> ExportShutdownOutcome {
            self.notifications_enabled.store(false, Ordering::Release);
            self.join_finished_workers();
            let workers = std::mem::take(&mut self.workers);
            self.close_worker_message_channel();
            if workers.is_empty() {
                return ExportShutdownOutcome::Complete;
            }

            match spawn_export_shutdown_joiner(workers, on_workers_joined) {
                Ok(()) => ExportShutdownOutcome::WaitingForWorkers,
                Err(workers) => {
                    join_export_workers(workers);
                    ExportShutdownOutcome::Complete
                }
            }
        }

        fn close_worker_message_channel(&mut self) {
            let (sender, receiver) = mpsc::channel();
            let previous_sender = std::mem::replace(&mut self.sender, sender);
            let previous_receiver = std::mem::replace(&mut self.receiver, receiver);
            drop(previous_receiver);
            drop(previous_sender);
        }

        fn try_start_export_worker(
            &mut self,
            spawn_worker: impl FnOnce() -> Result<ExportWorker, ViewerAppError>,
        ) -> Result<ExportStartOutcome, ViewerAppError> {
            self.join_finished_workers();
            if self.workers.len() >= MAX_IN_FLIGHT_EXPORT_WORKERS {
                return Ok(ExportStartOutcome::Busy);
            }

            let worker = spawn_worker()?;
            self.workers.push(worker);
            Ok(ExportStartOutcome::Started)
        }

        fn join_finished_workers(&mut self) {
            let mut index = 0;
            while index < self.workers.len() {
                if self.workers[index].handle.is_finished() {
                    let worker = self.workers.swap_remove(index);
                    let _ = worker.handle.join();
                } else {
                    index += 1;
                }
            }
        }
    }

    fn spawn_export_shutdown_joiner(
        workers: Vec<ExportWorker>,
        on_workers_joined: impl FnOnce() + Send + 'static,
    ) -> Result<(), Vec<ExportWorker>> {
        let (workers_sender, workers_receiver) = mpsc::sync_channel(1);
        let joiner = thread::Builder::new()
            .name("j3pic-export-shutdown".to_string())
            .spawn(move || {
                if let Ok(workers) = workers_receiver.recv() {
                    join_export_workers(workers);
                    on_workers_joined();
                }
            });

        match joiner {
            Ok(handle) => {
                drop(handle);
                workers_sender
                    .send(workers)
                    .map_err(|send_error| send_error.0)
            }
            Err(_) => Err(workers),
        }
    }

    fn join_export_workers(workers: Vec<ExportWorker>) {
        for worker in workers {
            let _ = worker.handle.join();
        }
    }

    fn spawn_export_worker(
        hwnd: HWND,
        sender: mpsc::Sender<ExportWorkerMessage>,
        notifications_enabled: Arc<AtomicBool>,
        request: ImageExportRequest,
        quality: Option<u8>,
    ) -> Result<ExportWorker, ViewerAppError> {
        let path = request.path().to_path_buf();
        let options = request.options();
        let error_path = path.clone();
        let hwnd_value = hwnd as isize;
        let handle = thread::Builder::new()
            .spawn(move || {
                let result = request.export();
                send_export_worker_message(
                    &sender,
                    notifications_enabled.as_ref(),
                    hwnd_value,
                    ExportWorkerMessage::Completed {
                        path,
                        options,
                        quality,
                        result,
                    },
                );
            })
            .map_err(|source| ViewerAppError::ExportWorkerStart {
                path: error_path,
                source,
            })?;

        Ok(ExportWorker { handle })
    }

    fn send_export_worker_message(
        sender: &mpsc::Sender<ExportWorkerMessage>,
        notifications_enabled: &AtomicBool,
        hwnd_value: isize,
        message: ExportWorkerMessage,
    ) {
        if !notifications_enabled.load(Ordering::Acquire) {
            return;
        }
        if sender.send(message).is_ok() && notifications_enabled.load(Ordering::Acquire) {
            notify_export_worker_messages(hwnd_value as HWND);
        }
    }

    #[cfg(test)]
    mod tests {
        use std::sync::atomic::{AtomicBool, Ordering};
        use std::sync::{mpsc, Arc, Condvar, Mutex};
        use std::thread;
        use std::time::Duration;

        use super::{
            ExportController, ExportShutdownOutcome, ExportStartOutcome, ExportWorker,
            MAX_IN_FLIGHT_EXPORT_WORKERS,
        };

        #[test]
        fn start_export_reports_busy_when_active_worker_limit_is_reached() {
            let mut controller = ExportController::new();
            let (worker, release_worker, worker_finished) = test_export_worker();
            controller.workers.push(worker);
            let spawn_called = Arc::new(AtomicBool::new(false));
            let spawn_called_clone = Arc::clone(&spawn_called);

            let outcome = controller
                .try_start_export_worker(|| {
                    spawn_called_clone.store(true, Ordering::Release);
                    let (worker, release_worker, worker_finished) = test_export_worker();
                    release_worker.release();
                    let _ = worker_finished.recv_timeout(Duration::from_secs(1));
                    Ok(worker)
                })
                .expect("start export worker");

            let worker_count = controller.workers.len();
            let spawn_called = spawn_called.load(Ordering::Acquire);
            release_worker.release();
            assert!(worker_finished.recv_timeout(Duration::from_secs(1)).is_ok());
            let _ = controller.shutdown(|| {});

            assert_eq!(outcome, ExportStartOutcome::Busy);
            assert!(!spawn_called);
            assert_eq!(worker_count, MAX_IN_FLIGHT_EXPORT_WORKERS);
            assert!(controller.workers.is_empty());
        }

        #[test]
        fn start_export_reports_busy_without_building_request() {
            let mut controller = ExportController::new();
            let (worker, release_worker, worker_finished) = test_export_worker();
            controller.workers.push(worker);
            let build_called = Arc::new(AtomicBool::new(false));
            let build_called_clone = Arc::clone(&build_called);

            let outcome = controller.start_export(
                std::ptr::null_mut(),
                || {
                    build_called_clone.store(true, Ordering::Release);
                    Err(crate::app::ViewerAppError::NoImageToExport)
                },
                None,
            );

            let worker_count = controller.workers.len();
            let build_called = build_called.load(Ordering::Acquire);
            release_worker.release();
            assert!(worker_finished.recv_timeout(Duration::from_secs(1)).is_ok());
            let _ = controller.shutdown(|| {});

            assert!(matches!(outcome, Ok(ExportStartOutcome::Busy)));
            assert!(!build_called);
            assert_eq!(worker_count, MAX_IN_FLIGHT_EXPORT_WORKERS);
            assert!(controller.workers.is_empty());
        }

        #[test]
        fn shutdown_offloads_active_export_worker_join_before_completion_callback() {
            let mut controller = ExportController::new();
            let (worker, release_worker, worker_finished) = test_export_worker();
            controller.workers.push(worker);
            let (shutdown_done_sender, shutdown_done_receiver) = mpsc::channel();
            let (completion_sender, completion_receiver) = mpsc::channel();

            let shutdown_thread = thread::spawn(move || {
                let outcome = controller.shutdown(move || {
                    let _ = completion_sender.send(());
                });
                let _ = shutdown_done_sender.send((outcome, controller.workers.is_empty()));
            });

            let shutdown_returned_before_worker_release = matches!(
                shutdown_done_receiver.recv_timeout(Duration::from_secs(1)),
                Ok((ExportShutdownOutcome::WaitingForWorkers, true))
            );
            let completion_before_worker_release = completion_receiver
                .recv_timeout(Duration::from_millis(100))
                .is_ok();
            let worker_finished_before_release = worker_finished
                .recv_timeout(Duration::from_millis(100))
                .is_ok();
            release_worker.release();
            let worker_finished_after_release =
                worker_finished.recv_timeout(Duration::from_secs(1)).is_ok();
            let completion_after_worker_release = completion_receiver
                .recv_timeout(Duration::from_secs(1))
                .is_ok();

            assert!(
                shutdown_returned_before_worker_release,
                "shutdown should return without waiting for an active export worker"
            );
            assert!(
                !completion_before_worker_release,
                "shutdown completion should wait for the export worker"
            );
            assert!(
                !worker_finished_before_release,
                "export worker should still be blocked before release"
            );
            assert!(
                worker_finished_after_release,
                "export worker should finish after release"
            );
            assert!(
                completion_after_worker_release,
                "shutdown completion should run after export worker exit"
            );
            assert!(
                shutdown_thread.join().is_ok(),
                "shutdown thread should not panic"
            );
        }

        fn test_export_worker() -> (ExportWorker, TestExportWorkerRelease, mpsc::Receiver<()>) {
            let release = TestExportWorkerRelease::new();
            let worker_release = release.clone();
            let (finished_sender, finished_receiver) = mpsc::channel();
            let handle = thread::spawn(move || {
                worker_release.wait();
                let _ = finished_sender.send(());
            });

            (ExportWorker { handle }, release, finished_receiver)
        }

        #[derive(Clone)]
        struct TestExportWorkerRelease {
            state: Arc<(Mutex<bool>, Condvar)>,
        }

        impl TestExportWorkerRelease {
            fn new() -> Self {
                Self {
                    state: Arc::new((Mutex::new(false), Condvar::new())),
                }
            }

            fn release(&self) {
                let (lock, condition) = &*self.state;
                let mut released = lock.lock().expect("test export worker release lock");
                *released = true;
                condition.notify_all();
            }

            fn wait(&self) {
                let (lock, condition) = &*self.state;
                let mut released = lock.lock().expect("test export worker release lock");
                while !*released {
                    released = condition
                        .wait(released)
                        .expect("test export worker release wait");
                }
            }
        }
    }
}

struct FullscreenState {
    restore: Option<FullscreenRestoreState>,
}

#[derive(Clone, Copy)]
struct FullscreenRestoreState {
    style: isize,
    placement: WINDOWPLACEMENT,
}

#[derive(Debug, Clone, Copy)]
enum FullscreenError {
    MissingWindowState,
    GetWindowPlacement { code: u32 },
    GetWindowStyle { code: u32 },
    GetMonitorInfo { code: u32 },
    SetWindowPlacement { code: u32 },
    SetWindowStyle { code: u32 },
    SetWindowPos { code: u32 },
}

impl FullscreenState {
    fn new() -> Self {
        Self { restore: None }
    }

    fn is_fullscreen(&self) -> bool {
        self.restore.is_some()
    }
}

struct PaintDibCache {
    key: Option<PaintDibCacheKey>,
    pixels: Vec<u8>,
    scratch_pixels: Vec<u8>,
}

const PAINT_DIB_CACHE_MAX_RETAINED_CAPACITY_MULTIPLIER: usize = 2;
const PAINT_DIB_CACHE_MAX_DISPLAY_OVERSAMPLE_MULTIPLIER: usize = 2;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintDibCacheKey {
    render_key: RenderImageCacheKey,
    source_rect: PaintDibSourceRect,
    scaling_quality: ScalingQuality,
}

struct PaintDibPixels<'a> {
    pixels: &'a [u8],
    source_rect: PaintDibSourceRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintDibSourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintDibPlacement {
    source_rect: PaintDibSourceRect,
    dest_x: i32,
    dest_y: i32,
    dest_width: i32,
    dest_height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct PaintDibAxisPlacement {
    source_start: u32,
    source_size: u32,
    dest_start: i32,
    dest_size: i32,
}

struct PaintRgba8Image<'a> {
    hdc: HDC,
    client_rect: &'a RECT,
    pixels: &'a PixelImage,
    rect: crate::domain::ImageDisplayRect,
    scaling_quality: ScalingQuality,
    cache_key: RenderImageCacheKey,
    max_cache_bytes: usize,
}

impl PaintDibCache {
    fn new() -> Self {
        Self {
            key: None,
            pixels: Vec::new(),
            scratch_pixels: Vec::new(),
        }
    }

    fn invalidate(&mut self) {
        self.key = None;
        self.pixels.clear();
        self.scratch_pixels = Vec::new();
    }

    fn release(&mut self) {
        self.key = None;
        self.pixels = Vec::new();
    }

    #[cfg(test)]
    fn pixels_for(
        &mut self,
        key: RenderImageCacheKey,
        rgba8: &Rgba8Image,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibPixels<'_>> {
        let pixels = PixelImage::from(rgba8.clone());
        self.pixels_for_pixels(key, &pixels, scaling_quality, max_cache_bytes)
    }

    fn pixels_for_pixels(
        &mut self,
        key: RenderImageCacheKey,
        pixels: &PixelImage,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibPixels<'_>> {
        let source_rect = PaintDibSourceRect::full(pixels.width(), pixels.height())?;
        self.pixels_for_pixel_rect(key, pixels, source_rect, scaling_quality, max_cache_bytes)
    }

    #[cfg(test)]
    fn pixels_for_rect(
        &mut self,
        render_key: RenderImageCacheKey,
        rgba8: &Rgba8Image,
        source_rect: PaintDibSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibPixels<'_>> {
        let pixels = PixelImage::from(rgba8.clone());
        self.pixels_for_pixel_rect(
            render_key,
            &pixels,
            source_rect,
            scaling_quality,
            max_cache_bytes,
        )
    }

    fn pixels_for_pixel_rect(
        &mut self,
        render_key: RenderImageCacheKey,
        pixels: &PixelImage,
        source_rect: PaintDibSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibPixels<'_>> {
        let source_rect = self.cache_source_rect_for_pixel_rect(
            render_key,
            pixels,
            source_rect,
            scaling_quality,
            max_cache_bytes,
        )?;

        Some(PaintDibPixels {
            pixels: &self.pixels,
            source_rect,
        })
    }

    fn pixels_for_paint_pixel_rect(
        &mut self,
        render_key: RenderImageCacheKey,
        pixels: &PixelImage,
        source_rect: PaintDibSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibPixels<'_>> {
        if let Some(source_rect) = self.cache_source_rect_for_pixel_rect(
            render_key,
            pixels,
            source_rect,
            scaling_quality,
            max_cache_bytes,
        ) {
            return Some(PaintDibPixels {
                pixels: &self.pixels,
                source_rect,
            });
        }

        convert_pixel_rect_to_bgra32_dib(pixels, source_rect, &mut self.scratch_pixels)?;
        Some(PaintDibPixels {
            pixels: &self.scratch_pixels,
            source_rect,
        })
    }

    fn cache_source_rect_for_pixel_rect(
        &mut self,
        render_key: RenderImageCacheKey,
        pixels: &PixelImage,
        source_rect: PaintDibSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<PaintDibSourceRect> {
        let full_source_rect = PaintDibSourceRect::full(pixels.width(), pixels.height())?;
        if source_rect.width == 0
            || source_rect.height == 0
            || !full_source_rect.contains(source_rect)?
        {
            return None;
        }

        if full_source_rect.byte_len()? <= max_cache_bytes {
            let key = PaintDibCacheKey {
                render_key,
                source_rect: full_source_rect,
                scaling_quality,
            };
            if self.key != Some(key) {
                convert_pixel_rect_to_bgra32_dib(pixels, full_source_rect, &mut self.pixels)?;
                self.key = Some(key);
            }

            return Some(full_source_rect);
        }

        if let Some(key) = self.key {
            if key.render_key == render_key && key.scaling_quality == scaling_quality {
                let cached_source_rect = key.source_rect;
                if cached_source_rect.byte_len()? <= max_cache_bytes
                    && cached_source_rect.contains(source_rect)?
                {
                    return Some(cached_source_rect);
                }
            }
        }

        let Some(cache_source_rect) =
            expanded_paint_dib_cache_source_rect(full_source_rect, source_rect, max_cache_bytes)
        else {
            self.release();
            return None;
        };

        let key = PaintDibCacheKey {
            render_key,
            source_rect: cache_source_rect,
            scaling_quality,
        };
        if self.key != Some(key) {
            convert_pixel_rect_to_bgra32_dib(pixels, cache_source_rect, &mut self.pixels)?;
            self.key = Some(key);
        }

        Some(cache_source_rect)
    }
}

fn expanded_paint_dib_cache_source_rect(
    full_source_rect: PaintDibSourceRect,
    source_rect: PaintDibSourceRect,
    max_cache_bytes: usize,
) -> Option<PaintDibSourceRect> {
    if source_rect.byte_len()? > max_cache_bytes {
        return None;
    }

    let max_pixels = max_cache_bytes / DIB_BYTES_PER_PIXEL;
    let full_width = usize::try_from(full_source_rect.width).ok()?;
    let full_height = usize::try_from(full_source_rect.height).ok()?;
    let source_width = usize::try_from(source_rect.width).ok()?;
    let source_height = usize::try_from(source_rect.height).ok()?;
    if max_pixels == 0 || source_width == 0 || source_height == 0 {
        return None;
    }

    let cache_width = full_width.min(max_pixels.checked_div(source_height)?);
    let cache_height = full_height.min(max_pixels.checked_div(cache_width)?);
    let cache_width = u32::try_from(cache_width).ok()?;
    let cache_height = u32::try_from(cache_height).ok()?;
    let cache_x = expanded_paint_dib_cache_axis_start(
        full_source_rect.x,
        full_source_rect.width,
        source_rect.x,
        source_rect.width,
        cache_width,
    )?;
    let cache_y = expanded_paint_dib_cache_axis_start(
        full_source_rect.y,
        full_source_rect.height,
        source_rect.y,
        source_rect.height,
        cache_height,
    )?;

    Some(PaintDibSourceRect {
        x: cache_x,
        y: cache_y,
        width: cache_width,
        height: cache_height,
    })
}

fn expanded_paint_dib_cache_axis_start(
    full_start: u32,
    full_size: u32,
    source_start: u32,
    source_size: u32,
    cache_size: u32,
) -> Option<u32> {
    let max_start = full_start.checked_add(full_size.checked_sub(cache_size)?)?;
    let extra_before = cache_size.checked_sub(source_size)? / 2;
    let centered_start = source_start.saturating_sub(extra_before);

    Some(centered_start.clamp(full_start, max_start))
}

impl PaintDibSourceRect {
    fn full(width: u32, height: u32) -> Option<Self> {
        if width == 0 || height == 0 {
            return None;
        }

        Some(Self {
            x: 0,
            y: 0,
            width,
            height,
        })
    }

    fn byte_len(self) -> Option<usize> {
        expected_dib_len(self.width, self.height)
    }

    fn contains(self, other: Self) -> Option<bool> {
        let right = self.x.checked_add(self.width)?;
        let bottom = self.y.checked_add(self.height)?;
        let other_right = other.x.checked_add(other.width)?;
        let other_bottom = other.y.checked_add(other.height)?;

        Some(
            self.x <= other.x
                && self.y <= other.y
                && other_right <= right
                && other_bottom <= bottom,
        )
    }
}

impl FullscreenError {
    fn user_message(self) -> String {
        match self {
            Self::MissingWindowState => {
                "전체화면 전환에 필요한 창 상태를 찾지 못했습니다.".to_owned()
            }
            Self::GetWindowPlacement { .. } => {
                "전체화면 전환 전 창 상태를 저장하지 못했습니다.".to_owned()
            }
            Self::GetWindowStyle { .. } => {
                "전체화면 전환 전 창 스타일을 읽지 못했습니다.".to_owned()
            }
            Self::GetMonitorInfo { .. } => {
                "전체화면으로 사용할 모니터 정보를 가져오지 못했습니다.".to_owned()
            }
            Self::SetWindowPlacement { .. } => {
                "전체화면 해제 중 이전 창 위치를 복원하지 못했습니다.".to_owned()
            }
            Self::SetWindowStyle { .. } => {
                "전체화면 전환 중 창 스타일을 변경하지 못했습니다.".to_owned()
            }
            Self::SetWindowPos { .. } => "전체화면 창 크기 변경에 실패했습니다.".to_owned(),
        }
    }

    fn user_message_for(self, language: UiLanguage) -> String {
        if language == UiLanguage::Korean {
            return self.user_message();
        }
        match self {
            Self::MissingWindowState => {
                "Could not find the window state needed to toggle fullscreen.".to_owned()
            }
            Self::GetWindowPlacement { .. } => {
                "Could not save the window state before entering fullscreen.".to_owned()
            }
            Self::GetWindowStyle { .. } => {
                "Could not read the window style before entering fullscreen.".to_owned()
            }
            Self::GetMonitorInfo { .. } => {
                "Could not get monitor information for fullscreen mode.".to_owned()
            }
            Self::SetWindowPlacement { .. } => {
                "Could not restore the previous window position when leaving fullscreen.".to_owned()
            }
            Self::SetWindowStyle { .. } => {
                "Could not change the window style while toggling fullscreen.".to_owned()
            }
            Self::SetWindowPos { .. } => "Could not resize the fullscreen window.".to_owned(),
        }
    }
}

impl fmt::Display for FullscreenError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingWindowState => formatter.write_str("missing window state"),
            Self::GetWindowPlacement { code } => {
                write!(formatter, "GetWindowPlacement failed: Win32 error {code}")
            }
            Self::GetWindowStyle { code } => {
                write!(
                    formatter,
                    "GetWindowLongPtrW(GWL_STYLE) failed: Win32 error {code}"
                )
            }
            Self::GetMonitorInfo { code } => {
                write!(formatter, "GetMonitorInfoW failed: Win32 error {code}")
            }
            Self::SetWindowPlacement { code } => {
                write!(formatter, "SetWindowPlacement failed: Win32 error {code}")
            }
            Self::SetWindowStyle { code } => {
                write!(
                    formatter,
                    "SetWindowLongPtrW(GWL_STYLE) failed: Win32 error {code}"
                )
            }
            Self::SetWindowPos { code } => {
                write!(formatter, "SetWindowPos failed: Win32 error {code}")
            }
        }
    }
}

fn module_instance() -> Result<HINSTANCE, Win32Error> {
    // SAFETY: Passing a null module name requests the current module handle.
    let instance = unsafe { GetModuleHandleW(null()) };

    if instance.is_null() {
        Err(Win32Error::ModuleHandle { code: last_error() })
    } else {
        Ok(instance)
    }
}

fn register_window_class(instance: HINSTANCE, class_name: *const u16) -> Result<(), Win32Error> {
    // SAFETY: Loading a predefined system cursor does not transfer ownership.
    let cursor = unsafe { LoadCursorW(null_mut(), IDC_ARROW) };
    let icon = load_large_app_icon(instance);
    let window_class = WNDCLASSW {
        style: CS_HREDRAW | CS_VREDRAW,
        lpfnWndProc: Some(wnd_proc),
        cbClsExtra: 0,
        cbWndExtra: 0,
        hInstance: instance,
        hIcon: icon,
        hCursor: cursor,
        hbrBackground: null_mut(),
        lpszMenuName: null(),
        lpszClassName: class_name,
    };

    // SAFETY: window_class points to stack data valid for the duration of the call.
    let atom = unsafe { RegisterClassW(&window_class) };
    if atom == 0 {
        let code = last_error();
        if code != ERROR_CLASS_ALREADY_EXISTS {
            return Err(Win32Error::RegisterClass { code });
        }
    }

    Ok(())
}

fn apply_window_icons(hwnd: HWND) {
    let Ok(instance) = module_instance() else {
        return;
    };

    let large_icon = load_large_app_icon(instance);
    if !large_icon.is_null() {
        // SAFETY: hwnd is a live top-level window and large_icon is a shared resource handle.
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_BIG as WPARAM, large_icon as LPARAM);
        }
    }

    let small_icon = load_small_app_icon(instance);
    if !small_icon.is_null() {
        // SAFETY: hwnd is a live top-level window and small_icon is a shared resource handle.
        unsafe {
            SendMessageW(hwnd, WM_SETICON, ICON_SMALL as WPARAM, small_icon as LPARAM);
            SendMessageW(
                hwnd,
                WM_SETICON,
                ICON_SMALL2 as WPARAM,
                small_icon as LPARAM,
            );
        }
    }
}

fn load_large_app_icon(instance: HINSTANCE) -> HICON {
    let width = unsafe { GetSystemMetrics(SM_CXICON) };
    let height = unsafe { GetSystemMetrics(SM_CYICON) };
    load_app_icon(instance, width, height)
}

fn load_small_app_icon(instance: HINSTANCE) -> HICON {
    let width = unsafe { GetSystemMetrics(SM_CXSMICON) };
    let height = unsafe { GetSystemMetrics(SM_CYSMICON) };
    load_app_icon(instance, width, height)
}

fn load_app_icon(instance: HINSTANCE, width: i32, height: i32) -> HICON {
    // SAFETY: The resource id is embedded by build.rs as an RT_GROUP_ICON resource.
    // LR_SHARED returns a process-shared icon handle that the window must not destroy.
    unsafe {
        LoadImageW(
            instance,
            app_icon_resource_name(),
            IMAGE_ICON,
            width,
            height,
            LR_DEFAULTCOLOR | LR_SHARED,
        ) as HICON
    }
}

fn app_icon_resource_name() -> *const u16 {
    APP_ICON_RESOURCE_ID as usize as *const u16
}

fn create_main_window(
    instance: HINSTANCE,
    class_name: *const u16,
    title: *const u16,
    app: ViewerApp,
    save_config_on_destroy: bool,
    startup_image_path: Option<PathBuf>,
) -> Result<HWND, Win32Error> {
    let (x, y, width, height) = window_creation_bounds(app.window_bounds());
    let mut context = WindowCreationContext {
        app: Some(app),
        save_config_on_destroy,
        startup_image_path,
        attached_to_hwnd: false,
    };

    // SAFETY: class_name and title are null-terminated UTF-16 buffers that outlive the call.
    // context is a stack value kept alive for the synchronous WM_NCCREATE/WM_CREATE messages.
    let hwnd = unsafe {
        CreateWindowExW(
            0,
            class_name,
            title,
            WS_OVERLAPPEDWINDOW,
            x,
            y,
            width,
            height,
            null_mut(),
            null_mut(),
            instance,
            (&mut context as *mut WindowCreationContext).cast(),
        )
    };

    if hwnd.is_null() {
        Err(Win32Error::CreateWindow { code: last_error() })
    } else {
        Ok(hwnd)
    }
}

fn window_creation_bounds(window_bounds: Option<WindowBounds>) -> (i32, i32, i32, i32) {
    window_bounds.map_or(
        (CW_USEDEFAULT, CW_USEDEFAULT, INITIAL_WIDTH, INITIAL_HEIGHT),
        |bounds| (bounds.x(), bounds.y(), bounds.width(), bounds.height()),
    )
}

fn message_loop() -> Result<i32, Win32Error> {
    // SAFETY: MSG is a plain Win32 data structure that may be zero-initialized before GetMessageW.
    let mut message: MSG = unsafe { std::mem::zeroed() };

    loop {
        // SAFETY: message is a valid mutable pointer and null hwnd means all thread messages.
        let result = unsafe { GetMessageW(&mut message, null_mut(), 0, 0) };
        match result {
            -1 => return Err(Win32Error::MessageLoop { code: last_error() }),
            0 => return Ok(message.wParam as i32),
            _ => {
                // SAFETY: message was filled by GetMessageW for this thread.
                unsafe {
                    TranslateMessage(&message);
                    DispatchMessageW(&message);
                }
            }
        }
    }
}

#[derive(Debug, PartialEq, Eq)]
enum UiThreadQuitOutcome {
    Posted,
    ProcessExitFallback { post_error: u32 },
}

fn request_ui_thread_quit_after_export_shutdown(ui_thread_id: u32) {
    let _ = request_ui_thread_quit_after_export_shutdown_with(
        || post_quit_to_ui_thread(ui_thread_id),
        || {
            std::thread::sleep(std::time::Duration::from_millis(
                UI_THREAD_QUIT_POST_RETRY_DELAY_MS,
            ));
        },
        || {
            std::process::exit(0);
        },
        debug_output_line,
    );
}

fn request_ui_thread_quit_after_export_shutdown_with(
    mut post_quit: impl FnMut() -> Result<(), u32>,
    mut wait_before_retry: impl FnMut(),
    mut fallback: impl FnMut(),
    mut log: impl FnMut(&str),
) -> UiThreadQuitOutcome {
    let mut post_error = ERROR_SUCCESS;

    for attempt in 1..=UI_THREAD_QUIT_POST_ATTEMPTS {
        match post_quit() {
            Ok(()) => return UiThreadQuitOutcome::Posted,
            Err(error) => post_error = error,
        }

        if attempt < UI_THREAD_QUIT_POST_ATTEMPTS {
            let message = format!(
                "[j3Pic] export shutdown WM_QUIT PostThreadMessageW failed; Win32 error {post_error}; retrying"
            );
            log(&message);
            wait_before_retry();
        }
    }

    let message = format!(
        "[j3Pic] export shutdown WM_QUIT PostThreadMessageW failed; Win32 error {post_error}; exiting process after workers joined"
    );
    log(&message);
    fallback();
    UiThreadQuitOutcome::ProcessExitFallback { post_error }
}

fn post_quit_to_ui_thread(ui_thread_id: u32) -> Result<(), u32> {
    // SAFETY: The id comes from the destroyed window's GUI thread while its message
    // loop is still active; WM_QUIT carries no Rust-owned pointers.
    if unsafe { PostThreadMessageW(ui_thread_id, WM_QUIT, 0, 0) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

unsafe extern "system" fn wnd_proc(
    hwnd: HWND,
    message: u32,
    wparam: WPARAM,
    lparam: LPARAM,
) -> LRESULT {
    match message {
        WM_NCCREATE => {
            dpi::enable_non_client_dpi_scaling(hwnd);
            attach_app_to_window(hwnd, lparam)
        }
        WM_CREATE => {
            if let Some(title) = with_app_mut(hwnd, |app| {
                app.handle_create();
                app.title().to_owned()
            }) {
                apply_window_icons(hwnd);
                set_window_title(hwnd, &title);
                // SAFETY: hwnd is the newly created top-level window and can receive shell drops.
                unsafe {
                    DragAcceptFiles(hwnd, 1);
                }
                queue_startup_image_open(hwnd);
                0
            } else {
                -1
            }
        }
        WM_SIZE => {
            let width = low_word(lparam);
            let height = high_word(lparam);
            handle_window_size(hwnd, width, height);
            0
        }
        WM_KEYDOWN | WM_SYSKEYDOWN => {
            if let Some(command) = command_from_key_message(hwnd, wparam) {
                handle_key_command(hwnd, command);
                0
            } else {
                // SAFETY: Default processing for unhandled key messages.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_DPICHANGED => {
            handle_dpi_changed(hwnd, lparam);
            0
        }
        WM_ENTERSIZEMOVE => {
            handle_enter_size_move(hwnd);
            0
        }
        WM_EXITSIZEMOVE => {
            handle_exit_size_move(hwnd);
            0
        }
        WM_MOUSEWHEEL => {
            handle_mouse_wheel(hwnd, wparam, lparam);
            0
        }
        WM_LBUTTONDOWN => {
            handle_left_button_down(hwnd, lparam);
            0
        }
        WM_MOUSEMOVE => {
            handle_mouse_move(hwnd, lparam);
            0
        }
        WM_LBUTTONUP => {
            handle_left_button_up(hwnd);
            0
        }
        WM_CONTEXTMENU => {
            show_viewer_context_menu(hwnd, lparam);
            0
        }
        WM_COMMAND => {
            if handle_window_command(hwnd, wparam) {
                0
            } else {
                // SAFETY: Default processing for command ids not owned by this window.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_CAPTURECHANGED => {
            handle_capture_changed(hwnd);
            0
        }
        WM_CANCELMODE => {
            cancel_active_pan(hwnd);
            // SAFETY: Default processing completes Win32's own modal/capture cancellation.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        WM_DROPFILES => {
            handle_drop_files(hwnd, wparam as HDROP);
            0
        }
        WM_IMAGE_DECODED => {
            handle_decode_worker_messages(hwnd);
            0
        }
        WM_IMAGE_EXPORTED => {
            handle_export_worker_messages(hwnd);
            0
        }
        WM_OPEN_STARTUP_IMAGE => {
            open_startup_image(hwnd);
            0
        }
        WM_TIMER => {
            if wparam == ANIMATION_TIMER_ID {
                handle_animation_timer(hwnd);
                0
            } else if wparam == DECODE_NOTIFICATION_TIMER_ID {
                kill_decode_notification_timer(hwnd);
                handle_decode_worker_messages(hwnd);
                0
            } else if wparam == EXPORT_NOTIFICATION_TIMER_ID {
                kill_export_notification_timer(hwnd);
                handle_export_worker_messages(hwnd);
                0
            } else if wparam == INTERACTIVE_RENDER_SETTLE_TIMER_ID {
                handle_interactive_render_settle_timer(hwnd);
                0
            } else {
                // SAFETY: Default processing for timer messages not owned by the viewer.
                unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
            }
        }
        WM_ERASEBKGND => 1,
        WM_PAINT => {
            if let Some(needs_render_settle) = with_window_state_mut(hwnd, |state| {
                paint_window(
                    hwnd,
                    &mut state.app,
                    &mut state.paint_cache,
                    &mut state.paint_buffer,
                    &state.ui_metrics,
                );
                state.app.handle_paint();
                state.app.has_deferred_scaling_cache_rebuild()
            }) {
                if needs_render_settle {
                    schedule_interactive_render_settle_or_finish(hwnd);
                }
            } else {
                paint_empty_window(hwnd);
            }
            0
        }
        WM_DESTROY => {
            // SAFETY: hwnd is the window currently being destroyed; this reads its GUI
            // thread id so a background export shutdown joiner can post WM_QUIT.
            let ui_thread_id = unsafe { GetWindowThreadProcessId(hwnd, null_mut()) };
            let should_post_quit = with_window_state_mut(hwnd, |state| {
                end_pan_and_release_capture(hwnd, &mut state.app);
                kill_animation_timer(hwnd);
                kill_decode_notification_timer(hwnd);
                kill_export_notification_timer(hwnd);
                kill_interactive_render_settle_timer(hwnd);
                save_app_config_on_destroy(hwnd, state);
                state.decoder.shutdown();
                let export_shutdown = state
                    .exporter
                    .shutdown(move || request_ui_thread_quit_after_export_shutdown(ui_thread_id));
                state.app.handle_destroy();
                export_shutdown == ExportShutdownOutcome::Complete
            })
            .unwrap_or(true);
            // SAFETY: hwnd is being destroyed, so shell drop acceptance can be disabled.
            unsafe {
                DragAcceptFiles(hwnd, 0);
            }
            if should_post_quit {
                // SAFETY: Posting WM_QUIT for the current GUI thread has no Rust-side invariants.
                unsafe {
                    PostQuitMessage(0);
                }
            }
            0
        }
        WM_NCDESTROY => {
            drop(take_window_state_from_window(hwnd));
            // SAFETY: Delegating unhandled non-client destruction to DefWindowProcW.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
        _ => {
            // SAFETY: Default processing for messages this skeleton does not handle.
            unsafe { DefWindowProcW(hwnd, message, wparam, lparam) }
        }
    }
}

fn attach_app_to_window(hwnd: HWND, lparam: LPARAM) -> LRESULT {
    if lparam == 0 {
        return 0;
    }

    // SAFETY: During WM_NCCREATE, lparam is a valid CREATESTRUCTW pointer supplied by Win32.
    let create_struct = unsafe { &*(lparam as *const CREATESTRUCTW) };
    let context_ptr = create_struct.lpCreateParams as *mut WindowCreationContext;
    if context_ptr.is_null() {
        return 0;
    }

    // SAFETY: create_main_window passes this context pointer and keeps it alive until
    // CreateWindowExW returns. WM_NCCREATE is delivered synchronously during that call.
    let context = unsafe { &mut *context_ptr };
    let Some(app) = context.app.take() else {
        return 0;
    };

    let state = WindowState {
        app,
        save_config_on_destroy: context.save_config_on_destroy,
        startup_image_path: context.startup_image_path.take(),
        decoder: DecodeController::new(),
        exporter: ExportController::new(),
        fullscreen: FullscreenState::new(),
        ui_metrics: WindowUiMetrics::for_window(hwnd),
        size_move_dpi: SizeMoveDpiState::default(),
        paint_cache: PaintDibCache::new(),
        paint_buffer: ReusableCompatiblePaintBuffer::new(),
    };
    let state_ptr = Box::into_raw(Box::new(state));
    if set_window_long_ptr_checked(hwnd, GWLP_USERDATA, state_ptr as isize).is_err() {
        // SAFETY: state_ptr was just created by Box::into_raw above and has not been
        // attached to the HWND, so this reclaims it on the creation-failure path.
        unsafe {
            drop(Box::from_raw(state_ptr));
        }
        return 0;
    }
    context.attached_to_hwnd = true;

    1
}

fn with_app_mut<R>(hwnd: HWND, action: impl FnOnce(&mut ViewerApp) -> R) -> Option<R> {
    let ptr = window_state_ptr(hwnd)?;

    // SAFETY: Win32 dispatches this window procedure serially on the GUI thread for normal
    // app mutations. The pointer stays valid until WM_NCDESTROY clears GWLP_USERDATA.
    Some(action(unsafe { &mut (*ptr).app }))
}

fn with_window_state_mut<R>(hwnd: HWND, action: impl FnOnce(&mut WindowState) -> R) -> Option<R> {
    let ptr = window_state_ptr(hwnd)?;

    // SAFETY: The pointer is the WindowState Box stored in GWLP_USERDATA and message dispatch
    // gives this function exclusive access for the duration of the call.
    Some(action(unsafe { &mut *ptr }))
}

fn window_state_ptr(hwnd: HWND) -> Option<*mut WindowState> {
    // SAFETY: GWLP_USERDATA is written only by attach_app_to_window with a WindowState pointer.
    let ptr = unsafe { GetWindowLongPtrW(hwnd, GWLP_USERDATA) } as *mut WindowState;
    if ptr.is_null() {
        None
    } else {
        Some(ptr)
    }
}

fn take_window_state_from_window(hwnd: HWND) -> Option<Box<WindowState>> {
    let ptr = window_state_ptr(hwnd)?;

    // SAFETY: Clearing userdata prevents later double-take. Box::from_raw reclaims the exact
    // pointer created by Box::into_raw in attach_app_to_window.
    unsafe {
        let _ = set_window_long_ptr_checked(hwnd, GWLP_USERDATA, 0);
        Some(Box::from_raw(ptr))
    }
}

fn get_window_long_ptr_checked(hwnd: HWND, index: i32) -> Result<isize, u32> {
    // SAFETY: SetLastError affects only the current thread and is required to disambiguate
    // Win32 APIs that return zero for both success and failure.
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    // SAFETY: The caller chooses an index valid for this hwnd. The function only reads Win32
    // window metadata and reports the thread last-error value.
    let value = unsafe { GetWindowLongPtrW(hwnd, index) };
    let code = last_error();
    if value == 0 && code != ERROR_SUCCESS {
        Err(code)
    } else {
        Ok(value)
    }
}

fn set_window_long_ptr_checked(hwnd: HWND, index: i32, value: isize) -> Result<isize, u32> {
    // SAFETY: SetLastError affects only the current thread and is required to disambiguate
    // Win32 APIs that return zero for both success and failure.
    unsafe {
        SetLastError(ERROR_SUCCESS);
    }
    // SAFETY: The caller chooses an index valid for this hwnd. The value is either a Win32
    // style value or the exact WindowState pointer managed by this module.
    let previous = unsafe { SetWindowLongPtrW(hwnd, index, value) };
    let code = last_error();
    if previous == 0 && code != ERROR_SUCCESS {
        Err(code)
    } else {
        Ok(previous)
    }
}

fn save_app_config_on_destroy(hwnd: HWND, state: &mut WindowState) {
    state
        .app
        .set_window_bounds(saved_window_bounds_for_config(hwnd, &state.fullscreen));
    if !state.save_config_on_destroy {
        debug_output_line(
            "[j3Pic] skipped config save on destroy after startup config load failure",
        );
        return;
    }

    let config = state.app.config_snapshot();
    if let Err(error) = save_app_config(&config) {
        debug_output_line(&format!(
            "[j3Pic] config save on destroy failed; internal={}; source={}",
            error,
            error_source_text(&error)
        ));
    }
}

fn saved_window_bounds_for_config(
    hwnd: HWND,
    fullscreen: &FullscreenState,
) -> Option<WindowBounds> {
    let placement = fullscreen
        .restore
        .map(|restore| restore.placement)
        .or_else(|| current_window_placement(hwnd).ok())?;
    window_bounds_from_placement(&placement)
}

fn window_bounds_from_placement(placement: &WINDOWPLACEMENT) -> Option<WindowBounds> {
    let rect = placement.rcNormalPosition;
    WindowBounds::new(
        rect.left,
        rect.top,
        rect.right.saturating_sub(rect.left),
        rect.bottom.saturating_sub(rect.top),
    )
}

fn toggle_fullscreen(hwnd: HWND) {
    let result = if is_window_fullscreen(hwnd) {
        exit_fullscreen(hwnd)
    } else {
        enter_fullscreen(hwnd)
    };
    handle_fullscreen_transition(hwnd, result);
}

fn exit_fullscreen_or_quit(hwnd: HWND) {
    if is_window_fullscreen(hwnd) {
        handle_fullscreen_transition(hwnd, exit_fullscreen(hwnd));
    } else {
        quit_window(hwnd);
    }
}

fn handle_fullscreen_transition(hwnd: HWND, result: Result<bool, FullscreenError>) {
    match result {
        Ok(true) => {
            refresh_view_after_window_bounds_change(hwnd);
            invalidate_window_after_interactive_view_change(hwnd);
        }
        Ok(false) => {}
        Err(error) => show_error_message(hwnd, &error.user_message_for(viewer_ui_language(hwnd))),
    }
}

fn enter_fullscreen(hwnd: HWND) -> Result<bool, FullscreenError> {
    if is_window_fullscreen(hwnd) {
        return Ok(false);
    }

    let placement = current_window_placement(hwnd)?;
    let style = current_window_style(hwnd)?;
    let monitor_rect = monitor_rect_for_window(hwnd)?;
    let restore = FullscreenRestoreState { style, placement };

    if !set_fullscreen_restore(hwnd, Some(restore)) {
        return Err(FullscreenError::MissingWindowState);
    }

    if let Err(error) = set_window_style(hwnd, style & !(WS_OVERLAPPEDWINDOW as isize)) {
        let _ = set_fullscreen_restore(hwnd, None);
        return Err(error);
    }

    let width = monitor_rect.right.saturating_sub(monitor_rect.left);
    let height = monitor_rect.bottom.saturating_sub(monitor_rect.top);
    // SAFETY: hwnd is live, hWndInsertAfter null is HWND_TOP, and monitor_rect comes from Win32.
    let positioned = unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            monitor_rect.left,
            monitor_rect.top,
            width,
            height,
            SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
        )
    } != 0;

    if positioned {
        Ok(true)
    } else {
        let code = last_error();
        let _ = restore_windowed_window(hwnd, &restore);
        let _ = set_fullscreen_restore(hwnd, None);
        Err(FullscreenError::SetWindowPos { code })
    }
}

fn exit_fullscreen(hwnd: HWND) -> Result<bool, FullscreenError> {
    let Some(restore) = fullscreen_restore(hwnd) else {
        return Ok(false);
    };

    restore_windowed_window(hwnd, &restore)?;
    let _ = set_fullscreen_restore(hwnd, None);
    Ok(true)
}

fn current_window_placement(hwnd: HWND) -> Result<WINDOWPLACEMENT, FullscreenError> {
    // SAFETY: WINDOWPLACEMENT is a Win32 data structure that must have length initialized.
    let mut placement: WINDOWPLACEMENT = unsafe { std::mem::zeroed() };
    placement.length = std::mem::size_of::<WINDOWPLACEMENT>() as u32;

    // SAFETY: hwnd is live and placement is valid writable storage.
    if unsafe { GetWindowPlacement(hwnd, &mut placement) } == 0 {
        Err(FullscreenError::GetWindowPlacement { code: last_error() })
    } else {
        Ok(placement)
    }
}

fn current_window_style(hwnd: HWND) -> Result<isize, FullscreenError> {
    get_window_long_ptr_checked(hwnd, GWL_STYLE)
        .map_err(|code| FullscreenError::GetWindowStyle { code })
}

fn set_window_style(hwnd: HWND, style: isize) -> Result<(), FullscreenError> {
    set_window_long_ptr_checked(hwnd, GWL_STYLE, style)
        .map(|_| ())
        .map_err(|code| FullscreenError::SetWindowStyle { code })
}

fn monitor_rect_for_window(hwnd: HWND) -> Result<RECT, FullscreenError> {
    // SAFETY: hwnd is live. MONITOR_DEFAULTTONEAREST asks Win32 to choose a monitor.
    let monitor = unsafe { MonitorFromWindow(hwnd, MONITOR_DEFAULTTONEAREST) };
    if monitor.is_null() {
        return Err(FullscreenError::GetMonitorInfo { code: last_error() });
    }

    // SAFETY: MONITORINFO is a Win32 data structure that must have cbSize initialized.
    let mut monitor_info: MONITORINFO = unsafe { std::mem::zeroed() };
    monitor_info.cbSize = std::mem::size_of::<MONITORINFO>() as u32;

    // SAFETY: monitor was returned by MonitorFromWindow and monitor_info is valid storage.
    if unsafe { GetMonitorInfoW(monitor, &mut monitor_info) } == 0 {
        Err(FullscreenError::GetMonitorInfo { code: last_error() })
    } else {
        Ok(monitor_info.rcMonitor)
    }
}

fn restore_windowed_window(
    hwnd: HWND,
    restore: &FullscreenRestoreState,
) -> Result<(), FullscreenError> {
    set_window_style(hwnd, restore.style)?;

    // SAFETY: restore.placement was captured from GetWindowPlacement for this hwnd.
    if unsafe { SetWindowPlacement(hwnd, &restore.placement) } == 0 {
        return Err(FullscreenError::SetWindowPlacement { code: last_error() });
    }

    // SAFETY: hwnd is live. Position and size are supplied by SetWindowPlacement above;
    // this call forces the non-client frame to be recalculated for the restored style.
    if unsafe {
        SetWindowPos(
            hwnd,
            null_mut(),
            0,
            0,
            0,
            0,
            SWP_NOMOVE | SWP_NOSIZE | SWP_NOZORDER | SWP_NOOWNERZORDER | SWP_FRAMECHANGED,
        )
    } == 0
    {
        Err(FullscreenError::SetWindowPos { code: last_error() })
    } else {
        Ok(())
    }
}

fn is_window_fullscreen(hwnd: HWND) -> bool {
    let Some(ptr) = window_state_ptr(hwnd) else {
        return false;
    };

    // SAFETY: The pointer is owned by this hwnd until WM_NCDESTROY.
    unsafe { (*ptr).fullscreen.is_fullscreen() }
}

fn fullscreen_restore(hwnd: HWND) -> Option<FullscreenRestoreState> {
    let ptr = window_state_ptr(hwnd)?;

    // SAFETY: The pointer is owned by this hwnd until WM_NCDESTROY.
    unsafe { (*ptr).fullscreen.restore }
}

fn set_fullscreen_restore(hwnd: HWND, restore: Option<FullscreenRestoreState>) -> bool {
    let Some(ptr) = window_state_ptr(hwnd) else {
        return false;
    };

    // SAFETY: The pointer is owned by this hwnd until WM_NCDESTROY. The mutable access is kept
    // within this statement so Win32 calls outside this helper do not hold a Rust borrow.
    unsafe {
        (*ptr).fullscreen.restore = restore;
    }
    true
}

fn refresh_view_after_window_bounds_change(hwnd: HWND) {
    let _ = with_window_state_mut(hwnd, |state| {
        end_pan_and_release_capture(hwnd, &mut state.app);
        if let Some((width, height)) = client_size(hwnd) {
            let client_rect = client_rect_from_size(width, height);
            resize_app_to_image_content_rect(&client_rect, &mut state.app, &state.ui_metrics);
        }
    });
}

fn handle_window_size(hwnd: HWND, width: i32, height: i32) {
    let updated = with_window_state_mut(hwnd, |state| {
        if state.size_move_dpi.should_defer_view_refresh() {
            return false;
        }

        let client_rect = client_rect_from_size(width, height);
        resize_app_to_image_content_rect(&client_rect, &mut state.app, &state.ui_metrics);
        true
    });

    if updated == Some(true) {
        invalidate_window_after_interactive_view_change(hwnd);
    }
}

fn handle_enter_size_move(hwnd: HWND) {
    let _ = with_window_state_mut(hwnd, |state| {
        state.size_move_dpi.enter_size_move();
        if state.app.has_deferred_scaling_cache_rebuild() {
            state.size_move_dpi.defer_render_settle_until_exit();
        }
    });
    kill_interactive_render_settle_timer(hwnd);
}

fn handle_exit_size_move(hwnd: HWND) {
    let exit_outcome = with_window_state_mut(hwnd, |state| state.size_move_dpi.exit_size_move())
        .unwrap_or_default();

    let mut render_settle_scheduled = false;
    if exit_outcome.dpi_changed {
        let _ = with_window_state_mut(hwnd, |state| {
            state.ui_metrics = WindowUiMetrics::for_window(hwnd);
            if let Some((width, height)) = client_size(hwnd) {
                let client_rect = client_rect_from_size(width, height);
                resize_app_to_image_content_rect(&client_rect, &mut state.app, &state.ui_metrics);
            }
        });
        invalidate_window_after_interactive_view_change(hwnd);
        render_settle_scheduled = true;
    }

    if exit_outcome.render_settle_pending && !render_settle_scheduled {
        schedule_interactive_render_settle_or_finish(hwnd);
    }
}

fn handle_dpi_changed(hwnd: HWND, lparam: LPARAM) {
    let should_apply_suggested_rect = with_window_state_mut(hwnd, |state| {
        state
            .size_move_dpi
            .should_apply_suggested_rect_for_dpi_change()
    })
    .unwrap_or(true);

    if should_apply_suggested_rect {
        apply_suggested_dpi_rect(hwnd, lparam);
    } else {
        return;
    }

    let _ = with_window_state_mut(hwnd, |state| {
        state.ui_metrics = WindowUiMetrics::for_window(hwnd);
        end_pan_and_release_capture(hwnd, &mut state.app);
        if let Some((width, height)) = client_size(hwnd) {
            let client_rect = client_rect_from_size(width, height);
            resize_app_to_image_content_rect(&client_rect, &mut state.app, &state.ui_metrics);
        }
    });
    invalidate_window_after_interactive_view_change(hwnd);
}

fn sync_app_viewport_to_current_client_rect(hwnd: HWND) {
    let _ = with_window_state_mut(hwnd, |state| {
        resize_app_to_current_image_content_rect(hwnd, state);
    });
}

fn resize_app_to_current_image_content_rect(hwnd: HWND, state: &mut WindowState) {
    if let Some((width, height)) = client_size(hwnd) {
        let client_rect = client_rect_from_size(width, height);
        resize_app_to_image_content_rect(&client_rect, &mut state.app, &state.ui_metrics);
    }
}

fn apply_suggested_dpi_rect(hwnd: HWND, lparam: LPARAM) {
    let Some(suggested) = dpi::suggested_rect_from_dpi_change(lparam) else {
        return;
    };
    let width = suggested.right.saturating_sub(suggested.left);
    let height = suggested.bottom.saturating_sub(suggested.top);
    if width <= 0 || height <= 0 {
        return;
    }

    // SAFETY: hwnd is live while handling WM_DPICHANGED. The suggested rectangle comes from
    // Win32 and keeping Z-order stable avoids changing activation during DPI migration.
    unsafe {
        let _ = SetWindowPos(
            hwnd,
            null_mut(),
            suggested.left,
            suggested.top,
            width,
            height,
            SWP_NOZORDER | SWP_NOOWNERZORDER,
        );
    }
}

fn cancel_active_pan(hwnd: HWND) {
    let _ = with_app_mut(hwnd, |app| {
        end_pan_and_release_capture(hwnd, app);
    });
}

fn end_pan_and_release_capture(hwnd: HWND, app: &mut ViewerApp) {
    if app.end_pan() || captured_window() == hwnd {
        // SAFETY: This only releases capture owned by the current GUI thread. It is safe if
        // capture has already been lost between the state transition and this call.
        unsafe {
            let _ = ReleaseCapture();
        }
    }
}

fn captured_window() -> HWND {
    // SAFETY: GetCapture reads the capture window for the current thread.
    unsafe { GetCapture() }
}

fn client_size(hwnd: HWND) -> Option<(i32, i32)> {
    let client_rect = client_rect(hwnd)?;
    Some((
        client_rect.right.saturating_sub(client_rect.left),
        client_rect.bottom.saturating_sub(client_rect.top),
    ))
}

fn client_rect_from_size(width: i32, height: i32) -> RECT {
    RECT {
        left: 0,
        top: 0,
        right: width.max(0),
        bottom: height.max(0),
    }
}

fn client_rect(hwnd: HWND) -> Option<RECT> {
    // SAFETY: RECT is a plain Win32 data structure filled by GetClientRect.
    let mut client_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is live and client_rect is valid writable storage.
    if unsafe { GetClientRect(hwnd, &mut client_rect) } == 0 {
        None
    } else {
        Some(client_rect)
    }
}

fn command_from_key_message(hwnd: HWND, wparam: WPARAM) -> Option<Command> {
    let key = key_code_from_wparam(wparam)?;
    let context = if with_app_mut(hwnd, |app| app.has_animation()).unwrap_or(false) {
        CommandContext::AnimationImage
    } else {
        CommandContext::StaticImage
    };
    command_for_key_input_with_context(KeyInput::new(key, current_key_modifiers()), context)
}

fn key_code_from_wparam(wparam: WPARAM) -> Option<KeyCode> {
    match wparam as u16 {
        KEY_C => Some(KeyCode::C),
        KEY_O => Some(KeyCode::O),
        KEY_P => Some(KeyCode::P),
        KEY_Q => Some(KeyCode::Q),
        KEY_R => Some(KeyCode::R),
        KEY_S => Some(KeyCode::S),
        KEY_0 => Some(KeyCode::Digit0),
        KEY_1 => Some(KeyCode::Digit1),
        VK_OEM_PLUS_KEY => Some(KeyCode::Equals),
        VK_OEM_MINUS_KEY => Some(KeyCode::Minus),
        VK_ADD_KEY => Some(KeyCode::NumpadAdd),
        VK_SUBTRACT_KEY => Some(KeyCode::NumpadSubtract),
        VK_LEFT => Some(KeyCode::Left),
        VK_RIGHT => Some(KeyCode::Right),
        VK_SPACE => Some(KeyCode::Space),
        VK_BACK => Some(KeyCode::Backspace),
        VK_PRIOR => Some(KeyCode::PageUp),
        VK_NEXT => Some(KeyCode::PageDown),
        VK_HOME => Some(KeyCode::Home),
        VK_OEM_4_KEY => Some(KeyCode::BracketLeft),
        VK_OEM_6_KEY => Some(KeyCode::BracketRight),
        VK_F4 => Some(KeyCode::F4),
        VK_F11 => Some(KeyCode::F11),
        VK_RETURN => Some(KeyCode::Enter),
        VK_ESCAPE => Some(KeyCode::Escape),
        _ => None,
    }
}

fn current_key_modifiers() -> KeyModifiers {
    KeyModifiers::new(
        is_key_pressed(VK_CONTROL),
        is_key_pressed(VK_SHIFT),
        is_key_pressed(VK_MENU),
    )
}

fn is_key_pressed(virtual_key: u16) -> bool {
    // SAFETY: GetKeyState reads the current thread key state for a virtual key code.
    unsafe { GetKeyState(virtual_key as i32) < 0 }
}

fn open_image_from_dialog(hwnd: HWND) {
    let initial_dir =
        with_app_mut(hwnd, |app| app.recent_folder().map(Path::to_path_buf)).flatten();
    match choose_image_file(hwnd, initial_dir.as_deref()) {
        Ok(Some(path)) => load_image_path(hwnd, path),
        Ok(None) => {}
        Err(_) => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not open the file open dialog.",
                "파일 열기 대화상자를 열 수 없습니다.",
            ),
        ),
    }
}

fn choose_image_file(hwnd: HWND, initial_dir: Option<&Path>) -> Result<Option<PathBuf>, u32> {
    let mut file_buffer = vec![0u16; OPEN_FILE_BUFFER_CHARS];
    let filter = open_file_filter();
    let title = wide_null("Open Image");
    let initial_dir = initial_dir.map(path_wide_null);

    let mut open_file_name = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file_buffer.as_mut_ptr(),
        nMaxFile: file_buffer.len() as u32,
        lpstrTitle: title.as_ptr(),
        lpstrInitialDir: initial_dir.as_ref().map_or(null(), |path| path.as_ptr()),
        Flags: OFN_EXPLORER
            | OFN_FILEMUSTEXIST
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR
            | OFN_PATHMUSTEXIST,
        ..Default::default()
    };

    // SAFETY: OPENFILENAMEW points to live buffers for the duration of the modal call.
    let selected = unsafe { GetOpenFileNameW(&mut open_file_name) } != 0;
    if selected {
        return Ok(path_from_wide_buffer(&file_buffer));
    }

    // SAFETY: CommDlgExtendedError returns the last common-dialog status for this thread.
    let code = unsafe { CommDlgExtendedError() };
    if code == 0 {
        Ok(None)
    } else {
        Err(code)
    }
}

fn open_file_filter() -> Vec<u16> {
    let label = format!("Supported Images ({OPEN_FILE_FILTER_PATTERNS})");
    let mut filter = Vec::new();
    push_wide_filter_part(&mut filter, &label);
    push_wide_filter_part(&mut filter, OPEN_FILE_FILTER_PATTERNS);
    filter.push(0);
    filter
}

fn push_wide_filter_part(filter: &mut Vec<u16>, part: &str) {
    filter.extend(OsStr::new(part).encode_wide());
    filter.push(0);
}

fn path_from_wide_buffer(buffer: &[u16]) -> Option<PathBuf> {
    let len = buffer
        .iter()
        .position(|value| *value == 0)
        .unwrap_or(buffer.len());
    if len == 0 {
        None
    } else {
        Some(OsString::from_wide(&buffer[..len]).into())
    }
}

struct ExportDialogDefaults {
    source_path: PathBuf,
    original_size: crate::domain::ImageSize,
    default_format: ExportFormat,
    default_quality: u8,
    suggested_path: PathBuf,
}

fn export_image_from_dialog(hwnd: HWND) {
    let Some(defaults) = current_export_defaults(hwnd) else {
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "There is no image to export.",
                "내보낼 이미지가 없습니다.",
            ),
        );
        return;
    };

    let option_defaults = ExportOptionsDialogDefaults::new(
        defaults.original_size,
        defaults.default_format,
        defaults.default_quality,
        false,
    );
    let option_selection =
        match show_export_options_dialog(hwnd, option_defaults, viewer_ui_language(hwnd)) {
            Ok(ExportOptionsDialogOutcome::Accepted(selection)) => selection,
            Ok(ExportOptionsDialogOutcome::Cancelled) => return,
            Err(error) => {
                debug_output_line(&format!(
                    "[j3Pic] export options dialog failed; internal={}; source={}",
                    error,
                    error_source_text(&error)
                ));
                show_error_message(
                    hwnd,
                    viewer_text(
                        hwnd,
                        "Could not open the export options dialog.",
                        "내보내기 옵션창을 열 수 없습니다.",
                    ),
                );
                return;
            }
        };
    let suggested_path =
        export_path_with_format_extension(&defaults.suggested_path, option_selection.format());
    let selection = match choose_export_file(hwnd, &suggested_path, option_selection.format()) {
        Ok(Some(selection)) => selection,
        Ok(None) => return,
        Err(_) => {
            show_error_message(
                hwnd,
                viewer_text(
                    hwnd,
                    "Could not open the file save dialog.",
                    "파일 저장 대화상자를 열 수 없습니다.",
                ),
            );
            return;
        }
    };

    if paths_refer_to_same_existing_file(&defaults.source_path, selection.path()) {
        show_error_message(hwnd, same_source_export_message(viewer_ui_language(hwnd)));
        return;
    }

    if corrected_export_path_requires_overwrite_confirmation(
        selection.selected_path(),
        selection.path(),
    ) && !confirm_overwrite_corrected_export_path(hwnd, selection.path())
    {
        return;
    }

    start_image_export(
        hwnd,
        selection.path(),
        option_selection.format(),
        option_selection.quality(),
        option_selection.rotation(),
        option_selection.target_size(),
        option_selection.remove_metadata(),
    );
}

fn start_image_export(
    hwnd: HWND,
    path: &Path,
    format: ExportFormat,
    quality: Option<u8>,
    rotation: crate::domain::ImageRotation,
    target_size: Option<crate::domain::ImageSize>,
    remove_metadata: bool,
) {
    let started = with_window_state_mut(hwnd, |state| {
        let app = &mut state.app;
        let exporter = &mut state.exporter;
        exporter.start_export(
            hwnd,
            || {
                let options = app
                    .export_options(format, quality)
                    .with_rotation(rotation)
                    .with_target_size(target_size)
                    .with_remove_metadata(remove_metadata);
                app.begin_current_image_export(path, options)
            },
            quality,
        )
    });
    match started {
        Some(Ok(ExportStartOutcome::Started)) => {}
        Some(Ok(ExportStartOutcome::Busy)) => {
            show_error_message(
                hwnd,
                viewer_text(
                    hwnd,
                    "Image export is already in progress.\n\nTry again after it finishes.",
                    "이미지 내보내기가 아직 진행 중입니다.\n\n완료된 뒤 다시 시도해 주세요.",
                ),
            );
        }
        Some(Err(error)) => {
            debug_log_viewer_error("image export worker start failed", &error);
            show_viewer_error_message(hwnd, &error);
        }
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not export the image because the window state was not found.",
                "창 상태를 찾을 수 없어 이미지를 내보낼 수 없습니다.",
            ),
        ),
    }
}

fn current_export_defaults(hwnd: HWND) -> Option<ExportDialogDefaults> {
    with_app_mut(hwnd, |app| {
        let source_path = app.current_source_path()?.to_path_buf();
        let source_format = app.current_source_format()?;
        let original_size = app.current_export_source_size()?;
        let suggested_path = app.suggested_export_path(&source_path, source_format);
        Some(ExportDialogDefaults {
            source_path,
            original_size,
            default_format: app.default_export_format_for_source(source_format),
            default_quality: app.export_default_quality(),
            suggested_path,
        })
    })
    .flatten()
}

fn choose_export_file(
    hwnd: HWND,
    suggested_path: &Path,
    format: ExportFormat,
) -> Result<Option<ExportFileSelection>, u32> {
    let mut file_buffer = vec![0u16; OPEN_FILE_BUFFER_CHARS];
    fill_wide_file_buffer(&mut file_buffer, suggested_path);

    let filter = export_file_filter(format);
    let title = wide_null("Export Image");
    let default_extension = wide_null(export_format_default_extension(format));

    let mut open_file_name = OPENFILENAMEW {
        lStructSize: std::mem::size_of::<OPENFILENAMEW>() as u32,
        hwndOwner: hwnd,
        lpstrFilter: filter.as_ptr(),
        lpstrFile: file_buffer.as_mut_ptr(),
        nMaxFile: file_buffer.len() as u32,
        lpstrTitle: title.as_ptr(),
        lpstrDefExt: default_extension.as_ptr(),
        nFilterIndex: 1,
        Flags: OFN_EXPLORER
            | OFN_HIDEREADONLY
            | OFN_NOCHANGEDIR
            | OFN_OVERWRITEPROMPT
            | OFN_PATHMUSTEXIST,
        ..Default::default()
    };

    // SAFETY: OPENFILENAMEW points to live buffers for the duration of the modal call.
    let selected = unsafe { GetSaveFileNameW(&mut open_file_name) } != 0;
    if selected {
        let Some(selected_path) = path_from_wide_buffer(&file_buffer) else {
            return Ok(None);
        };
        return Ok(Some(ExportFileSelection::from_selected_path(
            selected_path,
            format,
        )));
    }

    // SAFETY: CommDlgExtendedError returns the last common-dialog status for this thread.
    let code = unsafe { CommDlgExtendedError() };
    if code == 0 {
        Ok(None)
    } else {
        Err(code)
    }
}

fn fill_wide_file_buffer(buffer: &mut [u16], path: &Path) {
    buffer.fill(0);
    if buffer.is_empty() {
        return;
    }

    let wide_path = path.as_os_str().encode_wide().collect::<Vec<_>>();
    let len = wide_path.len().min(buffer.len().saturating_sub(1));
    buffer[..len].copy_from_slice(&wide_path[..len]);
}

fn export_file_filter(format: ExportFormat) -> Vec<u16> {
    let mut filter = Vec::new();
    let (label, pattern) = export_file_filter_label_and_pattern(format);
    push_wide_filter_part(&mut filter, label);
    push_wide_filter_part(&mut filter, pattern);
    filter.push(0);
    filter
}

fn export_file_filter_label_and_pattern(format: ExportFormat) -> (&'static str, &'static str) {
    match format {
        ExportFormat::Png => ("PNG image (*.png)", "*.png"),
        ExportFormat::Jpeg => ("JPEG image (*.jpg;*.jpeg)", "*.jpg;*.jpeg"),
        ExportFormat::Bmp => ("Bitmap image (*.bmp)", "*.bmp"),
        ExportFormat::Webp => ("WebP image (*.webp)", "*.webp"),
        ExportFormat::Ico => ("Icon image (*.ico)", "*.ico"),
    }
}

fn confirm_overwrite_corrected_export_path(hwnd: HWND, path: &Path) -> bool {
    let message = corrected_export_overwrite_message(viewer_ui_language(hwnd), path);
    let text = wide_null(&message);
    let caption = wide_null("j3Pic");

    // SAFETY: text and caption are null-terminated UTF-16 buffers valid for the call.
    unsafe {
        MessageBoxW(
            hwnd,
            text.as_ptr(),
            caption.as_ptr(),
            MB_YESNO | MB_ICONWARNING,
        ) == IDYES
    }
}

fn handle_drop_files(hwnd: HWND, hdrop: HDROP) {
    if hdrop.is_null() {
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not find dropped file information.",
                "드롭된 파일 정보를 찾을 수 없습니다.",
            ),
        );
        return;
    }

    let path = first_supported_drop_path(hdrop);
    // SAFETY: WM_DROPFILES transfers an HDROP handle that must be released with DragFinish.
    unsafe {
        DragFinish(hdrop);
    }

    if let Some(path) = path {
        load_image_path(hwnd, path);
    } else {
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not find a supported image file in the dropped items.\n\nSupported formats: jpg, jpeg, png, bmp, gif, webp, ico, tif, tiff, tga",
                "드롭한 항목에서 지원하는 이미지 파일을 찾지 못했습니다.\n\n지원 형식: jpg, jpeg, png, bmp, gif, webp, ico, tif, tiff, tga",
            ),
        );
    }
}

fn first_supported_drop_path(hdrop: HDROP) -> Option<PathBuf> {
    // SAFETY: hdrop is provided by WM_DROPFILES and remains valid until DragFinish.
    let count = unsafe { DragQueryFileW(hdrop, DROP_QUERY_FILE_COUNT, null_mut(), 0) };
    let mut buffer = Vec::new();
    for index in 0..count.min(MAX_DROPPED_FILE_PATHS_TO_SCAN) {
        let Some(path) = dropped_file_path_wide(hdrop, index, &mut buffer) else {
            continue;
        };
        if dropped_path_has_supported_image_extension(path) {
            return Some(OsString::from_wide(path).into());
        }
    }

    None
}

fn dropped_file_path_wide(hdrop: HDROP, index: u32, buffer: &mut Vec<u16>) -> Option<&[u16]> {
    // SAFETY: hdrop is valid during WM_DROPFILES handling. A null buffer queries length.
    let len = unsafe { DragQueryFileW(hdrop, index, null_mut(), 0) };
    if len == 0 {
        return None;
    }

    let required_chars = len.checked_add(1)?;
    let required_len = usize::try_from(required_chars).ok()?;
    if buffer.len() < required_len {
        buffer.resize(required_len, 0);
    }

    // SAFETY: buffer is valid writable UTF-16 storage with room for the null terminator.
    let copied = unsafe { DragQueryFileW(hdrop, index, buffer.as_mut_ptr(), required_chars) };
    if copied == 0 {
        return None;
    }

    Some(&buffer[..copied as usize])
}

fn dropped_path_has_supported_image_extension(path: &[u16]) -> bool {
    let Some(extension) = dropped_path_extension(path) else {
        return false;
    };
    if extension.len() > DROP_EXTENSION_ASCII_BUFFER_CHARS {
        return false;
    }

    let mut ascii_extension = [0u8; DROP_EXTENSION_ASCII_BUFFER_CHARS];
    for (index, unit) in extension.iter().copied().enumerate() {
        let Ok(byte) = u8::try_from(unit) else {
            return false;
        };
        if !byte.is_ascii() {
            return false;
        }
        ascii_extension[index] = byte;
    }

    let Ok(extension) = std::str::from_utf8(&ascii_extension[..extension.len()]) else {
        return false;
    };
    SupportedImageFormat::from_extension(extension).is_some()
}

fn dropped_path_extension(path: &[u16]) -> Option<&[u16]> {
    let file_start = path
        .iter()
        .rposition(|unit| *unit == b'\\' as u16 || *unit == b'/' as u16)
        .map_or(0, |separator| separator + 1);
    let file_name = &path[file_start..];
    let dot_index = file_name.iter().rposition(|unit| *unit == b'.' as u16)?;
    if dot_index == 0 {
        return None;
    }

    Some(&file_name[dot_index + 1..])
}

fn queue_startup_image_open(hwnd: HWND) {
    let should_open =
        with_window_state_mut(hwnd, |state| state.startup_image_path.is_some()).unwrap_or(false);
    if !should_open {
        return;
    }

    // SAFETY: hwnd is the newly created viewer window. The posted private message is handled
    // by this same window procedure after synchronous creation and initial sizing complete.
    if unsafe { PostMessageW(hwnd, WM_OPEN_STARTUP_IMAGE, 0, 0) } == 0 {
        open_startup_image(hwnd);
    }
}

fn open_startup_image(hwnd: HWND) {
    let path = with_window_state_mut(hwnd, |state| state.startup_image_path.take()).flatten();
    if let Some(path) = path {
        load_image_path(hwnd, path);
    }
}

fn load_image_path(hwnd: HWND, path: PathBuf) {
    cancel_active_pan(hwnd);
    kill_animation_timer(hwnd);
    cancel_deferred_render_settle(hwnd);
    let request = with_app_mut(hwnd, |app| app.begin_image_decode(path));

    match request {
        Some(request) => start_initial_decode(hwnd, request),
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not open the image because the window state was not found.",
                "창 상태를 찾을 수 없어 이미지를 열 수 없습니다.",
            ),
        ),
    }
}

fn navigate_image(hwnd: HWND, direction: ImageNavigationDirection) {
    cancel_active_pan(hwnd);
    kill_animation_timer(hwnd);
    cancel_deferred_render_settle(hwnd);
    let outcome = with_app_mut(hwnd, |app| {
        let outcome = app.begin_navigation_or_use_preloaded(direction);
        let title = app.title().to_owned();
        (outcome, title)
    });

    match outcome {
        Some((NavigationStartOutcome::Decode(request), _)) => start_initial_decode(hwnd, request),
        Some((NavigationStartOutcome::AppliedPreloaded, title)) => {
            sync_app_viewport_to_current_client_rect(hwnd);
            set_window_title(hwnd, &title);
            invalidate_window_after_image_content_change(hwnd);
            update_animation_timer(hwnd);
            start_full_resolution_decode_if_needed(hwnd);
            start_navigation_preloads_if_possible(hwnd);
        }
        Some((NavigationStartOutcome::Noop, _)) => {}
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not navigate images because the window state was not found.",
                "창 상태를 찾을 수 없어 이미지를 이동할 수 없습니다.",
            ),
        ),
    }
}

fn start_initial_decode(hwnd: HWND, request: ImageDecodeRequest) {
    let generation = request.generation();
    let started = with_window_state_mut(hwnd, |state| {
        state.decoder.start_initial_decode(hwnd, request)
    });
    match started {
        Some(Ok(())) => {}
        Some(Err(error)) => handle_initial_decode_start_error(hwnd, generation, error),
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not open the image because the window state was not found.",
                "창 상태를 찾을 수 없어 이미지를 열 수 없습니다.",
            ),
        ),
    }
}

fn start_navigation_preloads_if_possible(hwnd: HWND) {
    let requests = with_app_mut(hwnd, |app| app.navigation_preload_requests());
    if let Some(requests) = requests {
        if !requests.is_empty() {
            let _ = with_window_state_mut(hwnd, |state| {
                state.decoder.start_navigation_preloads(hwnd, requests);
            });
        }
    }
}

fn start_animation_frame_decode(hwnd: HWND, request: AnimationFrameDecodeRequest) {
    let generation = request.generation();
    let path = request.path().to_path_buf();
    let file_version = request.file_version();
    let frame_index = request.frame_index();
    match cached_animation_frame_pixels_for_loaded_image(
        request.path(),
        file_version,
        request.format(),
        request.source_size(),
        frame_index,
        request.viewport(),
        request.memory_policy(),
        None,
    ) {
        Ok(Some(rgba8)) => {
            handle_animation_frame_decode_message(
                hwnd,
                generation,
                path,
                Some(file_version),
                frame_index,
                Ok(rgba8),
            );
            return;
        }
        Ok(None) => {}
        Err(error) => {
            handle_animation_frame_decode_message(
                hwnd,
                generation,
                path,
                Some(file_version),
                frame_index,
                Err(error),
            );
            return;
        }
    }

    let started = with_window_state_mut(hwnd, |state| {
        state.decoder.start_animation_frame_decode(hwnd, request)
    });
    match started {
        Some(Ok(())) => {}
        Some(Err(error)) => {
            handle_animation_frame_decode_start_error(
                hwnd,
                generation,
                path,
                file_version,
                frame_index,
                error,
            );
        }
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not read the animation frame because the window state was not found.",
                "창 상태를 찾을 수 없어 애니메이션 프레임을 읽을 수 없습니다.",
            ),
        ),
    }
}

fn start_full_resolution_decode_if_needed(hwnd: HWND) {
    let request = with_app_mut(hwnd, ViewerApp::begin_full_resolution_decode);
    if let Some(Some(request)) = request {
        let generation = request.generation();
        let file_version = request.file_version();
        let started = with_window_state_mut(hwnd, |state| {
            state.decoder.start_full_resolution_decode(hwnd, request)
        });
        if let Some(Err(error)) = started {
            handle_full_resolution_decode_start_error(hwnd, generation, file_version, error);
        }
    }
}

fn update_animation_timer(hwnd: HWND) {
    let interval = with_app_mut(hwnd, |app| app.animation_timer_interval_ms()).flatten();
    match interval {
        Some(interval) => {
            debug_log_animation_timer(hwnd, "SetTimer", Some(interval));
            if !set_animation_timer(hwnd, interval) {
                show_error_message(
                    hwnd,
                    viewer_text(
                        hwnd,
                        "Could not start the animation timer.",
                        "애니메이션 타이머를 시작하지 못했습니다.",
                    ),
                );
            }
        }
        None => {
            debug_log_animation_timer(hwnd, "KillTimer", None);
            kill_animation_timer(hwnd);
        }
    }
}

fn set_animation_timer(hwnd: HWND, interval_ms: u32) -> bool {
    // SAFETY: hwnd is a live viewer window, ANIMATION_TIMER_ID is owned by this window,
    // and a null callback routes timer events through WM_TIMER.
    unsafe { SetTimer(hwnd, ANIMATION_TIMER_ID, interval_ms, None) != 0 }
}

fn kill_animation_timer(hwnd: HWND) {
    // SAFETY: Killing a missing timer is harmless for viewer state.
    unsafe {
        let _ = KillTimer(hwnd, ANIMATION_TIMER_ID);
    }
}

fn notify_decode_worker_messages(hwnd: HWND) -> DecodeNotificationOutcome {
    notify_decode_worker_messages_with(
        || post_decode_worker_message(hwnd),
        || set_decode_notification_timer(hwnd),
        || send_decode_worker_message(hwnd),
        debug_output_line,
    )
}

fn notify_decode_worker_messages_with(
    mut post_message: impl FnMut() -> Result<(), u32>,
    mut set_timer: impl FnMut() -> Result<(), u32>,
    mut send_message: impl FnMut(),
    mut log: impl FnMut(&str),
) -> DecodeNotificationOutcome {
    let Err(post_error) = post_message() else {
        return DecodeNotificationOutcome::Posted;
    };

    let message = format!(
        "[j3Pic] image decode notification PostMessageW failed; Win32 error {post_error}; falling back to WM_TIMER"
    );
    log(&message);

    match set_timer() {
        Ok(()) => DecodeNotificationOutcome::TimerFallback { post_error },
        Err(timer_error) => {
            let message = format!(
                "[j3Pic] image decode notification SetTimer failed; Win32 error {timer_error}; falling back to SendMessageW"
            );
            log(&message);
            send_message();
            DecodeNotificationOutcome::SendFallback {
                post_error,
                timer_error,
            }
        }
    }
}

fn post_decode_worker_message(hwnd: HWND) -> Result<(), u32> {
    // SAFETY: hwnd is the viewer window that owns the decode controller. The message carries no
    // borrowed pointers and only asks the UI thread to drain its decoder channel.
    if unsafe { PostMessageW(hwnd, WM_IMAGE_DECODED, 0, 0) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn set_decode_notification_timer(hwnd: HWND) -> Result<(), u32> {
    // SAFETY: hwnd is the viewer window and DECODE_NOTIFICATION_TIMER_ID is owned by this window,
    // with a null TIMERPROC so delivery happens through WM_TIMER on the UI thread.
    if unsafe {
        SetTimer(
            hwnd,
            DECODE_NOTIFICATION_TIMER_ID,
            DECODE_NOTIFICATION_TIMER_INTERVAL_MS,
            None,
        )
    } == 0
    {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn kill_decode_notification_timer(hwnd: HWND) {
    // SAFETY: Killing a missing timer is harmless for viewer state.
    unsafe {
        let _ = KillTimer(hwnd, DECODE_NOTIFICATION_TIMER_ID);
    }
}

fn send_decode_worker_message(hwnd: HWND) {
    // SAFETY: hwnd is the viewer window that owns the decode controller. The message carries no
    // borrowed pointers; SendMessageW is only used after async notification paths fail.
    unsafe {
        SendMessageW(hwnd, WM_IMAGE_DECODED, 0, 0);
    }
}

fn notify_export_worker_messages(hwnd: HWND) {
    notify_export_worker_messages_with(
        || post_export_worker_message(hwnd),
        || set_export_notification_timer(hwnd),
        || send_export_worker_message(hwnd),
        debug_output_line,
    );
}

#[derive(Debug, PartialEq, Eq)]
enum ExportNotificationOutcome {
    Posted,
    TimerFallback { post_error: u32 },
    SendFallback { post_error: u32, timer_error: u32 },
}

fn notify_export_worker_messages_with(
    mut post_message: impl FnMut() -> Result<(), u32>,
    mut set_timer: impl FnMut() -> Result<(), u32>,
    mut send_message: impl FnMut(),
    mut log: impl FnMut(&str),
) -> ExportNotificationOutcome {
    let Err(post_error) = post_message() else {
        return ExportNotificationOutcome::Posted;
    };

    let message = format!(
        "[j3Pic] image export notification PostMessageW failed; Win32 error {post_error}; falling back to WM_TIMER"
    );
    log(&message);

    match set_timer() {
        Ok(()) => ExportNotificationOutcome::TimerFallback { post_error },
        Err(timer_error) => {
            let message = format!(
                "[j3Pic] image export notification SetTimer failed; Win32 error {timer_error}; falling back to SendMessageW"
            );
            log(&message);
            send_message();
            ExportNotificationOutcome::SendFallback {
                post_error,
                timer_error,
            }
        }
    }
}

fn post_export_worker_message(hwnd: HWND) -> Result<(), u32> {
    // SAFETY: hwnd is the viewer window that owns the export controller. The message carries no
    // borrowed pointers and only asks the UI thread to drain its exporter channel.
    if unsafe { PostMessageW(hwnd, WM_IMAGE_EXPORTED, 0, 0) } == 0 {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn set_export_notification_timer(hwnd: HWND) -> Result<(), u32> {
    // SAFETY: hwnd is the viewer window and EXPORT_NOTIFICATION_TIMER_ID is owned by this window,
    // with a null TIMERPROC so delivery happens through WM_TIMER on the UI thread.
    if unsafe {
        SetTimer(
            hwnd,
            EXPORT_NOTIFICATION_TIMER_ID,
            EXPORT_NOTIFICATION_TIMER_INTERVAL_MS,
            None,
        )
    } == 0
    {
        Err(last_error())
    } else {
        Ok(())
    }
}

fn kill_export_notification_timer(hwnd: HWND) {
    // SAFETY: Killing a missing timer is harmless for viewer state.
    unsafe {
        let _ = KillTimer(hwnd, EXPORT_NOTIFICATION_TIMER_ID);
    }
}

fn send_export_worker_message(hwnd: HWND) {
    // SAFETY: hwnd is the viewer window that owns the export controller. The message carries no
    // borrowed pointers; SendMessageW is only used after async notification paths fail.
    unsafe {
        SendMessageW(hwnd, WM_IMAGE_EXPORTED, 0, 0);
    }
}

fn handle_initial_decode_start_error(
    hwnd: HWND,
    generation: DecodeGeneration,
    error: ViewerAppError,
) {
    let presentation = with_app_mut(hwnd, |app| {
        app.finish_failed_initial_decode(generation, &error)
    });
    match presentation {
        Some(DecodeFailurePresentation::MessageBox) => {
            debug_log_viewer_error("initial image decode worker start failed", &error);
            show_viewer_error_message(hwnd, &error);
            update_animation_timer(hwnd);
        }
        Some(DecodeFailurePresentation::StatusMessage) => {
            debug_log_viewer_error("navigation image decode worker start failed", &error);
            invalidate_window(hwnd);
            update_animation_timer(hwnd);
        }
        Some(DecodeFailurePresentation::RetryNavigation(request)) => {
            debug_log_viewer_error(
                "navigation image decode worker start failed; retrying",
                &error,
            );
            start_initial_decode(hwnd, request);
        }
        Some(DecodeFailurePresentation::Stale) | None => {}
    }
}

fn handle_full_resolution_decode_start_error(
    hwnd: HWND,
    generation: DecodeGeneration,
    file_version: Option<ImageFileVersion>,
    error: ViewerAppError,
) {
    let outcome = with_app_mut(hwnd, |app| {
        app.finish_failed_decode(generation, file_version)
    });
    if outcome == Some(DecodeApplyOutcome::Applied) {
        debug_log_viewer_error("full-resolution image decode worker start failed", &error);
        show_viewer_error_message(hwnd, &error);
    }
}

fn handle_animation_frame_decode_start_error(
    hwnd: HWND,
    generation: DecodeGeneration,
    path: PathBuf,
    file_version: ImageFileVersion,
    frame_index: usize,
    error: ViewerAppError,
) {
    let outcome = with_app_mut(hwnd, |app| {
        app.finish_failed_animation_frame_decode(
            generation,
            frame_index,
            path.as_path(),
            Some(file_version),
        )
    });
    if outcome == Some(DecodeApplyOutcome::Applied) {
        debug_log_viewer_error("animation frame decode worker start failed", &error);
        show_viewer_error_message(hwnd, &error);
    }
    update_animation_timer(hwnd);
}

fn handle_decode_worker_messages(hwnd: HWND) {
    let drain = with_window_state_mut(hwnd, |state| state.decoder.drain_messages());
    let Some(drain) = drain else {
        return;
    };

    for message in drain.messages {
        match message {
            DecodeWorkerMessage::Initial { generation, result } => {
                handle_initial_decode_message(hwnd, generation, result);
            }
            DecodeWorkerMessage::InitialDecodeCompleted { .. } => {}
            DecodeWorkerMessage::FolderScanned {
                generation,
                path,
                result,
            } => {
                handle_folder_scan_message(hwnd, generation, path, result);
            }
            DecodeWorkerMessage::FolderScanSkipped { generation, path } => {
                handle_folder_scan_skipped_message(hwnd, generation, path);
            }
            DecodeWorkerMessage::FullResolution {
                generation,
                file_version,
                result,
            } => {
                handle_full_resolution_decode_message(hwnd, generation, file_version, result);
            }
            DecodeWorkerMessage::AnimationFrame {
                generation,
                path,
                file_version,
                frame_index,
                result,
            } => {
                handle_animation_frame_decode_message(
                    hwnd,
                    generation,
                    path,
                    file_version,
                    frame_index,
                    result,
                );
            }
            DecodeWorkerMessage::NavigationPreload { request, result } => {
                handle_navigation_preload_message(hwnd, request, result);
            }
        }
    }

    for failure in drain.start_failures {
        handle_decode_start_failure(hwnd, failure);
    }
}

fn handle_export_worker_messages(hwnd: HWND) {
    let messages = with_window_state_mut(hwnd, |state| state.exporter.drain_messages());
    let Some(messages) = messages else {
        return;
    };

    for message in messages {
        match message {
            ExportWorkerMessage::Completed {
                path,
                options,
                quality,
                result,
            } => handle_export_completed_message(hwnd, path, options, quality, result),
        }
    }
}

fn handle_export_completed_message(
    hwnd: HWND,
    path: PathBuf,
    options: ExportOptions,
    quality: Option<u8>,
    result: Result<(), ViewerAppError>,
) {
    match result {
        Ok(()) => {
            let updated = with_app_mut(hwnd, |app| {
                app.finish_current_image_export(&path, options);
                if let Some(quality) = quality {
                    app.set_export_default_quality(quality);
                }
            });
            if updated.is_some() {
                invalidate_window(hwnd);
            }
        }
        Err(error) => {
            debug_log_viewer_error("image export failed", &error);
            show_viewer_error_message(hwnd, &error);
        }
    }
}

fn handle_decode_start_failure(hwnd: HWND, failure: DecodeStartFailure) {
    match failure {
        DecodeStartFailure::Initial { generation, error } => {
            handle_initial_decode_start_error(hwnd, generation, error);
        }
        DecodeStartFailure::FullResolution {
            generation,
            file_version,
            error,
        } => {
            handle_full_resolution_decode_start_error(hwnd, generation, file_version, error);
        }
        DecodeStartFailure::AnimationFrame {
            generation,
            path,
            file_version,
            frame_index,
            error,
        } => {
            handle_animation_frame_decode_start_error(
                hwnd,
                generation,
                path,
                file_version,
                frame_index,
                error,
            );
        }
    }
}

fn handle_initial_decode_message(
    hwnd: HWND,
    generation: DecodeGeneration,
    result: Result<(crate::domain::LoadedImage, ImageFolder), ViewerAppError>,
) {
    match result {
        Ok((image, folder)) => {
            let outcome = with_window_state_mut(hwnd, |state| {
                let outcome = state.app.apply_decoded_image(generation, image, folder);
                if outcome == DecodeApplyOutcome::Applied {
                    resize_app_to_current_image_content_rect(hwnd, state);
                }
                (outcome, state.app.title().to_owned())
            });
            if let Some((DecodeApplyOutcome::Applied, title)) = outcome {
                set_window_title(hwnd, &title);
                invalidate_window_after_image_content_change(hwnd);
                update_animation_timer(hwnd);
                start_full_resolution_decode_if_needed(hwnd);
                start_navigation_preloads_if_possible(hwnd);
            }
        }
        Err(error) => {
            let presentation = with_app_mut(hwnd, |app| {
                app.finish_failed_initial_decode(generation, &error)
            });
            match presentation {
                Some(DecodeFailurePresentation::MessageBox) => {
                    debug_log_viewer_error("initial image decode failed", &error);
                    show_viewer_error_message(hwnd, &error);
                    update_animation_timer(hwnd);
                }
                Some(DecodeFailurePresentation::StatusMessage) => {
                    debug_log_viewer_error("navigation image decode failed", &error);
                    invalidate_window(hwnd);
                    update_animation_timer(hwnd);
                }
                Some(DecodeFailurePresentation::RetryNavigation(request)) => {
                    debug_log_viewer_error("navigation image decode failed; retrying", &error);
                    start_initial_decode(hwnd, request);
                }
                Some(DecodeFailurePresentation::Stale) | None => {}
            }
        }
    }
}

fn handle_folder_scan_message(
    hwnd: HWND,
    generation: DecodeGeneration,
    path: PathBuf,
    result: Result<ImageFolder, ScanImageFolderError>,
) {
    match result {
        Ok(folder) => {
            let result = with_app_mut(hwnd, |app| {
                let outcome = app.apply_scanned_image_folder(generation, path.as_path(), folder);
                let pending_navigation = if outcome == DecodeApplyOutcome::Applied {
                    app.take_pending_navigation_after_folder_scan()
                } else {
                    None
                };
                (outcome, pending_navigation)
            });
            if let Some((DecodeApplyOutcome::Applied, Some(request))) = result {
                cancel_deferred_render_settle(hwnd);
                start_initial_decode(hwnd, request);
            } else if let Some((DecodeApplyOutcome::Applied, None)) = result {
                start_navigation_preloads_if_possible(hwnd);
            }
        }
        Err(error) => {
            let outcome = with_app_mut(hwnd, |app| {
                app.finish_pending_folder_scan_without_update(generation, path.as_path())
            });
            if let Some(DecodeApplyOutcome::Applied) = outcome {
                start_navigation_preloads_if_possible(hwnd);
            }
            debug_log_viewer_error(
                "image folder scan failed after successful image decode",
                &ViewerAppError::from(error),
            );
        }
    }
}

fn handle_folder_scan_skipped_message(hwnd: HWND, generation: DecodeGeneration, path: PathBuf) {
    let outcome = with_app_mut(hwnd, |app| {
        app.finish_pending_folder_scan_without_update(generation, path.as_path())
    });
    if let Some(DecodeApplyOutcome::Applied) = outcome {
        start_navigation_preloads_if_possible(hwnd);
    }
}

fn handle_navigation_preload_message(
    hwnd: HWND,
    request: ImagePreloadRequest,
    result: Result<crate::domain::LoadedImage, LoadImageError>,
) {
    if let Ok(image) = result {
        let _ = with_app_mut(hwnd, |app| {
            app.store_preloaded_navigation_image(&request, image)
        });
    }
}

fn handle_full_resolution_decode_message(
    hwnd: HWND,
    generation: DecodeGeneration,
    file_version: Option<ImageFileVersion>,
    result: Result<PixelImage, LoadImageError>,
) {
    match result {
        Ok(pixels) => {
            let outcome = with_app_mut(hwnd, |app| {
                let outcome = app.apply_full_resolution_image(generation, file_version, pixels);
                (outcome, app.title().to_owned())
            });
            if let Some((DecodeApplyOutcome::Applied, title)) = outcome {
                set_window_title(hwnd, &title);
                invalidate_window_after_image_content_change(hwnd);
                start_navigation_preloads_if_possible(hwnd);
            }
        }
        Err(error) => {
            let outcome = with_app_mut(hwnd, |app| {
                app.finish_failed_decode(generation, file_version)
            });
            if outcome == Some(DecodeApplyOutcome::Applied) && !error.is_canceled() {
                debug_log_load_image_error("full-resolution image decode failed", &error);
                show_error_message(hwnd, &error.user_message_for(viewer_ui_language(hwnd)));
            }
            if outcome == Some(DecodeApplyOutcome::Applied) {
                start_navigation_preloads_if_possible(hwnd);
            }
        }
    }
}

fn handle_animation_frame_decode_message(
    hwnd: HWND,
    generation: DecodeGeneration,
    path: PathBuf,
    file_version: Option<ImageFileVersion>,
    frame_index: usize,
    result: Result<AnimationFramePixels, LoadImageError>,
) {
    match result {
        Ok(frame) => {
            let outcome = with_app_mut(hwnd, |app| {
                app.apply_animation_frame_pixels(
                    generation,
                    frame_index,
                    path.as_path(),
                    file_version,
                    frame,
                )
            });
            if outcome == Some(DecodeApplyOutcome::Applied) {
                invalidate_window_after_image_content_change(hwnd);
            }
            update_animation_timer(hwnd);
        }
        Err(error) => {
            let outcome = with_app_mut(hwnd, |app| {
                app.finish_failed_animation_frame_decode(
                    generation,
                    frame_index,
                    path.as_path(),
                    file_version,
                )
            });
            if outcome == Some(DecodeApplyOutcome::Applied) && !error.is_canceled() {
                debug_log_load_image_error("animation frame decode failed", &error);
                show_error_message(hwnd, &error.user_message_for(viewer_ui_language(hwnd)));
            }
            update_animation_timer(hwnd);
        }
    }
}

fn copy_current_image_to_clipboard(hwnd: HWND) {
    let payload = with_app_mut(hwnd, clipboard_payloads_for_current_image);

    match payload {
        Some(Ok(Some(payload))) => {
            if let Err(error) = set_clipboard_image_payloads(hwnd, payload) {
                show_error_message(hwnd, &error.user_message_for(viewer_ui_language(hwnd)));
            }
        }
        Some(Ok(None)) => {}
        Some(Err(error)) => {
            show_error_message(hwnd, &error.user_message_for(viewer_ui_language(hwnd)))
        }
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not copy the image because the window state was not found.",
                "창 상태를 찾을 수 없어 이미지를 복사할 수 없습니다.",
            ),
        ),
    }
}

fn clipboard_payloads_for_current_image(
    app: &mut ViewerApp,
) -> Result<Option<ClipboardImagePayloads>, ClipboardCopyError> {
    let has_image = app.image_state().has_image();
    let Some(source) = app.display_pixel_source() else {
        return if has_image {
            Err(ClipboardCopyError::BuildDib(
                ClipboardDibError::DisplayImageUnavailable,
            ))
        } else {
            Ok(None)
        };
    };

    ClipboardImagePayloads::from_oriented_pixels(source.pixels(), source.orientation()).map(Some)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ClipboardDibError {
    DisplayImageUnavailable,
    InvalidPixelBuffer,
    ImageTooLarge,
    AllocationFailed,
}

#[derive(Debug)]
struct ClipboardImagePayloads {
    dib_memory: ClipboardMemory,
}

impl ClipboardImagePayloads {
    #[cfg(test)]
    fn from_pixels(pixels: &PixelImage) -> Result<Self, ClipboardCopyError> {
        Self::from_oriented_pixels(pixels, ImageOrientation::NORMAL)
    }

    fn from_oriented_pixels(
        pixels: &PixelImage,
        orientation: ImageOrientation,
    ) -> Result<Self, ClipboardCopyError> {
        let layout = clipboard_dib_layout_for_oriented_pixels(pixels, orientation)
            .map_err(ClipboardCopyError::BuildDib)?;
        let dib_header =
            clipboard_bitmapinfoheader(layout).map_err(ClipboardCopyError::BuildDib)?;

        let mut dib_memory = ClipboardMemory::allocate(clipboard_payload_byte_len(
            &dib_header,
            layout.pixel_bytes,
        )?)?;
        write_oriented_pixels_to_clipboard_memory(
            pixels,
            orientation,
            layout,
            &dib_header,
            &mut dib_memory,
        )?;

        Ok(Self { dib_memory })
    }

    #[cfg(test)]
    fn from_rgba8(rgba8: &Rgba8Image) -> Result<Self, ClipboardCopyError> {
        Self::from_pixels(&PixelImage::from(rgba8.clone()))
    }

    #[cfg(test)]
    fn dib_payload(&self) -> Result<Vec<u8>, ClipboardCopyError> {
        self.dib_memory.bytes_for_test()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct ClipboardDibLayout {
    width: i32,
    top_down_height: i32,
    pixel_bytes: usize,
    size_image: u32,
}

#[derive(Debug)]
enum ClipboardCopyError {
    BuildDib(ClipboardDibError),
    OpenClipboard { code: u32 },
    EmptyClipboard { code: u32 },
    AllocateMemory { code: u32 },
    LockMemory { code: u32 },
    UnlockMemory { code: u32 },
    SetClipboardData { code: u32 },
    CloseClipboard { code: u32 },
}

impl ClipboardCopyError {
    fn user_message(&self) -> String {
        match self {
            Self::BuildDib(ClipboardDibError::DisplayImageUnavailable) => {
                "표시 중인 이미지 데이터를 클립보드용으로 준비하지 못했습니다.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::InvalidPixelBuffer) => {
                "이미지 픽셀 데이터가 올바르지 않아 클립보드에 복사하지 못했습니다.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::ImageTooLarge) => {
                "이미지가 너무 커서 Windows 클립보드 형식으로 만들 수 없습니다.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::AllocationFailed) => {
                "클립보드 이미지 데이터를 만들 메모리를 확보하지 못했습니다.".to_owned()
            }
            Self::OpenClipboard { .. } => {
                "Windows 클립보드를 열 수 없습니다. 다른 앱이 클립보드를 사용 중일 수 있습니다."
                    .to_owned()
            }
            Self::EmptyClipboard { .. } => {
                "Windows 클립보드의 기존 내용을 비우지 못했습니다.".to_owned()
            }
            Self::AllocateMemory { .. } | Self::LockMemory { .. } | Self::UnlockMemory { .. } => {
                "Windows 클립보드 메모리를 준비하지 못했습니다.".to_owned()
            }
            Self::SetClipboardData { .. } => {
                "Windows 클립보드에 이미지 데이터를 등록하지 못했습니다.".to_owned()
            }
            Self::CloseClipboard { .. } => {
                "Windows 클립보드 작업을 마무리하지 못했습니다.".to_owned()
            }
        }
    }

    fn user_message_for(&self, language: UiLanguage) -> String {
        if language == UiLanguage::Korean {
            return self.user_message();
        }
        match self {
            Self::BuildDib(ClipboardDibError::DisplayImageUnavailable) => {
                "Could not prepare the displayed image data for the clipboard.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::InvalidPixelBuffer) => {
                "Could not copy the image because the pixel data is invalid.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::ImageTooLarge) => {
                "The image is too large for the Windows clipboard format.".to_owned()
            }
            Self::BuildDib(ClipboardDibError::AllocationFailed) => {
                "Could not allocate memory for clipboard image data.".to_owned()
            }
            Self::OpenClipboard { .. } => {
                "Could not open the Windows clipboard. Another app may be using it.".to_owned()
            }
            Self::EmptyClipboard { .. } => {
                "Could not clear the current Windows clipboard contents.".to_owned()
            }
            Self::AllocateMemory { .. } | Self::LockMemory { .. } | Self::UnlockMemory { .. } => {
                "Could not prepare Windows clipboard memory.".to_owned()
            }
            Self::SetClipboardData { .. } => {
                "Could not register image data with the Windows clipboard.".to_owned()
            }
            Self::CloseClipboard { .. } => {
                "Could not finish the Windows clipboard operation.".to_owned()
            }
        }
    }
}

impl fmt::Display for ClipboardCopyError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BuildDib(error) => {
                write!(
                    formatter,
                    "failed to build clipboard DIB payload: {error:?}"
                )
            }
            Self::OpenClipboard { code } => write!(formatter, "OpenClipboard failed: {code}"),
            Self::EmptyClipboard { code } => write!(formatter, "EmptyClipboard failed: {code}"),
            Self::AllocateMemory { code } => write!(formatter, "GlobalAlloc failed: {code}"),
            Self::LockMemory { code } => write!(formatter, "GlobalLock failed: {code}"),
            Self::UnlockMemory { code } => write!(formatter, "GlobalUnlock failed: {code}"),
            Self::SetClipboardData { code } => write!(formatter, "SetClipboardData failed: {code}"),
            Self::CloseClipboard { code } => write!(formatter, "CloseClipboard failed: {code}"),
        }
    }
}

impl Error for ClipboardCopyError {}

struct OpenClipboardGuard {
    closed: bool,
}

impl OpenClipboardGuard {
    fn open(hwnd: HWND) -> Result<Self, ClipboardCopyError> {
        // SAFETY: hwnd is the viewer window that will own the clipboard open operation.
        if unsafe { OpenClipboard(hwnd) } == 0 {
            Err(ClipboardCopyError::OpenClipboard { code: last_error() })
        } else {
            Ok(Self { closed: false })
        }
    }

    fn empty(&self) -> Result<(), ClipboardCopyError> {
        // SAFETY: The guard proves this thread currently has the clipboard open.
        if unsafe { EmptyClipboard() } == 0 {
            Err(ClipboardCopyError::EmptyClipboard { code: last_error() })
        } else {
            Ok(())
        }
    }

    fn close(mut self) -> Result<(), ClipboardCopyError> {
        // SAFETY: The guard closes the clipboard opened by OpenClipboardGuard::open.
        if unsafe { CloseClipboard() } == 0 {
            Err(ClipboardCopyError::CloseClipboard { code: last_error() })
        } else {
            self.closed = true;
            Ok(())
        }
    }
}

impl Drop for OpenClipboardGuard {
    fn drop(&mut self) {
        if !self.closed {
            // SAFETY: Best-effort cleanup for a clipboard opened by this guard.
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }
}

#[derive(Debug)]
struct ClipboardMemory {
    handle: HGLOBAL,
    byte_len: usize,
    owned: bool,
}

impl ClipboardMemory {
    fn allocate(byte_len: usize) -> Result<Self, ClipboardCopyError> {
        // SAFETY: GlobalAlloc returns a movable global memory handle or null on failure.
        // CF_DIB clipboard data must be supplied through a movable memory handle.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, byte_len) };
        if handle.is_null() {
            return Err(ClipboardCopyError::AllocateMemory { code: last_error() });
        }

        Ok(Self {
            handle,
            byte_len,
            owned: true,
        })
    }

    fn lock_for_write(&mut self) -> Result<ClipboardMemoryLock<'_>, ClipboardCopyError> {
        // SAFETY: handle was just allocated and is still owned by memory.
        let locked = unsafe { GlobalLock(self.handle) };
        if locked.is_null() {
            return Err(ClipboardCopyError::LockMemory { code: last_error() });
        }

        Ok(ClipboardMemoryLock {
            memory: self,
            ptr: locked.cast::<u8>(),
            unlocked: false,
        })
    }

    fn handle(&self) -> HGLOBAL {
        self.handle
    }

    fn release_to_clipboard(&mut self) {
        self.owned = false;
    }

    #[cfg(test)]
    fn bytes_for_test(&self) -> Result<Vec<u8>, ClipboardCopyError> {
        // SAFETY: The handle is owned by this wrapper while tests inspect the initialized bytes.
        let locked = unsafe { GlobalLock(self.handle) };
        if locked.is_null() {
            return Err(ClipboardCopyError::LockMemory { code: last_error() });
        }

        // SAFETY: locked points to the live GlobalAlloc buffer for byte_len bytes until unlock.
        let bytes = unsafe { std::slice::from_raw_parts(locked.cast::<u8>(), self.byte_len) };
        let copied = bytes.to_vec();

        // SAFETY: The handle was locked once above and must be unlocked after the copy.
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let unlocked = unsafe { GlobalUnlock(self.handle) };
        if unlocked == 0 {
            let code = last_error();
            if code != ERROR_SUCCESS {
                return Err(ClipboardCopyError::UnlockMemory { code });
            }
        }

        Ok(copied)
    }
}

struct ClipboardMemoryLock<'a> {
    memory: &'a mut ClipboardMemory,
    ptr: *mut u8,
    unlocked: bool,
}

impl ClipboardMemoryLock<'_> {
    fn as_mut_slice(&mut self) -> &mut [u8] {
        // SAFETY: The lock guard proves the HGLOBAL is locked. The buffer was allocated
        // for byte_len bytes, and the guard's mutable borrow prevents another writer.
        unsafe { std::slice::from_raw_parts_mut(self.ptr, self.memory.byte_len) }
    }

    fn unlock(mut self) -> Result<(), ClipboardCopyError> {
        // SAFETY: The handle was locked once above and must be unlocked before SetClipboardData.
        unsafe {
            SetLastError(ERROR_SUCCESS);
        }
        let unlocked = unsafe { GlobalUnlock(self.memory.handle) };
        if unlocked == 0 {
            let code = last_error();
            if code != ERROR_SUCCESS {
                return Err(ClipboardCopyError::UnlockMemory { code });
            }
        }

        self.unlocked = true;
        Ok(())
    }
}

impl Drop for ClipboardMemoryLock<'_> {
    fn drop(&mut self) {
        if !self.unlocked {
            // SAFETY: Best-effort cleanup for a lock acquired by ClipboardMemory::lock_for_write.
            unsafe {
                let _ = GlobalUnlock(self.memory.handle);
            }
        }
    }
}

impl Drop for ClipboardMemory {
    fn drop(&mut self) {
        if self.owned && !self.handle.is_null() {
            // SAFETY: The handle is still owned by this wrapper because SetClipboardData did
            // not accept it or has not been called. GlobalFree releases that allocation.
            unsafe {
                let _ = GlobalFree(self.handle);
            }
        }
    }
}

fn set_clipboard_image_payloads(
    hwnd: HWND,
    mut payloads: ClipboardImagePayloads,
) -> Result<(), ClipboardCopyError> {
    let mut clipboard = OpenClipboardGuard::open(hwnd)?;
    replace_clipboard_image_payloads(&mut clipboard, &mut payloads)?;

    clipboard.close()
}

trait ClipboardImageTarget {
    fn empty(&mut self) -> Result<(), ClipboardCopyError>;
    fn set_image_payloads(
        &mut self,
        payloads: &mut ClipboardImagePayloads,
    ) -> Result<(), ClipboardCopyError>;
}

impl ClipboardImageTarget for OpenClipboardGuard {
    fn empty(&mut self) -> Result<(), ClipboardCopyError> {
        OpenClipboardGuard::empty(self)
    }

    fn set_image_payloads(
        &mut self,
        payloads: &mut ClipboardImagePayloads,
    ) -> Result<(), ClipboardCopyError> {
        set_clipboard_data(CF_DIB_FORMAT, &mut payloads.dib_memory)
    }
}

fn replace_clipboard_image_payloads<T>(
    clipboard: &mut T,
    payloads: &mut ClipboardImagePayloads,
) -> Result<(), ClipboardCopyError>
where
    T: ClipboardImageTarget,
{
    clipboard.empty()?;
    clipboard.set_image_payloads(payloads)
}

fn set_clipboard_data(format: u32, memory: &mut ClipboardMemory) -> Result<(), ClipboardCopyError> {
    // SAFETY: The clipboard is open and memory.handle() is an unlocked movable HGLOBAL
    // containing a DIB payload. A non-null SetClipboardData return transfers ownership
    // of the HGLOBAL to the system clipboard; after that this process must not free it.
    if unsafe { SetClipboardData(format, memory.handle()) }.is_null() {
        return Err(ClipboardCopyError::SetClipboardData { code: last_error() });
    }
    memory.release_to_clipboard();
    Ok(())
}

fn clipboard_dib_layout(width: u32, height: u32) -> Option<ClipboardDibLayout> {
    if width == 0 || height == 0 {
        return None;
    }

    let width = i32::try_from(width).ok()?;
    let height = i32::try_from(height).ok()?;
    let pixel_bytes = expected_dib_len(width as u32, height as u32)?;
    let size_image = u32::try_from(pixel_bytes).ok()?;
    BITMAPINFOHEADER_SIZE.checked_add(pixel_bytes)?;

    Some(ClipboardDibLayout {
        width,
        top_down_height: height.checked_neg()?,
        pixel_bytes,
        size_image,
    })
}

#[cfg(test)]
fn clipboard_dib_layout_for_rgba8(
    rgba8: &Rgba8Image,
) -> Result<ClipboardDibLayout, ClipboardDibError> {
    clipboard_dib_layout_for_pixels(&PixelImage::from(rgba8.clone()))
}

#[cfg(test)]
fn clipboard_dib_layout_for_pixels(
    pixels: &PixelImage,
) -> Result<ClipboardDibLayout, ClipboardDibError> {
    clipboard_dib_layout_for_oriented_pixels(pixels, ImageOrientation::NORMAL)
}

fn clipboard_dib_layout_for_oriented_pixels(
    pixels: &PixelImage,
    orientation: ImageOrientation,
) -> Result<ClipboardDibLayout, ClipboardDibError> {
    let output_size = pixels.size().with_orientation(orientation);
    let layout = clipboard_dib_layout(output_size.width(), output_size.height())
        .ok_or(ClipboardDibError::ImageTooLarge)?;
    if expected_pixel_image_len(pixels) != Some(pixels.pixels().len()) {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    Ok(layout)
}

fn clipboard_payload_byte_len(
    header: &[u8],
    pixel_bytes: usize,
) -> Result<usize, ClipboardCopyError> {
    header
        .len()
        .checked_add(pixel_bytes)
        .ok_or(ClipboardCopyError::AllocateMemory { code: 0 })
}

fn write_oriented_pixels_to_clipboard_memory(
    pixels: &PixelImage,
    orientation: ImageOrientation,
    layout: ClipboardDibLayout,
    dib_header: &[u8],
    dib_memory: &mut ClipboardMemory,
) -> Result<(), ClipboardCopyError> {
    let output_size = pixels.size().with_orientation(orientation);
    debug_assert_eq!(
        expected_dib_len(output_size.width(), output_size.height()),
        Some(layout.pixel_bytes)
    );

    let mut dib_lock = dib_memory.lock_for_write()?;

    {
        let dib = dib_lock.as_mut_slice();
        dib[..dib_header.len()].copy_from_slice(dib_header);
        write_oriented_pixels_as_opaque_bgra_to_slice(
            pixels,
            orientation,
            &mut dib[dib_header.len()..],
        )
        .map_err(ClipboardCopyError::BuildDib)?;
    }

    dib_lock.unlock()
}

#[cfg(test)]
fn clipboard_dib_payload_from_rgba8(rgba8: &Rgba8Image) -> Result<Vec<u8>, ClipboardDibError> {
    let layout = clipboard_dib_layout_for_rgba8(rgba8)?;

    clipboard_payload_from_header_and_rgba8(&clipboard_bitmapinfoheader(layout)?, rgba8, layout)
}

#[cfg(test)]
fn clipboard_dib_payload_from_pixels(pixels: &PixelImage) -> Result<Vec<u8>, ClipboardDibError> {
    let layout = clipboard_dib_layout_for_pixels(pixels)?;

    clipboard_payload_from_header_and_pixels(&clipboard_bitmapinfoheader(layout)?, pixels, layout)
}

#[cfg(test)]
fn clipboard_dib_payload_from_oriented_pixels(
    pixels: &PixelImage,
    orientation: ImageOrientation,
) -> Result<Vec<u8>, ClipboardDibError> {
    let layout = clipboard_dib_layout_for_oriented_pixels(pixels, orientation)?;

    clipboard_payload_from_header_and_oriented_pixels(
        &clipboard_bitmapinfoheader(layout)?,
        pixels,
        orientation,
        layout,
    )
}

#[cfg(test)]
fn clipboard_payload_from_header_and_rgba8(
    header: &[u8],
    rgba8: &Rgba8Image,
    layout: ClipboardDibLayout,
) -> Result<Vec<u8>, ClipboardDibError> {
    clipboard_payload_from_header_and_pixels(header, &PixelImage::from(rgba8.clone()), layout)
}

#[cfg(test)]
fn clipboard_payload_from_header_and_pixels(
    header: &[u8],
    pixels: &PixelImage,
    layout: ClipboardDibLayout,
) -> Result<Vec<u8>, ClipboardDibError> {
    clipboard_payload_from_header_and_oriented_pixels(
        header,
        pixels,
        ImageOrientation::NORMAL,
        layout,
    )
}

#[cfg(test)]
fn clipboard_payload_from_header_and_oriented_pixels(
    header: &[u8],
    pixels: &PixelImage,
    orientation: ImageOrientation,
    layout: ClipboardDibLayout,
) -> Result<Vec<u8>, ClipboardDibError> {
    let total_bytes = header
        .len()
        .checked_add(layout.pixel_bytes)
        .ok_or(ClipboardDibError::ImageTooLarge)?;
    let mut dib = Vec::new();
    dib.try_reserve_exact(total_bytes)
        .map_err(|_| ClipboardDibError::AllocationFailed)?;
    dib.resize(total_bytes, 0);
    dib[..header.len()].copy_from_slice(header);
    write_oriented_pixels_as_opaque_bgra_to_slice(pixels, orientation, &mut dib[header.len()..])?;

    Ok(dib)
}

fn clipboard_bitmapinfoheader(layout: ClipboardDibLayout) -> Result<Vec<u8>, ClipboardDibError> {
    let mut header = Vec::new();
    header
        .try_reserve_exact(BITMAPINFOHEADER_SIZE)
        .map_err(|_| ClipboardDibError::AllocationFailed)?;
    append_bitmapinfoheader(&mut header, layout);

    Ok(header)
}

fn append_bitmapinfoheader(dib: &mut Vec<u8>, layout: ClipboardDibLayout) {
    append_u32(dib, BITMAPINFOHEADER_SIZE as u32);
    append_i32(dib, layout.width);
    append_i32(dib, layout.top_down_height);
    append_u16(dib, 1);
    append_u16(dib, 32);
    append_u32(dib, BI_RGB);
    append_u32(dib, layout.size_image);
    append_i32(dib, 0);
    append_i32(dib, 0);
    append_u32(dib, 0);
    append_u32(dib, 0);
}

fn write_rgba8_as_opaque_bgra_to_slice(
    rgba: &[u8],
    destination: &mut [u8],
) -> Result<(), ClipboardDibError> {
    if rgba.len() != destination.len() {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }
    if !rgba.chunks_exact(4).remainder().is_empty() {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    for (pixel, destination_pixel) in rgba.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
        let alpha = u16::from(pixel[3]);
        destination_pixel[0] = blend_channel_over_white(pixel[2], alpha);
        destination_pixel[1] = blend_channel_over_white(pixel[1], alpha);
        destination_pixel[2] = blend_channel_over_white(pixel[0], alpha);
        destination_pixel[3] = 255;
    }

    Ok(())
}

fn write_pixels_as_opaque_bgra_to_slice(
    pixels: &PixelImage,
    destination: &mut [u8],
) -> Result<(), ClipboardDibError> {
    if expected_dib_len(pixels.width(), pixels.height()) != Some(destination.len()) {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    match pixels {
        PixelImage::Rgb8(rgb8) => write_rgb8_as_opaque_bgra_to_slice(rgb8.pixels(), destination),
        PixelImage::Rgba8(rgba8) => {
            write_rgba8_as_opaque_bgra_to_slice(rgba8.pixels(), destination)
        }
        PixelImage::Bgra8(bgra8) => {
            write_bgra8_as_opaque_bgra_to_slice(bgra8.pixels(), destination)
        }
    }
}

fn write_oriented_pixels_as_opaque_bgra_to_slice(
    pixels: &PixelImage,
    orientation: ImageOrientation,
    destination: &mut [u8],
) -> Result<(), ClipboardDibError> {
    if orientation.is_identity() {
        return write_pixels_as_opaque_bgra_to_slice(pixels, destination);
    }

    let source_size = pixels.size();
    let output_size = source_size.with_orientation(orientation);
    if expected_dib_len(output_size.width(), output_size.height()) != Some(destination.len()) {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }
    if expected_pixel_image_len(pixels) != Some(pixels.pixels().len()) {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    match pixels {
        PixelImage::Rgb8(rgb8) => write_oriented_pixels_with_converter(
            rgb8.pixels(),
            source_size,
            orientation,
            3,
            destination,
            write_rgb8_pixel_as_opaque_bgra,
        ),
        PixelImage::Rgba8(rgba8) => write_oriented_pixels_with_converter(
            rgba8.pixels(),
            source_size,
            orientation,
            4,
            destination,
            write_rgba8_pixel_as_opaque_bgra,
        ),
        PixelImage::Bgra8(bgra8) => write_oriented_pixels_with_converter(
            bgra8.pixels(),
            source_size,
            orientation,
            4,
            destination,
            write_bgra8_pixel_as_opaque_bgra,
        ),
    }
}

fn write_oriented_pixels_with_converter(
    source_pixels: &[u8],
    source_size: ImageSize,
    orientation: ImageOrientation,
    bytes_per_pixel: usize,
    destination: &mut [u8],
    write_pixel: fn(&[u8], &mut [u8]),
) -> Result<(), ClipboardDibError> {
    let output_size = source_size.with_orientation(orientation);
    let source_width =
        usize::try_from(source_size.width()).map_err(|_| ClipboardDibError::InvalidPixelBuffer)?;
    let source_height =
        usize::try_from(source_size.height()).map_err(|_| ClipboardDibError::InvalidPixelBuffer)?;
    let output_width =
        usize::try_from(output_size.width()).map_err(|_| ClipboardDibError::InvalidPixelBuffer)?;
    let output_height =
        usize::try_from(output_size.height()).map_err(|_| ClipboardDibError::InvalidPixelBuffer)?;

    for output_y in 0..output_height {
        for output_x in 0..output_width {
            let (source_x, source_y) = source_pixel_position_for_oriented_output(
                output_x,
                output_y,
                source_width,
                source_height,
                orientation,
            );
            let source_index = pixel_byte_index(source_width, source_x, source_y, bytes_per_pixel);
            let destination_index =
                pixel_byte_index(output_width, output_x, output_y, DIB_BYTES_PER_PIXEL);
            write_pixel(
                &source_pixels[source_index..source_index + bytes_per_pixel],
                &mut destination[destination_index..destination_index + DIB_BYTES_PER_PIXEL],
            );
        }
    }

    Ok(())
}

fn source_pixel_position_for_oriented_output(
    output_x: usize,
    output_y: usize,
    source_width: usize,
    source_height: usize,
    orientation: ImageOrientation,
) -> (usize, usize) {
    match orientation {
        ImageOrientation::Normal => (output_x, output_y),
        ImageOrientation::FlipHorizontal => (source_width - 1 - output_x, output_y),
        ImageOrientation::Rotate180 => (source_width - 1 - output_x, source_height - 1 - output_y),
        ImageOrientation::FlipVertical => (output_x, source_height - 1 - output_y),
        ImageOrientation::Rotate90FlipHorizontal => (output_y, output_x),
        ImageOrientation::Rotate90 => (output_y, source_height - 1 - output_x),
        ImageOrientation::Rotate270FlipHorizontal => {
            (source_width - 1 - output_y, source_height - 1 - output_x)
        }
        ImageOrientation::Rotate270 => (source_width - 1 - output_y, output_x),
    }
}

fn pixel_byte_index(width: usize, x: usize, y: usize, bytes_per_pixel: usize) -> usize {
    (y * width + x) * bytes_per_pixel
}

fn write_rgb8_pixel_as_opaque_bgra(pixel: &[u8], destination: &mut [u8]) {
    destination[0] = pixel[2];
    destination[1] = pixel[1];
    destination[2] = pixel[0];
    destination[3] = 255;
}

fn write_rgba8_pixel_as_opaque_bgra(pixel: &[u8], destination: &mut [u8]) {
    let alpha = u16::from(pixel[3]);
    destination[0] = blend_channel_over_white(pixel[2], alpha);
    destination[1] = blend_channel_over_white(pixel[1], alpha);
    destination[2] = blend_channel_over_white(pixel[0], alpha);
    destination[3] = 255;
}

fn write_bgra8_pixel_as_opaque_bgra(pixel: &[u8], destination: &mut [u8]) {
    let alpha = u16::from(pixel[3]);
    destination[0] = blend_channel_over_white(pixel[0], alpha);
    destination[1] = blend_channel_over_white(pixel[1], alpha);
    destination[2] = blend_channel_over_white(pixel[2], alpha);
    destination[3] = 255;
}

fn write_rgb8_as_opaque_bgra_to_slice(
    rgb: &[u8],
    destination: &mut [u8],
) -> Result<(), ClipboardDibError> {
    if rgb.len().checked_mul(4) != destination.len().checked_mul(3) {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }
    if !rgb.chunks_exact(3).remainder().is_empty() {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    for (pixel, destination_pixel) in rgb.chunks_exact(3).zip(destination.chunks_exact_mut(4)) {
        destination_pixel[0] = pixel[2];
        destination_pixel[1] = pixel[1];
        destination_pixel[2] = pixel[0];
        destination_pixel[3] = 255;
    }

    Ok(())
}

fn write_bgra8_as_opaque_bgra_to_slice(
    bgra: &[u8],
    destination: &mut [u8],
) -> Result<(), ClipboardDibError> {
    if bgra.len() != destination.len() {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }
    if !bgra.chunks_exact(4).remainder().is_empty() {
        return Err(ClipboardDibError::InvalidPixelBuffer);
    }

    for (pixel, destination_pixel) in bgra.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
        let alpha = u16::from(pixel[3]);
        destination_pixel[0] = blend_channel_over_white(pixel[0], alpha);
        destination_pixel[1] = blend_channel_over_white(pixel[1], alpha);
        destination_pixel[2] = blend_channel_over_white(pixel[2], alpha);
        destination_pixel[3] = 255;
    }

    Ok(())
}

fn append_u16(buffer: &mut Vec<u8>, value: u16) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn append_u32(buffer: &mut Vec<u8>, value: u32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn append_i32(buffer: &mut Vec<u8>, value: i32) {
    buffer.extend_from_slice(&value.to_le_bytes());
}

fn show_viewer_context_menu(hwnd: HWND, lparam: LPARAM) {
    let language = viewer_ui_language(hwnd);
    match context_menu::show(hwnd, lparam, language, || viewer_has_image(hwnd)) {
        context_menu::ContextMenuResult::Selected(command) => handle_command(hwnd, command),
        context_menu::ContextMenuResult::CreateFailed => {
            show_error_message(
                hwnd,
                match language {
                    UiLanguage::English => "Could not create the context menu.",
                    UiLanguage::Korean => "컨텍스트 메뉴를 만들 수 없습니다.",
                },
            );
        }
        context_menu::ContextMenuResult::NoSelection => {}
    }
}

fn viewer_ui_language(hwnd: HWND) -> UiLanguage {
    with_app_mut(hwnd, |app| app.config().ui_language()).unwrap_or_default()
}

fn viewer_has_image(hwnd: HWND) -> bool {
    with_app_mut(hwnd, |app| app.image_state().has_image()).unwrap_or(false)
}

fn handle_key_command(hwnd: HWND, command: Command) {
    if is_keyboard_view_command(command) {
        handle_app_command_with_render_cache_update(
            hwnd,
            command,
            AppCommandRenderCacheUpdate::Deferred,
        );
    } else {
        handle_command(hwnd, command);
    }
}

fn is_keyboard_view_command(command: Command) -> bool {
    matches!(
        command,
        Command::ZoomIn
            | Command::ZoomOut
            | Command::ActualSize
            | Command::FitToWindow
            | Command::RotateClockwise
            | Command::RotateCounterClockwise
    )
}

fn handle_window_command(hwnd: HWND, wparam: WPARAM) -> bool {
    let id = wparam & 0xffff;
    if let Some(command) = context_menu::command_from_id(id) {
        handle_command(hwnd, command);
        true
    } else {
        false
    }
}

fn handle_command(hwnd: HWND, command: Command) {
    match command {
        Command::OpenImage => open_image_from_dialog(hwnd),
        Command::ExportImage => export_image_from_dialog(hwnd),
        Command::CopyImageToClipboard => copy_current_image_to_clipboard(hwnd),
        Command::ToggleFullscreen => toggle_fullscreen(hwnd),
        Command::OpenAbout => open_about_dialog(hwnd),
        Command::OpenSettings => open_settings_dialog(hwnd),
        Command::ExitFullscreenOrQuit => exit_fullscreen_or_quit(hwnd),
        Command::Quit => quit_window(hwnd),
        Command::Navigate(direction) => navigate_image(hwnd, direction),
        Command::Animation(command) => handle_animation_command(hwnd, command),
        Command::ContextualSpace => navigate_image(hwnd, ImageNavigationDirection::Next),
        Command::ZoomIn
        | Command::ZoomOut
        | Command::ActualSize
        | Command::FitToWindow
        | Command::RotateClockwise
        | Command::RotateCounterClockwise => handle_app_command(hwnd, command),
    }
}

#[derive(Clone, Copy)]
enum AppCommandRenderCacheUpdate {
    Immediate,
    Deferred,
}

fn open_about_dialog(hwnd: HWND) {
    let language = with_app_mut(hwnd, |app| app.config_snapshot().ui_language())
        .unwrap_or_else(UiLanguage::default);
    if let Err(error) = show_about_dialog(hwnd, language) {
        debug_output_line(&format!(
            "[j3Pic] about dialog failed; internal={}; source={}",
            error,
            error_source_text(&error)
        ));
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not open the About window.",
                "정보 창을 열 수 없습니다.",
            ),
        );
    }
}

fn open_settings_dialog(hwnd: HWND) {
    let Some(config) = with_app_mut(hwnd, |app| app.config_snapshot()) else {
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not open settings because the window state was not found.",
                "창 상태를 찾을 수 없어 설정을 열 수 없습니다.",
            ),
        );
        return;
    };

    match show_settings_dialog(hwnd, config) {
        Ok(SettingsDialogOutcome::Accepted(config)) => apply_settings_config(hwnd, *config),
        Ok(SettingsDialogOutcome::Cancelled) => {}
        Err(error) => {
            debug_output_line(&format!(
                "[j3Pic] settings dialog failed; internal={}; source={}",
                error,
                error_source_text(&error)
            ));
            show_error_message(
                hwnd,
                viewer_text(
                    hwnd,
                    "Could not open the settings window.",
                    "설정 창을 열 수 없습니다.",
                ),
            );
        }
    }
}

fn apply_settings_config(hwnd: HWND, config: crate::domain::AppConfig) {
    let applied = with_app_mut(hwnd, |app| app.apply_config(config.clone()));
    let Some(changed) = applied else {
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not apply settings because the window state was not found.",
                "창 상태를 찾을 수 없어 설정을 적용할 수 없습니다.",
            ),
        );
        return;
    };

    if let Err(error) = save_app_config(&config) {
        debug_output_line(&format!(
            "[j3Pic] settings save failed; internal={}; source={}",
            error,
            error_source_text(&error)
        ));
        show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not save settings. They were applied to the current window.",
                "설정을 저장하지 못했습니다. 현재 실행 중인 창에는 적용되었습니다.",
            ),
        );
    } else {
        let _ = with_window_state_mut(hwnd, |state| {
            state.save_config_on_destroy = true;
        });
    }

    if changed {
        sync_app_viewport_to_current_client_rect(hwnd);
        prepare_window_render_cache(hwnd);
        invalidate_window(hwnd);
        update_animation_timer(hwnd);
        start_full_resolution_decode_if_needed(hwnd);
    }
}

fn handle_app_command(hwnd: HWND, command: Command) {
    handle_app_command_with_render_cache_update(
        hwnd,
        command,
        AppCommandRenderCacheUpdate::Immediate,
    );
}

fn handle_app_command_with_render_cache_update(
    hwnd: HWND,
    command: Command,
    render_cache_update: AppCommandRenderCacheUpdate,
) {
    let result = with_app_mut(hwnd, |app| {
        app.handle_command(command)
            .map(|outcome| (outcome, app.title().to_owned()))
    });

    match result {
        Some(Ok((AppCommandOutcome::Changed, title))) => {
            set_window_title(hwnd, &title);
            update_window_render_cache_after_app_command(hwnd, render_cache_update);
            invalidate_window(hwnd);
            start_full_resolution_decode_if_needed(hwnd);
        }
        Some(Ok((AppCommandOutcome::Unchanged | AppCommandOutcome::Unhandled, _))) => {}
        Some(Err(error)) => show_viewer_error_message(hwnd, &error),
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not run the command because the window state was not found.",
                "창 상태를 찾을 수 없어 명령을 실행할 수 없습니다.",
            ),
        ),
    }
}

fn update_window_render_cache_after_app_command(
    hwnd: HWND,
    render_cache_update: AppCommandRenderCacheUpdate,
) {
    match render_cache_update {
        AppCommandRenderCacheUpdate::Immediate => {
            cancel_deferred_render_settle(hwnd);
            prepare_window_render_cache(hwnd);
        }
        AppCommandRenderCacheUpdate::Deferred => {
            defer_window_render_cache_rebuild(hwnd);
        }
    }
}

fn defer_window_render_cache_rebuild(hwnd: HWND) {
    if with_app_mut(hwnd, ViewerApp::defer_scaling_cache_rebuilds).is_some() {
        schedule_interactive_render_settle_or_finish(hwnd);
    }
}

fn handle_animation_command(hwnd: HWND, command: AnimationCommand) {
    kill_animation_timer(hwnd);
    debug_log_animation_state(hwnd, &format!("command {command:?} before"));
    let outcome = with_app_mut(hwnd, |app| app.handle_animation_command(command));
    debug_log_animation_state(hwnd, &format!("command {command:?} after"));
    handle_animation_frame_outcome(hwnd, outcome);
}

fn handle_animation_timer(hwnd: HWND) {
    kill_animation_timer(hwnd);
    debug_log_animation_state(hwnd, "WM_TIMER before");
    let outcome = with_app_mut(hwnd, ViewerApp::handle_animation_timer);
    debug_log_animation_state(hwnd, "WM_TIMER after");
    handle_animation_frame_outcome(hwnd, outcome);
}

fn handle_animation_frame_outcome(hwnd: HWND, outcome: Option<AnimationFrameOutcome>) {
    match outcome {
        Some(AnimationFrameOutcome::Updated) => {
            invalidate_window_after_image_content_change(hwnd);
            update_animation_timer(hwnd);
        }
        Some(AnimationFrameOutcome::StateChanged) => update_animation_timer(hwnd),
        Some(AnimationFrameOutcome::NeedsDecode(request)) => {
            debug_output_line(&format!(
                "[j3Pic] animation frame decode requested; generation={}; frame_index={}",
                request.generation().value(),
                request.frame_index()
            ));
            start_animation_frame_decode(hwnd, request);
        }
        Some(AnimationFrameOutcome::Unchanged) => update_animation_timer(hwnd),
        None => show_error_message(
            hwnd,
            viewer_text(
                hwnd,
                "Could not run the animation command because the window state was not found.",
                "창 상태를 찾을 수 없어 애니메이션 명령을 실행할 수 없습니다.",
            ),
        ),
    }
}

fn quit_window(hwnd: HWND) {
    // SAFETY: hwnd is a live top-level window owned by this message procedure.
    unsafe {
        DestroyWindow(hwnd);
    }
}

fn handle_mouse_wheel(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let delta = wheel_delta(wparam);
    if delta == 0 {
        return;
    }

    let modifiers = current_key_modifiers();
    let action = with_app_mut(hwnd, |app| {
        let interaction = app.config().interaction_settings();
        if mouse_event_matches(interaction.zoom_shortcut(), modifiers) {
            return Some(MouseWheelAction::Zoom);
        }
        if mouse_event_matches(interaction.image_navigation_shortcut(), modifiers) {
            return Some(MouseWheelAction::Navigate(wheel_navigation_direction(
                delta,
            )));
        }
        None
    })
    .flatten();

    match action {
        Some(MouseWheelAction::Zoom) => handle_mouse_wheel_zoom(hwnd, wparam, lparam),
        Some(MouseWheelAction::Navigate(direction)) => {
            handle_command(hwnd, Command::Navigate(direction));
        }
        None => {}
    }
}

#[derive(Clone, Copy)]
enum MouseWheelAction {
    Zoom,
    Navigate(ImageNavigationDirection),
}

fn handle_mouse_wheel_zoom(hwnd: HWND, wparam: WPARAM, lparam: LPARAM) {
    let delta = wheel_delta(wparam);
    let Some(anchor) = client_point_from_mouse_lparam(hwnd, lparam) else {
        return;
    };
    let changed = with_app_mut(hwnd, |app| {
        let Some(factor) = wheel_zoom_factor(app.zoom_step_factor(), delta) else {
            return false;
        };
        app.zoom_at(factor, anchor)
    });
    if changed == Some(true) {
        invalidate_window_after_interactive_view_change(hwnd);
        start_full_resolution_decode_if_needed(hwnd);
    }
}

fn handle_left_button_down(hwnd: HWND, lparam: LPARAM) {
    let modifiers = current_key_modifiers();
    let action = with_app_mut(hwnd, |app| {
        let interaction = app.config().interaction_settings();
        if mouse_event_matches(interaction.image_pan_shortcut(), modifiers) {
            return Some(LeftButtonAction::ImagePan);
        }
        if mouse_event_matches(interaction.window_move_shortcut(), modifiers) {
            return Some(LeftButtonAction::WindowMove);
        }
        None
    })
    .flatten();

    match action {
        Some(LeftButtonAction::ImagePan) => handle_image_pan_left_button_down(hwnd, lparam),
        Some(LeftButtonAction::WindowMove) => start_window_move(hwnd),
        None => {}
    }
}

#[derive(Clone, Copy)]
enum LeftButtonAction {
    ImagePan,
    WindowMove,
}

fn handle_image_pan_left_button_down(hwnd: HWND, lparam: LPARAM) {
    let point = client_point_from_client_lparam(lparam);
    let started = with_app_mut(hwnd, |app| app.begin_pan(point));
    if started == Some(true) {
        // SAFETY: hwnd is the live window that started the drag gesture.
        unsafe {
            SetCapture(hwnd);
        }
        if captured_window() != hwnd {
            let _ = with_app_mut(hwnd, ViewerApp::end_pan);
        }
    }
}

fn start_window_move(hwnd: HWND) {
    cancel_active_pan(hwnd);
    let lparam = cursor_screen_lparam().unwrap_or_else(|| {
        debug_output_line(&format!(
            "[j3Pic] GetCursorPos failed before window move; Win32 error {}",
            last_error()
        ));
        0
    });
    // SAFETY: Releasing capture and sending a caption hit-test button message asks Win32 to
    // start the normal top-level window move loop for this live viewer window. The lParam carries
    // the current screen cursor position expected by WM_NCLBUTTONDOWN.
    unsafe {
        ReleaseCapture();
        SendMessageW(hwnd, WM_NCLBUTTONDOWN, HTCAPTION as WPARAM, lparam);
    }
}

fn mouse_event_matches(shortcut: MouseShortcut, modifiers: KeyModifiers) -> bool {
    match shortcut {
        MouseShortcut::MouseWheel | MouseShortcut::LeftButtonDrag => {
            !modifiers.control() && !modifiers.shift() && !modifiers.alt()
        }
        MouseShortcut::CtrlMouseWheel | MouseShortcut::CtrlLeftButtonDrag => {
            modifiers.control() && !modifiers.shift() && !modifiers.alt()
        }
    }
}

fn wheel_navigation_direction(delta: i32) -> ImageNavigationDirection {
    if delta > 0 {
        ImageNavigationDirection::Previous
    } else {
        ImageNavigationDirection::Next
    }
}

fn handle_mouse_move(hwnd: HWND, lparam: LPARAM) {
    let point = client_point_from_client_lparam(lparam);
    let changed = with_app_mut(hwnd, |app| app.update_pan(point));
    if changed == Some(true) {
        invalidate_window_after_interactive_view_change(hwnd);
    }
}

fn handle_left_button_up(hwnd: HWND) {
    cancel_active_pan(hwnd);
}

fn handle_capture_changed(hwnd: HWND) {
    let _ = with_app_mut(hwnd, ViewerApp::end_pan);
}

fn client_point_from_mouse_lparam(hwnd: HWND, lparam: LPARAM) -> Option<ViewportPoint> {
    let mut point = POINT {
        x: signed_low_word(lparam),
        y: signed_high_word(lparam),
    };

    // SAFETY: point is initialized with the screen coordinates carried by WM_MOUSEWHEEL.
    if unsafe { ScreenToClient(hwnd, &mut point) } == 0 {
        None
    } else {
        Some(ViewportPoint::from_client_position(point.x, point.y))
    }
}

fn client_point_from_client_lparam(lparam: LPARAM) -> ViewportPoint {
    ViewportPoint::from_client_position(signed_low_word(lparam), signed_high_word(lparam))
}

fn cursor_screen_lparam() -> Option<LPARAM> {
    let mut point = POINT { x: 0, y: 0 };
    // SAFETY: point is a valid writable POINT for the current cursor position.
    if unsafe { GetCursorPos(&mut point) } == 0 {
        None
    } else {
        Some(screen_point_lparam(point))
    }
}

fn screen_point_lparam(point: POINT) -> LPARAM {
    let x = point.x as u16 as u32;
    let y = point.y as u16 as u32;
    (x | (y << 16)) as LPARAM
}

fn set_window_title(hwnd: HWND, title: &str) {
    let title = wide_null(title);
    // SAFETY: hwnd is live and title is a null-terminated UTF-16 buffer.
    unsafe {
        SetWindowTextW(hwnd, title.as_ptr());
    }
}

fn invalidate_window(hwnd: HWND) {
    // SAFETY: Passing null invalidates the whole client area for a live window. Erase is
    // disabled because WM_PAINT redraws a complete buffered frame.
    unsafe {
        InvalidateRect(hwnd, null(), 0);
    }
}

fn flush_invalidated_window(hwnd: HWND) {
    // SAFETY: hwnd is a live viewer window. UpdateWindow sends WM_PAINT synchronously only
    // when an update region exists, preventing repaint from waiting for the message queue to
    // become idle after decode or continuous input messages.
    unsafe {
        UpdateWindow(hwnd);
    }
}

fn invalidate_and_flush_window(hwnd: HWND) {
    invalidate_window(hwnd);
    flush_invalidated_window(hwnd);
}

fn invalidate_window_after_view_change(hwnd: HWND) {
    if is_in_native_size_move_loop(hwnd) {
        invalidate_window(hwnd);
    } else {
        invalidate_and_flush_window(hwnd);
    }
}

fn is_in_native_size_move_loop(hwnd: HWND) -> bool {
    with_window_state_mut(hwnd, |state| state.size_move_dpi.is_in_size_move_loop()).unwrap_or(false)
}

fn cancel_deferred_render_settle(hwnd: HWND) {
    kill_interactive_render_settle_timer(hwnd);
    let _ = with_app_mut(hwnd, ViewerApp::cancel_deferred_scaling_cache_rebuild);
}

fn invalidate_window_after_image_content_change(hwnd: HWND) {
    kill_interactive_render_settle_timer(hwnd);
    let _ = with_window_state_mut(hwnd, |state| {
        state.paint_cache.invalidate();
        state.app.cancel_deferred_scaling_cache_rebuild();
    });
    invalidate_window_after_view_change(hwnd);
}

fn prepare_window_render_cache(hwnd: HWND) {
    let Some(client_rect) = client_rect(hwnd) else {
        return;
    };

    let _ = with_window_state_mut(hwnd, |state| {
        state.paint_cache.invalidate();
        prepare_paint_cache_for_client_rect(
            &client_rect,
            &mut state.app,
            &mut state.paint_cache,
            &state.ui_metrics,
        );
    });
}

fn prepare_paint_cache_for_client_rect(
    client_rect: &RECT,
    app: &mut ViewerApp,
    paint_cache: &mut PaintDibCache,
    ui_metrics: &WindowUiMetrics,
) {
    let content_rect = image_content_rect_for_app(client_rect, app, ui_metrics);
    let viewport_width = content_rect.right.saturating_sub(content_rect.left);
    let viewport_height = content_rect.bottom.saturating_sub(content_rect.top);
    let viewport = ViewportSize::from_client_size(viewport_width, viewport_height);
    let max_cache_bytes = app.memory_policy().max_cache_entry_bytes();
    let Some(render_image) = app.render_rgba8(viewport) else {
        return;
    };

    prepare_paint_dib_cache(
        paint_cache,
        render_image.cache_key(),
        render_image.pixels(),
        render_image.rect(),
        render_image.scaling_quality(),
        &content_rect,
        max_cache_bytes,
    );
}

fn prepare_paint_dib_cache(
    paint_cache: &mut PaintDibCache,
    cache_key: RenderImageCacheKey,
    pixels: &PixelImage,
    rect: crate::domain::ImageDisplayRect,
    scaling_quality: ScalingQuality,
    client_rect: &RECT,
    max_cache_bytes: usize,
) {
    let Some(expected_len) = expected_pixel_image_len(pixels) else {
        return;
    };
    if expected_len != pixels.pixels().len() || rect.width() <= 0 || rect.height() <= 0 {
        return;
    }
    let Some(placement) = paint_dib_placement(rect, pixels.width(), pixels.height(), client_rect)
    else {
        return;
    };
    let cache_bytes = paint_dib_cache_budget_for_placement(max_cache_bytes, placement);

    let _ = paint_cache.pixels_for_pixel_rect(
        cache_key,
        pixels,
        placement.source_rect,
        scaling_quality,
        cache_bytes,
    );
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Win32PaintPrepareProfile {
    dib_bytes: usize,
    source_x: u32,
    source_y: u32,
    source_width: u32,
    source_height: u32,
    cached: bool,
}

impl Win32PaintPrepareProfile {
    fn new(source_rect: PaintDibSourceRect, dib_bytes: usize, cached: bool) -> Self {
        Self {
            dib_bytes,
            source_x: source_rect.x,
            source_y: source_rect.y,
            source_width: source_rect.width,
            source_height: source_rect.height,
            cached,
        }
    }

    pub fn dib_bytes(self) -> usize {
        self.dib_bytes
    }

    pub fn source_x(self) -> u32 {
        self.source_x
    }

    pub fn source_y(self) -> u32 {
        self.source_y
    }

    pub fn source_width(self) -> u32 {
        self.source_width
    }

    pub fn source_height(self) -> u32 {
        self.source_height
    }

    pub fn cached(self) -> bool {
        self.cached
    }
}

/// Profiles the Win32 DIB preparation that happens before `StretchDIBits`.
///
/// This is a development-only entry point used by `profile_open`; normal viewer
/// painting continues through `paint_window_content`.
pub fn profile_win32_paint_prepare(
    render_image: &RenderImage<'_>,
    viewport: ViewportSize,
    max_cache_bytes: usize,
) -> Option<Win32PaintPrepareProfile> {
    let client_rect = RECT {
        left: 0,
        top: 0,
        right: i32::try_from(viewport.width()).ok()?,
        bottom: i32::try_from(viewport.height()).ok()?,
    };
    let pixels = render_image.pixels();
    let expected_len = expected_pixel_image_len(pixels)?;
    if expected_len != pixels.pixels().len()
        || render_image.rect().width() <= 0
        || render_image.rect().height() <= 0
    {
        return None;
    }

    let placement = paint_dib_placement(
        render_image.rect(),
        pixels.width(),
        pixels.height(),
        &client_rect,
    )?;
    let full_source_rect = PaintDibSourceRect::full(pixels.width(), pixels.height())?;
    let cache_bytes = paint_dib_cache_budget_for_placement(max_cache_bytes, placement);
    let mut paint_cache = PaintDibCache::new();

    let cached_pixels = if placement.source_rect == full_source_rect {
        paint_cache.pixels_for_pixels(
            render_image.cache_key(),
            pixels,
            render_image.scaling_quality(),
            cache_bytes,
        )
    } else {
        paint_cache.pixels_for_pixel_rect(
            render_image.cache_key(),
            pixels,
            placement.source_rect,
            render_image.scaling_quality(),
            cache_bytes,
        )
    };
    if let Some(dib_pixels) = cached_pixels {
        return Some(Win32PaintPrepareProfile::new(
            dib_pixels.source_rect,
            dib_pixels.pixels.len(),
            true,
        ));
    }

    let converted = if placement.source_rect == full_source_rect {
        pixel_image_to_bgra32_dib(pixels)
    } else {
        pixel_rect_to_bgra32_dib(pixels, placement.source_rect)
    }?;
    Some(Win32PaintPrepareProfile::new(
        placement.source_rect,
        converted.len(),
        false,
    ))
}

fn invalidate_window_after_interactive_view_change(hwnd: HWND) {
    if with_app_mut(hwnd, ViewerApp::defer_scaling_cache_rebuilds).is_some() {
        schedule_interactive_render_settle_or_finish(hwnd);
    }
    invalidate_window_after_view_change(hwnd);
}

fn schedule_interactive_render_settle_or_finish(hwnd: HWND) {
    if !schedule_interactive_render_settle(hwnd) {
        finish_interactive_render_update(hwnd);
    }
}

fn schedule_interactive_render_settle(hwnd: HWND) -> bool {
    if with_window_state_mut(hwnd, |state| {
        state.size_move_dpi.defer_render_settle_until_exit()
    })
    .unwrap_or(false)
    {
        return true;
    }

    set_interactive_render_settle_timer(hwnd)
}

fn set_interactive_render_settle_timer(hwnd: HWND) -> bool {
    // SAFETY: hwnd is a live viewer window, INTERACTIVE_RENDER_SETTLE_TIMER_ID is owned by this
    // window, and a null callback routes timer events through WM_TIMER.
    unsafe {
        SetTimer(
            hwnd,
            INTERACTIVE_RENDER_SETTLE_TIMER_ID,
            INTERACTIVE_RENDER_SETTLE_TIMER_INTERVAL_MS,
            None,
        ) != 0
    }
}

fn kill_interactive_render_settle_timer(hwnd: HWND) {
    // SAFETY: Killing a missing timer is harmless for viewer state.
    unsafe {
        let _ = KillTimer(hwnd, INTERACTIVE_RENDER_SETTLE_TIMER_ID);
    }
}

fn handle_interactive_render_settle_timer(hwnd: HWND) {
    kill_interactive_render_settle_timer(hwnd);
    if with_window_state_mut(hwnd, |state| {
        state.size_move_dpi.defer_render_settle_until_exit()
    })
    .unwrap_or(false)
    {
        return;
    }
    finish_interactive_render_update(hwnd);
}

fn finish_interactive_render_update(hwnd: HWND) {
    if with_app_mut(hwnd, ViewerApp::resume_scaling_cache_rebuilds) == Some(true) {
        prepare_window_render_cache(hwnd);
        invalidate_window(hwnd);
    }
}

fn show_error_message(hwnd: HWND, message: &str) {
    let text = wide_null(message);
    let caption = wide_null("j3Pic");
    // SAFETY: text and caption are null-terminated UTF-16 buffers valid for the call.
    unsafe {
        MessageBoxW(hwnd, text.as_ptr(), caption.as_ptr(), MB_OK | MB_ICONERROR);
    }
}

fn show_viewer_error_message(hwnd: HWND, error: &ViewerAppError) {
    let message = error.user_message_for(viewer_ui_language(hwnd));
    show_error_message(hwnd, &message);
}

fn viewer_text(hwnd: HWND, english: &'static str, korean: &'static str) -> &'static str {
    match viewer_ui_language(hwnd) {
        UiLanguage::English => english,
        UiLanguage::Korean => korean,
    }
}

fn debug_log_viewer_error(context: &str, error: &ViewerAppError) {
    match error {
        ViewerAppError::LoadImage(error) => debug_log_load_image_error(context, error),
        ViewerAppError::ScanImageFolder(error) => debug_output_line(&format!(
            "[j3Pic] {context}; stage={:?}; category=FolderScan; user_message={:?}; internal={}; source={}",
            ImageLoadFailureStage::FileIo,
            error.brief_user_message(),
            error,
            error_source_text(error)
        )),
        ViewerAppError::ExportImage(error) => debug_output_line(&format!(
            "[j3Pic] {context}; stage=Export; category={:?}; user_message={:?}; internal={}; source={}",
            error.category(),
            error.brief_user_message(),
            error,
            error_source_text(error)
        )),
        ViewerAppError::DecodeWorkerStart { .. } => debug_output_line(&format!(
            "[j3Pic] {context}; stage={:?}; category=DecodeWorkerStart; user_message={:?}; internal={}; source={}",
            ImageLoadFailureStage::Decoder,
            error.brief_user_message(),
            error,
            error_source_text(error)
        )),
        ViewerAppError::ExportWorkerStart { .. } => debug_output_line(&format!(
            "[j3Pic] {context}; stage=Export; category=ExportWorkerStart; user_message={:?}; internal={}; source={}",
            error.brief_user_message(),
            error,
            error_source_text(error)
        )),
        ViewerAppError::NoImageToExport => debug_output_line(&format!(
            "[j3Pic] {context}; stage=Export; category=NoImageToExport; user_message={:?}; internal={}",
            error.brief_user_message(),
            error
        )),
    }
}

fn debug_log_load_image_error(context: &str, error: &LoadImageError) {
    debug_output_line(&format!(
        "[j3Pic] {context}; stage={:?}; category={:?}; user_message={:?}; internal={}; source={}",
        error.failure_stage(),
        error.category(),
        error.brief_user_message(),
        error,
        error_source_text(error)
    ));
}

fn debug_log_animation_state(hwnd: HWND, context: &str) {
    if !animation_debug_logging_enabled() {
        return;
    }

    if let Some(Some(summary)) = with_app_mut(hwnd, |app| app.animation_debug_summary()) {
        debug_output_line(&format!("[j3Pic] animation {context}; {summary}"));
    }
}

fn debug_log_animation_timer(hwnd: HWND, action: &str, interval_ms: Option<u32>) {
    if !animation_debug_logging_enabled() {
        return;
    }

    let summary = with_app_mut(hwnd, |app| app.animation_debug_summary())
        .flatten()
        .unwrap_or_else(|| "state=none".to_owned());
    debug_output_line(&format!(
        "[j3Pic] animation timer {action}; interval_ms={interval_ms:?}; {summary}"
    ));
}

fn animation_debug_logging_enabled() -> bool {
    static ENABLED: std::sync::OnceLock<bool> = std::sync::OnceLock::new();
    *ENABLED.get_or_init(|| std::env::var_os(ANIMATION_DEBUG_LOG_ENV).is_some())
}

fn debug_log_rendering_failure(reason: &str) {
    debug_output_line(&format!(
        "[j3Pic] render image failed; stage={:?}; reason={reason}",
        ImageLoadFailureStage::Win32Rendering
    ));
}

fn error_source_text(error: &(dyn Error + 'static)) -> String {
    error
        .source()
        .map(ToString::to_string)
        .unwrap_or_else(|| "none".to_owned())
}

fn debug_output_line(message: &str) {
    let line = format!("{message}\n");
    let line = wide_null(&line);
    // SAFETY: line is a null-terminated UTF-16 buffer valid for this call.
    unsafe {
        OutputDebugStringW(line.as_ptr());
    }
}

fn paint_window(
    hwnd: HWND,
    app: &mut ViewerApp,
    paint_cache: &mut PaintDibCache,
    paint_buffer: &mut ReusableCompatiblePaintBuffer,
    ui_metrics: &WindowUiMetrics,
) {
    // SAFETY: PAINTSTRUCT is a Win32 POD structure initialized by BeginPaint.
    let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is the window receiving WM_PAINT and paint is valid writable storage.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_null() {
        return;
    }

    // SAFETY: RECT is a plain Win32 data structure filled by GetClientRect.
    let mut client_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is live during WM_PAINT and client_rect is valid writable storage.
    let has_rect = unsafe { GetClientRect(hwnd, &mut client_rect) } != 0;
    if has_rect {
        paint_window_content_buffered(
            hdc,
            &client_rect,
            app,
            paint_cache,
            paint_buffer,
            ui_metrics,
        );
    }

    // SAFETY: Every successful BeginPaint call must be paired with EndPaint for this hwnd.
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn paint_empty_window(hwnd: HWND) {
    // SAFETY: PAINTSTRUCT is a Win32 POD structure initialized by BeginPaint.
    let mut paint: PAINTSTRUCT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is the window receiving WM_PAINT and paint is valid writable storage.
    let hdc = unsafe { BeginPaint(hwnd, &mut paint) };
    if hdc.is_null() {
        return;
    }

    // SAFETY: RECT is a plain Win32 data structure filled by GetClientRect.
    let mut client_rect: RECT = unsafe { std::mem::zeroed() };
    // SAFETY: hwnd is live during WM_PAINT and client_rect is valid writable storage.
    let has_rect = unsafe { GetClientRect(hwnd, &mut client_rect) } != 0;
    if has_rect {
        fill_window_background(hdc, &client_rect);
    }

    // SAFETY: Every successful BeginPaint call must be paired with EndPaint for this hwnd.
    unsafe {
        EndPaint(hwnd, &paint);
    }
}

fn paint_window_content_buffered(
    target_hdc: HDC,
    client_rect: &RECT,
    app: &mut ViewerApp,
    paint_cache: &mut PaintDibCache,
    paint_buffer: &mut ReusableCompatiblePaintBuffer,
    ui_metrics: &WindowUiMetrics,
) {
    let Some((width, height)) = client_rect_size(client_rect) else {
        return;
    };

    let buffered = if let Some(buffer) = paint_buffer.get_or_create(target_hdc, width, height) {
        let buffer_rect = RECT {
            left: 0,
            top: 0,
            right: width,
            bottom: height,
        };
        paint_window_content(buffer.hdc(), &buffer_rect, app, paint_cache, ui_metrics);
        Some(buffer.blit_to(target_hdc, client_rect.left, client_rect.top, width, height))
    } else {
        None
    };

    match buffered {
        Some(true) => {}
        Some(false) => {
            paint_buffer.clear();
            paint_window_content(target_hdc, client_rect, app, paint_cache, ui_metrics);
        }
        None => {
            paint_window_content(target_hdc, client_rect, app, paint_cache, ui_metrics);
        }
    }
}

fn paint_window_content(
    hdc: HDC,
    client_rect: &RECT,
    app: &mut ViewerApp,
    paint_cache: &mut PaintDibCache,
    ui_metrics: &WindowUiMetrics,
) {
    fill_window_background(hdc, client_rect);
    let status_text = app.image_info_text();
    let content_rect = image_content_rect(client_rect, status_text.is_some(), ui_metrics);
    let viewport_width = content_rect.right.saturating_sub(content_rect.left);
    let viewport_height = content_rect.bottom.saturating_sub(content_rect.top);
    let viewport = ViewportSize::from_client_size(viewport_width, viewport_height);
    let max_cache_bytes = app.memory_policy().max_cache_entry_bytes();
    if let Some(render_image) = app.render_rgba8_for_paint(viewport) {
        paint_rgba8_image(
            PaintRgba8Image {
                hdc,
                client_rect: &content_rect,
                pixels: render_image.pixels(),
                rect: render_image.rect(),
                scaling_quality: render_image.scaling_quality(),
                cache_key: render_image.cache_key(),
                max_cache_bytes,
            },
            paint_cache,
        );
    }
    if let Some(status_text) = status_text {
        paint_status_bar(hdc, client_rect, &status_text, ui_metrics);
    }
}

fn client_rect_size(client_rect: &RECT) -> Option<(i32, i32)> {
    let width = client_rect.right.saturating_sub(client_rect.left);
    let height = client_rect.bottom.saturating_sub(client_rect.top);
    if width <= 0 || height <= 0 {
        None
    } else {
        Some((width, height))
    }
}

struct CompatiblePaintBuffer {
    hdc: HDC,
    bitmap: HBITMAP,
    previous_bitmap: HGDIOBJ,
    width: i32,
    height: i32,
    compatibility: PaintBufferCompatibility,
}

struct ReusableCompatiblePaintBuffer {
    buffer: Option<CompatiblePaintBuffer>,
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PaintBufferCompatibility {
    planes: i32,
    bits_per_pixel: i32,
}

const MAX_REUSABLE_PAINT_BUFFER_DIMENSION_MULTIPLIER: i32 = 2;

impl ReusableCompatiblePaintBuffer {
    fn new() -> Self {
        Self { buffer: None }
    }

    fn get_or_create(
        &mut self,
        target_hdc: HDC,
        width: i32,
        height: i32,
    ) -> Option<&CompatiblePaintBuffer> {
        if width <= 0 || height <= 0 {
            return None;
        }

        let compatibility = PaintBufferCompatibility::from_hdc(target_hdc);
        let can_reuse = self
            .buffer
            .as_ref()
            .is_some_and(|buffer| buffer.can_reuse(width, height, compatibility));
        if !can_reuse {
            self.clear();
            self.buffer = CompatiblePaintBuffer::new(target_hdc, width, height, compatibility);
        }

        self.buffer.as_ref()
    }

    fn clear(&mut self) {
        self.buffer = None;
    }
}

impl PaintBufferCompatibility {
    fn from_hdc(hdc: HDC) -> Self {
        // SAFETY: hdc is valid for the current paint pass. GetDeviceCaps only reads device
        // metadata and returns zero for unsupported indices.
        let planes = unsafe { GetDeviceCaps(hdc, PLANES as i32) };
        // SAFETY: Same as above; this records enough format identity to avoid reusing a
        // compatible bitmap after the target device color format changes.
        let bits_per_pixel = unsafe { GetDeviceCaps(hdc, BITSPIXEL as i32) };
        Self {
            planes,
            bits_per_pixel,
        }
    }
}

impl CompatiblePaintBuffer {
    fn new(
        target_hdc: HDC,
        width: i32,
        height: i32,
        compatibility: PaintBufferCompatibility,
    ) -> Option<Self> {
        if width <= 0 || height <= 0 {
            return None;
        }

        // SAFETY: target_hdc is valid for the current paint pass.
        let hdc = unsafe { CreateCompatibleDC(target_hdc) };
        if hdc.is_null() {
            return None;
        }

        // SAFETY: target_hdc is valid and width/height were checked positive.
        let bitmap = unsafe { CreateCompatibleBitmap(target_hdc, width, height) };
        if bitmap.is_null() {
            // SAFETY: hdc was created by CreateCompatibleDC and has no selected bitmap yet.
            unsafe {
                DeleteDC(hdc);
            }
            return None;
        }

        // SAFETY: hdc and bitmap are valid GDI objects. The returned object is restored on drop.
        let previous_bitmap = unsafe { SelectObject(hdc, bitmap as HGDIOBJ) };
        if previous_bitmap.is_null() {
            // SAFETY: Handles were created in this function and are not returned to callers.
            unsafe {
                DeleteObject(bitmap as HGDIOBJ);
                DeleteDC(hdc);
            }
            return None;
        }

        Some(Self {
            hdc,
            bitmap,
            previous_bitmap,
            width,
            height,
            compatibility,
        })
    }

    fn hdc(&self) -> HDC {
        self.hdc
    }

    fn can_reuse(
        &self,
        required_width: i32,
        required_height: i32,
        compatibility: PaintBufferCompatibility,
    ) -> bool {
        let max_reusable_width =
            required_width.saturating_mul(MAX_REUSABLE_PAINT_BUFFER_DIMENSION_MULTIPLIER);
        let max_reusable_height =
            required_height.saturating_mul(MAX_REUSABLE_PAINT_BUFFER_DIMENSION_MULTIPLIER);

        self.width >= required_width
            && self.height >= required_height
            && self.width <= max_reusable_width
            && self.height <= max_reusable_height
            && self.compatibility == compatibility
    }

    fn blit_to(
        &self,
        target_hdc: HDC,
        target_x: i32,
        target_y: i32,
        width: i32,
        height: i32,
    ) -> bool {
        // SAFETY: Both DCs are valid for the current paint pass and the bitmap selected into
        // self.hdc covers the requested width x height pixels.
        unsafe {
            BitBlt(
                target_hdc, target_x, target_y, width, height, self.hdc, 0, 0, SRCCOPY,
            ) != 0
        }
    }
}

impl Drop for CompatiblePaintBuffer {
    fn drop(&mut self) {
        // SAFETY: The bitmap was selected into this memory DC in new(); restore the previous
        // object before deleting the owned bitmap and DC.
        unsafe {
            SelectObject(self.hdc, self.previous_bitmap);
            DeleteObject(self.bitmap as HGDIOBJ);
            DeleteDC(self.hdc);
        }
    }
}

fn fill_window_background(hdc: HDC, client_rect: &RECT) {
    // SAFETY: COLOR_WINDOW is a system brush managed by the OS and must not be destroyed.
    let brush = unsafe { GetSysColorBrush(COLOR_WINDOW) };
    if brush.is_null() {
        return;
    }

    // SAFETY: hdc is from BeginPaint, client_rect is initialized, and brush is system-owned.
    unsafe {
        FillRect(hdc, client_rect, brush);
    }
}

fn paint_status_bar(hdc: HDC, client_rect: &RECT, text: &str, ui_metrics: &WindowUiMetrics) {
    let Some(status_rect) = status_bar_rect(client_rect, ui_metrics) else {
        return;
    };
    // SAFETY: COLOR_3DFACE is a system brush managed by the OS and must not be destroyed.
    let brush = unsafe { GetSysColorBrush(COLOR_3DFACE) };
    if !brush.is_null() {
        // SAFETY: hdc is from BeginPaint, status_rect is initialized, and brush is system-owned.
        unsafe {
            FillRect(hdc, &status_rect, brush);
        }
    }

    let mut text_rect = status_rect;
    text_rect.left = text_rect
        .left
        .saturating_add(ui_metrics.status_text_horizontal_padding);
    text_rect.right = text_rect
        .right
        .saturating_sub(ui_metrics.status_text_horizontal_padding);
    if text_rect.right <= text_rect.left {
        return;
    }

    let text = wide_null(text);
    let text_len = text.len().saturating_sub(1).min(i32::MAX as usize) as i32;
    let draw_flags = DT_LEFT | DT_VCENTER | DT_SINGLELINE | DT_END_ELLIPSIS | DT_NOPREFIX;

    // SAFETY: hdc is valid for this paint pass. DrawTextW reads the UTF-16 buffer for text_len
    // characters and mutates only the local RECT passed by mutable pointer.
    unsafe {
        let previous_font = ui_metrics.status_font.as_ref().and_then(|font| {
            let previous = SelectObject(hdc, font.handle() as HGDIOBJ);
            (!previous.is_null()).then_some(previous)
        });
        let previous_background_mode = SetBkMode(hdc, TRANSPARENT as i32);
        let previous_text_color = SetTextColor(hdc, GetSysColor(COLOR_WINDOWTEXT));
        DrawTextW(hdc, text.as_ptr(), text_len, &mut text_rect, draw_flags);
        SetTextColor(hdc, previous_text_color);
        if previous_background_mode != 0 {
            SetBkMode(hdc, previous_background_mode);
        }
        if let Some(previous_font) = previous_font {
            let _ = SelectObject(hdc, previous_font);
        }
    }
}

fn status_bar_rect(client_rect: &RECT, ui_metrics: &WindowUiMetrics) -> Option<RECT> {
    let width = client_rect.right.saturating_sub(client_rect.left);
    let height = client_rect.bottom.saturating_sub(client_rect.top);
    if width <= 0 || height <= 0 {
        return None;
    }

    let status_height = ui_metrics.status_bar_height.min(height);
    Some(RECT {
        left: client_rect.left,
        top: client_rect.bottom.saturating_sub(status_height),
        right: client_rect.right,
        bottom: client_rect.bottom,
    })
}

fn image_content_rect_for_app(
    client_rect: &RECT,
    app: &ViewerApp,
    ui_metrics: &WindowUiMetrics,
) -> RECT {
    image_content_rect(client_rect, app.image_info_text().is_some(), ui_metrics)
}

fn image_content_rect(
    client_rect: &RECT,
    status_bar_visible: bool,
    ui_metrics: &WindowUiMetrics,
) -> RECT {
    let mut content_rect = *client_rect;
    if status_bar_visible {
        if let Some(status_rect) = status_bar_rect(client_rect, ui_metrics) {
            content_rect.bottom = status_rect.top.max(content_rect.top);
        }
    }
    content_rect
}

fn resize_app_to_image_content_rect(
    client_rect: &RECT,
    app: &mut ViewerApp,
    ui_metrics: &WindowUiMetrics,
) {
    let content_rect = image_content_rect_for_app(client_rect, app, ui_metrics);
    app.handle_resize(
        content_rect.right.saturating_sub(content_rect.left),
        content_rect.bottom.saturating_sub(content_rect.top),
    );
}

fn paint_rgba8_image(request: PaintRgba8Image<'_>, paint_cache: &mut PaintDibCache) {
    let PaintRgba8Image {
        hdc,
        client_rect,
        pixels,
        rect,
        scaling_quality,
        cache_key,
        max_cache_bytes,
    } = request;

    let Some(expected_len) = expected_pixel_image_len(pixels) else {
        debug_log_rendering_failure("pixel byte length overflow");
        return;
    };
    if expected_len != pixels.pixels().len() {
        debug_log_rendering_failure("pixel buffer length mismatch");
        return;
    }

    if rect.width() <= 0 || rect.height() <= 0 {
        return;
    }
    let Some(placement) = paint_dib_placement(rect, pixels.width(), pixels.height(), client_rect)
    else {
        return;
    };
    let Some(full_source_rect) = PaintDibSourceRect::full(pixels.width(), pixels.height()) else {
        return;
    };
    let cache_bytes =
        paint_dib_cache_budget_for_paint_placement(max_cache_bytes, full_source_rect, placement);

    let Some(dib_pixels) = paint_cache.pixels_for_paint_pixel_rect(
        cache_key,
        pixels,
        placement.source_rect,
        scaling_quality,
        cache_bytes,
    ) else {
        debug_log_rendering_failure("BGRA DIB allocation failed");
        return;
    };
    let Some(size_image) = u32::try_from(dib_pixels.pixels.len()).ok() else {
        debug_log_rendering_failure("DIB size does not fit BITMAPINFOHEADER");
        return;
    };
    let mut bitmap_info = BITMAPINFO::default();
    bitmap_info.bmiHeader.biSize = std::mem::size_of_val(&bitmap_info.bmiHeader) as u32;
    let Some(dib_width) = i32::try_from(dib_pixels.source_rect.width).ok() else {
        return;
    };
    let Some(dib_height) = i32::try_from(dib_pixels.source_rect.height).ok() else {
        return;
    };
    let Some(source_x) = placement
        .source_rect
        .x
        .checked_sub(dib_pixels.source_rect.x)
        .and_then(|value| i32::try_from(value).ok())
    else {
        return;
    };
    let Some(source_y) = placement
        .source_rect
        .y
        .checked_sub(dib_pixels.source_rect.y)
        .and_then(|value| i32::try_from(value).ok())
    else {
        return;
    };
    let Some(source_width) = i32::try_from(placement.source_rect.width).ok() else {
        return;
    };
    let Some(source_height) = i32::try_from(placement.source_rect.height).ok() else {
        return;
    };
    let Some(source_right) = source_x.checked_add(source_width) else {
        return;
    };
    let Some(source_bottom) = source_y.checked_add(source_height) else {
        return;
    };
    if source_right > dib_width || source_bottom > dib_height {
        return;
    };
    bitmap_info.bmiHeader.biWidth = dib_width;
    bitmap_info.bmiHeader.biHeight = -dib_height;
    bitmap_info.bmiHeader.biPlanes = 1;
    bitmap_info.bmiHeader.biBitCount = 32;
    bitmap_info.bmiHeader.biCompression = BI_RGB;
    bitmap_info.bmiHeader.biSizeImage = size_image;

    let previous_stretch_mode = set_stretch_mode(hdc, scaling_quality);

    // SAFETY: hdc is valid for this paint pass, bitmap_info describes a top-down 32-bit DIB,
    // and dib_pixels is kept alive for the duration of the call.
    let painted = unsafe {
        StretchDIBits(
            hdc,
            placement.dest_x,
            placement.dest_y,
            placement.dest_width,
            placement.dest_height,
            source_x,
            source_y,
            source_width,
            source_height,
            dib_pixels.pixels.as_ptr().cast::<c_void>(),
            &bitmap_info,
            DIB_RGB_COLORS,
            SRCCOPY,
        )
    };
    if painted == 0 {
        debug_log_rendering_failure("StretchDIBits returned 0");
    }

    restore_stretch_mode(hdc, previous_stretch_mode);
}

fn paint_dib_placement(
    rect: crate::domain::ImageDisplayRect,
    source_width: u32,
    source_height: u32,
    client_rect: &RECT,
) -> Option<PaintDibPlacement> {
    PaintDibSourceRect::full(source_width, source_height)?;
    let dest_left = rect.x();
    let dest_top = rect.y();
    let dest_right = dest_left.checked_add(rect.width())?;
    let dest_bottom = dest_top.checked_add(rect.height())?;
    let visible_left = dest_left.max(client_rect.left);
    let visible_top = dest_top.max(client_rect.top);
    let visible_right = dest_right.min(client_rect.right);
    let visible_bottom = dest_bottom.min(client_rect.bottom);
    if visible_left >= visible_right || visible_top >= visible_bottom {
        return None;
    }

    let one_to_one_width = i32::try_from(source_width).ok()?;
    let one_to_one_height = i32::try_from(source_height).ok()?;
    if rect.width() != one_to_one_width || rect.height() != one_to_one_height {
        let x_axis = scaled_paint_dib_axis_placement(
            dest_left,
            rect.width(),
            visible_left,
            visible_right,
            source_width,
        )?;
        let y_axis = scaled_paint_dib_axis_placement(
            dest_top,
            rect.height(),
            visible_top,
            visible_bottom,
            source_height,
        )?;
        return Some(PaintDibPlacement {
            source_rect: PaintDibSourceRect {
                x: x_axis.source_start,
                y: y_axis.source_start,
                width: x_axis.source_size,
                height: y_axis.source_size,
            },
            dest_x: x_axis.dest_start,
            dest_y: y_axis.dest_start,
            dest_width: x_axis.dest_size,
            dest_height: y_axis.dest_size,
        });
    }

    let source_x = u32::try_from(visible_left.checked_sub(dest_left)?).ok()?;
    let source_y = u32::try_from(visible_top.checked_sub(dest_top)?).ok()?;
    let width = u32::try_from(visible_right.checked_sub(visible_left)?).ok()?;
    let height = u32::try_from(visible_bottom.checked_sub(visible_top)?).ok()?;

    Some(PaintDibPlacement {
        source_rect: PaintDibSourceRect {
            x: source_x,
            y: source_y,
            width,
            height,
        },
        dest_x: visible_left,
        dest_y: visible_top,
        dest_width: visible_right.checked_sub(visible_left)?,
        dest_height: visible_bottom.checked_sub(visible_top)?,
    })
}

fn paint_dib_cache_budget_for_placement(
    max_cache_bytes: usize,
    placement: PaintDibPlacement,
) -> usize {
    let Some(source_bytes) = placement.source_rect.byte_len() else {
        return 0;
    };
    let Some(dest_bytes) = paint_dib_dest_byte_len(placement.dest_width, placement.dest_height)
    else {
        return 0;
    };

    let display_sized_budget =
        dest_bytes.saturating_mul(PAINT_DIB_CACHE_MAX_DISPLAY_OVERSAMPLE_MULTIPLIER);
    max_cache_bytes.min(source_bytes).min(display_sized_budget)
}

fn paint_dib_cache_budget_for_paint_placement(
    max_cache_bytes: usize,
    full_source_rect: PaintDibSourceRect,
    placement: PaintDibPlacement,
) -> usize {
    if placement.source_rect == full_source_rect {
        return placement
            .source_rect
            .byte_len()
            .map_or(0, |source_bytes| max_cache_bytes.min(source_bytes));
    }

    paint_dib_cache_budget_for_placement(max_cache_bytes, placement)
}

fn paint_dib_dest_byte_len(width: i32, height: i32) -> Option<usize> {
    let width = u32::try_from(width).ok()?;
    let height = u32::try_from(height).ok()?;
    expected_dib_len(width, height)
}

fn scaled_paint_dib_axis_placement(
    dest_start: i32,
    dest_size: i32,
    visible_start: i32,
    visible_end: i32,
    source_size: u32,
) -> Option<PaintDibAxisPlacement> {
    if dest_size <= 0 || source_size == 0 || visible_start >= visible_end {
        return None;
    }

    let visible_offset_start = visible_start.checked_sub(dest_start)?;
    let visible_offset_end = visible_end.checked_sub(dest_start)?;
    if visible_offset_start < 0
        || visible_offset_end < visible_offset_start
        || visible_offset_end > dest_size
    {
        return None;
    }

    let dest_size = u32::try_from(dest_size).ok()?;
    let visible_offset_start = u32::try_from(visible_offset_start).ok()?;
    let visible_offset_end = u32::try_from(visible_offset_end).ok()?;

    let source_start = floor_mul_div_u32(visible_offset_start, source_size, dest_size)?;
    let source_end = ceil_mul_div_u32(visible_offset_end, source_size, dest_size)?.min(source_size);
    if source_start >= source_end {
        return None;
    }

    let dest_offset_start = floor_mul_div_i32(source_start, dest_size, source_size)?;
    let dest_offset_end = ceil_mul_div_i32(source_end, dest_size, source_size)?;
    let mapped_dest_start = dest_start.checked_add(dest_offset_start)?;
    let mapped_dest_end = dest_start.checked_add(dest_offset_end)?;
    if mapped_dest_start >= mapped_dest_end {
        return None;
    }

    Some(PaintDibAxisPlacement {
        source_start,
        source_size: source_end.checked_sub(source_start)?,
        dest_start: mapped_dest_start,
        dest_size: mapped_dest_end.checked_sub(mapped_dest_start)?,
    })
}

fn floor_mul_div_u32(value: u32, numerator: u32, denominator: u32) -> Option<u32> {
    if denominator == 0 {
        return None;
    }

    let scaled = u64::from(value)
        .checked_mul(u64::from(numerator))?
        .checked_div(u64::from(denominator))?;
    u32::try_from(scaled).ok()
}

fn ceil_mul_div_u32(value: u32, numerator: u32, denominator: u32) -> Option<u32> {
    if denominator == 0 {
        return None;
    }

    let denominator = u64::from(denominator);
    let scaled = u64::from(value)
        .checked_mul(u64::from(numerator))?
        .checked_add(denominator.checked_sub(1)?)?
        .checked_div(denominator)?;
    u32::try_from(scaled).ok()
}

fn floor_mul_div_i32(value: u32, numerator: u32, denominator: u32) -> Option<i32> {
    i32::try_from(floor_mul_div_u32(value, numerator, denominator)?).ok()
}

fn ceil_mul_div_i32(value: u32, numerator: u32, denominator: u32) -> Option<i32> {
    i32::try_from(ceil_mul_div_u32(value, numerator, denominator)?).ok()
}

fn set_stretch_mode(hdc: HDC, scaling_quality: ScalingQuality) -> i32 {
    let mode = match scaling_quality {
        ScalingQuality::Nearest => COLORONCOLOR,
        ScalingQuality::Balanced | ScalingQuality::HighQuality => HALFTONE,
    };

    // SAFETY: hdc is valid for the current paint pass. SetStretchBltMode changes only the DC.
    let previous_mode = unsafe { SetStretchBltMode(hdc, mode) };
    if mode == HALFTONE {
        // SAFETY: hdc is valid and no previous brush origin is needed by this renderer.
        unsafe {
            SetBrushOrgEx(hdc, 0, 0, null_mut());
        }
    }

    previous_mode
}

fn restore_stretch_mode(hdc: HDC, previous_mode: i32) {
    if previous_mode == 0 {
        return;
    }

    // SAFETY: hdc is valid for the current paint pass and previous_mode came from GDI.
    unsafe {
        SetStretchBltMode(hdc, previous_mode);
    }
}

fn expected_dib_len(width: u32, height: u32) -> Option<usize> {
    let pixels = width.checked_mul(height)?;
    let bytes = pixels.checked_mul(DIB_BYTES_PER_PIXEL as u32)?;
    usize::try_from(bytes).ok()
}

fn expected_pixel_image_len(pixels: &PixelImage) -> Option<usize> {
    pixels.expected_byte_len()
}

fn pixel_image_to_bgra32_dib(pixels: &PixelImage) -> Option<Vec<u8>> {
    let mut dib = Vec::new();
    convert_pixel_image_to_bgra32_dib(pixels, &mut dib)?;
    Some(dib)
}

fn pixel_rect_to_bgra32_dib(
    pixels: &PixelImage,
    source_rect: PaintDibSourceRect,
) -> Option<Vec<u8>> {
    let mut dib = Vec::new();
    convert_pixel_rect_to_bgra32_dib(pixels, source_rect, &mut dib)?;
    Some(dib)
}

#[cfg(test)]
fn rgba8_to_bgra32_dib(rgba8: &Rgba8Image) -> Option<Vec<u8>> {
    let expected_len = expected_dib_len(rgba8.width(), rgba8.height())?;
    if expected_len != rgba8.pixels().len() {
        return None;
    }

    let mut dib = Vec::new();
    convert_rgba8_to_bgra32_dib(rgba8.pixels(), &mut dib)?;
    Some(dib)
}

#[cfg(test)]
fn rgba8_rect_to_bgra32_dib(
    rgba8: &Rgba8Image,
    source_rect: PaintDibSourceRect,
) -> Option<Vec<u8>> {
    let mut dib = Vec::new();
    convert_rgba8_rect_to_bgra32_dib(rgba8, source_rect, &mut dib)?;
    Some(dib)
}

#[cfg(test)]
fn convert_rgba8_to_bgra32_dib(rgba: &[u8], dib: &mut Vec<u8>) -> Option<()> {
    if !rgba.chunks_exact(4).remainder().is_empty() {
        return None;
    }

    prepare_reusable_dib_buffer(dib, rgba.len())?;
    append_rgba8_to_bgra32_dib(rgba, dib)
}

fn convert_pixel_image_to_bgra32_dib(pixels: &PixelImage, dib: &mut Vec<u8>) -> Option<()> {
    let output_len = expected_dib_len(pixels.width(), pixels.height())?;
    if expected_pixel_image_len(pixels) != Some(pixels.pixels().len()) {
        return None;
    }

    prepare_reusable_dib_buffer(dib, output_len)?;
    append_pixel_image_to_bgra32_dib(pixels, dib)
}

#[cfg(test)]
fn convert_rgba8_rect_to_bgra32_dib(
    rgba8: &Rgba8Image,
    source_rect: PaintDibSourceRect,
    dib: &mut Vec<u8>,
) -> Option<()> {
    let expected_len = expected_dib_len(rgba8.width(), rgba8.height())?;
    if expected_len != rgba8.pixels().len() {
        return None;
    }

    let image_width = usize::try_from(rgba8.width()).ok()?;
    let image_height = usize::try_from(rgba8.height()).ok()?;
    let source_x = usize::try_from(source_rect.x).ok()?;
    let source_y = usize::try_from(source_rect.y).ok()?;
    let source_width = usize::try_from(source_rect.width).ok()?;
    let source_height = usize::try_from(source_rect.height).ok()?;
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let source_right = source_x.checked_add(source_width)?;
    let source_bottom = source_y.checked_add(source_height)?;
    if source_right > image_width || source_bottom > image_height {
        return None;
    }

    let row_bytes = source_width.checked_mul(DIB_BYTES_PER_PIXEL)?;
    prepare_reusable_dib_buffer(dib, source_rect.byte_len()?)?;
    for row in 0..source_height {
        let image_y = source_y.checked_add(row)?;
        let row_start_pixels = image_y.checked_mul(image_width)?.checked_add(source_x)?;
        let row_start = row_start_pixels.checked_mul(DIB_BYTES_PER_PIXEL)?;
        let row_end = row_start.checked_add(row_bytes)?;
        append_rgba8_to_bgra32_dib(&rgba8.pixels()[row_start..row_end], dib)?;
    }
    Some(())
}

fn convert_pixel_rect_to_bgra32_dib(
    pixels: &PixelImage,
    source_rect: PaintDibSourceRect,
    dib: &mut Vec<u8>,
) -> Option<()> {
    if expected_pixel_image_len(pixels) != Some(pixels.pixels().len()) {
        return None;
    }

    let image_width = usize::try_from(pixels.width()).ok()?;
    let image_height = usize::try_from(pixels.height()).ok()?;
    let source_x = usize::try_from(source_rect.x).ok()?;
    let source_y = usize::try_from(source_rect.y).ok()?;
    let source_width = usize::try_from(source_rect.width).ok()?;
    let source_height = usize::try_from(source_rect.height).ok()?;
    if source_width == 0 || source_height == 0 {
        return None;
    }
    let source_right = source_x.checked_add(source_width)?;
    let source_bottom = source_y.checked_add(source_height)?;
    if source_right > image_width || source_bottom > image_height {
        return None;
    }

    let source_bytes_per_pixel = pixels.pixel_format().bytes_per_pixel();
    let row_bytes = source_width.checked_mul(source_bytes_per_pixel)?;
    prepare_reusable_dib_buffer(dib, source_rect.byte_len()?)?;
    for row in 0..source_height {
        let image_y = source_y.checked_add(row)?;
        let row_start_pixels = image_y.checked_mul(image_width)?.checked_add(source_x)?;
        let row_start = row_start_pixels.checked_mul(source_bytes_per_pixel)?;
        let row_end = row_start.checked_add(row_bytes)?;
        append_pixel_row_to_bgra32_dib(pixels, row_start, row_end, dib)?;
    }
    Some(())
}

fn prepare_reusable_dib_buffer(dib: &mut Vec<u8>, byte_len: usize) -> Option<()> {
    let max_retained_capacity =
        byte_len.saturating_mul(PAINT_DIB_CACHE_MAX_RETAINED_CAPACITY_MULTIPLIER);
    if dib.capacity() > max_retained_capacity {
        *dib = Vec::new();
    } else {
        dib.clear();
    }
    dib.try_reserve_exact(byte_len).ok()
}

fn append_rgba8_to_bgra32_dib(rgba: &[u8], dib: &mut Vec<u8>) -> Option<()> {
    if !rgba.chunks_exact(4).remainder().is_empty() {
        return None;
    }

    for pixel in rgba.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        dib.push(blend_channel_over_white(pixel[2], alpha));
        dib.push(blend_channel_over_white(pixel[1], alpha));
        dib.push(blend_channel_over_white(pixel[0], alpha));
        dib.push(0);
    }
    Some(())
}

fn append_pixel_image_to_bgra32_dib(pixels: &PixelImage, dib: &mut Vec<u8>) -> Option<()> {
    match pixels {
        PixelImage::Rgb8(image) => append_rgb8_to_bgra32_dib(image.pixels(), dib),
        PixelImage::Rgba8(image) => append_rgba8_to_bgra32_dib(image.pixels(), dib),
        PixelImage::Bgra8(image) => append_bgra8_to_bgra32_dib(image.pixels(), dib),
    }
}

fn append_pixel_row_to_bgra32_dib(
    pixels: &PixelImage,
    row_start: usize,
    row_end: usize,
    dib: &mut Vec<u8>,
) -> Option<()> {
    let row = pixels.pixels().get(row_start..row_end)?;
    match pixels {
        PixelImage::Rgb8(_) => append_rgb8_to_bgra32_dib(row, dib),
        PixelImage::Rgba8(_) => append_rgba8_to_bgra32_dib(row, dib),
        PixelImage::Bgra8(_) => append_bgra8_to_bgra32_dib(row, dib),
    }
}

fn append_rgb8_to_bgra32_dib(rgb: &[u8], dib: &mut Vec<u8>) -> Option<()> {
    if !rgb.chunks_exact(3).remainder().is_empty() {
        return None;
    }

    for pixel in rgb.chunks_exact(3) {
        dib.push(pixel[2]);
        dib.push(pixel[1]);
        dib.push(pixel[0]);
        dib.push(0);
    }
    Some(())
}

fn append_bgra8_to_bgra32_dib(bgra: &[u8], dib: &mut Vec<u8>) -> Option<()> {
    if !bgra.chunks_exact(4).remainder().is_empty() {
        return None;
    }

    for pixel in bgra.chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        dib.push(blend_channel_over_white(pixel[0], alpha));
        dib.push(blend_channel_over_white(pixel[1], alpha));
        dib.push(blend_channel_over_white(pixel[2], alpha));
        dib.push(0);
    }
    Some(())
}

fn blend_channel_over_white(channel: u8, alpha: u16) -> u8 {
    let inverse_alpha = 255u16.saturating_sub(alpha);
    ((u16::from(channel) * alpha + 255 * inverse_alpha) / 255) as u8
}

fn wheel_delta(wparam: WPARAM) -> i32 {
    ((wparam as u32 >> 16) & 0xffff) as i16 as i32
}

fn wheel_zoom_factor(zoom_step_factor: f64, delta: i32) -> Option<f64> {
    if delta == 0 || !zoom_step_factor.is_finite() || zoom_step_factor <= 0.0 {
        return None;
    }

    Some(zoom_step_factor.powf(f64::from(delta) / WHEEL_DELTA_UNITS))
}

fn low_word(value: LPARAM) -> i32 {
    (value as u32 & 0xffff) as u16 as i32
}

fn high_word(value: LPARAM) -> i32 {
    ((value as u32 >> 16) & 0xffff) as u16 as i32
}

fn signed_low_word(value: LPARAM) -> i32 {
    (value as u32 & 0xffff) as i16 as i32
}

fn signed_high_word(value: LPARAM) -> i32 {
    ((value as u32 >> 16) & 0xffff) as i16 as i32
}

fn wide_null(value: &str) -> Vec<u16> {
    OsStr::new(value)
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn path_wide_null(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(iter::once(0))
        .collect()
}

fn last_error() -> u32 {
    // SAFETY: GetLastError reads the calling thread's Win32 error code.
    unsafe { GetLastError() }
}

#[cfg(test)]
mod tests {
    use std::ptr::null_mut;
    use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
    use std::sync::{mpsc, Arc, Condvar, Mutex};
    use std::thread;
    use std::time::{Duration, SystemTime, UNIX_EPOCH};
    use std::{mem, path::PathBuf, ptr};

    use windows_sys::Win32::System::DataExchange::{GetClipboardData, IsClipboardFormatAvailable};
    use windows_sys::Win32::System::Memory::GlobalSize;

    use crate::app::{DecodeApplyOutcome, RenderImageCacheKey, ViewerApp};
    use crate::domain::{
        orient_pixel_image, ExportFormat, ExportOptions, ImageFolder, ImageMetadata,
        ImageOrientation, ImageSize, LoadedImage, PixelImage, Rgb8Image, ScalingQuality,
        SupportedImageFormat, UiLanguage, ViewportSize,
    };
    use crate::infra::export_rgba8_image;

    use super::{
        clipboard_dib_layout, clipboard_dib_payload_from_oriented_pixels,
        clipboard_dib_payload_from_pixels, clipboard_dib_payload_from_rgba8, context_menu,
        dropped_path_has_supported_image_extension, first_supported_drop_path, last_error,
        notify_decode_worker_messages_with, notify_export_worker_messages_with,
        profile_win32_paint_prepare, replace_clipboard_image_payloads,
        request_ui_thread_quit_after_export_shutdown_with, rgba8_rect_to_bgra32_dib,
        rgba8_to_bgra32_dib, scaled_paint_dib_axis_placement, screen_point_lparam,
        set_clipboard_image_payloads, signed_high_word, signed_low_word, wheel_zoom_factor,
        wide_null, ClipboardCopyError, ClipboardDibError, ClipboardImagePayloads,
        ClipboardImageTarget, CloseClipboard, Command, CreateWindowExW, DecodeController,
        DecodeNotificationOutcome, DecodeWorker, DecodeWorkerKind, DestroyWindow, DragFinish,
        ExportNotificationOutcome, FolderScanPermit, GlobalAlloc, GlobalLock, GlobalUnlock,
        OpenClipboard, PaintDibAxisPlacement, PaintDibCache, PaintDibPlacement, PaintDibSourceRect,
        PendingDecodeRequest, Rgba8Image, SizeMoveDpiState, SizeMoveExitOutcome,
        UiThreadQuitOutcome, WindowUiMetrics, BITMAPINFOHEADER_SIZE, BI_RGB, CF_DIB_FORMAT,
        ERROR_SUCCESS, GMEM_MOVEABLE, HDROP, MAX_IN_FLIGHT_DECODE_WORKERS,
        MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS, UI_THREAD_QUIT_POST_ATTEMPTS,
    };
    use super::{POINT, RECT};

    #[test]
    fn size_move_dpi_state_applies_dpi_rect_outside_native_move_loop() {
        let mut state = SizeMoveDpiState::default();

        assert!(state.should_apply_suggested_rect_for_dpi_change());
        assert_eq!(state.exit_size_move(), SizeMoveExitOutcome::default());
    }

    #[test]
    fn size_move_dpi_state_defers_dpi_rect_during_native_move_loop() {
        let mut state = SizeMoveDpiState::default();

        state.enter_size_move();

        assert!(!state.should_apply_suggested_rect_for_dpi_change());
        assert!(!state.should_apply_suggested_rect_for_dpi_change());
        assert!(state.should_defer_view_refresh());
        assert_eq!(
            state.exit_size_move(),
            SizeMoveExitOutcome {
                dpi_changed: true,
                render_settle_pending: false,
            }
        );
        assert!(!state.should_defer_view_refresh());
        assert!(state.should_apply_suggested_rect_for_dpi_change());
    }

    #[test]
    fn size_move_dpi_state_defers_render_settle_during_native_move_loop() {
        let mut state = SizeMoveDpiState::default();

        assert!(!state.defer_render_settle_until_exit());

        state.enter_size_move();

        assert!(state.defer_render_settle_until_exit());
        assert_eq!(
            state.exit_size_move(),
            SizeMoveExitOutcome {
                dpi_changed: false,
                render_settle_pending: true,
            }
        );
        assert!(!state.defer_render_settle_until_exit());
    }

    #[test]
    fn screen_point_lparam_preserves_signed_monitor_coordinates() {
        let lparam = screen_point_lparam(POINT { x: -320, y: 1200 });

        assert_eq!(signed_low_word(lparam), -320);
        assert_eq!(signed_high_word(lparam), 1200);
    }

    #[test]
    fn clipboard_dib_layout_calculates_header_and_pixel_lengths() {
        let layout = clipboard_dib_layout(2, 3).expect("layout");

        assert_eq!(layout.width, 2);
        assert_eq!(layout.top_down_height, -3);
        assert_eq!(layout.pixel_bytes, 24);
        assert_eq!(layout.size_image, 24);
    }

    #[test]
    fn clipboard_dib_layout_rejects_empty_or_oversized_images() {
        assert_eq!(clipboard_dib_layout(0, 1), None);
        assert_eq!(clipboard_dib_layout(1, 0), None);
        assert_eq!(clipboard_dib_layout(i32::MAX as u32 + 1, 1), None);
        assert_eq!(clipboard_dib_layout(65_536, 65_536), None);
    }

    #[test]
    fn clipboard_dib_payload_uses_bitmapinfoheader_and_opaque_bgra_pixels() {
        let image = Rgba8Image::new(2, 1, vec![10, 20, 30, 255, 100, 150, 200, 128]);
        let dib = clipboard_dib_payload_from_rgba8(&image).expect("DIB payload");

        assert_eq!(dib.len(), BITMAPINFOHEADER_SIZE + 8);
        assert_eq!(u32_at(&dib, 0), BITMAPINFOHEADER_SIZE as u32);
        assert_eq!(i32_at(&dib, 4), 2);
        assert_eq!(i32_at(&dib, 8), -1);
        assert_eq!(u16_at(&dib, 12), 1);
        assert_eq!(u16_at(&dib, 14), 32);
        assert_eq!(u32_at(&dib, 16), BI_RGB);
        assert_eq!(u32_at(&dib, 20), 8);
        assert_eq!(
            &dib[BITMAPINFOHEADER_SIZE..],
            &[30, 20, 10, 255, 227, 202, 177, 255]
        );
    }

    #[test]
    fn clipboard_dib_payload_writes_rgb8_without_rgba_expansion_at_domain_boundary() {
        let image = PixelImage::from(Rgb8Image::new(2, 1, vec![10, 20, 30, 100, 150, 200]));
        let dib = clipboard_dib_payload_from_pixels(&image).expect("DIB payload");

        assert_eq!(dib.len(), BITMAPINFOHEADER_SIZE + 8);
        assert_eq!(
            &dib[BITMAPINFOHEADER_SIZE..],
            &[30, 20, 10, 255, 200, 150, 100, 255]
        );
    }

    #[test]
    fn clipboard_dib_payload_from_oriented_pixels_matches_preoriented_pixels() {
        let image = PixelImage::from(Rgba8Image::new(
            2,
            3,
            vec![
                10, 20, 30, 255, 40, 50, 60, 128, 70, 80, 90, 0, 100, 110, 120, 255, 130, 140, 150,
                64, 160, 170, 180, 255,
            ],
        ));
        let orientations = [
            ImageOrientation::Normal,
            ImageOrientation::FlipHorizontal,
            ImageOrientation::Rotate180,
            ImageOrientation::FlipVertical,
            ImageOrientation::Rotate90FlipHorizontal,
            ImageOrientation::Rotate90,
            ImageOrientation::Rotate270FlipHorizontal,
            ImageOrientation::Rotate270,
        ];

        for orientation in orientations {
            let direct =
                clipboard_dib_payload_from_oriented_pixels(&image, orientation).expect("DIB");
            let oriented = orient_pixel_image(&image, orientation).expect("oriented image");
            let expected = clipboard_dib_payload_from_pixels(&oriented).expect("DIB");

            assert_eq!(direct, expected, "{orientation:?}");
        }
    }

    #[test]
    fn clipboard_image_payloads_include_single_dib_payload() {
        let image = Rgba8Image::new(1, 1, vec![80, 90, 100, 255]);
        let payloads = ClipboardImagePayloads::from_rgba8(&image).expect("clipboard payloads");
        let dib = payloads.dib_payload().expect("DIB payload");

        assert_eq!(u32_at(&dib, 0), BITMAPINFOHEADER_SIZE as u32);
        assert_eq!(&dib[BITMAPINFOHEADER_SIZE..], &[100, 90, 80, 255]);
    }

    #[test]
    fn clipboard_image_replace_sets_payload_without_snapshot_capture() {
        let mut target = FakeClipboardImageTarget::default();
        let mut payloads = test_clipboard_payloads();

        let result = replace_clipboard_image_payloads(&mut target, &mut payloads);

        assert!(result.is_ok());
        assert_eq!(
            target.events,
            vec![ClipboardTestEvent::Empty, ClipboardTestEvent::SetImage]
        );
    }

    #[test]
    fn clipboard_image_replace_does_not_restore_when_set_data_fails() {
        let mut target = FakeClipboardImageTarget {
            set_fails: true,
            ..FakeClipboardImageTarget::default()
        };
        let mut payloads = test_clipboard_payloads();

        let result = replace_clipboard_image_payloads(&mut target, &mut payloads);

        assert!(matches!(
            result,
            Err(ClipboardCopyError::SetClipboardData { code: 1_234 })
        ));
        assert_eq!(
            target.events,
            vec![ClipboardTestEvent::Empty, ClipboardTestEvent::SetImage]
        );
    }

    #[test]
    fn clipboard_image_replace_does_not_set_payload_when_empty_fails() {
        let mut target = FakeClipboardImageTarget {
            empty_fails: true,
            ..FakeClipboardImageTarget::default()
        };
        let mut payloads = test_clipboard_payloads();

        let result = replace_clipboard_image_payloads(&mut target, &mut payloads);

        assert!(matches!(
            result,
            Err(ClipboardCopyError::EmptyClipboard { code: 4_321 })
        ));
        assert_eq!(target.events, vec![ClipboardTestEvent::Empty]);
    }

    #[test]
    fn clipboard_dib_payload_rejects_invalid_pixel_length() {
        let image = Rgba8Image::new(2, 1, vec![0, 0, 0, 255]);

        assert_eq!(
            clipboard_dib_payload_from_rgba8(&image),
            Err(ClipboardDibError::InvalidPixelBuffer)
        );
    }

    #[test]
    #[ignore = "uses the global Windows clipboard; run manually in an interactive desktop session"]
    fn clipboard_payloads_can_be_registered_and_read_back_from_win32_clipboard() {
        let window = TestClipboardWindow::new().expect("create clipboard owner window");
        let image = Rgba8Image::new(2, 1, vec![10, 20, 30, 255, 100, 150, 200, 128]);
        let payloads = ClipboardImagePayloads::from_rgba8(&image).expect("clipboard payloads");
        let expected_dib = payloads.dib_payload().expect("DIB payload");

        set_clipboard_image_payloads(window.hwnd, payloads).expect("set clipboard image payloads");

        let _clipboard = TestClipboardGuard::open(window.hwnd).expect("open clipboard");
        assert_ne!(unsafe { IsClipboardFormatAvailable(CF_DIB_FORMAT) }, 0);
        assert_eq!(
            clipboard_format_bytes(CF_DIB_FORMAT, expected_dib.len()).expect("read CF_DIB"),
            expected_dib
        );
    }

    #[test]
    fn paint_dib_conversion_uses_bgra_order_and_flattens_alpha_over_white() {
        let image = Rgba8Image::new(
            3,
            1,
            vec![10, 20, 30, 255, 100, 150, 200, 128, 9, 19, 29, 0],
        );

        let dib = rgba8_to_bgra32_dib(&image).expect("paint DIB pixels");

        assert_eq!(dib, vec![30, 20, 10, 0, 227, 202, 177, 0, 255, 255, 255, 0]);
    }

    #[test]
    fn paint_dib_conversion_rejects_dimension_pixel_length_mismatch() {
        let image = Rgba8Image::new(2, 2, vec![10, 20, 30, 255]);

        assert_eq!(rgba8_to_bgra32_dib(&image), None);
    }

    #[test]
    fn paint_dib_rect_conversion_uses_only_selected_source_pixels() {
        let image = Rgba8Image::new(
            3,
            2,
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255, 130, 140,
                150, 128, 160, 170, 180, 0,
            ],
        );
        let source_rect = PaintDibSourceRect {
            x: 1,
            y: 0,
            width: 2,
            height: 2,
        };

        let dib = rgba8_rect_to_bgra32_dib(&image, source_rect).expect("paint DIB pixels");

        assert_eq!(
            dib,
            vec![60, 50, 40, 0, 90, 80, 70, 0, 202, 197, 192, 0, 255, 255, 255, 0,]
        );
    }

    #[test]
    fn scaled_paint_dib_axis_placement_clips_visible_source_range() {
        assert_eq!(
            scaled_paint_dib_axis_placement(-500, 1000, 0, 200, 100),
            Some(PaintDibAxisPlacement {
                source_start: 50,
                source_size: 20,
                dest_start: 0,
                dest_size: 200,
            })
        );
    }

    #[test]
    fn scaled_paint_dib_axis_placement_rounds_out_to_cover_visible_pixels() {
        assert_eq!(
            scaled_paint_dib_axis_placement(-7, 300, 0, 10, 100),
            Some(PaintDibAxisPlacement {
                source_start: 2,
                source_size: 4,
                dest_start: -1,
                dest_size: 12,
            })
        );
    }

    #[test]
    fn scaled_paint_dib_axis_placement_keeps_full_source_when_fully_visible() {
        assert_eq!(
            scaled_paint_dib_axis_placement(10, 30, 10, 40, 9),
            Some(PaintDibAxisPlacement {
                source_start: 0,
                source_size: 9,
                dest_start: 10,
                dest_size: 30,
            })
        );
    }

    #[test]
    fn paint_dib_cache_budget_keeps_full_source_for_scaled_down_paint() {
        let source_rect = PaintDibSourceRect::full(100, 80).expect("source rect");
        let placement = PaintDibPlacement {
            source_rect,
            dest_x: 0,
            dest_y: 0,
            dest_width: 50,
            dest_height: 40,
        };
        let budget =
            super::paint_dib_cache_budget_for_paint_placement(usize::MAX, source_rect, placement);
        let source_bytes = source_rect.byte_len().expect("source bytes");
        let display_bytes = PaintDibSourceRect::full(50, 40)
            .expect("display rect")
            .byte_len()
            .expect("display bytes");
        assert!(source_bytes > display_bytes * 2);
        assert_eq!(budget, source_bytes);

        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(100, 80));
        let image = Rgba8Image::new(100, 80, vec![255; source_bytes]);
        let mut cache = PaintDibCache::new();

        let first_ptr = {
            let pixels = cache
                .pixels_for_rect(key, &image, source_rect, ScalingQuality::Balanced, budget)
                .expect("cached full source DIB pixels");
            assert_eq!(pixels.source_rect, source_rect);
            assert_eq!(pixels.pixels.len(), source_bytes);
            pixels.pixels.as_ptr()
        };
        let second_ptr = {
            let pixels = cache
                .pixels_for_rect(key, &image, source_rect, ScalingQuality::Balanced, budget)
                .expect("reused full source DIB pixels");
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
    }

    #[test]
    fn paint_dib_cache_budget_uses_final_display_size_for_clipped_scaled_down_source() {
        let full_source_rect = PaintDibSourceRect::full(100, 80).expect("full source rect");
        let source_rect = PaintDibSourceRect {
            x: 10,
            y: 10,
            width: 80,
            height: 60,
        };
        let placement = PaintDibPlacement {
            source_rect,
            dest_x: 0,
            dest_y: 0,
            dest_width: 40,
            dest_height: 30,
        };
        let budget = super::paint_dib_cache_budget_for_paint_placement(
            usize::MAX,
            full_source_rect,
            placement,
        );
        let display_bytes = PaintDibSourceRect::full(40, 30)
            .expect("display rect")
            .byte_len()
            .expect("display bytes");

        assert_eq!(budget, display_bytes * 2);
        assert!(budget < source_rect.byte_len().expect("source bytes"));
    }

    #[test]
    fn paint_dib_cache_reuses_pixels_for_same_render_key() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(1, 1));
        let image = Rgba8Image::new(1, 1, vec![10, 20, 30, 255]);
        let mut cache = PaintDibCache::new();

        let first_ptr = {
            let pixels = cache
                .pixels_for(key, &image, ScalingQuality::Balanced, usize::MAX)
                .expect("cached DIB pixels");
            assert_eq!(pixels.pixels, &[30, 20, 10, 0]);
            pixels.pixels.as_ptr()
        };
        let second_ptr = {
            let pixels = cache
                .pixels_for(key, &image, ScalingQuality::Balanced, usize::MAX)
                .expect("reused DIB pixels");
            assert_eq!(pixels.pixels, &[30, 20, 10, 0]);
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);

        let next_key = RenderImageCacheKey::new(2, ImageOrientation::NORMAL, ImageSize::new(1, 1));
        let next_image = Rgba8Image::new(1, 1, vec![1, 2, 3, 255]);
        let next_pixels = cache
            .pixels_for(next_key, &next_image, ScalingQuality::Balanced, usize::MAX)
            .expect("updated DIB pixels");
        assert_eq!(next_pixels.pixels, &[3, 2, 1, 0]);
    }

    #[test]
    fn paint_dib_cache_invalidate_preserves_capacity_for_rebuild() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(2, 1));
        let image = Rgba8Image::new(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        let mut cache = PaintDibCache::new();

        let first_ptr = {
            let pixels = cache
                .pixels_for(key, &image, ScalingQuality::Balanced, usize::MAX)
                .expect("cached DIB pixels");
            assert_eq!(pixels.pixels, &[30, 20, 10, 0, 60, 50, 40, 0]);
            pixels.pixels.as_ptr()
        };
        let retained_capacity = cache.pixels.capacity();

        cache.invalidate();

        assert!(cache.key.is_none());
        assert!(cache.pixels.is_empty());
        assert_eq!(cache.pixels.capacity(), retained_capacity);

        let next_key = RenderImageCacheKey::new(2, ImageOrientation::NORMAL, ImageSize::new(2, 1));
        let next_image = Rgba8Image::new(2, 1, vec![1, 2, 3, 255, 4, 5, 6, 255]);
        let second_ptr = {
            let pixels = cache
                .pixels_for(next_key, &next_image, ScalingQuality::Balanced, usize::MAX)
                .expect("rebuilt DIB pixels");
            assert_eq!(pixels.pixels, &[3, 2, 1, 0, 6, 5, 4, 0]);
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
        assert_eq!(cache.pixels.capacity(), retained_capacity);
    }

    #[test]
    fn paint_dib_cache_reuses_full_image_for_changed_source_rects() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(3, 2));
        let image = Rgba8Image::new(
            3,
            2,
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255, 130, 140,
                150, 255, 160, 170, 180, 255,
            ],
        );
        let first_rect = PaintDibSourceRect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let second_rect = PaintDibSourceRect {
            x: 1,
            y: 1,
            width: 2,
            height: 1,
        };
        let full_source_rect = PaintDibSourceRect::full(3, 2).expect("full source rect");
        let mut cache = PaintDibCache::new();
        let max_cache_bytes = image.byte_len();

        let first_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    first_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("cached full DIB pixels");
            assert_eq!(pixels.source_rect, full_source_rect);
            assert_eq!(
                pixels.pixels,
                &[
                    30, 20, 10, 0, 60, 50, 40, 0, 90, 80, 70, 0, 120, 110, 100, 0, 150, 140, 130,
                    0, 180, 170, 160, 0,
                ]
            );
            pixels.pixels.as_ptr()
        };
        let second_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    second_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("reused full DIB pixels");
            assert_eq!(pixels.source_rect, full_source_rect);
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
    }

    #[test]
    fn paint_dib_cache_admits_visible_rect_under_full_image_cache_limit() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(3, 2));
        let image = Rgba8Image::new(
            3,
            2,
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255, 130, 140,
                150, 255, 160, 170, 180, 255,
            ],
        );
        let source_rect = PaintDibSourceRect {
            x: 1,
            y: 0,
            width: 2,
            height: 1,
        };
        let mut cache = PaintDibCache::new();
        let max_cache_bytes = source_rect.byte_len().expect("source rect bytes");

        let first_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    source_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("cached visible DIB pixels");
            assert_eq!(pixels.source_rect, source_rect);
            assert_eq!(pixels.pixels, &[60, 50, 40, 0, 90, 80, 70, 0]);
            pixels.pixels.as_ptr()
        };
        assert!(image.byte_len() > max_cache_bytes);
        let second_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    source_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("reused visible DIB pixels");
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
    }

    #[test]
    fn paint_dib_cache_key_tracks_source_rect_quality_and_orientation() {
        let normal_key =
            RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(3, 1));
        let rotated_key =
            RenderImageCacheKey::new(1, ImageOrientation::Rotate90, ImageSize::new(3, 1));
        let image = Rgba8Image::new(
            3,
            1,
            vec![10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255],
        );
        let first_rect = PaintDibSourceRect {
            x: 0,
            y: 0,
            width: 1,
            height: 1,
        };
        let second_rect = PaintDibSourceRect {
            x: 2,
            y: 0,
            width: 1,
            height: 1,
        };
        let mut cache = PaintDibCache::new();
        let max_cache_bytes = first_rect.byte_len().expect("single pixel DIB bytes");

        let pixels = cache
            .pixels_for_rect(
                normal_key,
                &image,
                first_rect,
                ScalingQuality::Balanced,
                max_cache_bytes,
            )
            .expect("first rect DIB pixels");
        assert_eq!(pixels.pixels, &[30, 20, 10, 0]);
        assert_eq!(
            cache.key,
            Some(super::PaintDibCacheKey {
                render_key: normal_key,
                source_rect: first_rect,
                scaling_quality: ScalingQuality::Balanced,
            })
        );

        let pixels = cache
            .pixels_for_rect(
                normal_key,
                &image,
                second_rect,
                ScalingQuality::Balanced,
                max_cache_bytes,
            )
            .expect("second rect DIB pixels");
        assert_eq!(pixels.pixels, &[90, 80, 70, 0]);
        assert_eq!(cache.key.expect("cache key").source_rect, second_rect);

        let _ = cache
            .pixels_for_rect(
                normal_key,
                &image,
                second_rect,
                ScalingQuality::Nearest,
                max_cache_bytes,
            )
            .expect("quality-specific DIB pixels");
        assert_eq!(
            cache.key.expect("cache key").scaling_quality,
            ScalingQuality::Nearest
        );

        let _ = cache
            .pixels_for_rect(
                rotated_key,
                &image,
                second_rect,
                ScalingQuality::Nearest,
                max_cache_bytes,
            )
            .expect("orientation-specific DIB pixels");
        assert_eq!(cache.key.expect("cache key").render_key, rotated_key);
    }

    #[test]
    fn paint_dib_cache_reuses_expanded_rect_for_changed_source_rects_over_full_limit() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(5, 1));
        let image = Rgba8Image::new(
            5,
            1,
            vec![
                10, 20, 30, 255, 40, 50, 60, 255, 70, 80, 90, 255, 100, 110, 120, 255, 130, 140,
                150, 255,
            ],
        );
        let first_rect = PaintDibSourceRect {
            x: 0,
            y: 0,
            width: 2,
            height: 1,
        };
        let second_rect = PaintDibSourceRect {
            x: 2,
            y: 0,
            width: 2,
            height: 1,
        };
        let cache_rect = PaintDibSourceRect {
            x: 0,
            y: 0,
            width: 4,
            height: 1,
        };
        let mut cache = PaintDibCache::new();
        let max_cache_bytes = cache_rect.byte_len().expect("cache rect bytes");

        assert!(image.byte_len() > max_cache_bytes);
        let first_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    first_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("cached expanded DIB pixels");
            assert_eq!(pixels.source_rect, cache_rect);
            assert_eq!(
                pixels.pixels,
                &[30, 20, 10, 0, 60, 50, 40, 0, 90, 80, 70, 0, 120, 110, 100, 0,]
            );
            pixels.pixels.as_ptr()
        };
        let second_ptr = {
            let pixels = cache
                .pixels_for_rect(
                    key,
                    &image,
                    second_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("reused expanded DIB pixels");
            assert_eq!(pixels.source_rect, cache_rect);
            pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
    }

    #[test]
    fn paint_dib_cache_releases_oversized_capacity_for_smaller_entry() {
        let large_key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(4, 1));
        let large_image = Rgba8Image::new(4, 1, vec![10; 16]);
        let small_key = RenderImageCacheKey::new(2, ImageOrientation::NORMAL, ImageSize::new(1, 1));
        let small_image = Rgba8Image::new(1, 1, vec![1, 2, 3, 255]);
        let mut cache = PaintDibCache::new();

        let Some(large_pixels) = cache.pixels_for(
            large_key,
            &large_image,
            ScalingQuality::Balanced,
            usize::MAX,
        ) else {
            panic!("large DIB pixels should be cached");
        };
        assert_eq!(large_pixels.pixels.len(), large_image.byte_len());
        let large_capacity = cache.pixels.capacity();

        let Some(small_pixels) = cache.pixels_for(
            small_key,
            &small_image,
            ScalingQuality::Balanced,
            usize::MAX,
        ) else {
            panic!("small DIB pixels should be cached");
        };
        assert_eq!(small_pixels.pixels, &[3, 2, 1, 0]);

        assert!(cache.pixels.capacity() < large_capacity);
    }

    #[test]
    fn paint_dib_cache_skips_entries_over_cache_limit() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(1, 1));
        let image = Rgba8Image::new(1, 1, vec![10, 20, 30, 255]);
        let mut cache = PaintDibCache::new();
        assert!(cache
            .pixels_for(key, &image, ScalingQuality::Balanced, image.byte_len())
            .is_some());

        let skipped = cache.pixels_for(
            key,
            &image,
            ScalingQuality::Balanced,
            image.byte_len().saturating_sub(1),
        );

        assert!(skipped.is_none());
        assert!(cache.key.is_none());
        assert!(cache.pixels.is_empty());
        assert_eq!(cache.pixels.capacity(), 0);
    }

    #[test]
    fn paint_dib_cache_reuses_scratch_pixels_for_uncacheable_paint_rect() {
        let key = RenderImageCacheKey::new(1, ImageOrientation::NORMAL, ImageSize::new(2, 1));
        let image = Rgba8Image::new(2, 1, vec![10, 20, 30, 255, 40, 50, 60, 255]);
        let pixels = PixelImage::from(image.clone());
        let full_source_rect = PaintDibSourceRect::full(2, 1).expect("full source rect");
        let max_cache_bytes = image.byte_len().saturating_sub(1);
        let mut cache = PaintDibCache::new();

        let first_ptr = {
            let dib_pixels = cache
                .pixels_for_paint_pixel_rect(
                    key,
                    &pixels,
                    full_source_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("uncached scratch DIB pixels");
            assert_eq!(dib_pixels.source_rect, full_source_rect);
            assert_eq!(dib_pixels.pixels, &[30, 20, 10, 0, 60, 50, 40, 0]);
            dib_pixels.pixels.as_ptr()
        };

        assert!(cache.key.is_none());
        assert!(cache.pixels.is_empty());
        assert_eq!(cache.pixels.capacity(), 0);
        assert!(cache.scratch_pixels.capacity() >= image.byte_len());

        let second_ptr = {
            let dib_pixels = cache
                .pixels_for_paint_pixel_rect(
                    key,
                    &pixels,
                    full_source_rect,
                    ScalingQuality::Balanced,
                    max_cache_bytes,
                )
                .expect("reused uncached scratch DIB pixels");
            assert_eq!(dib_pixels.pixels, &[30, 20, 10, 0, 60, 50, 40, 0]);
            dib_pixels.pixels.as_ptr()
        };

        assert_eq!(first_ptr, second_ptr);
        assert!(cache.key.is_none());

        cache.invalidate();
        assert_eq!(cache.scratch_pixels.capacity(), 0);
    }

    #[test]
    fn profile_win32_paint_prepare_reports_dib_shape_without_timing_assertion() {
        let dir = unique_temp_dir("win32-profile-paint-prepare");
        std::fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("small.png");
        write_png_fixture(&path, 3, 2);

        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(100, 100);
        app.handle_resize(100, 100);
        app.load_image(&path).expect("load image");
        let max_cache_bytes = app.memory_policy().max_cache_entry_bytes();
        let render = app.prepare_first_render(viewport).expect("first render");
        let profile = profile_win32_paint_prepare(&render, viewport, max_cache_bytes)
            .expect("paint prepare profile");

        assert_eq!(profile.source_x(), 0);
        assert_eq!(profile.source_y(), 0);
        assert_eq!(profile.source_width(), 3);
        assert_eq!(profile.source_height(), 2);
        assert_eq!(profile.dib_bytes(), 3 * 2 * super::DIB_BYTES_PER_PIXEL);
        assert!(profile.cached());

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn image_content_rect_reserves_status_bar_when_visible() {
        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 200,
        };
        let metrics = test_ui_metrics(28);

        let content_rect = super::image_content_rect(&client_rect, true, &metrics);

        assert_eq!(content_rect.left, 0);
        assert_eq!(content_rect.top, 0);
        assert_eq!(content_rect.right, 320);
        assert_eq!(content_rect.bottom, 172);
    }

    #[test]
    fn image_content_rect_uses_full_client_when_status_bar_hidden() {
        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 200,
        };
        let metrics = test_ui_metrics(28);

        let content_rect = super::image_content_rect(&client_rect, false, &metrics);

        assert_eq!(content_rect.bottom, 200);
    }

    #[test]
    fn app_viewport_resize_reserves_visible_status_bar() {
        let mut app = ViewerApp::new();
        let path = PathBuf::from("status-visible.png");
        let request = app.begin_image_decode(path.clone());
        let image = LoadedImage::new(
            Rgba8Image::new(10, 10, vec![255; 10 * 10 * 4]),
            ImageMetadata::new(path, 0, SupportedImageFormat::Png),
        );
        assert_eq!(
            app.apply_decoded_image(request.generation(), image, ImageFolder::empty()),
            DecodeApplyOutcome::Applied
        );

        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 320,
            bottom: 200,
        };
        let metrics = test_ui_metrics(28);
        super::resize_app_to_image_content_rect(&client_rect, &mut app, &metrics);

        assert_eq!(app.viewport(), ViewportSize::from_client_size(320, 172));
    }

    #[test]
    fn render_settle_prepares_scaled_paint_cache_after_first_render() {
        let dir = unique_temp_dir("win32-render-settle");
        std::fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("large.png");
        write_png_fixture(&path, 1000, 800);

        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.handle_resize(500, 400);
        app.load_image(&path).expect("load image");

        {
            let first = app.prepare_first_render(viewport).expect("first render");
            assert_eq!(first.pixels().size(), ImageSize::new(1000, 800));
        }
        assert!(app.has_deferred_scaling_cache_rebuild());
        assert!(app.resume_scaling_cache_rebuilds());

        let client_rect = RECT {
            left: 0,
            top: 0,
            right: 500,
            bottom: 400,
        };
        let mut cache = PaintDibCache::new();
        let metrics = test_ui_metrics(0);
        super::prepare_paint_cache_for_client_rect(&client_rect, &mut app, &mut cache, &metrics);

        assert!(!app.has_deferred_scaling_cache_rebuild());
        assert_eq!(cache.pixels.len(), 512 * 400 * super::DIB_BYTES_PER_PIXEL);

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn mouse_wheel_zoom_uses_configured_step_factor() {
        assert_eq!(wheel_zoom_factor(2.0, 120), Some(2.0));
        assert_eq!(wheel_zoom_factor(2.0, -120), Some(0.5));
        assert_eq!(wheel_zoom_factor(2.0, 240), Some(4.0));
        assert_eq!(wheel_zoom_factor(2.0, 0), None);
    }

    #[test]
    fn hdrop_query_selects_first_supported_drop_path() {
        let hdrop = test_hdrop(&[r"C:\drop\ignored.txt", r"C:\drop\favicon.ICO"]);
        let _guard = TestDropHandle(hdrop);

        let selected = first_supported_drop_path(hdrop).expect("supported drop path");

        assert_eq!(selected, PathBuf::from(r"C:\drop\favicon.ICO"));
    }

    #[test]
    fn hdrop_query_returns_none_without_supported_drop_path() {
        let hdrop = test_hdrop(&[r"C:\drop\ignored.txt", r"C:\drop\archive.tar.gz"]);
        let _guard = TestDropHandle(hdrop);

        assert_eq!(first_supported_drop_path(hdrop), None);
    }

    #[test]
    fn drop_path_extension_detection_matches_supported_path_cases() {
        for path in [
            r"C:\drop\photo.JPG",
            r"C:\drop\photo.jpeg",
            r"C:\drop.with.dot\icon.PnG",
            r"C:\drop\scan.BMP",
            r"C:\drop\clip.Gif",
            r"C:\drop\poster.WEBP",
            r"C:\drop\favicon.ICO",
            r"C:\drop\scan.tif",
            r"C:\drop\scan.TIFF",
            r"C:\drop\sprite.TGA",
        ] {
            assert!(has_supported_drop_extension(path), "{path}");
        }

        for path in [
            r"C:\drop\notes.txt",
            r"C:\drop\archive.tar.gz",
            r"C:\drop\no-extension",
            r"C:\drop\.jpg",
            r"C:\drop\folder.jpg\not-image.txt",
            r"C:\drop\file.",
        ] {
            assert!(!has_supported_drop_extension(path), "{path}");
        }
    }

    #[test]
    fn context_menu_keeps_settings_command_at_bottom() {
        let Some(context_menu::ContextMenuEntry::Command(item)) =
            context_menu::CONTEXT_MENU_ENTRIES.last()
        else {
            panic!("last context menu entry should be a command");
        };

        assert_eq!(item.id, context_menu::CONTEXT_MENU_ID_OPEN_SETTINGS);
        assert_eq!(item.command, Command::OpenSettings);
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::Korean, item.command),
            "설정..."
        );
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::English, item.command),
            "Settings..."
        );
    }

    #[test]
    fn context_menu_matches_reference_order_and_image_requirements() {
        let expected = [
            Some((Command::OpenImage, false)),
            Some((Command::ExportImage, true)),
            Some((Command::CopyImageToClipboard, true)),
            None,
            Some((Command::ActualSize, true)),
            Some((Command::FitToWindow, true)),
            Some((Command::RotateClockwise, true)),
            Some((Command::RotateCounterClockwise, true)),
            None,
            Some((Command::ToggleFullscreen, false)),
            None,
            Some((Command::OpenAbout, false)),
            Some((Command::OpenSettings, false)),
        ];
        let actual = context_menu::CONTEXT_MENU_ENTRIES
            .iter()
            .map(|entry| match entry {
                context_menu::ContextMenuEntry::Command(item) => {
                    Some((item.command, item.requires_image))
                }
                context_menu::ContextMenuEntry::Separator => None,
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::English, Command::OpenImage),
            "Open..."
        );
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::Korean, Command::OpenImage),
            "열기..."
        );
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::English, Command::OpenAbout),
            "About..."
        );
        assert_eq!(
            crate::ui_text::context_menu_label(UiLanguage::Korean, Command::OpenAbout),
            "정보..."
        );
    }

    #[test]
    fn context_menu_command_ids_are_unique_and_mapped() {
        let mut ids = context_menu::CONTEXT_MENU_ENTRIES
            .iter()
            .filter_map(|entry| match entry {
                context_menu::ContextMenuEntry::Command(item) => Some(item.id),
                context_menu::ContextMenuEntry::Separator => None,
            })
            .collect::<Vec<_>>();
        let total = ids.len();
        ids.sort_unstable();
        ids.dedup();

        assert_eq!(ids.len(), total);
        for id in ids {
            assert!(context_menu::command_from_id(id).is_some());
        }
    }

    #[test]
    fn decode_notification_uses_timer_fallback_when_post_fails() {
        let mut send_count = 0;
        let mut logs = Vec::new();

        let outcome = notify_decode_worker_messages_with(
            || Err(1234),
            || Ok(()),
            || send_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(
            outcome,
            DecodeNotificationOutcome::TimerFallback { post_error: 1234 }
        );
        assert_eq!(send_count, 0);
        assert!(logs
            .iter()
            .any(|message| message.contains("PostMessageW failed")));
    }

    #[test]
    fn decode_notification_uses_send_fallback_when_post_and_timer_fail() {
        let mut send_count = 0;
        let mut logs = Vec::new();

        let outcome = notify_decode_worker_messages_with(
            || Err(1234),
            || Err(5678),
            || send_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(
            outcome,
            DecodeNotificationOutcome::SendFallback {
                post_error: 1234,
                timer_error: 5678
            }
        );
        assert_eq!(send_count, 1);
        assert!(logs
            .iter()
            .any(|message| message.contains("SetTimer failed")));
    }

    #[test]
    fn export_notification_uses_timer_fallback_when_post_fails() {
        let mut send_count = 0;
        let mut logs = Vec::new();

        let outcome = notify_export_worker_messages_with(
            || Err(1234),
            || Ok(()),
            || send_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(
            outcome,
            ExportNotificationOutcome::TimerFallback { post_error: 1234 }
        );
        assert_eq!(send_count, 0);
        assert!(logs
            .iter()
            .any(|message| message.contains("PostMessageW failed")));
    }

    #[test]
    fn export_notification_uses_send_fallback_when_post_and_timer_fail() {
        let mut send_count = 0;
        let mut logs = Vec::new();

        let outcome = notify_export_worker_messages_with(
            || Err(1234),
            || Err(5678),
            || send_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(
            outcome,
            ExportNotificationOutcome::SendFallback {
                post_error: 1234,
                timer_error: 5678
            }
        );
        assert_eq!(send_count, 1);
        assert!(logs
            .iter()
            .any(|message| message.contains("SetTimer failed")));
    }

    #[test]
    fn export_shutdown_quit_retries_thread_message_before_fallback() {
        let mut post_count = 0;
        let mut wait_count = 0;
        let mut fallback_count = 0;
        let mut logs = Vec::new();

        let outcome = request_ui_thread_quit_after_export_shutdown_with(
            || {
                post_count += 1;
                Err(4321)
            },
            || wait_count += 1,
            || fallback_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(
            outcome,
            UiThreadQuitOutcome::ProcessExitFallback { post_error: 4321 }
        );
        assert_eq!(post_count, UI_THREAD_QUIT_POST_ATTEMPTS);
        assert_eq!(wait_count, UI_THREAD_QUIT_POST_ATTEMPTS - 1);
        assert_eq!(fallback_count, 1);
        assert!(logs
            .iter()
            .any(|message| message.contains("PostThreadMessageW failed")));
    }

    #[test]
    fn export_shutdown_quit_stops_retrying_after_thread_message_succeeds() {
        let mut post_count = 0;
        let mut wait_count = 0;
        let mut fallback_count = 0;
        let mut logs = Vec::new();

        let outcome = request_ui_thread_quit_after_export_shutdown_with(
            || {
                post_count += 1;
                if post_count == 1 {
                    Err(1234)
                } else {
                    Ok(())
                }
            },
            || wait_count += 1,
            || fallback_count += 1,
            |message| logs.push(message.to_owned()),
        );

        assert_eq!(outcome, UiThreadQuitOutcome::Posted);
        assert_eq!(post_count, 2);
        assert_eq!(wait_count, 1);
        assert_eq!(fallback_count, 0);
        assert!(logs
            .iter()
            .any(|message| message.contains("PostThreadMessageW failed")));
    }

    fn write_png_fixture(path: &std::path::Path, width: u32, height: u32) {
        let pixel_len = width as usize * height as usize * 4;
        let image = Rgba8Image::new(width, height, vec![255; pixel_len]);
        export_rgba8_image(path, &image, ExportOptions::new(ExportFormat::Png, None))
            .expect("write png fixture");
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("j3pic-{name}-{}-{nanos}", std::process::id()))
    }

    #[test]
    fn full_resolution_decode_can_replace_unjoined_initial_worker_for_same_generation() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let request = app.begin_image_decode(PathBuf::from("missing-full-resolution.png"));
        let generation = request.generation();
        let (worker, release_worker) = test_decode_worker(generation, DecodeWorkerKind::Initial);
        controller.active_worker = Some(worker);

        controller
            .start_full_resolution_decode(null_mut(), request)
            .expect("start full-resolution decode");

        assert!(controller.retired_workers.iter().any(|worker| {
            worker.kind == DecodeWorkerKind::Initial && worker.cancel.load(Ordering::Acquire)
        }));
        assert_eq!(
            controller.active_worker.as_ref().map(|worker| worker.kind),
            Some(DecodeWorkerKind::FullResolution)
        );
        release_worker.release();
        controller.shutdown();
    }

    #[test]
    fn duplicate_running_full_resolution_decode_for_same_generation_is_not_restarted() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let request = app.begin_image_decode(PathBuf::from("missing-full-resolution.png"));
        let generation = request.generation();
        let (worker, release_worker) =
            test_decode_worker(generation, DecodeWorkerKind::FullResolution);
        let cancel = Arc::clone(&worker.cancel);
        controller.active_worker = Some(worker);

        controller
            .start_full_resolution_decode(null_mut(), request)
            .expect("start full-resolution decode");

        assert!(controller.retired_workers.is_empty());
        assert!(!cancel.load(Ordering::Acquire));
        assert_eq!(
            controller.active_worker.as_ref().map(|worker| worker.kind),
            Some(DecodeWorkerKind::FullResolution)
        );
        release_worker.release();
        controller.shutdown();
    }

    #[test]
    fn repeated_initial_decodes_are_queued_when_retired_worker_limit_is_reached() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let mut releases = Vec::new();

        for index in 0..(MAX_IN_FLIGHT_DECODE_WORKERS - 1) {
            let generation = app
                .begin_image_decode(PathBuf::from(format!("blocked-retired-{index}.png")))
                .generation();
            let (worker, release) = test_decode_worker(generation, DecodeWorkerKind::Initial);
            worker.cancel.store(true, Ordering::Release);
            controller.retired_workers.push(worker);
            releases.push(release);
        }

        let active_generation = app
            .begin_image_decode(PathBuf::from("blocked-active.png"))
            .generation();
        let (active_worker, active_release) =
            test_decode_worker(active_generation, DecodeWorkerKind::Initial);
        let active_cancel = Arc::clone(&active_worker.cancel);
        controller.active_worker = Some(active_worker);
        releases.push(active_release);

        let first_request = app.begin_image_decode(PathBuf::from("queued-first.png"));
        controller
            .start_initial_decode(null_mut(), first_request)
            .expect("queue first initial decode");

        assert!(active_cancel.load(Ordering::Acquire));
        assert!(controller.active_worker.is_none());
        assert_eq!(
            controller.retired_workers.len(),
            MAX_IN_FLIGHT_DECODE_WORKERS
        );

        let latest_request = app.begin_image_decode(PathBuf::from("queued-latest.png"));
        let latest_generation = latest_request.generation();
        controller
            .start_initial_decode(null_mut(), latest_request)
            .expect("replace queued initial decode");

        assert!(controller.active_worker.is_none());
        assert_eq!(
            controller.retired_workers.len(),
            MAX_IN_FLIGHT_DECODE_WORKERS
        );
        match controller.pending_decode.as_ref() {
            Some(PendingDecodeRequest::Initial { request, .. }) => {
                assert_eq!(request.generation(), latest_generation);
            }
            _ => panic!("latest initial decode should remain queued"),
        }

        for release in releases {
            release.release();
        }
        controller.shutdown();
    }

    #[test]
    fn folder_scan_workers_are_canceled_without_blocking_replacement_decode() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let mut releases = Vec::new();

        for index in 0..MAX_IN_FLIGHT_DECODE_WORKERS {
            let generation = app
                .begin_image_decode(PathBuf::from(format!("blocked-folder-scan-{index}.png")))
                .generation();
            let (worker, release) = test_decode_worker(generation, DecodeWorkerKind::Initial);
            controller.folder_scan_workers.push(worker);
            releases.push(release);
        }

        let active_generation = app
            .begin_image_decode(PathBuf::from("blocked-active-folder-limit.png"))
            .generation();
        let (active_worker, active_release) =
            test_decode_worker(active_generation, DecodeWorkerKind::Initial);
        let active_cancel = Arc::clone(&active_worker.cancel);
        controller.active_worker = Some(active_worker);
        releases.push(active_release);

        let queued_request = app.begin_image_decode(PathBuf::from("queued-folder-limit.png"));
        let queued_generation = queued_request.generation();
        controller
            .start_initial_decode(null_mut(), queued_request)
            .expect("start initial decode despite folder scan workers");

        assert!(active_cancel.load(Ordering::Acquire));
        assert_eq!(controller.retired_workers.len(), 1);
        assert_eq!(
            controller
                .active_worker
                .as_ref()
                .map(|worker| worker.generation),
            Some(queued_generation)
        );
        assert!(controller.pending_decode.is_none());
        assert_eq!(
            controller.folder_scan_workers.len(),
            MAX_IN_FLIGHT_DECODE_WORKERS
        );
        assert!(controller
            .folder_scan_workers
            .iter()
            .all(|worker| worker.cancel.load(Ordering::Acquire)));

        for release in releases {
            release.release();
        }
        controller.shutdown();
    }

    #[test]
    fn folder_scan_permits_bound_concurrent_scans() {
        let active_count = Arc::new(AtomicUsize::new(0));
        let mut permits = Vec::new();

        for _ in 0..MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS {
            permits.push(
                FolderScanPermit::try_acquire(&active_count)
                    .expect("folder scan permit within limit"),
            );
        }

        assert!(FolderScanPermit::try_acquire(&active_count).is_none());
        assert_eq!(
            active_count.load(Ordering::Acquire),
            MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS
        );

        permits.pop();
        let permit = FolderScanPermit::try_acquire(&active_count)
            .expect("folder scan permit after one release");
        assert_eq!(
            active_count.load(Ordering::Acquire),
            MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS
        );

        drop(permit);
        drop(permits);
        assert_eq!(active_count.load(Ordering::Acquire), 0);
    }

    #[test]
    fn pending_decode_starts_when_only_folder_scan_workers_are_in_flight() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let mut releases = Vec::new();

        for index in 0..MAX_IN_FLIGHT_DECODE_WORKERS {
            let generation = app
                .begin_image_decode(PathBuf::from(format!("pending-folder-scan-{index}.png")))
                .generation();
            let (worker, release) = test_decode_worker(generation, DecodeWorkerKind::Initial);
            controller.folder_scan_workers.push(worker);
            releases.push(release);
        }

        let pending_request = app.begin_image_decode(PathBuf::from("pending-folder-limit.png"));
        controller.pending_decode = Some(PendingDecodeRequest::Initial {
            hwnd_value: 0,
            request: pending_request,
        });

        let drain = controller.drain_messages();

        assert!(drain.start_failures.is_empty());
        assert!(controller.pending_decode.is_none());
        assert!(controller
            .folder_scan_workers
            .iter()
            .all(|worker| worker.cancel.load(Ordering::Acquire)));

        for release in releases {
            release.release();
        }
        controller.shutdown();
    }

    #[test]
    fn shutdown_cancels_decode_workers_without_waiting_for_exit() {
        let mut controller = DecodeController::new();
        let mut app = ViewerApp::new();
        let generation = app
            .begin_image_decode(PathBuf::from("blocked-shutdown.png"))
            .generation();
        let (worker, release_worker) = test_decode_worker(generation, DecodeWorkerKind::Initial);
        let cancel = Arc::clone(&worker.cancel);
        controller.active_worker = Some(worker);
        let (shutdown_done_sender, shutdown_done_receiver) = mpsc::channel();

        let shutdown_thread = thread::spawn(move || {
            controller.shutdown();
            let _ = shutdown_done_sender.send(());
            controller
        });

        let returned_before_worker_exit = shutdown_done_receiver
            .recv_timeout(Duration::from_secs(1))
            .is_ok();
        release_worker.release();
        let controller = shutdown_thread.join().expect("shutdown thread");

        assert!(
            returned_before_worker_exit,
            "shutdown should not wait for decode worker exit"
        );
        assert!(cancel.load(Ordering::Acquire));
        assert!(controller.active_worker.is_none());
        assert!(controller.retired_workers.is_empty());
    }

    fn u16_at(bytes: &[u8], offset: usize) -> u16 {
        u16::from_le_bytes(bytes[offset..offset + 2].try_into().expect("u16 bytes"))
    }

    fn u32_at(bytes: &[u8], offset: usize) -> u32 {
        u32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("u32 bytes"))
    }

    fn i32_at(bytes: &[u8], offset: usize) -> i32 {
        i32::from_le_bytes(bytes[offset..offset + 4].try_into().expect("i32 bytes"))
    }

    fn test_ui_metrics(status_bar_height: i32) -> WindowUiMetrics {
        WindowUiMetrics {
            status_bar_height,
            status_text_horizontal_padding: 10,
            status_font: None,
        }
    }

    fn test_decode_worker(
        generation: crate::domain::DecodeGeneration,
        kind: DecodeWorkerKind,
    ) -> (DecodeWorker, TestDecodeWorkerRelease) {
        let cancel = Arc::new(AtomicBool::new(false));
        let release = TestDecodeWorkerRelease::new();
        let worker_release = release.clone();
        let handle = thread::spawn(move || {
            worker_release.wait();
        });

        (
            DecodeWorker {
                generation,
                kind,
                cancel,
                handle,
                animation_frame: None,
            },
            release,
        )
    }

    #[derive(Clone)]
    struct TestDecodeWorkerRelease {
        state: Arc<(Mutex<bool>, Condvar)>,
    }

    impl TestDecodeWorkerRelease {
        fn new() -> Self {
            Self {
                state: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }

        fn release(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            *released = true;
            condition.notify_all();
        }

        fn wait(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            while !*released {
                released = condition.wait(released).expect("test worker release wait");
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ClipboardTestEvent {
        Empty,
        SetImage,
    }

    #[derive(Default)]
    struct FakeClipboardImageTarget {
        events: Vec<ClipboardTestEvent>,
        empty_fails: bool,
        set_fails: bool,
    }

    impl ClipboardImageTarget for FakeClipboardImageTarget {
        fn empty(&mut self) -> Result<(), ClipboardCopyError> {
            self.events.push(ClipboardTestEvent::Empty);
            if self.empty_fails {
                Err(ClipboardCopyError::EmptyClipboard { code: 4_321 })
            } else {
                Ok(())
            }
        }

        fn set_image_payloads(
            &mut self,
            _payloads: &mut ClipboardImagePayloads,
        ) -> Result<(), ClipboardCopyError> {
            self.events.push(ClipboardTestEvent::SetImage);
            if self.set_fails {
                Err(ClipboardCopyError::SetClipboardData { code: 1_234 })
            } else {
                Ok(())
            }
        }
    }

    fn test_clipboard_payloads() -> ClipboardImagePayloads {
        let image = Rgba8Image::new(1, 1, vec![80, 90, 100, 255]);
        ClipboardImagePayloads::from_rgba8(&image).expect("clipboard payloads")
    }

    struct TestClipboardWindow {
        hwnd: super::HWND,
    }

    impl TestClipboardWindow {
        fn new() -> Result<Self, u32> {
            let class_name = wide_null("STATIC");
            let title = wide_null("j3Pic clipboard test");
            let hwnd = unsafe {
                CreateWindowExW(
                    0,
                    class_name.as_ptr(),
                    title.as_ptr(),
                    0,
                    0,
                    0,
                    1,
                    1,
                    null_mut(),
                    null_mut(),
                    null_mut(),
                    null_mut(),
                )
            };
            if hwnd.is_null() {
                Err(last_error())
            } else {
                Ok(Self { hwnd })
            }
        }
    }

    impl Drop for TestClipboardWindow {
        fn drop(&mut self) {
            unsafe {
                let _ = DestroyWindow(self.hwnd);
            }
        }
    }

    struct TestClipboardGuard;

    impl TestClipboardGuard {
        fn open(hwnd: super::HWND) -> Result<Self, u32> {
            if unsafe { OpenClipboard(hwnd) } == 0 {
                Err(last_error())
            } else {
                Ok(Self)
            }
        }
    }

    impl Drop for TestClipboardGuard {
        fn drop(&mut self) {
            unsafe {
                let _ = CloseClipboard();
            }
        }
    }

    fn clipboard_format_bytes(format: u32, expected_len: usize) -> Result<Vec<u8>, u32> {
        let handle = unsafe { GetClipboardData(format) };
        if handle.is_null() {
            return Err(last_error());
        }
        let actual_len = unsafe { GlobalSize(handle) };
        if actual_len < expected_len {
            return Err(last_error());
        }

        let locked = unsafe { GlobalLock(handle) };
        if locked.is_null() {
            return Err(last_error());
        }
        let bytes = unsafe { std::slice::from_raw_parts(locked.cast::<u8>(), expected_len) };
        let copied = bytes.to_vec();

        unsafe {
            super::SetLastError(ERROR_SUCCESS);
        }
        if unsafe { GlobalUnlock(handle) } == 0 {
            let code = last_error();
            if code != ERROR_SUCCESS {
                return Err(code);
            }
        }

        Ok(copied)
    }

    #[repr(C)]
    struct TestDropFiles {
        p_files: u32,
        pt_x: i32,
        pt_y: i32,
        f_nc: i32,
        f_wide: i32,
    }

    struct TestDropHandle(HDROP);

    impl Drop for TestDropHandle {
        fn drop(&mut self) {
            // SAFETY: The handle is allocated as a DROPFILES-compatible movable global memory
            // block for this process and is released exactly once by this guard.
            unsafe {
                DragFinish(self.0);
            }
        }
    }

    fn test_hdrop(paths: &[&str]) -> HDROP {
        let mut path_list = Vec::new();
        for path in paths {
            path_list.extend(path.encode_utf16());
            path_list.push(0);
        }
        path_list.push(0);

        let header = TestDropFiles {
            p_files: mem::size_of::<TestDropFiles>() as u32,
            pt_x: 0,
            pt_y: 0,
            f_nc: 0,
            f_wide: 1,
        };
        let header_len = mem::size_of::<TestDropFiles>();
        let path_bytes = path_list.len() * mem::size_of::<u16>();
        let total_len = header_len + path_bytes;

        // SAFETY: The allocation is immediately initialized as a DROPFILES block and then read
        // by DragQueryFileW in the same process during this test.
        let handle = unsafe { GlobalAlloc(GMEM_MOVEABLE, total_len) };
        assert!(!handle.is_null(), "GlobalAlloc failed");

        // SAFETY: handle is a live movable global allocation. The locked pointer is valid for
        // total_len bytes until GlobalUnlock below.
        let locked = unsafe { GlobalLock(handle) } as *mut u8;
        assert!(!locked.is_null(), "GlobalLock failed");

        // SAFETY: locked points to total_len writable bytes. Both source buffers are valid and
        // non-overlapping, and their combined lengths exactly match the allocation layout.
        unsafe {
            ptr::copy_nonoverlapping(
                (&header as *const TestDropFiles).cast::<u8>(),
                locked,
                header_len,
            );
            ptr::copy_nonoverlapping(
                path_list.as_ptr().cast::<u8>(),
                locked.add(header_len),
                path_bytes,
            );
            let _ = GlobalUnlock(handle);
        }

        handle as HDROP
    }

    fn has_supported_drop_extension(path: &str) -> bool {
        let wide = path.encode_utf16().collect::<Vec<_>>();
        dropped_path_has_supported_image_extension(&wide)
    }
}
