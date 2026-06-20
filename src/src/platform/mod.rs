use std::path::{Path, PathBuf};

use crate::domain::{export_path_with_format_extension, ExportFormat, UiLanguage};

#[cfg(target_os = "windows")]
pub(crate) mod export_options_dialog;

#[cfg(target_os = "windows")]
pub(crate) mod settings_dialog;

#[cfg(target_os = "linux")]
pub mod linux;

#[cfg(target_os = "windows")]
pub mod win32;

pub(crate) const PROJECT_LINK_URL: &str = "https://github.com/edgarp9";

#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct ExportFileSelection {
    selected_path: PathBuf,
    path: PathBuf,
}

impl ExportFileSelection {
    pub(crate) fn from_selected_path(selected_path: PathBuf, format: ExportFormat) -> Self {
        let path = export_path_with_format_extension(&selected_path, format);
        Self {
            selected_path,
            path,
        }
    }

    pub(crate) fn selected_path(&self) -> &Path {
        &self.selected_path
    }

    pub(crate) fn path(&self) -> &Path {
        &self.path
    }
}

pub(crate) fn paths_refer_to_same_existing_file(left: &Path, right: &Path) -> bool {
    let Ok(left) = std::fs::canonicalize(left) else {
        return false;
    };
    let Ok(right) = std::fs::canonicalize(right) else {
        return false;
    };

    left == right
}

pub(crate) fn corrected_export_path_requires_overwrite_confirmation(
    selected_path: &Path,
    corrected_path: &Path,
) -> bool {
    corrected_path != selected_path && corrected_path.exists()
}

pub(crate) fn same_source_export_message(language: UiLanguage) -> &'static str {
    crate::ui_text::same_source_export_message(language)
}

pub(crate) fn corrected_export_overwrite_message(language: UiLanguage, path: &Path) -> String {
    crate::ui_text::corrected_export_overwrite_message(language, path)
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::time::{SystemTime, UNIX_EPOCH};

    use super::*;

    #[test]
    fn export_file_selection_keeps_selected_and_corrected_paths() {
        let selection = ExportFileSelection::from_selected_path(
            PathBuf::from("/tmp/photo.png"),
            ExportFormat::Jpeg,
        );

        assert_eq!(selection.selected_path(), Path::new("/tmp/photo.png"));
        assert_eq!(selection.path(), Path::new("/tmp/photo.jpg"));
    }

    #[test]
    fn same_existing_file_detection_requires_both_paths_to_exist() {
        let dir = unique_temp_dir("same-existing-export");
        fs::create_dir_all(&dir).expect("test dir");
        let source = dir.join("source.png");
        fs::write(&source, b"image").expect("source file");
        let alias = dir.join(".").join("source.png");
        let missing = dir.join("missing.png");

        assert!(paths_refer_to_same_existing_file(&source, &alias));
        assert!(!paths_refer_to_same_existing_file(&source, &missing));
    }

    #[test]
    fn corrected_export_path_overwrite_confirmation_is_only_needed_for_existing_corrected_path() {
        let dir = unique_temp_dir("corrected-export-overwrite");
        fs::create_dir_all(&dir).expect("test dir");
        let selected = dir.join("photo.png");
        let corrected = dir.join("photo.jpg");

        assert!(!corrected_export_path_requires_overwrite_confirmation(
            &selected, &corrected
        ));
        fs::write(&corrected, b"existing").expect("corrected file");
        assert!(corrected_export_path_requires_overwrite_confirmation(
            &selected, &corrected
        ));
        assert!(!corrected_export_path_requires_overwrite_confirmation(
            &corrected, &corrected
        ));
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system time")
            .as_nanos();
        std::env::temp_dir().join(format!(
            "j3pic-platform-{name}-{}-{nanos}",
            std::process::id()
        ))
    }
}
