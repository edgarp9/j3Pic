use std::env;
use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::{BufWriter, Write};
use std::path::{Path, PathBuf};

use crate::domain::{parse_app_config, serialize_app_config, AppConfig, AppConfigParseError};

const APP_CONFIG_FILE_EXTENSION: &str = "toml";
const APP_CONFIG_TEMP_FILE_CREATE_ATTEMPTS: u32 = 100;

#[derive(Debug)]
pub enum AppConfigLoadError {
    FileRead {
        path: PathBuf,
        source: io::Error,
    },
    Parse {
        path: PathBuf,
        source: AppConfigParseError,
    },
}

impl fmt::Display for AppConfigLoadError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileRead { path, .. } => {
                write!(formatter, "failed to read app config: {}", path.display())
            }
            Self::Parse { path, .. } => {
                write!(formatter, "failed to parse app config: {}", path.display())
            }
        }
    }
}

impl Error for AppConfigLoadError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FileRead { source, .. } => Some(source),
            Self::Parse { source, .. } => Some(source),
        }
    }
}

#[derive(Debug)]
pub enum AppConfigSaveError {
    NoConfigPath,
    NoConfigFileName {
        path: PathBuf,
    },
    CreateDirectory {
        path: PathBuf,
        source: io::Error,
    },
    FileCreate {
        path: PathBuf,
        source: io::Error,
    },
    FileWrite {
        path: PathBuf,
        source: io::Error,
    },
    FileReplace {
        temporary_path: PathBuf,
        path: PathBuf,
        source: io::Error,
    },
}

impl fmt::Display for AppConfigSaveError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NoConfigPath => formatter.write_str("app config path is unavailable"),
            Self::NoConfigFileName { path } => {
                write!(
                    formatter,
                    "app config path has no file name: {}",
                    path.display()
                )
            }
            Self::CreateDirectory { path, .. } => {
                write!(
                    formatter,
                    "failed to create app config directory: {}",
                    path.display()
                )
            }
            Self::FileCreate { path, .. } => {
                write!(
                    formatter,
                    "failed to create temporary app config: {}",
                    path.display()
                )
            }
            Self::FileWrite { path, .. } => {
                write!(formatter, "failed to write app config: {}", path.display())
            }
            Self::FileReplace {
                temporary_path,
                path,
                ..
            } => write!(
                formatter,
                "failed to replace app config: {} -> {}",
                temporary_path.display(),
                path.display()
            ),
        }
    }
}

impl Error for AppConfigSaveError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::CreateDirectory { source, .. }
            | Self::FileCreate { source, .. }
            | Self::FileWrite { source, .. }
            | Self::FileReplace { source, .. } => Some(source),
            Self::NoConfigPath | Self::NoConfigFileName { .. } => None,
        }
    }
}

pub fn app_config_file_path() -> Option<PathBuf> {
    env::current_exe()
        .ok()
        .and_then(|path| app_config_file_path_for_executable(&path))
}

pub(super) fn app_config_file_path_for_executable(executable_path: &Path) -> Option<PathBuf> {
    let parent = executable_path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())?;
    let file_stem = executable_path
        .file_stem()
        .filter(|file_stem| !file_stem.is_empty())?;
    let mut file_name = file_stem.to_os_string();
    file_name.push(".");
    file_name.push(APP_CONFIG_FILE_EXTENSION);
    Some(parent.join(file_name))
}

pub fn load_app_config() -> Result<AppConfig, AppConfigLoadError> {
    let Some(path) = app_config_file_path() else {
        return Ok(AppConfig::default());
    };

    load_app_config_from_path(&path)
}

pub fn load_app_config_from_path(path: impl AsRef<Path>) -> Result<AppConfig, AppConfigLoadError> {
    let path = path.as_ref();
    match fs::read_to_string(path) {
        Ok(contents) => parse_app_config(&contents).map_err(|source| AppConfigLoadError::Parse {
            path: path.to_path_buf(),
            source,
        }),
        Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
        Err(source) => Err(AppConfigLoadError::FileRead {
            path: path.to_path_buf(),
            source,
        }),
    }
}

pub fn save_app_config(config: &AppConfig) -> Result<(), AppConfigSaveError> {
    let path = app_config_file_path().ok_or(AppConfigSaveError::NoConfigPath)?;
    save_app_config_to_path(&path, config)
}

pub fn save_app_config_to_path(
    path: impl AsRef<Path>,
    config: &AppConfig,
) -> Result<(), AppConfigSaveError> {
    let path = path.as_ref();
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .ok_or_else(|| AppConfigSaveError::NoConfigFileName {
            path: path.to_path_buf(),
        })?;
    fs::create_dir_all(parent).map_err(|source| AppConfigSaveError::CreateDirectory {
        path: parent.to_path_buf(),
        source,
    })?;

    let (temporary_path, file) = create_config_temporary_file(path)?;
    if let Err(error) = write_config_file(&temporary_path, file, config) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    super::replace_file(&temporary_path, path).map_err(|source| {
        let _ = fs::remove_file(&temporary_path);
        AppConfigSaveError::FileReplace {
            temporary_path,
            path: path.to_path_buf(),
            source,
        }
    })
}

fn create_config_temporary_file(path: &Path) -> Result<(PathBuf, File), AppConfigSaveError> {
    let file_name = path
        .file_name()
        .ok_or_else(|| AppConfigSaveError::NoConfigFileName {
            path: path.to_path_buf(),
        })?;
    let process_id = std::process::id();

    for attempt in 0..APP_CONFIG_TEMP_FILE_CREATE_ATTEMPTS {
        let mut temporary_name = file_name.to_os_string();
        temporary_name.push(format!(".tmp.{process_id}.{attempt}"));
        let temporary_path = path.with_file_name(temporary_name);
        match OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&temporary_path)
        {
            Ok(file) => return Ok((temporary_path, file)),
            Err(source) if source.kind() == io::ErrorKind::AlreadyExists => continue,
            Err(source) => {
                return Err(AppConfigSaveError::FileCreate {
                    path: temporary_path,
                    source,
                });
            }
        }
    }

    Err(AppConfigSaveError::FileCreate {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary app config file",
        ),
    })
}

fn write_config_file(
    path: &Path,
    file: File,
    config: &AppConfig,
) -> Result<(), AppConfigSaveError> {
    let mut writer = BufWriter::new(file);
    writer
        .write_all(serialize_app_config(config).as_bytes())
        .and_then(|()| writer.flush())
        .and_then(|()| writer.get_ref().sync_all())
        .map_err(|source| AppConfigSaveError::FileWrite {
            path: path.to_path_buf(),
            source,
        })
}
