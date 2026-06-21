use crate::domain::UiLanguage;

const APP_VERSION: &str = env!("CARGO_PKG_VERSION");
const ABOUT_TEXT: &str = include_str!("../about.txt");

pub(crate) fn about_title(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "About j3Pic",
        UiLanguage::Korean => "j3Pic 정보",
    }
}

pub(crate) fn licenses_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "Licenses",
        UiLanguage::Korean => "라이선스",
    }
}

pub(crate) fn version_label() -> String {
    format!("j3Pic {APP_VERSION}")
}

pub(crate) fn source_code_label(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "Source code:",
        UiLanguage::Korean => "소스 코드:",
    }
}

pub(crate) fn about_license_text(_language: UiLanguage) -> String {
    normalize_line_endings(ABOUT_TEXT)
}

fn normalize_line_endings(text: &str) -> String {
    text.replace("\r\n", "\n").replace('\r', "\n")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn about_license_text_uses_about_txt_only() {
        let text = about_license_text(UiLanguage::English);

        assert_eq!(text, normalize_line_endings(ABOUT_TEXT));
        assert!(text.contains("j3Pic"));
        assert!(text.contains("Version: 0.2.0"));
        assert!(text.contains("Copyright (C) 2026 j3Pic Contributors"));
        assert!(text.contains("License: GNU General Public License v3.0 or later"));
        assert!(text.contains("either version 3 of the License, or (at your option) any later"));
        assert!(text.contains("Full license text:\nLICENSE"));
        assert!(text.contains("Source code: binary distributions must include or offer"));
        assert!(text.contains("Corresponding source archive for this binary release:"));
        assert!(text.contains("j3pic-0.2.0-source.zip"));
        assert!(text.contains("THIRD_PARTY_NOTICES.txt"));
        assert!(!text.contains("THIRD PARTY NOTICES"));
        assert!(!text.contains("| image | 0.25.10 | MIT OR Apache-2.0 |"));
    }

    #[test]
    fn version_label_uses_cargo_package_version() {
        assert_eq!(
            version_label(),
            format!("j3Pic {}", env!("CARGO_PKG_VERSION"))
        );
    }

    #[test]
    fn normalize_line_endings_returns_lf_text() {
        assert_eq!(normalize_line_endings("a\r\nb\rc\n"), "a\nb\nc\n");
    }
}
