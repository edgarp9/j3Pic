use std::mem;
use std::ptr;
use std::sync::{Once, OnceLock};

use windows_sys::core::{BOOL, HRESULT};
use windows_sys::Win32::Foundation::{FARPROC, HWND, LPARAM, RECT};
use windows_sys::Win32::Graphics::Gdi::{
    CreateFontW, DeleteObject, GetDC, GetDeviceCaps, ReleaseDC, CLEARTYPE_QUALITY,
    CLIP_DEFAULT_PRECIS, DEFAULT_CHARSET, DEFAULT_PITCH, FF_DONTCARE, FW_NORMAL, HDC, HFONT,
    HGDIOBJ, LOGPIXELSY, OUT_DEFAULT_PRECIS,
};
use windows_sys::Win32::System::LibraryLoader::{
    GetProcAddress, LoadLibraryExA, LOAD_LIBRARY_SEARCH_SYSTEM32,
};
use windows_sys::Win32::UI::HiDpi::{
    DPI_AWARENESS_CONTEXT, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
    DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2, DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
    PROCESS_DPI_AWARENESS, PROCESS_PER_MONITOR_DPI_AWARE, PROCESS_SYSTEM_DPI_AWARE,
};

const DEFAULT_DPI: i32 = 96;
const POINTS_PER_INCH: i32 = 72;
const UI_FONT_FAMILY: &str = "Malgun Gothic";

type SetProcessDpiAwarenessContextFn = unsafe extern "system" fn(DPI_AWARENESS_CONTEXT) -> BOOL;
type SetProcessDpiAwarenessFn = unsafe extern "system" fn(PROCESS_DPI_AWARENESS) -> HRESULT;
type SetProcessDpiAwareFn = unsafe extern "system" fn() -> BOOL;
type EnableNonClientDpiScalingFn = unsafe extern "system" fn(HWND) -> BOOL;
type GetDpiForWindowFn = unsafe extern "system" fn(HWND) -> u32;

macro_rules! load_function {
    ($library:literal, $function:ident, $function_type:ty) => {{
        let procedure = load_procedure(
            concat!($library, "\0").as_bytes(),
            concat!(stringify!($function), "\0").as_bytes(),
        );
        procedure.map(|procedure| {
            // SAFETY: Each call site pairs the symbol name with the matching Win32 signature.
            unsafe {
                mem::transmute::<unsafe extern "system" fn() -> isize, $function_type>(procedure)
            }
        })
    }};
}

pub(crate) fn enable_process_dpi_awareness() {
    static ENABLE_DPI_AWARENESS: Once = Once::new();

    ENABLE_DPI_AWARENESS.call_once(|| {
        if let Some(set_awareness_context) = load_function!(
            "user32.dll",
            SetProcessDpiAwarenessContext,
            SetProcessDpiAwarenessContextFn
        ) {
            for context in process_dpi_awareness_context_candidates() {
                // SAFETY: Process DPI awareness must be set before creating UI. Failure is
                // expected when the process already has an awareness context or an older OS
                // rejects a context, so the next fallback can be tried.
                if unsafe { set_awareness_context(*context) } != 0 {
                    return;
                }
            }
        }

        if let Some(set_awareness) = load_function!(
            "shcore.dll",
            SetProcessDpiAwareness,
            SetProcessDpiAwarenessFn
        ) {
            for awareness in process_dpi_awareness_candidates() {
                // SAFETY: This process-wide fallback is best-effort on Windows 8.1. Failure is
                // expected when DPI awareness is already set or the mode is not supported.
                if hresult_succeeded(unsafe { set_awareness(*awareness) }) {
                    return;
                }
            }
        }

        if let Some(set_dpi_aware) =
            load_function!("user32.dll", SetProcessDPIAware, SetProcessDpiAwareFn)
        {
            // SAFETY: Last-resort system-DPI awareness fallback for older Windows versions.
            let _ = unsafe { set_dpi_aware() };
        }
    });
}

fn process_dpi_awareness_context_candidates() -> &'static [DPI_AWARENESS_CONTEXT] {
    &[
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
    ]
}

fn process_dpi_awareness_candidates() -> &'static [PROCESS_DPI_AWARENESS] {
    &[PROCESS_PER_MONITOR_DPI_AWARE, PROCESS_SYSTEM_DPI_AWARE]
}

fn hresult_succeeded(result: HRESULT) -> bool {
    result >= 0
}

pub(crate) fn enable_non_client_dpi_scaling(hwnd: HWND) {
    if hwnd.is_null() {
        return;
    }

    if let Some(enable_scaling) = load_function!(
        "user32.dll",
        EnableNonClientDpiScaling,
        EnableNonClientDpiScalingFn
    ) {
        // SAFETY: hwnd is a live top-level window during WM_NCCREATE.
        let _ = unsafe { enable_scaling(hwnd) };
    }
}

pub(crate) fn dpi_y_for_window(hwnd: HWND) -> i32 {
    if !hwnd.is_null() {
        if let Some(dpi) = dpi_for_window(hwnd) {
            return dpi;
        }
    }

    dpi_y_from_dc(hwnd).unwrap_or(DEFAULT_DPI)
}

pub(crate) fn scale_i32_for_dpi(value: i32, dpi_y: i32) -> i32 {
    let dpi_y = dpi_y.max(1);
    let scaled = i64::from(value) * i64::from(dpi_y);
    let rounded = if scaled >= 0 {
        (scaled + i64::from(DEFAULT_DPI / 2)) / i64::from(DEFAULT_DPI)
    } else {
        (scaled - i64::from(DEFAULT_DPI / 2)) / i64::from(DEFAULT_DPI)
    };
    rounded.clamp(i64::from(i32::MIN), i64::from(i32::MAX)) as i32
}

pub(crate) fn suggested_rect_from_dpi_change(lparam: LPARAM) -> Option<RECT> {
    if lparam == 0 {
        return None;
    }

    // SAFETY: WM_DPICHANGED supplies a RECT pointer in lparam for the duration of the message.
    unsafe { (lparam as *const RECT).as_ref().copied() }
}

pub(crate) struct DpiFont {
    handle: HFONT,
}

impl DpiFont {
    pub(crate) fn new_ui_font(point_size: i32, dpi_y: i32) -> Option<Self> {
        let height = -point_size_pixel_height(point_size, dpi_y);
        let face_name = wide_null(UI_FONT_FAMILY);

        // SAFETY: face_name is a null-terminated UTF-16 string valid for the call.
        let handle = unsafe {
            CreateFontW(
                height,
                0,
                0,
                0,
                FW_NORMAL as i32,
                0,
                0,
                0,
                u32::from(DEFAULT_CHARSET),
                u32::from(OUT_DEFAULT_PRECIS),
                u32::from(CLIP_DEFAULT_PRECIS),
                u32::from(CLEARTYPE_QUALITY),
                u32::from(DEFAULT_PITCH | FF_DONTCARE),
                face_name.as_ptr(),
            )
        };
        if handle.is_null() {
            None
        } else {
            Some(Self { handle })
        }
    }

    pub(crate) fn handle(&self) -> HFONT {
        self.handle
    }
}

impl Drop for DpiFont {
    fn drop(&mut self) {
        // SAFETY: handle is owned by this DpiFont and deleted exactly once.
        unsafe {
            let _ = DeleteObject(self.handle as HGDIOBJ);
        }
    }
}

fn dpi_for_window(hwnd: HWND) -> Option<i32> {
    static GET_DPI_FOR_WINDOW: OnceLock<Option<GetDpiForWindowFn>> = OnceLock::new();

    let get_dpi = (*GET_DPI_FOR_WINDOW
        .get_or_init(|| load_function!("user32.dll", GetDpiForWindow, GetDpiForWindowFn)))?;

    // SAFETY: hwnd is checked by the caller and the API only queries DPI for that window.
    let dpi = unsafe { get_dpi(hwnd) };
    i32::try_from(dpi).ok().filter(|dpi| *dpi > 0)
}

fn dpi_y_from_dc(hwnd: HWND) -> Option<i32> {
    // SAFETY: GetDC accepts a null or live HWND and returns a DC that must be released.
    let hdc = unsafe { GetDC(hwnd) };
    if hdc.is_null() {
        return None;
    }

    let dpi_y = get_dpi_y(hdc);

    // SAFETY: hdc was returned by GetDC for the same hwnd.
    unsafe {
        let _ = ReleaseDC(hwnd, hdc);
    }

    dpi_y
}

fn get_dpi_y(hdc: HDC) -> Option<i32> {
    // SAFETY: hdc is a live device context. GetDeviceCaps only reads device metadata.
    let dpi_y = unsafe { GetDeviceCaps(hdc, LOGPIXELSY as i32) };
    (dpi_y > 0).then_some(dpi_y)
}

fn point_size_pixel_height(point_size: i32, dpi_y: i32) -> i32 {
    let point_size = point_size.max(1);
    let dpi_y = dpi_y.max(1);
    let pixels = i64::from(point_size) * i64::from(dpi_y);
    ((pixels + i64::from(POINTS_PER_INCH / 2)) / i64::from(POINTS_PER_INCH))
        .clamp(1, i64::from(i32::MAX)) as i32
}

fn wide_null(value: &str) -> Vec<u16> {
    value.encode_utf16().chain(std::iter::once(0)).collect()
}

fn load_procedure(library: &'static [u8], function: &'static [u8]) -> FARPROC {
    debug_assert_eq!(library.last(), Some(&0));
    debug_assert_eq!(function.last(), Some(&0));

    // SAFETY: library is a static null-terminated ASCII DLL name. Restricting the search to
    // System32 avoids loading same-named DLLs from process-controlled directories.
    let module = unsafe {
        LoadLibraryExA(
            library.as_ptr(),
            ptr::null_mut(),
            LOAD_LIBRARY_SEARCH_SYSTEM32,
        )
    };
    if module.is_null() {
        return None;
    }

    // SAFETY: function is a static null-terminated ASCII symbol name for module.
    unsafe { GetProcAddress(module, function.as_ptr()) }
}

#[cfg(test)]
mod tests {
    use windows_sys::Win32::UI::HiDpi::{
        DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE, DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
        DPI_AWARENESS_CONTEXT_SYSTEM_AWARE, PROCESS_PER_MONITOR_DPI_AWARE,
        PROCESS_SYSTEM_DPI_AWARE,
    };

    use super::{
        hresult_succeeded, point_size_pixel_height, process_dpi_awareness_candidates,
        process_dpi_awareness_context_candidates, scale_i32_for_dpi,
    };

    #[test]
    fn scale_i32_for_dpi_uses_96_dpi_as_identity() {
        assert_eq!(scale_i32_for_dpi(28, 96), 28);
        assert_eq!(scale_i32_for_dpi(10, 144), 15);
        assert_eq!(scale_i32_for_dpi(-10, 144), -15);
    }

    #[test]
    fn point_size_pixel_height_scales_with_dpi() {
        assert_eq!(point_size_pixel_height(9, 96), 12);
        assert_eq!(point_size_pixel_height(9, 144), 18);
        assert_eq!(point_size_pixel_height(9, 192), 24);
    }

    #[test]
    fn process_dpi_awareness_prefers_per_monitor_v1_before_v2() {
        assert_eq!(
            process_dpi_awareness_context_candidates(),
            &[
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE,
                DPI_AWARENESS_CONTEXT_PER_MONITOR_AWARE_V2,
                DPI_AWARENESS_CONTEXT_SYSTEM_AWARE,
            ]
        );
        assert_eq!(
            process_dpi_awareness_candidates(),
            &[PROCESS_PER_MONITOR_DPI_AWARE, PROCESS_SYSTEM_DPI_AWARE]
        );
    }

    #[test]
    fn hresult_succeeded_accepts_non_negative_results() {
        assert!(hresult_succeeded(0));
        assert!(hresult_succeeded(1));
        assert!(!hresult_succeeded(-1));
    }
}
