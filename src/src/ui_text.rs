use std::path::Path;

use crate::domain::{Command, UiLanguage};

pub(crate) const UI_LANGUAGE_LABELS: &[&str] = &["English", "한국어"];

pub(crate) fn ui_language_index(language: UiLanguage) -> usize {
    match language {
        UiLanguage::English => 0,
        UiLanguage::Korean => 1,
    }
}

pub(crate) fn ui_language_from_index(index: usize) -> Option<UiLanguage> {
    match index {
        0 => Some(UiLanguage::English),
        1 => Some(UiLanguage::Korean),
        _ => None,
    }
}

pub(crate) fn context_menu_label(language: UiLanguage, command: Command) -> &'static str {
    match language {
        UiLanguage::English => match command {
            Command::OpenImage => "Open...",
            Command::ExportImage => "Export...",
            Command::CopyImageToClipboard => "Copy to Clipboard",
            Command::ActualSize => "Actual Size",
            Command::FitToWindow => "Fit to Window",
            Command::RotateClockwise => "Rotate Clockwise",
            Command::RotateCounterClockwise => "Rotate Counterclockwise",
            Command::ToggleFullscreen => "Fullscreen",
            Command::OpenAbout => "About...",
            Command::OpenSettings => "Settings...",
            _ => "",
        },
        UiLanguage::Korean => match command {
            Command::OpenImage => "열기...",
            Command::ExportImage => "내보내기...",
            Command::CopyImageToClipboard => "클립보드에 복사",
            Command::ActualSize => "실제 크기",
            Command::FitToWindow => "창에 맞춤",
            Command::RotateClockwise => "시계 방향 회전",
            Command::RotateCounterClockwise => "반시계 방향 회전",
            Command::ToggleFullscreen => "전체 화면",
            Command::OpenAbout => "정보...",
            Command::OpenSettings => "설정...",
            _ => "",
        },
    }
}

pub(crate) fn same_source_export_message(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => {
            "Cannot export to the same path as the source image.\n\nChoose a different file name."
        }
        UiLanguage::Korean => {
            "원본 이미지 파일과 같은 경로로는 내보낼 수 없습니다.\n\n다른 파일 이름을 선택해 주세요."
        }
    }
}

pub(crate) fn corrected_export_overwrite_message(language: UiLanguage, path: &Path) -> String {
    match language {
        UiLanguage::English => format!(
            "The file extension was changed to match the selected format.\n\nThe file already exists. Overwrite it?\n\n{}",
            path.display()
        ),
        UiLanguage::Korean => format!(
            "선택한 포맷에 맞춰 저장 확장자를 변경했습니다.\n\n파일이 이미 있습니다. 덮어쓸까요?\n\n{}",
            path.display()
        ),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn export_overwrite_message(language: UiLanguage, path: &Path) -> String {
    match language {
        UiLanguage::English => format!(
            "The file already exists. Overwrite it?\n\n{}",
            path.display()
        ),
        UiLanguage::Korean => format!("파일이 이미 있습니다. 덮어쓸까요?\n\n{}", path.display()),
    }
}

#[cfg(target_os = "linux")]
pub(crate) fn yes_no_buttons(language: UiLanguage) -> [(&'static str, bool); 2] {
    match language {
        UiLanguage::English => [("Yes", true), ("No", false)],
        UiLanguage::Korean => [("예", true), ("아니요", false)],
    }
}

pub(crate) fn settings_title(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "j3Pic Settings",
        UiLanguage::Korean => "j3Pic 설정",
    }
}

pub(crate) fn settings_group_titles(language: UiLanguage) -> &'static [&'static str; 7] {
    match language {
        UiLanguage::English => &[
            "General",
            "Zoom",
            "Animation",
            "Navigation",
            "Large Images / Memory",
            "Export",
            "Shortcuts",
        ],
        UiLanguage::Korean => &[
            "일반",
            "줌",
            "애니메이션",
            "탐색",
            "대용량 이미지/메모리",
            "내보내기",
            "단축키",
        ],
    }
}

pub(crate) fn settings_view_mode_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["Fit to Window", "Actual Size"],
        UiLanguage::Korean => &["창에 맞춤", "실제 크기"],
    }
}

pub(crate) fn settings_scaling_quality_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["Nearest Pixel", "Balanced", "High Quality"],
        UiLanguage::Korean => &["가장 가까운 픽셀", "균형", "고품질"],
    }
}

pub(crate) fn settings_export_policy_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["Source Format", "PNG", "JPEG", "BMP", "WebP", "ICO"],
        UiLanguage::Korean => &["원본 형식", "PNG", "JPEG", "BMP", "WebP", "ICO"],
    }
}

pub(crate) fn settings_wheel_shortcut_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["Mouse Wheel", "Ctrl+Mouse Wheel"],
        UiLanguage::Korean => &["마우스휠", "Ctrl+마우스휠"],
    }
}

pub(crate) fn settings_drag_shortcut_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["Left Button Drag", "Ctrl+Left Button Drag"],
        UiLanguage::Korean => &["마우스 왼쪽 클릭 이동", "Ctrl+마우스 왼쪽 클릭 이동"],
    }
}

pub(crate) fn export_dialog_title(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "Export Options",
        UiLanguage::Korean => "내보내기 옵션",
    }
}

pub(crate) fn export_rotation_labels(language: UiLanguage) -> &'static [&'static str] {
    match language {
        UiLanguage::English => &["0 deg", "90 deg clockwise", "180 deg", "270 deg clockwise"],
        UiLanguage::Korean => &["0도", "90도 시계 방향", "180도", "270도 시계 방향"],
    }
}
