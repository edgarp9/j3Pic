use std::error::Error;
use std::fmt;
use std::fs::{self, File, OpenOptions};
use std::io;
use std::io::{BufRead, BufReader, BufWriter, Read, Seek, SeekFrom, Write};
use std::ops::Range;
#[cfg(target_os = "windows")]
use std::os::windows::ffi::OsStrExt;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant, SystemTime};

#[cfg(target_os = "windows")]
use windows_sys::Win32::Storage::FileSystem::{
    MoveFileExW, MOVEFILE_REPLACE_EXISTING, MOVEFILE_WRITE_THROUGH,
};

use image::codecs::bmp::BmpEncoder;
use image::codecs::gif::GifDecoder;
use image::codecs::ico::{IcoEncoder, IcoFrame};
use image::codecs::jpeg::JpegEncoder;
use image::codecs::png::PngEncoder;
use image::codecs::webp::WebPDecoder;
use image::codecs::webp::WebPEncoder;
use image::error::{DecodingError, LimitError, LimitErrorKind, ParameterError, ParameterErrorKind};
use image::imageops::{resize, FilterType};
use image::metadata::LoopCount as ImageLoopCount;
use image::{
    AnimationDecoder, ColorType, ExtendedColorType, GenericImageView, ImageDecoder, ImageEncoder,
    ImageError, ImageFormat, ImageReader, Limits, Rgb, Rgba,
};
use jpeg_decoder::{Decoder as JpegPreviewDecoder, PixelFormat as JpegPreviewPixelFormat};

use crate::domain::{
    display_orientation, is_image_too_large, is_large_image, orient_pixel_image,
    preview_size_for_viewport, scaling_quality_for_render, should_load_static_preview_first,
    should_retain_full_resolution, supported_image_format_for_path, AnimationLoopPolicy,
    AnimationPlayback, AnimationTimingSettings, ExportFormat, ExportOptions,
    ExportSaveErrorCategory, ImageBufferKind, ImageFileVersion, ImageLoadFailureStage,
    ImageMemoryPolicy, ImageMetadata, ImageOpenErrorCategory, ImageOrientation, ImageRotation,
    ImageSize, LoadedImage, PixelFormat, PixelImage, RenderReadyImage, RenderReadySpec, Rgb8Image,
    RgbColor, Rgba8Image, ScalingQuality, SupportedImageFormat, UiLanguage, ViewMode,
    ViewTransform, ViewportSize, DEFAULT_IMAGE_MEMORY_POLICY, RGB8_BYTES_PER_PIXEL,
    RGBA8_BYTES_PER_PIXEL,
};

const SUPPORTED_FORMATS_TEXT: &str = "jpg, jpeg, png, bmp, gif, webp, ico, tif, tiff, tga";
const EXPORT_TEMP_FILE_CREATE_ATTEMPTS: u32 = 100;
const PNG_EXPORT_OXIPNG_PRESET: u8 = 2;
// Keep oxipng as an opportunistic PNG pass. Tiny outputs mostly pay fixed
// overhead, while multi-MiB outputs can keep the single export worker busy.
const PNG_EXPORT_OXIPNG_MIN_INPUT_BYTES: u64 = 256 * 1024;
const PNG_EXPORT_OXIPNG_MAX_INPUT_BYTES: u64 = 2 * 1024 * 1024;
const WIN_ERROR_FILE_NOT_FOUND: i32 = 2;
const WIN_ERROR_PATH_NOT_FOUND: i32 = 3;
const WIN_ERROR_ACCESS_DENIED: i32 = 5;
const WIN_ERROR_SHARING_VIOLATION: i32 = 32;
const WIN_ERROR_LOCK_VIOLATION: i32 = 33;
const ANIMATION_FRAME_CACHE_RADIUS: usize = 4;
const ANIMATION_FRAME_PREFETCH_RADIUS: usize = ANIMATION_FRAME_CACHE_RADIUS * 4;
const ANIMATION_INITIAL_CACHE_FRAME_LIMIT: usize = ANIMATION_FRAME_CACHE_RADIUS + 1;
const ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT: usize = ANIMATION_FRAME_PREFETCH_RADIUS * 4;
const ANIMATION_DELIVERED_PREFETCH_FRAME_LIMIT: usize = ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT;
const ANIMATION_DELIVERED_PREFETCH_BYTE_LIMIT: usize = 8 * 1024 * 1024;
const ANIMATION_PARALLEL_METADATA_MIN_FILE_BYTES: u64 = 1024 * 1024;
const ANIMATION_METADATA_JOIN_POLL_INTERVAL: Duration = Duration::from_millis(2);
const BMP_FILE_HEADER_LEN: usize = 14;
const BMP_MIN_DIB_HEADER_LEN: u32 = 40;
const BMP_V4_DIB_HEADER_LEN: u32 = 108;
const BMP_V5_DIB_HEADER_LEN: u32 = 124;
const BMP_COMPRESSION_RGB: u32 = 0;
const BMP_COMPRESSION_BITFIELDS: u32 = 3;
const BMP_BITS_PER_BYTE: u64 = 8;
const BMP_ROW_ALIGN_BITS: u64 = 32;
const BMP_ROW_ALIGN_BYTES: u64 = 4;
const BMP_RGBA_RED_MASK: u32 = 0x00ff0000;
const BMP_RGBA_GREEN_MASK: u32 = 0x0000ff00;
const BMP_RGBA_BLUE_MASK: u32 = 0x000000ff;
const BMP_RGBA_ALPHA_MASK: u32 = 0xff000000;
const STATIC_PREVIEW_SOURCE_DECODE_BYTE_MULTIPLIER: usize = 8;
const STATIC_PREVIEW_MIN_SOURCE_DECODE_BYTES: usize = 64 * 1024 * 1024;
const ICO_EXPORT_FRAME_EDGES: [u32; 4] = [16, 32, 48, 256];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageOpenProfile {
    stages: Vec<ImageOpenProfileStage>,
}

impl ImageOpenProfile {
    pub fn stages(&self) -> &[ImageOpenProfileStage] {
        &self.stages
    }

    pub fn total_duration(&self) -> Duration {
        self.stages
            .last()
            .map(ImageOpenProfileStage::total_duration)
            .unwrap_or_default()
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageOpenProfileStage {
    name: &'static str,
    duration: Duration,
    total_duration: Duration,
}

impl ImageOpenProfileStage {
    fn new(name: &'static str, duration: Duration, total_duration: Duration) -> Self {
        Self {
            name,
            duration,
            total_duration,
        }
    }

    pub fn name(&self) -> &'static str {
        self.name
    }

    pub fn duration(&self) -> Duration {
        self.duration
    }

    pub fn total_duration(&self) -> Duration {
        self.total_duration
    }
}

#[derive(Debug)]
pub struct ImageOpenProfiler {
    started_at: Instant,
    last_stage_at: Instant,
    stages: Vec<ImageOpenProfileStage>,
}

impl ImageOpenProfiler {
    pub fn new() -> Self {
        let now = Instant::now();
        Self {
            started_at: now,
            last_stage_at: now,
            stages: Vec::new(),
        }
    }

    pub fn record_stage(&mut self, name: &'static str) {
        let now = Instant::now();
        self.stages.push(ImageOpenProfileStage::new(
            name,
            now.saturating_duration_since(self.last_stage_at),
            now.saturating_duration_since(self.started_at),
        ));
        self.last_stage_at = now;
    }

    pub fn finish(self) -> ImageOpenProfile {
        ImageOpenProfile {
            stages: self.stages,
        }
    }
}

impl Default for ImageOpenProfiler {
    fn default() -> Self {
        Self::new()
    }
}

fn record_image_open_profile_stage(
    profiler: &mut Option<&mut ImageOpenProfiler>,
    name: &'static str,
) {
    if let Some(profiler) = profiler.as_mut() {
        profiler.record_stage(name);
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum StaticFullResolutionCacheMode {
    Refresh,
    Preserve,
}

impl StaticFullResolutionCacheMode {
    fn refreshes_cache(self) -> bool {
        matches!(self, Self::Refresh)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum FileIoErrorCategory {
    PermissionDenied,
    NotFound,
    FileLocked,
    Other,
}

fn classify_file_io_error(source: &io::Error) -> FileIoErrorCategory {
    match source.raw_os_error() {
        Some(WIN_ERROR_SHARING_VIOLATION | WIN_ERROR_LOCK_VIOLATION) => {
            FileIoErrorCategory::FileLocked
        }
        Some(WIN_ERROR_FILE_NOT_FOUND | WIN_ERROR_PATH_NOT_FOUND) => FileIoErrorCategory::NotFound,
        Some(WIN_ERROR_ACCESS_DENIED) => FileIoErrorCategory::PermissionDenied,
        _ => match source.kind() {
            io::ErrorKind::PermissionDenied => FileIoErrorCategory::PermissionDenied,
            io::ErrorKind::NotFound => FileIoErrorCategory::NotFound,
            _ => FileIoErrorCategory::Other,
        },
    }
}

pub use config_store::{
    app_config_file_path, load_app_config, load_app_config_from_path, save_app_config,
    save_app_config_to_path, AppConfigLoadError, AppConfigSaveError,
};
pub(crate) use folder_scanner::scan_image_folder_for_file_with_cancellation;
pub use folder_scanner::{
    scan_image_folder_for_file, scan_image_folder_for_file_or_empty, ScanImageFolderError,
};

mod config_store {
    use std::env;
    use std::error::Error;
    use std::fmt;
    use std::fs::{self, File, OpenOptions};
    use std::io;
    use std::io::{BufWriter, Write};
    use std::path::{Path, PathBuf};

    use crate::domain::{parse_app_config, serialize_app_config, AppConfig, AppConfigParseError};

    const APP_CONFIG_DIR_NAME: &str = "j3Pic";
    const APP_CONFIG_FILE_NAME: &str = "config.txt";
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
        NoConfigDirectory,
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
                Self::NoConfigDirectory => {
                    formatter.write_str("app config directory is unavailable")
                }
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
                Self::NoConfigDirectory | Self::NoConfigFileName { .. } => None,
            }
        }
    }

    pub fn app_config_file_path() -> Option<PathBuf> {
        app_config_base_dir().map(|base_dir| {
            base_dir
                .join(APP_CONFIG_DIR_NAME)
                .join(APP_CONFIG_FILE_NAME)
        })
    }

    pub fn load_app_config() -> Result<AppConfig, AppConfigLoadError> {
        let Some(path) = app_config_file_path() else {
            return Ok(AppConfig::default());
        };

        load_app_config_from_path(&path)
    }

    pub fn load_app_config_from_path(
        path: impl AsRef<Path>,
    ) -> Result<AppConfig, AppConfigLoadError> {
        let path = path.as_ref();
        match fs::read_to_string(path) {
            Ok(contents) => {
                parse_app_config(&contents).map_err(|source| AppConfigLoadError::Parse {
                    path: path.to_path_buf(),
                    source,
                })
            }
            Err(source) if source.kind() == io::ErrorKind::NotFound => Ok(AppConfig::default()),
            Err(source) => Err(AppConfigLoadError::FileRead {
                path: path.to_path_buf(),
                source,
            }),
        }
    }

    pub fn save_app_config(config: &AppConfig) -> Result<(), AppConfigSaveError> {
        let path = app_config_file_path().ok_or(AppConfigSaveError::NoConfigDirectory)?;
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

    #[cfg(target_os = "windows")]
    fn app_config_base_dir() -> Option<PathBuf> {
        app_config_env_dir("APPDATA").or_else(|| app_config_env_dir("LOCALAPPDATA"))
    }

    #[cfg(target_os = "linux")]
    fn app_config_base_dir() -> Option<PathBuf> {
        app_config_env_dir("XDG_CONFIG_HOME")
            .or_else(|| app_config_env_dir("HOME").map(|home| home.join(".config")))
    }

    #[cfg(not(any(target_os = "windows", target_os = "linux")))]
    fn app_config_base_dir() -> Option<PathBuf> {
        None
    }

    fn app_config_env_dir(name: &str) -> Option<PathBuf> {
        let value = env::var_os(name)?;
        if value.as_os_str().is_empty() {
            None
        } else {
            Some(PathBuf::from(value))
        }
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
}

mod folder_scanner {
    use std::collections::VecDeque;
    use std::error::Error;
    use std::fmt;
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Mutex, OnceLock};
    use std::time::SystemTime;

    use crate::domain::{
        is_supported_image_path, ImageFolder, UiLanguage, MAX_IMAGE_FOLDER_SNAPSHOT_PATHS,
    };

    use super::{classify_file_io_error, FileIoErrorCategory};

    const IMAGE_FOLDER_SCAN_CACHE_CAPACITY: usize = 4;

    static IMAGE_FOLDER_SCAN_CACHE: OnceLock<Mutex<ImageFolderScanCache>> = OnceLock::new();

    #[derive(Clone, PartialEq, Eq)]
    struct ImageFolderScanCacheKey {
        directory: PathBuf,
        modified: SystemTime,
    }

    impl ImageFolderScanCacheKey {
        fn new(directory: &Path) -> Option<Self> {
            Some(Self {
                directory: directory.to_path_buf(),
                modified: fs::metadata(directory).ok()?.modified().ok()?,
            })
        }
    }

    struct ImageFolderScanCacheEntry {
        key: ImageFolderScanCacheKey,
        folder: ImageFolder,
    }

    struct ImageFolderScanCache {
        entries: VecDeque<ImageFolderScanCacheEntry>,
    }

    impl ImageFolderScanCache {
        fn new() -> Self {
            Self {
                entries: VecDeque::with_capacity(IMAGE_FOLDER_SCAN_CACHE_CAPACITY),
            }
        }

        fn get(&mut self, key: &ImageFolderScanCacheKey) -> Option<ImageFolder> {
            let Some(index) = self.entries.iter().position(|entry| entry.key == *key) else {
                self.remove_directory(&key.directory);
                return None;
            };

            let entry = self.entries.remove(index)?;
            let folder = entry.folder.clone();
            self.entries.push_back(entry);
            Some(folder)
        }

        fn insert(&mut self, key: ImageFolderScanCacheKey, folder: &ImageFolder) {
            self.remove_directory(&key.directory);
            self.entries.push_back(ImageFolderScanCacheEntry {
                key,
                folder: folder.clone(),
            });
            while self.entries.len() > IMAGE_FOLDER_SCAN_CACHE_CAPACITY {
                self.entries.pop_front();
            }
        }

        fn remove(&mut self, key: &ImageFolderScanCacheKey) {
            self.entries.retain(|entry| entry.key != *key);
        }

        fn remove_directory(&mut self, directory: &Path) {
            self.entries
                .retain(|entry| entry.key.directory != directory);
        }

        #[cfg(test)]
        fn clear(&mut self) {
            self.entries.clear();
        }
    }

    #[derive(Debug)]
    pub enum ScanImageFolderError {
        NoParent {
            path: PathBuf,
        },
        DirectoryAccess {
            path: PathBuf,
            source: io::Error,
        },
        EntryAccess {
            directory: PathBuf,
            source: io::Error,
        },
        FileTypeAccess {
            path: PathBuf,
            source: io::Error,
        },
    }

    impl ScanImageFolderError {
        pub fn user_message(&self) -> String {
            match self {
                Self::NoParent { path } => format!(
                    "이미지의 부모 폴더를 찾을 수 없습니다.\n\n파일: {}",
                    path.display()
                ),
                Self::DirectoryAccess { path, .. } => format!(
                    "이미지 폴더를 읽을 수 없습니다.\n\n폴더: {}",
                    path.display()
                ),
                Self::EntryAccess { directory, .. } => format!(
                    "이미지 폴더의 항목을 읽을 수 없습니다.\n\n폴더: {}",
                    directory.display()
                ),
                Self::FileTypeAccess { path, .. } => format!(
                    "이미지 폴더 항목의 파일 정보를 읽을 수 없습니다.\n\n경로: {}",
                    path.display()
                ),
            }
        }

        pub fn user_message_for(&self, language: UiLanguage) -> String {
            if language == UiLanguage::Korean {
                return self.user_message();
            }
            match self {
                Self::NoParent { path } => format!(
                    "Could not find the image's parent folder.\n\nFile: {}",
                    path.display()
                ),
                Self::DirectoryAccess { path, .. } => format!(
                    "Could not read the image folder.\n\nFolder: {}",
                    path.display()
                ),
                Self::EntryAccess { directory, .. } => format!(
                    "Could not read an item in the image folder.\n\nFolder: {}",
                    directory.display()
                ),
                Self::FileTypeAccess { path, .. } => format!(
                    "Could not read file information for an image folder item.\n\nPath: {}",
                    path.display()
                ),
            }
        }

        pub fn brief_user_message(&self) -> &'static str {
            match self {
                Self::NoParent { .. } => "이미지의 부모 폴더를 찾을 수 없습니다.",
                Self::DirectoryAccess { source, .. }
                | Self::EntryAccess { source, .. }
                | Self::FileTypeAccess { source, .. } => match classify_file_io_error(source) {
                    FileIoErrorCategory::PermissionDenied => "이미지 폴더를 읽을 권한이 없습니다.",
                    FileIoErrorCategory::NotFound => "이미지 폴더를 찾을 수 없습니다.",
                    FileIoErrorCategory::FileLocked => "이미지 폴더 항목이 사용 중입니다.",
                    FileIoErrorCategory::Other => "이미지 폴더를 읽는 중 I/O 오류가 발생했습니다.",
                },
            }
        }

        pub fn brief_user_message_for(&self, language: UiLanguage) -> &'static str {
            if language == UiLanguage::Korean {
                return self.brief_user_message();
            }
            match self {
                Self::NoParent { .. } => "Could not find the image's parent folder.",
                Self::DirectoryAccess { source, .. }
                | Self::EntryAccess { source, .. }
                | Self::FileTypeAccess { source, .. } => match classify_file_io_error(source) {
                    FileIoErrorCategory::PermissionDenied => {
                        "You do not have permission to read the image folder."
                    }
                    FileIoErrorCategory::NotFound => "Could not find the image folder.",
                    FileIoErrorCategory::FileLocked => "An image folder item is in use.",
                    FileIoErrorCategory::Other => {
                        "An I/O error occurred while reading the image folder."
                    }
                },
            }
        }
    }

    impl fmt::Display for ScanImageFolderError {
        fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
            match self {
                Self::NoParent { path } => {
                    write!(formatter, "image path has no parent: {}", path.display())
                }
                Self::DirectoryAccess { path, .. } => {
                    write!(
                        formatter,
                        "failed to read image directory: {}",
                        path.display()
                    )
                }
                Self::EntryAccess { directory, .. } => {
                    write!(
                        formatter,
                        "failed to read image directory entry: {}",
                        directory.display()
                    )
                }
                Self::FileTypeAccess { path, .. } => {
                    write!(formatter, "failed to read file type: {}", path.display())
                }
            }
        }
    }

    impl Error for ScanImageFolderError {
        fn source(&self) -> Option<&(dyn Error + 'static)> {
            match self {
                Self::DirectoryAccess { source, .. }
                | Self::EntryAccess { source, .. }
                | Self::FileTypeAccess { source, .. } => Some(source),
                Self::NoParent { .. } => None,
            }
        }
    }

    pub fn scan_image_folder_for_file(
        path: impl AsRef<Path>,
    ) -> Result<ImageFolder, ScanImageFolderError> {
        let path = path.as_ref();
        match scan_image_folder_for_file_impl(path, || false)? {
            Some(folder) => Ok(folder),
            None => Ok(ImageFolder::empty()),
        }
    }

    pub(crate) fn scan_image_folder_for_file_with_cancellation(
        path: impl AsRef<Path>,
        cancel: &AtomicBool,
    ) -> Option<Result<ImageFolder, ScanImageFolderError>> {
        match scan_image_folder_for_file_impl(path.as_ref(), || cancel.load(Ordering::Acquire)) {
            Ok(Some(folder)) => Some(Ok(folder)),
            Ok(None) => None,
            Err(error) => Some(Err(error)),
        }
    }

    fn scan_image_folder_for_file_impl(
        path: &Path,
        is_canceled: impl Fn() -> bool,
    ) -> Result<Option<ImageFolder>, ScanImageFolderError> {
        if is_canceled() {
            return Ok(None);
        }
        let parent = parent_directory_for_scan(path)?;
        let cache_key = ImageFolderScanCacheKey::new(&parent);
        if is_canceled() {
            return Ok(None);
        }
        if let Some(folder) = cached_image_folder_scan(cache_key.as_ref(), path) {
            return Ok(Some(folder));
        }

        let entries =
            fs::read_dir(&parent).map_err(|source| ScanImageFolderError::DirectoryAccess {
                path: parent.clone(),
                source,
            })?;
        let mut paths = Vec::new();
        let current_file_name = path.file_name().map(|name| name.to_owned());
        if is_supported_image_path(path) {
            paths.push(path.to_path_buf());
        }

        for entry in entries {
            if is_canceled() {
                return Ok(None);
            }
            if paths.len() >= MAX_IMAGE_FOLDER_SNAPSHOT_PATHS {
                break;
            }
            let entry = entry.map_err(|source| ScanImageFolderError::EntryAccess {
                directory: parent.clone(),
                source,
            })?;
            let file_name = entry.file_name();
            if !is_supported_image_path(Path::new(&file_name)) {
                continue;
            }
            if current_file_name.as_deref() == Some(file_name.as_os_str()) {
                continue;
            }
            let entry_path = entry.path();
            let file_type =
                entry
                    .file_type()
                    .map_err(|source| ScanImageFolderError::FileTypeAccess {
                        path: entry_path.clone(),
                        source,
                    })?;

            if file_type.is_file() {
                paths.push(entry_path);
            }
        }

        if is_canceled() {
            return Ok(None);
        }
        let folder = ImageFolder::from_supported_paths(path, paths);
        store_image_folder_scan(cache_key, &folder);
        Ok(Some(folder))
    }

    pub fn scan_image_folder_for_file_or_empty(
        path: impl AsRef<Path>,
    ) -> (ImageFolder, Option<ScanImageFolderError>) {
        match scan_image_folder_for_file(path) {
            Ok(folder) => (folder, None),
            Err(error) => (ImageFolder::empty(), Some(error)),
        }
    }

    fn parent_directory_for_scan(path: &Path) -> Result<PathBuf, ScanImageFolderError> {
        let parent = path
            .parent()
            .ok_or_else(|| ScanImageFolderError::NoParent {
                path: path.to_path_buf(),
            })?;

        if parent.as_os_str().is_empty() {
            Ok(PathBuf::from("."))
        } else {
            Ok(parent.to_path_buf())
        }
    }

    fn cached_image_folder_scan(
        cache_key: Option<&ImageFolderScanCacheKey>,
        current_path: &Path,
    ) -> Option<ImageFolder> {
        let cache_key = cache_key?;
        let mut cache = lock_image_folder_scan_cache();
        let cached = cache.get(cache_key)?;

        match cached.retargeted_current_path(current_path) {
            Some(folder) => Some(folder),
            None => {
                cache.remove(cache_key);
                None
            }
        }
    }

    fn store_image_folder_scan(cache_key: Option<ImageFolderScanCacheKey>, folder: &ImageFolder) {
        let Some(cache_key) = cache_key else {
            return;
        };
        let mut cache = lock_image_folder_scan_cache();
        cache.insert(cache_key, folder);
    }

    #[cfg(test)]
    fn clear_image_folder_scan_cache() {
        let mut cache = lock_image_folder_scan_cache();
        cache.clear();
    }

    fn lock_image_folder_scan_cache() -> std::sync::MutexGuard<'static, ImageFolderScanCache> {
        let cache = IMAGE_FOLDER_SCAN_CACHE.get_or_init(|| Mutex::new(ImageFolderScanCache::new()));
        match cache.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    #[cfg(test)]
    mod tests {
        use std::time::{Duration, UNIX_EPOCH};

        use super::*;

        static TEST_CACHE_LOCK: std::sync::OnceLock<std::sync::Mutex<()>> =
            std::sync::OnceLock::new();

        fn lock_test_cache() -> std::sync::MutexGuard<'static, ()> {
            let lock = TEST_CACHE_LOCK.get_or_init(|| std::sync::Mutex::new(()));
            match lock.lock() {
                Ok(guard) => guard,
                Err(poisoned) => poisoned.into_inner(),
            }
        }

        #[test]
        fn cached_folder_scan_retargets_current_path_without_rebuilding_snapshot() {
            let _cache_guard = lock_test_cache();
            clear_image_folder_scan_cache();
            let key = ImageFolderScanCacheKey {
                directory: PathBuf::from("C:/images"),
                modified: UNIX_EPOCH,
            };
            let folder = ImageFolder::from_paths(
                "C:/images/a.png",
                ["C:/images/a.png", "C:/images/b.png"]
                    .into_iter()
                    .map(PathBuf::from),
            );
            store_image_folder_scan(Some(key.clone()), &folder);

            let cached = cached_image_folder_scan(Some(&key), Path::new("C:/images/b.png"))
                .expect("matching cache key should hit");

            assert_eq!(cached.paths(), folder.paths());
            assert_eq!(cached.current_index(), Some(1));
            clear_image_folder_scan_cache();
        }

        #[test]
        fn cached_folder_scan_invalidates_when_directory_modified_time_changes() {
            let _cache_guard = lock_test_cache();
            clear_image_folder_scan_cache();
            let old_key = ImageFolderScanCacheKey {
                directory: PathBuf::from("C:/images"),
                modified: UNIX_EPOCH,
            };
            let new_key = ImageFolderScanCacheKey {
                directory: PathBuf::from("C:/images"),
                modified: UNIX_EPOCH + Duration::from_secs(1),
            };
            let folder = ImageFolder::from_paths(
                "C:/images/a.png",
                ["C:/images/a.png", "C:/images/b.png"]
                    .into_iter()
                    .map(PathBuf::from),
            );
            store_image_folder_scan(Some(old_key.clone()), &folder);

            assert!(
                cached_image_folder_scan(Some(&new_key), Path::new("C:/images/a.png")).is_none()
            );
            assert!(
                cached_image_folder_scan(Some(&old_key), Path::new("C:/images/a.png")).is_none()
            );
            clear_image_folder_scan_cache();
        }

        #[test]
        fn cached_folder_scan_keeps_recent_folders_when_switching_directories() {
            let _cache_guard = lock_test_cache();
            clear_image_folder_scan_cache();
            let first_key = ImageFolderScanCacheKey {
                directory: PathBuf::from("C:/images/first"),
                modified: UNIX_EPOCH,
            };
            let second_key = ImageFolderScanCacheKey {
                directory: PathBuf::from("C:/images/second"),
                modified: UNIX_EPOCH,
            };
            let first_folder = ImageFolder::from_paths(
                "C:/images/first/a.png",
                ["C:/images/first/a.png", "C:/images/first/b.png"]
                    .into_iter()
                    .map(PathBuf::from),
            );
            let second_folder = ImageFolder::from_paths(
                "C:/images/second/a.png",
                ["C:/images/second/a.png", "C:/images/second/b.png"]
                    .into_iter()
                    .map(PathBuf::from),
            );

            store_image_folder_scan(Some(first_key.clone()), &first_folder);
            store_image_folder_scan(Some(second_key), &second_folder);

            let cached =
                cached_image_folder_scan(Some(&first_key), Path::new("C:/images/first/b.png"))
                    .expect("recent folder should stay cached after switching directories");

            assert_eq!(cached.paths(), first_folder.paths());
            assert_eq!(cached.current_index(), Some(1));
            clear_image_folder_scan_cache();
        }

        #[test]
        fn cached_folder_scan_evicts_oldest_folder_when_capacity_is_exceeded() {
            let _cache_guard = lock_test_cache();
            clear_image_folder_scan_cache();
            let mut keys = Vec::new();
            for index in 0..=IMAGE_FOLDER_SCAN_CACHE_CAPACITY {
                let directory = PathBuf::from(format!("C:/images/{index}"));
                let key = ImageFolderScanCacheKey {
                    directory: directory.clone(),
                    modified: UNIX_EPOCH,
                };
                let path = directory.join("a.png");
                let folder = ImageFolder::from_paths(&path, [path.clone()]);
                store_image_folder_scan(Some(key.clone()), &folder);
                keys.push((key, path));
            }

            assert!(cached_image_folder_scan(Some(&keys[0].0), &keys[0].1).is_none());
            assert!(cached_image_folder_scan(Some(&keys[1].0), &keys[1].1).is_some());
            clear_image_folder_scan_cache();
        }
    }
}

#[cfg(target_os = "windows")]
fn replace_file(temporary_path: &Path, path: &Path) -> io::Result<()> {
    replace_file_with_flags(temporary_path, path, replace_file_flags())
}

#[cfg(target_os = "windows")]
fn replace_export_file(temporary_path: &Path, path: &Path) -> io::Result<()> {
    replace_file_with_flags(temporary_path, path, export_replace_file_flags())
}

#[cfg(target_os = "windows")]
fn replace_file_with_flags(temporary_path: &Path, path: &Path, flags: u32) -> io::Result<()> {
    let temporary_path = wide_path(temporary_path);
    let path = wide_path(path);
    // SAFETY: Both buffers are null-terminated UTF-16 paths valid for the duration of the call.
    let moved = unsafe { MoveFileExW(temporary_path.as_ptr(), path.as_ptr(), flags) } != 0;
    if moved {
        Ok(())
    } else {
        Err(io::Error::last_os_error())
    }
}

#[cfg(target_os = "windows")]
fn replace_file_flags() -> u32 {
    MOVEFILE_REPLACE_EXISTING | MOVEFILE_WRITE_THROUGH
}

#[cfg(target_os = "windows")]
fn export_replace_file_flags() -> u32 {
    // Export syncs the final temporary file before this rename; keep atomic replace without
    // adding another write-through operation to the single export worker.
    MOVEFILE_REPLACE_EXISTING
}

#[cfg(not(target_os = "windows"))]
fn replace_file(temporary_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temporary_path, path)?;
    sync_parent_directory(path)
}

#[cfg(not(target_os = "windows"))]
fn replace_export_file(temporary_path: &Path, path: &Path) -> io::Result<()> {
    fs::rename(temporary_path, path)?;
    sync_parent_directory(path)
}

#[cfg(target_os = "linux")]
fn sync_parent_directory(path: &Path) -> io::Result<()> {
    let parent = path
        .parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .unwrap_or_else(|| Path::new("."));
    File::open(parent)?.sync_all()
}

#[cfg(all(not(target_os = "windows"), not(target_os = "linux")))]
fn sync_parent_directory(_path: &Path) -> io::Result<()> {
    Ok(())
}

#[cfg(target_os = "windows")]
fn wide_path(path: &Path) -> Vec<u16> {
    path.as_os_str()
        .encode_wide()
        .chain(std::iter::once(0))
        .collect()
}

#[derive(Debug)]
pub enum LoadImageError {
    UnsupportedFormat {
        path: PathBuf,
    },
    FileAccess {
        path: PathBuf,
        source: io::Error,
    },
    NotAFile {
        path: PathBuf,
    },
    FileChanged {
        path: PathBuf,
    },
    DecodeFailed {
        path: PathBuf,
        source: image::ImageError,
    },
    ImageTooLarge {
        path: PathBuf,
        size: ImageSize,
        source: Option<image::ImageError>,
    },
    OutOfMemory {
        path: PathBuf,
        source: Option<image::ImageError>,
    },
    DecodeCanceled {
        path: PathBuf,
    },
    InvalidPixelBuffer {
        path: PathBuf,
    },
    AnimationTooManyFrames {
        path: PathBuf,
        frame_count: usize,
        max_frame_count: usize,
    },
    AnimationFrameUnavailable {
        path: PathBuf,
        frame_index: usize,
    },
}

impl LoadImageError {
    pub fn user_message(&self) -> String {
        match self {
            Self::UnsupportedFormat { path } => format!(
                "지원하지 않는 이미지 형식입니다.\n\n파일: {}\n지원 형식: {}",
                path.display(),
                SUPPORTED_FORMATS_TEXT
            ),
            Self::FileAccess { path, .. } => match self.category() {
                ImageOpenErrorCategory::PermissionDenied => format!(
                    "이미지 파일을 열 권한이 없습니다.\n\n파일: {}",
                    path.display()
                ),
                ImageOpenErrorCategory::FileNotFoundOrMoved => format!(
                    "이미지 파일을 찾을 수 없습니다. 이동되었거나 삭제되었을 수 있습니다.\n\n파일: {}",
                    path.display()
                ),
                ImageOpenErrorCategory::FileLocked => format!(
                    "이미지 파일이 다른 프로그램에서 사용 중입니다.\n\n파일: {}",
                    path.display()
                ),
                _ => format!(
                    "이미지 파일을 읽는 중 알 수 없는 I/O 오류가 발생했습니다.\n\n파일: {}",
                    path.display()
                ),
            },
            Self::NotAFile { path } => format!(
                "선택한 경로는 이미지 파일이 아닙니다.\n\n경로: {}",
                path.display()
            ),
            Self::FileChanged { path } => format!(
                "이미지 파일이 로딩 중 변경되었습니다. 다시 열어 주세요.\n\n파일: {}",
                path.display()
            ),
            Self::DecodeFailed { path, .. } => match self.category() {
                ImageOpenErrorCategory::UnsupportedFormat => format!(
                    "지원하지 않는 이미지 형식입니다.\n\n파일: {}\n지원 형식: {}",
                    path.display(),
                    SUPPORTED_FORMATS_TEXT
                ),
                _ => format!(
                    "이미지가 손상되었거나 디코딩할 수 없습니다.\n\n파일: {}",
                    path.display()
                ),
            },
            Self::ImageTooLarge { path, size, .. } => format!(
                "이미지가 너무 커서 현재 메모리 한도 안에서 처리할 수 없습니다.\n\n파일: {}\n크기: {}x{}",
                path.display(),
                size.width(),
                size.height()
            ),
            Self::OutOfMemory { path, .. } => format!(
                "이미지를 디코딩할 메모리를 확보하지 못했습니다. 더 작은 이미지나 축소된 이미지를 사용해 주세요.\n\n파일: {}",
                path.display()
            ),
            Self::DecodeCanceled { path } => format!(
                "이미지 디코딩이 취소되었습니다.\n\n파일: {}",
                path.display()
            ),
            Self::InvalidPixelBuffer { path } => format!(
                "디코딩된 이미지 픽셀 데이터가 올바르지 않습니다.\n\n파일: {}",
                path.display()
            ),
            Self::AnimationTooManyFrames {
                path,
                frame_count,
                max_frame_count,
            } => format!(
                "애니메이션 프레임 수가 현재 메모리 정책 한도를 초과합니다.\n\n파일: {}\n프레임 수: {}\n한도: {}",
                path.display(),
                frame_count,
                max_frame_count
            ),
            Self::AnimationFrameUnavailable { path, frame_index } => format!(
                "애니메이션 프레임을 읽지 못했습니다.\n\n파일: {}\n프레임: {}",
                path.display(),
                frame_index + 1
            ),
        }
    }

    pub fn user_message_for(&self, language: UiLanguage) -> String {
        if language == UiLanguage::Korean {
            return self.user_message();
        }
        match self {
            Self::UnsupportedFormat { path } => format!(
                "Unsupported image format.\n\nFile: {}\nSupported formats: {}",
                path.display(),
                SUPPORTED_FORMATS_TEXT
            ),
            Self::FileAccess { path, .. } => match self.category() {
                ImageOpenErrorCategory::PermissionDenied => format!(
                    "You do not have permission to open the image file.\n\nFile: {}",
                    path.display()
                ),
                ImageOpenErrorCategory::FileNotFoundOrMoved => format!(
                    "Could not find the image file. It may have been moved or deleted.\n\nFile: {}",
                    path.display()
                ),
                ImageOpenErrorCategory::FileLocked => format!(
                    "The image file is in use by another program.\n\nFile: {}",
                    path.display()
                ),
                _ => format!(
                    "An unknown I/O error occurred while reading the image file.\n\nFile: {}",
                    path.display()
                ),
            },
            Self::NotAFile { path } => format!(
                "The selected path is not an image file.\n\nPath: {}",
                path.display()
            ),
            Self::FileChanged { path } => format!(
                "The image file changed while loading. Open it again.\n\nFile: {}",
                path.display()
            ),
            Self::DecodeFailed { path, .. } => match self.category() {
                ImageOpenErrorCategory::UnsupportedFormat => format!(
                    "Unsupported image format.\n\nFile: {}\nSupported formats: {}",
                    path.display(),
                    SUPPORTED_FORMATS_TEXT
                ),
                _ => format!(
                    "The image is damaged or cannot be decoded.\n\nFile: {}",
                    path.display()
                ),
            },
            Self::ImageTooLarge { path, size, .. } => format!(
                "The image is too large to process within the current memory limits.\n\nFile: {}\nSize: {}x{}",
                path.display(),
                size.width(),
                size.height()
            ),
            Self::OutOfMemory { path, .. } => format!(
                "Could not allocate memory to decode the image. Use a smaller or downscaled image.\n\nFile: {}",
                path.display()
            ),
            Self::DecodeCanceled { path } => format!(
                "Image decoding was canceled.\n\nFile: {}",
                path.display()
            ),
            Self::InvalidPixelBuffer { path } => format!(
                "Decoded image pixel data is invalid.\n\nFile: {}",
                path.display()
            ),
            Self::AnimationTooManyFrames {
                path,
                frame_count,
                max_frame_count,
            } => format!(
                "The animation frame count exceeds the current memory policy limit.\n\nFile: {}\nFrame count: {}\nLimit: {}",
                path.display(),
                frame_count,
                max_frame_count
            ),
            Self::AnimationFrameUnavailable { path, frame_index } => format!(
                "Could not read the animation frame.\n\nFile: {}\nFrame: {}",
                path.display(),
                frame_index + 1
            ),
        }
    }

    pub fn is_canceled(&self) -> bool {
        matches!(self, Self::DecodeCanceled { .. })
    }

    pub fn category(&self) -> ImageOpenErrorCategory {
        match self {
            Self::UnsupportedFormat { .. } => ImageOpenErrorCategory::UnsupportedFormat,
            Self::FileAccess { source, .. } => image_open_category_from_io_error(source),
            Self::NotAFile { .. } => ImageOpenErrorCategory::NotAFile,
            Self::FileChanged { .. } => ImageOpenErrorCategory::FileNotFoundOrMoved,
            Self::DecodeFailed { source, .. } if decode_error_is_unsupported(source) => {
                ImageOpenErrorCategory::UnsupportedFormat
            }
            Self::DecodeFailed { .. }
            | Self::InvalidPixelBuffer { .. }
            | Self::AnimationFrameUnavailable { .. } => {
                ImageOpenErrorCategory::CorruptOrDecodingFailed
            }
            Self::ImageTooLarge { .. }
            | Self::OutOfMemory { .. }
            | Self::AnimationTooManyFrames { .. } => {
                ImageOpenErrorCategory::ImageTooLargeOrOutOfMemory
            }
            Self::DecodeCanceled { .. } => ImageOpenErrorCategory::Canceled,
        }
    }

    pub fn failure_stage(&self) -> ImageLoadFailureStage {
        match self {
            Self::UnsupportedFormat { .. } => ImageLoadFailureStage::FormatDetection,
            Self::FileAccess { .. } | Self::NotAFile { .. } => ImageLoadFailureStage::FileIo,
            Self::FileChanged { .. } => ImageLoadFailureStage::FileIo,
            Self::DecodeFailed { .. }
            | Self::ImageTooLarge { .. }
            | Self::OutOfMemory { .. }
            | Self::DecodeCanceled { .. }
            | Self::AnimationTooManyFrames { .. }
            | Self::AnimationFrameUnavailable { .. } => ImageLoadFailureStage::Decoder,
            Self::InvalidPixelBuffer { .. } => ImageLoadFailureStage::PixelConversion,
        }
    }

    pub fn brief_user_message(&self) -> &'static str {
        if matches!(self, Self::FileChanged { .. }) {
            return "이미지 파일이 로딩 중 변경되었습니다.";
        }
        match self.category() {
            ImageOpenErrorCategory::UnsupportedFormat => "지원하지 않는 이미지 형식입니다.",
            ImageOpenErrorCategory::CorruptOrDecodingFailed => {
                "손상되었거나 디코딩할 수 없는 이미지입니다."
            }
            ImageOpenErrorCategory::PermissionDenied => "이미지 파일을 열 권한이 없습니다.",
            ImageOpenErrorCategory::FileNotFoundOrMoved => "이미지 파일을 찾을 수 없습니다.",
            ImageOpenErrorCategory::FileLocked => "이미지 파일이 사용 중입니다.",
            ImageOpenErrorCategory::ImageTooLargeOrOutOfMemory => {
                "이미지가 너무 크거나 메모리가 부족합니다."
            }
            ImageOpenErrorCategory::UnknownIo => "이미지 파일을 읽는 중 I/O 오류가 발생했습니다.",
            ImageOpenErrorCategory::NotAFile => "선택한 경로는 이미지 파일이 아닙니다.",
            ImageOpenErrorCategory::Canceled => "이미지 디코딩이 취소되었습니다.",
        }
    }

    pub fn brief_user_message_for(&self, language: UiLanguage) -> &'static str {
        if language == UiLanguage::Korean {
            return self.brief_user_message();
        }
        if matches!(self, Self::FileChanged { .. }) {
            return "The image file changed while loading.";
        }
        match self.category() {
            ImageOpenErrorCategory::UnsupportedFormat => "Unsupported image format.",
            ImageOpenErrorCategory::CorruptOrDecodingFailed => {
                "The image is damaged or cannot be decoded."
            }
            ImageOpenErrorCategory::PermissionDenied => {
                "You do not have permission to open the image file."
            }
            ImageOpenErrorCategory::FileNotFoundOrMoved => "Could not find the image file.",
            ImageOpenErrorCategory::FileLocked => "The image file is in use.",
            ImageOpenErrorCategory::ImageTooLargeOrOutOfMemory => {
                "The image is too large or memory is insufficient."
            }
            ImageOpenErrorCategory::UnknownIo => {
                "An I/O error occurred while reading the image file."
            }
            ImageOpenErrorCategory::NotAFile => "The selected path is not an image file.",
            ImageOpenErrorCategory::Canceled => "Image decoding was canceled.",
        }
    }
}

impl fmt::Display for LoadImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::UnsupportedFormat { path } => {
                write!(formatter, "unsupported image format: {}", path.display())
            }
            Self::FileAccess { path, .. } => {
                write!(formatter, "failed to access image file: {}", path.display())
            }
            Self::NotAFile { path } => write!(formatter, "path is not a file: {}", path.display()),
            Self::FileChanged { path } => {
                write!(formatter, "image file changed while loading: {}", path.display())
            }
            Self::DecodeFailed { path, .. } => {
                write!(formatter, "failed to decode image: {}", path.display())
            }
            Self::ImageTooLarge { path, size, .. } => write!(
                formatter,
                "image exceeds memory policy: {} ({}x{})",
                path.display(),
                size.width(),
                size.height()
            ),
            Self::OutOfMemory { path, .. } => {
                write!(
                    formatter,
                    "insufficient memory while decoding: {}",
                    path.display()
                )
            }
            Self::DecodeCanceled { path } => {
                write!(formatter, "image decode canceled: {}", path.display())
            }
            Self::InvalidPixelBuffer { path } => {
                write!(
                    formatter,
                    "invalid decoded pixel buffer: {}",
                    path.display()
                )
            }
            Self::AnimationTooManyFrames {
                path,
                frame_count,
                max_frame_count,
            } => write!(
                formatter,
                "animation frame count exceeds memory policy: {} ({frame_count} > {max_frame_count})",
                path.display()
            ),
            Self::AnimationFrameUnavailable { path, frame_index } => write!(
                formatter,
                "animation frame unavailable: {} (frame {frame_index})",
                path.display()
            ),
        }
    }
}

impl Error for LoadImageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FileAccess { source, .. } => Some(source),
            Self::DecodeFailed { source, .. } => Some(source),
            Self::ImageTooLarge {
                source: Some(source),
                ..
            }
            | Self::OutOfMemory {
                source: Some(source),
                ..
            } => Some(source),
            Self::UnsupportedFormat { .. }
            | Self::NotAFile { .. }
            | Self::FileChanged { .. }
            | Self::ImageTooLarge { source: None, .. }
            | Self::OutOfMemory { source: None, .. }
            | Self::DecodeCanceled { .. }
            | Self::InvalidPixelBuffer { .. }
            | Self::AnimationTooManyFrames { .. }
            | Self::AnimationFrameUnavailable { .. } => None,
        }
    }
}

#[derive(Debug)]
pub enum ExportImageError {
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
    EncodeFailed {
        path: PathBuf,
        format: ExportFormat,
        source: image::ImageError,
    },
    PngOptimizeFailed {
        path: PathBuf,
        source: oxipng::PngError,
    },
    InvalidPixelBuffer {
        path: PathBuf,
        size: ImageSize,
        actual_len: usize,
    },
    AllocationFailed {
        path: PathBuf,
    },
}

impl ExportImageError {
    pub fn user_message(&self) -> String {
        match self {
            Self::FileCreate { path, .. }
            | Self::FileWrite { path, .. }
            | Self::FileReplace { path, .. } => export_file_access_message(self.category(), path),
            Self::EncodeFailed { path, format, .. } => match self.category() {
                ExportSaveErrorCategory::PermissionDenied
                | ExportSaveErrorCategory::PathNotFound
                | ExportSaveErrorCategory::FileLocked
                | ExportSaveErrorCategory::UnknownIo => {
                    export_file_access_message(self.category(), path)
                }
                ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => format!(
                    "내보내기 중 메모리를 확보하지 못했습니다.\n\n파일: {}",
                    path.display()
                ),
                _ => format!(
                    "{} 형식으로 이미지를 인코딩하지 못했습니다.\n\n파일: {}",
                    crate::domain::export_format_display_name(*format),
                    path.display()
                ),
            },
            Self::PngOptimizeFailed { path, .. } => format!(
                "PNG 형식으로 이미지를 최적화하지 못했습니다.\n\n파일: {}",
                path.display()
            ),
            Self::InvalidPixelBuffer {
                path,
                size,
                actual_len,
            } => format!(
                "내보낼 이미지 픽셀 데이터가 올바르지 않습니다.\n\n파일: {}\n크기: {}x{}\n바이트 수: {}",
                path.display(),
                size.width(),
                size.height(),
                actual_len
            ),
            Self::AllocationFailed { path } => format!(
                "내보내기용 픽셀 버퍼를 만들 메모리를 확보하지 못했습니다.\n\n파일: {}",
                path.display()
            ),
        }
    }

    pub fn user_message_for(&self, language: UiLanguage) -> String {
        if language == UiLanguage::Korean {
            return self.user_message();
        }
        match self {
            Self::FileCreate { path, .. }
            | Self::FileWrite { path, .. }
            | Self::FileReplace { path, .. } => {
                export_file_access_message_for(self.category(), path, language)
            }
            Self::EncodeFailed { path, format, .. } => match self.category() {
                ExportSaveErrorCategory::PermissionDenied
                | ExportSaveErrorCategory::PathNotFound
                | ExportSaveErrorCategory::FileLocked
                | ExportSaveErrorCategory::UnknownIo => {
                    export_file_access_message_for(self.category(), path, language)
                }
                ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => format!(
                    "Could not allocate memory during export.\n\nFile: {}",
                    path.display()
                ),
                _ => format!(
                    "Could not encode the image as {}.\n\nFile: {}",
                    crate::domain::export_format_display_name(*format),
                    path.display()
                ),
            },
            Self::PngOptimizeFailed { path, .. } => format!(
                "Could not optimize the image as PNG.\n\nFile: {}",
                path.display()
            ),
            Self::InvalidPixelBuffer {
                path,
                size,
                actual_len,
            } => format!(
                "Export image pixel data is invalid.\n\nFile: {}\nSize: {}x{}\nByte count: {}",
                path.display(),
                size.width(),
                size.height(),
                actual_len
            ),
            Self::AllocationFailed { path } => format!(
                "Could not allocate an export pixel buffer.\n\nFile: {}",
                path.display()
            ),
        }
    }

    pub fn category(&self) -> ExportSaveErrorCategory {
        match self {
            Self::FileCreate { source, .. }
            | Self::FileWrite { source, .. }
            | Self::FileReplace { source, .. } => export_category_from_io_error(source),
            Self::EncodeFailed { source, .. } => export_category_from_image_error(source),
            Self::PngOptimizeFailed { .. } => ExportSaveErrorCategory::EncodingFailed,
            Self::InvalidPixelBuffer { .. } => ExportSaveErrorCategory::ImageDataInvalid,
            Self::AllocationFailed { .. } => ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory,
        }
    }

    pub fn brief_user_message(&self) -> &'static str {
        match self.category() {
            ExportSaveErrorCategory::PermissionDenied => "이미지를 저장할 권한이 없습니다.",
            ExportSaveErrorCategory::PathNotFound => "저장 경로를 찾을 수 없습니다.",
            ExportSaveErrorCategory::FileLocked => "저장할 파일이 사용 중입니다.",
            ExportSaveErrorCategory::EncodingFailed => "이미지를 인코딩하지 못했습니다.",
            ExportSaveErrorCategory::ImageDataInvalid => {
                "내보낼 이미지 데이터가 올바르지 않습니다."
            }
            ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => {
                "내보내기 중 메모리가 부족합니다."
            }
            ExportSaveErrorCategory::UnknownIo => "이미지를 저장하는 중 I/O 오류가 발생했습니다.",
        }
    }

    pub fn brief_user_message_for(&self, language: UiLanguage) -> &'static str {
        if language == UiLanguage::Korean {
            return self.brief_user_message();
        }
        match self.category() {
            ExportSaveErrorCategory::PermissionDenied => {
                "You do not have permission to save the image."
            }
            ExportSaveErrorCategory::PathNotFound => "Could not find the save path.",
            ExportSaveErrorCategory::FileLocked => "The file to save is in use.",
            ExportSaveErrorCategory::EncodingFailed => "Could not encode the image.",
            ExportSaveErrorCategory::ImageDataInvalid => "The export image data is invalid.",
            ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => {
                "Memory is insufficient during export."
            }
            ExportSaveErrorCategory::UnknownIo => "An I/O error occurred while saving the image.",
        }
    }
}

impl fmt::Display for ExportImageError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::FileCreate { path, .. } => {
                write!(
                    formatter,
                    "failed to create export file: {}",
                    path.display()
                )
            }
            Self::FileWrite { path, .. } => {
                write!(formatter, "failed to write export file: {}", path.display())
            }
            Self::FileReplace {
                temporary_path,
                path,
                ..
            } => write!(
                formatter,
                "failed to replace export file: {} -> {}",
                temporary_path.display(),
                path.display()
            ),
            Self::EncodeFailed { path, format, .. } => write!(
                formatter,
                "failed to encode export image as {:?}: {}",
                format,
                path.display()
            ),
            Self::PngOptimizeFailed { path, .. } => {
                write!(
                    formatter,
                    "failed to optimize PNG export: {}",
                    path.display()
                )
            }
            Self::InvalidPixelBuffer {
                path,
                size,
                actual_len,
            } => write!(
                formatter,
                "invalid export pixel buffer: {} ({}x{}, {} bytes)",
                path.display(),
                size.width(),
                size.height(),
                actual_len
            ),
            Self::AllocationFailed { path } => {
                write!(
                    formatter,
                    "failed to allocate export buffer: {}",
                    path.display()
                )
            }
        }
    }
}

impl Error for ExportImageError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::FileCreate { source, .. }
            | Self::FileWrite { source, .. }
            | Self::FileReplace { source, .. } => Some(source),
            Self::EncodeFailed { source, .. } => Some(source),
            Self::PngOptimizeFailed { source, .. } => Some(source),
            Self::InvalidPixelBuffer { .. } | Self::AllocationFailed { .. } => None,
        }
    }
}

pub fn load_image_file(path: impl AsRef<Path>) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view(
        path.as_ref(),
        ViewportSize::EMPTY,
        DEFAULT_IMAGE_MEMORY_POLICY,
        None,
    )
}

pub fn loaded_image_file_version_matches_current(
    image: &LoadedImage,
) -> Result<bool, LoadImageError> {
    let path = image.metadata().path();
    let current_metadata = read_file_metadata(path)?;
    Ok(match image.metadata().file_version() {
        Some(expected) => image_file_version_from_metadata(&current_metadata) == Some(expected),
        None => current_metadata.len() == image.metadata().file_size(),
    })
}

pub fn load_image_file_for_view(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing(
        path,
        viewport,
        policy,
        AnimationTimingSettings::default(),
        cancel,
    )
}

pub fn load_image_file_for_view_with_timing(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing_profiled(
        path,
        viewport,
        policy,
        animation_timing,
        None,
        cancel,
        None,
        StaticFullResolutionCacheMode::Refresh,
    )
}

pub(crate) fn preload_image_file_for_view_with_timing(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing_profiled(
        path,
        viewport,
        policy,
        animation_timing,
        None,
        cancel,
        None,
        StaticFullResolutionCacheMode::Preserve,
    )
}

pub fn load_image_file_for_view_with_timing_and_render_ready(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    render_ready_spec: Option<RenderReadySpec>,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing_profiled(
        path,
        viewport,
        policy,
        animation_timing,
        render_ready_spec,
        cancel,
        None,
        StaticFullResolutionCacheMode::Refresh,
    )
}

pub fn load_image_file_for_view_with_timing_and_profile(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    cancel: Option<&AtomicBool>,
    profiler: &mut ImageOpenProfiler,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing_profiled(
        path,
        viewport,
        policy,
        animation_timing,
        None,
        cancel,
        Some(profiler),
        StaticFullResolutionCacheMode::Refresh,
    )
}

pub fn load_image_file_for_view_with_render_ready_and_profile(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    render_ready_spec: Option<RenderReadySpec>,
    cancel: Option<&AtomicBool>,
    profiler: &mut ImageOpenProfiler,
) -> Result<LoadedImage, LoadImageError> {
    load_image_file_for_view_with_timing_profiled(
        path,
        viewport,
        policy,
        animation_timing,
        render_ready_spec,
        cancel,
        Some(profiler),
        StaticFullResolutionCacheMode::Refresh,
    )
}

fn load_image_file_for_view_with_timing_profiled(
    path: impl AsRef<Path>,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    render_ready_spec: Option<RenderReadySpec>,
    cancel: Option<&AtomicBool>,
    mut profiler: Option<&mut ImageOpenProfiler>,
    static_cache_mode: StaticFullResolutionCacheMode,
) -> Result<LoadedImage, LoadImageError> {
    let path = path.as_ref();
    let format = image_format_for_open(path)?;
    record_image_open_profile_stage(&mut profiler, "open.format_detection");
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(&mut profiler, "open.initial_cancel_check");
    if static_cache_mode.refreshes_cache() {
        replace_static_full_resolution_cache(None);
    }
    record_image_open_profile_stage(&mut profiler, "open.static_full_resolution_cache_reset");
    let image = if is_static_image_format(format) {
        load_static_image_for_view_profiled(
            path,
            format,
            viewport,
            policy,
            cancel,
            &mut profiler,
            static_cache_mode,
        )?
    } else {
        let metadata = read_file_metadata(path)?;
        record_image_open_profile_stage(&mut profiler, "animation.read_file_metadata");
        check_canceled(path, cancel)?;
        record_image_open_profile_stage(&mut profiler, "animation.metadata_cancel_check");
        let image = load_animation_image_for_view(
            path,
            format,
            &metadata,
            viewport,
            policy,
            animation_timing,
            cancel,
        )?;
        record_image_open_profile_stage(&mut profiler, "animation.decode_and_build_loaded_image");
        image
    };
    let image = attach_render_ready_image_for_view(path, image, render_ready_spec, policy, cancel)?;
    if render_ready_spec.is_some() {
        record_image_open_profile_stage(&mut profiler, "open.prepare_render_ready_buffer");
    }
    record_image_open_profile_stage(&mut profiler, "open.complete");
    Ok(image)
}

fn load_static_image_for_view(
    path: &Path,
    format: SupportedImageFormat,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    let mut profiler = None;
    load_static_image_for_view_profiled(
        path,
        format,
        viewport,
        policy,
        cancel,
        &mut profiler,
        StaticFullResolutionCacheMode::Refresh,
    )
}

fn load_static_image_for_view_profiled(
    path: &Path,
    format: SupportedImageFormat,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
    static_cache_mode: StaticFullResolutionCacheMode,
) -> Result<LoadedImage, LoadImageError> {
    let (file_metadata, mut decoder, preview_source) = open_image_decoder_with_metadata_and_source(
        path,
        format,
        decode_limits(policy.max_transient_decode_bytes()),
        cancel,
    )?;
    record_image_open_profile_stage(profiler, "static.open_file_metadata_decoder");
    let source_size = image_decoder_size(&decoder);
    record_image_open_profile_stage(profiler, "static.read_source_dimensions");
    check_canceled(path, cancel)?;
    if source_size.is_empty() || is_image_too_large(source_size, policy) {
        return Err(LoadImageError::ImageTooLarge {
            path: path.to_path_buf(),
            size: source_size,
            source: None,
        });
    }
    record_image_open_profile_stage(profiler, "static.source_size_policy_check");

    let exif_orientation = read_decoder_exif_orientation(path, &mut decoder, cancel)?;
    record_image_open_profile_stage(profiler, "static.read_exif_orientation");
    let image_metadata = image_metadata_from_file(path, &file_metadata, format, exif_orientation);
    record_image_open_profile_stage(profiler, "static.build_image_metadata");

    let preview_size = preview_size_for_viewport(source_size, viewport, policy);
    record_image_open_profile_stage(profiler, "static.compute_preview_size");
    let image =
        if should_load_static_preview_first(format, source_size, viewport, preview_size, policy) {
            let source_key = StaticImageSourceCacheKey::new(path, format, &file_metadata);
            record_image_open_profile_stage(profiler, "static.decode_preview_pixels.begin");
            let preview = decode_static_preview_pixel_image_from_decoder(
                path,
                format,
                source_key.as_ref(),
                decoder,
                &preview_source,
                policy,
                preview_size,
                source_size,
                !is_large_image(source_size, policy),
                cancel,
                profiler,
                static_cache_mode,
            )?;
            record_image_open_profile_stage(profiler, "static.decode_preview_pixels.complete");
            LoadedImage::from_preview_pixels(preview, source_size, image_metadata)
        } else {
            let pixels = decode_pixel_image_from_decoder_profiled(
                path,
                decoder,
                policy.max_transient_decode_bytes(),
                cancel,
                profiler,
            )?;
            check_canceled(path, cancel)?;
            record_image_open_profile_stage(profiler, "static.final_cancel_check");
            LoadedImage::from_pixels(pixels, image_metadata)
        };
    record_image_open_profile_stage(profiler, "static.build_loaded_image");

    if let Err(error) = ensure_current_file_metadata_matches(path, &file_metadata, cancel) {
        replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
        return Err(error);
    }
    record_image_open_profile_stage(profiler, "static.verify_file_unchanged");
    Ok(image)
}

fn attach_render_ready_image_for_view(
    path: &Path,
    mut image: LoadedImage,
    spec: Option<RenderReadySpec>,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    let Some(spec) = spec else {
        return Ok(image);
    };

    if let Some(render_ready) =
        render_ready_image_for_loaded_image(path, &image, spec, policy, cancel)?
    {
        image.set_render_ready_image(Some(render_ready));
    }
    Ok(image)
}

fn render_ready_image_for_loaded_image(
    path: &Path,
    image: &LoadedImage,
    spec: RenderReadySpec,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<RenderReadyImage>, LoadImageError> {
    if spec.viewport().is_empty() || spec.view_mode() != ViewMode::FitToWindow {
        return Ok(None);
    }

    let orientation = display_orientation(image.metadata().exif_orientation(), ImageRotation::ZERO);
    let logical_size = image.source_size().with_orientation(orientation);
    let transform = ViewTransform::FIT_TO_WINDOW;
    let Some(rect) = transform.display_rect(spec.viewport(), logical_size) else {
        return Ok(None);
    };
    let Some(target_size) = rect.size() else {
        return Ok(None);
    };
    let Some(effective_scale) = transform.effective_scale(spec.viewport(), logical_size) else {
        return Ok(None);
    };
    let scaling_quality = scaling_quality_for_render(spec.scaling_quality(), effective_scale);
    let render_format = render_ready_pixel_format(image.pixels().pixel_format());
    if !render_ready_buffer_fits_policy(image, target_size, render_format, policy) {
        return Ok(None);
    }

    check_canceled(path, cancel)?;
    let oriented = if orientation.is_identity() {
        image.pixels().clone()
    } else {
        orient_pixel_image(image.pixels(), orientation)
            .ok_or_else(|| invalid_pixel_buffer_error(path))?
    };
    check_canceled(path, cancel)?;

    let pixels = resize_render_ready_pixels(path, &oriented, target_size, scaling_quality, cancel)?;
    Ok(RenderReadyImage::new(
        pixels,
        spec.viewport(),
        spec.view_mode(),
        spec.scaling_quality(),
        orientation,
        rect,
        scaling_quality,
    ))
}

fn render_ready_pixel_format(format: PixelFormat) -> PixelFormat {
    match format {
        PixelFormat::Bgra8 => PixelFormat::Rgba8,
        PixelFormat::Rgb8 | PixelFormat::Rgba8 => format,
    }
}

fn render_ready_buffer_fits_policy(
    image: &LoadedImage,
    target_size: ImageSize,
    target_format: PixelFormat,
    policy: ImageMemoryPolicy,
) -> bool {
    let Some(target_bytes) = target_size.pixel_byte_len(target_format) else {
        return false;
    };
    target_bytes <= policy.max_cache_entry_bytes()
        && image
            .resident_byte_len()
            .checked_add(target_bytes)
            .is_some_and(|bytes| bytes <= policy.max_resident_bytes())
}

fn resize_render_ready_pixels(
    path: &Path,
    source: &PixelImage,
    target_size: ImageSize,
    quality: ScalingQuality,
    cancel: Option<&AtomicBool>,
) -> Result<PixelImage, LoadImageError> {
    if source.size() == target_size && source.pixel_format() != PixelFormat::Bgra8 {
        return Ok(source.clone());
    }

    check_canceled(path, cancel)?;
    let filter = filter_type_for_render_quality(quality);
    let pixels = match source {
        PixelImage::Rgb8(source) => {
            let source = BorrowedRgb8Image::new(source.width(), source.height(), source.pixels())
                .ok_or_else(|| invalid_pixel_buffer_error(path))?;
            let resized = resize(&source, target_size.width(), target_size.height(), filter);
            PixelImage::from(Rgb8Image::new(
                target_size.width(),
                target_size.height(),
                resized.into_raw(),
            ))
        }
        PixelImage::Rgba8(source) => {
            let source = BorrowedRgba8Image::new(source.width(), source.height(), source.pixels())
                .ok_or_else(|| invalid_pixel_buffer_error(path))?;
            let resized = resize(&source, target_size.width(), target_size.height(), filter);
            PixelImage::from(Rgba8Image::new(
                target_size.width(),
                target_size.height(),
                resized.into_raw(),
            ))
        }
        PixelImage::Bgra8(source) => {
            let rgba8 = PixelImage::from(source.clone())
                .to_rgba8()
                .ok_or_else(|| invalid_pixel_buffer_error(path))?;
            let source = BorrowedRgba8Image::new(rgba8.width(), rgba8.height(), rgba8.pixels())
                .ok_or_else(|| invalid_pixel_buffer_error(path))?;
            let resized = resize(&source, target_size.width(), target_size.height(), filter);
            PixelImage::from(Rgba8Image::new(
                target_size.width(),
                target_size.height(),
                resized.into_raw(),
            ))
        }
    };
    check_canceled(path, cancel)?;
    Ok(pixels)
}

fn filter_type_for_render_quality(quality: ScalingQuality) -> FilterType {
    match quality {
        ScalingQuality::Nearest => FilterType::Nearest,
        ScalingQuality::Balanced => FilterType::Triangle,
        ScalingQuality::HighQuality => FilterType::Lanczos3,
    }
}

fn invalid_pixel_buffer_error(path: &Path) -> LoadImageError {
    LoadImageError::InvalidPixelBuffer {
        path: path.to_path_buf(),
    }
}

struct BorrowedRgb8Image<'a> {
    width: u32,
    height: u32,
    row_stride: usize,
    pixels: &'a [u8],
}

impl<'a> BorrowedRgb8Image<'a> {
    fn new(width: u32, height: u32, pixels: &'a [u8]) -> Option<Self> {
        let row_stride = usize::try_from(width)
            .ok()?
            .checked_mul(RGB8_BYTES_PER_PIXEL)?;
        let expected_len = row_stride.checked_mul(usize::try_from(height).ok()?)?;
        if pixels.len() != expected_len {
            return None;
        }

        Some(Self {
            width,
            height,
            row_stride,
            pixels,
        })
    }
}

impl GenericImageView for BorrowedRgb8Image<'_> {
    type Pixel = Rgb<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let index = y as usize * self.row_stride + x as usize * RGB8_BYTES_PER_PIXEL;

        Rgb([
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
        ])
    }
}

struct BorrowedRgba8Image<'a> {
    width: u32,
    height: u32,
    row_stride: usize,
    pixels: &'a [u8],
}

impl<'a> BorrowedRgba8Image<'a> {
    fn new(width: u32, height: u32, pixels: &'a [u8]) -> Option<Self> {
        let row_stride = usize::try_from(width)
            .ok()?
            .checked_mul(RGBA8_BYTES_PER_PIXEL)?;
        let expected_len = row_stride.checked_mul(usize::try_from(height).ok()?)?;
        if pixels.len() != expected_len {
            return None;
        }

        Some(Self {
            width,
            height,
            row_stride,
            pixels,
        })
    }
}

impl GenericImageView for BorrowedRgba8Image<'_> {
    type Pixel = Rgba<u8>;

    fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }

    fn get_pixel(&self, x: u32, y: u32) -> Self::Pixel {
        let index = y as usize * self.row_stride + x as usize * RGBA8_BYTES_PER_PIXEL;

        Rgba([
            self.pixels[index],
            self.pixels[index + 1],
            self.pixels[index + 2],
            self.pixels[index + 3],
        ])
    }
}

fn decode_pixel_image_from_decoder_profiled(
    path: &Path,
    mut decoder: impl ImageDecoder,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
) -> Result<PixelImage, LoadImageError> {
    let mut limits = decode_limits(max_alloc_bytes);
    limits
        .reserve(decoder.total_bytes())
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    record_image_open_profile_stage(profiler, "static.reserve_decode_limits");
    decoder
        .set_limits(limits)
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    record_image_open_profile_stage(profiler, "static.apply_decode_limits");
    let decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    record_image_open_profile_stage(profiler, "static.decode_pixels");
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(profiler, "static.post_decode_cancel_check");
    let pixels = dynamic_image_into_pixel_image(decoded);
    record_image_open_profile_stage(profiler, "static.convert_to_pixel_image");
    Ok(pixels)
}

pub fn load_full_resolution_image(
    path: impl AsRef<Path>,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<PixelImage, LoadImageError> {
    load_full_resolution_image_with_file_version(path, policy, cancel).map(|(pixels, _)| pixels)
}

pub(crate) fn load_full_resolution_image_with_file_version(
    path: impl AsRef<Path>,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<(PixelImage, Option<ImageFileVersion>), LoadImageError> {
    let path = path.as_ref();
    let format = image_format_for_open(path)?;
    let cache_metadata = read_file_metadata(path)?;
    let cache_file_version = image_file_version_from_metadata(&cache_metadata);
    check_canceled(path, cancel)?;
    if let Some(source_key) = StaticImageSourceCacheKey::new(path, format, &cache_metadata) {
        if let Some(pixels) =
            take_cached_static_full_resolution_image(path, &source_key, policy, cancel)?
        {
            ensure_current_file_metadata_matches(path, &cache_metadata, cancel)?;
            return Ok((pixels, cache_file_version));
        }
    } else {
        replace_static_full_resolution_cache(None);
    }

    let (metadata, decoder) = open_image_decoder_with_known_metadata(
        path,
        format,
        decode_limits(policy.max_full_resolution_bytes()),
        cache_metadata,
        cancel,
    )?;
    let file_version = image_file_version_from_metadata(&metadata);
    let source_size = image_decoder_size(&decoder);
    check_canceled(path, cancel)?;
    if source_size.is_empty() || !should_retain_full_resolution(source_size, policy) {
        return Err(LoadImageError::ImageTooLarge {
            path: path.to_path_buf(),
            size: source_size,
            source: None,
        });
    }

    let pixels =
        decode_pixel_image_from_decoder(path, decoder, policy.max_full_resolution_bytes(), cancel)?;
    check_canceled(path, cancel)?;
    ensure_current_file_metadata_matches(path, &metadata, cancel)?;
    Ok((pixels, file_version))
}

pub fn load_animation_frame_for_view(
    path: impl AsRef<Path>,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let path = path.as_ref();
    match load_animation_frame_for_view_with_cache_radius(
        path,
        frame_index,
        viewport,
        policy,
        ANIMATION_FRAME_CACHE_RADIUS,
        None,
        None,
        cancel,
    )? {
        AnimationFrameRequestResult::Frame(frame) => Ok(frame.into_rgba8()),
        AnimationFrameRequestResult::Delivered => Err(LoadImageError::AnimationFrameUnavailable {
            path: path.to_path_buf(),
            frame_index,
        }),
    }
}

pub fn load_animation_frame_for_view_with_prefetch(
    path: impl AsRef<Path>,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    mut on_requested_frame: impl FnMut(Rgba8Image) -> bool,
    mut on_prefetched_frame: impl FnMut(usize, &Rgba8Image) -> bool,
    cancel: Option<&AtomicBool>,
) -> Result<(), LoadImageError> {
    let mut on_requested_frame =
        |_, frame: AnimationFramePixels| on_requested_frame(frame.into_rgba8());
    let mut on_prefetched_frame = |_, frame_index, frame: AnimationFramePixels| {
        on_prefetched_frame(frame_index, frame.as_rgba8())
    };
    load_animation_frame_for_view_with_prefetch_and_file_version(
        path,
        frame_index,
        viewport,
        policy,
        &mut on_requested_frame,
        &mut on_prefetched_frame,
        cancel,
    )
}

pub(crate) fn load_animation_frame_for_view_with_prefetch_and_file_version(
    path: impl AsRef<Path>,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    mut on_requested_frame: impl FnMut(Option<ImageFileVersion>, AnimationFramePixels) -> bool,
    mut on_prefetched_frame: impl FnMut(Option<ImageFileVersion>, usize, AnimationFramePixels) -> bool,
    cancel: Option<&AtomicBool>,
) -> Result<(), LoadImageError> {
    match load_animation_frame_for_view_with_cache_radius(
        path,
        frame_index,
        viewport,
        policy,
        ANIMATION_FRAME_PREFETCH_RADIUS,
        Some(&mut on_requested_frame),
        Some(&mut on_prefetched_frame),
        cancel,
    )? {
        AnimationFrameRequestResult::Frame(frame) => {
            on_requested_frame(None, frame);
            Ok(())
        }
        AnimationFrameRequestResult::Delivered => Ok(()),
    }
}

fn load_animation_frame_for_view_with_cache_radius(
    path: impl AsRef<Path>,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    forward_cache_radius: usize,
    mut on_requested_frame: Option<
        &mut dyn FnMut(Option<ImageFileVersion>, AnimationFramePixels) -> bool,
    >,
    mut on_prefetched_frame: Option<
        &mut dyn FnMut(Option<ImageFileVersion>, usize, AnimationFramePixels) -> bool,
    >,
    cancel: Option<&AtomicBool>,
) -> Result<AnimationFrameRequestResult, LoadImageError> {
    let path = path.as_ref();
    let format = image_format_for_open(path)?;
    let metadata = read_file_metadata(path)?;
    let file_version = image_file_version_from_metadata(&metadata);
    check_canceled(path, cancel)?;
    if let Some(source_key) = AnimationSourceCacheKey::new(path, format, &metadata) {
        if let Some(frame) = cached_animation_frame_for_view(
            path,
            &source_key,
            frame_index,
            viewport,
            policy,
            cancel,
        )? {
            if let Some(on_requested_frame) = on_requested_frame.as_deref_mut() {
                on_requested_frame(file_version, frame);
                return Ok(AnimationFrameRequestResult::Delivered);
            }
            return Ok(AnimationFrameRequestResult::Frame(frame));
        }

        let reusable_cache =
            reusable_animation_frame_cache_for_view(path, &source_key, viewport, policy, cancel)?;
        let has_requested_frame_callback = on_requested_frame.is_some();
        let has_prefetched_frame_callback = on_prefetched_frame.is_some();
        let mut requested_frame_callback = |frame| {
            on_requested_frame
                .as_deref_mut()
                .is_none_or(|callback| callback(file_version, frame))
        };
        let mut prefetched_frame_callback = |frame_index, frame: AnimationFramePixels| {
            on_prefetched_frame
                .as_deref_mut()
                .is_none_or(|callback| callback(file_version, frame_index, frame))
        };
        let requested_frame_callback = has_requested_frame_callback.then_some(
            &mut requested_frame_callback as &mut dyn FnMut(AnimationFramePixels) -> bool,
        );
        let prefetched_frame_callback = has_prefetched_frame_callback.then_some(
            &mut prefetched_frame_callback as &mut dyn FnMut(usize, AnimationFramePixels) -> bool,
        );
        return decode_animation_frame_for_format(
            path,
            format,
            Some(source_key),
            reusable_cache,
            frame_index,
            viewport,
            policy,
            forward_cache_radius,
            requested_frame_callback,
            prefetched_frame_callback,
            cancel,
        );
    }

    let has_requested_frame_callback = on_requested_frame.is_some();
    let has_prefetched_frame_callback = on_prefetched_frame.is_some();
    let mut requested_frame_callback = |frame| {
        on_requested_frame
            .as_deref_mut()
            .is_none_or(|callback| callback(file_version, frame))
    };
    let mut prefetched_frame_callback = |frame_index, frame: AnimationFramePixels| {
        on_prefetched_frame
            .as_deref_mut()
            .is_none_or(|callback| callback(file_version, frame_index, frame))
    };
    let requested_frame_callback = has_requested_frame_callback
        .then_some(&mut requested_frame_callback as &mut dyn FnMut(AnimationFramePixels) -> bool);
    let prefetched_frame_callback = has_prefetched_frame_callback.then_some(
        &mut prefetched_frame_callback as &mut dyn FnMut(usize, AnimationFramePixels) -> bool,
    );
    decode_animation_frame_for_format(
        path,
        format,
        None,
        None,
        frame_index,
        viewport,
        policy,
        forward_cache_radius,
        requested_frame_callback,
        prefetched_frame_callback,
        cancel,
    )
}

pub fn cached_animation_frame_for_loaded_image(
    path: impl AsRef<Path>,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Rgba8Image>, LoadImageError> {
    Ok(cached_animation_frame_pixels_for_loaded_image(
        path,
        file_version,
        format,
        source_size,
        frame_index,
        viewport,
        policy,
        cancel,
    )?
    .map(AnimationFramePixels::into_rgba8))
}

pub(crate) fn cached_animation_frame_pixels_for_loaded_image(
    path: impl AsRef<Path>,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<AnimationFramePixels>, LoadImageError> {
    let path = path.as_ref();
    cached_animation_frame_for_view_matching(path, frame_index, viewport, policy, cancel, |entry| {
        entry.source_key.path == path
            && entry.source_key.matches_file_version(file_version)
            && entry.source_key.format == format
            && entry.source_size == source_size
    })
}

pub fn animation_frame_prefetch_for_loaded_image_covers(
    path: impl AsRef<Path>,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    active_frame_index: usize,
    requested_frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
) -> bool {
    if requested_frame_index <= active_frame_index {
        return requested_frame_index == active_frame_index;
    }

    let path = path.as_ref();
    let buffer_kind = animation_buffer_kind(source_size, policy);
    let target_size = animation_target_size(source_size, viewport, policy, buffer_kind);
    let max_cache_byte_len = animation_frame_cache_byte_limit(policy);
    let reusable_cache = reusable_animation_frame_cache_window_for_loaded_image(
        path,
        file_version,
        format,
        source_size,
        target_size,
        buffer_kind,
    );
    let (_, cache_window_end) = animation_frame_cache_miss_window_for_reuse_window(
        active_frame_index,
        ANIMATION_FRAME_PREFETCH_RADIUS,
        reusable_cache,
        target_size,
        max_cache_byte_len,
    );
    let prefetch_end = animation_frame_delivered_prefetch_end(
        active_frame_index,
        cache_window_end,
        target_size,
        max_cache_byte_len,
    );

    requested_frame_index <= prefetch_end
}

pub fn export_rgba8_image(
    path: impl AsRef<Path>,
    rgba8: &Rgba8Image,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let path = path.as_ref();
    validate_export_pixel_buffer(path, rgba8)?;

    let (temporary_path, file) = create_export_temporary_file(path)?;
    if let Err(error) = write_export_temporary_file(path, &temporary_path, file, rgba8, options) {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    replace_export_file(&temporary_path, path).map_err(|source| {
        let _ = fs::remove_file(&temporary_path);
        ExportImageError::FileReplace {
            temporary_path,
            path: path.to_path_buf(),
            source,
        }
    })
}

pub(crate) fn export_owned_pixel_image(
    path: impl AsRef<Path>,
    pixels: PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let path = path.as_ref();
    validate_export_pixel_image(path, &pixels)?;

    let (temporary_path, file) = create_export_temporary_file(path)?;
    if let Err(error) =
        write_owned_export_pixel_temporary_file(path, &temporary_path, file, pixels, options)
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    replace_export_file(&temporary_path, path).map_err(|source| {
        let _ = fs::remove_file(&temporary_path);
        ExportImageError::FileReplace {
            temporary_path,
            path: path.to_path_buf(),
            source,
        }
    })
}

pub(crate) fn export_borrowed_pixel_image(
    path: impl AsRef<Path>,
    pixels: &PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let path = path.as_ref();
    validate_export_pixel_image(path, pixels)?;

    let (temporary_path, file) = create_export_temporary_file(path)?;
    if let Err(error) =
        write_borrowed_export_pixel_temporary_file(path, &temporary_path, file, pixels, options)
    {
        let _ = fs::remove_file(&temporary_path);
        return Err(error);
    }

    replace_export_file(&temporary_path, path).map_err(|source| {
        let _ = fs::remove_file(&temporary_path);
        ExportImageError::FileReplace {
            temporary_path,
            path: path.to_path_buf(),
            source,
        }
    })
}

fn create_export_temporary_file(path: &Path) -> Result<(PathBuf, File), ExportImageError> {
    let file_name = path
        .file_name()
        .filter(|file_name| !file_name.is_empty())
        .ok_or_else(|| ExportImageError::FileCreate {
            path: path.to_path_buf(),
            source: io::Error::new(io::ErrorKind::InvalidInput, "export path has no file name"),
        })?;
    let process_id = std::process::id();

    for attempt in 0..EXPORT_TEMP_FILE_CREATE_ATTEMPTS {
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
                return Err(ExportImageError::FileCreate {
                    path: path.to_path_buf(),
                    source,
                });
            }
        }
    }

    Err(ExportImageError::FileCreate {
        path: path.to_path_buf(),
        source: io::Error::new(
            io::ErrorKind::AlreadyExists,
            "could not create a unique temporary export file",
        ),
    })
}

fn write_export_temporary_file(
    path: &Path,
    temporary_path: &Path,
    file: File,
    rgba8: &Rgba8Image,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let optimize_png = {
        let mut writer = BufWriter::new(file);
        encode_rgba8_image(path, &mut writer, rgba8, options)?;
        finish_export_temporary_file_after_encode(path, &mut writer, options)?
    };

    if optimize_png {
        optimize_png_export_temporary_file(path, temporary_path, options)
    } else {
        Ok(())
    }
}

fn write_owned_export_pixel_temporary_file(
    path: &Path,
    temporary_path: &Path,
    file: File,
    pixels: PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let optimize_png = {
        let mut writer = BufWriter::new(file);
        encode_owned_pixel_image(path, &mut writer, pixels, options)?;
        finish_export_temporary_file_after_encode(path, &mut writer, options)?
    };

    if optimize_png {
        optimize_png_export_temporary_file(path, temporary_path, options)
    } else {
        Ok(())
    }
}

fn write_borrowed_export_pixel_temporary_file(
    path: &Path,
    temporary_path: &Path,
    file: File,
    pixels: &PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let optimize_png = {
        let mut writer = BufWriter::new(file);
        encode_pixel_image(path, &mut writer, pixels, options)?;
        finish_export_temporary_file_after_encode(path, &mut writer, options)?
    };

    if optimize_png {
        optimize_png_export_temporary_file(path, temporary_path, options)
    } else {
        Ok(())
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum ExportWriterSyncPolicy {
    FlushOnly,
    FlushAndSyncAll,
}

fn finish_export_temporary_file_after_encode(
    path: &Path,
    writer: &mut BufWriter<File>,
    options: ExportOptions,
) -> Result<bool, ExportImageError> {
    let optimize_png = should_optimize_png_export_after_encode(path, writer, options)?;
    sync_export_writer(
        path,
        writer,
        export_writer_sync_policy(options, optimize_png),
    )?;
    Ok(optimize_png)
}

fn should_optimize_png_export_after_encode(
    path: &Path,
    writer: &mut BufWriter<File>,
    options: ExportOptions,
) -> Result<bool, ExportImageError> {
    if options.format() != ExportFormat::Png {
        return Ok(false);
    }

    writer
        .flush()
        .map_err(|source| ExportImageError::FileWrite {
            path: path.to_path_buf(),
            source,
        })?;

    let encoded_len = writer
        .get_ref()
        .metadata()
        .map_err(|source| ExportImageError::FileWrite {
            path: path.to_path_buf(),
            source,
        })?
        .len();
    Ok(should_optimize_png_export_encoded_len(encoded_len))
}

fn should_optimize_png_export_encoded_len(encoded_len: u64) -> bool {
    (PNG_EXPORT_OXIPNG_MIN_INPUT_BYTES..=PNG_EXPORT_OXIPNG_MAX_INPUT_BYTES).contains(&encoded_len)
}

fn export_writer_sync_policy(options: ExportOptions, optimize_png: bool) -> ExportWriterSyncPolicy {
    if options.format() == ExportFormat::Png && optimize_png {
        ExportWriterSyncPolicy::FlushOnly
    } else {
        ExportWriterSyncPolicy::FlushAndSyncAll
    }
}

fn sync_export_writer(
    path: &Path,
    writer: &mut BufWriter<File>,
    policy: ExportWriterSyncPolicy,
) -> Result<(), ExportImageError> {
    let result = match policy {
        // PNG is optimized in place next, so syncing pre-optimized bytes only adds disk I/O.
        ExportWriterSyncPolicy::FlushOnly => writer.flush(),
        ExportWriterSyncPolicy::FlushAndSyncAll => {
            writer.flush().and_then(|()| writer.get_ref().sync_all())
        }
    };

    result.map_err(|source| ExportImageError::FileWrite {
        path: path.to_path_buf(),
        source,
    })
}

fn optimize_png_export_temporary_file(
    path: &Path,
    temporary_path: &Path,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    if options.format() != ExportFormat::Png {
        return Ok(());
    }

    let input = oxipng::InFile::Path(temporary_path.to_path_buf());
    let output = oxipng::OutFile::Path {
        path: None,
        preserve_attrs: false,
    };
    oxipng::optimize(&input, &output, &png_export_oxipng_options()).map_err(|source| {
        ExportImageError::PngOptimizeFailed {
            path: path.to_path_buf(),
            source,
        }
    })?;

    OpenOptions::new()
        .read(true)
        .write(true)
        .open(temporary_path)
        .and_then(|file| file.sync_all())
        .map_err(|source| ExportImageError::FileWrite {
            path: path.to_path_buf(),
            source,
        })
}

fn png_export_oxipng_options() -> oxipng::Options {
    let mut options = oxipng::Options::from_preset(PNG_EXPORT_OXIPNG_PRESET);
    options.bit_depth_reduction = false;
    options.color_type_reduction = false;
    options.palette_reduction = false;
    options.grayscale_reduction = false;
    options
}

struct AnimationDecodeResult {
    first_frame: Rgba8Image,
    source_size: ImageSize,
    exif_orientation: ImageOrientation,
    buffer_kind: ImageBufferKind,
    playback: Option<AnimationPlayback>,
}

struct AnimationMetadata {
    frame_delays_ms: Vec<u32>,
}

struct AnimationMetadataReadHandle {
    cancel: Arc<AtomicBool>,
    handle: std::thread::JoinHandle<Result<Option<AnimationMetadata>, LoadImageError>>,
}

struct AnimationParallelMetadataDecode<I> {
    format: SupportedImageFormat,
    probe: ImageProbe,
    cache_key: Option<AnimationSourceCacheKey>,
    loop_policy: AnimationLoopPolicy,
    frames: I,
    metadata_handle: AnimationMetadataReadHandle,
}

struct AnimationInitialDecodeResult {
    first_frame: AnimationFrameForReturn,
    source_size: ImageSize,
    exif_orientation: ImageOrientation,
    buffer_kind: ImageBufferKind,
    loop_policy: AnimationLoopPolicy,
    frame_cache: Option<AnimationFrameCacheBuilder>,
}

#[derive(Clone, Copy)]
struct AnimationDecodeContext<'a> {
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    cancel: Option<&'a AtomicBool>,
}

static STATIC_FULL_RESOLUTION_CACHE: OnceLock<Mutex<Option<StaticFullResolutionCacheEntry>>> =
    OnceLock::new();
static ANIMATION_FRAME_CACHE: OnceLock<Mutex<Option<AnimationFrameCacheEntry>>> = OnceLock::new();

#[derive(Clone, PartialEq, Eq)]
struct StaticImageSourceCacheKey {
    path: PathBuf,
    format: SupportedImageFormat,
    file_len: u64,
    modified: SystemTime,
}

impl StaticImageSourceCacheKey {
    fn new(path: &Path, format: SupportedImageFormat, metadata: &fs::Metadata) -> Option<Self> {
        let file_version = image_file_version_from_metadata(metadata)?;
        Some(Self {
            path: path.to_path_buf(),
            format,
            file_len: file_version.file_size(),
            modified: file_version.modified(),
        })
    }
}

struct StaticFullResolutionCacheEntry {
    source_key: StaticImageSourceCacheKey,
    source_size: ImageSize,
    decoded: image::DynamicImage,
}

#[derive(Clone, PartialEq, Eq)]
struct AnimationSourceCacheKey {
    path: PathBuf,
    format: SupportedImageFormat,
    file_len: u64,
    modified: SystemTime,
}

impl AnimationSourceCacheKey {
    fn new(path: &Path, format: SupportedImageFormat, metadata: &fs::Metadata) -> Option<Self> {
        let file_version = image_file_version_from_metadata(metadata)?;
        Some(Self {
            path: path.to_path_buf(),
            format,
            file_len: file_version.file_size(),
            modified: file_version.modified(),
        })
    }

    fn matches_file_version(&self, file_version: ImageFileVersion) -> bool {
        self.file_len == file_version.file_size() && self.modified == file_version.modified()
    }
}

struct AnimationFrameCacheEntry {
    source_key: AnimationSourceCacheKey,
    source_size: ImageSize,
    target_size: ImageSize,
    buffer_kind: ImageBufferKind,
    start_index: usize,
    frame_count: Option<usize>,
    frames: Vec<Arc<Rgba8Image>>,
    extra_frames: Vec<(usize, Arc<Rgba8Image>)>,
}

impl AnimationFrameCacheEntry {
    fn end_index(&self) -> Option<usize> {
        self.frames
            .len()
            .checked_sub(1)
            .and_then(|last_offset| self.start_index.checked_add(last_offset))
    }

    fn shared_frame(&self, frame_index: usize) -> Option<Arc<Rgba8Image>> {
        frame_index
            .checked_sub(self.start_index)
            .and_then(|offset| self.frames.get(offset))
            .map(Arc::clone)
            .or_else(|| {
                self.extra_frames
                    .iter()
                    .find(|(index, _)| *index == frame_index)
                    .map(|(_, frame)| Arc::clone(frame))
            })
    }

    fn resident_byte_len(&self) -> usize {
        self.frames
            .iter()
            .chain(self.extra_frames.iter().map(|(_, frame)| frame))
            .map(|frame| frame.pixels().len())
            .sum()
    }
}

struct AnimationFrameCacheReuse {
    start_index: usize,
    frame_count: Option<usize>,
    frames: Vec<Arc<Rgba8Image>>,
    extra_frames: Vec<(usize, Arc<Rgba8Image>)>,
}

#[derive(Clone, Copy)]
struct AnimationFrameCacheReuseWindow {
    start_index: usize,
    end_index: usize,
    frame_count: Option<usize>,
    frames_len: usize,
}

impl AnimationFrameCacheReuseWindow {
    fn from_reuse(reusable_cache: &AnimationFrameCacheReuse) -> Option<Self> {
        Some(Self {
            start_index: reusable_cache.start_index,
            end_index: reusable_cache.end_index()?,
            frame_count: reusable_cache.frame_count,
            frames_len: reusable_cache.frames.len(),
        })
    }
}

impl AnimationFrameCacheReuse {
    fn end_index(&self) -> Option<usize> {
        self.frames
            .len()
            .checked_sub(1)
            .and_then(|last_offset| self.start_index.checked_add(last_offset))
    }

    fn shared_frame(&self, frame_index: usize) -> Option<Arc<Rgba8Image>> {
        frame_index
            .checked_sub(self.start_index)
            .and_then(|offset| self.frames.get(offset))
            .map(Arc::clone)
            .or_else(|| {
                self.extra_frames
                    .iter()
                    .find(|(index, _)| *index == frame_index)
                    .map(|(_, frame)| Arc::clone(frame))
            })
    }

    fn retain_frames_outside_contiguous_cache(&self, cache: &mut AnimationFrameCacheBuilder) {
        for (offset, frame) in self.frames.iter().enumerate() {
            let Some(index) = self.start_index.checked_add(offset) else {
                break;
            };
            if !cache.contains_contiguous_index(index)
                && !cache.push_extra_shared(index, Arc::clone(frame))
            {
                return;
            }
        }

        for (index, frame) in &self.extra_frames {
            if !cache.contains_contiguous_index(*index)
                && !cache.push_extra_shared(*index, Arc::clone(frame))
            {
                return;
            }
        }
    }
}

enum AnimationFrameRequestResult {
    Frame(AnimationFramePixels),
    Delivered,
}

pub(crate) enum AnimationFramePixels {
    Owned(Rgba8Image),
    Shared(Arc<Rgba8Image>),
}

type AnimationFrameForReturn = AnimationFramePixels;

impl AnimationFramePixels {
    fn owned(frame: Rgba8Image) -> Self {
        Self::Owned(frame)
    }

    fn shared(frame: Arc<Rgba8Image>) -> Self {
        Self::Shared(frame)
    }

    pub(crate) fn as_rgba8(&self) -> &Rgba8Image {
        match self {
            Self::Owned(frame) => frame,
            Self::Shared(frame) => frame.as_ref(),
        }
    }

    pub(crate) fn into_rgba8(self) -> Rgba8Image {
        match self {
            Self::Owned(frame) => frame,
            Self::Shared(frame) => match Arc::try_unwrap(frame) {
                Ok(frame) => frame,
                Err(frame) => frame.as_ref().clone(),
            },
        }
    }
}

impl From<Rgba8Image> for AnimationFramePixels {
    fn from(frame: Rgba8Image) -> Self {
        Self::owned(frame)
    }
}

struct AnimationFrameCacheBuilder {
    source_key: AnimationSourceCacheKey,
    source_size: ImageSize,
    target_size: ImageSize,
    buffer_kind: ImageBufferKind,
    start_index: usize,
    frame_count: Option<usize>,
    frames: Vec<Arc<Rgba8Image>>,
    extra_frames: Vec<(usize, Arc<Rgba8Image>)>,
    byte_len: usize,
    max_byte_len: usize,
}

impl AnimationFrameCacheBuilder {
    fn new(
        source_key: AnimationSourceCacheKey,
        source_size: ImageSize,
        target_size: ImageSize,
        buffer_kind: ImageBufferKind,
        max_byte_len: usize,
    ) -> Self {
        Self::new_starting_at(
            source_key,
            source_size,
            target_size,
            buffer_kind,
            0,
            max_byte_len,
        )
    }

    fn new_starting_at(
        source_key: AnimationSourceCacheKey,
        source_size: ImageSize,
        target_size: ImageSize,
        buffer_kind: ImageBufferKind,
        start_index: usize,
        max_byte_len: usize,
    ) -> Self {
        Self {
            source_key,
            source_size,
            target_size,
            buffer_kind,
            start_index,
            frame_count: None,
            frames: Vec::new(),
            extra_frames: Vec::new(),
            byte_len: 0,
            max_byte_len,
        }
    }

    fn push(&mut self, frame: Rgba8Image) -> bool {
        self.push_shared(Arc::new(frame))
    }

    fn push_shared(&mut self, frame: Arc<Rgba8Image>) -> bool {
        if !self.add_frame_byte_len(frame.as_ref()) {
            return false;
        }

        self.frames.push(frame);
        true
    }

    fn push_for_return(
        &mut self,
        frame: Rgba8Image,
    ) -> Result<AnimationFrameForReturn, Rgba8Image> {
        if !self.add_frame_byte_len(&frame) {
            return Err(frame);
        }

        let frame = Arc::new(frame);
        self.frames.push(Arc::clone(&frame));
        Ok(AnimationFrameForReturn::shared(frame))
    }

    fn push_extra_shared(&mut self, frame_index: usize, frame: Arc<Rgba8Image>) -> bool {
        if self.contains_index(frame_index) {
            return true;
        }

        if !self.add_frame_byte_len(frame.as_ref()) {
            return false;
        }

        self.extra_frames.push((frame_index, frame));
        true
    }

    fn add_frame_byte_len(&mut self, frame: &Rgba8Image) -> bool {
        let Some(byte_len) = self.frame_byte_len_after(frame) else {
            return false;
        };

        self.byte_len = byte_len;
        true
    }

    fn can_add_frame_byte_len(&self, frame: &Rgba8Image) -> bool {
        self.frame_byte_len_after(frame).is_some()
    }

    fn frame_byte_len_after(&self, frame: &Rgba8Image) -> Option<usize> {
        let frame_bytes = frame.pixels().len();
        let byte_len = self.byte_len.checked_add(frame_bytes)?;
        if byte_len > self.max_byte_len {
            return None;
        }

        Some(byte_len)
    }

    fn contains_contiguous_index(&self, frame_index: usize) -> bool {
        frame_index >= self.start_index
            && self
                .frames
                .len()
                .checked_sub(1)
                .and_then(|last_offset| self.start_index.checked_add(last_offset))
                .is_some_and(|end_index| frame_index <= end_index)
    }

    fn contains_index(&self, frame_index: usize) -> bool {
        self.contains_contiguous_index(frame_index)
            || self
                .extra_frames
                .iter()
                .any(|(index, _)| *index == frame_index)
    }

    fn set_frame_count(&mut self, frame_count: usize) {
        self.frame_count = Some(frame_count);
    }

    fn finish(self) -> Option<AnimationFrameCacheEntry> {
        if self.frames.is_empty() {
            return None;
        }

        Some(AnimationFrameCacheEntry {
            source_key: self.source_key,
            source_size: self.source_size,
            target_size: self.target_size,
            buffer_kind: self.buffer_kind,
            start_index: self.start_index,
            frame_count: self.frame_count,
            frames: self.frames,
            extra_frames: self.extra_frames,
        })
    }
}

fn animation_frame_cache_byte_limit(policy: ImageMemoryPolicy) -> usize {
    policy
        .max_cache_entry_bytes()
        .min(policy.max_resident_bytes())
}

fn static_full_resolution_cache_byte_limit(policy: ImageMemoryPolicy) -> usize {
    policy
        .max_full_resolution_bytes()
        .min(policy.max_cache_entry_bytes())
}

fn static_full_resolution_cache_fits_policy(
    decoded_byte_len: usize,
    resident_base_byte_len: usize,
    policy: ImageMemoryPolicy,
) -> bool {
    if decoded_byte_len > static_full_resolution_cache_byte_limit(policy) {
        return false;
    }

    resident_base_byte_len
        .checked_add(decoded_byte_len)
        .is_some_and(|resident_byte_len| resident_byte_len <= policy.max_resident_bytes())
}

fn animation_frame_cache_window(frame_index: usize) -> (usize, usize) {
    (
        frame_index.saturating_sub(ANIMATION_FRAME_CACHE_RADIUS),
        frame_index.saturating_add(ANIMATION_FRAME_CACHE_RADIUS),
    )
}

fn animation_frame_cache_miss_window(
    frame_index: usize,
    forward_cache_radius: usize,
    reusable_cache: Option<&AnimationFrameCacheReuse>,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> (usize, usize) {
    animation_frame_cache_miss_window_for_reuse_window(
        frame_index,
        forward_cache_radius,
        reusable_cache.and_then(AnimationFrameCacheReuseWindow::from_reuse),
        target_size,
        max_cache_byte_len,
    )
}

fn animation_frame_cache_miss_window_for_reuse_window(
    frame_index: usize,
    forward_cache_radius: usize,
    reusable_cache: Option<AnimationFrameCacheReuseWindow>,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> (usize, usize) {
    let (cache_window_start, cache_window_end) = animation_frame_cache_window(frame_index);
    let is_prefetch_request = forward_cache_radius > ANIMATION_FRAME_CACHE_RADIUS;
    let forward_cache_radius = animation_frame_cache_miss_forward_radius(forward_cache_radius);
    let cache_window_end = cache_window_end.max(frame_index.saturating_add(forward_cache_radius));

    let Some(reusable_cache) = reusable_cache else {
        return (cache_window_start, cache_window_end);
    };
    if let Some(last_frame_index) =
        full_animation_cache_window_end_index(reusable_cache, target_size, max_cache_byte_len)
    {
        return (0, last_frame_index);
    }
    let reusable_cache_end = reusable_cache.end_index;
    if cache_window_start > reusable_cache_end.saturating_add(1) {
        if reusable_cache.start_index == 0
            && animation_frame_prefix_cache_fits(cache_window_end, target_size, max_cache_byte_len)
        {
            return (0, cache_window_end);
        }
        if is_prefetch_request {
            return (
                cache_window_start,
                animation_frame_cache_prefetch_budget_end(
                    cache_window_start,
                    cache_window_end,
                    reusable_cache,
                    target_size,
                    max_cache_byte_len,
                ),
            );
        }
        return (cache_window_start, cache_window_end);
    }

    let cache_window_start = if is_prefetch_request {
        animation_frame_cache_prefetch_start(
            cache_window_start.min(reusable_cache.start_index),
            cache_window_start,
            frame_index,
            target_size,
            max_cache_byte_len,
        )
    } else {
        cache_window_start.min(reusable_cache.start_index)
    };
    let extension = reusable_cache
        .frames_len
        .min(ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT)
        .max(forward_cache_radius);
    let mut cache_window_end = cache_window_end.max(reusable_cache_end.saturating_add(extension));
    if is_prefetch_request {
        cache_window_end = animation_frame_cache_prefetch_budget_end(
            cache_window_start,
            cache_window_end,
            reusable_cache,
            target_size,
            max_cache_byte_len,
        );
    }
    if let Some(last_frame_index) = reusable_cache
        .frame_count
        .and_then(|frame_count| frame_count.checked_sub(1))
    {
        cache_window_end = cache_window_end.min(last_frame_index);
    }

    (cache_window_start, cache_window_end)
}

fn animation_frame_cache_prefetch_start(
    preferred_start: usize,
    fallback_start: usize,
    frame_index: usize,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> usize {
    if animation_frame_cache_range_fits(
        preferred_start,
        frame_index,
        target_size,
        max_cache_byte_len,
    ) {
        return preferred_start;
    }
    if animation_frame_cache_range_fits(
        fallback_start,
        frame_index,
        target_size,
        max_cache_byte_len,
    ) {
        return fallback_start;
    }
    frame_index
}

fn animation_frame_cache_prefetch_budget_end(
    cache_window_start: usize,
    cache_window_end: usize,
    reusable_cache: AnimationFrameCacheReuseWindow,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> usize {
    let Some(last_frame_index) = reusable_cache
        .frame_count
        .and_then(|frame_count| frame_count.checked_sub(1))
    else {
        return cache_window_end;
    };
    let Some(capacity_end) =
        animation_frame_cache_capacity_end(cache_window_start, target_size, max_cache_byte_len)
    else {
        return cache_window_end;
    };
    cache_window_end.max(capacity_end).min(last_frame_index)
}

fn animation_frame_delivered_prefetch_limit(
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> usize {
    let Some(frame_byte_len) = target_size.rgba8_byte_len() else {
        return 0;
    };
    if frame_byte_len == 0 {
        return 0;
    }

    ANIMATION_DELIVERED_PREFETCH_BYTE_LIMIT
        .min(max_cache_byte_len)
        .saturating_div(frame_byte_len)
        .min(ANIMATION_DELIVERED_PREFETCH_FRAME_LIMIT)
}

fn animation_frame_delivered_prefetch_end(
    frame_index: usize,
    cache_window_end: usize,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> usize {
    frame_index
        .saturating_add(animation_frame_delivered_prefetch_limit(
            target_size,
            max_cache_byte_len,
        ))
        .min(cache_window_end)
}

fn animation_frame_cache_miss_forward_radius(forward_cache_radius: usize) -> usize {
    if forward_cache_radius > ANIMATION_FRAME_CACHE_RADIUS {
        forward_cache_radius.max(ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT)
    } else {
        forward_cache_radius
    }
}

fn animation_frame_prefix_cache_fits(
    cache_window_end: usize,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> bool {
    let Some(frame_count) = cache_window_end.checked_add(1) else {
        return false;
    };
    animation_frame_cache_byte_len(target_size, frame_count)
        .is_some_and(|byte_len| byte_len <= max_cache_byte_len)
}

fn animation_frame_cache_range_fits(
    start_index: usize,
    end_index: usize,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> bool {
    let Some(frame_count) = end_index
        .checked_sub(start_index)
        .and_then(|offset| offset.checked_add(1))
    else {
        return false;
    };
    animation_frame_cache_byte_len(target_size, frame_count)
        .is_some_and(|byte_len| byte_len <= max_cache_byte_len)
}

fn animation_frame_cache_capacity_end(
    start_index: usize,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> Option<usize> {
    let frame_byte_len = target_size.rgba8_byte_len()?;
    if frame_byte_len == 0 {
        return None;
    }
    let frame_count = max_cache_byte_len / frame_byte_len;
    frame_count
        .checked_sub(1)
        .and_then(|last_offset| start_index.checked_add(last_offset))
}

fn full_animation_cache_window_end_index(
    reusable_cache: AnimationFrameCacheReuseWindow,
    target_size: ImageSize,
    max_cache_byte_len: usize,
) -> Option<usize> {
    if reusable_cache.start_index != 0 {
        return None;
    }
    let frame_count = reusable_cache.frame_count?;
    let total_byte_len = animation_frame_cache_byte_len(target_size, frame_count)?;
    if total_byte_len > max_cache_byte_len {
        return None;
    }

    frame_count.checked_sub(1)
}

fn animation_frame_cache_byte_len(target_size: ImageSize, frame_count: usize) -> Option<usize> {
    target_size.rgba8_byte_len()?.checked_mul(frame_count)
}

fn take_cached_static_full_resolution_image(
    path: &Path,
    source_key: &StaticImageSourceCacheKey,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<PixelImage>, LoadImageError> {
    check_canceled(path, cancel)?;
    let cached = {
        let mut cache = lock_static_full_resolution_cache();
        match cache.as_ref() {
            Some(entry) if entry.source_key == *source_key => cache.take(),
            Some(_) => {
                *cache = None;
                None
            }
            None => None,
        }
    };
    let Some(entry) = cached else {
        return Ok(None);
    };
    if entry.source_size.is_empty() || !should_retain_full_resolution(entry.source_size, policy) {
        return Err(LoadImageError::ImageTooLarge {
            path: path.to_path_buf(),
            size: entry.source_size,
            source: None,
        });
    }
    if !static_full_resolution_cache_fits_policy(entry.decoded.as_bytes().len(), 0, policy) {
        return Ok(None);
    }

    check_canceled(path, cancel)?;
    let pixels = dynamic_image_into_pixel_image(entry.decoded);
    check_canceled(path, cancel)?;
    Ok(Some(pixels))
}

fn replace_static_full_resolution_cache(entry: Option<StaticFullResolutionCacheEntry>) {
    let mut cache = lock_static_full_resolution_cache();
    *cache = entry;
}

fn replace_static_full_resolution_cache_for_mode(
    mode: StaticFullResolutionCacheMode,
    entry: Option<StaticFullResolutionCacheEntry>,
) {
    if mode.refreshes_cache() {
        replace_static_full_resolution_cache(entry);
    }
}

fn lock_static_full_resolution_cache(
) -> std::sync::MutexGuard<'static, Option<StaticFullResolutionCacheEntry>> {
    let cache = STATIC_FULL_RESOLUTION_CACHE.get_or_init(|| Mutex::new(None));
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn cached_animation_frame_for_view(
    path: &Path,
    source_key: &AnimationSourceCacheKey,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<AnimationFramePixels>, LoadImageError> {
    cached_animation_frame_for_view_matching(path, frame_index, viewport, policy, cancel, |entry| {
        entry.source_key == *source_key
    })
}

fn reusable_animation_frame_cache_for_view(
    path: &Path,
    source_key: &AnimationSourceCacheKey,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<Option<AnimationFrameCacheReuse>, LoadImageError> {
    check_canceled(path, cancel)?;
    let reusable_cache = {
        let cache = lock_animation_frame_cache();
        let Some(entry) = cache.as_ref() else {
            return Ok(None);
        };
        if entry.source_key != *source_key {
            return Ok(None);
        }
        if entry.end_index().is_none() {
            return Ok(None);
        }
        if entry.source_size.is_empty() || is_image_too_large(entry.source_size, policy) {
            return Err(LoadImageError::ImageTooLarge {
                path: path.to_path_buf(),
                size: entry.source_size,
                source: None,
            });
        }

        let buffer_kind = animation_buffer_kind(entry.source_size, policy);
        let target_size = animation_target_size(entry.source_size, viewport, policy, buffer_kind);
        if entry.buffer_kind != buffer_kind || entry.target_size != target_size {
            return Ok(None);
        }

        AnimationFrameCacheReuse {
            start_index: entry.start_index,
            frame_count: entry.frame_count,
            frames: entry.frames.clone(),
            extra_frames: entry.extra_frames.clone(),
        }
    };

    Ok(Some(reusable_cache))
}

fn reusable_animation_frame_cache_window_for_loaded_image(
    path: &Path,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    target_size: ImageSize,
    buffer_kind: ImageBufferKind,
) -> Option<AnimationFrameCacheReuseWindow> {
    let cache = lock_animation_frame_cache();
    let entry = cache.as_ref()?;
    let end_index = entry.end_index()?;
    if entry.source_key.path != path
        || !entry.source_key.matches_file_version(file_version)
        || entry.source_key.format != format
        || entry.source_size != source_size
        || entry.target_size != target_size
        || entry.buffer_kind != buffer_kind
    {
        return None;
    }

    Some(AnimationFrameCacheReuseWindow {
        start_index: entry.start_index,
        end_index,
        frame_count: entry.frame_count,
        frames_len: entry.frames.len(),
    })
}

fn cached_animation_frame_for_view_matching(
    path: &Path,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
    matches_source: impl FnOnce(&AnimationFrameCacheEntry) -> bool,
) -> Result<Option<AnimationFramePixels>, LoadImageError> {
    check_canceled(path, cancel)?;
    let cached_frame = {
        let cache = lock_animation_frame_cache();
        let Some(entry) = cache.as_ref() else {
            return Ok(None);
        };
        if !matches_source(entry) {
            return Ok(None);
        }
        if entry.source_size.is_empty() || is_image_too_large(entry.source_size, policy) {
            return Err(LoadImageError::ImageTooLarge {
                path: path.to_path_buf(),
                size: entry.source_size,
                source: None,
            });
        }

        let buffer_kind = animation_buffer_kind(entry.source_size, policy);
        let target_size = animation_target_size(entry.source_size, viewport, policy, buffer_kind);
        if entry.buffer_kind != buffer_kind || entry.target_size != target_size {
            return Ok(None);
        }

        let cached_frame = entry.shared_frame(frame_index);
        if cached_frame.is_none()
            && entry
                .frame_count
                .is_some_and(|frame_count| frame_index >= frame_count)
        {
            return Err(LoadImageError::AnimationFrameUnavailable {
                path: path.to_path_buf(),
                frame_index,
            });
        }

        cached_frame
    };

    let Some(cached_frame) = cached_frame else {
        return Ok(None);
    };

    check_canceled(path, cancel)?;
    Ok(Some(AnimationFramePixels::shared(cached_frame)))
}

fn replace_animation_frame_cache(entry: Option<AnimationFrameCacheEntry>) {
    let mut cache = lock_animation_frame_cache();
    *cache = entry;
}

pub(crate) fn clear_animation_frame_resident_cache() {
    replace_animation_frame_cache(None);
}

pub(crate) fn animation_frame_resident_cache_byte_len() -> usize {
    let cache = lock_animation_frame_cache();
    cache
        .as_ref()
        .map(AnimationFrameCacheEntry::resident_byte_len)
        .unwrap_or(0)
}

fn lock_animation_frame_cache() -> std::sync::MutexGuard<'static, Option<AnimationFrameCacheEntry>>
{
    let cache = ANIMATION_FRAME_CACHE.get_or_init(|| Mutex::new(None));
    match cache.lock() {
        Ok(guard) => guard,
        Err(poisoned) => poisoned.into_inner(),
    }
}

fn load_animation_image_for_view(
    path: &Path,
    format: SupportedImageFormat,
    file_metadata: &fs::Metadata,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    cancel: Option<&AtomicBool>,
) -> Result<LoadedImage, LoadImageError> {
    if is_static_image_format(format) {
        return load_static_image_for_view(path, format, viewport, policy, cancel);
    }

    let cache_key = AnimationSourceCacheKey::new(path, format, file_metadata);
    let context = AnimationDecodeContext {
        viewport,
        policy,
        animation_timing,
        cancel,
    };
    let decoded = decode_animation_image_for_format(path, format, cache_key, context)?;
    let image_metadata =
        image_metadata_from_file(path, file_metadata, format, decoded.exif_orientation);

    let image = if let Some(playback) = decoded.playback {
        LoadedImage::from_animation(
            decoded.first_frame,
            decoded.source_size,
            decoded.buffer_kind,
            image_metadata,
            playback,
        )
    } else if decoded.buffer_kind == ImageBufferKind::Preview {
        LoadedImage::from_preview(decoded.first_frame, decoded.source_size, image_metadata)
    } else {
        LoadedImage::new(decoded.first_frame, image_metadata)
    };

    Ok(image)
}

fn decode_animation_image_for_format(
    path: &Path,
    format: SupportedImageFormat,
    cache_key: Option<AnimationSourceCacheKey>,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationDecodeResult, LoadImageError> {
    if should_decode_animation_with_parallel_metadata(cache_key.as_ref()) {
        return match format {
            SupportedImageFormat::Gif => {
                decode_gif_animation_image_with_parallel_metadata(path, cache_key, context)
            }
            SupportedImageFormat::Webp => {
                decode_webp_animation_image_with_parallel_metadata(path, cache_key, context)
            }
            SupportedImageFormat::Jpeg
            | SupportedImageFormat::Png
            | SupportedImageFormat::Bmp
            | SupportedImageFormat::Ico
            | SupportedImageFormat::Tiff
            | SupportedImageFormat::Tga => {
                decode_animation_image_for_format_sequential(path, format, cache_key, context)
            }
        };
    }

    decode_animation_image_for_format_sequential(path, format, cache_key, context)
}

fn should_decode_animation_with_parallel_metadata(
    cache_key: Option<&AnimationSourceCacheKey>,
) -> bool {
    cache_key
        .is_some_and(|cache_key| cache_key.file_len >= ANIMATION_PARALLEL_METADATA_MIN_FILE_BYTES)
}

impl AnimationMetadataReadHandle {
    fn cancel(&self) {
        self.cancel.store(true, Ordering::Release);
    }

    fn join(
        self,
        path: &Path,
        cancel: Option<&AtomicBool>,
    ) -> Result<Option<AnimationMetadata>, LoadImageError> {
        while !self.handle.is_finished() {
            if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
                self.cancel();
                let _ = self.handle.join();
                return Err(LoadImageError::DecodeCanceled {
                    path: path.to_path_buf(),
                });
            }
            std::thread::sleep(ANIMATION_METADATA_JOIN_POLL_INTERVAL);
        }

        self.handle.join().map_err(|_| {
            animation_metadata_worker_io_error(path, "animation metadata worker panicked")
        })?
    }

    fn join_after_decode_error(self, error: LoadImageError) -> LoadImageError {
        if error.is_canceled() {
            self.cancel();
            let _ = self.handle.join();
            return error;
        }

        match self.handle.join() {
            Ok(Err(metadata_error)) => metadata_error,
            Ok(Ok(_)) | Err(_) => error,
        }
    }
}

fn spawn_animation_metadata_read(
    path: &Path,
    format: SupportedImageFormat,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationMetadataReadHandle, LoadImageError> {
    check_canceled(path, context.cancel)?;

    let file = File::open(path).map_err(|source| LoadImageError::FileAccess {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = file
        .metadata()
        .map_err(|source| LoadImageError::FileAccess {
            path: path.to_path_buf(),
            source,
        })?;
    require_file_metadata(path, metadata)?;
    check_canceled(path, context.cancel)?;

    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let worker_path = path.to_path_buf();
    let error_path = path.to_path_buf();
    let policy = context.policy;
    let animation_timing = context.animation_timing;
    let handle = std::thread::Builder::new()
        .name("j3pic-animation-metadata".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(CancelableFile {
                file: Arc::new(file),
                position: 0,
                cancel: Some(worker_cancel.as_ref()),
            });
            read_animation_metadata_for_format(
                &worker_path,
                format,
                &mut reader,
                AnimationDecodeContext {
                    viewport: ViewportSize::EMPTY,
                    policy,
                    animation_timing,
                    cancel: Some(worker_cancel.as_ref()),
                },
            )
        })
        .map_err(|source| LoadImageError::FileAccess {
            path: error_path,
            source,
        })?;

    Ok(AnimationMetadataReadHandle { cancel, handle })
}

fn animation_metadata_worker_io_error(path: &Path, message: &'static str) -> LoadImageError {
    LoadImageError::FileAccess {
        path: path.to_path_buf(),
        source: io::Error::other(message),
    }
}

fn decode_animation_image_for_format_sequential(
    path: &Path,
    format: SupportedImageFormat,
    cache_key: Option<AnimationSourceCacheKey>,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationDecodeResult, LoadImageError> {
    match format {
        SupportedImageFormat::Gif => {
            let reader = open_cancelable_buffered_file(path, context.cancel)?;
            let (metadata, reader) =
                read_animation_metadata_and_replay(path, format, reader, context)?;
            let mut decoder = GifDecoder::new(reader)
                .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
            set_decoder_limits(
                path,
                &mut decoder,
                context.policy.max_transient_decode_bytes(),
                context.cancel,
            )?;
            let probe =
                read_image_probe_from_decoder(path, &mut decoder, context.policy, context.cancel)?;
            let loop_policy = animation_loop_policy_from_image(decoder.loop_count());
            let frames = decoder.into_frames();
            collect_animation_decode_result(
                path,
                probe,
                context,
                cache_key,
                loop_policy,
                frames,
                metadata,
            )
        }
        SupportedImageFormat::Webp => {
            let reader = open_cancelable_buffered_file(path, context.cancel)?;
            let (metadata, reader) =
                read_animation_metadata_and_replay(path, format, reader, context)?;
            let mut decoder = WebPDecoder::new(reader)
                .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
            set_decoder_limits(
                path,
                &mut decoder,
                context.policy.max_transient_decode_bytes(),
                context.cancel,
            )?;
            let probe =
                read_image_probe_from_decoder(path, &mut decoder, context.policy, context.cancel)?;
            if !decoder.has_animation() {
                replace_animation_frame_cache(None);
                return decode_static_image_from_decoder(path, decoder, probe, context);
            }

            let loop_policy = animation_loop_policy_from_image(decoder.loop_count());
            let frames = decoder.into_frames();
            collect_animation_decode_result(
                path,
                probe,
                context,
                cache_key,
                loop_policy,
                frames,
                metadata,
            )
        }
        SupportedImageFormat::Jpeg
        | SupportedImageFormat::Png
        | SupportedImageFormat::Bmp
        | SupportedImageFormat::Ico
        | SupportedImageFormat::Tiff
        | SupportedImageFormat::Tga => {
            let mut decoder = open_image_decoder(
                path,
                format,
                decode_limits(context.policy.max_transient_decode_bytes()),
                context.cancel,
            )?;
            let probe =
                read_image_probe_from_decoder(path, &mut decoder, context.policy, context.cancel)?;
            replace_animation_frame_cache(None);
            decode_static_image_from_decoder(path, decoder, probe, context)
        }
    }
}

fn decode_static_image_from_decoder(
    path: &Path,
    decoder: impl ImageDecoder,
    probe: ImageProbe,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationDecodeResult, LoadImageError> {
    let buffer_kind = animation_buffer_kind(probe.source_size, context.policy);
    let first_frame = if buffer_kind == ImageBufferKind::Preview {
        let target_size = animation_target_size(
            probe.source_size,
            context.viewport,
            context.policy,
            buffer_kind,
        );
        decode_preview_rgba8_image_from_decoder(
            path,
            decoder,
            context.policy.max_transient_decode_bytes(),
            target_size,
            context.cancel,
        )?
    } else {
        let rgba8 = decode_rgba8_image_from_decoder(
            path,
            decoder,
            context.policy.max_transient_decode_bytes(),
            context.cancel,
        )?;
        check_canceled(path, context.cancel)?;
        rgba8
    };

    Ok(AnimationDecodeResult {
        first_frame,
        source_size: probe.source_size,
        exif_orientation: probe.exif_orientation,
        buffer_kind,
        playback: None,
    })
}

fn decode_gif_animation_image_with_parallel_metadata(
    path: &Path,
    cache_key: Option<AnimationSourceCacheKey>,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationDecodeResult, LoadImageError> {
    let metadata_handle =
        match spawn_animation_metadata_read(path, SupportedImageFormat::Gif, context) {
            Ok(handle) => handle,
            Err(error) if error.is_canceled() => return Err(error),
            Err(_) => {
                return decode_animation_image_for_format_sequential(
                    path,
                    SupportedImageFormat::Gif,
                    cache_key,
                    context,
                );
            }
        };

    let decoded = (|| {
        let reader = open_cancelable_buffered_file(path, context.cancel)?;
        let mut decoder = GifDecoder::new(reader)
            .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
        set_decoder_limits(
            path,
            &mut decoder,
            context.policy.max_transient_decode_bytes(),
            context.cancel,
        )?;
        let probe =
            read_image_probe_from_decoder(path, &mut decoder, context.policy, context.cancel)?;
        let loop_policy = animation_loop_policy_from_image(decoder.loop_count());
        Ok::<_, LoadImageError>((probe, loop_policy, decoder.into_frames()))
    })();
    let (probe, loop_policy, frames) = match decoded {
        Ok(decoded) => decoded,
        Err(error) => return Err(metadata_handle.join_after_decode_error(error)),
    };
    collect_animation_decode_result_from_parallel_metadata(
        path,
        context,
        AnimationParallelMetadataDecode {
            format: SupportedImageFormat::Gif,
            probe,
            cache_key,
            loop_policy,
            frames,
            metadata_handle,
        },
    )
}

fn decode_webp_animation_image_with_parallel_metadata(
    path: &Path,
    cache_key: Option<AnimationSourceCacheKey>,
    context: AnimationDecodeContext<'_>,
) -> Result<AnimationDecodeResult, LoadImageError> {
    let reader = open_cancelable_buffered_file(path, context.cancel)?;
    let mut decoder = WebPDecoder::new(reader)
        .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
    set_decoder_limits(
        path,
        &mut decoder,
        context.policy.max_transient_decode_bytes(),
        context.cancel,
    )?;
    let probe = read_image_probe_from_decoder(path, &mut decoder, context.policy, context.cancel)?;
    if !decoder.has_animation() {
        replace_animation_frame_cache(None);
        return decode_static_image_from_decoder(path, decoder, probe, context);
    }

    let metadata_handle =
        match spawn_animation_metadata_read(path, SupportedImageFormat::Webp, context) {
            Ok(handle) => handle,
            Err(error) if error.is_canceled() => return Err(error),
            Err(_) => {
                return decode_animation_image_for_format_sequential(
                    path,
                    SupportedImageFormat::Webp,
                    cache_key,
                    context,
                );
            }
        };

    let loop_policy = animation_loop_policy_from_image(decoder.loop_count());
    let frames = decoder.into_frames();
    collect_animation_decode_result_from_parallel_metadata(
        path,
        context,
        AnimationParallelMetadataDecode {
            format: SupportedImageFormat::Webp,
            probe,
            cache_key,
            loop_policy,
            frames,
            metadata_handle,
        },
    )
}

fn decode_animation_frame_for_format(
    path: &Path,
    format: SupportedImageFormat,
    cache_key: Option<AnimationSourceCacheKey>,
    reusable_cache: Option<AnimationFrameCacheReuse>,
    frame_index: usize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    forward_cache_radius: usize,
    on_requested_frame: Option<&mut dyn FnMut(AnimationFramePixels) -> bool>,
    on_prefetched_frame: Option<&mut dyn FnMut(usize, AnimationFramePixels) -> bool>,
    cancel: Option<&AtomicBool>,
) -> Result<AnimationFrameRequestResult, LoadImageError> {
    match format {
        SupportedImageFormat::Gif => {
            let mut decoder = GifDecoder::new(open_cancelable_buffered_file(path, cancel)?)
                .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
            let source_size = image_decoder_size(&decoder);
            check_canceled(path, cancel)?;
            if source_size.is_empty() || is_image_too_large(source_size, policy) {
                return Err(LoadImageError::ImageTooLarge {
                    path: path.to_path_buf(),
                    size: source_size,
                    source: None,
                });
            }
            decoder
                .set_limits(decode_limits(policy.max_transient_decode_bytes()))
                .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
            decode_animation_frame_from_iter_with_reuse(
                path,
                frame_index,
                source_size,
                viewport,
                policy,
                cache_key,
                decoder.into_frames(),
                forward_cache_radius,
                reusable_cache,
                on_requested_frame,
                on_prefetched_frame,
                cancel,
            )
        }
        SupportedImageFormat::Webp => {
            let mut decoder = WebPDecoder::new(open_cancelable_buffered_file(path, cancel)?)
                .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
            let source_size = image_decoder_size(&decoder);
            check_canceled(path, cancel)?;
            if source_size.is_empty() || is_image_too_large(source_size, policy) {
                return Err(LoadImageError::ImageTooLarge {
                    path: path.to_path_buf(),
                    size: source_size,
                    source: None,
                });
            }
            decoder
                .set_limits(decode_limits(policy.max_transient_decode_bytes()))
                .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
            if !decoder.has_animation() {
                return Err(LoadImageError::AnimationFrameUnavailable {
                    path: path.to_path_buf(),
                    frame_index,
                });
            }

            decode_animation_frame_from_iter_with_reuse(
                path,
                frame_index,
                source_size,
                viewport,
                policy,
                cache_key,
                decoder.into_frames(),
                forward_cache_radius,
                reusable_cache,
                on_requested_frame,
                on_prefetched_frame,
                cancel,
            )
        }
        SupportedImageFormat::Jpeg
        | SupportedImageFormat::Png
        | SupportedImageFormat::Bmp
        | SupportedImageFormat::Ico
        | SupportedImageFormat::Tiff
        | SupportedImageFormat::Tga => {
            let source_size = read_image_dimensions(path, format, cancel)?;
            if source_size.is_empty() || is_image_too_large(source_size, policy) {
                return Err(LoadImageError::ImageTooLarge {
                    path: path.to_path_buf(),
                    size: source_size,
                    source: None,
                });
            }

            Err(LoadImageError::AnimationFrameUnavailable {
                path: path.to_path_buf(),
                frame_index,
            })
        }
    }
}

fn collect_animation_decode_result<I>(
    path: &Path,
    probe: ImageProbe,
    context: AnimationDecodeContext<'_>,
    cache_key: Option<AnimationSourceCacheKey>,
    loop_policy: AnimationLoopPolicy,
    frames: I,
    metadata: Option<AnimationMetadata>,
) -> Result<AnimationDecodeResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    if let Some(metadata) = metadata {
        collect_animation_decode_result_from_metadata(
            path,
            probe,
            context,
            cache_key,
            loop_policy,
            frames,
            metadata,
        )
    } else {
        collect_animation_decode_result_from_frames(
            path,
            probe,
            context,
            cache_key,
            loop_policy,
            frames,
        )
    }
}

fn collect_animation_decode_result_from_parallel_metadata<I>(
    path: &Path,
    context: AnimationDecodeContext<'_>,
    decode: AnimationParallelMetadataDecode<I>,
) -> Result<AnimationDecodeResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    let AnimationParallelMetadataDecode {
        format,
        probe,
        cache_key,
        loop_policy,
        frames,
        metadata_handle,
    } = decode;
    let fallback_cache_key = cache_key.clone();
    let initial = match collect_animation_initial_decode_result(
        path,
        probe,
        context,
        cache_key,
        loop_policy,
        frames,
        ANIMATION_INITIAL_CACHE_FRAME_LIMIT,
    ) {
        Ok(initial) => initial,
        Err(error) => return Err(metadata_handle.join_after_decode_error(error)),
    };

    match metadata_handle.join(path, context.cancel)? {
        Some(metadata) => Ok(finish_animation_decode_result_from_metadata(
            context, initial, metadata,
        )),
        None => {
            drop(initial);
            decode_animation_image_for_format_sequential(path, format, fallback_cache_key, context)
        }
    }
}

fn collect_animation_decode_result_from_frames<I>(
    path: &Path,
    probe: ImageProbe,
    context: AnimationDecodeContext<'_>,
    cache_key: Option<AnimationSourceCacheKey>,
    loop_policy: AnimationLoopPolicy,
    frames: I,
) -> Result<AnimationDecodeResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    replace_animation_frame_cache(None);
    let buffer_kind = animation_buffer_kind(probe.source_size, context.policy);
    let target_size = animation_target_size(
        probe.source_size,
        context.viewport,
        context.policy,
        buffer_kind,
    );
    let mut frame_delays_ms = Vec::new();
    let mut first_frame = None;
    let mut frame_cache = cache_key.map(|source_key| {
        AnimationFrameCacheBuilder::new(
            source_key,
            probe.source_size,
            target_size,
            buffer_kind,
            animation_frame_cache_byte_limit(context.policy),
        )
    });
    let mut can_cache_frames = frame_cache.is_some();

    for frame_result in frames {
        check_canceled(path, context.cancel)?;
        if frame_delays_ms.len() >= context.policy.max_animation_metadata_frames() {
            return Err(LoadImageError::AnimationTooManyFrames {
                path: path.to_path_buf(),
                frame_count: frame_delays_ms.len().saturating_add(1),
                max_frame_count: context.policy.max_animation_metadata_frames(),
            });
        }

        let frame = frame_result
            .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
        let delay_ms = animation_delay_ms(frame.delay(), context.animation_timing);

        let frame_index = frame_delays_ms.len();
        let should_decode_frame = first_frame.is_none()
            || (can_cache_frames && frame_index < ANIMATION_INITIAL_CACHE_FRAME_LIMIT);
        if should_decode_frame {
            let decoded_frame =
                animation_frame_to_rgba8(path, frame, target_size, buffer_kind, context.cancel);
            match decoded_frame {
                Ok(decoded_frame) => {
                    if first_frame.is_none() {
                        if can_cache_frames {
                            if let Some(cache) = frame_cache.as_mut() {
                                match cache.push_for_return(decoded_frame) {
                                    Ok(frame) => {
                                        first_frame = Some(frame);
                                    }
                                    Err(decoded_frame) => {
                                        first_frame =
                                            Some(AnimationFrameForReturn::owned(decoded_frame));
                                        can_cache_frames = false;
                                    }
                                }
                            } else {
                                first_frame = Some(AnimationFrameForReturn::owned(decoded_frame));
                            }
                        } else {
                            first_frame = Some(AnimationFrameForReturn::owned(decoded_frame));
                        }
                    } else if can_cache_frames {
                        if let Some(cache) = frame_cache.as_mut() {
                            can_cache_frames = cache.push(decoded_frame);
                        }
                    }
                }
                Err(error) if first_frame.is_some() && !error.is_canceled() => {
                    frame_cache = None;
                    can_cache_frames = false;
                }
                Err(error) => return Err(error),
            }
        }
        frame_delays_ms.push(delay_ms);
    }

    let Some(first_frame) = first_frame else {
        return Err(LoadImageError::AnimationFrameUnavailable {
            path: path.to_path_buf(),
            frame_index: 0,
        });
    };

    let playback =
        AnimationPlayback::new_with_timing(frame_delays_ms, loop_policy, context.animation_timing);
    let cache_entry = if playback.is_some() {
        if let Some(cache) = frame_cache.as_mut() {
            cache.set_frame_count(
                playback
                    .as_ref()
                    .map_or(0, |playback| playback.frame_delays_ms().len()),
            );
        }
        frame_cache.and_then(AnimationFrameCacheBuilder::finish)
    } else {
        drop(frame_cache);
        None
    };
    replace_animation_frame_cache(cache_entry);
    let first_frame = first_frame.into_rgba8();

    Ok(AnimationDecodeResult {
        first_frame,
        source_size: probe.source_size,
        exif_orientation: probe.exif_orientation,
        buffer_kind,
        playback,
    })
}

fn collect_animation_decode_result_from_metadata<I>(
    path: &Path,
    probe: ImageProbe,
    context: AnimationDecodeContext<'_>,
    cache_key: Option<AnimationSourceCacheKey>,
    loop_policy: AnimationLoopPolicy,
    frames: I,
    metadata: AnimationMetadata,
) -> Result<AnimationDecodeResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    let initial_cache_frame_limit =
        ANIMATION_INITIAL_CACHE_FRAME_LIMIT.min(metadata.frame_delays_ms.len());
    let initial = collect_animation_initial_decode_result(
        path,
        probe,
        context,
        cache_key,
        loop_policy,
        frames,
        initial_cache_frame_limit,
    )?;

    Ok(finish_animation_decode_result_from_metadata(
        context, initial, metadata,
    ))
}

fn collect_animation_initial_decode_result<I>(
    path: &Path,
    probe: ImageProbe,
    context: AnimationDecodeContext<'_>,
    cache_key: Option<AnimationSourceCacheKey>,
    loop_policy: AnimationLoopPolicy,
    frames: I,
    initial_cache_frame_limit: usize,
) -> Result<AnimationInitialDecodeResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    replace_animation_frame_cache(None);
    let buffer_kind = animation_buffer_kind(probe.source_size, context.policy);
    let target_size = animation_target_size(
        probe.source_size,
        context.viewport,
        context.policy,
        buffer_kind,
    );
    let mut first_frame = None;
    let mut frame_cache = cache_key.map(|source_key| {
        AnimationFrameCacheBuilder::new(
            source_key,
            probe.source_size,
            target_size,
            buffer_kind,
            animation_frame_cache_byte_limit(context.policy),
        )
    });
    let mut can_cache_frames = frame_cache.is_some();
    let mut frames = frames.into_iter();

    for _frame_index in 0..initial_cache_frame_limit {
        if first_frame.is_some() && !can_cache_frames {
            break;
        }
        check_canceled(path, context.cancel)?;
        let Some(frame_result) = frames.next() else {
            break;
        };
        let frame = frame_result
            .map_err(|source| image_decode_error_or_canceled(path, source, context.cancel))?;
        let decoded_frame =
            animation_frame_to_rgba8(path, frame, target_size, buffer_kind, context.cancel)?;

        if first_frame.is_none() {
            if can_cache_frames {
                if let Some(cache) = frame_cache.as_mut() {
                    match cache.push_for_return(decoded_frame) {
                        Ok(frame) => {
                            first_frame = Some(frame);
                        }
                        Err(decoded_frame) => {
                            first_frame = Some(AnimationFrameForReturn::owned(decoded_frame));
                            can_cache_frames = false;
                        }
                    }
                } else {
                    first_frame = Some(AnimationFrameForReturn::owned(decoded_frame));
                }
            } else {
                first_frame = Some(AnimationFrameForReturn::owned(decoded_frame));
            }
        } else if can_cache_frames {
            if let Some(cache) = frame_cache.as_mut() {
                can_cache_frames = cache.push(decoded_frame);
            }
        }
    }

    let Some(first_frame) = first_frame else {
        return Err(LoadImageError::AnimationFrameUnavailable {
            path: path.to_path_buf(),
            frame_index: 0,
        });
    };

    Ok(AnimationInitialDecodeResult {
        first_frame,
        source_size: probe.source_size,
        exif_orientation: probe.exif_orientation,
        buffer_kind,
        loop_policy,
        frame_cache,
    })
}

fn finish_animation_decode_result_from_metadata(
    context: AnimationDecodeContext<'_>,
    initial: AnimationInitialDecodeResult,
    metadata: AnimationMetadata,
) -> AnimationDecodeResult {
    let AnimationInitialDecodeResult {
        first_frame,
        source_size,
        exif_orientation,
        buffer_kind,
        loop_policy,
        mut frame_cache,
    } = initial;
    let playback = AnimationPlayback::new_with_timing(
        metadata.frame_delays_ms,
        loop_policy,
        context.animation_timing,
    );
    let frame_count = playback.as_ref().map_or(1, AnimationPlayback::frame_count);

    let cache_entry = if playback.is_some() {
        let cache_exceeds_frame_count = frame_cache
            .as_ref()
            .is_some_and(|cache| cache.frames.len() > frame_count);
        if cache_exceeds_frame_count {
            frame_cache = None;
        } else if let Some(cache) = frame_cache.as_mut() {
            cache.set_frame_count(frame_count);
        }
        frame_cache.and_then(AnimationFrameCacheBuilder::finish)
    } else {
        drop(frame_cache);
        None
    };
    replace_animation_frame_cache(cache_entry);
    let first_frame = first_frame.into_rgba8();

    AnimationDecodeResult {
        first_frame,
        source_size,
        exif_orientation,
        buffer_kind,
        playback,
    }
}

#[cfg(test)]
fn decode_animation_frame_from_iter<I>(
    path: &Path,
    frame_index: usize,
    source_size: ImageSize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cache_key: Option<AnimationSourceCacheKey>,
    frames: I,
    forward_cache_radius: usize,
    on_requested_frame: Option<&mut dyn FnMut(AnimationFramePixels) -> bool>,
    cancel: Option<&AtomicBool>,
) -> Result<AnimationFrameRequestResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    decode_animation_frame_from_iter_with_reuse(
        path,
        frame_index,
        source_size,
        viewport,
        policy,
        cache_key,
        frames,
        forward_cache_radius,
        None,
        on_requested_frame,
        None,
        cancel,
    )
}

fn decode_animation_frame_from_iter_with_reuse<I>(
    path: &Path,
    frame_index: usize,
    source_size: ImageSize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    cache_key: Option<AnimationSourceCacheKey>,
    frames: I,
    forward_cache_radius: usize,
    reusable_cache: Option<AnimationFrameCacheReuse>,
    mut on_requested_frame: Option<&mut dyn FnMut(AnimationFramePixels) -> bool>,
    mut on_prefetched_frame: Option<&mut dyn FnMut(usize, AnimationFramePixels) -> bool>,
    cancel: Option<&AtomicBool>,
) -> Result<AnimationFrameRequestResult, LoadImageError>
where
    I: IntoIterator<Item = image::ImageResult<image::Frame>>,
{
    let buffer_kind = animation_buffer_kind(source_size, policy);
    let target_size = animation_target_size(source_size, viewport, policy, buffer_kind);
    let max_cache_byte_len = animation_frame_cache_byte_limit(policy);
    let cache_source_key = cache_key.clone();
    let (cache_window_start, cache_window_end) = animation_frame_cache_miss_window(
        frame_index,
        forward_cache_radius,
        reusable_cache.as_ref(),
        target_size,
        max_cache_byte_len,
    );
    let can_deliver_prefetched_frames = on_prefetched_frame.is_some();
    let delivered_prefetch_end = if on_requested_frame.is_some() {
        Some(animation_frame_delivered_prefetch_end(
            frame_index,
            cache_window_end,
            target_size,
            max_cache_byte_len,
        ))
    } else {
        None
    };
    let known_last_frame_index = reusable_cache
        .as_ref()
        .and_then(|cache| cache.frame_count)
        .and_then(|frame_count| frame_count.checked_sub(1));
    let mut active_cache_start = cache_window_start;
    let mut frame_cache = cache_key.map(|source_key| {
        AnimationFrameCacheBuilder::new_starting_at(
            source_key,
            source_size,
            target_size,
            buffer_kind,
            cache_window_start,
            max_cache_byte_len,
        )
    });
    let mut can_cache_frames = frame_cache.is_some();
    let mut start_cache_at_requested = false;
    let mut requested_frame = None;
    let mut requested_frame_delivered = false;
    let mut completed_iter = true;
    let mut decoded_frame_count = 0usize;

    for (index, frame_result) in frames.into_iter().enumerate() {
        decoded_frame_count = index.saturating_add(1);
        let requested_frame_seen = requested_frame.is_some() || requested_frame_delivered;
        if requested_frame_seen && cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
            frame_cache = None;
            completed_iter = false;
            break;
        }
        if !requested_frame_seen {
            check_canceled(path, cancel)?;
        }

        let frame = match frame_result {
            Ok(frame) => frame,
            Err(_) if requested_frame_seen => {
                frame_cache = None;
                completed_iter = false;
                break;
            }
            Err(source) => return Err(image_decode_error_or_canceled(path, source, cancel)),
        };

        let is_cache_window_frame =
            can_cache_frames && index >= active_cache_start && index <= cache_window_end;
        let is_prefetch_delivery_frame = requested_frame_seen
            && can_deliver_prefetched_frames
            && delivered_prefetch_end.is_some_and(|end| index <= end);
        if !is_cache_window_frame && index != frame_index && !is_prefetch_delivery_frame {
            continue;
        }
        if index != frame_index && is_cache_window_frame {
            if let Some(cached_frame) = reusable_cache
                .as_ref()
                .and_then(|cache| cache.shared_frame(index))
            {
                if requested_frame_seen {
                    if let Some(on_prefetched_frame) = on_prefetched_frame.as_deref_mut() {
                        if !on_prefetched_frame(
                            index,
                            AnimationFrameForReturn::shared(Arc::clone(&cached_frame)),
                        ) {
                            completed_iter = false;
                            break;
                        }
                    }
                }
                let did_push = frame_cache
                    .as_mut()
                    .is_some_and(|cache| cache.push_shared(cached_frame));
                if !did_push {
                    if index < frame_index {
                        frame_cache = None;
                        can_cache_frames = false;
                        start_cache_at_requested = cache_source_key.is_some();
                        continue;
                    }
                    completed_iter = false;
                    break;
                }

                if (requested_frame.is_some() || requested_frame_delivered)
                    && should_stop_animation_frame_cache_miss_after_request(
                        index,
                        cache_window_end,
                        delivered_prefetch_end,
                        can_cache_frames,
                        can_deliver_prefetched_frames,
                        known_last_frame_index,
                    )
                {
                    completed_iter = false;
                    break;
                }
                continue;
            }
        }

        let decoded_frame =
            match animation_frame_to_rgba8(path, frame, target_size, buffer_kind, cancel) {
                Ok(decoded_frame) => decoded_frame,
                Err(error) if index == frame_index => return Err(error),
                Err(error) if error.is_canceled() && !requested_frame_seen => return Err(error),
                Err(_) if requested_frame_seen => {
                    frame_cache = None;
                    completed_iter = false;
                    break;
                }
                Err(_) => {
                    frame_cache = None;
                    can_cache_frames = false;
                    if index < frame_index {
                        start_cache_at_requested = cache_source_key.is_some();
                    }
                    continue;
                }
            };

        if index == frame_index {
            if start_cache_at_requested {
                frame_cache = cache_source_key.clone().map(|source_key| {
                    AnimationFrameCacheBuilder::new_starting_at(
                        source_key,
                        source_size,
                        target_size,
                        buffer_kind,
                        frame_index,
                        max_cache_byte_len,
                    )
                });
                can_cache_frames = frame_cache.is_some();
                active_cache_start = frame_index;
                start_cache_at_requested = false;
            }

            let is_cache_window_frame =
                can_cache_frames && index >= active_cache_start && index <= cache_window_end;
            let has_requested_frame_callback = on_requested_frame.is_some();
            let mut skip_stop_check = false;
            let frame_for_return = if is_cache_window_frame {
                let push_result = if let Some(cache) = frame_cache.as_mut() {
                    cache.push_for_return(decoded_frame)
                } else {
                    Err(decoded_frame)
                };

                match push_result {
                    Ok(frame) => frame,
                    Err(decoded_frame) => {
                        frame_cache = None;
                        can_cache_frames = false;
                        if let Some(source_key) = cache_source_key.clone() {
                            let mut cache = AnimationFrameCacheBuilder::new_starting_at(
                                source_key,
                                source_size,
                                target_size,
                                buffer_kind,
                                frame_index,
                                max_cache_byte_len,
                            );
                            match cache.push_for_return(decoded_frame) {
                                Ok(frame) => {
                                    frame_cache = Some(cache);
                                    can_cache_frames = true;
                                    active_cache_start = frame_index;
                                    skip_stop_check = !has_requested_frame_callback;
                                    frame
                                }
                                Err(decoded_frame) => {
                                    if has_requested_frame_callback {
                                        AnimationFrameForReturn::owned(decoded_frame)
                                    } else {
                                        return Ok(AnimationFrameRequestResult::Frame(
                                            decoded_frame.into(),
                                        ));
                                    }
                                }
                            }
                        } else if has_requested_frame_callback {
                            AnimationFrameForReturn::owned(decoded_frame)
                        } else {
                            return Ok(AnimationFrameRequestResult::Frame(decoded_frame.into()));
                        }
                    }
                }
            } else {
                AnimationFrameForReturn::owned(decoded_frame)
            };

            if let Some(on_requested_frame) = on_requested_frame.as_deref_mut() {
                requested_frame_delivered = true;
                if !on_requested_frame(frame_for_return) {
                    completed_iter = false;
                    break;
                }
                if should_stop_animation_frame_cache_miss_after_request(
                    index,
                    cache_window_end,
                    delivered_prefetch_end,
                    can_cache_frames,
                    can_deliver_prefetched_frames,
                    known_last_frame_index,
                ) {
                    completed_iter = false;
                    break;
                }
                continue;
            }

            requested_frame = Some(frame_for_return);
            if skip_stop_check {
                continue;
            }
            if should_stop_animation_frame_cache_miss_after_request(
                index,
                cache_window_end,
                delivered_prefetch_end,
                can_cache_frames,
                can_deliver_prefetched_frames,
                known_last_frame_index,
            ) {
                completed_iter = false;
                break;
            }
            continue;
        }

        let is_cache_window_frame =
            can_cache_frames && index >= active_cache_start && index <= cache_window_end;
        if index != frame_index && requested_frame_seen {
            if let Some(on_prefetched_frame) = on_prefetched_frame.as_deref_mut() {
                if is_cache_window_frame
                    && frame_cache
                        .as_ref()
                        .is_some_and(|cache| cache.can_add_frame_byte_len(&decoded_frame))
                {
                    let shared_frame = Arc::new(decoded_frame);
                    if !on_prefetched_frame(
                        index,
                        AnimationFrameForReturn::shared(Arc::clone(&shared_frame)),
                    ) {
                        completed_iter = false;
                        break;
                    }
                    let did_push = frame_cache
                        .as_mut()
                        .is_some_and(|cache| cache.push_shared(shared_frame));
                    if !did_push {
                        completed_iter = false;
                        break;
                    }
                    if should_stop_animation_frame_cache_miss_after_request(
                        index,
                        cache_window_end,
                        delivered_prefetch_end,
                        can_cache_frames,
                        can_deliver_prefetched_frames,
                        known_last_frame_index,
                    ) {
                        completed_iter = false;
                        break;
                    }
                    continue;
                }

                if !on_prefetched_frame(index, AnimationFrameForReturn::owned(decoded_frame)) {
                    completed_iter = false;
                    break;
                }
                if is_cache_window_frame {
                    completed_iter = false;
                    break;
                }
                if should_stop_animation_frame_cache_miss_after_request(
                    index,
                    cache_window_end,
                    delivered_prefetch_end,
                    can_cache_frames,
                    can_deliver_prefetched_frames,
                    known_last_frame_index,
                ) {
                    completed_iter = false;
                    break;
                }
                continue;
            }
        }

        if is_cache_window_frame {
            let did_push = frame_cache
                .as_mut()
                .is_some_and(|cache| cache.push(decoded_frame));
            if !did_push {
                if index < frame_index {
                    frame_cache = None;
                    can_cache_frames = false;
                    start_cache_at_requested = cache_source_key.is_some();
                    continue;
                }
                completed_iter = false;
                break;
            }
        }

        if (requested_frame.is_some() || requested_frame_delivered)
            && should_stop_animation_frame_cache_miss_after_request(
                index,
                cache_window_end,
                delivered_prefetch_end,
                can_cache_frames,
                can_deliver_prefetched_frames,
                known_last_frame_index,
            )
        {
            completed_iter = false;
            break;
        }
    }

    if completed_iter {
        if let Some(cache) = frame_cache.as_mut() {
            cache.set_frame_count(decoded_frame_count);
        }
    }

    if let (Some(cache), Some(reusable_cache)) = (frame_cache.as_mut(), reusable_cache.as_ref()) {
        reusable_cache.retain_frames_outside_contiguous_cache(cache);
    }

    if let Some(entry) = frame_cache.and_then(AnimationFrameCacheBuilder::finish) {
        replace_animation_frame_cache(Some(entry));
    }

    if requested_frame_delivered {
        return Ok(AnimationFrameRequestResult::Delivered);
    }

    let Some(requested_frame) = requested_frame else {
        return Err(LoadImageError::AnimationFrameUnavailable {
            path: path.to_path_buf(),
            frame_index,
        });
    };

    Ok(AnimationFrameRequestResult::Frame(requested_frame))
}

fn should_stop_animation_frame_cache_miss(
    index: usize,
    cache_window_end: usize,
    can_cache_frames: bool,
    known_last_frame_index: Option<usize>,
) -> bool {
    !can_cache_frames || (index >= cache_window_end && known_last_frame_index != Some(index))
}

fn should_stop_animation_frame_cache_miss_after_request(
    index: usize,
    cache_window_end: usize,
    delivered_prefetch_end: Option<usize>,
    can_cache_frames: bool,
    can_deliver_prefetched_frames: bool,
    known_last_frame_index: Option<usize>,
) -> bool {
    let reached_prefetch_end = delivered_prefetch_end
        .is_some_and(|end| index >= end && known_last_frame_index != Some(index));
    if can_cache_frames {
        return should_stop_animation_frame_cache_miss(
            index,
            cache_window_end,
            can_cache_frames,
            known_last_frame_index,
        ) || reached_prefetch_end;
    }

    if can_deliver_prefetched_frames {
        return delivered_prefetch_end
            .is_none_or(|end| index >= end && known_last_frame_index != Some(index));
    }

    true
}

fn animation_frame_to_rgba8(
    path: &Path,
    frame: image::Frame,
    target_size: ImageSize,
    buffer_kind: ImageBufferKind,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let rgba8 = frame.into_buffer();
    let (width, height) = rgba8.dimensions();
    let rgba8 = Rgba8Image::new(width, height, rgba8.into_raw());

    if buffer_kind == ImageBufferKind::Preview {
        downscale_rgba8(path, rgba8, target_size, cancel)
    } else {
        Ok(rgba8)
    }
}

fn animation_buffer_kind(source_size: ImageSize, policy: ImageMemoryPolicy) -> ImageBufferKind {
    if is_large_image(source_size, policy) {
        ImageBufferKind::Preview
    } else {
        ImageBufferKind::FullResolution
    }
}

fn animation_target_size(
    source_size: ImageSize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
    buffer_kind: ImageBufferKind,
) -> ImageSize {
    if buffer_kind == ImageBufferKind::Preview {
        preview_size_for_viewport(source_size, viewport, policy)
    } else {
        source_size
    }
}

fn animation_loop_policy_from_image(loop_count: ImageLoopCount) -> AnimationLoopPolicy {
    match loop_count {
        ImageLoopCount::Infinite => AnimationLoopPolicy::Infinite,
        ImageLoopCount::Finite(repeat_count) => AnimationLoopPolicy::finite(repeat_count.get()),
    }
}

fn animation_delay_ms(delay: image::Delay, timing: AnimationTimingSettings) -> u32 {
    let (numerator, denominator) = delay.numer_denom_ms();
    let raw_delay_ms = if denominator == 0 {
        0
    } else {
        let numerator = u64::from(numerator);
        let denominator = u64::from(denominator);
        numerator
            .saturating_add(denominator.saturating_sub(1))
            .saturating_div(denominator)
            .min(u64::from(u32::MAX)) as u32
    };
    normalize_animation_delay_ms(raw_delay_ms, timing)
}

fn normalize_animation_delay_ms(raw_delay_ms: u32, timing: AnimationTimingSettings) -> u32 {
    timing.normalize_frame_delay_ms(raw_delay_ms)
}

fn read_animation_metadata_for_format(
    path: &Path,
    format: SupportedImageFormat,
    reader: &mut (impl Read + Seek),
    context: AnimationDecodeContext<'_>,
) -> Result<Option<AnimationMetadata>, LoadImageError> {
    match format {
        SupportedImageFormat::Gif => read_gif_animation_metadata(path, reader, context),
        SupportedImageFormat::Webp => read_webp_animation_metadata(path, reader, context),
        SupportedImageFormat::Jpeg
        | SupportedImageFormat::Png
        | SupportedImageFormat::Bmp
        | SupportedImageFormat::Ico
        | SupportedImageFormat::Tiff
        | SupportedImageFormat::Tga => Ok(None),
    }
}

fn read_animation_metadata_and_replay<R>(
    path: &Path,
    format: SupportedImageFormat,
    reader: R,
    context: AnimationDecodeContext<'_>,
) -> Result<
    (
        Option<AnimationMetadata>,
        BufReader<AnimationMetadataReplayReader<R>>,
    ),
    LoadImageError,
>
where
    R: Read + Seek,
{
    let mut reader = AnimationMetadataRecordingReader::new(reader);
    let metadata = read_animation_metadata_for_format(path, format, &mut reader, context)?;
    check_canceled(path, context.cancel)?;
    Ok((metadata, BufReader::new(reader.into_replay_reader())))
}

fn read_gif_animation_metadata(
    path: &Path,
    reader: &mut (impl Read + Seek),
    context: AnimationDecodeContext<'_>,
) -> Result<Option<AnimationMetadata>, LoadImageError> {
    let mut header = [0u8; 13];
    if !read_exact_animation_metadata(path, reader, &mut header, context.cancel)? {
        return Ok(None);
    }
    if &header[0..3] != b"GIF" {
        return Ok(None);
    }

    let global_color_table_size = gif_color_table_byte_len(header[10]);
    if global_color_table_size > 0
        && !skip_animation_metadata_bytes(
            path,
            reader,
            global_color_table_size as u64,
            context.cancel,
        )?
    {
        return Ok(None);
    }

    let mut frame_delays_ms = Vec::new();
    let mut pending_delay_ms = 0u32;
    loop {
        let Some(block_id) = read_animation_metadata_byte(path, reader, context.cancel)? else {
            return Ok(None);
        };
        match block_id {
            0x3B => break,
            0x21 => {
                let Some(label) = read_animation_metadata_byte(path, reader, context.cancel)?
                else {
                    return Ok(None);
                };
                if label == 0xF9 {
                    let Some(block_size) =
                        read_animation_metadata_byte(path, reader, context.cancel)?
                    else {
                        return Ok(None);
                    };
                    if block_size != 4 {
                        return Ok(None);
                    }
                    let mut control = [0u8; 4];
                    if !read_exact_animation_metadata(path, reader, &mut control, context.cancel)? {
                        return Ok(None);
                    }
                    let Some(terminator) =
                        read_animation_metadata_byte(path, reader, context.cancel)?
                    else {
                        return Ok(None);
                    };
                    if terminator != 0 {
                        return Ok(None);
                    }
                    let delay_cs = u16::from_le_bytes([control[1], control[2]]);
                    pending_delay_ms = u32::from(delay_cs).saturating_mul(10);
                } else if !skip_gif_sub_blocks(path, reader, context.cancel)? {
                    return Ok(None);
                }
            }
            0x2C => {
                let mut descriptor = [0u8; 9];
                if !read_exact_animation_metadata(path, reader, &mut descriptor, context.cancel)? {
                    return Ok(None);
                }
                let local_color_table_size = gif_color_table_byte_len(descriptor[8]);
                if local_color_table_size > 0
                    && !skip_animation_metadata_bytes(
                        path,
                        reader,
                        local_color_table_size as u64,
                        context.cancel,
                    )?
                {
                    return Ok(None);
                }
                if read_animation_metadata_byte(path, reader, context.cancel)?.is_none() {
                    return Ok(None);
                }
                if !skip_gif_sub_blocks(path, reader, context.cancel)? {
                    return Ok(None);
                }
                push_animation_metadata_delay(
                    path,
                    &mut frame_delays_ms,
                    pending_delay_ms,
                    context,
                )?;
                pending_delay_ms = 0;
            }
            _ => return Ok(None),
        }
    }

    if frame_delays_ms.is_empty() {
        Ok(None)
    } else {
        Ok(Some(AnimationMetadata { frame_delays_ms }))
    }
}

fn read_webp_animation_metadata(
    path: &Path,
    reader: &mut (impl Read + Seek),
    context: AnimationDecodeContext<'_>,
) -> Result<Option<AnimationMetadata>, LoadImageError> {
    let mut header = [0u8; 12];
    if !read_exact_animation_metadata(path, reader, &mut header, context.cancel)? {
        return Ok(None);
    }
    if &header[0..4] != b"RIFF" || &header[8..12] != b"WEBP" {
        return Ok(None);
    }
    let riff_size = u32::from_le_bytes([header[4], header[5], header[6], header[7]]) as u64;
    if riff_size < 4 {
        return Ok(None);
    }

    let mut frame_delays_ms = Vec::new();
    let mut remaining_bytes = riff_size.saturating_sub(4);
    while remaining_bytes > 0 {
        if remaining_bytes < 8 {
            return Ok(None);
        }
        let mut chunk_header = [0u8; 8];
        if !read_exact_animation_metadata(path, reader, &mut chunk_header, context.cancel)? {
            return Ok(None);
        }
        remaining_bytes = remaining_bytes.saturating_sub(8);
        let chunk_len = u32::from_le_bytes([
            chunk_header[4],
            chunk_header[5],
            chunk_header[6],
            chunk_header[7],
        ]) as u64;
        let padded_chunk_len = chunk_len + (chunk_len % 2);
        if padded_chunk_len > remaining_bytes {
            return Ok(None);
        }
        if &chunk_header[0..4] == b"ANMF" {
            if chunk_len < 16 {
                return Ok(None);
            }
            let mut frame_header = [0u8; 16];
            if !read_exact_animation_metadata(path, reader, &mut frame_header, context.cancel)? {
                return Ok(None);
            }
            let duration_ms = u32::from(frame_header[12])
                | (u32::from(frame_header[13]) << 8)
                | (u32::from(frame_header[14]) << 16);
            push_animation_metadata_delay(path, &mut frame_delays_ms, duration_ms, context)?;
            if !skip_animation_metadata_bytes(
                path,
                reader,
                padded_chunk_len.saturating_sub(16),
                context.cancel,
            )? {
                return Ok(None);
            }
        } else if !skip_animation_metadata_bytes(path, reader, padded_chunk_len, context.cancel)? {
            return Ok(None);
        }
        remaining_bytes = remaining_bytes.saturating_sub(padded_chunk_len);
    }

    if frame_delays_ms.is_empty() {
        Ok(None)
    } else {
        Ok(Some(AnimationMetadata { frame_delays_ms }))
    }
}

fn push_animation_metadata_delay(
    path: &Path,
    frame_delays_ms: &mut Vec<u32>,
    raw_delay_ms: u32,
    context: AnimationDecodeContext<'_>,
) -> Result<(), LoadImageError> {
    if frame_delays_ms.len() >= context.policy.max_animation_metadata_frames() {
        return Err(LoadImageError::AnimationTooManyFrames {
            path: path.to_path_buf(),
            frame_count: frame_delays_ms.len().saturating_add(1),
            max_frame_count: context.policy.max_animation_metadata_frames(),
        });
    }
    frame_delays_ms.push(normalize_animation_delay_ms(
        raw_delay_ms,
        context.animation_timing,
    ));
    Ok(())
}

fn gif_color_table_byte_len(packed: u8) -> usize {
    if packed & 0x80 == 0 {
        0
    } else {
        3usize * (1usize << (usize::from(packed & 0x07) + 1))
    }
}

fn skip_gif_sub_blocks<R: Read + Seek>(
    path: &Path,
    reader: &mut R,
    cancel: Option<&AtomicBool>,
) -> Result<bool, LoadImageError> {
    loop {
        let Some(block_size) = read_animation_metadata_byte(path, reader, cancel)? else {
            return Ok(false);
        };
        if block_size == 0 {
            return Ok(true);
        }
        if !skip_animation_metadata_bytes(path, reader, u64::from(block_size), cancel)? {
            return Ok(false);
        }
    }
}

fn read_animation_metadata_byte<R: Read>(
    path: &Path,
    reader: &mut R,
    cancel: Option<&AtomicBool>,
) -> Result<Option<u8>, LoadImageError> {
    let mut byte = [0u8; 1];
    if read_exact_animation_metadata(path, reader, &mut byte, cancel)? {
        Ok(Some(byte[0]))
    } else {
        Ok(None)
    }
}

fn read_exact_animation_metadata<R: Read>(
    path: &Path,
    reader: &mut R,
    buffer: &mut [u8],
    cancel: Option<&AtomicBool>,
) -> Result<bool, LoadImageError> {
    match reader.read_exact(buffer) {
        Ok(()) => Ok(true),
        Err(_) => {
            check_canceled(path, cancel)?;
            Ok(false)
        }
    }
}

fn skip_animation_metadata_bytes<R: Seek>(
    path: &Path,
    reader: &mut R,
    byte_count: u64,
    cancel: Option<&AtomicBool>,
) -> Result<bool, LoadImageError> {
    check_canceled(path, cancel)?;
    let Ok(offset) = i64::try_from(byte_count) else {
        return Ok(false);
    };
    match reader.seek(SeekFrom::Current(offset)) {
        Ok(_) => {
            check_canceled(path, cancel)?;
            Ok(true)
        }
        Err(_) => {
            check_canceled(path, cancel)?;
            Ok(false)
        }
    }
}

// Cache only bytes the metadata parser actually reads; forward-seeked payload
// ranges are left on disk for the image decoder to read once during replay.
struct AnimationMetadataRecordingReader<R> {
    inner: R,
    position: u64,
    cached_segments: Vec<AnimationMetadataCachedSegment>,
}

impl<R> AnimationMetadataRecordingReader<R> {
    fn new(inner: R) -> Self {
        Self {
            inner,
            position: 0,
            cached_segments: Vec::new(),
        }
    }

    fn into_replay_reader(self) -> AnimationMetadataReplayReader<R> {
        AnimationMetadataReplayReader {
            inner: self.inner,
            position: 0,
            cached_segments: self.cached_segments,
        }
    }

    fn push_cached_bytes(&mut self, start: u64, bytes: &[u8]) {
        if bytes.is_empty() {
            return;
        }

        if let Some(last) = self.cached_segments.last_mut() {
            if last.end() == start {
                last.bytes.extend_from_slice(bytes);
                return;
            }
            if last.end() > start {
                return;
            }
        }

        self.cached_segments.push(AnimationMetadataCachedSegment {
            start,
            bytes: bytes.to_vec(),
        });
    }
}

impl<R: Read> Read for AnimationMetadataRecordingReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        let bytes_read = self.inner.read(buffer)?;
        if bytes_read > 0 {
            let start = self.position;
            self.position = self.position.saturating_add(bytes_read as u64);
            self.push_cached_bytes(start, &buffer[..bytes_read]);
        }
        Ok(bytes_read)
    }
}

impl<R: Seek> Seek for AnimationMetadataRecordingReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = self.inner.seek(position)?;
        self.position = position;
        Ok(position)
    }
}

struct AnimationMetadataReplayReader<R> {
    inner: R,
    position: u64,
    cached_segments: Vec<AnimationMetadataCachedSegment>,
}

impl<R> AnimationMetadataReplayReader<R> {
    fn cached_segment_at(&self, position: u64) -> Option<&AnimationMetadataCachedSegment> {
        let index = self.cached_segment_partition_point(position);
        if index == 0 {
            return None;
        }

        let segment = &self.cached_segments[index - 1];
        if position < segment.end() {
            Some(segment)
        } else {
            None
        }
    }

    fn next_cached_segment_start(&self, position: u64) -> Option<u64> {
        let index = self.cached_segment_partition_point(position);
        self.cached_segments.get(index).map(|segment| segment.start)
    }

    fn cached_segment_partition_point(&self, position: u64) -> usize {
        let mut left = 0;
        let mut right = self.cached_segments.len();
        while left < right {
            let middle = left + (right - left) / 2;
            if self.cached_segments[middle].start <= position {
                left = middle + 1;
            } else {
                right = middle;
            }
        }
        left
    }
}

impl<R: Read + Seek> Read for AnimationMetadataReplayReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if buffer.is_empty() {
            return Ok(0);
        }

        if let Some(segment) = self.cached_segment_at(self.position) {
            let offset = (self.position - segment.start) as usize;
            let bytes_available = segment.bytes.len().saturating_sub(offset);
            let bytes_to_copy = bytes_available.min(buffer.len());
            buffer[..bytes_to_copy].copy_from_slice(&segment.bytes[offset..offset + bytes_to_copy]);
            self.position = self.position.saturating_add(bytes_to_copy as u64);
            return Ok(bytes_to_copy);
        }

        let read_len = self
            .next_cached_segment_start(self.position)
            .and_then(|next_start| usize::try_from(next_start.saturating_sub(self.position)).ok())
            .map_or(buffer.len(), |len| len.min(buffer.len()));
        if read_len == 0 {
            return Ok(0);
        }

        self.inner.seek(SeekFrom::Start(self.position))?;
        let bytes_read = self.inner.read(&mut buffer[..read_len])?;
        self.position = self.position.saturating_add(bytes_read as u64);
        Ok(bytes_read)
    }
}

impl<R: Seek> Seek for AnimationMetadataReplayReader<R> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        let position = match position {
            SeekFrom::Start(position) => position,
            SeekFrom::Current(offset) => animation_metadata_relative_seek(self.position, offset)?,
            SeekFrom::End(offset) => self.inner.seek(SeekFrom::End(offset))?,
        };
        self.position = position;
        Ok(position)
    }
}

struct AnimationMetadataCachedSegment {
    start: u64,
    bytes: Vec<u8>,
}

impl AnimationMetadataCachedSegment {
    fn end(&self) -> u64 {
        self.start.saturating_add(self.bytes.len() as u64)
    }
}

fn animation_metadata_relative_seek(position: u64, offset: i64) -> io::Result<u64> {
    let position = if offset >= 0 {
        position.checked_add(offset as u64)
    } else {
        let distance = offset
            .checked_abs()
            .map_or(1u64 << 63, |value| value as u64);
        position.checked_sub(distance)
    };
    position.ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid seek"))
}

struct CancelableFileSource<'a> {
    file: Arc<File>,
    cancel: Option<&'a AtomicBool>,
}

impl<'a> CancelableFileSource<'a> {
    fn new(file: File, cancel: Option<&'a AtomicBool>) -> Self {
        Self {
            file: Arc::new(file),
            cancel,
        }
    }

    fn reader(&self) -> BufReader<CancelableFile<'a>> {
        BufReader::new(CancelableFile {
            file: Arc::clone(&self.file),
            position: 0,
            cancel: self.cancel,
        })
    }
}

struct CancelableFile<'a> {
    file: Arc<File>,
    position: u64,
    cancel: Option<&'a AtomicBool>,
}

impl CancelableFile<'_> {
    fn check_canceled(&self) -> io::Result<()> {
        if self
            .cancel
            .is_some_and(|cancel| cancel.load(Ordering::Acquire))
        {
            Err(io::Error::new(
                io::ErrorKind::Interrupted,
                "image decode canceled",
            ))
        } else {
            Ok(())
        }
    }
}

impl Read for CancelableFile<'_> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        self.check_canceled()?;
        let bytes_read = read_file_at(self.file.as_ref(), buffer, self.position)?;
        let next_position = self
            .position
            .checked_add(bytes_read as u64)
            .ok_or_else(|| io::Error::new(io::ErrorKind::InvalidInput, "invalid file position"))?;
        self.position = next_position;
        self.check_canceled()?;
        Ok(bytes_read)
    }
}

impl Seek for CancelableFile<'_> {
    fn seek(&mut self, position: SeekFrom) -> io::Result<u64> {
        self.check_canceled()?;
        let offset = match position {
            SeekFrom::Start(position) => position,
            SeekFrom::Current(offset) => animation_metadata_relative_seek(self.position, offset)?,
            SeekFrom::End(offset) => {
                let file_len = self.file.metadata()?.len();
                animation_metadata_relative_seek(file_len, offset)?
            }
        };
        self.position = offset;
        self.check_canceled()?;
        Ok(offset)
    }
}

#[cfg(unix)]
fn read_file_at(file: &File, buffer: &mut [u8], position: u64) -> io::Result<usize> {
    use std::os::unix::fs::FileExt;

    file.read_at(buffer, position)
}

#[cfg(windows)]
fn read_file_at(file: &File, buffer: &mut [u8], position: u64) -> io::Result<usize> {
    use std::os::windows::fs::FileExt;

    file.seek_read(buffer, position)
}

#[cfg(not(any(unix, windows)))]
fn read_file_at(file: &File, buffer: &mut [u8], position: u64) -> io::Result<usize> {
    let mut file = file.try_clone()?;
    file.seek(SeekFrom::Start(position))?;
    file.read(buffer)
}

fn open_cancelable_buffered_file<'a>(
    path: &Path,
    cancel: Option<&'a AtomicBool>,
) -> Result<BufReader<CancelableFile<'a>>, LoadImageError> {
    let (_, reader) = open_cancelable_buffered_file_with_metadata(path, cancel)?;
    Ok(reader)
}

fn open_cancelable_buffered_file_with_metadata<'a>(
    path: &Path,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, BufReader<CancelableFile<'a>>), LoadImageError> {
    open_cancelable_buffered_file_with_optional_metadata(path, None, cancel)
}

fn open_cancelable_buffered_file_with_optional_metadata<'a>(
    path: &Path,
    known_metadata: Option<fs::Metadata>,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, BufReader<CancelableFile<'a>>), LoadImageError> {
    let (metadata, source) =
        open_cancelable_file_source_with_optional_metadata(path, known_metadata, cancel)?;
    Ok((metadata, source.reader()))
}

fn open_cancelable_file_source_with_metadata<'a>(
    path: &Path,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, CancelableFileSource<'a>), LoadImageError> {
    open_cancelable_file_source_with_optional_metadata(path, None, cancel)
}

fn open_cancelable_file_source_with_optional_metadata<'a>(
    path: &Path,
    known_metadata: Option<fs::Metadata>,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, CancelableFileSource<'a>), LoadImageError> {
    let file = File::open(path).map_err(|source| LoadImageError::FileAccess {
        path: path.to_path_buf(),
        source,
    })?;
    let metadata = match known_metadata {
        Some(metadata) => require_file_metadata(path, metadata)?,
        None => {
            let metadata = file
                .metadata()
                .map_err(|source| LoadImageError::FileAccess {
                    path: path.to_path_buf(),
                    source,
                })?;
            require_file_metadata(path, metadata)?
        }
    };
    Ok((metadata, CancelableFileSource::new(file, cancel)))
}

struct ImageProbe {
    source_size: ImageSize,
    exif_orientation: ImageOrientation,
}

fn open_image_decoder<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    cancel: Option<&'a AtomicBool>,
) -> Result<impl ImageDecoder + 'a, LoadImageError> {
    let (_, decoder) = open_image_decoder_with_metadata(path, format, limits, cancel)?;
    Ok(decoder)
}

fn open_image_decoder_with_metadata<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, impl ImageDecoder + 'a), LoadImageError> {
    let (metadata, reader) = open_cancelable_buffered_file_with_metadata(path, cancel)?;
    open_image_decoder_from_reader_with_metadata(path, format, limits, metadata, reader, cancel)
}

fn open_image_decoder_with_metadata_and_source<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    cancel: Option<&'a AtomicBool>,
) -> Result<
    (
        fs::Metadata,
        impl ImageDecoder + 'a,
        CancelableFileSource<'a>,
    ),
    LoadImageError,
> {
    let (metadata, source) = open_cancelable_file_source_with_metadata(path, cancel)?;
    let decoder = open_image_decoder_from_reader(path, format, limits, source.reader(), cancel)?;
    Ok((metadata, decoder, source))
}

fn open_image_decoder_with_known_metadata<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    known_metadata: fs::Metadata,
    cancel: Option<&'a AtomicBool>,
) -> Result<(fs::Metadata, impl ImageDecoder + 'a), LoadImageError> {
    let (metadata, reader) =
        open_cancelable_buffered_file_with_optional_metadata(path, Some(known_metadata), cancel)?;
    open_image_decoder_from_reader_with_metadata(path, format, limits, metadata, reader, cancel)
}

fn open_image_decoder_from_reader_with_metadata<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    metadata: fs::Metadata,
    reader: BufReader<CancelableFile<'a>>,
    cancel: Option<&AtomicBool>,
) -> Result<(fs::Metadata, impl ImageDecoder + 'a), LoadImageError> {
    let decoder = open_image_decoder_from_reader(path, format, limits, reader, cancel)?;
    Ok((metadata, decoder))
}

fn open_image_decoder_from_reader<'a>(
    path: &Path,
    format: SupportedImageFormat,
    limits: Limits,
    reader: BufReader<CancelableFile<'a>>,
    cancel: Option<&AtomicBool>,
) -> Result<impl ImageDecoder + 'a, LoadImageError> {
    let mut reader = ImageReader::with_format(reader, image_format_for_decode(format));
    reader.limits(limits);
    let decoder = reader
        .into_decoder()
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    Ok(decoder)
}

fn set_decoder_limits(
    path: &Path,
    decoder: &mut impl ImageDecoder,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<(), LoadImageError> {
    decoder
        .set_limits(decode_limits(max_alloc_bytes))
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))
}

fn read_image_probe_from_decoder(
    path: &Path,
    decoder: &mut impl ImageDecoder,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
) -> Result<ImageProbe, LoadImageError> {
    let source_size = image_decoder_size(&*decoder);
    check_canceled(path, cancel)?;
    if source_size.is_empty() || is_image_too_large(source_size, policy) {
        return Err(LoadImageError::ImageTooLarge {
            path: path.to_path_buf(),
            size: source_size,
            source: None,
        });
    }

    let exif_orientation = read_decoder_exif_orientation(path, decoder, cancel)?;

    Ok(ImageProbe {
        source_size,
        exif_orientation,
    })
}

fn image_decoder_size(decoder: &impl ImageDecoder) -> ImageSize {
    let (width, height) = decoder.dimensions();
    ImageSize::new(width, height)
}

fn read_image_dimensions(
    path: &Path,
    format: SupportedImageFormat,
    cancel: Option<&AtomicBool>,
) -> Result<ImageSize, LoadImageError> {
    let decoder = open_image_decoder(path, format, Limits::default(), cancel)?;
    let size = image_decoder_size(&decoder);
    check_canceled(path, cancel)?;
    Ok(size)
}

fn read_decoder_exif_orientation(
    path: &Path,
    decoder: &mut impl ImageDecoder,
    cancel: Option<&AtomicBool>,
) -> Result<ImageOrientation, LoadImageError> {
    check_canceled(path, cancel)?;
    let orientation = match decoder.orientation() {
        Ok(orientation) => orientation,
        Err(source) => {
            let error = image_decode_error_or_canceled(path, source, cancel);
            return if error.is_canceled() {
                Err(error)
            } else {
                Ok(ImageOrientation::NORMAL)
            };
        }
    };
    check_canceled(path, cancel)?;

    let Some(orientation) = ImageOrientation::from_exif_value(orientation.to_exif()) else {
        return Ok(ImageOrientation::NORMAL);
    };
    Ok(orientation)
}

#[cfg(test)]
fn decode_rgba8_image(
    path: &Path,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded = decode_dynamic_image(path, max_alloc_bytes, cancel)?;
    Ok(dynamic_image_into_rgba8(decoded))
}

fn decode_rgba8_image_from_decoder(
    path: &Path,
    decoder: impl ImageDecoder,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded = decode_dynamic_image_from_decoder(path, decoder, max_alloc_bytes, cancel)?;
    Ok(dynamic_image_into_rgba8(decoded))
}

fn decode_pixel_image_from_decoder(
    path: &Path,
    decoder: impl ImageDecoder,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<PixelImage, LoadImageError> {
    let decoded = decode_dynamic_image_from_decoder(path, decoder, max_alloc_bytes, cancel)?;
    Ok(dynamic_image_into_pixel_image(decoded))
}

#[cfg(test)]
fn decode_preview_rgba8_image(
    path: &Path,
    max_alloc_bytes: usize,
    target_size: ImageSize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded = decode_dynamic_image(path, max_alloc_bytes, cancel)?;
    dynamic_image_into_preview_rgba8(path, decoded, target_size, cancel)
}

fn decode_preview_rgba8_image_from_decoder(
    path: &Path,
    decoder: impl ImageDecoder,
    max_alloc_bytes: usize,
    target_size: ImageSize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded = decode_dynamic_image_from_decoder(path, decoder, max_alloc_bytes, cancel)?;
    dynamic_image_into_preview_rgba8(path, decoded, target_size, cancel)
}

fn decode_static_preview_pixel_image_from_decoder(
    path: &Path,
    format: SupportedImageFormat,
    source_key: Option<&StaticImageSourceCacheKey>,
    decoder: impl ImageDecoder,
    preview_source: &CancelableFileSource<'_>,
    policy: ImageMemoryPolicy,
    target_size: ImageSize,
    source_size: ImageSize,
    allow_full_fallback: bool,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
    static_cache_mode: StaticFullResolutionCacheMode,
) -> Result<PixelImage, LoadImageError> {
    if format != SupportedImageFormat::Jpeg {
        return decode_static_preview_fallback_pixel_image_from_decoder(
            path,
            format,
            source_key,
            decoder,
            preview_source,
            policy,
            target_size,
            source_size,
            allow_full_fallback,
            cancel,
            profiler,
            static_cache_mode,
            true,
        );
    }

    let preview_max_alloc_bytes = static_preview_decode_max_alloc_bytes(path, policy, target_size)?;
    if let Some(preview) = decode_scaled_static_jpeg_preview_pixel_image(
        path,
        preview_source.reader(),
        preview_max_alloc_bytes,
        target_size,
        allow_full_fallback,
        cancel,
        profiler,
    )? {
        replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
        return Ok(preview);
    }

    if allow_full_fallback {
        let decoded = decode_dynamic_image_from_decoder(
            path,
            decoder,
            policy.max_transient_decode_bytes(),
            cancel,
        )?;
        return dynamic_image_into_preview_pixel_image_and_cache_full_resolution(
            path,
            decoded,
            target_size,
            source_key,
            source_size,
            policy,
            cancel,
            static_cache_mode,
        );
    }

    decode_static_preview_fallback_pixel_image_from_decoder(
        path,
        format,
        source_key,
        decoder,
        preview_source,
        policy,
        target_size,
        source_size,
        allow_full_fallback,
        cancel,
        profiler,
        static_cache_mode,
        false,
    )
}

fn decode_static_preview_fallback_pixel_image_from_decoder(
    path: &Path,
    format: SupportedImageFormat,
    source_key: Option<&StaticImageSourceCacheKey>,
    decoder: impl ImageDecoder,
    preview_source: &CancelableFileSource<'_>,
    policy: ImageMemoryPolicy,
    target_size: ImageSize,
    source_size: ImageSize,
    allow_full_fallback: bool,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
    static_cache_mode: StaticFullResolutionCacheMode,
    allow_sampled_rgb8_output: bool,
) -> Result<PixelImage, LoadImageError> {
    if format == SupportedImageFormat::Bmp {
        if let Some(preview) = decode_bmp_preview_rgba8_image(
            path,
            source_size,
            target_size,
            policy.max_transient_decode_bytes(),
            cancel,
        )? {
            replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
            return Ok(PixelImage::from(preview));
        }
    }

    let preview_max_alloc_bytes = if matches!(
        format,
        SupportedImageFormat::Jpeg | SupportedImageFormat::Png | SupportedImageFormat::Webp
    ) {
        Some(static_preview_decode_max_alloc_bytes(
            path,
            policy,
            target_size,
        )?)
    } else {
        None
    };

    if format == SupportedImageFormat::Jpeg {
        if let Some(preview_max_alloc_bytes) = preview_max_alloc_bytes {
            if let Some(preview) = decode_scaled_static_jpeg_preview_rgba8_image(
                path,
                preview_source.reader(),
                preview_max_alloc_bytes,
                target_size,
                allow_full_fallback,
                cancel,
                profiler,
            )? {
                replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
                return Ok(PixelImage::from(preview));
            }
            if allow_full_fallback {
                let decoded = decode_dynamic_image_from_decoder(
                    path,
                    decoder,
                    policy.max_transient_decode_bytes(),
                    cancel,
                )?;
                return dynamic_image_into_preview_rgba8_and_cache_full_resolution(
                    path,
                    decoded,
                    target_size,
                    source_key,
                    source_size,
                    policy,
                    cancel,
                    static_cache_mode,
                )
                .map(PixelImage::from);
            }
        }
    }

    if format == SupportedImageFormat::Png {
        if let Some(preview_max_alloc_bytes) = preview_max_alloc_bytes {
            if let Some(preview) = decode_sampled_static_png_preview_pixel_image(
                path,
                preview_source.reader(),
                preview_max_alloc_bytes,
                policy.max_transient_decode_bytes(),
                target_size,
                source_size,
                allow_sampled_rgb8_output,
                cancel,
            )? {
                replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
                return Ok(preview);
            }
        }
    }

    if matches!(
        format,
        SupportedImageFormat::Jpeg | SupportedImageFormat::Png | SupportedImageFormat::Webp
    ) {
        if let Some(bytes_per_pixel) = sampled_static_preview_color_type(decoder.color_type()) {
            // Generic ImageDecoder::read_image needs a complete decoded buffer. Keep this
            // fallback tied to the preview size so large sources fail before materializing full pixels.
            let source_max_alloc_bytes =
                preview_max_alloc_bytes.unwrap_or_else(|| policy.max_transient_decode_bytes());
            let preview = decode_sampled_static_preview_pixel_image_from_decoder(
                path,
                decoder,
                bytes_per_pixel,
                source_max_alloc_bytes,
                source_max_alloc_bytes,
                target_size,
                source_size,
                allow_sampled_rgb8_output,
                cancel,
            )?;
            replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
            return Ok(preview);
        }
    }

    let fallback_max_alloc_bytes =
        preview_max_alloc_bytes.unwrap_or_else(|| policy.max_transient_decode_bytes());
    if preview_max_alloc_bytes.is_some() {
        static_preview_source_byte_len(
            path,
            decoder.total_bytes(),
            fallback_max_alloc_bytes,
            source_size,
        )?;
    }
    let decoded =
        decode_dynamic_image_from_decoder(path, decoder, fallback_max_alloc_bytes, cancel)?;
    let preview = dynamic_image_into_preview_rgba8_and_cache_full_resolution(
        path,
        decoded,
        target_size,
        source_key,
        source_size,
        policy,
        cancel,
        static_cache_mode,
    )?;
    Ok(PixelImage::from(preview))
}

fn dynamic_image_into_preview_rgba8_and_cache_full_resolution(
    path: &Path,
    decoded: image::DynamicImage,
    target_size: ImageSize,
    source_key: Option<&StaticImageSourceCacheKey>,
    source_size: ImageSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
    static_cache_mode: StaticFullResolutionCacheMode,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded_size = ImageSize::new(decoded.width(), decoded.height());

    if decoded_size == target_size {
        replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
        return Ok(dynamic_image_into_rgba8(decoded));
    }

    check_canceled(path, cancel)?;
    let preview = resize(
        &decoded,
        target_size.width(),
        target_size.height(),
        FilterType::Triangle,
    );
    check_canceled(path, cancel)?;

    replace_static_full_resolution_cache_for_mode(
        static_cache_mode,
        static_full_resolution_cache_entry(
            source_key,
            source_size,
            decoded,
            preview.as_raw().len(),
            policy,
        ),
    );

    Ok(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        preview.into_raw(),
    ))
}

fn dynamic_image_into_preview_pixel_image_and_cache_full_resolution(
    path: &Path,
    decoded: image::DynamicImage,
    target_size: ImageSize,
    source_key: Option<&StaticImageSourceCacheKey>,
    source_size: ImageSize,
    policy: ImageMemoryPolicy,
    cancel: Option<&AtomicBool>,
    static_cache_mode: StaticFullResolutionCacheMode,
) -> Result<PixelImage, LoadImageError> {
    let decoded_size = ImageSize::new(decoded.width(), decoded.height());

    if decoded_size == target_size {
        replace_static_full_resolution_cache_for_mode(static_cache_mode, None);
        return Ok(dynamic_image_into_pixel_image(decoded));
    }

    check_canceled(path, cancel)?;
    let preview = resize(
        &decoded,
        target_size.width(),
        target_size.height(),
        FilterType::Triangle,
    );
    check_canceled(path, cancel)?;

    replace_static_full_resolution_cache_for_mode(
        static_cache_mode,
        static_full_resolution_cache_entry(
            source_key,
            source_size,
            decoded,
            preview.as_raw().len(),
            policy,
        ),
    );

    Ok(PixelImage::from(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        preview.into_raw(),
    )))
}

fn static_full_resolution_cache_entry(
    source_key: Option<&StaticImageSourceCacheKey>,
    source_size: ImageSize,
    decoded: image::DynamicImage,
    resident_base_byte_len: usize,
    policy: ImageMemoryPolicy,
) -> Option<StaticFullResolutionCacheEntry> {
    let source_key = source_key?;
    if source_size.is_empty() || !should_retain_full_resolution(source_size, policy) {
        return None;
    }
    if !static_full_resolution_cache_fits_policy(
        decoded.as_bytes().len(),
        resident_base_byte_len,
        policy,
    ) {
        return None;
    }

    Some(StaticFullResolutionCacheEntry {
        source_key: source_key.clone(),
        source_size,
        decoded,
    })
}

fn dynamic_image_into_preview_rgba8(
    path: &Path,
    decoded: image::DynamicImage,
    target_size: ImageSize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let decoded_size = ImageSize::new(decoded.width(), decoded.height());

    if decoded_size == target_size {
        return Ok(dynamic_image_into_rgba8(decoded));
    }

    check_canceled(path, cancel)?;
    let preview = resize(
        &decoded,
        target_size.width(),
        target_size.height(),
        FilterType::Triangle,
    );
    check_canceled(path, cancel)?;
    drop(decoded);

    Ok(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        preview.into_raw(),
    ))
}

fn decode_sampled_static_preview_pixel_image_from_decoder(
    path: &Path,
    mut decoder: impl ImageDecoder,
    bytes_per_pixel: usize,
    preview_max_alloc_bytes: usize,
    source_max_alloc_bytes: usize,
    target_size: ImageSize,
    source_size: ImageSize,
    allow_sampled_rgb8_output: bool,
    cancel: Option<&AtomicBool>,
) -> Result<PixelImage, LoadImageError> {
    let color_type = decoder.color_type();
    check_canceled(path, cancel)?;
    let use_rgb8_output =
        sampled_static_preview_uses_rgb8_output(color_type, allow_sampled_rgb8_output);
    let target_byte_len =
        sampled_static_preview_target_byte_len(path, target_size, use_rgb8_output)?;
    if target_byte_len > preview_max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, target_size));
    }

    let source_max_alloc_bytes = static_preview_generic_source_max_alloc_bytes(
        path,
        source_max_alloc_bytes,
        target_byte_len,
        source_size,
    )?;
    let max_alloc_bytes = preview_max_alloc_bytes.min(source_max_alloc_bytes);
    set_decoder_limits(path, &mut decoder, max_alloc_bytes, cancel)?;

    let source_byte_len =
        static_preview_source_byte_len(path, decoder.total_bytes(), max_alloc_bytes, source_size)?;

    let mut source = Vec::new();
    source
        .try_reserve_exact(source_byte_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    source.resize(source_byte_len, 0);
    decoder
        .read_image(&mut source)
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    check_canceled(path, cancel)?;

    sampled_static_preview_pixel_image_from_decoded(
        path,
        source,
        color_type,
        bytes_per_pixel,
        source_size,
        target_size,
        target_byte_len,
        use_rgb8_output,
        cancel,
    )
}

fn static_preview_decode_max_alloc_bytes(
    path: &Path,
    policy: ImageMemoryPolicy,
    target_size: ImageSize,
) -> Result<usize, LoadImageError> {
    let target_byte_len = target_size
        .rgba8_byte_len()
        .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
    if target_byte_len > policy.max_transient_decode_bytes() {
        return Err(sampled_static_preview_too_large(path, target_size));
    }

    // Preview-only decode should scale with the requested preview, not the full-image limit.
    let preview_decode_bytes = target_byte_len
        .saturating_mul(STATIC_PREVIEW_SOURCE_DECODE_BYTE_MULTIPLIER)
        .max(STATIC_PREVIEW_MIN_SOURCE_DECODE_BYTES);
    Ok(policy
        .max_transient_decode_bytes()
        .min(preview_decode_bytes))
}

fn static_preview_source_byte_len(
    path: &Path,
    total_bytes: u64,
    max_alloc_bytes: usize,
    source_size: ImageSize,
) -> Result<usize, LoadImageError> {
    let source_byte_len = usize::try_from(total_bytes)
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    if source_byte_len > max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, source_size));
    }
    Ok(source_byte_len)
}

fn static_preview_generic_source_max_alloc_bytes(
    path: &Path,
    source_max_alloc_bytes: usize,
    target_byte_len: usize,
    source_size: ImageSize,
) -> Result<usize, LoadImageError> {
    let preview_scaled_source_bytes = target_byte_len
        .checked_mul(STATIC_PREVIEW_SOURCE_DECODE_BYTE_MULTIPLIER)
        .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
    Ok(source_max_alloc_bytes.min(preview_scaled_source_bytes))
}

fn decode_scaled_static_jpeg_preview_pixel_image(
    path: &Path,
    reader: impl Read,
    preview_max_alloc_bytes: usize,
    target_size: ImageSize,
    allow_full_fallback: bool,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
) -> Result<Option<PixelImage>, LoadImageError> {
    check_canceled(path, cancel)?;
    let target_byte_len = target_size
        .pixel_byte_len(PixelFormat::Rgb8)
        .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
    if target_byte_len > preview_max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, target_size));
    }

    let requested_width = u16::try_from(target_size.width().max(1))
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let requested_height = u16::try_from(target_size.height().max(1))
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let mut decoder = JpegPreviewDecoder::new(reader);
    decoder.set_max_decoding_buffer_size(preview_max_alloc_bytes);
    record_image_open_profile_stage(profiler, "static.jpeg_preview.open_file_decoder");
    let (scaled_width, scaled_height) = match decoder.scale(requested_width, requested_height) {
        Ok(size) => size,
        Err(source) => {
            if allow_full_fallback && jpeg_preview_error_can_full_fallback(&source) {
                return Ok(None);
            }
            return Err(jpeg_decode_error_or_canceled(path, source, cancel));
        }
    };
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.scale_decoder");

    let info = decoder
        .info()
        .ok_or_else(|| jpeg_preview_missing_info_error(path))?;
    let Some((color_type, bytes_per_pixel)) = jpeg_preview_pixel_format(info.pixel_format) else {
        return Ok(None);
    };
    let scaled_size = ImageSize::new(u32::from(scaled_width), u32::from(scaled_height));
    let source_byte_len =
        jpeg_preview_source_byte_len(path, scaled_size, bytes_per_pixel, preview_max_alloc_bytes)?;

    let source = match decoder.decode() {
        Ok(source) => source,
        Err(source) => {
            if allow_full_fallback && jpeg_preview_error_can_full_fallback(&source) {
                return Ok(None);
            }
            return Err(jpeg_decode_error_or_canceled(path, source, cancel));
        }
    };
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.decode_scaled_pixels");
    if source.len() != source_byte_len {
        return Err(LoadImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
        });
    }

    let preview = sampled_static_preview_rgb8_from_decoded(
        path,
        source,
        color_type,
        bytes_per_pixel,
        scaled_size,
        target_size,
        target_byte_len,
        cancel,
    )?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.sample_to_target");
    Ok(Some(PixelImage::from(preview)))
}

fn decode_scaled_static_jpeg_preview_rgba8_image(
    path: &Path,
    reader: impl Read,
    preview_max_alloc_bytes: usize,
    target_size: ImageSize,
    allow_full_fallback: bool,
    cancel: Option<&AtomicBool>,
    profiler: &mut Option<&mut ImageOpenProfiler>,
) -> Result<Option<Rgba8Image>, LoadImageError> {
    check_canceled(path, cancel)?;
    let target_byte_len = target_size
        .rgba8_byte_len()
        .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
    if target_byte_len > preview_max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, target_size));
    }

    let requested_width = u16::try_from(target_size.width().max(1))
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let requested_height = u16::try_from(target_size.height().max(1))
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let mut decoder = JpegPreviewDecoder::new(reader);
    decoder.set_max_decoding_buffer_size(preview_max_alloc_bytes);
    record_image_open_profile_stage(profiler, "static.jpeg_preview.open_file_decoder");
    let (scaled_width, scaled_height) = match decoder.scale(requested_width, requested_height) {
        Ok(size) => size,
        Err(source) => {
            if allow_full_fallback && jpeg_preview_error_can_full_fallback(&source) {
                return Ok(None);
            }
            return Err(jpeg_decode_error_or_canceled(path, source, cancel));
        }
    };
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.scale_decoder");

    let info = decoder
        .info()
        .ok_or_else(|| jpeg_preview_missing_info_error(path))?;
    let Some((color_type, bytes_per_pixel)) = jpeg_preview_pixel_format(info.pixel_format) else {
        return Ok(None);
    };
    let scaled_size = ImageSize::new(u32::from(scaled_width), u32::from(scaled_height));
    let source_byte_len =
        jpeg_preview_source_byte_len(path, scaled_size, bytes_per_pixel, preview_max_alloc_bytes)?;

    let source = match decoder.decode() {
        Ok(source) => source,
        Err(source) => {
            if allow_full_fallback && jpeg_preview_error_can_full_fallback(&source) {
                return Ok(None);
            }
            return Err(jpeg_decode_error_or_canceled(path, source, cancel));
        }
    };
    check_canceled(path, cancel)?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.decode_scaled_pixels");
    if source.len() != source_byte_len {
        return Err(LoadImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
        });
    }

    let preview = sampled_static_preview_rgba8_from_decoded(
        path,
        source,
        color_type,
        bytes_per_pixel,
        scaled_size,
        target_size,
        target_byte_len,
        cancel,
    )?;
    record_image_open_profile_stage(profiler, "static.jpeg_preview.sample_to_target");
    Ok(Some(preview))
}

fn jpeg_preview_error_can_full_fallback(source: &jpeg_decoder::Error) -> bool {
    !matches!(source, jpeg_decoder::Error::Io(_))
}

fn jpeg_preview_pixel_format(pixel_format: JpegPreviewPixelFormat) -> Option<(ColorType, usize)> {
    match pixel_format {
        JpegPreviewPixelFormat::L8 => Some((ColorType::L8, 1)),
        JpegPreviewPixelFormat::RGB24 => Some((ColorType::Rgb8, 3)),
        JpegPreviewPixelFormat::L16 | JpegPreviewPixelFormat::CMYK32 => None,
    }
}

fn jpeg_preview_source_byte_len(
    path: &Path,
    size: ImageSize,
    bytes_per_pixel: usize,
    max_alloc_bytes: usize,
) -> Result<usize, LoadImageError> {
    let width =
        usize::try_from(size.width()).map_err(|_| sampled_static_preview_too_large(path, size))?;
    let height =
        usize::try_from(size.height()).map_err(|_| sampled_static_preview_too_large(path, size))?;
    let byte_len = width
        .checked_mul(height)
        .and_then(|pixels| pixels.checked_mul(bytes_per_pixel))
        .ok_or_else(|| sampled_static_preview_too_large(path, size))?;
    if byte_len > max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, size));
    }
    Ok(byte_len)
}

fn jpeg_preview_missing_info_error(path: &Path) -> LoadImageError {
    LoadImageError::DecodeFailed {
        path: path.to_path_buf(),
        source: ImageError::Decoding(DecodingError::new(
            ImageFormat::Jpeg.into(),
            io::Error::new(
                io::ErrorKind::InvalidData,
                "JPEG preview decoder did not expose metadata after scaling",
            ),
        )),
    }
}

fn decode_sampled_static_png_preview_pixel_image(
    path: &Path,
    reader: impl BufRead + Seek,
    preview_max_alloc_bytes: usize,
    source_max_alloc_bytes: usize,
    target_size: ImageSize,
    source_size: ImageSize,
    allow_sampled_rgb8_output: bool,
    cancel: Option<&AtomicBool>,
) -> Result<Option<PixelImage>, LoadImageError> {
    check_canceled(path, cancel)?;
    let mut decoder = png::Decoder::new_with_limits(
        reader,
        png::Limits {
            bytes: source_max_alloc_bytes,
        },
    );
    decoder.set_ignore_text_chunk(false);
    decoder.set_transformations(png::Transformations::EXPAND);
    let mut reader = decoder
        .read_info()
        .map_err(|source| png_decode_error_or_canceled(path, source, cancel))?;

    let png_source_size = {
        let info = reader.info();
        ImageSize::new(info.width, info.height)
    };
    if png_source_size != source_size {
        return Err(LoadImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
        });
    }
    if reader.info().interlaced || reader.info().animation_control.is_some() {
        return Ok(None);
    }

    let (png_color_type, png_bit_depth) = reader.output_color_type();
    let Some((color_type, bytes_per_pixel)) =
        sampled_static_preview_png_color_type(png_color_type, png_bit_depth)
    else {
        return Ok(None);
    };
    let use_rgb8_output =
        sampled_static_preview_uses_rgb8_output(color_type, allow_sampled_rgb8_output);
    let target_byte_len =
        sampled_static_preview_target_byte_len(path, target_size, use_rgb8_output)?;
    if target_byte_len > preview_max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, target_size));
    }

    let target_width = usize::try_from(target_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let target_height = usize::try_from(target_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let source_width = usize::try_from(source_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_height = usize::try_from(source_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_row_len = source_width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
    let png_row_len = reader
        .output_line_size(source_size.width())
        .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
    if png_row_len != source_row_len {
        return Ok(None);
    }
    if source_row_len > source_max_alloc_bytes {
        return Err(sampled_static_preview_too_large(path, source_size));
    }

    let source_x_offsets = sampled_static_preview_source_x_offsets(
        path,
        target_width,
        source_width,
        bytes_per_pixel,
        source_size,
    )?;
    let mut source_row = try_zeroed_preview_vec(path, source_row_len)?;
    let mut pixels = try_zeroed_preview_vec(path, target_byte_len)?;
    let mut target_y = 0usize;

    for source_y in 0..source_height {
        if target_y >= target_height {
            break;
        }
        check_canceled(path, cancel)?;
        let row_read = reader
            .read_row(&mut source_row)
            .map_err(|source| png_decode_error_or_canceled(path, source, cancel))?;
        if row_read.is_none() {
            return Err(LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            });
        }

        while target_y < target_height
            && sampled_static_preview_source_index(target_y, target_height, source_height)
                == source_y
        {
            if use_rgb8_output {
                write_sampled_static_preview_rgb8_row(
                    path,
                    color_type,
                    &source_row,
                    bytes_per_pixel,
                    &source_x_offsets,
                    target_y,
                    target_width,
                    target_size,
                    source_size,
                    &mut pixels,
                )?;
            } else {
                write_sampled_static_preview_rgba8_row(
                    path,
                    color_type,
                    &source_row,
                    bytes_per_pixel,
                    &source_x_offsets,
                    target_y,
                    target_width,
                    target_size,
                    source_size,
                    &mut pixels,
                )?;
            }
            target_y += 1;
        }
    }

    if target_y != target_height {
        return Err(LoadImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
        });
    }
    check_canceled(path, cancel)?;
    if use_rgb8_output {
        Ok(Some(PixelImage::from(Rgb8Image::new(
            target_size.width(),
            target_size.height(),
            pixels,
        ))))
    } else {
        Ok(Some(PixelImage::from(Rgba8Image::new(
            target_size.width(),
            target_size.height(),
            pixels,
        ))))
    }
}

fn sampled_static_preview_pixel_image_from_decoded(
    path: &Path,
    source: Vec<u8>,
    color_type: ColorType,
    bytes_per_pixel: usize,
    source_size: ImageSize,
    target_size: ImageSize,
    target_byte_len: usize,
    use_rgb8_output: bool,
    cancel: Option<&AtomicBool>,
) -> Result<PixelImage, LoadImageError> {
    if use_rgb8_output {
        let preview = sampled_static_preview_rgb8_from_decoded(
            path,
            source,
            color_type,
            bytes_per_pixel,
            source_size,
            target_size,
            target_byte_len,
            cancel,
        )?;
        Ok(PixelImage::from(preview))
    } else {
        let preview = sampled_static_preview_rgba8_from_decoded(
            path,
            source,
            color_type,
            bytes_per_pixel,
            source_size,
            target_size,
            target_byte_len,
            cancel,
        )?;
        Ok(PixelImage::from(preview))
    }
}

fn sampled_static_preview_rgba8_from_decoded(
    path: &Path,
    source: Vec<u8>,
    color_type: ColorType,
    bytes_per_pixel: usize,
    source_size: ImageSize,
    target_size: ImageSize,
    target_byte_len: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    let target_width = usize::try_from(target_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let target_height = usize::try_from(target_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let source_width = usize::try_from(source_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_height = usize::try_from(source_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_row_len = source_width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;

    if color_type == ColorType::Rgba8 && target_size == source_size {
        if source.len() != target_byte_len {
            return Err(LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            });
        }
        check_canceled(path, cancel)?;
        return Ok(Rgba8Image::new(
            target_size.width(),
            target_size.height(),
            source,
        ));
    }

    let source_x_offsets = sampled_static_preview_source_x_offsets(
        path,
        target_width,
        source_width,
        bytes_per_pixel,
        source_size,
    )?;

    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(target_byte_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    pixels.resize(target_byte_len, 0);

    for target_y in 0..target_height {
        check_canceled(path, cancel)?;
        let source_y = sampled_static_preview_source_index(target_y, target_height, source_height);
        let source_row = source_y
            .checked_mul(source_row_len)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_row_end = source_row
            .checked_add(source_row_len)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_row = source.get(source_row..source_row_end).ok_or_else(|| {
            LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            }
        })?;
        write_sampled_static_preview_rgba8_row(
            path,
            color_type,
            source_row,
            bytes_per_pixel,
            &source_x_offsets,
            target_y,
            target_width,
            target_size,
            source_size,
            &mut pixels,
        )?;
    }

    drop(source);
    check_canceled(path, cancel)?;
    Ok(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        pixels,
    ))
}

fn sampled_static_preview_rgb8_from_decoded(
    path: &Path,
    source: Vec<u8>,
    color_type: ColorType,
    bytes_per_pixel: usize,
    source_size: ImageSize,
    target_size: ImageSize,
    target_byte_len: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgb8Image, LoadImageError> {
    let target_width = usize::try_from(target_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let target_height = usize::try_from(target_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, target_size))?;
    let source_width = usize::try_from(source_size.width())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_height = usize::try_from(source_size.height())
        .map_err(|_| sampled_static_preview_too_large(path, source_size))?;
    let source_row_len = source_width
        .checked_mul(bytes_per_pixel)
        .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;

    if color_type == ColorType::Rgb8 && target_size == source_size {
        if source.len() != target_byte_len {
            return Err(LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            });
        }
        check_canceled(path, cancel)?;
        return Ok(Rgb8Image::new(
            target_size.width(),
            target_size.height(),
            source,
        ));
    }

    let source_x_offsets = sampled_static_preview_source_x_offsets(
        path,
        target_width,
        source_width,
        bytes_per_pixel,
        source_size,
    )?;

    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(target_byte_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    pixels.resize(target_byte_len, 0);

    for target_y in 0..target_height {
        check_canceled(path, cancel)?;
        let source_y = sampled_static_preview_source_index(target_y, target_height, source_height);
        let source_row = source_y
            .checked_mul(source_row_len)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_row_end = source_row
            .checked_add(source_row_len)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_row = source.get(source_row..source_row_end).ok_or_else(|| {
            LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            }
        })?;
        write_sampled_static_preview_rgb8_row(
            path,
            color_type,
            source_row,
            bytes_per_pixel,
            &source_x_offsets,
            target_y,
            target_width,
            target_size,
            source_size,
            &mut pixels,
        )?;
    }

    drop(source);
    check_canceled(path, cancel)?;
    Ok(Rgb8Image::new(
        target_size.width(),
        target_size.height(),
        pixels,
    ))
}

fn sampled_static_preview_source_x_offsets(
    path: &Path,
    target_width: usize,
    source_width: usize,
    bytes_per_pixel: usize,
    source_size: ImageSize,
) -> Result<Vec<usize>, LoadImageError> {
    let mut source_x_offsets = Vec::new();
    source_x_offsets
        .try_reserve_exact(target_width)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    for target_x in 0..target_width {
        let source_x = sampled_static_preview_source_index(target_x, target_width, source_width);
        let offset = source_x
            .checked_mul(bytes_per_pixel)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        source_x_offsets.push(offset);
    }
    Ok(source_x_offsets)
}

fn write_sampled_static_preview_rgba8_row(
    path: &Path,
    color_type: ColorType,
    source_row: &[u8],
    bytes_per_pixel: usize,
    source_x_offsets: &[usize],
    target_y: usize,
    target_width: usize,
    target_size: ImageSize,
    source_size: ImageSize,
    pixels: &mut [u8],
) -> Result<(), LoadImageError> {
    let output_row = target_y
        .checked_mul(target_width)
        .and_then(|offset| offset.checked_mul(4))
        .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;

    for (target_x, source_offset) in source_x_offsets.iter().copied().enumerate() {
        let source_pixel_end = source_offset
            .checked_add(bytes_per_pixel)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_pixel = source_row
            .get(source_offset..source_pixel_end)
            .ok_or_else(|| LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            })?;

        let output_offset = output_row
            .checked_add(
                target_x
                    .checked_mul(4)
                    .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?,
            )
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
        let output_pixel_end = output_offset
            .checked_add(4)
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
        let output = pixels
            .get_mut(output_offset..output_pixel_end)
            .ok_or_else(|| LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            })?;
        write_sampled_static_preview_rgba8_pixel(color_type, source_pixel, output);
    }

    Ok(())
}

fn write_sampled_static_preview_rgb8_row(
    path: &Path,
    color_type: ColorType,
    source_row: &[u8],
    bytes_per_pixel: usize,
    source_x_offsets: &[usize],
    target_y: usize,
    target_width: usize,
    target_size: ImageSize,
    source_size: ImageSize,
    pixels: &mut [u8],
) -> Result<(), LoadImageError> {
    let output_row = target_y
        .checked_mul(target_width)
        .and_then(|offset| offset.checked_mul(3))
        .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;

    for (target_x, source_offset) in source_x_offsets.iter().copied().enumerate() {
        let source_pixel_end = source_offset
            .checked_add(bytes_per_pixel)
            .ok_or_else(|| sampled_static_preview_too_large(path, source_size))?;
        let source_pixel = source_row
            .get(source_offset..source_pixel_end)
            .ok_or_else(|| LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            })?;

        let output_offset = output_row
            .checked_add(
                target_x
                    .checked_mul(3)
                    .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?,
            )
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
        let output_pixel_end = output_offset
            .checked_add(3)
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))?;
        let output = pixels
            .get_mut(output_offset..output_pixel_end)
            .ok_or_else(|| LoadImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
            })?;
        write_sampled_static_preview_rgb8_pixel(color_type, source_pixel, output);
    }

    Ok(())
}

fn try_zeroed_preview_vec(path: &Path, byte_len: usize) -> Result<Vec<u8>, LoadImageError> {
    let mut bytes = Vec::new();
    bytes
        .try_reserve_exact(byte_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    bytes.resize(byte_len, 0);
    Ok(bytes)
}

fn sampled_static_preview_color_type(color_type: ColorType) -> Option<usize> {
    match color_type {
        ColorType::L8 => Some(1),
        ColorType::La8 => Some(2),
        ColorType::Rgb8 => Some(3),
        ColorType::Rgba8 => Some(4),
        _ => None,
    }
}

fn sampled_static_preview_uses_rgb8_output(
    color_type: ColorType,
    allow_sampled_rgb8_output: bool,
) -> bool {
    allow_sampled_rgb8_output && matches!(color_type, ColorType::L8 | ColorType::Rgb8)
}

fn sampled_static_preview_target_byte_len(
    path: &Path,
    target_size: ImageSize,
    use_rgb8_output: bool,
) -> Result<usize, LoadImageError> {
    if use_rgb8_output {
        target_size
            .pixel_byte_len(PixelFormat::Rgb8)
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))
    } else {
        target_size
            .rgba8_byte_len()
            .ok_or_else(|| sampled_static_preview_too_large(path, target_size))
    }
}

fn sampled_static_preview_png_color_type(
    color_type: png::ColorType,
    bit_depth: png::BitDepth,
) -> Option<(ColorType, usize)> {
    if bit_depth != png::BitDepth::Eight {
        return None;
    }

    match color_type {
        png::ColorType::Grayscale => Some((ColorType::L8, 1)),
        png::ColorType::GrayscaleAlpha => Some((ColorType::La8, 2)),
        png::ColorType::Rgb => Some((ColorType::Rgb8, 3)),
        png::ColorType::Rgba => Some((ColorType::Rgba8, 4)),
        png::ColorType::Indexed => None,
    }
}

fn write_sampled_static_preview_rgba8_pixel(
    color_type: ColorType,
    source: &[u8],
    output: &mut [u8],
) {
    match color_type {
        ColorType::L8 => {
            output[0] = source[0];
            output[1] = source[0];
            output[2] = source[0];
            output[3] = 255;
        }
        ColorType::La8 => {
            output[0] = source[0];
            output[1] = source[0];
            output[2] = source[0];
            output[3] = source[1];
        }
        ColorType::Rgb8 => {
            output[0] = source[0];
            output[1] = source[1];
            output[2] = source[2];
            output[3] = 255;
        }
        ColorType::Rgba8 => {
            output.copy_from_slice(source);
        }
        _ => {}
    }
}

fn write_sampled_static_preview_rgb8_pixel(
    color_type: ColorType,
    source: &[u8],
    output: &mut [u8],
) {
    match color_type {
        ColorType::L8 => {
            output[0] = source[0];
            output[1] = source[0];
            output[2] = source[0];
        }
        ColorType::Rgb8 => {
            output.copy_from_slice(source);
        }
        _ => {}
    }
}

fn sampled_static_preview_source_index(
    target_index: usize,
    target_len: usize,
    source_len: usize,
) -> usize {
    target_index.saturating_mul(source_len) / target_len.max(1)
}

fn sampled_static_preview_too_large(path: &Path, size: ImageSize) -> LoadImageError {
    LoadImageError::ImageTooLarge {
        path: path.to_path_buf(),
        size,
        source: None,
    }
}

#[derive(Clone, Copy)]
struct BmpPreviewHeader {
    source_size: ImageSize,
    data_offset: u64,
    row_stride: u64,
    pixel_stride: usize,
    top_down: bool,
    pixel_layout: BmpPreviewPixelLayout,
}

#[derive(Clone, Copy)]
enum BmpPreviewPixelLayout {
    Bgr24,
    Bgrx32,
    Bgra32,
}

fn decode_bmp_preview_rgba8_image(
    path: &Path,
    source_size: ImageSize,
    target_size: ImageSize,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<Option<Rgba8Image>, LoadImageError> {
    let mut reader = open_cancelable_buffered_file(path, cancel)?;
    let Some(header) = read_bmp_preview_header(path, &mut reader, cancel)? else {
        return Ok(None);
    };
    if header.source_size != source_size {
        return Ok(None);
    }

    let target_byte_len = target_size
        .rgba8_byte_len()
        .ok_or_else(|| bmp_preview_too_large(path, target_size))?;
    if target_byte_len > max_alloc_bytes {
        return Err(bmp_preview_too_large(path, target_size));
    }

    let target_width = usize::try_from(target_size.width())
        .map_err(|_| bmp_preview_too_large(path, target_size))?;
    let target_height = usize::try_from(target_size.height())
        .map_err(|_| bmp_preview_too_large(path, target_size))?;
    let source_width = usize::try_from(source_size.width())
        .map_err(|_| bmp_preview_too_large(path, source_size))?;
    let row_len =
        usize::try_from(header.row_stride).map_err(|_| bmp_preview_too_large(path, source_size))?;

    let mut pixels = Vec::new();
    pixels
        .try_reserve_exact(target_byte_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    pixels.resize(target_byte_len, 0);

    let mut row = Vec::new();
    row.try_reserve_exact(row_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    row.resize(row_len, 0);

    let mut source_x_offsets = Vec::new();
    source_x_offsets
        .try_reserve_exact(target_width)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    for target_x in 0..target_width {
        let source_x = bmp_preview_sample_source_index(target_x, target_width, source_width);
        let offset = source_x
            .checked_mul(header.pixel_stride)
            .ok_or_else(|| bmp_preview_too_large(path, source_size))?;
        source_x_offsets.push(offset);
    }

    let source_height = usize::try_from(source_size.height())
        .map_err(|_| bmp_preview_too_large(path, source_size))?;
    for target_y in 0..target_height {
        check_canceled(path, cancel)?;
        let source_y = bmp_preview_sample_source_index(target_y, target_height, source_height);
        let file_y = if header.top_down {
            source_y
        } else {
            source_height
                .checked_sub(1)
                .and_then(|last_y| last_y.checked_sub(source_y))
                .ok_or_else(|| bmp_preview_too_large(path, source_size))?
        };
        let row_offset = header
            .data_offset
            .checked_add(
                u64::try_from(file_y)
                    .ok()
                    .and_then(|y| y.checked_mul(header.row_stride))
                    .ok_or_else(|| bmp_preview_too_large(path, source_size))?,
            )
            .ok_or_else(|| bmp_preview_too_large(path, source_size))?;
        if !bmp_preview_seek(path, &mut reader, row_offset, cancel)?
            || !bmp_preview_read_exact_or_unsupported(path, &mut reader, &mut row, cancel)?
        {
            return Ok(None);
        }

        let output_row = target_y
            .checked_mul(target_width)
            .and_then(|offset| offset.checked_mul(4))
            .ok_or_else(|| bmp_preview_too_large(path, target_size))?;
        for (target_x, source_offset) in source_x_offsets.iter().copied().enumerate() {
            let source_pixel_end = source_offset
                .checked_add(header.pixel_stride)
                .ok_or_else(|| bmp_preview_too_large(path, source_size))?;
            if source_pixel_end > row.len() {
                return Ok(None);
            }

            let output_offset = output_row
                .checked_add(
                    target_x
                        .checked_mul(4)
                        .ok_or_else(|| bmp_preview_too_large(path, target_size))?,
                )
                .ok_or_else(|| bmp_preview_too_large(path, target_size))?;
            let output_pixel_end = output_offset
                .checked_add(4)
                .ok_or_else(|| bmp_preview_too_large(path, target_size))?;
            let output = pixels
                .get_mut(output_offset..output_pixel_end)
                .ok_or_else(|| LoadImageError::InvalidPixelBuffer {
                    path: path.to_path_buf(),
                })?;
            let source = &row[source_offset..source_pixel_end];
            match header.pixel_layout {
                BmpPreviewPixelLayout::Bgr24 => {
                    output[0] = source[2];
                    output[1] = source[1];
                    output[2] = source[0];
                    output[3] = 255;
                }
                BmpPreviewPixelLayout::Bgrx32 => {
                    output[0] = source[2];
                    output[1] = source[1];
                    output[2] = source[0];
                    output[3] = 255;
                }
                BmpPreviewPixelLayout::Bgra32 => {
                    output[0] = source[2];
                    output[1] = source[1];
                    output[2] = source[0];
                    output[3] = source[3];
                }
            }
        }
    }

    check_canceled(path, cancel)?;
    Ok(Some(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        pixels,
    )))
}

fn read_bmp_preview_header(
    path: &Path,
    reader: &mut (impl Read + Seek),
    cancel: Option<&AtomicBool>,
) -> Result<Option<BmpPreviewHeader>, LoadImageError> {
    let mut file_header = [0u8; BMP_FILE_HEADER_LEN];
    if !bmp_preview_read_exact_or_unsupported(path, reader, &mut file_header, cancel)? {
        return Ok(None);
    }
    if &file_header[0..2] != b"BM" {
        return Ok(None);
    }
    let Some(data_offset) = read_le_u32(&file_header, 10).map(u64::from) else {
        return Ok(None);
    };

    let mut dib_header_len_bytes = [0u8; 4];
    if !bmp_preview_read_exact_or_unsupported(path, reader, &mut dib_header_len_bytes, cancel)? {
        return Ok(None);
    }
    let dib_header_len = u32::from_le_bytes(dib_header_len_bytes);
    if !(BMP_MIN_DIB_HEADER_LEN..=BMP_V5_DIB_HEADER_LEN).contains(&dib_header_len) {
        return Ok(None);
    }

    let dib_len = usize::try_from(dib_header_len)
        .map_err(|_| bmp_preview_too_large(path, ImageSize::new(0, 0)))?;
    let mut dib_header = Vec::new();
    dib_header
        .try_reserve_exact(dib_len)
        .map_err(|_| LoadImageError::OutOfMemory {
            path: path.to_path_buf(),
            source: None,
        })?;
    dib_header.extend_from_slice(&dib_header_len_bytes);
    dib_header.resize(dib_len, 0);
    if !bmp_preview_read_exact_or_unsupported(path, reader, &mut dib_header[4..], cancel)? {
        return Ok(None);
    }

    let Some(width) = read_le_i32(&dib_header, 4) else {
        return Ok(None);
    };
    let Some(height) = read_le_i32(&dib_header, 8) else {
        return Ok(None);
    };
    let Some(planes) = read_le_u16(&dib_header, 12) else {
        return Ok(None);
    };
    let Some(bits_per_pixel) = read_le_u16(&dib_header, 14) else {
        return Ok(None);
    };
    let Some(compression) = read_le_u32(&dib_header, 16) else {
        return Ok(None);
    };
    if width <= 0 || height == 0 || planes != 1 {
        return Ok(None);
    }
    let top_down = height < 0;
    let Some(height_abs) = height.checked_abs() else {
        return Ok(None);
    };
    let Ok(width) = u32::try_from(width) else {
        return Ok(None);
    };
    let Ok(height) = u32::try_from(height_abs) else {
        return Ok(None);
    };
    let source_size = ImageSize::new(width, height);
    let Some(row_stride) = bmp_preview_row_stride(width, bits_per_pixel) else {
        return Ok(None);
    };
    let Some(pixel_layout) = bmp_preview_pixel_layout(
        path,
        reader,
        &dib_header,
        data_offset,
        bits_per_pixel,
        compression,
        cancel,
    )?
    else {
        return Ok(None);
    };
    let pixel_stride = usize::from(bits_per_pixel / BMP_BITS_PER_BYTE as u16);
    if data_offset
        < u64::from(BMP_FILE_HEADER_LEN as u32)
            .checked_add(u64::from(dib_header_len))
            .ok_or_else(|| bmp_preview_too_large(path, source_size))?
    {
        return Ok(None);
    }

    Ok(Some(BmpPreviewHeader {
        source_size,
        data_offset,
        row_stride,
        pixel_stride,
        top_down,
        pixel_layout,
    }))
}

fn bmp_preview_pixel_layout(
    path: &Path,
    reader: &mut (impl Read + Seek),
    dib_header: &[u8],
    data_offset: u64,
    bits_per_pixel: u16,
    compression: u32,
    cancel: Option<&AtomicBool>,
) -> Result<Option<BmpPreviewPixelLayout>, LoadImageError> {
    match (bits_per_pixel, compression) {
        (24, BMP_COMPRESSION_RGB) => Ok(Some(BmpPreviewPixelLayout::Bgr24)),
        (32, BMP_COMPRESSION_RGB) => Ok(Some(BmpPreviewPixelLayout::Bgrx32)),
        (32, BMP_COMPRESSION_BITFIELDS) => {
            let Some((red_mask, green_mask, blue_mask, alpha_mask)) =
                bmp_preview_bitfield_masks(path, reader, dib_header, data_offset, cancel)?
            else {
                return Ok(None);
            };
            if red_mask == BMP_RGBA_RED_MASK
                && green_mask == BMP_RGBA_GREEN_MASK
                && blue_mask == BMP_RGBA_BLUE_MASK
            {
                match alpha_mask {
                    BMP_RGBA_ALPHA_MASK => Ok(Some(BmpPreviewPixelLayout::Bgra32)),
                    0 => Ok(Some(BmpPreviewPixelLayout::Bgrx32)),
                    _ => Ok(None),
                }
            } else {
                Ok(None)
            }
        }
        _ => Ok(None),
    }
}

fn bmp_preview_bitfield_masks(
    path: &Path,
    reader: &mut (impl Read + Seek),
    dib_header: &[u8],
    data_offset: u64,
    cancel: Option<&AtomicBool>,
) -> Result<Option<(u32, u32, u32, u32)>, LoadImageError> {
    if dib_header.len() >= BMP_V4_DIB_HEADER_LEN as usize {
        let Some(red_mask) = read_le_u32(dib_header, 40) else {
            return Ok(None);
        };
        let Some(green_mask) = read_le_u32(dib_header, 44) else {
            return Ok(None);
        };
        let Some(blue_mask) = read_le_u32(dib_header, 48) else {
            return Ok(None);
        };
        let Some(alpha_mask) = read_le_u32(dib_header, 52) else {
            return Ok(None);
        };
        return Ok(Some((red_mask, green_mask, blue_mask, alpha_mask)));
    }

    let masks_offset = u64::from(BMP_FILE_HEADER_LEN as u32)
        .checked_add(
            u64::try_from(dib_header.len())
                .map_err(|_| bmp_preview_too_large(path, ImageSize::new(0, 0)))?,
        )
        .ok_or_else(|| bmp_preview_too_large(path, ImageSize::new(0, 0)))?;
    let Some(bytes_before_pixels) = data_offset.checked_sub(masks_offset) else {
        return Ok(None);
    };
    if bytes_before_pixels < 12 {
        return Ok(None);
    }

    let mask_bytes_len = if bytes_before_pixels >= 16 { 16 } else { 12 };
    let mut mask_bytes = [0u8; 16];
    if !bmp_preview_seek(path, reader, masks_offset, cancel)?
        || !bmp_preview_read_exact_or_unsupported(
            path,
            reader,
            &mut mask_bytes[..mask_bytes_len],
            cancel,
        )?
    {
        return Ok(None);
    }

    let Some(red_mask) = read_le_u32(&mask_bytes, 0) else {
        return Ok(None);
    };
    let Some(green_mask) = read_le_u32(&mask_bytes, 4) else {
        return Ok(None);
    };
    let Some(blue_mask) = read_le_u32(&mask_bytes, 8) else {
        return Ok(None);
    };
    let alpha_mask = if mask_bytes_len >= 16 {
        let Some(alpha_mask) = read_le_u32(&mask_bytes, 12) else {
            return Ok(None);
        };
        alpha_mask
    } else {
        0
    };

    Ok(Some((red_mask, green_mask, blue_mask, alpha_mask)))
}

fn bmp_preview_row_stride(width: u32, bits_per_pixel: u16) -> Option<u64> {
    u64::from(width)
        .checked_mul(u64::from(bits_per_pixel))?
        .checked_add(BMP_ROW_ALIGN_BITS - 1)?
        .checked_div(BMP_ROW_ALIGN_BITS)?
        .checked_mul(BMP_ROW_ALIGN_BYTES)
}

fn bmp_preview_sample_source_index(
    target_index: usize,
    target_len: usize,
    source_len: usize,
) -> usize {
    target_index.saturating_mul(source_len) / target_len.max(1)
}

fn bmp_preview_seek(
    path: &Path,
    reader: &mut impl Seek,
    offset: u64,
    cancel: Option<&AtomicBool>,
) -> Result<bool, LoadImageError> {
    match reader.seek(SeekFrom::Start(offset)) {
        Ok(_) => {
            check_canceled(path, cancel)?;
            Ok(true)
        }
        Err(source) => match check_canceled(path, cancel) {
            Ok(()) => Err(LoadImageError::FileAccess {
                path: path.to_path_buf(),
                source,
            }),
            Err(error) => Err(error),
        },
    }
}

fn bmp_preview_read_exact_or_unsupported(
    path: &Path,
    reader: &mut impl Read,
    buffer: &mut [u8],
    cancel: Option<&AtomicBool>,
) -> Result<bool, LoadImageError> {
    match reader.read_exact(buffer) {
        Ok(()) => {
            check_canceled(path, cancel)?;
            Ok(true)
        }
        Err(source) => match check_canceled(path, cancel) {
            Ok(()) if source.kind() == io::ErrorKind::UnexpectedEof => Ok(false),
            Ok(()) => Err(LoadImageError::FileAccess {
                path: path.to_path_buf(),
                source,
            }),
            Err(error) => Err(error),
        },
    }
}

fn bmp_preview_too_large(path: &Path, size: ImageSize) -> LoadImageError {
    LoadImageError::ImageTooLarge {
        path: path.to_path_buf(),
        size,
        source: None,
    }
}

fn read_le_u16(buffer: &[u8], offset: usize) -> Option<u16> {
    Some(u16::from_le_bytes(read_fixed_bytes(buffer, offset)?))
}

fn read_le_u32(buffer: &[u8], offset: usize) -> Option<u32> {
    Some(u32::from_le_bytes(read_fixed_bytes(buffer, offset)?))
}

fn read_le_i32(buffer: &[u8], offset: usize) -> Option<i32> {
    Some(i32::from_le_bytes(read_fixed_bytes(buffer, offset)?))
}

fn read_fixed_bytes<const LEN: usize>(buffer: &[u8], offset: usize) -> Option<[u8; LEN]> {
    buffer
        .get(offset..offset.checked_add(LEN)?)?
        .try_into()
        .ok()
}

#[cfg(test)]
fn decode_dynamic_image(
    path: &Path,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<image::DynamicImage, LoadImageError> {
    let format = image_format_for_open(path)?;
    let decoder = open_image_decoder(path, format, decode_limits(max_alloc_bytes), cancel)?;
    decode_dynamic_image_from_decoder(path, decoder, max_alloc_bytes, cancel)
}

fn decode_dynamic_image_from_decoder(
    path: &Path,
    mut decoder: impl ImageDecoder,
    max_alloc_bytes: usize,
    cancel: Option<&AtomicBool>,
) -> Result<image::DynamicImage, LoadImageError> {
    let mut limits = decode_limits(max_alloc_bytes);
    limits
        .reserve(decoder.total_bytes())
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    decoder
        .set_limits(limits)
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    let decoded = image::DynamicImage::from_decoder(decoder)
        .map_err(|source| image_decode_error_or_canceled(path, source, cancel))?;
    check_canceled(path, cancel)?;
    Ok(decoded)
}

fn dynamic_image_into_rgba8(decoded: image::DynamicImage) -> Rgba8Image {
    let rgba8 = decoded.into_rgba8();
    let (width, height) = rgba8.dimensions();
    Rgba8Image::new(width, height, rgba8.into_raw())
}

fn dynamic_image_into_pixel_image(decoded: image::DynamicImage) -> PixelImage {
    match decoded {
        image::DynamicImage::ImageLuma8(luma8) => {
            let (width, height) = luma8.dimensions();
            let luma8 = luma8.into_raw();
            let target_len = luma8.len().checked_mul(3).unwrap_or(0);
            let mut rgb8 = Vec::with_capacity(target_len);
            for luma in luma8 {
                rgb8.extend_from_slice(&[luma, luma, luma]);
            }
            PixelImage::from(Rgb8Image::new(width, height, rgb8))
        }
        image::DynamicImage::ImageRgb8(rgb8) => {
            let (width, height) = rgb8.dimensions();
            PixelImage::from(Rgb8Image::new(width, height, rgb8.into_raw()))
        }
        image::DynamicImage::ImageRgba8(rgba8) => {
            let (width, height) = rgba8.dimensions();
            PixelImage::from(Rgba8Image::new(width, height, rgba8.into_raw()))
        }
        decoded => PixelImage::from(dynamic_image_into_rgba8(decoded)),
    }
}

fn image_format_for_decode(format: SupportedImageFormat) -> ImageFormat {
    match format {
        SupportedImageFormat::Jpeg => ImageFormat::Jpeg,
        SupportedImageFormat::Png => ImageFormat::Png,
        SupportedImageFormat::Bmp => ImageFormat::Bmp,
        SupportedImageFormat::Gif => ImageFormat::Gif,
        SupportedImageFormat::Webp => ImageFormat::WebP,
        SupportedImageFormat::Ico => ImageFormat::Ico,
        SupportedImageFormat::Tiff => ImageFormat::Tiff,
        SupportedImageFormat::Tga => ImageFormat::Tga,
    }
}

fn is_static_image_format(format: SupportedImageFormat) -> bool {
    matches!(
        format,
        SupportedImageFormat::Jpeg
            | SupportedImageFormat::Png
            | SupportedImageFormat::Bmp
            | SupportedImageFormat::Ico
            | SupportedImageFormat::Tiff
            | SupportedImageFormat::Tga
    )
}

fn downscale_rgba8(
    path: &Path,
    rgba8: Rgba8Image,
    target_size: ImageSize,
    cancel: Option<&AtomicBool>,
) -> Result<Rgba8Image, LoadImageError> {
    if rgba8.size() == target_size {
        return Ok(rgba8);
    }

    check_canceled(path, cancel)?;
    let width = rgba8.width();
    let height = rgba8.height();
    let source = image::RgbaImage::from_raw(width, height, rgba8.into_raw()).ok_or_else(|| {
        LoadImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
        }
    })?;
    check_canceled(path, cancel)?;
    let preview = resize(
        &source,
        target_size.width(),
        target_size.height(),
        FilterType::Triangle,
    );
    check_canceled(path, cancel)?;

    Ok(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        preview.into_raw(),
    ))
}

fn validate_export_pixel_buffer(path: &Path, rgba8: &Rgba8Image) -> Result<(), ExportImageError> {
    if rgba8
        .size()
        .rgba8_byte_len()
        .is_some_and(|expected_len| expected_len == rgba8.pixels().len())
    {
        Ok(())
    } else {
        Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: rgba8.size(),
            actual_len: rgba8.pixels().len(),
        })
    }
}

fn validate_export_pixel_image(path: &Path, pixels: &PixelImage) -> Result<(), ExportImageError> {
    if pixels.is_valid() {
        Ok(())
    } else {
        Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: pixels.size(),
            actual_len: pixels.pixels().len(),
        })
    }
}

fn finish_metadata_stripping_writer(
    path: &Path,
    result: io::Result<()>,
) -> Result<(), ExportImageError> {
    result.map_err(|source| ExportImageError::FileWrite {
        path: path.to_path_buf(),
        source,
    })
}

const JPEG_SOI: [u8; 2] = [0xff, 0xd8];
const JPEG_SOS_MARKER: u8 = 0xda;
const JPEG_EOI_MARKER: u8 = 0xd9;
const JPEG_COM_MARKER: u8 = 0xfe;
const PNG_SIGNATURE: [u8; 8] = [0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
const PNG_CHUNK_HEADER_LEN: usize = 8;
const PNG_CHUNK_CRC_LEN: usize = 4;
const WEBP_RIFF_HEADER_LEN: usize = 12;
const WEBP_CHUNK_HEADER_LEN: usize = 8;
const WEBP_RIFF_SIZE_FIELD_OFFSET: u64 = 4;
const WEBP_RIFF_SIZE_EXCLUDED_LEN: u64 = 8;

struct MetadataStripRanges {
    ranges: Vec<Range<usize>>,
    stripped_len: usize,
}

impl MetadataStripRanges {
    fn new() -> Self {
        Self {
            ranges: Vec::new(),
            stripped_len: 0,
        }
    }

    #[cfg(test)]
    fn with_stripped_len(stripped_len: usize) -> Self {
        Self {
            ranges: Vec::new(),
            stripped_len,
        }
    }

    fn push(&mut self, range: Range<usize>) {
        if range.start == range.end {
            return;
        }

        self.stripped_len += range.end - range.start;
        self.ranges.push(range);
    }
}

fn write_encoded_ranges<W: Write>(
    writer: &mut W,
    encoded: &[u8],
    ranges: &[Range<usize>],
) -> io::Result<()> {
    for range in ranges {
        writer.write_all(&encoded[range.start..range.end])?;
    }
    Ok(())
}

struct JpegMetadataStrippingWriter<'a, W: Write> {
    writer: &'a mut W,
    buffer: Vec<u8>,
    passthrough: bool,
}

impl<'a, W: Write> JpegMetadataStrippingWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            buffer: Vec::new(),
            passthrough: false,
        }
    }

    fn finish(mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.writer.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        self.writer.flush()
    }

    fn process_buffer(&mut self) -> io::Result<()> {
        if self.buffer.len() < JPEG_SOI.len() {
            return Ok(());
        }
        if self.buffer[..JPEG_SOI.len()] != JPEG_SOI {
            return self.switch_to_passthrough();
        }

        let mut ranges = MetadataStripRanges::new();
        ranges.push(0..JPEG_SOI.len());
        let mut index = JPEG_SOI.len();

        while index < self.buffer.len() {
            let marker_start = index;
            if self.buffer[index] != 0xff {
                return self.switch_to_passthrough();
            }
            while index < self.buffer.len() && self.buffer[index] == 0xff {
                index += 1;
            }
            if index >= self.buffer.len() {
                return Ok(());
            }

            let marker = self.buffer[index];
            index += 1;
            if marker == JPEG_EOI_MARKER || marker == JPEG_SOS_MARKER {
                ranges.push(marker_start..self.buffer.len());
                write_encoded_ranges(self.writer, &self.buffer, &ranges.ranges)?;
                self.buffer.clear();
                self.passthrough = true;
                return Ok(());
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                ranges.push(marker_start..index);
                continue;
            }

            if index + 2 > self.buffer.len() {
                return Ok(());
            }
            let length = u16::from_be_bytes([self.buffer[index], self.buffer[index + 1]]) as usize;
            if length < 2 {
                return self.switch_to_passthrough();
            }
            let segment_end = index + length;
            if segment_end > self.buffer.len() {
                return Ok(());
            }
            if !(is_jpeg_app_marker(marker) || marker == JPEG_COM_MARKER) {
                ranges.push(marker_start..segment_end);
            }
            index = segment_end;
        }

        Ok(())
    }

    fn switch_to_passthrough(&mut self) -> io::Result<()> {
        self.passthrough = true;
        if !self.buffer.is_empty() {
            self.writer.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl<W: Write> Write for JpegMetadataStrippingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.passthrough {
            self.writer.write_all(buffer)?;
        } else {
            self.buffer.extend_from_slice(buffer);
            self.process_buffer()?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

struct PngMetadataStrippingWriter<'a, W: Write> {
    writer: &'a mut W,
    buffer: Vec<u8>,
    signature_written: bool,
    passthrough: bool,
}

impl<'a, W: Write> PngMetadataStrippingWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            buffer: Vec::new(),
            signature_written: false,
            passthrough: false,
        }
    }

    fn finish(mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.writer.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        self.writer.flush()
    }

    fn process_buffer(&mut self) -> io::Result<()> {
        if !self.signature_written {
            if self.buffer.len() < PNG_SIGNATURE.len() {
                return Ok(());
            }
            if self.buffer[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
                return self.switch_to_passthrough();
            }
            self.writer.write_all(&PNG_SIGNATURE)?;
            self.buffer.drain(..PNG_SIGNATURE.len());
            self.signature_written = true;
        }

        loop {
            if self.buffer.len() < PNG_CHUNK_HEADER_LEN + PNG_CHUNK_CRC_LEN {
                return Ok(());
            }

            let length = u32::from_be_bytes([
                self.buffer[0],
                self.buffer[1],
                self.buffer[2],
                self.buffer[3],
            ]) as usize;
            let data_start = PNG_CHUNK_HEADER_LEN;
            let Some(crc_start) = data_start.checked_add(length) else {
                return self.switch_to_passthrough();
            };
            let Some(chunk_end) = crc_start.checked_add(PNG_CHUNK_CRC_LEN) else {
                return self.switch_to_passthrough();
            };
            if chunk_end > self.buffer.len() {
                return Ok(());
            }

            let chunk_type = [
                self.buffer[4],
                self.buffer[5],
                self.buffer[6],
                self.buffer[7],
            ];
            if should_keep_png_chunk(&chunk_type) {
                self.writer.write_all(&self.buffer[..chunk_end])?;
            }
            self.buffer.drain(..chunk_end);

            if chunk_type == *b"IEND" {
                if !self.buffer.is_empty() {
                    self.writer.write_all(&self.buffer)?;
                    self.buffer.clear();
                }
                self.passthrough = true;
                return Ok(());
            }
        }
    }

    fn switch_to_passthrough(&mut self) -> io::Result<()> {
        self.passthrough = true;
        if !self.buffer.is_empty() {
            self.writer.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl<W: Write> Write for PngMetadataStrippingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        if self.passthrough {
            self.writer.write_all(buffer)?;
        } else {
            self.buffer.extend_from_slice(buffer);
            self.process_buffer()?;
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

#[derive(Clone, Copy)]
enum WebpMetadataStrippingState {
    RiffHeader,
    ChunkHeader,
    ChunkPayload { remaining: u64, keep: bool },
    Passthrough,
}

struct WebpMetadataStrippingWriter<'a, W: Write + Seek> {
    writer: &'a mut W,
    buffer: Vec<u8>,
    state: WebpMetadataStrippingState,
    stripped_len: u64,
}

impl<'a, W: Write + Seek> WebpMetadataStrippingWriter<'a, W> {
    fn new(writer: &'a mut W) -> Self {
        Self {
            writer,
            buffer: Vec::with_capacity(WEBP_RIFF_HEADER_LEN),
            state: WebpMetadataStrippingState::RiffHeader,
            stripped_len: 0,
        }
    }

    fn finish(mut self) -> io::Result<()> {
        match self.state {
            WebpMetadataStrippingState::Passthrough => {}
            WebpMetadataStrippingState::RiffHeader => self.flush_pending_passthrough()?,
            WebpMetadataStrippingState::ChunkHeader if self.buffer.is_empty() => {
                self.update_riff_size()?;
            }
            WebpMetadataStrippingState::ChunkHeader => self.flush_pending_passthrough()?,
            WebpMetadataStrippingState::ChunkPayload { .. } => {}
        }
        self.writer.flush()
    }

    fn process_riff_header(&mut self, input: &mut &[u8]) -> io::Result<()> {
        fill_metadata_stripping_buffer(&mut self.buffer, input, WEBP_RIFF_HEADER_LEN);
        if self.buffer.len() < WEBP_RIFF_HEADER_LEN {
            return Ok(());
        }

        if &self.buffer[0..4] != b"RIFF" || &self.buffer[8..12] != b"WEBP" {
            return self.switch_to_passthrough();
        }

        self.writer.write_all(&self.buffer)?;
        self.buffer.clear();
        self.state = WebpMetadataStrippingState::ChunkHeader;
        self.stripped_len = WEBP_RIFF_HEADER_LEN as u64;
        Ok(())
    }

    fn process_chunk_header(&mut self, input: &mut &[u8]) -> io::Result<()> {
        fill_metadata_stripping_buffer(&mut self.buffer, input, WEBP_CHUNK_HEADER_LEN);
        if self.buffer.len() < WEBP_CHUNK_HEADER_LEN {
            return Ok(());
        }

        let chunk_type = [
            self.buffer[0],
            self.buffer[1],
            self.buffer[2],
            self.buffer[3],
        ];
        let chunk_size = u32::from_le_bytes([
            self.buffer[4],
            self.buffer[5],
            self.buffer[6],
            self.buffer[7],
        ]) as u64;
        let padded_size = chunk_size + (chunk_size % 2);
        let keep = !is_webp_metadata_chunk(&chunk_type);

        if keep {
            self.writer.write_all(&self.buffer)?;
            self.add_stripped_len(WEBP_CHUNK_HEADER_LEN as u64)?;
        }
        self.buffer.clear();

        if padded_size == 0 {
            self.state = WebpMetadataStrippingState::ChunkHeader;
        } else {
            self.state = WebpMetadataStrippingState::ChunkPayload {
                remaining: padded_size,
                keep,
            };
        }
        Ok(())
    }

    fn process_chunk_payload(
        &mut self,
        input: &mut &[u8],
        remaining: u64,
        keep: bool,
    ) -> io::Result<()> {
        let take = if remaining > input.len() as u64 {
            input.len()
        } else {
            remaining as usize
        };

        if keep {
            self.writer.write_all(&input[..take])?;
            self.add_stripped_len(take as u64)?;
        }
        *input = &input[take..];

        let remaining = remaining - take as u64;
        if remaining == 0 {
            self.state = WebpMetadataStrippingState::ChunkHeader;
        } else {
            self.state = WebpMetadataStrippingState::ChunkPayload { remaining, keep };
        }
        Ok(())
    }

    fn update_riff_size(&mut self) -> io::Result<()> {
        let riff_size = self
            .stripped_len
            .checked_sub(WEBP_RIFF_SIZE_EXCLUDED_LEN)
            .and_then(|len| u32::try_from(len).ok())
            .ok_or_else(webp_metadata_stripping_size_error)?;
        let end_position = self.writer.stream_position()?;

        self.writer
            .seek(SeekFrom::Start(WEBP_RIFF_SIZE_FIELD_OFFSET))?;
        self.writer.write_all(&riff_size.to_le_bytes())?;
        self.writer.seek(SeekFrom::Start(end_position))?;
        Ok(())
    }

    fn add_stripped_len(&mut self, len: u64) -> io::Result<()> {
        self.stripped_len = self
            .stripped_len
            .checked_add(len)
            .ok_or_else(webp_metadata_stripping_size_error)?;
        Ok(())
    }

    fn switch_to_passthrough(&mut self) -> io::Result<()> {
        self.state = WebpMetadataStrippingState::Passthrough;
        self.flush_pending_passthrough()
    }

    fn flush_pending_passthrough(&mut self) -> io::Result<()> {
        if !self.buffer.is_empty() {
            self.writer.write_all(&self.buffer)?;
            self.buffer.clear();
        }
        Ok(())
    }
}

impl<W: Write + Seek> Write for WebpMetadataStrippingWriter<'_, W> {
    fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
        let mut input = buffer;
        while !input.is_empty() {
            match self.state {
                WebpMetadataStrippingState::RiffHeader => self.process_riff_header(&mut input)?,
                WebpMetadataStrippingState::ChunkHeader => self.process_chunk_header(&mut input)?,
                WebpMetadataStrippingState::ChunkPayload { remaining, keep } => {
                    self.process_chunk_payload(&mut input, remaining, keep)?;
                }
                WebpMetadataStrippingState::Passthrough => {
                    self.writer.write_all(input)?;
                    input = &[];
                }
            }
        }
        Ok(buffer.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        self.writer.flush()
    }
}

fn fill_metadata_stripping_buffer(buffer: &mut Vec<u8>, input: &mut &[u8], target_len: usize) {
    let needed = target_len.saturating_sub(buffer.len());
    let take = needed.min(input.len());
    buffer.extend_from_slice(&input[..take]);
    *input = &input[take..];
}

fn webp_metadata_stripping_size_error() -> io::Error {
    io::Error::new(
        io::ErrorKind::InvalidData,
        "stripped WebP output is too large",
    )
}

#[cfg(test)]
fn write_jpeg_metadata_stripped<W: Write>(encoded: &[u8], writer: &mut W) -> io::Result<()> {
    let Some(ranges) = jpeg_metadata_strip_ranges(encoded) else {
        return writer.write_all(encoded);
    };

    if ranges.stripped_len == encoded.len() {
        return writer.write_all(encoded);
    }

    write_encoded_ranges(writer, encoded, &ranges.ranges)
}

#[cfg(test)]
fn jpeg_metadata_strip_ranges(encoded: &[u8]) -> Option<MetadataStripRanges> {
    if encoded.len() < JPEG_SOI.len() || encoded[..JPEG_SOI.len()] != JPEG_SOI {
        return None;
    }

    let mut ranges = MetadataStripRanges::new();
    ranges.push(0..JPEG_SOI.len());
    let mut index = JPEG_SOI.len();

    while index < encoded.len() {
        let marker_start = index;
        if encoded[index] != 0xff {
            return None;
        }
        while index < encoded.len() && encoded[index] == 0xff {
            index += 1;
        }
        if index >= encoded.len() {
            return None;
        }

        let marker = encoded[index];
        index += 1;
        if marker == JPEG_EOI_MARKER {
            ranges.push(marker_start..index);
            ranges.push(index..encoded.len());
            return Some(ranges);
        }
        if marker == JPEG_SOS_MARKER {
            ranges.push(marker_start..encoded.len());
            return Some(ranges);
        }
        if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
            ranges.push(marker_start..index);
            continue;
        }

        if index + 2 > encoded.len() {
            return None;
        }
        let length = u16::from_be_bytes([encoded[index], encoded[index + 1]]) as usize;
        if length < 2 || index + length > encoded.len() {
            return None;
        }
        let segment_end = index + length;
        if !(is_jpeg_app_marker(marker) || marker == JPEG_COM_MARKER) {
            ranges.push(marker_start..segment_end);
        }
        index = segment_end;
    }

    None
}

fn is_jpeg_app_marker(marker: u8) -> bool {
    (0xe0..=0xef).contains(&marker)
}

#[cfg(test)]
fn write_png_metadata_stripped<W: Write>(encoded: &[u8], writer: &mut W) -> io::Result<()> {
    let Some(ranges) = png_metadata_strip_ranges(encoded) else {
        return writer.write_all(encoded);
    };

    if ranges.stripped_len == encoded.len() {
        return writer.write_all(encoded);
    }

    write_encoded_ranges(writer, encoded, &ranges.ranges)
}

#[cfg(test)]
fn png_metadata_strip_ranges(encoded: &[u8]) -> Option<MetadataStripRanges> {
    if encoded.len() < PNG_SIGNATURE.len() || encoded[..PNG_SIGNATURE.len()] != PNG_SIGNATURE {
        return None;
    }

    let mut ranges = MetadataStripRanges::new();
    ranges.push(0..PNG_SIGNATURE.len());
    let mut index = PNG_SIGNATURE.len();

    while index + PNG_CHUNK_HEADER_LEN + PNG_CHUNK_CRC_LEN <= encoded.len() {
        let chunk_start = index;
        let length = u32::from_be_bytes([
            encoded[index],
            encoded[index + 1],
            encoded[index + 2],
            encoded[index + 3],
        ]) as usize;
        let chunk_type_start = index + 4;
        let data_start = index + PNG_CHUNK_HEADER_LEN;
        let crc_start = data_start.checked_add(length)?;
        let chunk_end = crc_start.checked_add(PNG_CHUNK_CRC_LEN)?;
        if chunk_end > encoded.len() {
            return None;
        }

        let chunk_type = [
            encoded[chunk_type_start],
            encoded[chunk_type_start + 1],
            encoded[chunk_type_start + 2],
            encoded[chunk_type_start + 3],
        ];
        if should_keep_png_chunk(&chunk_type) {
            ranges.push(chunk_start..chunk_end);
        }

        index = chunk_end;
        if chunk_type == *b"IEND" {
            return (index == encoded.len()).then_some(ranges);
        }
    }

    None
}

fn should_keep_png_chunk(chunk_type: &[u8]) -> bool {
    chunk_type.len() == 4 && (chunk_type[0] & 0x20 == 0 || chunk_type == b"tRNS")
}

#[cfg(test)]
fn write_webp_metadata_stripped<W: Write>(encoded: &[u8], writer: &mut W) -> io::Result<()> {
    let Some(ranges) = webp_metadata_strip_ranges(encoded) else {
        return writer.write_all(encoded);
    };
    let Ok(riff_size) = u32::try_from(ranges.stripped_len - 8) else {
        return writer.write_all(encoded);
    };
    let riff_size_bytes = riff_size.to_le_bytes();

    if ranges.stripped_len == encoded.len() && encoded[4..8] == riff_size_bytes[..] {
        return writer.write_all(encoded);
    }

    writer.write_all(b"RIFF")?;
    writer.write_all(&riff_size_bytes)?;
    writer.write_all(b"WEBP")?;
    write_encoded_ranges(writer, encoded, &ranges.ranges)
}

#[cfg(test)]
fn webp_metadata_strip_ranges(encoded: &[u8]) -> Option<MetadataStripRanges> {
    if encoded.len() < WEBP_RIFF_HEADER_LEN
        || &encoded[0..4] != b"RIFF"
        || &encoded[8..12] != b"WEBP"
    {
        return None;
    }

    let mut ranges = MetadataStripRanges::with_stripped_len(WEBP_RIFF_HEADER_LEN);
    let mut index = WEBP_RIFF_HEADER_LEN;

    while index + WEBP_CHUNK_HEADER_LEN <= encoded.len() {
        let chunk_start = index;
        let chunk_type = [
            encoded[index],
            encoded[index + 1],
            encoded[index + 2],
            encoded[index + 3],
        ];
        let chunk_size = u32::from_le_bytes([
            encoded[index + 4],
            encoded[index + 5],
            encoded[index + 6],
            encoded[index + 7],
        ]) as usize;
        let data_start = index + WEBP_CHUNK_HEADER_LEN;
        let padded_size = chunk_size + (chunk_size % 2);
        let chunk_end = data_start.checked_add(padded_size)?;
        if chunk_end > encoded.len() {
            return None;
        }

        if !is_webp_metadata_chunk(&chunk_type) {
            ranges.push(chunk_start..chunk_end);
        }
        index = chunk_end;
    }

    if index != encoded.len() {
        return None;
    }

    Some(ranges)
}

fn is_webp_metadata_chunk(chunk_type: &[u8]) -> bool {
    matches!(chunk_type, b"EXIF" | b"XMP " | b"ICCP")
}

fn encode_rgba8_image<W: Write + Seek>(
    path: &Path,
    writer: &mut W,
    rgba8: &Rgba8Image,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    if options.remove_metadata() {
        match options.format() {
            ExportFormat::Jpeg => {
                let mut stripped_writer = JpegMetadataStrippingWriter::new(writer);
                encode_rgba8_image_raw(path, &mut stripped_writer, rgba8, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Png => {
                let mut stripped_writer = PngMetadataStrippingWriter::new(writer);
                encode_rgba8_image_raw(path, &mut stripped_writer, rgba8, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Webp => {
                let mut stripped_writer = WebpMetadataStrippingWriter::new(writer);
                encode_rgba8_image_raw(path, &mut stripped_writer, rgba8, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Bmp | ExportFormat::Ico => {}
        }
    }

    encode_rgba8_image_raw(path, writer, rgba8, options)
}

fn encode_rgba8_image_raw<W: Write>(
    path: &Path,
    writer: &mut W,
    rgba8: &Rgba8Image,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    if options.format() == ExportFormat::Ico {
        return encode_rgba8_icon(path, writer, rgba8);
    }

    let result = match options.format() {
        ExportFormat::Png => PngEncoder::new(&mut *writer).write_image(
            rgba8.pixels(),
            rgba8.width(),
            rgba8.height(),
            ExtendedColorType::Rgba8,
        ),
        ExportFormat::Jpeg => {
            let rgb8 = flatten_rgba8_to_rgb8(path, rgba8, options.jpeg_alpha_background_rgb())?;
            let quality = options.quality().unwrap_or_else(|| {
                crate::domain::export_quality_range(ExportFormat::Jpeg)
                    .map_or(90, |range| range.default())
            });
            JpegEncoder::new_with_quality(&mut *writer, quality).write_image(
                &rgb8,
                rgba8.width(),
                rgba8.height(),
                ExtendedColorType::Rgb8,
            )
        }
        ExportFormat::Bmp => BmpEncoder::new(&mut *writer).write_image(
            rgba8.pixels(),
            rgba8.width(),
            rgba8.height(),
            ExtendedColorType::Rgba8,
        ),
        ExportFormat::Webp => WebPEncoder::new_lossless(&mut *writer).write_image(
            rgba8.pixels(),
            rgba8.width(),
            rgba8.height(),
            ExtendedColorType::Rgba8,
        ),
        ExportFormat::Ico => unreachable!("ICO export is handled before single-image encoders"),
    };

    result.map_err(|source| ExportImageError::EncodeFailed {
        path: path.to_path_buf(),
        format: options.format(),
        source,
    })
}

fn encode_pixel_image<W: Write + Seek>(
    path: &Path,
    writer: &mut W,
    pixels: &PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    if options.remove_metadata() {
        match options.format() {
            ExportFormat::Jpeg => {
                let mut stripped_writer = JpegMetadataStrippingWriter::new(writer);
                encode_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Png => {
                let mut stripped_writer = PngMetadataStrippingWriter::new(writer);
                encode_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Webp => {
                let mut stripped_writer = WebpMetadataStrippingWriter::new(writer);
                encode_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Bmp | ExportFormat::Ico => {}
        }
    }

    encode_pixel_image_raw(path, writer, pixels, options)
}

fn encode_pixel_image_raw<W: Write>(
    path: &Path,
    writer: &mut W,
    pixels: &PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let result = match (options.format(), pixels) {
        (ExportFormat::Png, PixelImage::Rgb8(rgb8)) => PngEncoder::new(&mut *writer).write_image(
            rgb8.pixels(),
            rgb8.width(),
            rgb8.height(),
            ExtendedColorType::Rgb8,
        ),
        (ExportFormat::Png, PixelImage::Rgba8(rgba8)) => PngEncoder::new(&mut *writer).write_image(
            rgba8.pixels(),
            rgba8.width(),
            rgba8.height(),
            ExtendedColorType::Rgba8,
        ),
        (ExportFormat::Bmp, PixelImage::Rgb8(rgb8)) => BmpEncoder::new(&mut *writer).write_image(
            rgb8.pixels(),
            rgb8.width(),
            rgb8.height(),
            ExtendedColorType::Rgb8,
        ),
        (ExportFormat::Bmp, PixelImage::Rgba8(rgba8)) => BmpEncoder::new(&mut *writer).write_image(
            rgba8.pixels(),
            rgba8.width(),
            rgba8.height(),
            ExtendedColorType::Rgba8,
        ),
        (ExportFormat::Webp, PixelImage::Rgb8(rgb8)) => WebPEncoder::new_lossless(&mut *writer)
            .write_image(
                rgb8.pixels(),
                rgb8.width(),
                rgb8.height(),
                ExtendedColorType::Rgb8,
            ),
        (ExportFormat::Webp, PixelImage::Rgba8(rgba8)) => WebPEncoder::new_lossless(&mut *writer)
            .write_image(
                rgba8.pixels(),
                rgba8.width(),
                rgba8.height(),
                ExtendedColorType::Rgba8,
            ),
        (ExportFormat::Ico, PixelImage::Rgb8(rgb8)) => {
            return encode_rgb8_icon(path, writer, rgb8);
        }
        (ExportFormat::Ico, PixelImage::Rgba8(rgba8)) => {
            return encode_rgba8_icon(path, writer, rgba8);
        }
        (ExportFormat::Jpeg, PixelImage::Rgb8(rgb8)) => {
            return encode_rgb8_jpeg(
                path,
                writer,
                rgb8.width(),
                rgb8.height(),
                rgb8.pixels(),
                options,
            );
        }
        (ExportFormat::Jpeg, PixelImage::Rgba8(rgba8)) => {
            return encode_rgba8_image_raw(path, writer, rgba8, options);
        }
        (_, PixelImage::Bgra8(_)) => {
            let rgba8 = pixels
                .to_rgba8()
                .ok_or_else(|| ExportImageError::InvalidPixelBuffer {
                    path: path.to_path_buf(),
                    size: pixels.size(),
                    actual_len: pixels.pixels().len(),
                })?;
            return encode_rgba8_image_raw(path, writer, &rgba8, options);
        }
    };

    result.map_err(|source| ExportImageError::EncodeFailed {
        path: path.to_path_buf(),
        format: options.format(),
        source,
    })
}

fn encode_owned_pixel_image<W: Write + Seek>(
    path: &Path,
    writer: &mut W,
    pixels: PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    if options.remove_metadata() {
        match options.format() {
            ExportFormat::Jpeg => {
                let mut stripped_writer = JpegMetadataStrippingWriter::new(writer);
                encode_owned_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Png => {
                let mut stripped_writer = PngMetadataStrippingWriter::new(writer);
                encode_owned_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Webp => {
                let mut stripped_writer = WebpMetadataStrippingWriter::new(writer);
                encode_owned_pixel_image_raw(path, &mut stripped_writer, pixels, options)?;
                return finish_metadata_stripping_writer(path, stripped_writer.finish());
            }
            ExportFormat::Bmp | ExportFormat::Ico => {}
        }
    }

    encode_owned_pixel_image_raw(path, writer, pixels, options)
}

fn encode_owned_pixel_image_raw<W: Write>(
    path: &Path,
    writer: &mut W,
    pixels: PixelImage,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    match pixels {
        PixelImage::Rgb8(rgb8) if options.format() == ExportFormat::Jpeg => encode_rgb8_jpeg(
            path,
            writer,
            rgb8.width(),
            rgb8.height(),
            rgb8.pixels(),
            options,
        ),
        PixelImage::Rgba8(rgba8) if options.format() == ExportFormat::Jpeg => {
            encode_owned_rgba8_jpeg(path, writer, rgba8, options)
        }
        PixelImage::Bgra8(bgra8) if options.format() == ExportFormat::Jpeg => {
            let size = bgra8.size();
            let actual_len = bgra8.pixels().len();
            let rgba8 = PixelImage::from(bgra8).into_rgba8().ok_or_else(|| {
                ExportImageError::InvalidPixelBuffer {
                    path: path.to_path_buf(),
                    size,
                    actual_len,
                }
            })?;
            encode_owned_rgba8_jpeg(path, writer, rgba8, options)
        }
        pixels => encode_pixel_image_raw(path, writer, &pixels, options),
    }
}

fn encode_owned_rgba8_jpeg<W: Write>(
    path: &Path,
    writer: &mut W,
    rgba8: Rgba8Image,
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let width = rgba8.width();
    let height = rgba8.height();
    let background = options.jpeg_alpha_background_rgb();
    let rgb8 = match rgba8.try_into_raw() {
        Ok(mut raw) => {
            flatten_rgba8_to_rgb8_in_place(path, &mut raw, background)?;
            raw
        }
        Err(rgba8) => flatten_rgba8_to_rgb8(path, &rgba8, background)?,
    };
    let quality = options.quality().unwrap_or_else(|| {
        crate::domain::export_quality_range(ExportFormat::Jpeg).map_or(90, |range| range.default())
    });

    JpegEncoder::new_with_quality(&mut *writer, quality)
        .write_image(&rgb8, width, height, ExtendedColorType::Rgb8)
        .map_err(|source| ExportImageError::EncodeFailed {
            path: path.to_path_buf(),
            format: options.format(),
            source,
        })
}

fn encode_rgb8_jpeg<W: Write>(
    path: &Path,
    writer: &mut W,
    width: u32,
    height: u32,
    rgb8: &[u8],
    options: ExportOptions,
) -> Result<(), ExportImageError> {
    let quality = options.quality().unwrap_or_else(|| {
        crate::domain::export_quality_range(ExportFormat::Jpeg).map_or(90, |range| range.default())
    });

    JpegEncoder::new_with_quality(&mut *writer, quality)
        .write_image(rgb8, width, height, ExtendedColorType::Rgb8)
        .map_err(|source| ExportImageError::EncodeFailed {
            path: path.to_path_buf(),
            format: options.format(),
            source,
        })
}

fn encode_rgb8_icon<W: Write>(
    path: &Path,
    writer: &mut W,
    rgb8: &Rgb8Image,
) -> Result<(), ExportImageError> {
    if rgb8.width() == 0 || rgb8.height() == 0 {
        return Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: rgb8.size(),
            actual_len: rgb8.byte_len(),
        });
    }

    let source =
        BorrowedRgb8Image::new(rgb8.width(), rgb8.height(), rgb8.pixels()).ok_or_else(|| {
            ExportImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
                size: rgb8.size(),
                actual_len: rgb8.byte_len(),
            }
        })?;
    let mut frames = Vec::with_capacity(ICO_EXPORT_FRAME_EDGES.len());

    for edge in ICO_EXPORT_FRAME_EDGES {
        let content_size = icon_content_size(rgb8.size(), edge).ok_or_else(|| {
            ExportImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
                size: rgb8.size(),
                actual_len: rgb8.byte_len(),
            }
        })?;
        let resized = resize(
            &source,
            content_size.width(),
            content_size.height(),
            FilterType::Lanczos3,
        );
        let frame_pixels = rgb8_icon_frame_pixels(path, resized.as_raw(), content_size, edge)?;
        let frame = IcoFrame::as_png(&frame_pixels, edge, edge, ExtendedColorType::Rgba8).map_err(
            |source| ExportImageError::EncodeFailed {
                path: path.to_path_buf(),
                format: ExportFormat::Ico,
                source,
            },
        )?;
        frames.push(frame);
    }

    IcoEncoder::new(&mut *writer)
        .encode_images(&frames)
        .map_err(|source| ExportImageError::EncodeFailed {
            path: path.to_path_buf(),
            format: ExportFormat::Ico,
            source,
        })
}

fn encode_rgba8_icon<W: Write>(
    path: &Path,
    writer: &mut W,
    rgba8: &Rgba8Image,
) -> Result<(), ExportImageError> {
    if rgba8.width() == 0 || rgba8.height() == 0 {
        return Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: rgba8.size(),
            actual_len: rgba8.byte_len(),
        });
    }

    let source = BorrowedRgba8Image::new(rgba8.width(), rgba8.height(), rgba8.pixels())
        .ok_or_else(|| ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: rgba8.size(),
            actual_len: rgba8.byte_len(),
        })?;
    let mut frames = Vec::with_capacity(ICO_EXPORT_FRAME_EDGES.len());

    for edge in ICO_EXPORT_FRAME_EDGES {
        let content_size = icon_content_size(rgba8.size(), edge).ok_or_else(|| {
            ExportImageError::InvalidPixelBuffer {
                path: path.to_path_buf(),
                size: rgba8.size(),
                actual_len: rgba8.byte_len(),
            }
        })?;
        let resized = resize(
            &source,
            content_size.width(),
            content_size.height(),
            FilterType::Lanczos3,
        );
        let frame_pixels = rgba8_icon_frame_pixels(path, resized.as_raw(), content_size, edge)?;
        let frame = IcoFrame::as_png(&frame_pixels, edge, edge, ExtendedColorType::Rgba8).map_err(
            |source| ExportImageError::EncodeFailed {
                path: path.to_path_buf(),
                format: ExportFormat::Ico,
                source,
            },
        )?;
        frames.push(frame);
    }

    IcoEncoder::new(&mut *writer)
        .encode_images(&frames)
        .map_err(|source| ExportImageError::EncodeFailed {
            path: path.to_path_buf(),
            format: ExportFormat::Ico,
            source,
        })
}

fn icon_content_size(source_size: ImageSize, edge: u32) -> Option<ImageSize> {
    if edge == 0 || source_size.is_empty() {
        return None;
    }
    if source_size.width() >= source_size.height() {
        let height = scaled_icon_axis(source_size.height(), edge, source_size.width())?;
        Some(ImageSize::new(edge, height))
    } else {
        let width = scaled_icon_axis(source_size.width(), edge, source_size.height())?;
        Some(ImageSize::new(width, edge))
    }
}

fn scaled_icon_axis(source_axis: u32, target_axis: u32, dominant_source_axis: u32) -> Option<u32> {
    if source_axis == 0 || target_axis == 0 || dominant_source_axis == 0 {
        return None;
    }
    let numerator = u64::from(source_axis) * u64::from(target_axis);
    let rounded = numerator.saturating_add(u64::from(dominant_source_axis) / 2)
        / u64::from(dominant_source_axis);
    u32::try_from(rounded.clamp(1, u64::from(target_axis))).ok()
}

fn rgb8_icon_frame_pixels(
    path: &Path,
    content_pixels: &[u8],
    content_size: ImageSize,
    edge: u32,
) -> Result<Vec<u8>, ExportImageError> {
    let expected_content_len = content_size
        .pixel_byte_len(PixelFormat::Rgb8)
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    if content_pixels.len() != expected_content_len {
        return Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: content_size,
            actual_len: content_pixels.len(),
        });
    }

    let mut frame = transparent_rgba8_icon_frame(path, edge)?;
    let content_row_bytes = usize::try_from(content_size.width())
        .ok()
        .and_then(|width| width.checked_mul(RGB8_BYTES_PER_PIXEL))
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    let frame_row_bytes = usize::try_from(edge)
        .ok()
        .and_then(|width| width.checked_mul(RGBA8_BYTES_PER_PIXEL))
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    let x_offset = usize::try_from((edge - content_size.width()) / 2).map_err(|_| {
        ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        }
    })?;
    let y_offset = usize::try_from((edge - content_size.height()) / 2).map_err(|_| {
        ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        }
    })?;

    for row in 0..usize::try_from(content_size.height()).map_err(|_| {
        ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        }
    })? {
        let source_start = row.checked_mul(content_row_bytes).ok_or_else(|| {
            ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            }
        })?;
        let target_row =
            row.checked_add(y_offset)
                .ok_or_else(|| ExportImageError::AllocationFailed {
                    path: path.to_path_buf(),
                })?;
        let mut target_index = target_row
            .checked_mul(frame_row_bytes)
            .and_then(|start| {
                x_offset
                    .checked_mul(RGBA8_BYTES_PER_PIXEL)
                    .and_then(|x| start.checked_add(x))
            })
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
        for pixel in content_pixels[source_start..source_start + content_row_bytes]
            .chunks_exact(RGB8_BYTES_PER_PIXEL)
        {
            frame[target_index..target_index + RGB8_BYTES_PER_PIXEL].copy_from_slice(pixel);
            frame[target_index + 3] = 255;
            target_index += RGBA8_BYTES_PER_PIXEL;
        }
    }

    Ok(frame)
}

fn rgba8_icon_frame_pixels(
    path: &Path,
    content_pixels: &[u8],
    content_size: ImageSize,
    edge: u32,
) -> Result<Vec<u8>, ExportImageError> {
    let expected_content_len =
        content_size
            .rgba8_byte_len()
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
    if content_pixels.len() != expected_content_len {
        return Err(ExportImageError::InvalidPixelBuffer {
            path: path.to_path_buf(),
            size: content_size,
            actual_len: content_pixels.len(),
        });
    }

    let mut frame = transparent_rgba8_icon_frame(path, edge)?;

    let x_offset = (edge - content_size.width()) / 2;
    let y_offset = (edge - content_size.height()) / 2;
    let content_row_bytes = usize::try_from(content_size.width())
        .ok()
        .and_then(|width| width.checked_mul(RGBA8_BYTES_PER_PIXEL))
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    let frame_row_bytes = usize::try_from(edge)
        .ok()
        .and_then(|width| width.checked_mul(RGBA8_BYTES_PER_PIXEL))
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    let x_offset_bytes = usize::try_from(x_offset)
        .ok()
        .and_then(|offset| offset.checked_mul(RGBA8_BYTES_PER_PIXEL))
        .ok_or_else(|| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;

    for row in 0..content_size.height() {
        let row = usize::try_from(row).map_err(|_| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
        let source_start = row.checked_mul(content_row_bytes).ok_or_else(|| {
            ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            }
        })?;
        let target_row = row
            .checked_add(usize::try_from(y_offset).map_err(|_| {
                ExportImageError::AllocationFailed {
                    path: path.to_path_buf(),
                }
            })?)
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
        let target_start = target_row
            .checked_mul(frame_row_bytes)
            .and_then(|start| start.checked_add(x_offset_bytes))
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
        let source_end = source_start.checked_add(content_row_bytes).ok_or_else(|| {
            ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            }
        })?;
        let target_end = target_start.checked_add(content_row_bytes).ok_or_else(|| {
            ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            }
        })?;
        frame[target_start..target_end].copy_from_slice(&content_pixels[source_start..source_end]);
    }

    Ok(frame)
}

fn transparent_rgba8_icon_frame(path: &Path, edge: u32) -> Result<Vec<u8>, ExportImageError> {
    let frame_size = ImageSize::new(edge, edge);
    let frame_len =
        frame_size
            .rgba8_byte_len()
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
    let mut frame = Vec::new();
    frame
        .try_reserve_exact(frame_len)
        .map_err(|_| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;
    frame.resize(frame_len, 0);
    Ok(frame)
}

fn flatten_rgba8_to_rgb8(
    path: &Path,
    rgba8: &Rgba8Image,
    background: RgbColor,
) -> Result<Vec<u8>, ExportImageError> {
    let pixel_count = rgba8.pixels().len() / 4;
    let target_len =
        pixel_count
            .checked_mul(3)
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;
    let mut rgb = Vec::new();
    rgb.try_reserve_exact(target_len)
        .map_err(|_| ExportImageError::AllocationFailed {
            path: path.to_path_buf(),
        })?;

    for pixel in rgba8.pixels().chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        rgb.push(blend_channel_over_background(
            pixel[0],
            alpha,
            background.red(),
        ));
        rgb.push(blend_channel_over_background(
            pixel[1],
            alpha,
            background.green(),
        ));
        rgb.push(blend_channel_over_background(
            pixel[2],
            alpha,
            background.blue(),
        ));
    }

    Ok(rgb)
}

fn flatten_rgba8_to_rgb8_in_place(
    path: &Path,
    rgba8: &mut Vec<u8>,
    background: RgbColor,
) -> Result<(), ExportImageError> {
    let pixel_count = rgba8.len() / 4;
    let target_len =
        pixel_count
            .checked_mul(3)
            .ok_or_else(|| ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })?;

    for pixel_index in 0..pixel_count {
        let source_index = pixel_index * 4;
        let target_index = pixel_index * 3;
        let alpha = u16::from(rgba8[source_index + 3]);
        rgba8[target_index] =
            blend_channel_over_background(rgba8[source_index], alpha, background.red());
        rgba8[target_index + 1] =
            blend_channel_over_background(rgba8[source_index + 1], alpha, background.green());
        rgba8[target_index + 2] =
            blend_channel_over_background(rgba8[source_index + 2], alpha, background.blue());
    }

    rgba8.truncate(target_len);
    Ok(())
}

fn blend_channel_over_background(channel: u8, alpha: u16, background: u8) -> u8 {
    let inverse_alpha = 255u16.saturating_sub(alpha);
    ((u16::from(channel) * alpha + u16::from(background) * inverse_alpha) / 255) as u8
}

fn image_open_category_from_io_error(source: &io::Error) -> ImageOpenErrorCategory {
    match classify_file_io_error(source) {
        FileIoErrorCategory::PermissionDenied => ImageOpenErrorCategory::PermissionDenied,
        FileIoErrorCategory::NotFound => ImageOpenErrorCategory::FileNotFoundOrMoved,
        FileIoErrorCategory::FileLocked => ImageOpenErrorCategory::FileLocked,
        FileIoErrorCategory::Other => ImageOpenErrorCategory::UnknownIo,
    }
}

fn export_category_from_io_error(source: &io::Error) -> ExportSaveErrorCategory {
    match classify_file_io_error(source) {
        FileIoErrorCategory::PermissionDenied => ExportSaveErrorCategory::PermissionDenied,
        FileIoErrorCategory::NotFound => ExportSaveErrorCategory::PathNotFound,
        FileIoErrorCategory::FileLocked => ExportSaveErrorCategory::FileLocked,
        FileIoErrorCategory::Other => ExportSaveErrorCategory::UnknownIo,
    }
}

fn export_category_from_image_error(source: &ImageError) -> ExportSaveErrorCategory {
    match source {
        ImageError::IoError(source) => export_category_from_io_error(source),
        ImageError::Limits(limit) if matches!(limit.kind(), LimitErrorKind::InsufficientMemory) => {
            ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory
        }
        _ => ExportSaveErrorCategory::EncodingFailed,
    }
}

fn export_file_access_message(category: ExportSaveErrorCategory, path: &Path) -> String {
    match category {
        ExportSaveErrorCategory::PermissionDenied => format!(
            "이미지를 저장할 권한이 없습니다.\n\n파일: {}",
            path.display()
        ),
        ExportSaveErrorCategory::PathNotFound => format!(
            "저장 경로를 찾을 수 없습니다. 폴더가 이동되었거나 삭제되었을 수 있습니다.\n\n파일: {}",
            path.display()
        ),
        ExportSaveErrorCategory::FileLocked => format!(
            "저장할 파일이 다른 프로그램에서 사용 중입니다.\n\n파일: {}",
            path.display()
        ),
        ExportSaveErrorCategory::UnknownIo => format!(
            "이미지를 저장하는 중 알 수 없는 I/O 오류가 발생했습니다.\n\n파일: {}",
            path.display()
        ),
        ExportSaveErrorCategory::EncodingFailed
        | ExportSaveErrorCategory::ImageDataInvalid
        | ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => {
            format!("이미지를 저장하지 못했습니다.\n\n파일: {}", path.display())
        }
    }
}

fn export_file_access_message_for(
    category: ExportSaveErrorCategory,
    path: &Path,
    language: UiLanguage,
) -> String {
    if language == UiLanguage::Korean {
        return export_file_access_message(category, path);
    }
    match category {
        ExportSaveErrorCategory::PermissionDenied => format!(
            "You do not have permission to save the image.\n\nFile: {}",
            path.display()
        ),
        ExportSaveErrorCategory::PathNotFound => format!(
            "Could not find the save path. The folder may have been moved or deleted.\n\nFile: {}",
            path.display()
        ),
        ExportSaveErrorCategory::FileLocked => format!(
            "The file to save is in use by another program.\n\nFile: {}",
            path.display()
        ),
        ExportSaveErrorCategory::UnknownIo => format!(
            "An unknown I/O error occurred while saving the image.\n\nFile: {}",
            path.display()
        ),
        ExportSaveErrorCategory::EncodingFailed
        | ExportSaveErrorCategory::ImageDataInvalid
        | ExportSaveErrorCategory::ImageTooLargeOrOutOfMemory => {
            format!("Could not save the image.\n\nFile: {}", path.display())
        }
    }
}

fn decode_limits(max_alloc_bytes: usize) -> Limits {
    let mut limits = Limits::default();
    limits.max_alloc = Some(max_alloc_bytes as u64);
    limits
}

fn image_decode_error(path: &Path, source: ImageError) -> LoadImageError {
    match source {
        ImageError::Limits(limit) => match limit.kind() {
            LimitErrorKind::InsufficientMemory => LoadImageError::OutOfMemory {
                path: path.to_path_buf(),
                source: Some(ImageError::Limits(limit)),
            },
            LimitErrorKind::DimensionError => LoadImageError::ImageTooLarge {
                path: path.to_path_buf(),
                size: ImageSize::new(0, 0),
                source: Some(ImageError::Limits(limit)),
            },
            LimitErrorKind::Unsupported { .. } => LoadImageError::DecodeFailed {
                path: path.to_path_buf(),
                source: ImageError::Limits(limit),
            },
            _ => LoadImageError::DecodeFailed {
                path: path.to_path_buf(),
                source: ImageError::Limits(limit),
            },
        },
        ImageError::IoError(source) => match classify_file_io_error(&source) {
            FileIoErrorCategory::PermissionDenied
            | FileIoErrorCategory::NotFound
            | FileIoErrorCategory::FileLocked => LoadImageError::FileAccess {
                path: path.to_path_buf(),
                source,
            },
            FileIoErrorCategory::Other
                if matches!(
                    source.kind(),
                    io::ErrorKind::UnexpectedEof | io::ErrorKind::InvalidData
                ) =>
            {
                LoadImageError::DecodeFailed {
                    path: path.to_path_buf(),
                    source: ImageError::IoError(source),
                }
            }
            FileIoErrorCategory::Other => LoadImageError::FileAccess {
                path: path.to_path_buf(),
                source,
            },
        },
        source => LoadImageError::DecodeFailed {
            path: path.to_path_buf(),
            source,
        },
    }
}

fn decode_error_is_unsupported(source: &ImageError) -> bool {
    matches!(source, ImageError::Unsupported(_))
        || matches!(
            source,
            ImageError::Limits(limit)
                if matches!(limit.kind(), LimitErrorKind::Unsupported { .. })
        )
}

fn image_decode_error_or_canceled(
    path: &Path,
    source: ImageError,
    cancel: Option<&AtomicBool>,
) -> LoadImageError {
    match check_canceled(path, cancel) {
        Ok(()) => image_decode_error(path, source),
        Err(error) => error,
    }
}

fn jpeg_decode_error_or_canceled(
    path: &Path,
    source: jpeg_decoder::Error,
    cancel: Option<&AtomicBool>,
) -> LoadImageError {
    image_decode_error_or_canceled(path, image_error_from_jpeg_decode_error(source), cancel)
}

fn image_error_from_jpeg_decode_error(source: jpeg_decoder::Error) -> ImageError {
    match source {
        jpeg_decoder::Error::Io(source) => ImageError::IoError(source),
        source => ImageError::Decoding(DecodingError::new(ImageFormat::Jpeg.into(), source)),
    }
}

fn png_decode_error_or_canceled(
    path: &Path,
    source: png::DecodingError,
    cancel: Option<&AtomicBool>,
) -> LoadImageError {
    image_decode_error_or_canceled(path, image_error_from_png_decode_error(source), cancel)
}

fn image_error_from_png_decode_error(source: png::DecodingError) -> ImageError {
    match source {
        png::DecodingError::IoError(source) => ImageError::IoError(source),
        source @ png::DecodingError::Format(_) => {
            ImageError::Decoding(DecodingError::new(ImageFormat::Png.into(), source))
        }
        source @ png::DecodingError::Parameter(_) => ImageError::Parameter(
            ParameterError::from_kind(ParameterErrorKind::Generic(source.to_string())),
        ),
        png::DecodingError::LimitsExceeded => {
            ImageError::Limits(LimitError::from_kind(LimitErrorKind::InsufficientMemory))
        }
    }
}

fn check_canceled(path: &Path, cancel: Option<&AtomicBool>) -> Result<(), LoadImageError> {
    if cancel.is_some_and(|cancel| cancel.load(Ordering::Acquire)) {
        Err(LoadImageError::DecodeCanceled {
            path: path.to_path_buf(),
        })
    } else {
        Ok(())
    }
}

fn image_format_for_open(path: &Path) -> Result<SupportedImageFormat, LoadImageError> {
    supported_image_format_for_path(path).ok_or_else(|| LoadImageError::UnsupportedFormat {
        path: path.to_path_buf(),
    })
}

fn image_file_version_from_metadata(metadata: &fs::Metadata) -> Option<ImageFileVersion> {
    Some(ImageFileVersion::new(
        metadata.len(),
        metadata.modified().ok()?,
    ))
}

fn image_metadata_from_file(
    path: &Path,
    file_metadata: &fs::Metadata,
    format: SupportedImageFormat,
    exif_orientation: ImageOrientation,
) -> ImageMetadata {
    match image_file_version_from_metadata(file_metadata) {
        Some(file_version) => ImageMetadata::with_file_version_and_exif_orientation(
            path.to_path_buf(),
            file_version,
            format,
            exif_orientation,
        ),
        None => ImageMetadata::with_exif_orientation(
            path.to_path_buf(),
            file_metadata.len(),
            format,
            exif_orientation,
        ),
    }
}

fn ensure_current_file_metadata_matches(
    path: &Path,
    expected_metadata: &fs::Metadata,
    cancel: Option<&AtomicBool>,
) -> Result<(), LoadImageError> {
    check_canceled(path, cancel)?;
    let current_metadata = read_file_metadata(path)?;
    check_canceled(path, cancel)?;
    if image_file_metadata_matches(expected_metadata, &current_metadata) {
        Ok(())
    } else {
        Err(LoadImageError::FileChanged {
            path: path.to_path_buf(),
        })
    }
}

fn image_file_metadata_matches(left: &fs::Metadata, right: &fs::Metadata) -> bool {
    match (
        image_file_version_from_metadata(left),
        image_file_version_from_metadata(right),
    ) {
        (Some(left), Some(right)) => left == right,
        _ => left.len() == right.len(),
    }
}

fn read_file_metadata(path: &Path) -> Result<std::fs::Metadata, LoadImageError> {
    let metadata = std::fs::metadata(path).map_err(|source| LoadImageError::FileAccess {
        path: path.to_path_buf(),
        source,
    })?;

    require_file_metadata(path, metadata)
}

fn require_file_metadata(
    path: &Path,
    metadata: std::fs::Metadata,
) -> Result<std::fs::Metadata, LoadImageError> {
    if metadata.is_file() {
        Ok(metadata)
    } else {
        Err(LoadImageError::NotAFile {
            path: path.to_path_buf(),
        })
    }
}

#[cfg(test)]
mod tests {
    use std::error::Error;
    use std::fs;
    use std::fs::File;
    use std::io::{self, BufWriter, Cursor, Write};
    use std::path::Path;
    #[cfg(target_os = "windows")]
    use std::path::PathBuf;
    #[cfg(target_os = "windows")]
    use std::process::{Command, Stdio};
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::{Arc, Mutex, MutexGuard};
    use std::time::{SystemTime, UNIX_EPOCH};

    use image::codecs::gif::{GifEncoder, Repeat};
    use image::codecs::png::PngEncoder;
    use image::codecs::webp::WebPEncoder;
    use image::error::{ImageFormatHint, UnsupportedError, UnsupportedErrorKind};
    use image::{
        ColorType, Delay, ExtendedColorType, Frame, ImageEncoder, ImageError, ImageFormat,
        RgbaImage,
    };

    use crate::domain::{
        AnimationLoopPolicy, AnimationPlayback, AnimationTimingSettings, AppConfig, ExportFormat,
        ExportOptions, ExportSaveErrorCategory, ImageBufferKind, ImageFileVersion,
        ImageLoadFailureStage, ImageOpenErrorCategory, ImageOrientation, ImageSize,
        MemoryPolicySettings, PixelFormat, PixelImage, Rgba8Image, ScalingQuality,
        SupportedImageFormat, ViewMode, ViewportSize, WindowBounds, DEFAULT_IMAGE_MEMORY_POLICY,
    };

    use super::{
        animation_frame_cache_byte_limit, animation_frame_delivered_prefetch_limit,
        animation_frame_prefetch_for_loaded_image_covers, cached_animation_frame_for_loaded_image,
        cached_animation_frame_for_view, classify_file_io_error, collect_animation_decode_result,
        decode_animation_frame_from_iter, decode_animation_frame_from_iter_with_reuse,
        decode_gif_animation_image_with_parallel_metadata, decode_limits,
        decode_preview_rgba8_image, decode_rgba8_image,
        decode_sampled_static_png_preview_pixel_image,
        decode_sampled_static_preview_pixel_image_from_decoder,
        decode_webp_animation_image_with_parallel_metadata, downscale_rgba8,
        ensure_current_file_metadata_matches, export_rgba8_image, export_writer_sync_policy,
        finish_export_temporary_file_after_encode, flatten_rgba8_to_rgb8,
        flatten_rgba8_to_rgb8_in_place, image_decode_error, image_decoder_size,
        load_animation_frame_for_view, load_app_config_from_path, load_full_resolution_image,
        load_image_file, load_image_file_for_view,
        load_image_file_for_view_with_timing_and_profile, lock_animation_frame_cache,
        lock_static_full_resolution_cache, open_cancelable_file_source_with_metadata,
        open_image_decoder, open_image_decoder_with_metadata_and_source,
        optimize_png_export_temporary_file, png_export_oxipng_options,
        read_decoder_exif_orientation, read_file_metadata, replace_animation_frame_cache,
        replace_static_full_resolution_cache, reusable_animation_frame_cache_for_view,
        sampled_static_preview_pixel_image_from_decoded, sampled_static_preview_rgba8_from_decoded,
        save_app_config_to_path, scan_image_folder_for_file_or_empty,
        scan_image_folder_for_file_with_cancellation, write_jpeg_metadata_stripped,
        write_png_metadata_stripped, write_webp_metadata_stripped, AnimationDecodeContext,
        AnimationFrameCacheBuilder, AnimationFrameForReturn, AnimationFramePixels,
        AnimationFrameRequestResult, AnimationMetadata, AnimationSourceCacheKey,
        AppConfigLoadError, ExportImageError, ExportWriterSyncPolicy, FileIoErrorCategory,
        ImageOpenProfiler, ImageProbe, JpegMetadataStrippingWriter, LoadImageError,
        PngMetadataStrippingWriter, ScanImageFolderError, WebpMetadataStrippingWriter,
        ANIMATION_FRAME_CACHE_RADIUS, ANIMATION_FRAME_PREFETCH_RADIUS,
        ANIMATION_INITIAL_CACHE_FRAME_LIMIT, ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT,
        PNG_EXPORT_OXIPNG_MAX_INPUT_BYTES, PNG_EXPORT_OXIPNG_MIN_INPUT_BYTES,
    };
    #[cfg(target_os = "windows")]
    use super::{scan_image_folder_for_file, AppConfigSaveError};

    static ANIMATION_FRAME_CACHE_TEST_MUTEX: Mutex<()> = Mutex::new(());
    static STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX: Mutex<()> = Mutex::new(());

    fn expect_returned_animation_frame(result: AnimationFrameRequestResult) -> Rgba8Image {
        match result {
            AnimationFrameRequestResult::Frame(frame) => frame.into_rgba8(),
            AnimationFrameRequestResult::Delivered => {
                panic!("test decode should return the requested frame")
            }
        }
    }

    fn contains_bytes(haystack: &[u8], needle: &[u8]) -> bool {
        !needle.is_empty()
            && haystack
                .windows(needle.len())
                .any(|window| window == needle)
    }

    fn jpeg_has_app_or_comment_segment(bytes: &[u8]) -> bool {
        if bytes.len() < 2 || bytes[0..2] != [0xff, 0xd8] {
            return false;
        }

        let mut index = 2;
        while index < bytes.len() {
            if bytes[index] != 0xff {
                return false;
            }
            while index < bytes.len() && bytes[index] == 0xff {
                index += 1;
            }
            if index >= bytes.len() {
                return false;
            }
            let marker = bytes[index];
            index += 1;
            if marker == 0xda {
                return false;
            }
            if (0xe0..=0xef).contains(&marker) || marker == 0xfe {
                return true;
            }
            if marker == 0x01 || (0xd0..=0xd7).contains(&marker) {
                continue;
            }
            if index + 2 > bytes.len() {
                return false;
            }
            let length = u16::from_be_bytes([bytes[index], bytes[index + 1]]) as usize;
            if length < 2 || index + length > bytes.len() {
                return false;
            }
            index += length;
        }

        false
    }

    fn png_test_chunk(chunk_type: &[u8; 4], data: &[u8]) -> Vec<u8> {
        let mut chunk = Vec::new();
        chunk.extend_from_slice(&(data.len() as u32).to_be_bytes());
        chunk.extend_from_slice(chunk_type);
        chunk.extend_from_slice(data);
        chunk.extend_from_slice(&[0, 0, 0, 0]);
        chunk
    }

    fn webp_test_file(chunks: &[(&[u8; 4], &[u8])]) -> Vec<u8> {
        let mut bytes = Vec::new();
        bytes.extend_from_slice(b"RIFF");
        bytes.extend_from_slice(&[0; 4]);
        bytes.extend_from_slice(b"WEBP");
        for (chunk_type, data) in chunks {
            bytes.extend_from_slice(*chunk_type);
            bytes.extend_from_slice(&(data.len() as u32).to_le_bytes());
            bytes.extend_from_slice(data);
            if data.len() % 2 == 1 {
                bytes.push(0);
            }
        }
        let riff_size = u32::try_from(bytes.len() - 8).expect("test WebP size fits");
        bytes[4..8].copy_from_slice(&riff_size.to_le_bytes());
        bytes
    }

    #[test]
    fn file_io_error_classification_distinguishes_common_windows_failures() {
        assert_eq!(
            classify_file_io_error(&io::Error::from_raw_os_error(5)),
            FileIoErrorCategory::PermissionDenied
        );
        assert_eq!(
            classify_file_io_error(&io::Error::from_raw_os_error(2)),
            FileIoErrorCategory::NotFound
        );
        assert_eq!(
            classify_file_io_error(&io::Error::from_raw_os_error(32)),
            FileIoErrorCategory::FileLocked
        );
        assert_eq!(
            classify_file_io_error(&io::Error::other("other")),
            FileIoErrorCategory::Other
        );
    }

    #[test]
    fn current_file_metadata_mismatch_reports_file_changed() {
        let dir = unique_temp_dir("metadata-changed");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("source.png");
        fs::write(&path, b"old").expect("write old file");
        let metadata = read_file_metadata(&path).expect("old metadata");
        fs::write(&path, b"new file contents").expect("replace file");

        let error = ensure_current_file_metadata_matches(&path, &metadata, None)
            .expect_err("changed file should be rejected");

        assert!(matches!(error, LoadImageError::FileChanged { .. }));
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::FileIo);
        assert_eq!(
            error.brief_user_message(),
            "이미지 파일이 로딩 중 변경되었습니다."
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_config_returns_default_for_missing_file() {
        let dir = unique_temp_dir("missing-config");
        let path = dir.join("config.txt");

        let config = load_app_config_from_path(&path).expect("missing config uses defaults");

        assert_eq!(config, AppConfig::default());
    }

    #[test]
    fn load_config_reports_parse_error_for_corrupt_data() {
        let dir = unique_temp_dir("corrupt");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");
        fs::write(&path, "version=1\nnot a config line").expect("corrupt config");

        let error = load_app_config_from_path(&path).expect_err("corrupt config should fail");
        let source = error
            .source()
            .and_then(|source| source.downcast_ref::<crate::domain::AppConfigParseError>())
            .expect("parse source should be chained");
        assert_eq!(
            source,
            &crate::domain::AppConfigParseError::MalformedLine { line: 2 }
        );

        match error {
            AppConfigLoadError::Parse { source, .. } => {
                assert_eq!(
                    source,
                    crate::domain::AppConfigParseError::MalformedLine { line: 2 }
                );
            }
            AppConfigLoadError::FileRead { .. } => panic!("expected parse error"),
        }
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_config_accepts_legacy_version1_without_new_setting_keys() {
        let dir = unique_temp_dir("legacy-config");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");
        fs::write(
            &path,
            "\
version=1
window.x=10
window.y=20
window.width=640
window.height=480
default_view_mode=actual_size
scaling_quality=nearest
recent_folder=C:\\\\Images
export_default_quality=85
animation_autoplay=false
",
        )
        .expect("legacy config");

        let config = load_app_config_from_path(&path).expect("legacy config should load");

        assert_eq!(config.window_bounds(), WindowBounds::new(10, 20, 640, 480));
        assert_eq!(config.default_view_mode(), ViewMode::ActualSize);
        assert_eq!(config.scaling_quality(), ScalingQuality::Nearest);
        assert_eq!(config.recent_folder(), Some(Path::new(r"C:\Images")));
        assert_eq!(config.export_default_quality(), 85);
        assert!(!config.animation_autoplay());
        assert_eq!(config.image_memory_policy(), DEFAULT_IMAGE_MEMORY_POLICY);
        assert_eq!(config.export_settings().export_filename_suffix(), "-export");
        assert!(config.status_ui_settings().show_status_bar());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_config_reports_parse_error_for_unsupported_version_and_invalid_escape() {
        let dir = unique_temp_dir("damaged-config-variants");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");

        for (contents, expected) in [
            (
                "version=2\n",
                crate::domain::AppConfigParseError::UnsupportedVersion { line: 1 },
            ),
            (
                "version=1\nrecent_folder=C:\\q\n",
                crate::domain::AppConfigParseError::InvalidEscape { line: 2 },
            ),
        ] {
            fs::write(&path, contents).expect("damaged config");

            let error = load_app_config_from_path(&path).expect_err("damaged config should fail");

            match error {
                AppConfigLoadError::Parse { source, .. } => {
                    assert_eq!(source, expected, "{contents:?}");
                }
                AppConfigLoadError::FileRead { .. } => panic!("expected parse error"),
            }
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn load_config_permission_denied_is_reported_and_acl_is_restored() {
        let dir = unique_temp_dir("config-read-denied");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");
        save_app_config_to_path(&path, &AppConfig::default()).expect("write config");
        let user = current_windows_identity();
        let mut acl_guard = DeniedAclGuard::deny_read(&path, user);

        let error = load_app_config_from_path(&path).expect_err("denied config should fail");

        match error {
            AppConfigLoadError::FileRead { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            AppConfigLoadError::Parse { .. } => panic!("expected file read error"),
        }

        acl_guard.restore();
        let restored = load_app_config_from_path(&path).expect("restored config should load");
        assert_eq!(restored, AppConfig::default());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn load_image_error_classification_and_messages_are_user_facing() {
        let path = Path::new("missing.png");
        let missing = LoadImageError::FileAccess {
            path: path.to_path_buf(),
            source: io::Error::from_raw_os_error(2),
        };
        let denied = LoadImageError::FileAccess {
            path: Path::new("denied.png").to_path_buf(),
            source: io::Error::from_raw_os_error(5),
        };
        let locked = LoadImageError::FileAccess {
            path: Path::new("locked.png").to_path_buf(),
            source: io::Error::from_raw_os_error(32),
        };

        assert_eq!(
            missing.category(),
            ImageOpenErrorCategory::FileNotFoundOrMoved
        );
        assert!(missing.user_message().contains("찾을 수 없습니다"));
        assert_eq!(missing.failure_stage(), ImageLoadFailureStage::FileIo);
        assert_eq!(denied.category(), ImageOpenErrorCategory::PermissionDenied);
        assert!(denied.user_message().contains("권한"));
        assert_eq!(locked.category(), ImageOpenErrorCategory::FileLocked);
        assert!(locked.user_message().contains("사용 중"));
    }

    #[test]
    fn image_decode_io_error_keeps_source_chain_and_classifies_truncated_file_as_corrupt() {
        let error = image_decode_error(
            Path::new("broken.png"),
            ImageError::IoError(io::Error::new(io::ErrorKind::UnexpectedEof, "short read")),
        );

        assert_eq!(
            error.category(),
            ImageOpenErrorCategory::CorruptOrDecodingFailed
        );
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::Decoder);
        assert!(error.user_message().contains("디코딩할 수 없습니다"));
        assert!(Error::source(&error).is_some());
    }

    #[test]
    fn image_decode_invalid_data_io_error_is_corrupt_not_file_access() {
        let error = image_decode_error(
            Path::new("broken.png"),
            ImageError::IoError(io::Error::new(io::ErrorKind::InvalidData, "bad image data")),
        );

        assert_eq!(
            error.category(),
            ImageOpenErrorCategory::CorruptOrDecodingFailed
        );
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::Decoder);
        assert!(error.user_message().contains("디코딩할 수 없습니다"));
        assert!(Error::source(&error).is_some());
    }

    #[test]
    fn unsupported_decoder_feature_reports_unsupported_format_message() {
        let unsupported = UnsupportedError::from_format_and_kind(
            ImageFormatHint::Exact(ImageFormat::Png),
            UnsupportedErrorKind::GenericFeature("test feature".to_owned()),
        );
        let error = image_decode_error(
            Path::new("unsupported.png"),
            ImageError::Unsupported(unsupported),
        );

        assert_eq!(error.category(), ImageOpenErrorCategory::UnsupportedFormat);
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::Decoder);
        assert!(error.user_message().contains("지원하지 않는 이미지 형식"));
        assert!(Error::source(&error).is_some());
    }

    #[test]
    fn fixture_images_load_for_all_supported_open_formats() {
        let dir = unique_temp_dir("formats");
        fs::create_dir_all(&dir).expect("test dir");
        let cases = [
            (
                "sample.jpg",
                FixtureImageFormat::Jpeg,
                SupportedImageFormat::Jpeg,
                ImageSize::new(2, 2),
            ),
            (
                "sample.jpeg",
                FixtureImageFormat::Jpeg,
                SupportedImageFormat::Jpeg,
                ImageSize::new(2, 2),
            ),
            (
                "sample.png",
                FixtureImageFormat::Png,
                SupportedImageFormat::Png,
                ImageSize::new(2, 2),
            ),
            (
                "sample.bmp",
                FixtureImageFormat::Bmp,
                SupportedImageFormat::Bmp,
                ImageSize::new(2, 2),
            ),
            (
                "sample.gif",
                FixtureImageFormat::Gif,
                SupportedImageFormat::Gif,
                ImageSize::new(2, 2),
            ),
            (
                "sample.WEBP",
                FixtureImageFormat::Webp,
                SupportedImageFormat::Webp,
                ImageSize::new(2, 2),
            ),
            (
                "sample.ICO",
                FixtureImageFormat::Ico,
                SupportedImageFormat::Ico,
                ImageSize::new(256, 256),
            ),
            (
                "sample.tif",
                FixtureImageFormat::Tiff,
                SupportedImageFormat::Tiff,
                ImageSize::new(2, 2),
            ),
            (
                "sample.TIFF",
                FixtureImageFormat::Tiff,
                SupportedImageFormat::Tiff,
                ImageSize::new(2, 2),
            ),
            (
                "sample.tga",
                FixtureImageFormat::Tga,
                SupportedImageFormat::Tga,
                ImageSize::new(2, 2),
            ),
        ];

        for (file_name, fixture_format, expected_format, expected_size) in cases {
            let path = dir.join(file_name);
            write_fixture_image(&path, fixture_format);

            let image = load_image_file(&path).expect("fixture image should load");

            assert_eq!(image.metadata().format(), expected_format);
            assert_eq!(image.source_size(), expected_size);
            assert_eq!(
                image.pixels().expected_byte_len(),
                Some(image.pixels().pixels().len())
            );
            if expected_format == SupportedImageFormat::Jpeg {
                assert_eq!(image.pixels().pixel_format(), PixelFormat::Rgb8);
            }
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_writes_supported_formats_and_reopens_with_expected_metadata() {
        let dir = unique_temp_dir("export-formats");
        fs::create_dir_all(&dir).expect("test dir");
        let source = varied_rgba8(3, 2);
        let cases = [
            (
                "out.png",
                ExportFormat::Png,
                SupportedImageFormat::Png,
                ImageSize::new(3, 2),
            ),
            (
                "out.jpg",
                ExportFormat::Jpeg,
                SupportedImageFormat::Jpeg,
                ImageSize::new(3, 2),
            ),
            (
                "out.jpeg",
                ExportFormat::Jpeg,
                SupportedImageFormat::Jpeg,
                ImageSize::new(3, 2),
            ),
            (
                "out.bmp",
                ExportFormat::Bmp,
                SupportedImageFormat::Bmp,
                ImageSize::new(3, 2),
            ),
            (
                "out.webp",
                ExportFormat::Webp,
                SupportedImageFormat::Webp,
                ImageSize::new(3, 2),
            ),
            (
                "out.ico",
                ExportFormat::Ico,
                SupportedImageFormat::Ico,
                ImageSize::new(256, 256),
            ),
        ];

        for (file_name, export_format, expected_format, expected_size) in cases {
            let path = dir.join(file_name);
            export_rgba8_image(&path, &source, ExportOptions::new(export_format, Some(90)))
                .expect("export image");
            let file_size = fs::metadata(&path).expect("export metadata").len();
            let loaded = load_image_file(&path).expect("reload exported image");

            assert!(file_size > 0, "{file_name}");
            assert_eq!(loaded.metadata().format(), expected_format, "{file_name}");
            assert_eq!(loaded.source_size(), expected_size, "{file_name}");
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_ico_writes_fixed_frames_and_preserves_transparency() {
        let dir = unique_temp_dir("export-ico");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("out.ico");
        let source = Rgba8Image::new(2, 1, vec![255, 0, 0, 255, 0, 0, 255, 0]);

        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Ico, None))
            .expect("export ico");

        let bytes = fs::read(&path).expect("read ico");
        let entries = ico_entries(&bytes);
        assert_eq!(entries.len(), 4);
        assert_eq!(
            entries
                .iter()
                .map(|entry| (entry.width, entry.height))
                .collect::<Vec<_>>(),
            vec![(16, 16), (32, 32), (48, 48), (256, 256)]
        );

        for entry in entries {
            let data = &bytes[entry.data_offset..entry.data_offset + entry.data_size];
            assert!(data.starts_with(&[0x89, b'P', b'N', b'G']));
            let frame = image::load_from_memory_with_format(data, ImageFormat::Png)
                .expect("decode ico png frame")
                .into_rgba8();
            assert_eq!((frame.width(), frame.height()), (entry.width, entry.height));
            assert!(
                frame.pixels().any(|pixel| pixel.0[3] == 0),
                "ICO frame should keep transparent pixels"
            );
            assert!(
                frame.pixels().any(|pixel| pixel.0[3] == 255),
                "ICO frame should keep opaque pixels"
            );
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn png_export_runs_oxipng_optimizer() {
        let dir = unique_temp_dir("png-export-oxipng");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("optimized.png");
        let source = varied_rgba8(768, 512);

        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Png, None))
            .expect("export optimized png");

        let exported = fs::read(&path).expect("read exported png");
        let mut raw = Vec::new();
        PngEncoder::new(&mut raw)
            .write_image(
                source.pixels(),
                source.width(),
                source.height(),
                ExtendedColorType::Rgba8,
            )
            .expect("encode raw png");
        let expected = oxipng::optimize_from_memory(&raw, &png_export_oxipng_options())
            .expect("optimize raw png");

        assert_eq!(exported, expected);
        assert!(exported.len() <= raw.len());
        let loaded = load_image_file(&path).expect("reload optimized png");
        assert_eq!(loaded.pixels().pixels(), source.pixels());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn png_export_sync_policy_depends_on_optimizer_guard() {
        assert_eq!(
            export_writer_sync_policy(ExportOptions::new(ExportFormat::Png, None), true),
            ExportWriterSyncPolicy::FlushOnly
        );
        assert_eq!(
            export_writer_sync_policy(ExportOptions::new(ExportFormat::Png, None), false),
            ExportWriterSyncPolicy::FlushAndSyncAll
        );
        assert_eq!(
            export_writer_sync_policy(ExportOptions::new(ExportFormat::Jpeg, None), false),
            ExportWriterSyncPolicy::FlushAndSyncAll
        );
    }

    #[test]
    fn small_png_export_skips_optimizer_after_encode() {
        let dir = unique_temp_dir("png-export-small-skip-oxipng");
        fs::create_dir_all(&dir).expect("test dir");
        let temporary_path = dir.join("small.png.tmp");
        let file = File::create(&temporary_path).expect("create temp png");
        let encoded_len = PNG_EXPORT_OXIPNG_MIN_INPUT_BYTES - 1;
        file.set_len(encoded_len).expect("mark temp png as small");
        let mut writer = BufWriter::new(file);

        let optimize_png = finish_export_temporary_file_after_encode(
            Path::new("small.png"),
            &mut writer,
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect("small png finish should sync encoded file");

        assert!(!optimize_png);
        drop(writer);
        assert_eq!(
            fs::metadata(&temporary_path)
                .expect("small temp metadata")
                .len(),
            encoded_len
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn large_png_export_skips_optimizer_after_encode() {
        let dir = unique_temp_dir("png-export-large-skip-oxipng");
        fs::create_dir_all(&dir).expect("test dir");
        let temporary_path = dir.join("large.png.tmp");
        let file = File::create(&temporary_path).expect("create temp png");
        file.set_len(PNG_EXPORT_OXIPNG_MAX_INPUT_BYTES + 1)
            .expect("mark temp png as large");
        let mut writer = BufWriter::new(file);

        let optimize_png = finish_export_temporary_file_after_encode(
            Path::new("large.png"),
            &mut writer,
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect("large png finish should sync encoded file");

        assert!(!optimize_png);
        drop(writer);
        assert_eq!(
            fs::metadata(&temporary_path)
                .expect("large temp metadata")
                .len(),
            PNG_EXPORT_OXIPNG_MAX_INPUT_BYTES + 1
        );
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn export_replace_keeps_atomic_replace_without_write_through() {
        assert_eq!(
            super::export_replace_file_flags(),
            super::MOVEFILE_REPLACE_EXISTING
        );
        assert_eq!(
            super::replace_file_flags(),
            super::MOVEFILE_REPLACE_EXISTING | super::MOVEFILE_WRITE_THROUGH
        );
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn config_replace_renames_file_and_syncs_parent_directory() {
        let dir = unique_temp_dir("config-replace-parent-sync");
        fs::create_dir_all(&dir).expect("test dir");
        let temporary_path = dir.join("config.txt.tmp");
        let path = dir.join("config.txt");
        fs::write(&temporary_path, b"new config").expect("temporary config");
        fs::write(&path, b"old config").expect("existing config");

        super::replace_file(&temporary_path, &path).expect("replace config");

        assert!(!temporary_path.exists());
        assert_eq!(fs::read(&path).expect("replaced config"), b"new config");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn png_optimizer_failure_is_reported_as_export_error() {
        let dir = unique_temp_dir("png-export-oxipng-failure");
        fs::create_dir_all(&dir).expect("test dir");
        let temporary_path = dir.join("broken.png.tmp");
        fs::write(&temporary_path, b"not a png").expect("write broken png");

        let error = optimize_png_export_temporary_file(
            Path::new("broken.png"),
            &temporary_path,
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect_err("broken png optimization should fail");

        assert!(matches!(&error, ExportImageError::PngOptimizeFailed { .. }));
        assert_eq!(error.category(), ExportSaveErrorCategory::EncodingFailed);
        assert!(error.user_message().contains("PNG"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_remove_metadata_strips_jpeg_app_segments() {
        let dir = unique_temp_dir("export-remove-metadata-jpeg");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("metadata-free.jpg");
        let source = varied_rgba8(16, 12);

        export_rgba8_image(
            &path,
            &source,
            ExportOptions::new(ExportFormat::Jpeg, Some(90)).with_remove_metadata(true),
        )
        .expect("export metadata-free jpeg");

        let bytes = fs::read(&path).expect("read exported jpeg");
        assert!(!jpeg_has_app_or_comment_segment(&bytes));
        let loaded = load_image_file(&path).expect("reload metadata-free jpeg");
        assert_eq!(loaded.metadata().format(), SupportedImageFormat::Jpeg);
        assert_eq!(loaded.source_size(), ImageSize::new(16, 12));

        let _ = fs::remove_dir_all(dir);
    }

    fn stripped_jpeg_metadata(encoded: &[u8]) -> Vec<u8> {
        let mut stripped = Vec::new();
        write_jpeg_metadata_stripped(encoded, &mut stripped).expect("write stripped jpeg");
        stripped
    }

    fn stripped_png_metadata(encoded: &[u8]) -> Vec<u8> {
        let mut stripped = Vec::new();
        write_png_metadata_stripped(encoded, &mut stripped).expect("write stripped png");
        stripped
    }

    fn stripped_webp_metadata(encoded: &[u8]) -> Vec<u8> {
        let mut stripped = Vec::new();
        write_webp_metadata_stripped(encoded, &mut stripped).expect("write stripped webp");
        stripped
    }

    fn stripped_jpeg_metadata_streaming(encoded: &[u8], chunk_size: usize) -> Vec<u8> {
        let mut stripped = Vec::new();
        {
            let mut writer = JpegMetadataStrippingWriter::new(&mut stripped);
            for chunk in encoded.chunks(chunk_size) {
                writer
                    .write_all(chunk)
                    .expect("write streaming stripped jpeg chunk");
            }
            writer.finish().expect("finish streaming stripped jpeg");
        }
        stripped
    }

    fn stripped_png_metadata_streaming(encoded: &[u8], chunk_size: usize) -> Vec<u8> {
        let mut stripped = Vec::new();
        {
            let mut writer = PngMetadataStrippingWriter::new(&mut stripped);
            for chunk in encoded.chunks(chunk_size) {
                writer
                    .write_all(chunk)
                    .expect("write streaming stripped png chunk");
            }
            writer.finish().expect("finish streaming stripped png");
        }
        stripped
    }

    fn stripped_webp_metadata_streaming(encoded: &[u8], chunk_size: usize) -> Vec<u8> {
        let mut stripped = Cursor::new(Vec::new());
        {
            let mut writer = WebpMetadataStrippingWriter::new(&mut stripped);
            for chunk in encoded.chunks(chunk_size) {
                writer
                    .write_all(chunk)
                    .expect("write streaming stripped webp chunk");
            }
            writer.finish().expect("finish streaming stripped webp");
        }
        stripped.into_inner()
    }

    #[test]
    fn metadata_strippers_keep_original_when_encoded_bytes_are_invalid() {
        let invalid = b"not an encoded image";

        assert_eq!(stripped_jpeg_metadata(invalid), invalid.to_vec());
        assert_eq!(stripped_png_metadata(invalid), invalid.to_vec());
        assert_eq!(stripped_webp_metadata(invalid), invalid.to_vec());
    }

    #[test]
    fn jpeg_metadata_stripper_removes_app_and_comment_segments() {
        let encoded = vec![
            0xff, 0xd8, 0xff, 0xe1, 0x00, 0x04, b'e', b'x', 0xff, 0xfe, 0x00, 0x03, b'c', 0xff,
            0xdb, 0x00, 0x02, 0xff, 0xda, 0x00, 0x02, 0x11, 0xff, 0x00, 0xff, 0xd9,
        ];

        let stripped = stripped_jpeg_metadata(&encoded);

        assert_eq!(
            stripped,
            vec![
                0xff, 0xd8, 0xff, 0xdb, 0x00, 0x02, 0xff, 0xda, 0x00, 0x02, 0x11, 0xff, 0x00, 0xff,
                0xd9,
            ]
        );
    }

    #[test]
    fn streaming_jpeg_metadata_stripper_matches_buffered_stripper() {
        let encoded = vec![
            0xff, 0xd8, 0xff, 0xe1, 0x00, 0x04, b'e', b'x', 0xff, 0xfe, 0x00, 0x03, b'c', 0xff,
            0xdb, 0x00, 0x02, 0xff, 0xda, 0x00, 0x02, 0x11, 0xff, 0x00, 0xff, 0xd9,
        ];

        assert_eq!(
            stripped_jpeg_metadata_streaming(&encoded, 3),
            stripped_jpeg_metadata(&encoded)
        );
    }

    #[test]
    fn png_metadata_stripper_removes_ancillary_metadata_chunks() {
        let mut encoded = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        encoded.extend_from_slice(&png_test_chunk(b"IHDR", &[]));
        encoded.extend_from_slice(&png_test_chunk(b"tEXt", b"comment"));
        encoded.extend_from_slice(&png_test_chunk(b"tRNS", &[1]));
        encoded.extend_from_slice(&png_test_chunk(b"IDAT", &[2, 3]));
        encoded.extend_from_slice(&png_test_chunk(b"IEND", &[]));

        let stripped = stripped_png_metadata(&encoded);

        assert!(!contains_bytes(&stripped, b"tEXt"));
        assert!(contains_bytes(&stripped, b"tRNS"));
        assert!(contains_bytes(&stripped, b"IDAT"));
    }

    #[test]
    fn streaming_png_metadata_stripper_matches_buffered_stripper() {
        let mut encoded = vec![0x89, b'P', b'N', b'G', 0x0d, 0x0a, 0x1a, 0x0a];
        encoded.extend_from_slice(&png_test_chunk(b"IHDR", &[]));
        encoded.extend_from_slice(&png_test_chunk(b"tEXt", b"comment"));
        encoded.extend_from_slice(&png_test_chunk(b"tRNS", &[1]));
        encoded.extend_from_slice(&png_test_chunk(b"IDAT", &[2, 3]));
        encoded.extend_from_slice(&png_test_chunk(b"IEND", &[]));

        assert_eq!(
            stripped_png_metadata_streaming(&encoded, 5),
            stripped_png_metadata(&encoded)
        );
    }

    #[test]
    fn webp_metadata_stripper_removes_exif_xmp_and_iccp_chunks() {
        let encoded = webp_test_file(&[
            (b"VP8L", &[1, 2, 3]),
            (b"EXIF", b"exif"),
            (b"XMP ", b"xmp"),
            (b"ICCP", b"icc"),
        ]);

        let stripped = stripped_webp_metadata(&encoded);

        assert!(contains_bytes(&stripped, b"VP8L"));
        assert!(!contains_bytes(&stripped, b"EXIF"));
        assert!(!contains_bytes(&stripped, b"XMP "));
        assert!(!contains_bytes(&stripped, b"ICCP"));
        assert_eq!(
            u32::from_le_bytes([stripped[4], stripped[5], stripped[6], stripped[7]]) as usize,
            stripped.len() - 8
        );
    }

    #[test]
    fn streaming_webp_metadata_stripper_matches_buffered_stripper() {
        let encoded = webp_test_file(&[
            (b"VP8L", &[1, 2, 3]),
            (b"EXIF", b"exif"),
            (b"ALPH", &[4, 5]),
            (b"XMP ", b"xmp"),
            (b"VP8 ", &[6, 7, 8, 9]),
        ]);

        assert_eq!(
            stripped_webp_metadata_streaming(&encoded, 3),
            stripped_webp_metadata(&encoded)
        );
    }

    #[test]
    fn failed_export_keeps_existing_target_and_removes_temporary_file() {
        let dir = unique_temp_dir("export-failure-preserves-target");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("existing.jpg");
        let original = b"original target bytes";
        fs::write(&path, original).expect("write original target");
        let empty_image = Rgba8Image::new(0, 1, Vec::new());

        let error = export_rgba8_image(
            &path,
            &empty_image,
            ExportOptions::new(ExportFormat::Jpeg, None),
        )
        .expect_err("empty image should fail during export encoding");

        assert_eq!(error.category(), ExportSaveErrorCategory::EncodingFailed);
        assert_eq!(fs::read(&path).expect("read original target"), original);
        assert_export_temporary_files_removed(&dir, "existing.jpg");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jpeg_quality_changes_encoded_file_size() {
        let dir = unique_temp_dir("jpeg-quality-size");
        fs::create_dir_all(&dir).expect("test dir");
        let source = varied_rgba8(96, 64);
        let low_quality = dir.join("q10.jpg");
        let high_quality = dir.join("q95.jpg");

        export_rgba8_image(
            &low_quality,
            &source,
            ExportOptions::new(ExportFormat::Jpeg, Some(10)),
        )
        .expect("export low-quality jpeg");
        export_rgba8_image(
            &high_quality,
            &source,
            ExportOptions::new(ExportFormat::Jpeg, Some(95)),
        )
        .expect("export high-quality jpeg");

        let low_size = fs::metadata(&low_quality).expect("low metadata").len();
        let high_size = fs::metadata(&high_quality).expect("high metadata").len();
        assert!(high_size > low_size, "low={low_size}, high={high_size}");

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn flatten_rgba8_to_rgb8_in_place_reuses_buffer_and_matches_allocating_path() {
        let path = Path::new("jpeg-rgba8-flatten");
        let source = Rgba8Image::new(
            2,
            2,
            vec![
                200, 10, 30, 0, 80, 160, 240, 128, 7, 9, 11, 255, 250, 100, 50, 64,
            ],
        );
        let background = ExportOptions::new(ExportFormat::Jpeg, None).jpeg_alpha_background_rgb();
        let expected =
            flatten_rgba8_to_rgb8(path, &source, background).expect("allocating flatten");
        let mut actual = source.into_raw();
        let capacity = actual.capacity();

        flatten_rgba8_to_rgb8_in_place(path, &mut actual, background).expect("in-place flatten");

        assert_eq!(actual, expected);
        assert_eq!(actual.capacity(), capacity);
    }

    #[test]
    #[ignore = "allocates a large fixture for manual memory smoke testing"]
    fn large_bmp_fixture_loads_preview_then_full_resolution_on_demand() {
        let dir = unique_temp_dir("large-smoke");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("large-7000x4000.bmp");
        let source_size = ImageSize::new(7000, 4000);
        let fixture = large_gradient_rgba8(source_size.width(), source_size.height());
        export_rgba8_image(&path, &fixture, ExportOptions::new(ExportFormat::Bmp, None))
            .expect("write large bmp fixture");
        drop(fixture);

        let preview = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(1000, 500),
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("load large image preview");

        assert_eq!(preview.source_size(), source_size);
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(1750, 1000));
        assert!(preview.pixels().byte_len() < source_size.rgba8_byte_len().expect("byte len"));
        drop(preview);

        let full = load_full_resolution_image(&path, DEFAULT_IMAGE_MEMORY_POLICY, None)
            .expect("load full-resolution image on demand");
        assert_eq!(full.size(), source_size);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn large_static_bmp_preview_does_not_retain_full_resolution_cache() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("large-bmp-preview-cache");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.bmp");
        let source = varied_rgba8(32, 16);
        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Bmp, None))
            .expect("write bmp fixture");

        let mut settings = MemoryPolicySettings::DEFAULT;
        settings.set_large_image_pixel_threshold(1);
        settings.set_preview_oversample(1);
        settings.set_preview_max_pixels(64);
        let preview = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(8, 4),
            settings.image_memory_policy(),
            None,
        )
        .expect("load bmp preview");

        assert_eq!(preview.source_size(), ImageSize::new(32, 16));
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(8, 4));
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn jpeg_viewport_preview_loads_before_full_resolution_on_demand() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("jpeg-preview-first");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.jpg");
        let source = varied_rgba8(256, 1024);
        export_rgba8_image(
            &path,
            &source,
            ExportOptions::new(ExportFormat::Jpeg, Some(90)),
        )
        .expect("write jpeg fixture");

        let mut profiler = ImageOpenProfiler::new();
        let profiled_preview = load_image_file_for_view_with_timing_and_profile(
            &path,
            ViewportSize::from_client_size(64, 64),
            DEFAULT_IMAGE_MEMORY_POLICY,
            AnimationTimingSettings::default(),
            None,
            &mut profiler,
        )
        .expect("load profiled jpeg preview");
        let profile = profiler.finish();
        let stage_names = profile
            .stages()
            .iter()
            .map(|stage| stage.name())
            .collect::<Vec<_>>();
        assert_eq!(profiled_preview.pixels().pixel_format(), PixelFormat::Rgb8);
        assert!(!stage_names.contains(&"static.convert_to_rgba8"));
        let preview_begin_index = stage_names
            .iter()
            .position(|stage| *stage == "static.decode_preview_pixels.begin")
            .expect("preview begin stage");
        let open_index = stage_names
            .iter()
            .position(|stage| *stage == "static.jpeg_preview.open_file_decoder")
            .expect("jpeg preview open stage");
        let scale_index = stage_names
            .iter()
            .position(|stage| *stage == "static.jpeg_preview.scale_decoder")
            .expect("jpeg preview scale stage");
        let decode_index = stage_names
            .iter()
            .position(|stage| *stage == "static.jpeg_preview.decode_scaled_pixels")
            .expect("jpeg preview decode stage");
        let sample_index = stage_names
            .iter()
            .position(|stage| *stage == "static.jpeg_preview.sample_to_target")
            .expect("jpeg preview sample stage");
        let preview_complete_index = stage_names
            .iter()
            .position(|stage| *stage == "static.decode_preview_pixels.complete")
            .expect("preview complete stage");
        assert!(preview_begin_index < open_index);
        assert!(open_index < scale_index);
        assert!(scale_index < decode_index);
        assert!(decode_index < sample_index);
        assert!(sample_index < preview_complete_index);
        drop(profiled_preview);

        let preview = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(64, 64),
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("load jpeg preview");

        assert_eq!(preview.source_size(), source.size());
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().pixel_format(), PixelFormat::Rgb8);
        assert_eq!(preview.pixels().size(), ImageSize::new(32, 128));
        assert!(preview.pixels().byte_len() < source.byte_len());
        assert!(lock_static_full_resolution_cache().is_none());
        drop(preview);

        let full = load_full_resolution_image(&path, DEFAULT_IMAGE_MEMORY_POLICY, None)
            .expect("full-resolution jpeg should load on demand");
        assert_eq!(full.pixel_format(), PixelFormat::Rgb8);
        assert_eq!(full.size(), source.size());
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn profiled_static_full_decode_records_pixel_conversion_stage() {
        let dir = unique_temp_dir("profile-static-conversion");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("full.png");
        write_export_fixture(&path, ExportFormat::Png);

        let mut profiler = ImageOpenProfiler::new();
        let image = load_image_file_for_view_with_timing_and_profile(
            &path,
            ViewportSize::from_client_size(64, 64),
            DEFAULT_IMAGE_MEMORY_POLICY,
            AnimationTimingSettings::default(),
            None,
            &mut profiler,
        )
        .expect("load profiled png");
        let profile = profiler.finish();
        let stage_names = profile
            .stages()
            .iter()
            .map(|stage| stage.name())
            .collect::<Vec<_>>();
        let decode_index = stage_names
            .iter()
            .position(|stage| *stage == "static.decode_pixels")
            .expect("decode stage");
        let conversion_index = stage_names
            .iter()
            .position(|stage| *stage == "static.convert_to_pixel_image")
            .expect("conversion stage");

        assert!(conversion_index > decode_index);
        assert!(image.pixels().is_valid());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn large_static_png_sampled_preview_does_not_retain_full_resolution_cache() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("large-png-sampled-preview");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.png");
        let source = varied_rgba8(32, 16);
        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Png, None))
            .expect("write png fixture");

        let mut settings = MemoryPolicySettings::DEFAULT;
        settings.set_large_image_pixel_threshold(1);
        settings.set_preview_oversample(1);
        settings.set_preview_max_pixels(64);
        let preview = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(8, 4),
            settings.image_memory_policy(),
            None,
        )
        .expect("load png preview");

        assert_eq!(preview.source_size(), ImageSize::new(32, 16));
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(8, 4));
        assert_eq!(
            preview.pixels().pixels(),
            expected_sampled_rgba8_pixels(&source, ImageSize::new(8, 4)).as_slice()
        );
        assert!(lock_static_full_resolution_cache().is_none());
        drop(preview);

        let full = load_full_resolution_image(&path, settings.image_memory_policy(), None)
            .expect("full-resolution image should load on demand");
        assert_eq!(full.size(), source.size());
        assert_eq!(full.pixels(), source.pixels());
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sampled_static_preview_rejects_source_decode_above_preview_alloc_limit() {
        let dir = unique_temp_dir("sampled-preview-source-limit");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.png");
        let source = varied_rgba8(2, 2);
        let target_size = ImageSize::new(1, 1);
        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Png, None))
            .expect("write png fixture");

        let preview_max_alloc_bytes = target_size.rgba8_byte_len().expect("preview byte length");
        let source_max_alloc_bytes = source.pixels().len() * 2;
        assert!(source.pixels().len() > preview_max_alloc_bytes);

        let decoder = open_image_decoder(
            &path,
            SupportedImageFormat::Png,
            decode_limits(source_max_alloc_bytes),
            None,
        )
        .expect("open png decoder");
        let error = decode_sampled_static_preview_pixel_image_from_decoder(
            &path,
            decoder,
            4,
            preview_max_alloc_bytes,
            source_max_alloc_bytes,
            target_size,
            source.size(),
            true,
            None,
        )
        .expect_err("full-source fallback should stay within the preview allocation limit");
        assert!(matches!(error, LoadImageError::ImageTooLarge { .. }));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sampled_static_png_preview_uses_reader_from_reused_metadata_source() {
        let dir = unique_temp_dir("sampled-png-preview-source-reuse");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.png");
        let source = varied_rgba8(32, 16);
        let target_size = ImageSize::new(8, 4);
        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Png, None))
            .expect("write png fixture");

        let (_metadata, mut metadata_decoder, preview_source) =
            open_image_decoder_with_metadata_and_source(
                &path,
                SupportedImageFormat::Png,
                decode_limits(DEFAULT_IMAGE_MEMORY_POLICY.max_transient_decode_bytes()),
                None,
            )
            .expect("open png metadata decoder and preview source");
        assert_eq!(image_decoder_size(&metadata_decoder), source.size());
        assert_eq!(
            read_decoder_exif_orientation(&path, &mut metadata_decoder, None)
                .expect("read png orientation"),
            ImageOrientation::NORMAL
        );

        let preview_max_alloc_bytes = target_size.rgba8_byte_len().expect("preview byte length");
        let source_max_alloc_bytes = source.pixels().len() / 2;
        let preview = decode_sampled_static_png_preview_pixel_image(
            &path,
            preview_source.reader(),
            preview_max_alloc_bytes,
            source_max_alloc_bytes,
            target_size,
            source.size(),
            true,
            None,
        )
        .expect("stream sampled png preview from shared source")
        .expect("non-interlaced rgba8 png should stream");
        let PixelImage::Rgba8(preview) = preview else {
            panic!("rgba8 png should stay rgba8");
        };

        assert_eq!(preview.size(), target_size);
        assert_eq!(
            preview.pixels(),
            expected_sampled_rgba8_pixels(&source, target_size).as_slice()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sampled_static_png_preview_streams_rows_below_full_source_limit() {
        let dir = unique_temp_dir("sampled-png-preview-row-stream");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview.png");
        let source = varied_rgba8(32, 16);
        let target_size = ImageSize::new(8, 4);
        export_rgba8_image(&path, &source, ExportOptions::new(ExportFormat::Png, None))
            .expect("write png fixture");

        let preview_max_alloc_bytes = target_size.rgba8_byte_len().expect("preview byte length");
        let source_max_alloc_bytes = source.pixels().len() / 2;
        assert!(source.pixels().len() > source_max_alloc_bytes);

        let (_metadata, preview_source) = open_cancelable_file_source_with_metadata(&path, None)
            .expect("open png preview source");
        let preview = decode_sampled_static_png_preview_pixel_image(
            &path,
            preview_source.reader(),
            preview_max_alloc_bytes,
            source_max_alloc_bytes,
            target_size,
            source.size(),
            true,
            None,
        )
        .expect("stream sampled png preview")
        .expect("non-interlaced rgba8 png should stream");
        let PixelImage::Rgba8(preview) = preview else {
            panic!("rgba8 png should stay rgba8");
        };

        assert_eq!(preview.size(), target_size);
        assert_eq!(
            preview.pixels(),
            expected_sampled_rgba8_pixels(&source, target_size).as_slice()
        );

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sampled_static_preview_rgb8_returns_rgb8_output() {
        let path = Path::new("rgb8-preview");
        let source_size = ImageSize::new(4, 4);
        let target_size = ImageSize::new(2, 2);
        let mut source = Vec::new();
        for y in 0..source_size.height() {
            for x in 0..source_size.width() {
                source.extend_from_slice(&[
                    (x * 40) as u8,
                    (y * 50) as u8,
                    x.wrapping_add(y) as u8,
                ]);
            }
        }

        let preview = sampled_static_preview_pixel_image_from_decoded(
            path,
            source,
            ColorType::Rgb8,
            3,
            source_size,
            target_size,
            target_size
                .pixel_byte_len(PixelFormat::Rgb8)
                .expect("rgb8 preview byte length"),
            true,
            None,
        )
        .expect("sample rgb8 preview");
        let PixelImage::Rgb8(preview) = preview else {
            panic!("rgb8 sampled preview should stay rgb8");
        };

        assert_eq!(preview.size(), target_size);
        assert_eq!(
            preview.pixels(),
            &[0, 0, 0, 80, 0, 2, 0, 100, 2, 80, 100, 4,]
        );
    }

    #[test]
    fn sampled_static_preview_rgba8_returns_target_sized_buffer() {
        let path = Path::new("rgba8-preview");
        let source = varied_rgba8(32, 32);
        let source_size = source.size();
        let target_size = ImageSize::new(2, 2);
        let expected = expected_sampled_rgba8_pixels(&source, target_size);
        let source_byte_len = source.pixels().len();

        let preview = sampled_static_preview_rgba8_from_decoded(
            path,
            source.pixels().to_vec(),
            ColorType::Rgba8,
            4,
            source_size,
            target_size,
            target_size
                .rgba8_byte_len()
                .expect("rgba8 preview byte length"),
            None,
        )
        .expect("sample rgba8 preview");

        assert_eq!(preview.size(), target_size);
        let preview_pixels = preview.into_raw();
        assert_eq!(preview_pixels, expected);
        assert!(preview_pixels.capacity() < source_byte_len);
    }

    #[test]
    fn large_static_png_fallback_preview_reuses_full_resolution_cache() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("large-png-fallback-preview-cache");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview-rgba16.png");
        let source = binary_rgba8(32, 16);
        write_rgba16_png_fixture(&path, &source);

        let mut settings = MemoryPolicySettings::DEFAULT;
        settings.set_large_image_pixel_threshold(1);
        settings.set_preview_oversample(1);
        settings.set_preview_max_pixels(64);
        let preview = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(8, 4),
            settings.image_memory_policy(),
            None,
        )
        .expect("load fallback png preview");

        assert_eq!(preview.source_size(), ImageSize::new(32, 16));
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(8, 4));
        {
            let cache = lock_static_full_resolution_cache();
            let entry = cache
                .as_ref()
                .expect("fallback preview should retain full-resolution cache");
            assert_eq!(entry.source_key.path, path);
            assert_eq!(entry.source_key.format, SupportedImageFormat::Png);
            assert_eq!(entry.source_size, source.size());
        }
        drop(preview);

        let full = load_full_resolution_image(&path, settings.image_memory_policy(), None)
            .expect("full-resolution image should load from retained cache");
        assert_eq!(full.size(), source.size());
        assert_eq!(full.pixels(), source.pixels());
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn large_static_png_fallback_preview_drops_full_resolution_cache_over_cache_entry_limit() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("large-png-fallback-preview-cache-entry-limit");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview-rgba16.png");
        let source = binary_rgba8(513, 513);
        write_rgba16_png_fixture(&path, &source);

        let mut settings = MemoryPolicySettings::DEFAULT;
        settings.set_large_image_pixel_threshold(1);
        settings.set_preview_oversample(1);
        settings.set_preview_max_pixels(64);
        settings.set_max_cache_entry_mib(1);
        let policy = settings.image_memory_policy();
        assert!(source.pixels().len() > policy.max_cache_entry_bytes());
        assert!(source.pixels().len() <= policy.max_full_resolution_bytes());

        let preview =
            load_image_file_for_view(&path, ViewportSize::from_client_size(8, 8), policy, None)
                .expect("load fallback png preview");

        assert_eq!(preview.source_size(), source.size());
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(8, 8));
        assert!(lock_static_full_resolution_cache().is_none());
        drop(preview);

        let full = load_full_resolution_image(&path, policy, None)
            .expect("full-resolution image should load on demand");
        assert_eq!(full.size(), source.size());
        assert_eq!(full.pixels(), source.pixels());
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn large_static_png_fallback_preview_drops_full_resolution_cache_over_resident_limit() {
        let _cache_guard = STATIC_FULL_RESOLUTION_CACHE_TEST_MUTEX
            .lock()
            .expect("static full-resolution cache test mutex");
        replace_static_full_resolution_cache(None);

        let dir = unique_temp_dir("large-png-fallback-preview-resident-limit");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview-rgba16.png");
        let source = binary_rgba8(512, 512);
        write_rgba16_png_fixture(&path, &source);

        let mut settings = MemoryPolicySettings::DEFAULT;
        settings.set_large_image_pixel_threshold(1);
        settings.set_preview_oversample(1);
        settings.set_preview_max_pixels(64);
        settings.set_max_resident_mib(2);
        settings.set_max_cache_entry_mib(2);
        let policy = settings.image_memory_policy();
        let rgba16_byte_len = source.pixels().len() * 2;
        assert_eq!(rgba16_byte_len, policy.max_cache_entry_bytes());
        assert_eq!(rgba16_byte_len, policy.max_resident_bytes());

        let preview =
            load_image_file_for_view(&path, ViewportSize::from_client_size(8, 8), policy, None)
                .expect("load fallback png preview");

        assert_eq!(preview.source_size(), source.size());
        assert_eq!(preview.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(preview.pixels().size(), ImageSize::new(8, 8));
        assert!(preview.pixels().byte_len() > 0);
        assert!(lock_static_full_resolution_cache().is_none());
        drop(preview);

        let full = load_full_resolution_image(&path, policy, None)
            .expect("full-resolution image should load on demand");
        assert_eq!(full.size(), source.size());
        assert_eq!(full.pixels(), source.pixels());
        assert!(lock_static_full_resolution_cache().is_none());

        replace_static_full_resolution_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn static_preview_decode_matches_rgba8_downscale_output() {
        let dir = unique_temp_dir("static-preview-decode");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("preview-source.jpg");
        let source = varied_rgba8(96, 64);
        let target_size = ImageSize::new(24, 16);

        export_rgba8_image(
            &path,
            &source,
            ExportOptions::new(ExportFormat::Jpeg, Some(90)),
        )
        .expect("write jpeg fixture");

        let preview = decode_preview_rgba8_image(
            &path,
            DEFAULT_IMAGE_MEMORY_POLICY.max_transient_decode_bytes(),
            target_size,
            None,
        )
        .expect("decode preview before rgba8 conversion");
        let full_rgba8 = decode_rgba8_image(
            &path,
            DEFAULT_IMAGE_MEMORY_POLICY.max_transient_decode_bytes(),
            None,
        )
        .expect("decode full rgba8 fixture");
        let expected = downscale_rgba8(&path, full_rgba8, target_size, None)
            .expect("downscale full rgba8 fixture");

        assert_eq!(preview.size(), target_size);
        assert_eq!(preview.pixels(), expected.pixels());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn oversized_bmp_header_is_rejected_before_pixel_decode() {
        let dir = unique_temp_dir("oversized-header");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("huge-header.bmp");
        write_bmp_header_only_fixture(&path, 20_000, 10_000);

        let error = load_image_file_for_view(
            &path,
            ViewportSize::from_client_size(1000, 800),
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect_err("oversized dimensions should be rejected before decode");

        assert_eq!(
            error.category(),
            ImageOpenErrorCategory::ImageTooLargeOrOutOfMemory
        );
        match error {
            LoadImageError::ImageTooLarge { size, .. } => {
                assert_eq!(size, ImageSize::new(20_000, 10_000));
            }
            unexpected => panic!("unexpected oversized image error: {unexpected:?}"),
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_jpeg_exif_orientation_fixtures_load_metadata_values_1_through_8() {
        let dir = unique_temp_dir("exif-orientation");
        fs::create_dir_all(&dir).expect("test dir");

        for value in 1..=8 {
            let path = dir.join(format!("orientation-{value}.jpg"));
            write_jpeg_exif_orientation_fixture(&path, value);

            let image = load_image_file(&path).expect("EXIF fixture should load");
            let orientation =
                ImageOrientation::from_exif_value(value).expect("fixture orientation");

            assert_eq!(image.metadata().format(), SupportedImageFormat::Jpeg);
            assert_eq!(image.metadata().exif_orientation(), orientation);
            assert_eq!(image.source_size(), ImageSize::new(3, 2));
            assert_eq!(image.pixels().size(), ImageSize::new(3, 2));
            assert_eq!(
                image.source_size().with_orientation(orientation),
                ImageSize::new(3, 2).with_orientation(orientation)
            );
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn generated_animated_gif_fixture_preserves_frame_order_delays_and_loop() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let dir = unique_temp_dir("animated-gif");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("frames.gif");
        write_animated_gif_fixture(&path);

        let image = load_image_file(&path).expect("animated GIF should load");

        assert_eq!(image.metadata().format(), SupportedImageFormat::Gif);
        assert!(image.is_animated());
        assert_eq!(image.pixels().pixels(), gif_frame_pixels(0));
        let playback = image.animation().expect("animation playback");
        assert_eq!(playback.frame_delays_ms(), &[30, 70, 110]);
        assert_eq!(playback.loop_policy(), AnimationLoopPolicy::finite(2));

        let second = load_animation_frame_for_view(
            &path,
            1,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("second GIF frame");
        let third = load_animation_frame_for_view(
            &path,
            2,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("third GIF frame");

        assert_eq!(second.pixels(), gif_frame_pixels(1));
        assert_eq!(third.pixels(), gif_frame_pixels(2));
        replace_animation_frame_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parallel_metadata_gif_fixture_preserves_frame_order_delays_and_loop() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let dir = unique_temp_dir("parallel-metadata-gif");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("frames.gif");
        write_animated_gif_fixture(&path);
        let file_metadata = fs::metadata(&path).expect("animated gif metadata");

        let decoded = decode_gif_animation_image_with_parallel_metadata(
            &path,
            AnimationSourceCacheKey::new(&path, SupportedImageFormat::Gif, &file_metadata),
            AnimationDecodeContext {
                viewport: ViewportSize::EMPTY,
                policy: DEFAULT_IMAGE_MEMORY_POLICY,
                animation_timing: AnimationTimingSettings::default(),
                cancel: None,
            },
        )
        .expect("parallel metadata GIF should load");

        assert_eq!(decoded.first_frame.pixels(), gif_frame_pixels(0));
        let playback = decoded.playback.expect("animation playback");
        assert_eq!(playback.frame_delays_ms(), &[30, 70, 110]);
        assert_eq!(playback.loop_policy(), AnimationLoopPolicy::finite(2));

        replace_animation_frame_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn animation_metadata_initial_decode_stops_after_initial_cache_window() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("metadata-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let metadata = AnimationMetadata {
            frame_delays_ms: vec![30; 100],
        };

        let decoded = collect_animation_decode_result(
            path,
            ImageProbe {
                source_size: frame_size,
                exif_orientation: ImageOrientation::NORMAL,
            },
            AnimationDecodeContext {
                viewport: ViewportSize::EMPTY,
                policy: DEFAULT_IMAGE_MEMORY_POLICY,
                animation_timing: AnimationTimingSettings::default(),
                cancel: None,
            },
            Some(source_key),
            AnimationLoopPolicy::Infinite,
            BoundedAnimationFrames::new(frame_size, 100, ANIMATION_INITIAL_CACHE_FRAME_LIMIT),
            Some(metadata),
        )
        .expect("metadata path should decode only the initial cache window");

        assert_eq!(decoded.first_frame.pixels(), animation_frame_color(0));
        assert_eq!(
            decoded
                .playback
                .as_ref()
                .map(AnimationPlayback::frame_count),
            Some(100)
        );
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("initial metadata path should cache");
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frame_count, Some(100));
            assert_eq!(entry.frames.len(), ANIMATION_INITIAL_CACHE_FRAME_LIMIT);
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_miss_populates_requested_frame_window() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let dir = unique_temp_dir("animation-frame-cache-miss");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("frames.gif");
        write_animated_gif_fixture(&path);

        let third = load_animation_frame_for_view(
            &path,
            2,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("third GIF frame");

        assert_eq!(third.pixels(), gif_frame_pixels(2));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("frame request should populate cache");
            assert_eq!(entry.source_key.path.as_path(), path.as_path());
            assert_eq!(entry.source_key.format, SupportedImageFormat::Gif);
            assert_eq!(entry.source_size, ImageSize::new(2, 1));
            assert_eq!(entry.target_size, ImageSize::new(2, 1));
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frame_count, Some(3));
            assert_eq!(entry.frames.len(), 3);
            assert_eq!(entry.frames[0].pixels(), gif_frame_pixels(0));
            assert_eq!(entry.frames[1].pixels(), gif_frame_pixels(1));
            assert_eq!(entry.frames[2].pixels(), gif_frame_pixels(2));
        }

        replace_animation_frame_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn animation_frame_decode_stops_after_requested_cache_window() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("long-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };

        let second = decode_animation_frame_from_iter(
            path,
            1,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, 100, ANIMATION_INITIAL_CACHE_FRAME_LIMIT + 1),
            ANIMATION_FRAME_CACHE_RADIUS,
            None,
            None,
        )
        .expect("second frame should decode without consuming beyond cache window");
        let second = expect_returned_animation_frame(second);

        assert_eq!(second.pixels(), animation_frame_color(1));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("requested window should be cached");
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frame_count, None);
            assert_eq!(entry.frames.len(), ANIMATION_INITIAL_CACHE_FRAME_LIMIT + 1);
            for (index, frame) in entry.frames.iter().enumerate() {
                assert_eq!(frame.pixels(), animation_frame_color(index));
            }
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_miss_extends_forward_cache_window() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("prefetch-long-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let requested_index = 10usize;
        let expected_cache_start = requested_index.saturating_sub(ANIMATION_FRAME_CACHE_RADIUS);
        let expected_cache_end =
            requested_index.saturating_add(ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT);

        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, 100, expected_cache_end + 1),
            ANIMATION_FRAME_PREFETCH_RADIUS,
            None,
            None,
            None,
            None,
        )
        .expect("prefetch miss should decode through the extended forward cache window");
        let requested = expect_returned_animation_frame(requested);

        assert_eq!(requested.pixels(), animation_frame_color(requested_index));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("prefetch window should be cached");
            assert_eq!(entry.start_index, expected_cache_start);
            assert_eq!(
                entry.frames.len(),
                expected_cache_end - expected_cache_start + 1
            );
            assert_eq!(
                entry
                    .shared_frame(expected_cache_end)
                    .expect("last prefetched frame")
                    .pixels(),
                animation_frame_color(expected_cache_end)
            );
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_delivery_extends_to_post_delivery_budget() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("prefetch-delivered-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let requested_index = 10usize;
        let expected_cache_start = requested_index.saturating_sub(ANIMATION_FRAME_CACHE_RADIUS);
        let post_delivery_limit = animation_frame_delivered_prefetch_limit(
            frame_size,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        assert_eq!(
            post_delivery_limit,
            ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT
        );
        let expected_cache_end = requested_index.saturating_add(post_delivery_limit);
        let mut delivered = None;
        let mut on_requested_frame = |frame: AnimationFramePixels| {
            delivered = Some(frame.into_rgba8());
            true
        };

        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, 100, expected_cache_end + 1),
            ANIMATION_FRAME_PREFETCH_RADIUS,
            None,
            Some(&mut on_requested_frame),
            None,
            None,
        )
        .expect("delivered prefetch should extend to the post-delivery budget");

        assert!(matches!(requested, AnimationFrameRequestResult::Delivered));
        let delivered = delivered.expect("requested frame should be delivered");
        assert_eq!(delivered.pixels(), animation_frame_color(requested_index));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache
                .as_ref()
                .expect("extended prefetch window should cache");
            assert_eq!(entry.start_index, expected_cache_start);
            assert_eq!(
                entry.frames.len(),
                expected_cache_end - expected_cache_start + 1
            );
            assert_eq!(
                entry
                    .shared_frame(expected_cache_end)
                    .expect("last post-delivery prefetched frame")
                    .pixels(),
                animation_frame_color(expected_cache_end)
            );
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_delivery_reports_follow_up_frame() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("prefetch-follow-up-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let requested_index = 10usize;
        let follow_up_index = requested_index + 2;
        let mut delivered = None;
        let mut follow_up = None;
        let mut on_requested_frame = |frame: AnimationFramePixels| {
            delivered = Some(frame.into_rgba8());
            true
        };
        let mut on_prefetched_frame = |index, frame: AnimationFramePixels| {
            if index == follow_up_index {
                follow_up = Some(frame.into_rgba8());
                return false;
            }
            true
        };

        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, 100, follow_up_index + 1),
            ANIMATION_FRAME_PREFETCH_RADIUS,
            None,
            Some(&mut on_requested_frame),
            Some(&mut on_prefetched_frame),
            None,
        )
        .expect("prefetch should report follow-up frames after delivery");

        assert!(matches!(requested, AnimationFrameRequestResult::Delivered));
        assert_eq!(
            delivered.expect("requested frame").pixels(),
            animation_frame_color(requested_index)
        );
        assert_eq!(
            follow_up.expect("follow-up frame").pixels(),
            animation_frame_color(follow_up_index)
        );

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_delivery_continues_without_cache_key() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("prefetch-follow-up-without-cache-key.gif");
        let requested_index = 10usize;
        let follow_up_index = requested_index + 2;
        let mut delivered = None;
        let mut follow_up = None;
        let mut on_requested_frame = |frame: AnimationFramePixels| {
            delivered = Some(frame.into_rgba8());
            true
        };
        let mut on_prefetched_frame = |index, frame: AnimationFramePixels| {
            if index == follow_up_index {
                follow_up = Some(frame.into_rgba8());
                return false;
            }
            true
        };

        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
            BoundedAnimationFrames::new(frame_size, 100, follow_up_index + 1),
            ANIMATION_FRAME_PREFETCH_RADIUS,
            None,
            Some(&mut on_requested_frame),
            Some(&mut on_prefetched_frame),
            None,
        )
        .expect("prefetch should report follow-up frames without a resident cache key");

        assert!(matches!(requested, AnimationFrameRequestResult::Delivered));
        assert_eq!(
            delivered.expect("requested frame").pixels(),
            animation_frame_color(requested_index)
        );
        assert_eq!(
            follow_up.expect("follow-up frame").pixels(),
            animation_frame_color(follow_up_index)
        );
        assert!(lock_animation_frame_cache().is_none());

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_coverage_uses_active_window_without_resident_cache() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("prefetch-coverage-without-cache.gif");
        let active_index = 10usize;
        let prefetch_limit = animation_frame_delivered_prefetch_limit(
            frame_size,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        assert!(prefetch_limit > 0);

        assert!(animation_frame_prefetch_for_loaded_image_covers(
            path,
            ImageFileVersion::new(1, UNIX_EPOCH),
            SupportedImageFormat::Gif,
            frame_size,
            active_index,
            active_index + prefetch_limit,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
        ));
        assert!(!animation_frame_prefetch_for_loaded_image_covers(
            path,
            ImageFileVersion::new(1, UNIX_EPOCH),
            SupportedImageFormat::Gif,
            frame_size,
            active_index,
            active_index + prefetch_limit + 1,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
        ));

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_prefetch_delivery_budget_scales_with_frame_size() {
        let small_frame = ImageSize::new(1, 1);
        assert_eq!(
            animation_frame_delivered_prefetch_limit(small_frame, usize::MAX),
            ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT
        );

        let large_frame = ImageSize::new(2048, 2048);
        assert_eq!(
            animation_frame_delivered_prefetch_limit(large_frame, usize::MAX),
            0
        );
    }

    #[test]
    fn animation_frame_prefetch_miss_extends_reused_cache_to_budget() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let mut memory = MemoryPolicySettings::default();
        memory.set_max_transient_decode_mib(64);
        memory.set_max_resident_mib(2);
        memory.set_max_cache_entry_mib(1);
        let policy = memory.image_memory_policy();
        let frame_size = ImageSize::new(48, 48);
        let frame_byte_len = frame_size.rgba8_byte_len().expect("frame byte length");
        let frame_pixel_count = frame_byte_len / 4;
        let cache_frame_capacity = animation_frame_cache_byte_limit(policy) / frame_byte_len;
        assert!(cache_frame_capacity > ANIMATION_SEQUENTIAL_CACHE_EXTENSION_LIMIT);
        let expected_cache_end = cache_frame_capacity
            .checked_sub(1)
            .expect("cache fits at least one frame");
        let frame_count = expected_cache_end + 16;

        let path = Path::new("prefetch-reused-budget-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let mut initial_cache = AnimationFrameCacheBuilder::new(
            source_key.clone(),
            frame_size,
            frame_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(policy),
        );
        for index in 0..ANIMATION_INITIAL_CACHE_FRAME_LIMIT {
            assert!(initial_cache.push(Rgba8Image::new(
                frame_size.width(),
                frame_size.height(),
                animation_frame_color(index).repeat(frame_pixel_count)
            )));
        }
        initial_cache.set_frame_count(frame_count);
        replace_animation_frame_cache(initial_cache.finish());

        let reusable_cache = reusable_animation_frame_cache_for_view(
            path,
            &source_key,
            ViewportSize::EMPTY,
            policy,
            None,
        )
        .expect("reusable cache lookup")
        .expect("initial prefix frame cache");
        let requested_index = ANIMATION_INITIAL_CACHE_FRAME_LIMIT;

        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            policy,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, frame_count, expected_cache_end + 1),
            ANIMATION_FRAME_PREFETCH_RADIUS,
            Some(reusable_cache),
            None,
            None,
            None,
        )
        .expect("prefetch miss should extend reused cache to the resident budget");
        let requested = expect_returned_animation_frame(requested);

        assert_eq!(
            &requested.pixels()[..4],
            animation_frame_color(requested_index).as_slice()
        );
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("budget-sized prefetch should cache");
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frames.len(), cache_frame_capacity);
            assert_eq!(
                &entry
                    .shared_frame(expected_cache_end)
                    .expect("last budget-prefetched frame")
                    .pixels()[..4],
                animation_frame_color(expected_cache_end).as_slice()
            );
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_miss_completes_known_animation_when_it_fits() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("known-small-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let frame_count = 12usize;
        let mut initial_cache = AnimationFrameCacheBuilder::new(
            source_key.clone(),
            frame_size,
            frame_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        for index in 0..ANIMATION_INITIAL_CACHE_FRAME_LIMIT {
            assert!(initial_cache.push(Rgba8Image::new(
                1,
                1,
                animation_frame_color(index).to_vec()
            )));
        }
        initial_cache.set_frame_count(frame_count);
        replace_animation_frame_cache(initial_cache.finish());

        let reusable_cache = reusable_animation_frame_cache_for_view(
            path,
            &source_key,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("reusable cache lookup")
        .expect("initial known frame cache");
        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            10,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, frame_count, frame_count + 1),
            ANIMATION_FRAME_CACHE_RADIUS,
            Some(reusable_cache),
            None,
            None,
            None,
        )
        .expect("known small animation should decode to the end");
        let requested = expect_returned_animation_frame(requested);

        assert_eq!(requested.pixels(), animation_frame_color(10));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("full known animation should cache");
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frame_count, Some(frame_count));
            assert_eq!(entry.frames.len(), frame_count);
            for (index, frame) in entry.frames.iter().enumerate() {
                assert_eq!(frame.pixels(), animation_frame_color(index));
            }
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_miss_extends_reused_prefix_when_it_fits() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("long-prefix-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let mut initial_cache = AnimationFrameCacheBuilder::new(
            source_key.clone(),
            frame_size,
            frame_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        for index in 0..ANIMATION_INITIAL_CACHE_FRAME_LIMIT {
            assert!(initial_cache.push(Rgba8Image::new(
                1,
                1,
                animation_frame_color(index).to_vec()
            )));
        }
        replace_animation_frame_cache(initial_cache.finish());

        let reusable_cache = reusable_animation_frame_cache_for_view(
            path,
            &source_key,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("reusable cache lookup")
        .expect("initial prefix frame cache");
        let requested_index = 20usize;
        let expected_cache_end = requested_index.saturating_add(ANIMATION_FRAME_CACHE_RADIUS);
        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key),
            BoundedAnimationFrames::new(frame_size, 100, expected_cache_end + 1),
            ANIMATION_FRAME_CACHE_RADIUS,
            Some(reusable_cache),
            None,
            None,
            None,
        )
        .expect("prefix cache miss should decode requested frame");
        let requested = expect_returned_animation_frame(requested);

        assert_eq!(requested.pixels(), animation_frame_color(requested_index));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("extended prefix should be cached");
            assert_eq!(entry.start_index, 0);
            assert_eq!(entry.frame_count, None);
            assert_eq!(entry.frames.len(), expected_cache_end + 1);
            for (index, frame) in entry.frames.iter().enumerate() {
                assert_eq!(frame.pixels(), animation_frame_color(index));
            }
        }

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_miss_retains_previous_sparse_frames_within_budget() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let frame_size = ImageSize::new(1, 1);
        let path = Path::new("sparse-retained-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let previous_start = 20usize;
        let previous_len = ANIMATION_INITIAL_CACHE_FRAME_LIMIT;
        let mut previous_cache = AnimationFrameCacheBuilder::new_starting_at(
            source_key.clone(),
            frame_size,
            frame_size,
            ImageBufferKind::FullResolution,
            previous_start,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        for index in previous_start..previous_start.saturating_add(previous_len) {
            assert!(previous_cache.push(Rgba8Image::new(
                1,
                1,
                animation_frame_color(index).to_vec()
            )));
        }
        replace_animation_frame_cache(previous_cache.finish());

        let retained_frame_index = previous_start + 2;
        let resident_retained_frame = {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("previous frame cache");
            Arc::clone(&entry.frames[retained_frame_index - previous_start])
        };
        let reusable_cache = reusable_animation_frame_cache_for_view(
            path,
            &source_key,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("reusable cache lookup")
        .expect("previous frame cache");
        assert!(Arc::ptr_eq(
            &resident_retained_frame,
            &reusable_cache.frames[retained_frame_index - previous_start]
        ));
        let requested_index = 40usize;
        let expected_cache_start = requested_index.saturating_sub(ANIMATION_FRAME_CACHE_RADIUS);
        let expected_cache_end = requested_index.saturating_add(ANIMATION_FRAME_CACHE_RADIUS);
        let requested = decode_animation_frame_from_iter_with_reuse(
            path,
            requested_index,
            frame_size,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            Some(source_key.clone()),
            BoundedAnimationFrames::new(frame_size, 100, expected_cache_end + 1),
            ANIMATION_FRAME_CACHE_RADIUS,
            Some(reusable_cache),
            None,
            None,
            None,
        )
        .expect("distant cache miss should decode requested frame");
        let requested = expect_returned_animation_frame(requested);

        assert_eq!(requested.pixels(), animation_frame_color(requested_index));
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("new frame window should be cached");
            assert_eq!(entry.start_index, expected_cache_start);
            assert_eq!(
                entry.frames.len(),
                expected_cache_end - expected_cache_start + 1
            );
            assert_eq!(entry.extra_frames.len(), previous_len);
            let retained_extra_frame = entry
                .extra_frames
                .iter()
                .find(|(index, _)| *index == retained_frame_index)
                .expect("previous sparse frame should be retained");
            assert!(Arc::ptr_eq(
                &resident_retained_frame,
                &retained_extra_frame.1
            ));
        }

        let retained = cached_animation_frame_for_view(
            path,
            &source_key,
            retained_frame_index,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("retained frame cache lookup should succeed")
        .expect("previous sparse frame should remain cached");
        assert_eq!(
            retained.as_rgba8().pixels(),
            animation_frame_color(retained_frame_index)
        );

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_miss_keeps_requested_window_after_budget_exceeded() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let mut memory = MemoryPolicySettings::default();
        memory.set_max_transient_decode_mib(64);
        memory.set_max_resident_mib(2);
        memory.set_max_cache_entry_mib(1);
        let policy = memory.image_memory_policy();
        let frame_size = ImageSize::new(512, 512);
        let path = Path::new("large-frames.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        const FIRST: [u8; 4] = [255, 0, 0, 255];
        const SECOND: [u8; 4] = [0, 255, 0, 255];
        const THIRD: [u8; 4] = [0, 0, 255, 255];
        let frames: Vec<image::ImageResult<Frame>> = vec![
            Ok(solid_animation_frame(frame_size, FIRST, 30)),
            Ok(solid_animation_frame(frame_size, SECOND, 70)),
            Ok(solid_animation_frame(frame_size, THIRD, 110)),
        ];

        let third = decode_animation_frame_from_iter(
            path,
            2,
            frame_size,
            ViewportSize::EMPTY,
            policy,
            Some(source_key.clone()),
            frames,
            ANIMATION_FRAME_CACHE_RADIUS,
            None,
            None,
        )
        .expect("third frame should decode");
        let third = expect_returned_animation_frame(third);

        assert_eq!(&third.pixels()[..4], THIRD.as_slice());
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("requested window should be cached");
            assert_eq!(entry.source_key.path.as_path(), path);
            assert_eq!(entry.start_index, 2);
            assert_eq!(entry.frame_count, Some(3));
            assert_eq!(entry.frames.len(), 1);
            assert_eq!(&entry.frames[0].pixels()[..4], THIRD.as_slice());
        }

        let cached = cached_animation_frame_for_view(
            path,
            &source_key,
            2,
            ViewportSize::EMPTY,
            policy,
            None,
        )
        .expect("cache lookup should succeed")
        .expect("requested frame should be cached");
        assert_eq!(&cached.as_rgba8().pixels()[..4], THIRD.as_slice());

        let older = cached_animation_frame_for_view(
            path,
            &source_key,
            1,
            ViewportSize::EMPTY,
            policy,
            None,
        )
        .expect("older uncached frame should fall back to decode");
        assert!(older.is_none());

        replace_animation_frame_cache(None);
    }

    #[test]
    fn loaded_animation_frame_cache_hit_does_not_require_file_metadata() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let path = Path::new("missing-resident-cache.gif");
        let source_size = ImageSize::new(1, 1);
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 42,
            modified: UNIX_EPOCH,
        };
        let mut cache = AnimationFrameCacheBuilder::new(
            source_key,
            source_size,
            source_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        assert!(cache.push(Rgba8Image::new(1, 1, vec![9, 8, 7, 255])));
        replace_animation_frame_cache(cache.finish());

        let cached = cached_animation_frame_for_loaded_image(
            path,
            ImageFileVersion::new(42, UNIX_EPOCH),
            SupportedImageFormat::Gif,
            source_size,
            0,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("resident cache lookup should not read missing file metadata")
        .expect("resident frame should be cached");

        assert_eq!(cached.pixels(), &[9, 8, 7, 255]);
        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_hit_returns_shared_frame_pixels() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let source_size = ImageSize::new(1, 1);
        let path = Path::new("shared-cache-hit.gif");
        let source_key = AnimationSourceCacheKey {
            path: path.to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let mut cache = AnimationFrameCacheBuilder::new(
            source_key.clone(),
            source_size,
            source_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        assert!(cache.push(Rgba8Image::new(1, 1, vec![4, 5, 6, 255])));
        replace_animation_frame_cache(cache.finish());

        let cached = cached_animation_frame_for_view(
            path,
            &source_key,
            0,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("cache lookup should succeed")
        .expect("frame should be cached");
        let AnimationFramePixels::Shared(cached) = cached else {
            panic!("cache hit should return the resident Arc");
        };
        {
            let cache = lock_animation_frame_cache();
            let entry = cache.as_ref().expect("resident cache");
            assert!(Arc::ptr_eq(&cached, &entry.frames[0]));
        }
        assert_eq!(cached.pixels(), &[4, 5, 6, 255]);

        replace_animation_frame_cache(None);
    }

    #[test]
    fn animation_frame_cache_builder_push_for_return_shares_cached_frame() {
        let source_size = ImageSize::new(1, 1);
        let source_key = AnimationSourceCacheKey {
            path: Path::new("shared-return-cache.gif").to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let mut cache = AnimationFrameCacheBuilder::new(
            source_key,
            source_size,
            source_size,
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(DEFAULT_IMAGE_MEMORY_POLICY),
        );
        let frame = Rgba8Image::new(1, 1, vec![1, 2, 3, 255]);

        let frame_for_return = match cache.push_for_return(frame) {
            Ok(frame) => frame,
            Err(_) => panic!("test frame should fit in the cache"),
        };

        let AnimationFrameForReturn::Shared(shared_frame) = &frame_for_return else {
            panic!("cached return frame should share the cache allocation");
        };
        assert_eq!(cache.frames.len(), 1);
        assert_eq!(
            shared_frame.pixels().as_ptr(),
            cache.frames[0].pixels().as_ptr()
        );
        assert_eq!(shared_frame.pixels(), &[1, 2, 3, 255]);
    }

    #[test]
    fn animation_frame_cache_stays_within_resident_entry_budget() {
        let mut memory = MemoryPolicySettings::default();
        memory.set_max_transient_decode_mib(64);
        memory.set_max_resident_mib(2);
        memory.set_max_cache_entry_mib(1);
        let policy = memory.image_memory_policy();
        let max_cache_entry_bytes: usize = 1024 * 1024;
        assert_eq!(
            animation_frame_cache_byte_limit(policy),
            max_cache_entry_bytes
        );

        let source_key = AnimationSourceCacheKey {
            path: Path::new("frames.gif").to_path_buf(),
            format: SupportedImageFormat::Gif,
            file_len: 1,
            modified: UNIX_EPOCH,
        };
        let mut cache = AnimationFrameCacheBuilder::new(
            source_key,
            ImageSize::new(512, 512),
            ImageSize::new(512, 512),
            ImageBufferKind::FullResolution,
            animation_frame_cache_byte_limit(policy),
        );

        assert!(cache.push(Rgba8Image::new(512, 512, vec![0; max_cache_entry_bytes])));
        assert!(!cache.push(Rgba8Image::new(1, 1, vec![0; 4])));
    }

    #[test]
    fn generated_animated_webp_fixture_preserves_frame_order_delays_and_loop() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let dir = unique_temp_dir("animated-webp");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("frames.webp");
        write_animated_webp_fixture(&path);

        let image = load_image_file(&path).expect("animated WebP should load");

        assert_eq!(image.metadata().format(), SupportedImageFormat::Webp);
        assert!(image.is_animated());
        assert_rgba_pixels_close(image.pixels().pixels(), webp_frame_pixels(0), 2);
        let playback = image.animation().expect("animation playback");
        assert_eq!(playback.frame_delays_ms(), &[40, 90]);
        assert_eq!(playback.loop_policy(), AnimationLoopPolicy::finite(2));

        let second = load_animation_frame_for_view(
            &path,
            1,
            ViewportSize::EMPTY,
            DEFAULT_IMAGE_MEMORY_POLICY,
            None,
        )
        .expect("second WebP frame");

        assert_rgba_pixels_close(second.pixels(), webp_frame_pixels(1), 2);
        replace_animation_frame_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn parallel_metadata_webp_fixture_preserves_frame_order_delays_and_loop() {
        let _cache_guard = animation_frame_cache_test_guard();
        replace_animation_frame_cache(None);

        let dir = unique_temp_dir("parallel-metadata-webp");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("frames.webp");
        write_animated_webp_fixture(&path);
        let file_metadata = fs::metadata(&path).expect("animated webp metadata");

        let decoded = decode_webp_animation_image_with_parallel_metadata(
            &path,
            AnimationSourceCacheKey::new(&path, SupportedImageFormat::Webp, &file_metadata),
            AnimationDecodeContext {
                viewport: ViewportSize::EMPTY,
                policy: DEFAULT_IMAGE_MEMORY_POLICY,
                animation_timing: AnimationTimingSettings::default(),
                cancel: None,
            },
        )
        .expect("parallel metadata WebP should load");

        assert_rgba_pixels_close(decoded.first_frame.pixels(), webp_frame_pixels(0), 2);
        let playback = decoded.playback.expect("animation playback");
        assert_eq!(playback.frame_delays_ms(), &[40, 90]);
        assert_eq!(playback.loop_policy(), AnimationLoopPolicy::finite(2));

        replace_animation_frame_cache(None);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn unsupported_extension_is_rejected_at_format_detection() {
        let path = Path::new("missing.avif");
        let error = load_image_file(path).expect_err("extension should be rejected first");

        assert_eq!(error.category(), ImageOpenErrorCategory::UnsupportedFormat);
        assert_eq!(
            error.failure_stage(),
            ImageLoadFailureStage::FormatDetection
        );
        assert!(Error::source(&error).is_none());
    }

    #[test]
    fn valid_image_bytes_with_unsupported_extension_stop_at_format_detection() {
        let dir = unique_temp_dir("valid-image-unsupported-extension");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("valid-image.avif");
        write_export_fixture(&path, ExportFormat::Png);

        let error = load_image_file(&path).expect_err("extension should be rejected");

        assert_eq!(error.category(), ImageOpenErrorCategory::UnsupportedFormat);
        assert_eq!(
            error.failure_stage(),
            ImageLoadFailureStage::FormatDetection
        );
        assert!(Error::source(&error).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn missing_supported_extension_file_is_file_io_not_found() {
        let dir = unique_temp_dir("missing-supported-image");
        let path = dir.join("missing.png");

        let error = load_image_file(&path).expect_err("missing image should fail at file I/O");

        assert_eq!(
            error.category(),
            ImageOpenErrorCategory::FileNotFoundOrMoved
        );
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::FileIo);
        assert!(error.user_message().contains("찾을 수 없습니다"));
        assert!(Error::source(&error).is_some());
    }

    #[test]
    fn corrupt_and_misleading_supported_extension_files_are_decode_failures() {
        let dir = unique_temp_dir("corrupt-images");
        fs::create_dir_all(&dir).expect("test dir");
        let truncated_png = dir.join("truncated.png");
        let fake_jpeg = dir.join("fake.jpg");
        fs::write(&truncated_png, b"\x89PNG\r\n\x1a\nnot enough image data").expect("png data");
        fs::write(&fake_jpeg, b"this is not an image").expect("fake jpeg data");

        for path in [truncated_png, fake_jpeg] {
            let error = load_image_file(&path).expect_err("broken image should fail");

            assert_eq!(
                error.category(),
                ImageOpenErrorCategory::CorruptOrDecodingFailed,
                "{}",
                path.display()
            );
            assert_eq!(error.failure_stage(), ImageLoadFailureStage::Decoder);
            assert!(error.user_message().contains("디코딩할 수 없습니다"));
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn permission_denied_file_is_classified_and_acl_is_restored() {
        let dir = unique_temp_dir("permission-denied");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("denied.png");
        write_fixture_image(&path, FixtureImageFormat::Png);
        let user = current_windows_identity();
        let mut acl_guard = DeniedAclGuard::deny_read(&path, user);

        let error = load_image_file(&path).expect_err("denied file should fail");

        assert_eq!(error.category(), ImageOpenErrorCategory::PermissionDenied);
        assert_eq!(error.failure_stage(), ImageLoadFailureStage::FileIo);

        acl_guard.restore();
        let restored = load_image_file(&path).expect("restored file should load");
        assert_eq!(restored.source_size(), ImageSize::new(2, 2));
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn permission_denied_image_folder_scan_is_reported_and_acl_is_restored() {
        let dir = unique_temp_dir("folder-permission-denied");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("image.png");
        write_fixture_image(&path, FixtureImageFormat::Png);
        let user = current_windows_identity();
        let mut acl_guard = DeniedAclGuard::deny_read(&dir, user);

        let error = scan_image_folder_for_file(&path).expect_err("denied folder scan should fail");

        match &error {
            ScanImageFolderError::DirectoryAccess { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            unexpected => panic!("unexpected folder scan error: {unexpected:?}"),
        }
        assert_eq!(
            error.brief_user_message(),
            "이미지 폴더를 읽을 권한이 없습니다."
        );
        assert!(error
            .user_message()
            .contains("이미지 폴더를 읽을 수 없습니다"));
        assert!(Error::source(&error).is_some());

        acl_guard.restore();
        let folder = scan_image_folder_for_file(&path).expect("restored folder scan should work");
        assert_eq!(folder.len(), 1);
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn optional_image_folder_scan_returns_empty_folder_on_error() {
        let (folder, error) = scan_image_folder_for_file_or_empty(Path::new(""));

        assert_eq!(folder.len(), 0);
        assert!(matches!(error, Some(ScanImageFolderError::NoParent { .. })));
    }

    #[test]
    fn canceled_image_folder_scan_returns_no_result() {
        let cancel = AtomicBool::new(true);

        let result = scan_image_folder_for_file_with_cancellation(Path::new(""), &cancel);

        assert!(result.is_none());
        assert!(cancel.load(Ordering::Acquire));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn export_permission_denied_is_classified_and_acl_is_restored() {
        let dir = unique_temp_dir("export-permission-denied");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("denied.png");
        let user = current_windows_identity();
        let mut acl_guard = DeniedAclGuard::deny_write(&dir, user);

        let error = export_rgba8_image(
            &path,
            &fixture_rgba8(),
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect_err("denied directory should fail export");

        assert_eq!(error.category(), ExportSaveErrorCategory::PermissionDenied);

        acl_guard.restore();
        export_rgba8_image(
            &path,
            &fixture_rgba8(),
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect("restored directory should allow export");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_error_classification_and_messages_are_user_facing() {
        let denied = ExportImageError::FileCreate {
            path: Path::new("denied.png").to_path_buf(),
            source: io::Error::from_raw_os_error(5),
        };
        let missing_folder = ExportImageError::FileCreate {
            path: Path::new("missing").join("out.png"),
            source: io::Error::from_raw_os_error(3),
        };
        let locked = ExportImageError::FileWrite {
            path: Path::new("locked.png").to_path_buf(),
            source: io::Error::from_raw_os_error(32),
        };
        let replace_denied = ExportImageError::FileReplace {
            temporary_path: Path::new("replace.png.tmp").to_path_buf(),
            path: Path::new("replace.png").to_path_buf(),
            source: io::Error::from_raw_os_error(5),
        };
        let invalid = ExportImageError::InvalidPixelBuffer {
            path: Path::new("invalid.png").to_path_buf(),
            size: ImageSize::new(2, 2),
            actual_len: 3,
        };

        assert_eq!(denied.category(), ExportSaveErrorCategory::PermissionDenied);
        assert!(denied.user_message().contains("권한"));
        assert_eq!(
            missing_folder.category(),
            ExportSaveErrorCategory::PathNotFound
        );
        assert!(missing_folder.user_message().contains("저장 경로"));
        assert_eq!(locked.category(), ExportSaveErrorCategory::FileLocked);
        assert!(locked.user_message().contains("사용 중"));
        assert_eq!(
            replace_denied.category(),
            ExportSaveErrorCategory::PermissionDenied
        );
        assert!(replace_denied.user_message().contains("권한"));
        assert_eq!(
            invalid.category(),
            ExportSaveErrorCategory::ImageDataInvalid
        );
        assert!(invalid.user_message().contains("픽셀 데이터"));
    }

    #[test]
    fn invalid_export_pixel_buffer_returns_result_instead_of_panicking() {
        let image = Rgba8Image::new(2, 2, vec![0, 0, 0, 255]);
        let error = export_rgba8_image(
            Path::new("invalid-export.png"),
            &image,
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect_err("invalid buffer should be rejected before file creation");

        assert_eq!(error.category(), ExportSaveErrorCategory::ImageDataInvalid);
    }

    #[test]
    fn save_config_writes_temp_then_loads_roundtrip() {
        let dir = unique_temp_dir("roundtrip");
        let path = dir.join("config.txt");
        let config = AppConfig::new(
            WindowBounds::new(100, 120, 900, 700),
            ViewMode::ActualSize,
            ScalingQuality::HighQuality,
            Some(dir.join("images")),
            95,
            false,
        );

        save_app_config_to_path(&path, &config).expect("save config");
        let loaded = load_app_config_from_path(&path).expect("load saved config");

        assert_eq!(loaded, config);
        assert!(!path.with_file_name("config.txt.tmp").exists());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn save_config_does_not_reuse_existing_fixed_temporary_file() {
        let dir = unique_temp_dir("fixed-config-temp");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");
        let fixed_temporary_path = path.with_file_name("config.txt.tmp");
        fs::write(&fixed_temporary_path, "stale temporary data").expect("write stale temp");
        let config = AppConfig::new(
            WindowBounds::new(20, 30, 800, 600),
            ViewMode::FitToWindow,
            ScalingQuality::Nearest,
            Some(dir.join("images")),
            88,
            true,
        );

        save_app_config_to_path(&path, &config).expect("save config");
        let loaded = load_app_config_from_path(&path).expect("load saved config");

        assert_eq!(loaded, config);
        assert_eq!(
            fs::read_to_string(&fixed_temporary_path).expect("read stale temp"),
            "stale temporary data"
        );
        assert_config_temporary_files_removed(&dir, "config.txt");
        let _ = fs::remove_dir_all(dir);
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn save_config_permission_denied_is_reported_and_acl_is_restored() {
        let dir = unique_temp_dir("config-write-denied");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("config.txt");
        let user = current_windows_identity();
        let mut acl_guard = DeniedAclGuard::deny_write(&dir, user);

        let error = save_app_config_to_path(&path, &AppConfig::default())
            .expect_err("denied directory should fail config save");

        match error {
            AppConfigSaveError::FileCreate { source, .. }
            | AppConfigSaveError::FileWrite { source, .. }
            | AppConfigSaveError::FileReplace { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            AppConfigSaveError::CreateDirectory { source, .. } => {
                assert_eq!(source.kind(), io::ErrorKind::PermissionDenied);
            }
            unexpected => panic!("unexpected config save error: {unexpected:?}"),
        }

        acl_guard.restore();
        save_app_config_to_path(&path, &AppConfig::default())
            .expect("restored directory should allow config save");
        let _ = fs::remove_dir_all(dir);
    }

    fn unique_temp_dir(name: &str) -> std::path::PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("j3pic-{name}-{}-{nanos}", std::process::id()))
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    struct IcoTestEntry {
        width: u32,
        height: u32,
        data_offset: usize,
        data_size: usize,
    }

    fn ico_entries(bytes: &[u8]) -> Vec<IcoTestEntry> {
        assert!(bytes.len() >= 6, "ICO header should be present");
        assert_eq!(&bytes[0..2], &[0, 0], "ICO reserved field");
        assert_eq!(&bytes[2..4], &[1, 0], "ICO image type");
        let count = u16::from_le_bytes([bytes[4], bytes[5]]) as usize;
        assert!(
            bytes.len() >= 6 + count * 16,
            "ICO directory entries should be present"
        );

        (0..count)
            .map(|index| {
                let offset = 6 + index * 16;
                let width = ico_dimension_byte(bytes[offset]);
                let height = ico_dimension_byte(bytes[offset + 1]);
                let data_size = u32::from_le_bytes([
                    bytes[offset + 8],
                    bytes[offset + 9],
                    bytes[offset + 10],
                    bytes[offset + 11],
                ]) as usize;
                let data_offset = u32::from_le_bytes([
                    bytes[offset + 12],
                    bytes[offset + 13],
                    bytes[offset + 14],
                    bytes[offset + 15],
                ]) as usize;
                assert!(
                    data_offset
                        .checked_add(data_size)
                        .is_some_and(|end| end <= bytes.len()),
                    "ICO entry data should fit in file"
                );
                IcoTestEntry {
                    width,
                    height,
                    data_offset,
                    data_size,
                }
            })
            .collect()
    }

    fn ico_dimension_byte(value: u8) -> u32 {
        if value == 0 {
            256
        } else {
            u32::from(value)
        }
    }

    fn animation_frame_cache_test_guard() -> MutexGuard<'static, ()> {
        match ANIMATION_FRAME_CACHE_TEST_MUTEX.lock() {
            Ok(guard) => guard,
            Err(poisoned) => poisoned.into_inner(),
        }
    }

    fn assert_config_temporary_files_removed(dir: &Path, target_file_name: &str) {
        let temporary_prefix = format!("{target_file_name}.tmp.");
        let leftovers = fs::read_dir(dir)
            .expect("read config dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|file_name| file_name.starts_with(&temporary_prefix))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temporary config files remain: {leftovers:?}"
        );
    }

    #[derive(Debug, Clone, Copy)]
    enum FixtureImageFormat {
        Jpeg,
        Png,
        Bmp,
        Gif,
        Webp,
        Ico,
        Tiff,
        Tga,
    }

    const GIF_FRAME_PIXELS: [[u8; 8]; 3] = [
        [255, 0, 0, 255, 0, 255, 0, 255],
        [0, 0, 255, 255, 255, 255, 0, 255],
        [255, 0, 255, 255, 0, 255, 255, 255],
    ];
    const WEBP_FRAME_PIXELS: [[u8; 8]; 2] = [
        [255, 0, 0, 255, 0, 255, 0, 255],
        [0, 0, 255, 255, 255, 255, 0, 255],
    ];

    fn write_fixture_image(path: &Path, format: FixtureImageFormat) {
        match format {
            FixtureImageFormat::Jpeg => write_export_fixture(path, ExportFormat::Jpeg),
            FixtureImageFormat::Png => write_export_fixture(path, ExportFormat::Png),
            FixtureImageFormat::Bmp => write_export_fixture(path, ExportFormat::Bmp),
            FixtureImageFormat::Gif => write_gif_fixture(path),
            FixtureImageFormat::Webp => write_export_fixture(path, ExportFormat::Webp),
            FixtureImageFormat::Ico => write_export_fixture(path, ExportFormat::Ico),
            FixtureImageFormat::Tiff => write_image_format_fixture(path, ImageFormat::Tiff),
            FixtureImageFormat::Tga => write_image_format_fixture(path, ImageFormat::Tga),
        }
    }

    fn write_export_fixture(path: &Path, format: ExportFormat) {
        export_rgba8_image(path, &fixture_rgba8(), ExportOptions::new(format, None))
            .expect("write image fixture");
    }

    fn write_image_format_fixture(path: &Path, format: ImageFormat) {
        let image = fixture_rgba8();
        image::save_buffer_with_format(
            path,
            image.pixels(),
            image.width(),
            image.height(),
            ColorType::Rgba8,
            format,
        )
        .expect("write image fixture");
    }

    fn assert_export_temporary_files_removed(dir: &Path, target_file_name: &str) {
        let temporary_prefix = format!("{target_file_name}.tmp.");
        let leftovers = fs::read_dir(dir)
            .expect("read export dir")
            .filter_map(|entry| entry.ok())
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .filter(|file_name| file_name.starts_with(&temporary_prefix))
            .collect::<Vec<_>>();
        assert!(
            leftovers.is_empty(),
            "temporary export files remain: {leftovers:?}"
        );
    }

    fn write_bmp_header_only_fixture(path: &Path, width: i32, height: i32) {
        let mut bmp = Vec::new();
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&[0, 0, 0, 0]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&width.to_le_bytes());
        bmp.extend_from_slice(&height.to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0i32.to_le_bytes());
        bmp.extend_from_slice(&0i32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        bmp.extend_from_slice(&0u32.to_le_bytes());
        fs::write(path, bmp).expect("write BMP header fixture");
    }

    fn write_gif_fixture(path: &Path) {
        let file = File::create(path).expect("create gif fixture");
        let rgba = image::RgbaImage::from_raw(2, 2, fixture_rgba8().into_raw())
            .expect("fixture rgba dimensions");
        let mut encoder = GifEncoder::new(file);
        encoder
            .encode_frame(image::Frame::new(rgba))
            .expect("write gif fixture");
    }

    fn write_jpeg_exif_orientation_fixture(path: &Path, orientation: u8) {
        let image = Rgba8Image::new(
            3,
            2,
            vec![
                250, 0, 0, 255, 0, 250, 0, 255, 0, 0, 250, 255, 250, 250, 0, 255, 250, 0, 250, 255,
                0, 250, 250, 255,
            ],
        );
        export_rgba8_image(
            path,
            &image,
            ExportOptions::new(ExportFormat::Jpeg, Some(100)),
        )
        .expect("write base jpeg fixture");
        let jpeg = fs::read(path).expect("read base jpeg fixture");
        let with_exif = jpeg_with_exif_orientation(jpeg, orientation);
        fs::write(path, with_exif).expect("write exif jpeg fixture");
    }

    fn jpeg_with_exif_orientation(jpeg: Vec<u8>, orientation: u8) -> Vec<u8> {
        assert!(jpeg.starts_with(&[0xff, 0xd8]), "fixture is not a JPEG");
        let exif = exif_orientation_app1_payload(orientation);
        let length = u16::try_from(exif.len() + 2).expect("APP1 length");

        let mut output = Vec::with_capacity(jpeg.len() + exif.len() + 4);
        output.extend_from_slice(&jpeg[..2]);
        output.extend_from_slice(&[0xff, 0xe1]);
        output.extend_from_slice(&length.to_be_bytes());
        output.extend_from_slice(&exif);
        output.extend_from_slice(&jpeg[2..]);
        output
    }

    fn exif_orientation_app1_payload(orientation: u8) -> Vec<u8> {
        let mut exif = Vec::new();
        exif.extend_from_slice(b"Exif\0\0");
        exif.extend_from_slice(b"II");
        exif.extend_from_slice(&42u16.to_le_bytes());
        exif.extend_from_slice(&8u32.to_le_bytes());
        exif.extend_from_slice(&1u16.to_le_bytes());
        exif.extend_from_slice(&0x0112u16.to_le_bytes());
        exif.extend_from_slice(&3u16.to_le_bytes());
        exif.extend_from_slice(&1u32.to_le_bytes());
        exif.extend_from_slice(&u16::from(orientation).to_le_bytes());
        exif.extend_from_slice(&0u16.to_le_bytes());
        exif.extend_from_slice(&0u32.to_le_bytes());
        exif
    }

    fn write_animated_gif_fixture(path: &Path) {
        let file = File::create(path).expect("create animated gif fixture");
        let mut encoder = GifEncoder::new(file);
        encoder
            .set_repeat(Repeat::Finite(2))
            .expect("set gif repeat");

        for (index, delay_ms) in [30u32, 70, 110].into_iter().enumerate() {
            let rgba = RgbaImage::from_raw(2, 1, gif_frame_pixels(index).to_vec())
                .expect("gif frame dimensions");
            let frame = Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(delay_ms, 1));
            encoder
                .encode_frame(frame)
                .expect("write animated gif frame");
        }
    }

    fn solid_animation_frame(size: ImageSize, color: [u8; 4], delay_ms: u32) -> Frame {
        let byte_len = size.rgba8_byte_len().expect("frame byte length");
        let mut pixels = Vec::with_capacity(byte_len);
        for _ in 0..(byte_len / 4) {
            pixels.extend_from_slice(&color);
        }
        let rgba =
            RgbaImage::from_raw(size.width(), size.height(), pixels).expect("frame dimensions");
        Frame::from_parts(rgba, 0, 0, Delay::from_numer_denom_ms(delay_ms, 1))
    }

    struct BoundedAnimationFrames {
        size: ImageSize,
        frame_count: usize,
        panic_at: usize,
        next_index: usize,
    }

    impl BoundedAnimationFrames {
        fn new(size: ImageSize, frame_count: usize, panic_at: usize) -> Self {
            Self {
                size,
                frame_count,
                panic_at,
                next_index: 0,
            }
        }
    }

    impl Iterator for BoundedAnimationFrames {
        type Item = image::ImageResult<Frame>;

        fn next(&mut self) -> Option<Self::Item> {
            if self.next_index >= self.panic_at {
                panic!("animation frame iterator consumed beyond cache window");
            }
            if self.next_index >= self.frame_count {
                return None;
            }

            let index = self.next_index;
            self.next_index = self.next_index.saturating_add(1);
            Some(Ok(solid_animation_frame(
                self.size,
                animation_frame_color(index),
                30,
            )))
        }
    }

    fn animation_frame_color(index: usize) -> [u8; 4] {
        let red = index.min(usize::from(u8::MAX)) as u8;
        [red, 0, 255u8.saturating_sub(red), 255]
    }

    fn gif_frame_pixels(index: usize) -> &'static [u8] {
        &GIF_FRAME_PIXELS[index]
    }

    fn write_animated_webp_fixture(path: &Path) {
        let mut chunks = Vec::new();
        let mut vp8x = vec![0x12, 0, 0, 0];
        push_u24(&mut vp8x, 1);
        push_u24(&mut vp8x, 0);
        push_webp_chunk(&mut chunks, b"VP8X", &vp8x);

        let anim = [255, 255, 255, 255, 2, 0];
        push_webp_chunk(&mut chunks, b"ANIM", &anim);

        for (index, duration_ms) in [40u32, 90].into_iter().enumerate() {
            let mut frame_payload = Vec::new();
            push_u24(&mut frame_payload, 0);
            push_u24(&mut frame_payload, 0);
            push_u24(&mut frame_payload, 1);
            push_u24(&mut frame_payload, 0);
            push_u24(&mut frame_payload, duration_ms);
            frame_payload.push(0);
            frame_payload.extend_from_slice(&lossless_webp_image_chunks(webp_frame_pixels(index)));
            push_webp_chunk(&mut chunks, b"ANMF", &frame_payload);
        }

        fs::write(path, riff_webp(chunks)).expect("write animated webp fixture");
    }

    fn webp_frame_pixels(index: usize) -> &'static [u8] {
        &WEBP_FRAME_PIXELS[index]
    }

    fn lossless_webp_image_chunks(rgba: &[u8]) -> Vec<u8> {
        let mut encoded = Vec::new();
        WebPEncoder::new_lossless(&mut encoded)
            .write_image(rgba, 2, 1, ExtendedColorType::Rgba8)
            .expect("encode webp frame");
        assert_eq!(&encoded[0..4], b"RIFF");
        assert_eq!(&encoded[8..12], b"WEBP");
        encoded[12..].to_vec()
    }

    fn riff_webp(chunks: Vec<u8>) -> Vec<u8> {
        let riff_size = u32::try_from(4usize + chunks.len()).expect("RIFF size");
        let mut webp = Vec::with_capacity(12 + chunks.len());
        webp.extend_from_slice(b"RIFF");
        webp.extend_from_slice(&riff_size.to_le_bytes());
        webp.extend_from_slice(b"WEBP");
        webp.extend_from_slice(&chunks);
        webp
    }

    fn push_webp_chunk(output: &mut Vec<u8>, fourcc: &[u8; 4], payload: &[u8]) {
        output.extend_from_slice(fourcc);
        output.extend_from_slice(
            &u32::try_from(payload.len())
                .expect("WebP chunk length")
                .to_le_bytes(),
        );
        output.extend_from_slice(payload);
        if payload.len() % 2 == 1 {
            output.push(0);
        }
    }

    fn push_u24(output: &mut Vec<u8>, value: u32) {
        let bytes = value.to_le_bytes();
        output.extend_from_slice(&bytes[..3]);
    }

    fn assert_rgba_pixels_close(actual: &[u8], expected: &[u8], tolerance: u8) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            let diff = actual.abs_diff(*expected);
            assert!(
                diff <= tolerance,
                "pixel byte {index}: actual {actual} differs from expected {expected} by {diff}"
            );
        }
    }

    fn fixture_rgba8() -> Rgba8Image {
        Rgba8Image::new(
            2,
            2,
            vec![
                255, 0, 0, 255, 0, 255, 0, 255, 0, 0, 255, 255, 255, 255, 255, 255,
            ],
        )
    }

    fn varied_rgba8(width: u32, height: u32) -> Rgba8Image {
        let byte_len = ImageSize::new(width, height)
            .rgba8_byte_len()
            .expect("fixture byte length");
        let mut pixels = Vec::with_capacity(byte_len);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[
                    ((x * 31 + y * 17) & 0xff) as u8,
                    ((x * 13 + y * 47) & 0xff) as u8,
                    ((x * 7 + y * 29) & 0xff) as u8,
                    255,
                ]);
            }
        }
        Rgba8Image::new(width, height, pixels)
    }

    fn binary_rgba8(width: u32, height: u32) -> Rgba8Image {
        let byte_len = ImageSize::new(width, height)
            .rgba8_byte_len()
            .expect("fixture byte length");
        let mut pixels = Vec::with_capacity(byte_len);
        for y in 0..height {
            for x in 0..width {
                let red = if (x + y) % 2 == 0 { 0 } else { 255 };
                let green = if x % 2 == 0 { 255 } else { 0 };
                let blue = if y % 2 == 0 { 0 } else { 255 };
                pixels.extend_from_slice(&[red, green, blue, 255]);
            }
        }
        Rgba8Image::new(width, height, pixels)
    }

    fn write_rgba16_png_fixture(path: &Path, source: &Rgba8Image) {
        let file = File::create(path).expect("create rgba16 png fixture");
        let mut pixels = Vec::with_capacity(source.pixels().len() * 2);
        for value in source.pixels() {
            let value = u16::from(*value) * 257;
            pixels.extend_from_slice(&value.to_be_bytes());
        }
        image::codecs::png::PngEncoder::new(file)
            .write_image(
                &pixels,
                source.width(),
                source.height(),
                ExtendedColorType::Rgba16,
            )
            .expect("write rgba16 png fixture");
    }

    fn expected_sampled_rgba8_pixels(source: &Rgba8Image, target_size: ImageSize) -> Vec<u8> {
        let target_width = target_size.width();
        let target_height = target_size.height();
        let mut pixels = Vec::with_capacity(
            target_size
                .rgba8_byte_len()
                .expect("sampled fixture byte length"),
        );
        for target_y in 0..target_height {
            let source_y = target_y.saturating_mul(source.height()) / target_height.max(1);
            for target_x in 0..target_width {
                let source_x = target_x.saturating_mul(source.width()) / target_width.max(1);
                let offset = usize::try_from(
                    (u64::from(source_y) * u64::from(source.width()) + u64::from(source_x)) * 4,
                )
                .expect("sampled fixture offset");
                pixels.extend_from_slice(&source.pixels()[offset..offset + 4]);
            }
        }
        pixels
    }

    fn large_gradient_rgba8(width: u32, height: u32) -> Rgba8Image {
        let byte_len = ImageSize::new(width, height)
            .rgba8_byte_len()
            .expect("large fixture byte length");
        let mut pixels = Vec::with_capacity(byte_len);
        for y in 0..height {
            for x in 0..width {
                pixels.extend_from_slice(&[x as u8, y as u8, x.wrapping_add(y) as u8, 255]);
            }
        }
        Rgba8Image::new(width, height, pixels)
    }

    #[cfg(target_os = "windows")]
    fn current_windows_identity() -> String {
        let output = Command::new("whoami").output().expect("run whoami");
        assert!(output.status.success(), "whoami failed: {output:?}");
        String::from_utf8(output.stdout)
            .expect("whoami output utf8")
            .trim()
            .to_owned()
    }

    #[cfg(target_os = "windows")]
    struct DeniedAclGuard {
        path: PathBuf,
        user: String,
        active: bool,
    }

    #[cfg(target_os = "windows")]
    impl DeniedAclGuard {
        fn deny_read(path: &Path, user: String) -> Self {
            Self::deny(path, user, "R")
        }

        fn deny_write(path: &Path, user: String) -> Self {
            Self::deny(path, user, "W")
        }

        fn deny(path: &Path, user: String, rights: &str) -> Self {
            let status = Command::new("icacls")
                .arg(path)
                .arg("/deny")
                .arg(format!("{user}:{rights}"))
                .stdout(Stdio::null())
                .stderr(Stdio::null())
                .status()
                .expect("run icacls deny");
            assert!(status.success(), "icacls deny failed: {status}");
            Self {
                path: path.to_path_buf(),
                user,
                active: true,
            }
        }

        fn restore(&mut self) {
            if self.active {
                let status = Command::new("icacls")
                    .arg(&self.path)
                    .arg("/remove:d")
                    .arg(&self.user)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status()
                    .expect("run icacls restore");
                assert!(status.success(), "icacls restore failed: {status}");
                self.active = false;
            }
        }
    }

    #[cfg(target_os = "windows")]
    impl Drop for DeniedAclGuard {
        fn drop(&mut self) {
            if self.active {
                let _ = Command::new("icacls")
                    .arg(&self.path)
                    .arg("/remove:d")
                    .arg(&self.user)
                    .stdout(Stdio::null())
                    .stderr(Stdio::null())
                    .status();
            }
        }
    }
}
