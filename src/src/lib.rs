pub(crate) mod about;
pub mod app;
pub mod domain;
pub mod infra;
pub mod platform;
pub(crate) mod ui_text;

use std::ffi::OsString;
use std::path::PathBuf;

#[cfg(target_os = "windows")]
pub fn run() -> Result<i32, platform::win32::Win32Error> {
    run_with_args(std::env::args_os().skip(1))
}

#[cfg(target_os = "windows")]
pub fn run_with_args<I>(args: I) -> Result<i32, platform::win32::Win32Error>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let startup = startup_config_from_load_result(
        infra::load_app_config(),
        infra::app_config_file_path().is_some(),
    );
    let startup_args = startup_args_from_iter(args);
    platform::win32::run_native_viewer(
        app::ViewerApp::with_config(startup.config),
        startup.save_config_on_destroy,
        startup_args.initial_image_path,
    )
}

#[cfg(target_os = "linux")]
pub fn run() -> Result<i32, platform::linux::GtkError> {
    run_with_args(std::env::args_os().skip(1))
}

#[cfg(target_os = "linux")]
pub fn run_with_args<I>(args: I) -> Result<i32, platform::linux::GtkError>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let startup = startup_config_from_load_result(
        infra::load_app_config(),
        infra::app_config_file_path().is_some(),
    );
    let startup_args = startup_args_from_iter(args);
    platform::linux::run_native_viewer(
        app::ViewerApp::with_config(startup.config),
        startup.save_config_on_destroy,
        startup_args.initial_image_path,
    )
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupConfig {
    config: domain::AppConfig,
    save_config_on_destroy: bool,
}

fn startup_config_from_load_result(
    result: Result<domain::AppConfig, infra::AppConfigLoadError>,
    can_save_config: bool,
) -> StartupConfig {
    match result {
        Ok(config) => StartupConfig {
            config,
            save_config_on_destroy: can_save_config,
        },
        Err(_error) => StartupConfig {
            config: domain::AppConfig::default(),
            save_config_on_destroy: false,
        },
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn run() -> Result<i32, UnsupportedPlatformError> {
    run_with_args(std::env::args_os().skip(1))
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
pub fn run_with_args<I>(_args: I) -> Result<i32, UnsupportedPlatformError>
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    Err(UnsupportedPlatformError)
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
#[derive(Debug)]
pub struct UnsupportedPlatformError;

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl std::fmt::Display for UnsupportedPlatformError {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter.write_str("j3Pic supports native Windows and Linux builds")
    }
}

#[cfg(not(any(target_os = "windows", target_os = "linux")))]
impl std::error::Error for UnsupportedPlatformError {}

#[derive(Debug, Clone, PartialEq, Eq)]
struct StartupArgs {
    initial_image_path: Option<PathBuf>,
}

fn startup_args_from_iter<I>(args: I) -> StartupArgs
where
    I: IntoIterator,
    I::Item: Into<OsString>,
{
    let initial_image_path = args
        .into_iter()
        .map(Into::into)
        .find(|arg: &OsString| !arg.as_os_str().is_empty())
        .map(PathBuf::from);

    StartupArgs { initial_image_path }
}

#[cfg(test)]
mod tests {
    use std::ffi::OsString;
    use std::io;
    use std::path::PathBuf;

    use crate::domain::{AppConfig, AppConfigParseError, ScalingQuality, ViewMode};
    use crate::infra::AppConfigLoadError;

    use super::{
        startup_args_from_iter, startup_config_from_load_result, StartupArgs, StartupConfig,
    };

    #[test]
    fn startup_config_uses_loaded_config_when_available() {
        let config = AppConfig::new(
            None,
            ViewMode::ActualSize,
            ScalingQuality::Nearest,
            None,
            80,
            false,
        );

        assert_eq!(
            startup_config_from_load_result(Ok(config.clone()), true),
            StartupConfig {
                config,
                save_config_on_destroy: true
            }
        );
    }

    #[test]
    fn startup_config_skips_destroy_save_without_config_path() {
        assert_eq!(
            startup_config_from_load_result(Ok(AppConfig::default()), false),
            StartupConfig {
                config: AppConfig::default(),
                save_config_on_destroy: false
            }
        );
    }

    #[test]
    fn startup_config_falls_back_to_defaults_without_destroy_save_when_config_load_fails() {
        let error = AppConfigLoadError::FileRead {
            path: PathBuf::from("j3pic.toml"),
            source: io::Error::new(io::ErrorKind::PermissionDenied, "denied"),
        };

        assert_eq!(
            startup_config_from_load_result(Err(error), true),
            StartupConfig {
                config: AppConfig::default(),
                save_config_on_destroy: false
            }
        );
    }

    #[test]
    fn startup_config_blocks_destroy_save_when_config_parse_fails() {
        let error = AppConfigLoadError::Parse {
            path: PathBuf::from("j3pic.toml"),
            source: AppConfigParseError::UnsupportedVersion { line: 1 },
        };

        assert_eq!(
            startup_config_from_load_result(Err(error), true),
            StartupConfig {
                config: AppConfig::default(),
                save_config_on_destroy: false
            }
        );
    }

    #[test]
    fn startup_args_use_first_argument_as_initial_image_path() {
        assert_eq!(
            startup_args_from_iter(
                ["C:/Images/photo one.png", "C:/Images/photo two.png"].map(OsString::from)
            ),
            StartupArgs {
                initial_image_path: Some(PathBuf::from("C:/Images/photo one.png"))
            }
        );
    }

    #[test]
    fn startup_args_ignore_empty_arguments_before_initial_image_path() {
        assert_eq!(
            startup_args_from_iter(["", "C:/Images/photo.png"].map(OsString::from)),
            StartupArgs {
                initial_image_path: Some(PathBuf::from("C:/Images/photo.png"))
            }
        );
    }

    #[test]
    fn startup_args_are_empty_without_arguments() {
        assert_eq!(
            startup_args_from_iter(std::iter::empty::<OsString>()),
            StartupArgs {
                initial_image_path: None
            }
        );
    }
}
