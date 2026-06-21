use std::collections::hash_map::DefaultHasher;
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::io;
use std::path::{Path, PathBuf};
use std::sync::Arc;

use image::imageops::{resize, FilterType};
use image::{GenericImageView, Rgb, Rgba};

use crate::domain::{
    animation_state_after_home, animation_state_after_manual_step,
    animation_state_after_timer_tick, animation_state_after_toggle, animation_timer_interval_ms,
    display_orientation, export_format_display_name, image_status_text_with_settings,
    is_stale_decode_generation, memory_cache_slots_to_evict, orient_pixel_image,
    scaling_cache_key_for_render, scaling_quality_for_render, should_rebuild_scaling_cache,
    should_request_full_resolution_for_view, suggested_export_path_with_suffix, AnimationCommand,
    AnimationPlayback, AnimationPlaybackState, AnimationPlaybackTransition,
    AnimationTimingSettings, AppConfig, Command, DecodeGeneration, ExportFormat, ExportOptions,
    ImageCacheSlot, ImageDisplayRect, ImageFileVersion, ImageFolder, ImageMemoryPolicy,
    ImageNavigationDirection, ImageOrientation, ImageRotation, ImageSize, ImageState, LoadedImage,
    MemoryCacheEntry, NavigationSettings, PixelImage, Rgb8Image, Rgba8Image, ScalingCacheKey,
    ScalingQuality, SupportedImageFormat, UiLanguage, ViewMode, ViewOffset, ViewTransform,
    ViewportPoint, ViewportSize, WindowBounds,
};
use crate::infra::{
    animation_frame_resident_cache_byte_len, clear_animation_frame_resident_cache,
    export_borrowed_pixel_image, export_owned_pixel_image, load_image_file_for_view_with_timing,
    load_image_file_for_view_with_timing_and_profile, loaded_image_file_version_matches_current,
    scan_image_folder_for_file_or_empty, AnimationFramePixels, ExportImageError, ImageOpenProfile,
    ImageOpenProfiler, LoadImageError, ScanImageFolderError,
};

const DEFAULT_TITLE: &str = "j3Pic";
const RGB8_BYTES_PER_PIXEL: usize = 3;
const RGBA8_BYTES_PER_PIXEL: usize = 4;
// ImageCacheSlot has no distinct resident animation cache variant.
const RESIDENT_ANIMATION_FRAME_CACHE_SLOT_INDEX: usize = usize::MAX;

#[derive(Debug)]
pub enum ViewerAppError {
    LoadImage(LoadImageError),
    ScanImageFolder(ScanImageFolderError),
    ExportImage(ExportImageError),
    DecodeWorkerStart { path: PathBuf, source: io::Error },
    ExportWorkerStart { path: PathBuf, source: io::Error },
    NoImageToExport,
}

impl ViewerAppError {
    pub fn user_message(&self) -> String {
        match self {
            Self::LoadImage(error) => error.user_message(),
            Self::ScanImageFolder(error) => error.user_message(),
            Self::ExportImage(error) => error.user_message(),
            Self::DecodeWorkerStart { path, .. } => format!(
                "이미지 디코드 작업을 시작하지 못했습니다. 시스템 리소스가 부족할 수 있습니다.\n\n파일: {}",
                path.display()
            ),
            Self::ExportWorkerStart { path, .. } => format!(
                "이미지 내보내기 작업을 시작하지 못했습니다. 시스템 리소스가 부족할 수 있습니다.\n\n파일: {}",
                path.display()
            ),
            Self::NoImageToExport => "내보낼 이미지가 없습니다.".to_owned(),
        }
    }

    pub fn user_message_for(&self, language: UiLanguage) -> String {
        if language == UiLanguage::Korean {
            return self.user_message();
        }
        match self {
            Self::LoadImage(error) => error.user_message_for(language),
            Self::ScanImageFolder(error) => error.user_message_for(language),
            Self::ExportImage(error) => error.user_message_for(language),
            Self::DecodeWorkerStart { path, .. } => format!(
                "Could not start the image decode task. System resources may be low.\n\nFile: {}",
                path.display()
            ),
            Self::ExportWorkerStart { path, .. } => format!(
                "Could not start the image export task. System resources may be low.\n\nFile: {}",
                path.display()
            ),
            Self::NoImageToExport => "There is no image to export.".to_owned(),
        }
    }

    pub fn brief_user_message(&self) -> &'static str {
        match self {
            Self::LoadImage(error) => error.brief_user_message(),
            Self::ScanImageFolder(error) => error.brief_user_message(),
            Self::ExportImage(error) => error.brief_user_message(),
            Self::DecodeWorkerStart { .. } => "이미지 디코드 작업을 시작하지 못했습니다.",
            Self::ExportWorkerStart { .. } => "이미지 내보내기 작업을 시작하지 못했습니다.",
            Self::NoImageToExport => "내보낼 이미지가 없습니다.",
        }
    }

    pub fn brief_user_message_for(&self, language: UiLanguage) -> &'static str {
        if language == UiLanguage::Korean {
            return self.brief_user_message();
        }
        match self {
            Self::LoadImage(error) => error.brief_user_message_for(language),
            Self::ScanImageFolder(error) => error.brief_user_message_for(language),
            Self::ExportImage(error) => error.brief_user_message_for(language),
            Self::DecodeWorkerStart { .. } => "Could not start the image decode task.",
            Self::ExportWorkerStart { .. } => "Could not start the image export task.",
            Self::NoImageToExport => "There is no image to export.",
        }
    }
}

impl fmt::Display for ViewerAppError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::LoadImage(error) => error.fmt(formatter),
            Self::ScanImageFolder(error) => error.fmt(formatter),
            Self::ExportImage(error) => error.fmt(formatter),
            Self::DecodeWorkerStart { path, .. } => {
                write!(
                    formatter,
                    "failed to start image decode worker: {}",
                    path.display()
                )
            }
            Self::ExportWorkerStart { path, .. } => {
                write!(
                    formatter,
                    "failed to start image export worker: {}",
                    path.display()
                )
            }
            Self::NoImageToExport => formatter.write_str("no image to export"),
        }
    }
}

impl Error for ViewerAppError {
    fn source(&self) -> Option<&(dyn Error + 'static)> {
        match self {
            Self::LoadImage(error) => Some(error),
            Self::ScanImageFolder(error) => Some(error),
            Self::ExportImage(error) => Some(error),
            Self::DecodeWorkerStart { source, .. } | Self::ExportWorkerStart { source, .. } => {
                Some(source)
            }
            Self::NoImageToExport => None,
        }
    }
}

impl From<LoadImageError> for ViewerAppError {
    fn from(error: LoadImageError) -> Self {
        Self::LoadImage(error)
    }
}

impl From<ScanImageFolderError> for ViewerAppError {
    fn from(error: ScanImageFolderError) -> Self {
        Self::ScanImageFolder(error)
    }
}

impl From<ExportImageError> for ViewerAppError {
    fn from(error: ExportImageError) -> Self {
        Self::ExportImage(error)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationOutcome {
    Moved,
    Noop,
}

#[derive(Debug, Clone, PartialEq)]
pub enum NavigationStartOutcome {
    Decode(ImageDecodeRequest),
    AppliedPreloaded,
    Noop,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AppCommandOutcome {
    Changed,
    Unchanged,
    Unhandled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum DecodeApplyOutcome {
    Applied,
    Stale,
    Unchanged,
}

#[derive(Debug, Clone, PartialEq)]
pub enum DecodeFailurePresentation {
    MessageBox,
    StatusMessage,
    RetryNavigation(ImageDecodeRequest),
    Stale,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageDecodePurpose {
    OpenImage,
    FolderNavigation(ImageNavigationDirection),
    FullResolution,
}

impl ImageDecodePurpose {
    fn is_folder_navigation(self) -> bool {
        matches!(self, Self::FolderNavigation(_))
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImageDecodeRequest {
    generation: DecodeGeneration,
    path: PathBuf,
    file_version: Option<ImageFileVersion>,
    viewport: ViewportSize,
    memory_policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    purpose: ImageDecodePurpose,
}

impl ImageDecodeRequest {
    fn new(
        generation: DecodeGeneration,
        path: PathBuf,
        file_version: Option<ImageFileVersion>,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
        animation_timing: AnimationTimingSettings,
        purpose: ImageDecodePurpose,
    ) -> Self {
        Self {
            generation,
            path,
            file_version,
            viewport,
            memory_policy,
            animation_timing,
            purpose,
        }
    }

    pub fn generation(&self) -> DecodeGeneration {
        self.generation
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_version(&self) -> Option<ImageFileVersion> {
        self.file_version
    }

    pub fn viewport(&self) -> ViewportSize {
        self.viewport
    }

    pub fn memory_policy(&self) -> ImageMemoryPolicy {
        self.memory_policy
    }

    pub fn animation_timing(&self) -> AnimationTimingSettings {
        self.animation_timing
    }

    pub fn purpose(&self) -> ImageDecodePurpose {
        self.purpose
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct ImagePreloadRequest {
    generation: DecodeGeneration,
    path: PathBuf,
    viewport: ViewportSize,
    memory_policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
}

impl ImagePreloadRequest {
    fn new(
        generation: DecodeGeneration,
        path: PathBuf,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
        animation_timing: AnimationTimingSettings,
    ) -> Self {
        Self {
            generation,
            path,
            viewport,
            memory_policy,
            animation_timing,
        }
    }

    pub fn generation(&self) -> DecodeGeneration {
        self.generation
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn viewport(&self) -> ViewportSize {
        self.viewport
    }

    pub fn memory_policy(&self) -> ImageMemoryPolicy {
        self.memory_policy
    }

    pub fn animation_timing(&self) -> AnimationTimingSettings {
        self.animation_timing
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct AnimationFrameDecodeRequest {
    generation: DecodeGeneration,
    path: PathBuf,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    frame_index: usize,
    viewport: ViewportSize,
    memory_policy: ImageMemoryPolicy,
}

impl AnimationFrameDecodeRequest {
    fn new(
        generation: DecodeGeneration,
        path: PathBuf,
        file_version: ImageFileVersion,
        format: SupportedImageFormat,
        source_size: ImageSize,
        frame_index: usize,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
    ) -> Self {
        Self {
            generation,
            path,
            file_version,
            format,
            source_size,
            frame_index,
            viewport,
            memory_policy,
        }
    }

    pub fn generation(&self) -> DecodeGeneration {
        self.generation
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_version(&self) -> ImageFileVersion {
        self.file_version
    }

    pub fn format(&self) -> SupportedImageFormat {
        self.format
    }

    pub fn source_size(&self) -> ImageSize {
        self.source_size
    }

    pub fn frame_index(&self) -> usize {
        self.frame_index
    }

    pub fn viewport(&self) -> ViewportSize {
        self.viewport
    }

    pub fn memory_policy(&self) -> ImageMemoryPolicy {
        self.memory_policy
    }
}

pub struct ImageExportRequest {
    path: PathBuf,
    source: ImageExportSource,
    orientation: ImageOrientation,
    options: ExportOptions,
}

enum ImageExportSource {
    SharedImageState(Arc<ImageState>),
    SharedPixels(Arc<PixelImage>),
}

impl ImageExportRequest {
    fn new(
        path: PathBuf,
        source: ImageExportSource,
        orientation: ImageOrientation,
        options: ExportOptions,
    ) -> Self {
        Self {
            path,
            source,
            orientation,
            options,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn options(&self) -> ExportOptions {
        self.options
    }

    pub fn export(self) -> Result<(), ViewerAppError> {
        let Self {
            path,
            source,
            orientation,
            options,
        } = self;
        let export_orientation = orientation.then_rotation(options.rotation());
        source.export(&path, export_orientation, options)?;
        Ok(())
    }
}

impl ImageExportSource {
    fn export(
        self,
        path: &Path,
        orientation: ImageOrientation,
        options: ExportOptions,
    ) -> Result<(), ViewerAppError> {
        match self {
            Self::SharedImageState(image_state) => {
                let ImageState::Loaded(image) = image_state.as_ref() else {
                    return Err(ViewerAppError::NoImageToExport);
                };
                export_borrowed_source_pixels(path, image.pixels(), orientation, options)?;
            }
            Self::SharedPixels(pixels) => {
                export_borrowed_source_pixels(path, pixels.as_ref(), orientation, options)?;
            }
        }
        Ok(())
    }

    #[cfg(test)]
    fn size(&self) -> Option<ImageSize> {
        match self {
            Self::SharedImageState(image_state) => match image_state.as_ref() {
                ImageState::Loaded(image) => Some(image.pixels().size()),
                ImageState::Empty => None,
            },
            Self::SharedPixels(pixels) => Some(pixels.size()),
        }
    }

    #[cfg(test)]
    fn shared_pixels(&self) -> Option<&PixelImage> {
        match self {
            Self::SharedPixels(pixels) => Some(pixels.as_ref()),
            Self::SharedImageState(_) => None,
        }
    }

    #[cfg(test)]
    fn is_shared_image_state(&self) -> bool {
        matches!(self, Self::SharedImageState(_))
    }
}

fn export_borrowed_source_pixels(
    path: &Path,
    pixels: &PixelImage,
    orientation: ImageOrientation,
    options: ExportOptions,
) -> Result<(), ViewerAppError> {
    let pixels = if orientation.is_identity() {
        resize_borrowed_export_pixels(path, pixels, options)?
    } else {
        let pixels = orient_pixel_image(pixels, orientation).ok_or_else(|| {
            ViewerAppError::from(ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })
        })?;
        ExportPixels::Owned(resize_export_pixels(path, pixels, options)?)
    };
    export_pixels(path, pixels, options)?;
    Ok(())
}

#[derive(Debug, Clone, PartialEq, Eq)]
enum ImageDecodeState {
    Idle,
    Loading {
        generation: DecodeGeneration,
        path: PathBuf,
        file_version: Option<ImageFileVersion>,
        purpose: ImageDecodePurpose,
        navigation: Option<NavigationDecodeState>,
    },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct NavigationDecodeState {
    direction: ImageNavigationDirection,
    attempt_index: usize,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct PendingFolderScan {
    generation: DecodeGeneration,
    path: PathBuf,
}

#[derive(Debug, Clone, PartialEq)]
struct NavigationPreloadCacheEntry {
    path_key: u64,
    image: LoadedImage,
    viewport: ViewportSize,
    memory_policy: ImageMemoryPolicy,
    animation_timing: AnimationTimingSettings,
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ImageDecodeCoordinator {
    generation: DecodeGeneration,
    state: ImageDecodeState,
}

impl ImageDecodeCoordinator {
    fn new() -> Self {
        Self {
            generation: DecodeGeneration::ZERO,
            state: ImageDecodeState::Idle,
        }
    }

    fn generation(&self) -> DecodeGeneration {
        self.generation
    }

    fn is_idle(&self) -> bool {
        matches!(self.state, ImageDecodeState::Idle)
    }

    fn is_stale(&self, generation: DecodeGeneration) -> bool {
        is_stale_decode_generation(self.generation, generation)
    }

    fn begin_initial(
        &mut self,
        path: PathBuf,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
        animation_timing: AnimationTimingSettings,
        purpose: ImageDecodePurpose,
        navigation: Option<NavigationDecodeState>,
    ) -> ImageDecodeRequest {
        self.generation = self.generation.next();
        self.state = ImageDecodeState::Loading {
            generation: self.generation,
            path: path.clone(),
            file_version: None,
            purpose,
            navigation,
        };
        ImageDecodeRequest::new(
            self.generation,
            path,
            None,
            viewport,
            memory_policy,
            animation_timing,
            purpose,
        )
    }

    fn begin_full_resolution(
        &mut self,
        path: PathBuf,
        file_version: ImageFileVersion,
        viewport: ViewportSize,
        memory_policy: ImageMemoryPolicy,
        animation_timing: AnimationTimingSettings,
    ) -> ImageDecodeRequest {
        self.state = ImageDecodeState::Loading {
            generation: self.generation,
            path: path.clone(),
            file_version: Some(file_version),
            purpose: ImageDecodePurpose::FullResolution,
            navigation: None,
        };
        ImageDecodeRequest::new(
            self.generation,
            path,
            Some(file_version),
            viewport,
            memory_policy,
            animation_timing,
            ImageDecodePurpose::FullResolution,
        )
    }

    fn clear(&mut self) {
        self.state = ImageDecodeState::Idle;
    }

    fn active_initial_state(
        &self,
        generation: DecodeGeneration,
    ) -> Option<(ImageDecodePurpose, Option<NavigationDecodeState>)> {
        match self.state {
            ImageDecodeState::Loading {
                generation: active_generation,
                purpose,
                navigation,
                ..
            } if active_generation == generation
                && purpose != ImageDecodePurpose::FullResolution =>
            {
                Some((purpose, navigation))
            }
            ImageDecodeState::Idle | ImageDecodeState::Loading { .. } => None,
        }
    }

    fn active_initial_path(&self, generation: DecodeGeneration) -> Option<&Path> {
        match &self.state {
            ImageDecodeState::Loading {
                generation: active_generation,
                path,
                purpose,
                ..
            } if *active_generation == generation
                && *purpose != ImageDecodePurpose::FullResolution =>
            {
                Some(path.as_path())
            }
            ImageDecodeState::Idle | ImageDecodeState::Loading { .. } => None,
        }
    }

    fn active_full_resolution_source(
        &self,
        generation: DecodeGeneration,
    ) -> Option<(&Path, ImageFileVersion)> {
        match &self.state {
            ImageDecodeState::Loading {
                generation: active_generation,
                path,
                file_version: Some(file_version),
                purpose: ImageDecodePurpose::FullResolution,
                ..
            } if *active_generation == generation => Some((path.as_path(), *file_version)),
            ImageDecodeState::Idle | ImageDecodeState::Loading { .. } => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq)]
pub enum AnimationFrameOutcome {
    Updated,
    StateChanged,
    NeedsDecode(AnimationFrameDecodeRequest),
    Unchanged,
}

#[derive(Debug)]
pub struct ViewerApp {
    title: String,
    config: AppConfig,
    viewport: ViewportSize,
    image_state: Arc<ImageState>,
    decode: ImageDecodeCoordinator,
    memory_policy: ImageMemoryPolicy,
    image_folder: ImageFolder,
    navigation_preload_cache: Vec<NavigationPreloadCacheEntry>,
    image_revision: u64,
    user_rotation: ImageRotation,
    render_cache: RenderCache,
    animation_frame_cache: Vec<AnimationFrameCacheEntry>,
    pending_animation_frame: Option<usize>,
    pending_animation_file_version: Option<ImageFileVersion>,
    pending_animation_state: Option<AnimationPlayback>,
    pending_folder_scan: Option<PendingFolderScan>,
    pending_navigation_after_folder_scan: Option<ImageNavigationDirection>,
    status_message: Option<String>,
    view_transform: ViewTransform,
    panning: Option<PanningState>,
    defer_scaling_cache_rebuild: bool,
    defer_next_paint_scaling_cache_rebuild: bool,
    scaling_cache_rebuild_deferred: bool,
    paint_count: u64,
}

#[derive(Debug, Clone, Copy, PartialEq)]
struct PanningState {
    start_point: ViewportPoint,
    start_offset: ViewOffset,
}

#[derive(Debug, Default)]
struct RenderCache {
    oriented_image: Option<OrientedImageCache>,
    scaled_image: Option<ScaledImageCache>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct OrientedImageCacheKey {
    image_revision: u64,
    source_size: ImageSize,
    orientation: ImageOrientation,
}

impl OrientedImageCacheKey {
    fn new(image_revision: u64, source_size: ImageSize, orientation: ImageOrientation) -> Self {
        Self {
            image_revision,
            source_size,
            orientation,
        }
    }

    fn display_size(self) -> ImageSize {
        self.source_size.with_orientation(self.orientation)
    }
}

impl RenderCache {
    fn clear_all(&mut self) {
        self.oriented_image = None;
        self.scaled_image = None;
    }

    fn clear_oriented(&mut self) {
        self.oriented_image = None;
    }

    fn clear_scaled(&mut self) {
        self.scaled_image = None;
    }

    fn oriented_image_needs_rebuild(
        &self,
        image_revision: u64,
        source_size: ImageSize,
        orientation: ImageOrientation,
    ) -> bool {
        let key = OrientedImageCacheKey::new(image_revision, source_size, orientation);
        self.oriented_image
            .as_ref()
            .map(|cache| cache.key != key)
            .unwrap_or(true)
    }

    fn store_oriented_image(
        &mut self,
        image_revision: u64,
        source_size: ImageSize,
        orientation: ImageOrientation,
        pixels: PixelImage,
        last_used: u64,
    ) {
        self.oriented_image = Some(OrientedImageCache {
            key: OrientedImageCacheKey::new(image_revision, source_size, orientation),
            pixels: Arc::new(pixels),
            last_used,
        });
    }

    fn matching_oriented_image(
        &self,
        image_revision: u64,
        source_size: ImageSize,
        orientation: ImageOrientation,
    ) -> Option<&OrientedImageCache> {
        self.matching_oriented_image_by_key(OrientedImageCacheKey::new(
            image_revision,
            source_size,
            orientation,
        ))
    }

    fn matching_oriented_image_by_key(
        &self,
        key: OrientedImageCacheKey,
    ) -> Option<&OrientedImageCache> {
        self.oriented_image
            .as_ref()
            .filter(|cache| cache.key == key && cache.pixels.size() == key.display_size())
    }

    fn oriented_image_by_key(&self, key: OrientedImageCacheKey) -> Option<&PixelImage> {
        self.matching_oriented_image_by_key(key)
            .map(|cache| cache.pixels.as_ref())
    }

    fn oriented_image(
        &self,
        image_revision: u64,
        source_size: ImageSize,
        orientation: ImageOrientation,
    ) -> Option<&PixelImage> {
        self.matching_oriented_image(image_revision, source_size, orientation)
            .map(|cache| cache.pixels.as_ref())
    }

    fn shared_oriented_image(
        &self,
        image_revision: u64,
        source_size: ImageSize,
        orientation: ImageOrientation,
    ) -> Option<Arc<PixelImage>> {
        self.matching_oriented_image(image_revision, source_size, orientation)
            .map(|cache| Arc::clone(&cache.pixels))
    }

    fn scaled_key(&self) -> Option<ScalingCacheKey> {
        self.scaled_image.as_ref().map(|cache| cache.key)
    }

    fn store_scaled_image(&mut self, key: ScalingCacheKey, pixels: PixelImage, last_used: u64) {
        self.scaled_image = Some(ScaledImageCache {
            key,
            pixels,
            last_used,
        });
    }

    fn scaled_image(&self, key: ScalingCacheKey) -> Option<&ScaledImageCache> {
        self.scaled_image.as_ref().filter(|cache| cache.key == key)
    }

    fn append_memory_entries(
        &self,
        entries: &mut Vec<MemoryCacheEntry>,
        protected_oriented_image: Option<OrientedImageCacheKey>,
    ) {
        if let Some(cache) = &self.oriented_image {
            if protected_oriented_image != Some(cache.key) {
                entries.push(MemoryCacheEntry::new(
                    ImageCacheSlot::OrientedImage,
                    cache.pixels.byte_len(),
                    cache.last_used,
                ));
            }
        }
        if let Some(cache) = &self.scaled_image {
            entries.push(MemoryCacheEntry::new(
                ImageCacheSlot::ScaledImage,
                cache.pixels.byte_len(),
                cache.last_used,
            ));
        }
    }

    fn evict(&mut self, slot: ImageCacheSlot) {
        match slot {
            ImageCacheSlot::OrientedImage => self.oriented_image = None,
            ImageCacheSlot::ScaledImage => self.scaled_image = None,
            ImageCacheSlot::AnimationFrame { .. } | ImageCacheSlot::NavigationPreload { .. } => {}
        }
    }

    #[cfg(test)]
    fn has_oriented_image(&self) -> bool {
        self.oriented_image.is_some()
    }

    #[cfg(test)]
    fn has_scaled_image(&self) -> bool {
        self.scaled_image.is_some()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct OrientedImageCache {
    key: OrientedImageCacheKey,
    pixels: Arc<PixelImage>,
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct ScaledImageCache {
    key: ScalingCacheKey,
    pixels: PixelImage,
    last_used: u64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct AnimationFrameCacheEntry {
    frame_index: usize,
    rgba8: Arc<Rgba8Image>,
    last_used: u64,
}

#[derive(Debug)]
pub struct RenderImage<'a> {
    rect: ImageDisplayRect,
    pixels: &'a PixelImage,
    scaling_quality: ScalingQuality,
    cache_key: RenderImageCacheKey,
}

#[derive(Debug, Clone, Copy)]
pub struct DisplayPixelSource<'a> {
    pixels: &'a PixelImage,
    orientation: ImageOrientation,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderImageCacheKey {
    image_revision: u64,
    orientation: ImageOrientation,
    source_size: ImageSize,
}

impl RenderImageCacheKey {
    pub fn new(image_revision: u64, orientation: ImageOrientation, source_size: ImageSize) -> Self {
        Self {
            image_revision,
            orientation,
            source_size,
        }
    }
}

impl<'a> RenderImage<'a> {
    pub fn rect(&self) -> ImageDisplayRect {
        self.rect
    }

    pub fn pixels(&self) -> &'a PixelImage {
        self.pixels
    }

    pub fn rgba8(&self) -> Option<&'a Rgba8Image> {
        self.pixels.as_rgba8()
    }

    pub fn scaling_quality(&self) -> ScalingQuality {
        self.scaling_quality
    }

    pub fn cache_key(&self) -> RenderImageCacheKey {
        self.cache_key
    }
}

impl<'a> DisplayPixelSource<'a> {
    fn new(pixels: &'a PixelImage, orientation: ImageOrientation) -> Self {
        Self {
            pixels,
            orientation,
        }
    }

    pub fn pixels(&self) -> &'a PixelImage {
        self.pixels
    }

    pub fn orientation(&self) -> ImageOrientation {
        self.orientation
    }
}

impl ViewerApp {
    pub fn new() -> Self {
        Self::with_config(AppConfig::default())
    }

    pub fn with_config(config: AppConfig) -> Self {
        let view_transform = config.default_view_transform();
        let memory_policy = config.image_memory_policy();
        Self {
            title: DEFAULT_TITLE.to_owned(),
            config,
            viewport: ViewportSize::EMPTY,
            image_state: Arc::new(ImageState::Empty),
            decode: ImageDecodeCoordinator::new(),
            memory_policy,
            image_folder: ImageFolder::empty(),
            navigation_preload_cache: Vec::new(),
            image_revision: 0,
            user_rotation: ImageRotation::ZERO,
            render_cache: RenderCache::default(),
            animation_frame_cache: Vec::new(),
            pending_animation_frame: None,
            pending_animation_file_version: None,
            pending_animation_state: None,
            pending_folder_scan: None,
            pending_navigation_after_folder_scan: None,
            status_message: None,
            view_transform,
            panning: None,
            defer_scaling_cache_rebuild: false,
            defer_next_paint_scaling_cache_rebuild: false,
            scaling_cache_rebuild_deferred: false,
            paint_count: 0,
        }
    }

    pub fn title(&self) -> &str {
        &self.title
    }

    pub fn config(&self) -> &AppConfig {
        &self.config
    }

    pub fn config_snapshot(&self) -> AppConfig {
        self.config.clone()
    }

    pub fn window_bounds(&self) -> Option<WindowBounds> {
        self.config.window_bounds()
    }

    pub fn set_window_bounds(&mut self, window_bounds: Option<WindowBounds>) {
        self.config.set_window_bounds(window_bounds);
    }

    pub fn apply_config(&mut self, config: AppConfig) -> bool {
        if self.config == config {
            return false;
        }

        let old_scaling_quality = self.config.scaling_quality();
        let old_zoom_settings = self.config.zoom_settings();
        let old_animation_autoplay = self.config.animation_autoplay();
        let old_animation_timing = self.config.animation_timing_settings();
        let old_memory_policy = self.memory_policy;
        let old_navigation_settings = self.config.navigation_settings();

        self.config = config;
        self.memory_policy = self.config.image_memory_policy();

        if self.config.scaling_quality() != old_scaling_quality
            || self.config.zoom_settings() != old_zoom_settings
        {
            self.invalidate_scaled_image_cache();
        }
        if let Some(image_size) = self.current_view_image_size() {
            self.view_transform = self.view_transform.constrain_to_viewport_with_settings(
                self.viewport,
                image_size,
                self.config.zoom_settings(),
            );
        }
        let animation_autoplay_changed = self.config.animation_autoplay() != old_animation_autoplay;
        if animation_autoplay_changed
            || self.config.animation_timing_settings() != old_animation_timing
        {
            self.apply_animation_config_to_current_image(animation_autoplay_changed);
        }
        if self.memory_policy != old_memory_policy
            || self.config.animation_timing_settings() != old_animation_timing
            || self.config.navigation_settings() != old_navigation_settings
        {
            self.clear_navigation_preloads();
        }
        self.panning = None;
        self.enforce_memory_policy();
        self.update_title();
        true
    }

    pub fn recent_folder(&self) -> Option<&Path> {
        self.config.recent_folder()
    }

    pub fn export_default_quality(&self) -> u8 {
        self.config.export_default_quality()
    }

    pub fn set_export_default_quality(&mut self, quality: u8) {
        self.config.set_export_default_quality(quality);
    }

    pub fn zoom_step_factor(&self) -> f64 {
        self.config.zoom_settings().zoom_step_factor()
    }

    pub fn default_export_format_for_source(
        &self,
        source_format: SupportedImageFormat,
    ) -> ExportFormat {
        self.config
            .export_settings()
            .default_export_format_policy()
            .export_format_for_source(source_format)
    }

    pub fn suggested_export_path(
        &self,
        source_path: &Path,
        source_format: SupportedImageFormat,
    ) -> PathBuf {
        let export_settings = self.config.export_settings();
        suggested_export_path_with_suffix(
            source_path,
            export_settings
                .default_export_format_policy()
                .export_format_for_source(source_format),
            export_settings.export_filename_suffix(),
        )
    }

    pub fn export_options(&self, format: ExportFormat, quality: Option<u8>) -> ExportOptions {
        ExportOptions::new(format, quality).with_jpeg_alpha_background_rgb(
            self.config.export_settings().jpeg_alpha_background_rgb(),
        )
    }

    pub fn viewport(&self) -> ViewportSize {
        self.viewport
    }

    pub fn image_state(&self) -> &ImageState {
        self.image_state.as_ref()
    }

    fn image_state_mut(&mut self) -> &mut ImageState {
        Arc::make_mut(&mut self.image_state)
    }

    pub fn has_animation(&self) -> bool {
        self.image_state.has_animation()
    }

    pub fn animation_timer_interval_ms(&self) -> Option<u32> {
        if self.pending_animation_frame.is_some() {
            return None;
        }

        self.current_animation_playback()
            .and_then(animation_timer_interval_ms)
    }

    pub fn animation_debug_summary(&self) -> Option<String> {
        let playback = self.current_animation_playback()?;
        Some(format!(
            "state={:?}; frame_index={}; frame_count={}; delay_ms={:?}; loop={:?}; completed_loops={}; pending_frame={:?}",
            playback.playback_state(),
            playback.current_frame_index(),
            playback.frame_count(),
            animation_timer_interval_ms(playback),
            playback.loop_policy(),
            playback.completed_loops(),
            self.pending_animation_frame,
        ))
    }

    pub fn image_folder(&self) -> &ImageFolder {
        &self.image_folder
    }

    pub fn decode_generation(&self) -> DecodeGeneration {
        self.decode.generation()
    }

    pub fn memory_policy(&self) -> ImageMemoryPolicy {
        self.memory_policy
    }

    pub fn rotation(&self) -> ImageRotation {
        self.user_rotation
    }

    pub fn user_rotation(&self) -> ImageRotation {
        self.user_rotation
    }

    pub fn display_orientation(&self) -> Option<ImageOrientation> {
        self.current_display_orientation()
    }

    pub fn view_transform(&self) -> ViewTransform {
        self.view_transform
    }

    pub fn view_state(&self) -> ViewTransform {
        self.view_transform
    }

    pub fn paint_count(&self) -> u64 {
        self.paint_count
    }

    pub fn image_info_text(&self) -> Option<String> {
        let status_ui = self.config.status_ui_settings();
        if !status_ui.show_status_bar() {
            return None;
        }

        if let Some(message) = &self.status_message {
            return Some(message.clone());
        }

        match self.image_state.as_ref() {
            ImageState::Empty => None,
            ImageState::Loaded(image) => Some(image_status_text_with_settings(
                image,
                self.view_transform,
                self.user_rotation,
                self.config.zoom_settings(),
                status_ui.detailed_status_text(),
            )),
        }
    }

    pub fn handle_command(
        &mut self,
        command: Command,
    ) -> Result<AppCommandOutcome, ViewerAppError> {
        self.clear_status_message();

        match command {
            Command::Navigate(direction) => {
                let outcome = self.navigate_image(direction)?;
                Ok(match outcome {
                    NavigationOutcome::Moved => AppCommandOutcome::Changed,
                    NavigationOutcome::Noop => AppCommandOutcome::Unchanged,
                })
            }
            Command::ZoomIn => Ok(command_outcome(
                self.zoom_from_center(self.config.zoom_settings().zoom_step_factor()),
            )),
            Command::ZoomOut => Ok(command_outcome(
                self.zoom_from_center(1.0 / self.config.zoom_settings().zoom_step_factor()),
            )),
            Command::ActualSize => Ok(command_outcome(self.show_actual_size())),
            Command::FitToWindow => Ok(command_outcome(self.fit_to_window())),
            Command::RotateClockwise => Ok(command_outcome(self.rotate_clockwise())),
            Command::RotateCounterClockwise => Ok(command_outcome(self.rotate_counter_clockwise())),
            Command::OpenImage
            | Command::ExportImage
            | Command::CopyImageToClipboard
            | Command::Animation(_)
            | Command::ContextualSpace
            | Command::ToggleFullscreen
            | Command::OpenAbout
            | Command::OpenSettings
            | Command::ExitFullscreenOrQuit
            | Command::Quit => Ok(AppCommandOutcome::Unhandled),
        }
    }

    pub fn handle_animation_command(&mut self, command: AnimationCommand) -> AnimationFrameOutcome {
        self.clear_status_message();
        self.discard_pending_animation_frame_transition();

        let Some(playback) = self.current_animation_playback().cloned() else {
            return AnimationFrameOutcome::Unchanged;
        };

        let transition = match command {
            AnimationCommand::TogglePlayback => animation_state_after_toggle(&playback),
            AnimationCommand::StepFrame(direction) => {
                animation_state_after_manual_step(&playback, direction)
            }
            AnimationCommand::FirstFrame => animation_state_after_home(&playback),
        };
        if matches!(command, AnimationCommand::TogglePlayback) {
            self.config.set_animation_autoplay(matches!(
                transition.state().playback_state(),
                AnimationPlaybackState::Playing
            ));
        }
        self.apply_animation_transition(transition)
    }

    pub fn handle_animation_timer(&mut self) -> AnimationFrameOutcome {
        if self.pending_animation_frame.is_some() {
            return AnimationFrameOutcome::Unchanged;
        }

        let Some(playback) = self.current_animation_playback().cloned() else {
            return AnimationFrameOutcome::Unchanged;
        };

        self.apply_animation_transition(animation_state_after_timer_tick(&playback))
    }

    pub fn begin_image_decode(&mut self, path: PathBuf) -> ImageDecodeRequest {
        self.begin_image_decode_with_purpose(path, ImageDecodePurpose::OpenImage, None)
    }

    pub fn begin_navigation_or_use_preloaded(
        &mut self,
        direction: ImageNavigationDirection,
    ) -> NavigationStartOutcome {
        let navigation_settings = self.config.navigation_settings();
        if let Some(path) = self
            .image_folder
            .navigation_path_for_attempt(direction, navigation_settings, 0)
            .map(Path::to_path_buf)
        {
            if let Some(image) = self.take_valid_preloaded_navigation_image(&path) {
                self.replace_loaded_navigation_image(image);
                return NavigationStartOutcome::AppliedPreloaded;
            }
        }

        self.begin_navigation_decode(direction)
            .map(NavigationStartOutcome::Decode)
            .unwrap_or(NavigationStartOutcome::Noop)
    }

    fn begin_image_decode_with_purpose(
        &mut self,
        path: PathBuf,
        purpose: ImageDecodePurpose,
        navigation: Option<NavigationDecodeState>,
    ) -> ImageDecodeRequest {
        self.clear_status_message();
        self.panning = None;
        self.pending_animation_frame = None;
        self.pending_animation_file_version = None;
        self.pending_animation_state = None;
        self.pending_folder_scan = None;
        self.pending_navigation_after_folder_scan = None;
        self.cancel_deferred_scaling_cache_rebuild();
        self.decode.begin_initial(
            path,
            self.viewport,
            self.memory_policy,
            self.config.animation_timing_settings(),
            purpose,
            navigation,
        )
    }

    pub fn navigation_preload_requests(&self) -> Vec<ImagePreloadRequest> {
        if !self.decode.is_idle() || !self.image_state.has_image() {
            return Vec::new();
        }

        let navigation_settings = self.config.navigation_settings();
        let mut paths = Vec::new();
        for direction in [
            ImageNavigationDirection::Previous,
            ImageNavigationDirection::Next,
        ] {
            let Some(path) =
                self.image_folder
                    .navigation_path_for_attempt(direction, navigation_settings, 0)
            else {
                continue;
            };
            if self.current_source_path() == Some(path)
                || paths.iter().any(|existing: &PathBuf| existing == path)
                || self.has_current_navigation_preload(path)
            {
                continue;
            }
            paths.push(path.to_path_buf());
        }

        paths
            .into_iter()
            .map(|path| {
                ImagePreloadRequest::new(
                    self.decode.generation(),
                    path,
                    self.viewport,
                    self.memory_policy,
                    self.config.animation_timing_settings(),
                )
            })
            .collect()
    }

    pub fn store_preloaded_navigation_image(
        &mut self,
        request: &ImagePreloadRequest,
        image: LoadedImage,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(request.generation())
            || request.viewport() != self.viewport
            || request.memory_policy() != self.memory_policy
            || request.animation_timing() != self.config.animation_timing_settings()
            || request.path() != image.metadata().path()
            || !self.is_navigation_preload_target(request.path())
        {
            return DecodeApplyOutcome::Stale;
        }

        let path_key = navigation_preload_path_key(request.path());
        if let Some(entry) = self.navigation_preload_cache.iter_mut().find(|entry| {
            entry.path_key == path_key && entry.image.metadata().path() == request.path()
        }) {
            entry.image = image;
            entry.viewport = request.viewport();
            entry.memory_policy = request.memory_policy();
            entry.animation_timing = request.animation_timing();
            entry.last_used = self.paint_count;
        } else {
            self.navigation_preload_cache
                .push(NavigationPreloadCacheEntry {
                    path_key,
                    image,
                    viewport: request.viewport(),
                    memory_policy: request.memory_policy(),
                    animation_timing: request.animation_timing(),
                    last_used: self.paint_count,
                });
        }
        self.enforce_memory_policy();
        DecodeApplyOutcome::Applied
    }

    pub fn begin_navigation_decode(
        &mut self,
        direction: ImageNavigationDirection,
    ) -> Option<ImageDecodeRequest> {
        let request = self.begin_navigation_decode_attempt(direction, 0);
        if request.is_none() && self.is_current_folder_scan_pending() {
            self.pending_navigation_after_folder_scan = Some(direction);
        }
        request
    }

    fn begin_navigation_decode_attempt(
        &mut self,
        direction: ImageNavigationDirection,
        attempt_index: usize,
    ) -> Option<ImageDecodeRequest> {
        let navigation_settings = self.config.navigation_settings();
        let path = self
            .image_folder
            .navigation_path_for_attempt(direction, navigation_settings, attempt_index)
            .map(Path::to_path_buf)?;
        Some(self.begin_image_decode_with_purpose(
            path,
            ImageDecodePurpose::FolderNavigation(direction),
            Some(NavigationDecodeState {
                direction,
                attempt_index,
            }),
        ))
    }

    fn is_current_folder_scan_pending(&self) -> bool {
        let Some(pending) = &self.pending_folder_scan else {
            return false;
        };
        self.current_source_path() == Some(pending.path.as_path())
            && pending.generation == self.decode.generation()
    }

    pub fn begin_full_resolution_decode(&mut self) -> Option<ImageDecodeRequest> {
        if !self.decode.is_idle() {
            return None;
        }

        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };
        if image.is_animated() || image.has_full_resolution() {
            return None;
        }

        let image_size = image
            .source_size()
            .with_orientation(self.current_display_orientation()?);
        let effective_scale = self.view_transform.effective_scale_with_settings(
            self.viewport,
            image_size,
            self.config.zoom_settings(),
        )?;
        if !should_request_full_resolution_for_view(
            image.buffer_kind(),
            effective_scale,
            image.source_size(),
            self.memory_policy,
        ) {
            return None;
        }

        let path = image.metadata().path().to_path_buf();
        let file_version = image.metadata().file_version()?;
        Some(self.decode.begin_full_resolution(
            path,
            file_version,
            self.viewport,
            self.memory_policy,
            self.config.animation_timing_settings(),
        ))
    }

    pub fn apply_decoded_image(
        &mut self,
        generation: DecodeGeneration,
        image: LoadedImage,
        image_folder: ImageFolder,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }
        let Some((purpose, _)) = self.decode.active_initial_state(generation) else {
            return DecodeApplyOutcome::Stale;
        };
        if self
            .decode
            .active_initial_path(generation)
            .is_none_or(|path| path != image.metadata().path())
        {
            return DecodeApplyOutcome::Stale;
        }

        if purpose.is_folder_navigation() {
            self.replace_loaded_navigation_image(image);
        } else {
            let path = image.metadata().path().to_path_buf();
            self.replace_loaded_image(image, image_folder);
            self.pending_folder_scan = Some(PendingFolderScan { generation, path });
        }
        DecodeApplyOutcome::Applied
    }

    pub fn apply_scanned_image_folder(
        &mut self,
        generation: DecodeGeneration,
        current_path: &Path,
        image_folder: ImageFolder,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }
        if self.current_source_path() != Some(current_path) {
            return DecodeApplyOutcome::Stale;
        }

        self.image_folder = image_folder;
        self.pending_folder_scan = None;
        DecodeApplyOutcome::Applied
    }

    pub fn finish_pending_folder_scan_without_update(
        &mut self,
        generation: DecodeGeneration,
        current_path: &Path,
    ) -> DecodeApplyOutcome {
        let Some(pending) = &self.pending_folder_scan else {
            return DecodeApplyOutcome::Stale;
        };
        if self.decode.is_stale(generation)
            || pending.generation != generation
            || pending.path.as_path() != current_path
            || self.current_source_path() != Some(current_path)
        {
            return DecodeApplyOutcome::Stale;
        }

        self.pending_folder_scan = None;
        self.pending_navigation_after_folder_scan = None;
        DecodeApplyOutcome::Applied
    }

    pub fn take_pending_navigation_after_folder_scan(&mut self) -> Option<ImageDecodeRequest> {
        let direction = self.pending_navigation_after_folder_scan.take()?;
        self.begin_navigation_decode_attempt(direction, 0)
    }

    pub fn apply_full_resolution_image(
        &mut self,
        generation: DecodeGeneration,
        file_version: Option<ImageFileVersion>,
        pixels: impl Into<PixelImage>,
    ) -> DecodeApplyOutcome {
        let pixels = pixels.into();
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }

        let Some((path, expected_file_version)) = self
            .decode
            .active_full_resolution_source(generation)
            .map(|(path, file_version)| (path.to_path_buf(), file_version))
        else {
            return DecodeApplyOutcome::Stale;
        };

        self.decode.clear();
        let ImageState::Loaded(image) = self.image_state_mut() else {
            return DecodeApplyOutcome::Unchanged;
        };
        if image.metadata().path() != path
            || image.metadata().file_version() != Some(expected_file_version)
            || file_version != Some(expected_file_version)
        {
            return DecodeApplyOutcome::Stale;
        }
        if !image.replace_with_full_resolution(pixels) {
            return DecodeApplyOutcome::Unchanged;
        }

        self.image_revision = self.image_revision.wrapping_add(1);
        self.invalidate_image_caches();
        self.defer_next_paint_scaling_cache_rebuild = true;
        self.enforce_memory_policy();
        self.update_title();
        DecodeApplyOutcome::Applied
    }

    pub fn apply_animation_frame(
        &mut self,
        generation: DecodeGeneration,
        frame_index: usize,
        path: &Path,
        file_version: Option<ImageFileVersion>,
        rgba8: Rgba8Image,
    ) -> DecodeApplyOutcome {
        self.apply_animation_frame_pixels(generation, frame_index, path, file_version, rgba8.into())
    }

    pub(crate) fn apply_animation_frame_pixels(
        &mut self,
        generation: DecodeGeneration,
        frame_index: usize,
        path: &Path,
        file_version: Option<ImageFileVersion>,
        rgba8: AnimationFramePixels,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }
        let Some(expected_file_version) = self.pending_animation_file_version else {
            return DecodeApplyOutcome::Stale;
        };
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return DecodeApplyOutcome::Stale;
        };
        if image.metadata().path() != path
            || image.metadata().file_version() != Some(expected_file_version)
            || file_version != Some(expected_file_version)
        {
            return DecodeApplyOutcome::Stale;
        }
        if self.pending_animation_frame != Some(frame_index) {
            return DecodeApplyOutcome::Stale;
        }

        self.pending_animation_frame = None;
        self.pending_animation_file_version = None;
        let Some(playback) = self.pending_animation_state.take() else {
            return DecodeApplyOutcome::Unchanged;
        };
        if !self.apply_animation_frame_image(frame_index, rgba8, playback) {
            return DecodeApplyOutcome::Unchanged;
        }

        DecodeApplyOutcome::Applied
    }

    pub fn finish_failed_animation_frame_decode(
        &mut self,
        generation: DecodeGeneration,
        frame_index: usize,
        path: &Path,
        file_version: Option<ImageFileVersion>,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }
        let Some(expected_file_version) = self.pending_animation_file_version else {
            return DecodeApplyOutcome::Stale;
        };
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return DecodeApplyOutcome::Stale;
        };
        if image.metadata().path() != path
            || image.metadata().file_version() != Some(expected_file_version)
            || file_version != Some(expected_file_version)
        {
            return DecodeApplyOutcome::Stale;
        }
        if self.pending_animation_frame != Some(frame_index) {
            return DecodeApplyOutcome::Stale;
        }

        self.pending_animation_frame = None;
        self.pending_animation_file_version = None;
        self.pending_animation_state = None;
        self.pause_current_animation_after_frame_decode_failure();
        DecodeApplyOutcome::Applied
    }

    pub fn finish_failed_initial_decode(
        &mut self,
        generation: DecodeGeneration,
        error: &ViewerAppError,
    ) -> DecodeFailurePresentation {
        if self.decode.is_stale(generation) {
            return DecodeFailurePresentation::Stale;
        }

        let Some((purpose, navigation)) = self.decode.active_initial_state(generation) else {
            return DecodeFailurePresentation::Stale;
        };
        if purpose.is_folder_navigation() {
            if let Some(navigation) = navigation {
                if let Some(request) = self.navigation_retry_request(navigation) {
                    return DecodeFailurePresentation::RetryNavigation(request);
                }
            }
            self.decode.clear();
            self.status_message = Some(navigation_failure_status_text(
                error,
                self.config.navigation_settings(),
                self.config.ui_language(),
            ));
            DecodeFailurePresentation::StatusMessage
        } else {
            self.decode.clear();
            DecodeFailurePresentation::MessageBox
        }
    }

    pub fn finish_failed_decode(
        &mut self,
        generation: DecodeGeneration,
        file_version: Option<ImageFileVersion>,
    ) -> DecodeApplyOutcome {
        if self.decode.is_stale(generation) {
            return DecodeApplyOutcome::Stale;
        }
        let Some((_, expected_file_version)) =
            self.decode.active_full_resolution_source(generation)
        else {
            return DecodeApplyOutcome::Stale;
        };
        if file_version != Some(expected_file_version) {
            return DecodeApplyOutcome::Stale;
        }

        self.decode.clear();
        DecodeApplyOutcome::Applied
    }

    fn navigation_retry_request(
        &mut self,
        navigation: NavigationDecodeState,
    ) -> Option<ImageDecodeRequest> {
        let settings = self.config.navigation_settings();
        if !settings.auto_skip_failed_navigation() {
            return None;
        }
        let next_attempt = navigation.attempt_index.checked_add(1)?;
        if next_attempt >= settings.max_navigation_attempts_per_command() {
            return None;
        }
        self.begin_navigation_decode_attempt(navigation.direction, next_attempt)
    }

    pub fn load_image(&mut self, path: impl AsRef<Path>) -> Result<(), ViewerAppError> {
        let image = load_image_file_for_view_with_timing(
            path.as_ref(),
            self.viewport,
            self.memory_policy,
            self.config.animation_timing_settings(),
            None,
        )?;
        let (image_folder, _) = scan_image_folder_for_file_or_empty(image.metadata().path());
        self.replace_loaded_image(image, image_folder);
        Ok(())
    }

    pub fn load_image_with_profile(
        &mut self,
        path: impl AsRef<Path>,
    ) -> Result<ImageOpenProfile, ViewerAppError> {
        let mut profiler = ImageOpenProfiler::new();
        let image = load_image_file_for_view_with_timing_and_profile(
            path.as_ref(),
            self.viewport,
            self.memory_policy,
            self.config.animation_timing_settings(),
            None,
            &mut profiler,
        )?;
        let (image_folder, _) = scan_image_folder_for_file_or_empty(image.metadata().path());
        profiler.record_stage("app.scan_image_folder");
        self.replace_loaded_image(image, image_folder);
        profiler.record_stage("app.replace_loaded_image");
        Ok(profiler.finish())
    }

    pub fn navigate_image(
        &mut self,
        direction: ImageNavigationDirection,
    ) -> Result<NavigationOutcome, ViewerAppError> {
        let navigation_settings = self.config.navigation_settings();
        let max_attempts = navigation_settings.max_navigation_attempts_per_command();
        let mut last_error = None;

        for attempt_index in 0..max_attempts {
            let Some(path) = self
                .image_folder
                .navigation_path_for_attempt(direction, navigation_settings, attempt_index)
                .map(Path::to_path_buf)
            else {
                break;
            };

            if let Some(image) = self.take_valid_preloaded_navigation_image(&path) {
                self.replace_loaded_navigation_image(image);
                return Ok(NavigationOutcome::Moved);
            }

            match self.load_navigation_target(&path) {
                Ok(image) => {
                    self.replace_loaded_navigation_image(image);
                    return Ok(NavigationOutcome::Moved);
                }
                Err(error) => {
                    last_error = Some(error);
                    if !navigation_settings.auto_skip_failed_navigation() {
                        break;
                    }
                }
            }
        }

        if let Some(error) = last_error {
            self.status_message = Some(navigation_failure_status_text(
                &error,
                navigation_settings,
                self.config.ui_language(),
            ));
        }
        Ok(NavigationOutcome::Noop)
    }

    fn load_navigation_target(&self, path: &Path) -> Result<LoadedImage, ViewerAppError> {
        let image = load_image_file_for_view_with_timing(
            path,
            self.viewport,
            self.memory_policy,
            self.config.animation_timing_settings(),
            None,
        )?;
        Ok(image)
    }

    fn take_valid_preloaded_navigation_image(&mut self, path: &Path) -> Option<LoadedImage> {
        let position = self
            .navigation_preload_cache
            .iter()
            .position(|entry| entry.image.metadata().path() == path)?;
        let entry = self.navigation_preload_cache.swap_remove(position);
        if entry.viewport != self.viewport
            || entry.memory_policy != self.memory_policy
            || entry.animation_timing != self.config.animation_timing_settings()
        {
            return None;
        }
        match loaded_image_file_version_matches_current(&entry.image) {
            Ok(true) => Some(entry.image),
            Ok(false) | Err(_) => None,
        }
    }

    fn has_current_navigation_preload(&self, path: &Path) -> bool {
        self.navigation_preload_cache.iter().any(|entry| {
            entry.image.metadata().path() == path
                && entry.viewport == self.viewport
                && entry.memory_policy == self.memory_policy
                && entry.animation_timing == self.config.animation_timing_settings()
        })
    }

    fn is_navigation_preload_target(&self, path: &Path) -> bool {
        let navigation_settings = self.config.navigation_settings();
        [
            ImageNavigationDirection::Previous,
            ImageNavigationDirection::Next,
        ]
        .into_iter()
        .filter_map(|direction| {
            self.image_folder
                .navigation_path_for_attempt(direction, navigation_settings, 0)
        })
        .any(|target| target == path)
    }

    fn clear_navigation_preloads(&mut self) {
        self.navigation_preload_cache.clear();
    }

    fn evict_navigation_preload(&mut self, path_key: u64) {
        self.navigation_preload_cache
            .retain(|entry| entry.path_key != path_key);
    }

    fn replace_loaded_image(&mut self, image: LoadedImage, image_folder: ImageFolder) {
        self.image_folder = image_folder;
        self.replace_loaded_image_state(image);
    }

    fn replace_loaded_navigation_image(&mut self, image: LoadedImage) {
        if self
            .image_folder
            .retarget_current_path(image.metadata().path())
        {
            self.replace_loaded_image_state(image);
        } else {
            let (image_folder, _) = scan_image_folder_for_file_or_empty(image.metadata().path());
            self.replace_loaded_image(image, image_folder);
        }
    }

    fn replace_loaded_image_state(&mut self, mut image: LoadedImage) {
        self.apply_animation_autoplay_preference(&mut image);
        self.config
            .set_recent_folder(parent_folder(image.metadata().path()));
        self.decode.clear();
        self.image_state = Arc::new(ImageState::Loaded(image));
        self.image_revision = self.image_revision.wrapping_add(1);
        self.user_rotation = ImageRotation::ZERO;
        self.clear_status_message();
        self.invalidate_image_caches();
        self.defer_next_paint_scaling_cache_rebuild = true;
        self.invalidate_animation_frame_cache();
        self.clear_navigation_preloads();
        self.pending_animation_frame = None;
        self.pending_animation_file_version = None;
        self.pending_animation_state = None;
        self.pending_folder_scan = None;
        self.pending_navigation_after_folder_scan = None;
        self.view_transform = self.config.default_view_transform();
        self.panning = None;
        self.enforce_memory_policy();
        self.update_title();
    }

    pub fn handle_create(&mut self) {}

    pub fn handle_resize(&mut self, width: i32, height: i32) {
        let old_viewport = self.viewport;
        self.viewport = ViewportSize::from_client_size(width, height);
        if self.viewport != old_viewport {
            self.invalidate_scaled_image_cache();
            self.clear_navigation_preloads();
        }
        if let Some(image_size) = self.current_view_image_size() {
            self.view_transform = self.view_transform.resize_viewport_with_settings(
                old_viewport,
                self.viewport,
                image_size,
                self.config.zoom_settings(),
            );
        }
    }

    pub fn handle_paint(&mut self) {
        self.paint_count = self.paint_count.saturating_add(1);
    }

    pub fn handle_destroy(&mut self) {}

    pub(crate) fn defer_scaling_cache_rebuilds(&mut self) {
        self.defer_scaling_cache_rebuild = true;
    }

    pub(crate) fn resume_scaling_cache_rebuilds(&mut self) -> bool {
        self.defer_scaling_cache_rebuild = false;
        let repaint_needed = self.scaling_cache_rebuild_deferred;
        self.scaling_cache_rebuild_deferred = false;
        repaint_needed
    }

    pub(crate) fn cancel_deferred_scaling_cache_rebuild(&mut self) {
        self.defer_scaling_cache_rebuild = false;
        self.scaling_cache_rebuild_deferred = false;
    }

    pub(crate) fn has_deferred_scaling_cache_rebuild(&self) -> bool {
        self.scaling_cache_rebuild_deferred
    }

    pub fn begin_pan(&mut self, point: ViewportPoint) -> bool {
        let Some(image_size) = self.current_view_image_size() else {
            self.panning = None;
            return false;
        };
        let Some(start_offset) = self.view_transform.panning_start_offset_with_settings(
            self.viewport,
            image_size,
            self.config.zoom_settings(),
        ) else {
            self.panning = None;
            return false;
        };

        self.panning = Some(PanningState {
            start_point: point,
            start_offset,
        });
        true
    }

    pub fn update_pan(&mut self, point: ViewportPoint) -> bool {
        let Some(panning) = self.panning else {
            return false;
        };
        let Some(image_size) = self.current_view_image_size() else {
            self.panning = None;
            return false;
        };
        if !self.view_transform.can_pan_with_settings(
            self.viewport,
            image_size,
            self.config.zoom_settings(),
        ) {
            self.panning = None;
            return false;
        }

        let next_offset = ViewOffset::new(
            panning.start_offset.x() + point.x() - panning.start_point.x(),
            panning.start_offset.y() + point.y() - panning.start_point.y(),
        );
        let next_transform = self.view_transform.pan_to_offset_with_settings(
            self.viewport,
            image_size,
            next_offset,
            self.config.zoom_settings(),
        );
        let changed = next_transform != self.view_transform;
        self.view_transform = next_transform;
        changed
    }

    pub fn end_pan(&mut self) -> bool {
        self.panning.take().is_some()
    }

    pub fn zoom_at(&mut self, factor: f64, anchor: ViewportPoint) -> bool {
        let Some(image_size) = self.current_view_image_size() else {
            return false;
        };

        let next_transform = self.view_transform.zoom_at_with_settings(
            self.viewport,
            image_size,
            factor,
            anchor,
            self.config.zoom_settings(),
        );
        let changed = next_transform != self.view_transform || self.panning.is_some();
        self.view_transform = next_transform;
        self.panning = None;
        changed
    }

    pub fn zoom_from_center(&mut self, factor: f64) -> bool {
        let Some(anchor) = ViewportPoint::center(self.viewport) else {
            return false;
        };

        self.zoom_at(factor, anchor)
    }

    pub fn show_actual_size(&mut self) -> bool {
        if self.current_view_image_size().is_none() {
            return false;
        }

        self.config.set_default_view_mode(ViewMode::ActualSize);
        self.view_transform = ViewTransform::ACTUAL_SIZE;
        self.panning = None;
        true
    }

    pub fn fit_to_window(&mut self) -> bool {
        if self.current_view_image_size().is_none() {
            return false;
        }

        self.config.set_default_view_mode(ViewMode::FitToWindow);
        self.view_transform = ViewTransform::FIT_TO_WINDOW;
        self.panning = None;
        true
    }

    pub fn rotate_clockwise(&mut self) -> bool {
        self.rotate_to(self.user_rotation.clockwise())
    }

    pub fn rotate_counter_clockwise(&mut self) -> bool {
        self.rotate_to(self.user_rotation.counter_clockwise())
    }

    pub fn display_rgba8(&mut self) -> Option<&Rgba8Image> {
        self.display_pixels()?.as_rgba8()
    }

    pub fn display_pixels(&mut self) -> Option<&PixelImage> {
        self.prepare_display_pixels()?;
        self.display_pixels_ref()
    }

    pub fn display_pixel_source(&self) -> Option<DisplayPixelSource<'_>> {
        let orientation = self.current_display_orientation()?;
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };
        if orientation.is_identity() {
            return Some(DisplayPixelSource::new(
                image.pixels(),
                ImageOrientation::NORMAL,
            ));
        }

        let source_size = image.pixels().size();
        self.render_cache
            .oriented_image(self.image_revision, source_size, orientation)
            .map(|pixels| DisplayPixelSource::new(pixels, ImageOrientation::NORMAL))
            .or_else(|| Some(DisplayPixelSource::new(image.pixels(), orientation)))
    }

    pub fn begin_current_image_export(
        &mut self,
        path: impl AsRef<Path>,
        options: ExportOptions,
    ) -> Result<ImageExportRequest, ViewerAppError> {
        let path = path.as_ref();
        let orientation = self
            .current_display_orientation()
            .ok_or(ViewerAppError::NoImageToExport)?;
        let image_state = Arc::clone(&self.image_state);
        let ImageState::Loaded(image) = image_state.as_ref() else {
            return Err(ViewerAppError::NoImageToExport);
        };
        let options = options.with_jpeg_alpha_background_rgb(
            self.config.export_settings().jpeg_alpha_background_rgb(),
        );
        let source_size = image.pixels().size();

        let cached_oriented =
            self.render_cache
                .shared_oriented_image(self.image_revision, source_size, orientation);
        let (source, orientation) = cached_oriented.map_or_else(
            || {
                (
                    ImageExportSource::SharedImageState(image_state),
                    orientation,
                )
            },
            |pixels| {
                (
                    ImageExportSource::SharedPixels(pixels),
                    ImageOrientation::NORMAL,
                )
            },
        );

        Ok(ImageExportRequest::new(
            path.to_path_buf(),
            source,
            orientation,
            options,
        ))
    }

    pub fn finish_current_image_export(&mut self, path: &Path, options: ExportOptions) {
        self.status_message = Some(export_success_status_text(
            path,
            options,
            self.config.ui_language(),
        ));
    }

    pub fn current_source_path(&self) -> Option<&Path> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        Some(image.metadata().path())
    }

    pub fn current_source_format(&self) -> Option<SupportedImageFormat> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        Some(image.metadata().format())
    }

    pub fn current_export_source_size(&self) -> Option<ImageSize> {
        let orientation = self.current_display_orientation()?;
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        Some(image.pixels().size().with_orientation(orientation))
    }

    pub fn export_current_image(
        &mut self,
        path: impl AsRef<Path>,
        options: ExportOptions,
    ) -> Result<(), ViewerAppError> {
        let request = self.begin_current_image_export(path, options)?;
        let path = request.path().to_path_buf();
        let options = request.options();

        request.export()?;
        self.finish_current_image_export(&path, options);
        Ok(())
    }

    pub fn render_rgba8(&mut self, viewport: ViewportSize) -> Option<RenderImage<'_>> {
        let defer_scaling_cache_rebuild = self.defer_scaling_cache_rebuild;
        self.render_rgba8_with_scaling_defer(viewport, defer_scaling_cache_rebuild)
    }

    pub fn prepare_first_render(&mut self, viewport: ViewportSize) -> Option<RenderImage<'_>> {
        self.defer_next_paint_scaling_cache_rebuild = false;
        self.render_rgba8_with_scaling_defer(viewport, true)
    }

    pub(crate) fn render_rgba8_for_paint(
        &mut self,
        viewport: ViewportSize,
    ) -> Option<RenderImage<'_>> {
        let defer_next_paint = self.defer_next_paint_scaling_cache_rebuild;
        self.defer_next_paint_scaling_cache_rebuild = false;
        self.render_rgba8_with_scaling_defer(
            viewport,
            self.defer_scaling_cache_rebuild
                || defer_next_paint
                || self.scaling_cache_rebuild_deferred,
        )
    }

    fn render_rgba8_with_scaling_defer(
        &mut self,
        viewport: ViewportSize,
        defer_scaling_cache_rebuild: bool,
    ) -> Option<RenderImage<'_>> {
        let logical_image_size = self.current_view_image_size()?;
        let orientation = self.current_display_orientation()?;
        let zoom_settings = self.config.zoom_settings();
        let rect = self.view_transform.display_rect_with_settings(
            viewport,
            logical_image_size,
            zoom_settings,
        )?;
        let display_size = rect.size()?;
        let effective_scale = self.view_transform.effective_scale_with_settings(
            viewport,
            logical_image_size,
            zoom_settings,
        )?;
        let quality = scaling_quality_for_render(self.config.scaling_quality(), effective_scale);

        self.prepare_display_pixels()?;
        let display_pixels = self.display_pixels_ref()?;
        let buffer_image_size = display_pixels.size();
        let buffer_pixel_format = display_pixels.pixel_format();
        let mut deferred_scaling_cache_rebuild = false;

        if let Some(key) = scaling_cache_key_for_render(
            self.image_revision,
            orientation,
            buffer_image_size,
            display_size,
            quality,
        ) {
            let cached_key = self.render_cache.scaled_key();
            if should_rebuild_scaling_cache(cached_key, key) {
                if self.should_defer_scaling_cache_rebuild(
                    key.target_size(),
                    buffer_pixel_format,
                    defer_scaling_cache_rebuild,
                ) {
                    self.scaling_cache_rebuild_deferred = true;
                    deferred_scaling_cache_rebuild = true;
                } else if let Some(scaled) =
                    self.resample_display_pixels(key.target_size(), key.quality())
                {
                    self.render_cache
                        .store_scaled_image(key, scaled, self.paint_count);
                    self.scaling_cache_rebuild_deferred = false;
                    self.enforce_memory_policy();
                } else {
                    self.invalidate_scaled_image_cache();
                    self.scaling_cache_rebuild_deferred = false;
                }
            }

            if self.render_cache.scaled_image(key).is_some() {
                self.scaling_cache_rebuild_deferred = false;
                let cache = self.render_cache.scaled_image(key)?;
                return Some(RenderImage {
                    rect,
                    pixels: &cache.pixels,
                    scaling_quality: quality,
                    cache_key: RenderImageCacheKey::new(
                        self.image_revision,
                        orientation,
                        cache.pixels.size(),
                    ),
                });
            }
        }

        self.invalidate_scaled_image_cache();
        if !deferred_scaling_cache_rebuild {
            self.scaling_cache_rebuild_deferred = false;
        }
        let pixels = self.display_pixels_ref()?;
        Some(RenderImage {
            rect,
            pixels,
            scaling_quality: quality,
            cache_key: RenderImageCacheKey::new(self.image_revision, orientation, pixels.size()),
        })
    }

    fn prepare_display_pixels(&mut self) -> Option<()> {
        if !self.image_state.has_image() {
            self.invalidate_image_caches();
            return None;
        }

        let orientation = self.current_display_orientation()?;
        if orientation.is_identity() {
            self.render_cache.clear_oriented();
            return Some(());
        }

        let source_size = match self.image_state.as_ref() {
            ImageState::Empty => return None,
            ImageState::Loaded(image) => image.pixels().size(),
        };
        let needs_cache = self.render_cache.oriented_image_needs_rebuild(
            self.image_revision,
            source_size,
            orientation,
        );
        if needs_cache {
            let oriented = match self.image_state.as_ref() {
                ImageState::Empty => return None,
                ImageState::Loaded(image) => orient_pixel_image(image.pixels(), orientation)?,
            };
            self.render_cache.store_oriented_image(
                self.image_revision,
                source_size,
                orientation,
                oriented,
                self.paint_count,
            );
            self.enforce_memory_policy();
        }

        Some(())
    }

    fn display_pixels_ref(&self) -> Option<&PixelImage> {
        let orientation = self.current_display_orientation()?;
        if orientation.is_identity() {
            return match self.image_state.as_ref() {
                ImageState::Empty => None,
                ImageState::Loaded(image) => Some(image.pixels()),
            };
        }

        let source_size = match self.image_state.as_ref() {
            ImageState::Empty => return None,
            ImageState::Loaded(image) => image.pixels().size(),
        };

        self.render_cache
            .oriented_image(self.image_revision, source_size, orientation)
    }

    fn rotate_to(&mut self, rotation: ImageRotation) -> bool {
        if !self.image_state.has_image() || rotation == self.user_rotation {
            return false;
        }

        self.user_rotation = rotation;
        self.invalidate_image_caches();
        if let Some(image_size) = self.current_view_image_size() {
            self.view_transform = if self.view_transform.mode() == ViewMode::FitToWindow {
                ViewTransform::FIT_TO_WINDOW
            } else {
                self.view_transform.constrain_to_viewport_with_settings(
                    self.viewport,
                    image_size,
                    self.config.zoom_settings(),
                )
            };
        }
        self.panning = None;
        self.update_title();
        true
    }

    fn apply_animation_transition(
        &mut self,
        transition: AnimationPlaybackTransition,
    ) -> AnimationFrameOutcome {
        let (playback, frame_index) = transition.into_parts();
        let Some(frame_index) = frame_index else {
            return if self.set_current_animation_playback(playback) {
                AnimationFrameOutcome::StateChanged
            } else {
                AnimationFrameOutcome::Unchanged
            };
        };

        if let Some(frame) = self.take_cached_animation_frame(frame_index) {
            if self.apply_animation_frame_image(frame_index, frame, playback) {
                return AnimationFrameOutcome::Updated;
            }
            return AnimationFrameOutcome::Unchanged;
        }

        let Some(request) = self.animation_frame_decode_request(frame_index) else {
            return AnimationFrameOutcome::Unchanged;
        };
        self.pending_animation_frame = Some(frame_index);
        self.pending_animation_file_version = Some(request.file_version());
        self.pending_animation_state = Some(playback);
        AnimationFrameOutcome::NeedsDecode(request)
    }

    fn animation_frame_decode_request(
        &self,
        frame_index: usize,
    ) -> Option<AnimationFrameDecodeRequest> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };
        if !image.is_animated() {
            return None;
        }
        let file_version = image.metadata().file_version()?;

        Some(AnimationFrameDecodeRequest::new(
            self.decode.generation(),
            image.metadata().path().to_path_buf(),
            file_version,
            image.metadata().format(),
            image.source_size(),
            frame_index,
            self.viewport,
            self.memory_policy,
        ))
    }

    fn apply_animation_frame_image(
        &mut self,
        frame_index: usize,
        frame: AnimationFramePixels,
        playback: AnimationPlayback,
    ) -> bool {
        let old_frame = self
            .current_animation_playback()
            .map(AnimationPlayback::current_frame_index);
        let cache_frame = {
            let ImageState::Loaded(image) = self.image_state_mut() else {
                return false;
            };
            if !image.is_animated() {
                return false;
            }
            let rgba8 = frame.into_rgba8();
            let Some(old_rgba8) = image.replace_current_rgba8(rgba8) else {
                return false;
            };
            if !image.set_animation_playback(playback) {
                let _ = image.replace_current_rgba8(old_rgba8);
                return false;
            }

            old_frame
                .filter(|old_frame| *old_frame != frame_index)
                .map(|old_frame| (old_frame, old_rgba8))
        };

        if let Some((old_frame, old_rgba8)) = cache_frame {
            self.store_animation_frame_cache(old_frame, old_rgba8);
        }

        self.image_revision = self.image_revision.wrapping_add(1);
        self.invalidate_image_caches();
        self.defer_next_paint_scaling_cache_rebuild = true;
        self.enforce_memory_policy();
        true
    }

    fn set_current_animation_playback(&mut self, playback: AnimationPlayback) -> bool {
        let ImageState::Loaded(image) = self.image_state_mut() else {
            return false;
        };

        image.set_animation_playback(playback)
    }

    fn apply_animation_autoplay_preference(&self, image: &mut LoadedImage) {
        let Some(playback) = image.animation().cloned() else {
            return;
        };
        let playback = playback.with_autoplay(self.config.animation_autoplay());
        let _ = image.set_animation_playback(playback);
    }

    fn apply_animation_config_to_current_image(&mut self, apply_autoplay: bool) {
        self.discard_pending_animation_frame_transition();
        let Some(playback) = self.current_animation_playback().cloned() else {
            return;
        };

        let mut playback = playback.with_timing_settings(self.config.animation_timing_settings());
        if apply_autoplay {
            playback = playback.with_autoplay(self.config.animation_autoplay());
        }
        let _ = self.set_current_animation_playback(playback);
    }

    fn pause_current_animation_after_frame_decode_failure(&mut self) {
        let Some(playback) = self.current_animation_playback().cloned() else {
            return;
        };
        if playback.playback_state() != AnimationPlaybackState::Playing {
            return;
        }

        let transition = animation_state_after_toggle(&playback);
        let _ = self.set_current_animation_playback(transition.state().clone());
    }

    fn current_animation_playback(&self) -> Option<&AnimationPlayback> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        image.animation()
    }

    fn take_cached_animation_frame(&mut self, frame_index: usize) -> Option<AnimationFramePixels> {
        let position = self
            .animation_frame_cache
            .iter()
            .position(|entry| entry.frame_index == frame_index)?;
        Some(AnimationFramePixels::Shared(
            self.animation_frame_cache.swap_remove(position).rgba8,
        ))
    }

    fn store_animation_frame_cache(&mut self, frame_index: usize, rgba8: Rgba8Image) {
        if rgba8.byte_len() > self.memory_policy.max_cache_entry_bytes() {
            return;
        }

        let rgba8 = Arc::new(rgba8);
        self.animation_frame_cache
            .retain(|entry| entry.frame_index != frame_index);
        self.animation_frame_cache.push(AnimationFrameCacheEntry {
            frame_index,
            rgba8,
            last_used: self.paint_count,
        });
    }

    fn resample_display_pixels(
        &self,
        target_size: ImageSize,
        quality: ScalingQuality,
    ) -> Option<PixelImage> {
        let source = self.display_pixels_ref()?;
        if !self.scaled_cache_entry_fits(target_size, source.pixel_format()) {
            return None;
        }

        let filter = filter_type_for_quality(quality);
        match source {
            PixelImage::Rgb8(source) => {
                let source =
                    BorrowedRgb8Image::new(source.width(), source.height(), source.pixels())?;
                let resized = resize(&source, target_size.width(), target_size.height(), filter);
                Some(PixelImage::from(Rgb8Image::new(
                    target_size.width(),
                    target_size.height(),
                    resized.into_raw(),
                )))
            }
            PixelImage::Rgba8(source) => {
                let source =
                    BorrowedRgba8Image::new(source.width(), source.height(), source.pixels())?;
                let resized = resize(&source, target_size.width(), target_size.height(), filter);
                Some(PixelImage::from(Rgba8Image::new(
                    target_size.width(),
                    target_size.height(),
                    resized.into_raw(),
                )))
            }
            PixelImage::Bgra8(_) => {
                let source = source.to_rgba8()?;
                let source =
                    BorrowedRgba8Image::new(source.width(), source.height(), source.pixels())?;
                let resized = resize(&source, target_size.width(), target_size.height(), filter);
                Some(PixelImage::from(Rgba8Image::new(
                    target_size.width(),
                    target_size.height(),
                    resized.into_raw(),
                )))
            }
        }
    }

    fn should_defer_scaling_cache_rebuild(
        &self,
        target_size: ImageSize,
        pixel_format: crate::domain::PixelFormat,
        defer_scaling_cache_rebuild: bool,
    ) -> bool {
        defer_scaling_cache_rebuild && self.scaled_cache_entry_fits(target_size, pixel_format)
    }

    fn scaled_cache_entry_fits(
        &self,
        target_size: ImageSize,
        pixel_format: crate::domain::PixelFormat,
    ) -> bool {
        target_size
            .pixel_byte_len(pixel_format)
            .is_some_and(|bytes| bytes <= self.memory_policy.max_cache_entry_bytes())
    }

    fn invalidate_image_caches(&mut self) {
        self.render_cache.clear_all();
    }

    fn invalidate_scaled_image_cache(&mut self) {
        self.render_cache.clear_scaled();
    }

    fn invalidate_animation_frame_cache(&mut self) {
        self.animation_frame_cache.clear();
    }

    fn evict_animation_frame_cache(&mut self, frame_index: Option<usize>) {
        if let Some(frame_index) = frame_index {
            if frame_index == RESIDENT_ANIMATION_FRAME_CACHE_SLOT_INDEX {
                clear_animation_frame_resident_cache();
            } else {
                self.animation_frame_cache
                    .retain(|entry| entry.frame_index != frame_index);
            }
        } else {
            self.invalidate_animation_frame_cache();
            clear_animation_frame_resident_cache();
        }
    }

    fn discard_pending_animation_frame_transition(&mut self) {
        self.pending_animation_frame = None;
        self.pending_animation_file_version = None;
        self.pending_animation_state = None;
    }

    fn clear_status_message(&mut self) {
        self.status_message = None;
    }

    fn enforce_memory_policy(&mut self) {
        let protected_oriented_image = self.protected_oriented_image_cache_key();
        let protected_oriented_image_bytes = protected_oriented_image
            .and_then(|key| self.render_cache.oriented_image_by_key(key))
            .map(PixelImage::byte_len)
            .unwrap_or(0);
        let base_bytes = self
            .current_image_byte_len()
            .saturating_add(protected_oriented_image_bytes);
        let mut entries = Vec::new();
        self.render_cache
            .append_memory_entries(&mut entries, protected_oriented_image);
        let resident_animation_cache_bytes = animation_frame_resident_cache_byte_len();
        if resident_animation_cache_bytes > 0 {
            entries.push(MemoryCacheEntry::new(
                ImageCacheSlot::AnimationFrame {
                    frame_index: Some(RESIDENT_ANIMATION_FRAME_CACHE_SLOT_INDEX),
                },
                resident_animation_cache_bytes,
                0,
            ));
        }
        for cache in &self.animation_frame_cache {
            entries.push(MemoryCacheEntry::new(
                ImageCacheSlot::AnimationFrame {
                    frame_index: Some(cache.frame_index),
                },
                cache.rgba8.byte_len(),
                cache.last_used,
            ));
        }
        for cache in &self.navigation_preload_cache {
            entries.push(MemoryCacheEntry::new(
                ImageCacheSlot::NavigationPreload {
                    path_key: cache.path_key,
                },
                cache.image.resident_byte_len(),
                cache.last_used,
            ));
        }

        for slot in memory_cache_slots_to_evict(base_bytes, &entries, self.memory_policy) {
            match slot {
                ImageCacheSlot::OrientedImage | ImageCacheSlot::ScaledImage => {
                    self.render_cache.evict(slot)
                }
                ImageCacheSlot::AnimationFrame { frame_index } => {
                    self.evict_animation_frame_cache(frame_index)
                }
                ImageCacheSlot::NavigationPreload { path_key } => {
                    self.evict_navigation_preload(path_key)
                }
            }
        }
    }

    fn protected_oriented_image_cache_key(&self) -> Option<OrientedImageCacheKey> {
        let orientation = self.current_display_orientation()?;
        if orientation.is_identity() {
            return None;
        }

        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };
        let source_size = image.pixels().size();
        Some(OrientedImageCacheKey::new(
            self.image_revision,
            source_size,
            orientation,
        ))
    }

    fn current_image_byte_len(&self) -> usize {
        match self.image_state.as_ref() {
            ImageState::Empty => 0,
            ImageState::Loaded(image) => image.resident_byte_len(),
        }
    }

    fn current_display_orientation(&self) -> Option<ImageOrientation> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        Some(display_orientation(
            image.metadata().exif_orientation(),
            self.user_rotation,
        ))
    }

    fn current_view_image_size(&self) -> Option<ImageSize> {
        let ImageState::Loaded(image) = self.image_state.as_ref() else {
            return None;
        };

        Some(
            image
                .source_size()
                .with_orientation(self.current_display_orientation()?),
        )
    }

    fn update_title(&mut self) {
        self.title = match self.image_state.as_ref() {
            ImageState::Empty => DEFAULT_TITLE.to_owned(),
            ImageState::Loaded(image) => image_title(image, self.user_rotation),
        };
    }
}

impl Default for ViewerApp {
    fn default() -> Self {
        Self::new()
    }
}

fn command_outcome(changed: bool) -> AppCommandOutcome {
    if changed {
        AppCommandOutcome::Changed
    } else {
        AppCommandOutcome::Unchanged
    }
}

fn resize_export_pixels(
    path: &Path,
    pixels: PixelImage,
    options: ExportOptions,
) -> Result<PixelImage, ViewerAppError> {
    let Some(target_size) = options.target_size() else {
        return Ok(pixels);
    };
    if target_size == pixels.size() {
        return Ok(pixels);
    }

    let filter = FilterType::Lanczos3;
    match pixels {
        PixelImage::Rgb8(source) => resize_rgb8_export_pixels(path, &source, target_size, filter),
        PixelImage::Rgba8(source) => resize_rgba8_export_pixels(path, &source, target_size, filter),
        PixelImage::Bgra8(source) => {
            let size = source.size();
            let actual_len = source.byte_len();
            let rgba8 = PixelImage::from(source)
                .into_rgba8()
                .ok_or_else(|| invalid_export_pixels(path, size, actual_len))?;
            resize_export_pixels(
                path,
                PixelImage::from(rgba8),
                options.with_target_size(Some(target_size)),
            )
        }
    }
}

enum ExportPixels<'a> {
    Borrowed(&'a PixelImage),
    Owned(PixelImage),
}

fn export_pixels(
    path: &Path,
    pixels: ExportPixels<'_>,
    options: ExportOptions,
) -> Result<(), ViewerAppError> {
    match pixels {
        ExportPixels::Borrowed(pixels) => export_borrowed_pixel_image(path, pixels, options)?,
        ExportPixels::Owned(pixels) => export_owned_pixel_image(path, pixels, options)?,
    }
    Ok(())
}

fn resize_borrowed_export_pixels<'a>(
    path: &Path,
    pixels: &'a PixelImage,
    options: ExportOptions,
) -> Result<ExportPixels<'a>, ViewerAppError> {
    let Some(target_size) = options.target_size() else {
        return Ok(ExportPixels::Borrowed(pixels));
    };
    if target_size == pixels.size() {
        return Ok(ExportPixels::Borrowed(pixels));
    }

    let filter = FilterType::Lanczos3;
    let pixels = match pixels {
        PixelImage::Rgb8(source) => resize_rgb8_export_pixels(path, source, target_size, filter)?,
        PixelImage::Rgba8(source) => resize_rgba8_export_pixels(path, source, target_size, filter)?,
        PixelImage::Bgra8(_) => resize_export_pixels(path, pixels.clone(), options)?,
    };
    Ok(ExportPixels::Owned(pixels))
}

fn resize_rgb8_export_pixels(
    path: &Path,
    source: &Rgb8Image,
    target_size: ImageSize,
    filter: FilterType,
) -> Result<PixelImage, ViewerAppError> {
    ensure_export_target_allocation(path, target_size, crate::domain::PixelFormat::Rgb8)?;
    let source_view = BorrowedRgb8Image::new(source.width(), source.height(), source.pixels())
        .ok_or_else(|| invalid_export_pixels(path, source.size(), source.byte_len()))?;
    let resized = resize(
        &source_view,
        target_size.width(),
        target_size.height(),
        filter,
    );
    Ok(PixelImage::from(Rgb8Image::new(
        target_size.width(),
        target_size.height(),
        resized.into_raw(),
    )))
}

fn resize_rgba8_export_pixels(
    path: &Path,
    source: &Rgba8Image,
    target_size: ImageSize,
    filter: FilterType,
) -> Result<PixelImage, ViewerAppError> {
    ensure_export_target_allocation(path, target_size, crate::domain::PixelFormat::Rgba8)?;
    let source_view = BorrowedRgba8Image::new(source.width(), source.height(), source.pixels())
        .ok_or_else(|| invalid_export_pixels(path, source.size(), source.byte_len()))?;
    let resized = resize(
        &source_view,
        target_size.width(),
        target_size.height(),
        filter,
    );
    Ok(PixelImage::from(Rgba8Image::new(
        target_size.width(),
        target_size.height(),
        resized.into_raw(),
    )))
}

fn ensure_export_target_allocation(
    path: &Path,
    target_size: ImageSize,
    pixel_format: crate::domain::PixelFormat,
) -> Result<(), ViewerAppError> {
    target_size
        .pixel_byte_len(pixel_format)
        .ok_or_else(|| {
            ViewerAppError::from(ExportImageError::AllocationFailed {
                path: path.to_path_buf(),
            })
        })
        .map(|_| ())
}

fn invalid_export_pixels(path: &Path, size: ImageSize, actual_len: usize) -> ViewerAppError {
    ViewerAppError::from(ExportImageError::InvalidPixelBuffer {
        path: path.to_path_buf(),
        size,
        actual_len,
    })
}

struct BorrowedRgba8Image<'a> {
    width: u32,
    height: u32,
    row_stride: usize,
    pixels: &'a [u8],
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

fn image_title(image: &LoadedImage, user_rotation: ImageRotation) -> String {
    let file_name = image.metadata().file_name_for_display();

    match (
        image.metadata().exif_orientation().is_identity(),
        user_rotation.is_identity(),
    ) {
        (true, true) => format!("{file_name} - {DEFAULT_TITLE}"),
        (false, true) => format!(
            "{file_name} (EXIF {}) - {DEFAULT_TITLE}",
            image.metadata().exif_orientation().exif_value()
        ),
        (true, false) => format!(
            "{file_name} (rotation {} deg) - {DEFAULT_TITLE}",
            user_rotation.degrees()
        ),
        (false, false) => format!(
            "{file_name} (EXIF {} + rotation {} deg) - {DEFAULT_TITLE}",
            image.metadata().exif_orientation().exif_value(),
            user_rotation.degrees()
        ),
    }
}

fn filter_type_for_quality(quality: ScalingQuality) -> FilterType {
    match quality {
        ScalingQuality::Nearest => FilterType::Nearest,
        ScalingQuality::Balanced => FilterType::Triangle,
        ScalingQuality::HighQuality => FilterType::Lanczos3,
    }
}

fn export_success_status_text(path: &Path, options: ExportOptions, language: UiLanguage) -> String {
    let file_name = path
        .file_name()
        .and_then(|name| name.to_str())
        .map(str::to_owned)
        .unwrap_or_else(|| path.display().to_string());
    let format = export_format_display_name(options.format());
    let size_suffix = options
        .target_size()
        .map(|size| format!(" | {}x{}", size.width(), size.height()))
        .unwrap_or_default();

    match (language, options.quality()) {
        (UiLanguage::Korean, Some(quality)) => {
            format!("저장됨: {file_name} | {format} quality {quality}{size_suffix}")
        }
        (UiLanguage::Korean, None) => format!("저장됨: {file_name} | {format}{size_suffix}"),
        (UiLanguage::English, Some(quality)) => {
            format!("Saved: {file_name} | {format} quality {quality}{size_suffix}")
        }
        (UiLanguage::English, None) => format!("Saved: {file_name} | {format}{size_suffix}"),
    }
}

fn navigation_failure_status_text(
    error: &ViewerAppError,
    navigation_settings: NavigationSettings,
    language: UiLanguage,
) -> String {
    if navigation_settings.auto_skip_failed_navigation()
        && navigation_settings.max_navigation_attempts_per_command() > 1
    {
        match language {
            UiLanguage::Korean => format!(
                "이동할 이미지를 열 수 없습니다: {} 건너뛸 수 있는 이미지를 찾지 못해 현재 이미지를 유지합니다.",
                error.brief_user_message_for(language)
            ),
            UiLanguage::English => format!(
                "Could not open the image to navigate to: {} No skippable image was found, so the current image is kept.",
                error.brief_user_message_for(language)
            ),
        }
    } else {
        match language {
            UiLanguage::Korean => format!(
                "이동할 이미지를 열 수 없습니다: {} 현재 이미지를 유지합니다.",
                error.brief_user_message_for(language)
            ),
            UiLanguage::English => format!(
                "Could not open the image to navigate to: {} The current image is kept.",
                error.brief_user_message_for(language)
            ),
        }
    }
}

fn parent_folder(path: &Path) -> Option<PathBuf> {
    path.parent()
        .filter(|parent| !parent.as_os_str().is_empty())
        .map(Path::to_path_buf)
}

fn navigation_preload_path_key(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

#[cfg(test)]
mod tests {
    use std::fs;
    use std::io;
    use std::path::{Path, PathBuf};
    use std::time::{Duration, SystemTime, UNIX_EPOCH};

    use image::ImageError;

    use crate::domain::{
        rotate_rgba8_image, AnimationCommand, AnimationFrameStepDirection, AnimationLoopPolicy,
        AnimationPlayback, AnimationPlaybackState, AnimationTimingSettings, AppConfig, Command,
        DefaultExportFormatPolicy, ExportFormat, ExportOptions, ExportSettings, ImageBufferKind,
        ImageDisplayRect, ImageFileVersion, ImageFolder, ImageMetadata, ImageNavigationDirection,
        ImageOrientation, ImageRotation, ImageSize, ImageState, LoadedImage, MemoryPolicySettings,
        NavigationSettings, PixelImage, RenderReadyImage, Rgb8Image, RgbColor, Rgba8Image,
        ScalingQuality, StatusUiSettings, SupportedImageFormat, ViewMode, ViewOffset,
        ViewTransform, ViewportPoint, ViewportSize, ZoomSettings,
    };
    use crate::infra::{export_rgba8_image, load_image_file, LoadImageError};

    use super::{
        navigation_preload_path_key, AnimationFrameOutcome, AppCommandOutcome, DecodeApplyOutcome,
        DecodeFailurePresentation, ImageDecodePurpose, NavigationOutcome,
        NavigationPreloadCacheEntry, NavigationStartOutcome, ViewerApp, ViewerAppError,
    };

    #[test]
    fn rotation_updates_title_and_resets_when_image_is_replaced() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("first.png", 2, 1), ImageFolder::empty());

        assert!(app.rotate_clockwise());
        assert_eq!(app.rotation(), ImageRotation::Degrees90);
        assert_eq!(app.title(), "first.png (rotation 90 deg) - j3Pic");

        app.replace_loaded_image(test_image("second.png", 1, 2), ImageFolder::empty());

        assert_eq!(app.rotation(), ImageRotation::Degrees0);
        assert_eq!(app.title(), "second.png - j3Pic");
    }

    #[test]
    fn export_current_image_writes_display_oriented_pixels() {
        let dir = unique_temp_dir("export-display-orientation");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("oriented.png");
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image_with_exif("source.jpg", 3, 2, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );
        assert!(app.rotate_clockwise());
        let expected = app.display_rgba8().expect("display image").clone();

        app.export_current_image(&path, ExportOptions::new(ExportFormat::Png, None))
            .expect("export display image");

        let exported = load_image_file(&path).expect("reload exported image");
        assert_eq!(exported.metadata().format(), SupportedImageFormat::Png);
        assert_eq!(exported.pixels().size(), expected.size());
        assert_eq!(exported.pixels().pixels(), expected.pixels());
        assert!(app
            .image_info_text()
            .expect("export status")
            .contains("Saved"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_current_image_applies_export_rotation_after_display_orientation() {
        let dir = unique_temp_dir("export-extra-rotation");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("rotated.png");
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image_with_exif("source.jpg", 3, 2, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );
        let display = app.display_rgba8().expect("display image").clone();
        let expected =
            rotate_rgba8_image(&display, ImageRotation::Degrees90).expect("rotated display image");

        app.export_current_image(
            &path,
            ExportOptions::new(ExportFormat::Png, None).with_rotation(ImageRotation::Degrees90),
        )
        .expect("export additionally rotated image");

        let exported = load_image_file(&path).expect("reload exported image");
        assert_eq!(exported.pixels().size(), expected.size());
        assert_eq!(exported.pixels().pixels(), expected.pixels());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn export_current_image_resizes_display_oriented_pixels() {
        let dir = unique_temp_dir("export-resize");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("resized.png");
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image_with_exif("source.jpg", 4, 2, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );

        assert_eq!(app.current_export_source_size(), Some(ImageSize::new(2, 4)));
        app.export_current_image(
            &path,
            ExportOptions::new(ExportFormat::Png, None)
                .with_target_size(Some(ImageSize::new(1, 2))),
        )
        .expect("export resized image");

        let exported = load_image_file(&path).expect("reload exported image");
        assert_eq!(exported.pixels().size(), ImageSize::new(1, 2));
        assert!(app
            .image_info_text()
            .expect("export status")
            .contains("1x2"));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn borrowed_export_pixels_are_reused_when_resize_is_noop() {
        let pixels = PixelImage::from(test_rgba8_image(3, 2));
        let source_ptr = pixels.pixels().as_ptr();
        let path = Path::new("noop-export.png");

        let without_target = super::resize_borrowed_export_pixels(
            path,
            &pixels,
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect("prepare borrowed export pixels");
        let super::ExportPixels::Borrowed(actual) = without_target else {
            panic!("no target size should borrow source pixels");
        };
        assert_eq!(actual.pixels().as_ptr(), source_ptr);

        let same_size = super::resize_borrowed_export_pixels(
            path,
            &pixels,
            ExportOptions::new(ExportFormat::Png, None).with_target_size(Some(pixels.size())),
        )
        .expect("prepare borrowed export pixels");
        let super::ExportPixels::Borrowed(actual) = same_size else {
            panic!("same target size should borrow source pixels");
        };
        assert_eq!(actual.pixels().as_ptr(), source_ptr);
    }

    #[test]
    fn begin_export_shares_cached_display_orientation_when_available() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("cached-export.png", 3, 2), ImageFolder::empty());
        assert!(app.rotate_clockwise());

        let cached_pixels = app
            .display_rgba8()
            .expect("display image")
            .pixels()
            .as_ptr();
        let request = app
            .begin_current_image_export(
                Path::new("cached-export.png"),
                ExportOptions::new(ExportFormat::Png, None),
            )
            .expect("export request");

        assert_eq!(request.orientation, ImageOrientation::NORMAL);
        let shared_pixels = request.source.shared_pixels().expect("shared cache pixels");
        assert_eq!(shared_pixels.size(), ImageSize::new(2, 3));
        assert_eq!(shared_pixels.pixels().as_ptr(), cached_pixels);
        let retained_pixels = app
            .render_cache
            .oriented_image(
                app.image_revision,
                ImageSize::new(3, 2),
                ImageOrientation::Rotate90,
            )
            .expect("retained oriented cache");
        assert_eq!(retained_pixels.pixels().as_ptr(), cached_pixels);
    }

    #[test]
    fn begin_export_keeps_orientation_fallback_when_display_cache_is_missing() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image("uncached-export.png", 3, 2),
            ImageFolder::empty(),
        );
        assert!(app.rotate_clockwise());

        let request = app
            .begin_current_image_export(
                Path::new("uncached-export.png"),
                ExportOptions::new(ExportFormat::Png, None),
            )
            .expect("export request");

        assert_eq!(request.orientation, ImageOrientation::Rotate90);
        assert!(request.source.is_shared_image_state());
        assert_eq!(request.source.size(), Some(ImageSize::new(3, 2)));
    }

    #[test]
    fn display_pixel_source_keeps_orientation_without_building_display_cache() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("copy-source.png", 3, 2), ImageFolder::empty());
        assert!(app.rotate_clockwise());

        let source_pixels = match app.image_state() {
            ImageState::Loaded(image) => image.pixels().pixels().as_ptr(),
            ImageState::Empty => panic!("test image should be loaded"),
        };
        let source = app
            .display_pixel_source()
            .expect("display pixel source for clipboard");

        assert_eq!(source.orientation(), ImageOrientation::Rotate90);
        assert_eq!(source.pixels().size(), ImageSize::new(3, 2));
        assert_eq!(source.pixels().pixels().as_ptr(), source_pixels);
        assert!(!app.render_cache.has_oriented_image());
    }

    #[test]
    fn display_pixel_source_reuses_cached_display_orientation() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image("cached-copy-source.png", 3, 2),
            ImageFolder::empty(),
        );
        assert!(app.rotate_clockwise());

        let cached_pixels = app
            .display_pixels()
            .expect("cached display pixels")
            .pixels()
            .as_ptr();
        let source = app
            .display_pixel_source()
            .expect("display pixel source for clipboard");

        assert_eq!(source.orientation(), ImageOrientation::NORMAL);
        assert_eq!(source.pixels().size(), ImageSize::new(2, 3));
        assert_eq!(source.pixels().pixels().as_ptr(), cached_pixels);
    }

    #[test]
    fn app_export_settings_drive_default_format_suffix_and_jpeg_background() {
        let mut config = AppConfig::default();
        config.set_export_settings(ExportSettings::new(
            DefaultExportFormatPolicy::Jpeg,
            "_edited",
            RgbColor::new(10, 20, 30),
        ));
        let app = ViewerApp::with_config(config);

        assert_eq!(
            app.default_export_format_for_source(SupportedImageFormat::Png),
            ExportFormat::Jpeg
        );
        assert_eq!(
            app.suggested_export_path(Path::new("C:/images/photo.png"), SupportedImageFormat::Png),
            PathBuf::from("C:/images/photo_edited.jpg")
        );
        assert_eq!(
            app.export_options(ExportFormat::Jpeg, None)
                .jpeg_alpha_background_rgb(),
            RgbColor::new(10, 20, 30)
        );
    }

    #[test]
    fn app_export_settings_are_used_for_actual_jpeg_save() {
        let dir = unique_temp_dir("export-configured-jpeg");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("transparent.jpg");
        let mut config = AppConfig::default();
        config.set_export_default_quality(100);
        config.set_export_settings(ExportSettings::new(
            DefaultExportFormatPolicy::Jpeg,
            "_configured",
            RgbColor::new(12, 34, 56),
        ));
        let mut app = ViewerApp::with_config(config);
        let mut pixels = Vec::new();
        for _ in 0..64 {
            pixels.extend_from_slice(&[200, 0, 0, 0]);
        }
        app.replace_loaded_image(
            LoadedImage::new(
                Rgba8Image::new(8, 8, pixels),
                ImageMetadata::new(
                    PathBuf::from("transparent.png"),
                    0,
                    SupportedImageFormat::Png,
                ),
            ),
            ImageFolder::empty(),
        );

        let options = app.export_options(ExportFormat::Jpeg, Some(app.export_default_quality()));
        app.export_current_image(&path, options)
            .expect("export configured jpeg");
        let exported = load_image_file(&path).expect("reload configured jpeg");

        assert_eq!(exported.metadata().format(), SupportedImageFormat::Jpeg);
        assert_eq!(exported.source_size(), ImageSize::new(8, 8));
        for pixel in exported.pixels().pixels().chunks_exact(3) {
            assert_rgb_close(pixel, [12, 34, 56], 3);
        }

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn exif_orientation_is_kept_separate_from_user_rotation() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image_with_exif("exif.jpg", 3, 2, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );

        assert_eq!(app.user_rotation(), ImageRotation::Degrees0);
        assert_eq!(app.display_orientation(), Some(ImageOrientation::Rotate90));
        assert_eq!(app.title(), "exif.jpg (EXIF 6) - j3Pic");
        assert_eq!(
            app.image_info_text(),
            Some("exif.jpg | source 3x2 | display 2x3 | EXIF 6 | 0 B | JPEG | Fit".to_owned())
        );

        assert!(app.rotate_clockwise());

        assert_eq!(app.user_rotation(), ImageRotation::Degrees90);
        assert_eq!(app.display_orientation(), Some(ImageOrientation::Rotate180));
        assert_eq!(app.title(), "exif.jpg (EXIF 6 + rotation 90 deg) - j3Pic");
    }

    #[test]
    fn status_ui_settings_hide_or_simplify_status_text() {
        let mut hidden_config = AppConfig::default();
        let mut hidden_status = StatusUiSettings::default();
        hidden_status.set_show_status_bar(false);
        hidden_config.set_status_ui_settings(hidden_status);
        let mut hidden_app = ViewerApp::with_config(hidden_config);
        hidden_app.replace_loaded_image(test_image("hidden.png", 2, 1), ImageFolder::empty());

        assert_eq!(hidden_app.image_info_text(), None);

        let mut simple_config = AppConfig::default();
        let mut simple_status = StatusUiSettings::default();
        simple_status.set_detailed_status_text(false);
        simple_config.set_status_ui_settings(simple_status);
        let mut simple_app = ViewerApp::with_config(simple_config);
        simple_app.replace_loaded_image(test_image("simple.png", 2, 1), ImageFolder::empty());

        assert_eq!(
            simple_app.image_info_text(),
            Some("simple.png | Fit".to_owned())
        );
    }

    #[test]
    fn applying_config_updates_default_view_mode_for_subsequent_images() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("first.png", 100, 80), ImageFolder::empty());
        assert_eq!(app.view_transform().mode(), ViewMode::FitToWindow);

        let mut config = app.config_snapshot();
        config.set_default_view_mode(ViewMode::ActualSize);
        assert!(app.apply_config(config));
        app.replace_loaded_image(test_image("actual.png", 100, 80), ImageFolder::empty());

        assert_eq!(app.view_transform().mode(), ViewMode::ActualSize);

        let mut config = app.config_snapshot();
        config.set_default_view_mode(ViewMode::FitToWindow);
        assert!(app.apply_config(config));
        app.replace_loaded_image(test_image("fit.png", 100, 80), ImageFolder::empty());

        assert_eq!(app.view_transform().mode(), ViewMode::FitToWindow);
    }

    #[test]
    fn zoom_settings_drive_keyboard_zoom_and_scale_limits() {
        let mut config = AppConfig::default();
        config.set_zoom_settings(ZoomSettings::new(0.5, 2.0, 2.0));
        let mut app = ViewerApp::with_config(config);
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("zoom.png", 100, 100), ImageFolder::empty());

        app.handle_command(Command::ZoomIn).expect("zoom in");
        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        assert_approx_eq(app.view_transform().zoom_scale(), 2.0);

        app.handle_command(Command::ZoomIn).expect("zoom in clamp");
        assert_approx_eq(app.view_transform().zoom_scale(), 2.0);

        app.handle_command(Command::ZoomOut).expect("zoom out");
        app.handle_command(Command::ZoomOut)
            .expect("zoom out clamp");
        assert_approx_eq(app.view_transform().zoom_scale(), 1.0);
    }

    #[test]
    fn zoom_at_reports_unchanged_when_scale_is_already_clamped() {
        let mut config = AppConfig::default();
        config.set_zoom_settings(ZoomSettings::new(0.5, 2.0, 2.0));
        let mut app = ViewerApp::with_config(config);
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("zoom-clamp.png", 100, 100), ImageFolder::empty());

        let anchor = ViewportPoint::from_client_position(50, 50);
        assert!(app.zoom_at(2.0, anchor));
        assert!(!app.zoom_at(2.0, anchor));
    }

    #[test]
    fn applying_zoom_settings_drives_keyboard_and_wheel_equivalent_zoom() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("zoom-apply.png", 100, 100), ImageFolder::empty());
        let mut config = app.config_snapshot();
        config.set_zoom_settings(ZoomSettings::new(0.5, 2.0, 2.0));

        assert!(app.apply_config(config));
        app.handle_command(Command::ZoomIn)
            .expect("configured keyboard zoom in");
        assert_approx_eq(app.view_transform().zoom_scale(), 2.0);

        assert!(app.fit_to_window());
        let wheel_step = app.zoom_step_factor();
        assert_approx_eq(wheel_step, 2.0);
        assert!(app.zoom_at(wheel_step, ViewportPoint::from_client_position(50, 50)));
        assert_approx_eq(app.view_transform().zoom_scale(), 2.0);

        assert!(app.fit_to_window());
        assert!(!app.zoom_at(
            1.0 / wheel_step,
            ViewportPoint::from_client_position(50, 50)
        ));
        assert!(!app.zoom_at(
            1.0 / wheel_step,
            ViewportPoint::from_client_position(50, 50)
        ));
        assert_eq!(app.view_transform(), ViewTransform::FIT_TO_WINDOW);
    }

    #[test]
    fn replacing_image_resets_only_user_rotation_and_uses_new_exif_orientation() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            test_image_with_exif("first.jpg", 3, 2, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );
        assert!(app.rotate_clockwise());

        app.replace_loaded_image(
            test_image_with_exif("second.jpg", 3, 2, ImageOrientation::FlipHorizontal),
            ImageFolder::empty(),
        );

        assert_eq!(app.user_rotation(), ImageRotation::Degrees0);
        assert_eq!(
            app.display_orientation(),
            Some(ImageOrientation::FlipHorizontal)
        );
        assert_eq!(app.title(), "second.jpg (EXIF 2) - j3Pic");
    }

    #[test]
    fn manual_zoom_offset_is_clamped_after_rotation() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("wide.png", 300, 50), ImageFolder::empty());
        app.view_transform = ViewTransform::manual_zoom(1.0, ViewOffset::new(1000.0, 1000.0));

        assert!(app.rotate_clockwise());

        let offset = app.view_transform().offset();
        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        assert_approx_eq(offset.x(), 0.0);
        assert_approx_eq(offset.y(), 100.0);
    }

    #[test]
    fn pan_lifecycle_updates_offset_and_ends_cleanly() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("pan.png", 300, 300), ImageFolder::empty());
        assert!(app.show_actual_size());

        assert!(app.begin_pan(ViewportPoint::from_client_position(50, 50)));
        assert!(app.panning.is_some());
        assert!(app.update_pan(ViewportPoint::from_client_position(70, 80)));

        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        let offset = app.view_transform().offset();
        assert_approx_eq(offset.x(), 20.0);
        assert_approx_eq(offset.y(), 30.0);
        assert!(app.end_pan());
        assert!(!app.end_pan());
    }

    #[test]
    fn zoom_and_new_decode_clear_active_pan_state() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("pan-clear.png", 300, 300), ImageFolder::empty());
        assert!(app.show_actual_size());
        assert!(app.begin_pan(ViewportPoint::from_client_position(50, 50)));

        assert!(app.zoom_at(1.25, ViewportPoint::from_client_position(50, 50)));
        assert!(app.panning.is_none());

        assert!(app.begin_pan(ViewportPoint::from_client_position(50, 50)));
        let request = app.begin_image_decode(PathBuf::from("next.png"));

        assert_eq!(request.path(), Path::new("next.png"));
        assert!(app.panning.is_none());
    }

    #[test]
    fn decode_requests_carry_configured_memory_policy_and_animation_timing() {
        let mut config = AppConfig::default();
        let mut memory = MemoryPolicySettings::default();
        memory.set_max_transient_decode_mib(321);
        config.set_memory_policy_settings(memory);
        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(25);
        timing.set_max_frame_delay_ms(500);
        timing.set_default_frame_delay_ms(75);
        config.set_animation_timing_settings(timing);

        let mut app = ViewerApp::with_config(config.clone());
        let request = app.begin_image_decode(PathBuf::from("configured.png"));

        assert_eq!(app.memory_policy(), config.image_memory_policy());
        assert_eq!(request.memory_policy(), config.image_memory_policy());
        assert_eq!(
            request.animation_timing(),
            config.animation_timing_settings()
        );
    }

    #[test]
    fn applying_config_updates_current_animation_timer_behavior() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            animated_test_image_with_delays("settings-anim.gif", 2, 1, vec![0, 5, 500]),
            ImageFolder::empty(),
        );
        assert_eq!(app.animation_timer_interval_ms(), Some(100));

        let mut config = app.config_snapshot();
        config.set_animation_autoplay(false);
        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(50);
        timing.set_max_frame_delay_ms(200);
        timing.set_default_frame_delay_ms(150);
        config.set_animation_timing_settings(timing);

        assert!(app.apply_config(config));
        let playback = app
            .current_animation_playback()
            .expect("animation playback after applying config");
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Paused);
        assert_eq!(playback.frame_delays_ms(), &[150, 50, 200]);
        assert_eq!(app.animation_timer_interval_ms(), None);

        let mut config = app.config_snapshot();
        config.set_animation_autoplay(true);
        assert!(app.apply_config(config));

        let playback = app
            .current_animation_playback()
            .expect("animation playback after enabling autoplay");
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Playing);
        assert_eq!(app.animation_timer_interval_ms(), Some(150));
    }

    #[test]
    fn applying_config_updates_runtime_policy_and_future_decode_requests() {
        let mut app = ViewerApp::new();
        let mut config = app.config_snapshot();
        let mut memory = MemoryPolicySettings::default();
        memory.set_max_image_pixels(12_345);
        config.set_memory_policy_settings(memory);
        let mut timing = AnimationTimingSettings::default();
        timing.set_default_frame_delay_ms(75);
        config.set_animation_timing_settings(timing);

        assert!(app.apply_config(config.clone()));
        let request = app.begin_image_decode(PathBuf::from("configured-after-apply.png"));

        assert_eq!(app.config(), &config);
        assert_eq!(app.memory_policy(), config.image_memory_policy());
        assert_eq!(request.memory_policy(), config.image_memory_policy());
        assert_eq!(
            request.animation_timing(),
            config.animation_timing_settings()
        );
    }

    #[test]
    fn memory_policy_settings_drive_preview_and_full_resolution_decisions() {
        let dir = unique_temp_dir("memory-policy-preview");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("large-by-policy.png");
        write_png_fixture(&path, 100, 100);

        let mut memory = MemoryPolicySettings::default();
        memory.set_large_image_pixel_threshold(1);
        memory.set_preview_max_pixels(400);
        memory.set_preview_oversample(1);
        let mut config = AppConfig::default();
        config.set_memory_policy_settings(memory);
        let mut app = ViewerApp::with_config(config);
        app.handle_resize(20, 20);

        app.load_image(&path)
            .expect("load preview-backed image by configured policy");
        let ImageState::Loaded(image) = app.image_state() else {
            panic!("image should be loaded");
        };
        assert_eq!(image.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(image.source_size(), ImageSize::new(100, 100));
        assert_eq!(image.pixels().size(), ImageSize::new(20, 20));

        assert!(app.show_actual_size());
        let mut deny_memory = memory;
        deny_memory.set_full_resolution_request_scale(2.0);
        let mut config = app.config_snapshot();
        config.set_memory_policy_settings(deny_memory);
        assert!(app.apply_config(config));
        assert_eq!(app.begin_full_resolution_decode(), None);

        let mut allow_memory = deny_memory;
        allow_memory.set_full_resolution_request_scale(0.5);
        let mut config = app.config_snapshot();
        config.set_memory_policy_settings(allow_memory);
        assert!(app.apply_config(config.clone()));
        let request = app
            .begin_full_resolution_decode()
            .expect("full-resolution request after applying memory policy");
        assert_eq!(request.memory_policy(), config.image_memory_policy());

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn resize_clamps_manual_zoom_offset_through_app_state() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(
            test_image("resize-clamp.png", 300, 300),
            ImageFolder::empty(),
        );
        app.view_transform = ViewTransform::manual_zoom(1.0, ViewOffset::new(1000.0, 1000.0));

        app.handle_resize(200, 150);

        let rect = app
            .view_transform()
            .display_rect(app.viewport(), ImageSize::new(300, 300))
            .expect("display rect after resize");
        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        assert!(rect.x() <= 0);
        assert!(rect.y() <= 0);
        assert!(rect.x() + rect.width() >= app.viewport().width() as i32);
        assert!(rect.y() + rect.height() >= app.viewport().height() as i32);
    }

    #[test]
    fn loaded_image_render_stays_visible_after_zoom_pan_rotate_and_resize() {
        let dir = unique_temp_dir("view-transform-smoke");
        fs::create_dir_all(&dir).expect("test dir");
        let path = dir.join("wide.png");
        write_png_fixture(&path, 1200, 300);

        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.handle_resize(500, 400);
        app.load_image(&path).expect("load wide image");

        let fit_rect = app.render_rgba8(viewport).expect("fit render").rect();
        assert_render_rect_visible(fit_rect, viewport);

        assert!(app.zoom_at(4.0, ViewportPoint::from_client_position(250, 200)));
        assert!(app.begin_pan(ViewportPoint::from_client_position(250, 200)));
        assert!(app.update_pan(ViewportPoint::from_client_position(-1000, 1000)));
        assert!(app.end_pan());

        let panned_rect = app.render_rgba8(viewport).expect("panned render").rect();
        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        assert_render_rect_visible(panned_rect, viewport);

        assert!(app.rotate_clockwise());
        let rotated_rect = app.render_rgba8(viewport).expect("rotated render").rect();
        assert_render_rect_visible(rotated_rect, viewport);

        let resized_viewport = ViewportSize::from_client_size(320, 700);
        app.handle_resize(320, 700);
        let resized_rect = app
            .render_rgba8(resized_viewport)
            .expect("resized render")
            .rect();
        assert_eq!(app.view_transform().mode(), ViewMode::ManualZoom);
        assert_render_rect_visible(resized_rect, resized_viewport);

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn rotated_display_image_is_cached_for_same_image_and_rotation() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("cache.png", 3, 2), ImageFolder::empty());

        assert!(app.rotate_clockwise());
        let first_pixels = app
            .display_rgba8()
            .expect("display image")
            .pixels()
            .as_ptr();
        let second_pixels = app
            .display_rgba8()
            .expect("display image")
            .pixels()
            .as_ptr();

        assert_eq!(first_pixels, second_pixels);
    }

    #[test]
    fn downscaled_render_image_reuses_resample_cache_for_small_target_changes() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("scaled.png", 1000, 800), ImageFolder::empty());

        let first_pixels = {
            let first = app
                .render_rgba8(ViewportSize::from_client_size(501, 401))
                .expect("first render image");
            assert_eq!(first.pixels().size(), ImageSize::new(512, 416));
            assert_eq!(first.scaling_quality(), ScalingQuality::Balanced);
            first.pixels().pixels().as_ptr()
        };

        let second = app
            .render_rgba8(ViewportSize::from_client_size(510, 408))
            .expect("second render image");
        assert_eq!(second.pixels().size(), ImageSize::new(512, 416));
        assert_eq!(second.pixels().pixels().as_ptr(), first_pixels);
    }

    #[test]
    fn interactive_render_defers_scaled_cache_rebuild_until_resumed() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.replace_loaded_image(
            test_image("interactive.png", 1000, 800),
            ImageFolder::empty(),
        );

        app.defer_scaling_cache_rebuilds();
        {
            let render = app
                .render_rgba8(viewport)
                .expect("interactive render image");
            assert_eq!(render.rect().size(), Some(ImageSize::new(500, 400)));
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());

        {
            let render = app
                .render_rgba8_for_paint(viewport)
                .expect("paint before settle timer resumes");
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());
        assert!(app.resume_scaling_cache_rebuilds());

        {
            let render = app.render_rgba8(viewport).expect("settled render image");
            assert_eq!(render.pixels().size(), ImageSize::new(512, 400));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(app.render_cache.has_scaled_image());
        assert!(!app.resume_scaling_cache_rebuilds());
    }

    #[test]
    fn prepare_first_render_defers_scaled_cache_rebuild_without_interactive_state() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.replace_loaded_image(test_image("paint.png", 1000, 800), ImageFolder::empty());

        {
            let render = app
                .prepare_first_render(viewport)
                .expect("first render image");
            assert_eq!(render.rect().size(), Some(ImageSize::new(500, 400)));
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());
        assert!(app.resume_scaling_cache_rebuilds());

        {
            let render = app.render_rgba8(viewport).expect("settled render image");
            assert_eq!(render.pixels().size(), ImageSize::new(512, 400));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(app.render_cache.has_scaled_image());
    }

    #[test]
    fn starting_new_decode_clears_deferred_scaling_cache_rebuild() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.replace_loaded_image(
            test_image("deferred-old.png", 1000, 800),
            ImageFolder::empty(),
        );

        app.prepare_first_render(viewport)
            .expect("paint render image");
        assert!(app.has_deferred_scaling_cache_rebuild());

        let _request = app.begin_image_decode(PathBuf::from("next.png"));

        assert!(!app.has_deferred_scaling_cache_rebuild());
        assert!(!app.render_cache.has_scaled_image());
    }

    #[test]
    fn decoded_image_first_paint_defers_scaled_cache_rebuild() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        let path = PathBuf::from("decoded-next.png");
        let request = app.begin_image_decode(path.clone());

        let outcome = app.apply_decoded_image(
            request.generation(),
            test_image_from_path(path, 1000, 800),
            ImageFolder::empty(),
        );
        assert_eq!(outcome, DecodeApplyOutcome::Applied);

        {
            let render = app
                .render_rgba8_for_paint(viewport)
                .expect("first paint render image");
            assert_eq!(render.rect().size(), Some(ImageSize::new(500, 400)));
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());
    }

    #[test]
    fn render_for_paint_builds_scaled_cache_after_first_paint_defer_resumes() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        let path = PathBuf::from("decoded-settle.png");
        let request = app.begin_image_decode(path.clone());

        let outcome = app.apply_decoded_image(
            request.generation(),
            test_image_from_path(path, 1000, 800),
            ImageFolder::empty(),
        );
        assert_eq!(outcome, DecodeApplyOutcome::Applied);

        {
            let render = app
                .render_rgba8_for_paint(viewport)
                .expect("first paint render image");
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());

        {
            let render = app
                .render_rgba8_for_paint(viewport)
                .expect("paint before first-render settle resumes");
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
        }
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());
        assert!(app.resume_scaling_cache_rebuilds());

        {
            let render = app
                .render_rgba8_for_paint(viewport)
                .expect("settled paint render image");
            assert_eq!(render.rect().size(), Some(ImageSize::new(500, 400)));
            assert_eq!(render.pixels().size(), ImageSize::new(512, 400));
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
        }
        assert!(app.render_cache.has_scaled_image());
        assert!(!app.has_deferred_scaling_cache_rebuild());
        assert!(!app.resume_scaling_cache_rebuilds());
    }

    #[test]
    fn decoded_image_first_render_ignores_worker_render_ready_buffer() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        let path = PathBuf::from("decoded-render-ready.png");
        let request = app.begin_image_decode(path.clone());
        let mut image = test_image_from_path(path, 1000, 800);
        let rect = ViewTransform::FIT_TO_WINDOW
            .display_rect(viewport, ImageSize::new(1000, 800))
            .expect("fit display rect");
        let render_ready_pixels = PixelImage::from(test_rgba8_image(500, 400));
        let render_ready_pointer = render_ready_pixels.pixels().as_ptr();
        image.set_render_ready_image(RenderReadyImage::new(
            render_ready_pixels,
            viewport,
            ViewMode::FitToWindow,
            ScalingQuality::Balanced,
            ImageOrientation::NORMAL,
            rect,
            ScalingQuality::Balanced,
        ));

        let outcome = app.apply_decoded_image(request.generation(), image, ImageFolder::empty());
        assert_eq!(outcome, DecodeApplyOutcome::Applied);

        let render = app
            .prepare_first_render(viewport)
            .expect("first render image");
        assert_eq!(render.rect().size(), Some(ImageSize::new(500, 400)));
        assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
        assert_ne!(render.pixels().pixels().as_ptr(), render_ready_pointer);
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.has_deferred_scaling_cache_rebuild());
    }

    #[test]
    fn actual_size_render_skips_scaled_cache() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("actual.png", 100, 80), ImageFolder::empty());
        app.render_rgba8(ViewportSize::from_client_size(50, 40))
            .expect("downscaled render image");
        assert!(app.render_cache.has_scaled_image());

        assert!(app.show_actual_size());

        let pixels = {
            let render = app
                .render_rgba8(ViewportSize::from_client_size(200, 200))
                .expect("render image");
            assert_eq!(render.pixels().size(), ImageSize::new(100, 80));
            assert_eq!(render.scaling_quality(), ScalingQuality::Nearest);
            render.pixels().pixels().as_ptr()
        };
        assert_eq!(
            pixels,
            app.display_rgba8()
                .expect("display image")
                .pixels()
                .as_ptr()
        );
        assert!(!app.render_cache.has_scaled_image());
    }

    #[test]
    fn applying_scaling_quality_changes_render_path() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(500, 400);
        app.replace_loaded_image(test_image("quality.png", 1000, 800), ImageFolder::empty());
        {
            let render = app.render_rgba8(viewport).expect("balanced render");
            assert_eq!(render.scaling_quality(), ScalingQuality::Balanced);
            assert_eq!(render.pixels().size(), ImageSize::new(512, 400));
        }
        assert!(app.render_cache.has_scaled_image());

        let mut config = app.config_snapshot();
        config.set_scaling_quality(ScalingQuality::Nearest);
        assert!(app.apply_config(config));
        {
            let render = app.render_rgba8(viewport).expect("nearest render");
            assert_eq!(render.scaling_quality(), ScalingQuality::Nearest);
            assert_eq!(render.pixels().size(), ImageSize::new(1000, 800));
        }
        assert!(!app.render_cache.has_scaled_image());

        let mut config = app.config_snapshot();
        config.set_scaling_quality(ScalingQuality::HighQuality);
        assert!(app.apply_config(config));
        {
            let render = app.render_rgba8(viewport).expect("high quality render");
            assert_eq!(render.scaling_quality(), ScalingQuality::HighQuality);
            assert_eq!(render.pixels().size(), ImageSize::new(512, 400));
        }
        assert!(app.render_cache.has_scaled_image());
    }

    #[test]
    fn fit_to_window_at_exact_size_skips_scaled_cache() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("fit-actual.png", 100, 80), ImageFolder::empty());

        let pixels = {
            let render = app
                .render_rgba8(ViewportSize::from_client_size(100, 80))
                .expect("render image");
            assert_eq!(render.rect().size(), Some(ImageSize::new(100, 80)));
            assert_eq!(render.pixels().size(), ImageSize::new(100, 80));
            assert_eq!(render.scaling_quality(), ScalingQuality::Nearest);
            render.pixels().pixels().as_ptr()
        };

        assert_eq!(
            pixels,
            app.display_rgba8()
                .expect("display image")
                .pixels()
                .as_ptr()
        );
        assert!(!app.render_cache.has_scaled_image());
    }

    #[test]
    fn window_resize_rotation_and_image_replacement_invalidate_scaled_cache() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("first.png", 1000, 800), ImageFolder::empty());
        app.render_rgba8(ViewportSize::from_client_size(500, 400))
            .expect("render image");
        assert!(app.render_cache.has_scaled_image());

        app.handle_resize(600, 400);
        assert!(!app.render_cache.has_scaled_image());

        app.render_rgba8(ViewportSize::from_client_size(500, 400))
            .expect("render image");
        assert!(app.render_cache.has_scaled_image());
        assert!(app.rotate_clockwise());
        assert!(!app.render_cache.has_scaled_image());

        app.render_rgba8(ViewportSize::from_client_size(500, 400))
            .expect("render image");
        assert!(app.render_cache.has_scaled_image());
        app.replace_loaded_image(test_image("second.png", 1000, 800), ImageFolder::empty());
        assert!(!app.render_cache.has_scaled_image());
    }

    #[test]
    fn navigation_preloads_do_not_evict_required_oriented_display_pixels() {
        let mut app = ViewerApp::new();
        let viewport = ViewportSize::from_client_size(40, 40);
        app.handle_resize(40, 40);
        app.replace_loaded_image(
            test_rgb8_image_with_exif("current.jpg", 100, 80, ImageOrientation::Rotate90),
            ImageFolder::empty(),
        );

        app.render_rgba8(viewport).expect("initial render");
        assert!(app.render_cache.has_oriented_image());
        assert!(app.render_cache.has_scaled_image());
        app.handle_paint();

        for (index, path) in ["previous.jpg", "next.jpg"].into_iter().enumerate() {
            app.navigation_preload_cache
                .push(NavigationPreloadCacheEntry {
                    path_key: navigation_preload_path_key(Path::new(path)),
                    image: test_rgb8_image(path, 100, 80),
                    viewport,
                    memory_policy: app.memory_policy,
                    animation_timing: app.config.animation_timing_settings(),
                    last_used: app.paint_count + index as u64 + 1,
                });
        }
        app.enforce_memory_policy();

        assert!(app.render_cache.has_oriented_image());
        let render = app
            .render_rgba8(viewport)
            .expect("oriented image should remain renderable after preload pressure");
        assert!(render.pixels().is_valid());
    }

    #[test]
    fn image_transition_clears_all_memory_backed_caches() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(test_image("first.png", 1000, 800), ImageFolder::empty());

        assert!(app.rotate_clockwise());
        app.display_rgba8().expect("oriented image");
        app.render_rgba8(ViewportSize::from_client_size(500, 400))
            .expect("scaled image");
        app.store_animation_frame_cache(1, test_rgba8_image(2, 1));

        assert!(app.render_cache.has_oriented_image());
        assert!(app.render_cache.has_scaled_image());
        assert_eq!(app.animation_frame_cache.len(), 1);

        app.replace_loaded_image(test_image("second.png", 10, 10), ImageFolder::empty());

        assert!(!app.render_cache.has_oriented_image());
        assert!(!app.render_cache.has_scaled_image());
        assert!(app.animation_frame_cache.is_empty());
    }

    #[test]
    fn memory_policy_evicts_only_selected_animation_frame_cache_entry() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());
        app.paint_count = 1;
        app.store_animation_frame_cache(1, test_rgba8_image(2, 1));
        app.paint_count = 2;
        app.store_animation_frame_cache(2, test_rgba8_image(2, 1));

        let mut config = app.config_snapshot();
        let mut memory_policy = config.memory_policy_settings();
        memory_policy.set_max_cache_entries(1);
        config.set_memory_policy_settings(memory_policy);
        assert!(app.apply_config(config));

        assert_eq!(app.animation_frame_cache.len(), 1);
        assert_eq!(app.animation_frame_cache[0].frame_index, 2);
    }

    #[test]
    fn unidentified_animation_frame_eviction_clears_frame_cache() {
        let mut app = ViewerApp::new();
        app.store_animation_frame_cache(1, test_rgba8_image(2, 1));
        app.store_animation_frame_cache(2, test_rgba8_image(2, 1));

        app.evict_animation_frame_cache(None);

        assert!(app.animation_frame_cache.is_empty());
    }

    #[test]
    fn image_info_text_is_empty_without_image_and_populated_when_loaded() {
        let mut app = ViewerApp::new();

        assert_eq!(app.image_info_text(), None);

        app.replace_loaded_image(test_image("info.png", 640, 480), ImageFolder::empty());

        assert_eq!(
            app.image_info_text(),
            Some("info.png | 640x480 | 0 B | PNG | Fit".to_owned())
        );
    }

    #[test]
    fn configured_default_view_mode_is_used_for_new_images() {
        let config = AppConfig::new(
            None,
            ViewMode::ActualSize,
            ScalingQuality::Balanced,
            None,
            90,
            true,
        );
        let mut app = ViewerApp::with_config(config);

        app.replace_loaded_image(
            test_image("actual-default.png", 640, 480),
            ImageFolder::empty(),
        );

        assert_eq!(app.view_transform().mode(), ViewMode::ActualSize);
    }

    #[test]
    fn configured_memory_policy_is_used_by_app() {
        let mut config = AppConfig::default();
        let mut memory_policy = MemoryPolicySettings::default();
        memory_policy.set_max_image_pixels(12_345);
        config.set_memory_policy_settings(memory_policy);

        let app = ViewerApp::with_config(config);

        assert_eq!(app.memory_policy().max_image_pixels(), 12_345);
    }

    #[test]
    fn loading_image_updates_recent_folder_in_config() {
        let mut app = ViewerApp::new();
        let path = PathBuf::from("Pictures").join("photo.png");

        app.replace_loaded_image(test_image_from_path(path, 640, 480), ImageFolder::empty());

        assert_eq!(app.recent_folder(), Some(std::path::Path::new("Pictures")));
    }

    #[test]
    fn animation_autoplay_config_pauses_new_animations() {
        let config = AppConfig::new(
            None,
            ViewMode::FitToWindow,
            ScalingQuality::Balanced,
            None,
            90,
            false,
        );
        let mut app = ViewerApp::with_config(config);

        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());

        let playback = app
            .current_animation_playback()
            .expect("animation playback");
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Paused);
        assert_eq!(app.animation_timer_interval_ms(), None);
    }

    #[test]
    fn app_command_path_handles_view_commands() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(test_image("command.png", 300, 50), ImageFolder::empty());

        let outcome = app
            .handle_command(Command::RotateCounterClockwise)
            .expect("command handled");

        assert_eq!(outcome, AppCommandOutcome::Changed);
        assert_eq!(app.rotation(), ImageRotation::Degrees270);
    }

    #[test]
    fn repeated_menu_image_commands_keep_loaded_image_renderable_after_resize() {
        let mut app = ViewerApp::new();
        app.handle_resize(240, 160);
        app.replace_loaded_image(
            test_image("menu-command.png", 320, 120),
            ImageFolder::empty(),
        );

        let commands = [
            Command::ActualSize,
            Command::ZoomIn,
            Command::ZoomOut,
            Command::FitToWindow,
            Command::RotateClockwise,
            Command::RotateCounterClockwise,
            Command::Navigate(ImageNavigationDirection::Next),
            Command::Navigate(ImageNavigationDirection::Previous),
        ];

        for round in 0..2 {
            for command in commands {
                let outcome = app.handle_command(command).expect("menu command");
                assert_ne!(outcome, AppCommandOutcome::Unhandled);
                assert!(app.image_state().has_image());
                let render = app
                    .render_rgba8(app.viewport())
                    .expect("image remains renderable after menu command");
                assert_render_rect_visible(render.rect(), app.viewport());
            }

            let width = 180 + round * 70;
            let height = 220 - round * 60;
            app.handle_resize(width, height);
        }
    }

    #[test]
    fn stale_decode_result_does_not_replace_current_image() {
        let mut app = ViewerApp::new();
        let first = app.begin_image_decode(PathBuf::from("first.png"));
        let _second = app.begin_image_decode(PathBuf::from("second.png"));

        let outcome = app.apply_decoded_image(
            first.generation(),
            test_image("first.png", 10, 10),
            ImageFolder::empty(),
        );

        assert_eq!(outcome, DecodeApplyOutcome::Stale);
        assert_eq!(app.image_info_text(), None);
    }

    #[test]
    fn decode_result_for_same_generation_but_different_path_is_stale() {
        let mut app = ViewerApp::new();
        let request = app.begin_image_decode(PathBuf::from("requested.png"));

        let outcome = app.apply_decoded_image(
            request.generation(),
            test_image("other.png", 10, 10),
            ImageFolder::empty(),
        );

        assert_eq!(outcome, DecodeApplyOutcome::Stale);
        assert_eq!(app.image_info_text(), None);
    }

    #[test]
    fn stale_full_resolution_result_after_image_transition_is_ignored() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(
            preview_test_image("first.png", 10, 10, 20, 20),
            ImageFolder::empty(),
        );
        assert!(app.show_actual_size());
        let request = app
            .begin_full_resolution_decode()
            .expect("full-resolution request");

        app.replace_loaded_image(
            preview_test_image("second.png", 10, 10, 20, 20),
            ImageFolder::empty(),
        );
        let outcome = app.apply_full_resolution_image(
            request.generation(),
            request.file_version(),
            test_rgba8_image(20, 20),
        );

        assert_eq!(outcome, DecodeApplyOutcome::Stale);
        assert_eq!(app.current_source_path(), Some(Path::new("second.png")));
        let ImageState::Loaded(image) = app.image_state() else {
            panic!("image should remain loaded");
        };
        assert_eq!(image.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(image.pixels().size(), ImageSize::new(10, 10));
    }

    #[test]
    fn full_resolution_result_for_changed_file_version_is_stale() {
        let mut app = ViewerApp::new();
        app.handle_resize(100, 100);
        app.replace_loaded_image(
            preview_test_image("same.png", 10, 10, 20, 20),
            ImageFolder::empty(),
        );
        assert!(app.show_actual_size());
        let request = app
            .begin_full_resolution_decode()
            .expect("full-resolution request");
        let changed_version = ImageFileVersion::new(0, UNIX_EPOCH + Duration::from_secs(1));

        let outcome = app.apply_full_resolution_image(
            request.generation(),
            Some(changed_version),
            test_rgba8_image(20, 20),
        );

        assert_eq!(outcome, DecodeApplyOutcome::Stale);
        let ImageState::Loaded(image) = app.image_state() else {
            panic!("image should remain loaded");
        };
        assert_eq!(image.buffer_kind(), ImageBufferKind::Preview);
        assert_eq!(image.pixels().size(), ImageSize::new(10, 10));
    }

    #[test]
    fn navigation_decode_failure_keeps_current_image_and_reports_status() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("a.png");
        let failed_path = PathBuf::from("b.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            vec![current_path.clone(), failed_path.clone()],
        );
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);

        let request = app
            .begin_navigation_decode(ImageNavigationDirection::Next)
            .expect("navigation request");
        let error = ViewerAppError::from(LoadImageError::FileAccess {
            path: failed_path,
            source: io::Error::from_raw_os_error(2),
        });
        let outcome = app.finish_failed_initial_decode(request.generation(), &error);

        assert_eq!(outcome, DecodeFailurePresentation::StatusMessage);
        assert_eq!(app.current_source_path(), Some(current_path.as_path()));
        let status = app.image_info_text().expect("navigation failure status");
        assert!(status.contains("Could not open the image to navigate to"));
        assert!(status.contains("The current image is kept"));
        assert!(status.contains("Could not find the image file"));
    }

    #[test]
    fn navigation_decode_success_reuses_existing_folder_snapshot() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("a.png");
        let next_path = PathBuf::from("b.png");
        let rescanned_extra_path = PathBuf::from("c.png");
        let folder =
            ImageFolder::from_paths(&current_path, vec![current_path.clone(), next_path.clone()]);
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);
        let request = app
            .begin_navigation_decode(ImageNavigationDirection::Next)
            .expect("navigation request");
        let rescanned_folder =
            ImageFolder::from_paths(&next_path, vec![next_path.clone(), rescanned_extra_path]);

        let outcome = app.apply_decoded_image(
            request.generation(),
            test_image_from_path(next_path.clone(), 2, 1),
            rescanned_folder,
        );

        assert_eq!(outcome, DecodeApplyOutcome::Applied);
        assert_eq!(app.current_source_path(), Some(next_path.as_path()));
        assert_eq!(folder_file_names(app.image_folder()), ["a.png", "b.png"]);
        assert_eq!(app.image_folder().current_index(), Some(1));
    }

    #[test]
    fn navigation_preload_requests_select_adjacent_folder_images() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("b.png");
        let previous_path = PathBuf::from("a.png");
        let next_path = PathBuf::from("c.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            [
                previous_path.clone(),
                current_path.clone(),
                next_path.clone(),
            ],
        );
        app.replace_loaded_image(test_image_from_path(current_path, 2, 1), folder);

        let preload_paths = app
            .navigation_preload_requests()
            .into_iter()
            .map(|request| request.path().to_path_buf())
            .collect::<Vec<_>>();

        assert_eq!(preload_paths, vec![previous_path, next_path]);
    }

    #[test]
    fn navigation_uses_preloaded_image_when_current_file_version_matches() {
        let dir = unique_temp_dir("navigation-preload-hit");
        std::fs::create_dir_all(&dir).expect("test dir");
        let current_path = dir.join("a.png");
        let next_path = dir.join("b.png");
        let extra_path = dir.join("c.png");
        write_png_fixture(&current_path, 2, 1);
        write_png_fixture(&next_path, 3, 1);
        write_png_fixture(&extra_path, 4, 1);

        let mut app = ViewerApp::new();
        app.load_image(&current_path).expect("load current image");
        let request = app
            .navigation_preload_requests()
            .into_iter()
            .find(|request| request.path() == next_path.as_path())
            .expect("next preload request");
        let preloaded = load_image_file(&next_path).expect("load preloaded image");

        assert_eq!(
            app.store_preloaded_navigation_image(&request, preloaded),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(
            app.begin_navigation_or_use_preloaded(ImageNavigationDirection::Next),
            NavigationStartOutcome::AppliedPreloaded
        );

        assert_eq!(app.current_source_path(), Some(next_path.as_path()));
        let ImageState::Loaded(image) = app.image_state() else {
            panic!("preloaded navigation image should be current");
        };
        assert_eq!(image.source_size(), ImageSize::new(3, 1));

        let _ = std::fs::remove_dir_all(dir);
    }

    #[test]
    fn initial_decode_can_apply_deferred_folder_scan_after_image_is_loaded() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("b.png");
        let request = app.begin_image_decode(current_path.clone());
        let initial_folder = ImageFolder::from_paths(&current_path, [current_path.clone()]);
        let scanned_folder = ImageFolder::from_paths(
            &current_path,
            [
                PathBuf::from("a.png"),
                current_path.clone(),
                PathBuf::from("c.png"),
            ],
        );

        let initial_outcome = app.apply_decoded_image(
            request.generation(),
            test_image_from_path(current_path.clone(), 2, 1),
            initial_folder,
        );
        let scan_outcome =
            app.apply_scanned_image_folder(request.generation(), &current_path, scanned_folder);

        assert_eq!(initial_outcome, DecodeApplyOutcome::Applied);
        assert_eq!(scan_outcome, DecodeApplyOutcome::Applied);
        assert_eq!(app.current_source_path(), Some(current_path.as_path()));
        assert_eq!(
            folder_file_names(app.image_folder()),
            ["a.png", "b.png", "c.png"]
        );
        assert_eq!(app.image_folder().current_index(), Some(1));
    }

    #[test]
    fn navigation_requested_before_deferred_folder_scan_starts_after_scan_applies() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("b.png");
        let next_path = PathBuf::from("c.png");
        let request = app.begin_image_decode(current_path.clone());
        let initial_folder = ImageFolder::from_paths(&current_path, [current_path.clone()]);

        assert_eq!(
            app.apply_decoded_image(
                request.generation(),
                test_image_from_path(current_path.clone(), 2, 1),
                initial_folder,
            ),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );

        let scanned_folder = ImageFolder::from_paths(
            &current_path,
            [
                PathBuf::from("a.png"),
                current_path.clone(),
                next_path.clone(),
            ],
        );
        assert_eq!(
            app.apply_scanned_image_folder(request.generation(), &current_path, scanned_folder),
            DecodeApplyOutcome::Applied
        );
        let pending_request = app
            .take_pending_navigation_after_folder_scan()
            .expect("pending navigation request");

        assert_eq!(pending_request.path(), next_path.as_path());
        assert_eq!(
            pending_request.purpose(),
            ImageDecodePurpose::FolderNavigation(ImageNavigationDirection::Next)
        );
    }

    #[test]
    fn pending_navigation_after_folder_scan_is_cleared_when_no_target_exists() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("only.png");
        let request = app.begin_image_decode(current_path.clone());
        let initial_folder = ImageFolder::from_paths(&current_path, [current_path.clone()]);

        assert_eq!(
            app.apply_decoded_image(
                request.generation(),
                test_image_from_path(current_path.clone(), 2, 1),
                initial_folder,
            ),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );
        let scanned_folder = ImageFolder::from_paths(&current_path, [current_path.clone()]);
        assert_eq!(
            app.apply_scanned_image_folder(request.generation(), &current_path, scanned_folder),
            DecodeApplyOutcome::Applied
        );

        assert_eq!(app.take_pending_navigation_after_folder_scan(), None);
        assert_eq!(app.take_pending_navigation_after_folder_scan(), None);
    }

    #[test]
    fn failed_deferred_folder_scan_clears_pending_navigation_wait() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("only.png");
        let request = app.begin_image_decode(current_path.clone());
        let initial_folder = ImageFolder::from_paths(&current_path, [current_path.clone()]);

        assert_eq!(
            app.apply_decoded_image(
                request.generation(),
                test_image_from_path(current_path.clone(), 2, 1),
                initial_folder,
            ),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );
        assert!(app.pending_folder_scan.is_some());
        assert_eq!(
            app.pending_navigation_after_folder_scan,
            Some(ImageNavigationDirection::Next)
        );

        assert_eq!(
            app.finish_pending_folder_scan_without_update(request.generation(), &current_path),
            DecodeApplyOutcome::Applied
        );

        assert!(app.pending_folder_scan.is_none());
        assert_eq!(app.pending_navigation_after_folder_scan, None);
        assert_eq!(app.take_pending_navigation_after_folder_scan(), None);
        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );
        assert_eq!(app.pending_navigation_after_folder_scan, None);
    }

    #[test]
    fn navigation_settings_can_disable_wrap_in_app_flow() {
        let mut config = AppConfig::default();
        let mut navigation = NavigationSettings::default();
        navigation.set_wrap_navigation(false);
        config.set_navigation_settings(navigation);
        let mut app = ViewerApp::with_config(config);
        let current_path = PathBuf::from("b.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            [PathBuf::from("a.png"), current_path.clone()],
        );
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);

        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );
        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next without wrap"),
            NavigationOutcome::Noop
        );
        assert_eq!(app.current_source_path(), Some(current_path.as_path()));
    }

    #[test]
    fn applying_navigation_settings_updates_existing_folder_flow() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("b.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            [PathBuf::from("a.png"), current_path.clone()],
        );
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);

        let mut config = app.config_snapshot();
        let mut navigation = NavigationSettings::default();
        navigation.set_wrap_navigation(false);
        config.set_navigation_settings(navigation);
        assert!(app.apply_config(config));

        assert_eq!(
            app.begin_navigation_decode(ImageNavigationDirection::Next),
            None
        );

        let current_path = PathBuf::from("a.png");
        let failed_path = PathBuf::from("b.png");
        let retry_path = PathBuf::from("c.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            vec![
                current_path.clone(),
                failed_path.clone(),
                retry_path.clone(),
            ],
        );
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);
        let mut config = app.config_snapshot();
        let mut navigation = NavigationSettings::default();
        navigation.set_auto_skip_failed_navigation(true);
        navigation.set_max_navigation_attempts_per_command(2);
        config.set_navigation_settings(navigation);
        assert!(app.apply_config(config));

        let request = app
            .begin_navigation_decode(ImageNavigationDirection::Next)
            .expect("navigation request");
        let error = ViewerAppError::from(LoadImageError::FileAccess {
            path: failed_path,
            source: io::Error::from_raw_os_error(2),
        });
        match app.finish_failed_initial_decode(request.generation(), &error) {
            DecodeFailurePresentation::RetryNavigation(retry) => {
                assert_eq!(retry.path(), retry_path.as_path());
            }
            other => panic!("expected retry after applying navigation settings, got {other:?}"),
        }
    }

    #[test]
    fn navigation_decode_failure_can_request_configured_auto_skip_retry() {
        let mut config = AppConfig::default();
        let mut navigation = NavigationSettings::default();
        navigation.set_auto_skip_failed_navigation(true);
        navigation.set_max_navigation_attempts_per_command(2);
        config.set_navigation_settings(navigation);
        let mut app = ViewerApp::with_config(config);
        let current_path = PathBuf::from("a.png");
        let failed_path = PathBuf::from("b.png");
        let retry_path = PathBuf::from("c.png");
        let folder = ImageFolder::from_paths(
            &current_path,
            vec![
                current_path.clone(),
                failed_path.clone(),
                retry_path.clone(),
            ],
        );
        app.replace_loaded_image(test_image_from_path(current_path.clone(), 2, 1), folder);

        let request = app
            .begin_navigation_decode(ImageNavigationDirection::Next)
            .expect("navigation request");
        let error = ViewerAppError::from(LoadImageError::FileAccess {
            path: failed_path,
            source: io::Error::from_raw_os_error(2),
        });
        let outcome = app.finish_failed_initial_decode(request.generation(), &error);

        match outcome {
            DecodeFailurePresentation::RetryNavigation(retry) => {
                assert_eq!(retry.path(), retry_path.as_path());
                assert_ne!(retry.generation(), request.generation());
            }
            other => panic!("expected retry, got {other:?}"),
        }
    }

    #[test]
    fn sync_navigation_auto_skip_moves_to_next_loadable_file() {
        let dir = unique_temp_dir("navigation-auto-skip");
        fs::create_dir_all(&dir).expect("test dir");
        let current = dir.join("a-current.png");
        let broken = dir.join("b-broken.png");
        let next = dir.join("c-next.png");
        write_png_fixture(&current, 16, 16);
        fs::write(&broken, b"not an image").expect("broken image");
        write_png_fixture(&next, 24, 18);

        let mut config = AppConfig::default();
        let mut navigation = NavigationSettings::default();
        navigation.set_auto_skip_failed_navigation(true);
        navigation.set_max_navigation_attempts_per_command(2);
        config.set_navigation_settings(navigation);
        let mut app = ViewerApp::with_config(config);
        app.load_image(&current).expect("load current image");

        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("auto-skip next"),
            NavigationOutcome::Moved
        );
        assert_eq!(app.current_source_path(), Some(next.as_path()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn sync_navigation_success_reuses_existing_folder_snapshot() {
        let dir = unique_temp_dir("navigation-folder-cache");
        fs::create_dir_all(&dir).expect("test dir");
        let current = dir.join("a-current.png");
        let next = dir.join("b-next.png");
        let added_after_scan = dir.join("c-added.png");
        write_png_fixture(&current, 16, 16);
        write_png_fixture(&next, 24, 18);

        let mut app = ViewerApp::new();
        app.load_image(&current).expect("load current image");
        assert_eq!(
            folder_file_names(app.image_folder()),
            ["a-current.png", "b-next.png"]
        );
        write_png_fixture(&added_after_scan, 32, 24);

        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next navigation"),
            NavigationOutcome::Moved
        );

        assert_eq!(app.current_source_path(), Some(next.as_path()));
        assert_eq!(
            folder_file_names(app.image_folder()),
            ["a-current.png", "b-next.png"]
        );
        assert_eq!(app.image_folder().current_index(), Some(1));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn app_folder_navigation_with_real_varied_images_wraps_and_keeps_current_on_broken_file() {
        let dir = unique_temp_dir("navigation-varied");
        fs::create_dir_all(&dir).expect("test dir");
        let current = dir.join("A-small.png");
        let wide = dir.join("b-wide.png");
        let tall = dir.join("c-tall.png");
        let large = dir.join("d-large.png");
        let broken = dir.join("z-broken.png");
        let note = dir.join("notes.txt");
        write_png_fixture(&current, 16, 16);
        write_png_fixture(&wide, 320, 80);
        write_png_fixture(&tall, 80, 320);
        write_png_fixture(&large, 2048, 1536);
        fs::write(&broken, b"not an image").expect("broken image");
        fs::write(&note, b"not scanned").expect("non-image note");

        let mut app = ViewerApp::new();
        app.load_image(&current).expect("load current image");

        assert_eq!(
            folder_file_names(app.image_folder()),
            [
                "A-small.png",
                "b-wide.png",
                "c-tall.png",
                "d-large.png",
                "z-broken.png"
            ]
        );

        let previous = app
            .navigate_image(ImageNavigationDirection::Previous)
            .expect("previous navigation");
        assert_eq!(previous, NavigationOutcome::Noop);
        assert_eq!(app.current_source_path(), Some(current.as_path()));
        let status = app.image_info_text().expect("navigation failure status");
        assert!(status.contains("The current image is kept"));
        assert!(status.contains("cannot be decoded"));

        let next = app
            .navigate_image(ImageNavigationDirection::Next)
            .expect("next navigation");
        assert_eq!(next, NavigationOutcome::Moved);
        assert_eq!(app.current_source_path(), Some(wide.as_path()));
        assert!(app
            .image_info_text()
            .expect("wide image status")
            .contains("320x80"));

        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next to tall"),
            NavigationOutcome::Moved
        );
        assert_eq!(app.current_source_path(), Some(tall.as_path()));
        assert!(app
            .image_info_text()
            .expect("tall image status")
            .contains("80x320"));

        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next to large"),
            NavigationOutcome::Moved
        );
        assert_eq!(app.current_source_path(), Some(large.as_path()));
        assert!(app
            .image_info_text()
            .expect("large image status")
            .contains("2048x1536"));

        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next to broken"),
            NavigationOutcome::Noop
        );
        assert_eq!(app.current_source_path(), Some(large.as_path()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn app_navigation_is_noop_for_single_real_image_folder() {
        let dir = unique_temp_dir("navigation-single");
        fs::create_dir_all(&dir).expect("test dir");
        let image = dir.join("only.png");
        write_png_fixture(&image, 24, 18);

        let mut app = ViewerApp::new();
        app.load_image(&image).expect("load only image");

        assert_eq!(app.image_folder().len(), 1);
        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Next)
                .expect("next noop"),
            NavigationOutcome::Noop
        );
        assert_eq!(
            app.navigate_image(ImageNavigationDirection::Previous)
                .expect("previous noop"),
            NavigationOutcome::Noop
        );
        assert_eq!(app.current_source_path(), Some(image.as_path()));

        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn open_decode_failure_uses_message_box_policy_without_status_text() {
        let mut app = ViewerApp::new();
        let request = app.begin_image_decode(PathBuf::from("missing.png"));
        let error = ViewerAppError::from(LoadImageError::FileAccess {
            path: PathBuf::from("missing.png"),
            source: io::Error::from_raw_os_error(2),
        });

        let outcome = app.finish_failed_initial_decode(request.generation(), &error);

        assert_eq!(outcome, DecodeFailurePresentation::MessageBox);
        assert_eq!(app.image_info_text(), None);
    }

    #[test]
    fn open_decode_failure_keeps_current_image() {
        let mut app = ViewerApp::new();
        let current_path = PathBuf::from("current.png");
        app.replace_loaded_image(
            test_image_from_path(current_path.clone(), 2, 1),
            ImageFolder::empty(),
        );
        let request = app.begin_image_decode(PathBuf::from("broken.png"));
        let error = ViewerAppError::from(LoadImageError::DecodeFailed {
            path: PathBuf::from("broken.png"),
            source: ImageError::IoError(io::Error::new(io::ErrorKind::UnexpectedEof, "short read")),
        });

        let outcome = app.finish_failed_initial_decode(request.generation(), &error);

        assert_eq!(outcome, DecodeFailurePresentation::MessageBox);
        assert_eq!(app.current_source_path(), Some(current_path.as_path()));
        let info = app.image_info_text().expect("current image info");
        assert!(info.contains("current.png"));
    }

    #[test]
    fn animated_image_exposes_timer_and_disables_full_resolution_decode() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());

        assert!(app.has_animation());
        assert_eq!(app.animation_timer_interval_ms(), Some(100));
        assert_eq!(app.begin_full_resolution_decode(), None);
    }

    #[test]
    fn animation_step_requests_frame_decode_then_applies_frame() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());

        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        assert_eq!(request.frame_index(), 1);
        assert_eq!(app.animation_timer_interval_ms(), None);

        let frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let applied = app.apply_animation_frame(
            request.generation(),
            request.frame_index(),
            request.path(),
            Some(request.file_version()),
            frame,
        );

        assert_eq!(applied, DecodeApplyOutcome::Applied);
        assert_eq!(
            app.display_rgba8().expect("display frame").pixels(),
            &[9, 9, 9, 255, 8, 8, 8, 255]
        );
        assert_eq!(app.animation_timer_interval_ms(), None);
    }

    #[test]
    fn animation_step_reuses_cached_frame_allocation() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());
        let cached_frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let cached_pixels = cached_frame.pixels().as_ptr();
        app.store_animation_frame_cache(1, cached_frame);

        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));

        assert_eq!(outcome, AnimationFrameOutcome::Updated);
        assert_eq!(
            app.display_rgba8()
                .expect("display frame")
                .pixels()
                .as_ptr(),
            cached_pixels
        );
    }

    #[test]
    fn animation_frame_result_for_changed_file_version_is_stale() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("same.gif", 2, 1), ImageFolder::empty());

        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };

        let changed_version = ImageFileVersion::new(0, UNIX_EPOCH + Duration::from_secs(1));
        let frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let applied = app.apply_animation_frame(
            request.generation(),
            request.frame_index(),
            request.path(),
            Some(changed_version),
            frame,
        );

        assert_eq!(applied, DecodeApplyOutcome::Stale);
        assert_eq!(
            app.display_rgba8().expect("display frame").pixels(),
            &[0, 0, 0, 255, 1, 1, 1, 255]
        );
    }

    #[test]
    fn pausing_while_animation_frame_decode_is_pending_discards_pending_transition() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());

        let outcome = app.handle_animation_timer();
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        assert_eq!(request.frame_index(), 1);
        assert_eq!(app.pending_animation_frame, Some(1));

        let paused = app.handle_animation_command(AnimationCommand::TogglePlayback);

        assert_eq!(paused, AnimationFrameOutcome::StateChanged);
        assert_eq!(app.pending_animation_frame, None);
        assert_eq!(app.pending_animation_state, None);
        let playback = app
            .current_animation_playback()
            .expect("animation playback");
        assert_eq!(playback.current_frame_index(), 0);
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Paused);

        let late_frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let late = app.apply_animation_frame(
            request.generation(),
            request.frame_index(),
            request.path(),
            Some(request.file_version()),
            late_frame,
        );

        assert_eq!(late, DecodeApplyOutcome::Stale);
        let playback = app
            .current_animation_playback()
            .expect("animation playback");
        assert_eq!(playback.current_frame_index(), 0);
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Paused);
        assert_eq!(
            app.display_rgba8().expect("display frame").pixels(),
            &[0, 0, 0, 255, 1, 1, 1, 255]
        );
    }

    #[test]
    fn failed_animation_timer_frame_decode_pauses_playback_and_stops_timer() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());

        let outcome = app.handle_animation_timer();
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        assert_eq!(request.frame_index(), 1);
        assert_eq!(app.animation_timer_interval_ms(), None);

        let failed = app.finish_failed_animation_frame_decode(
            request.generation(),
            request.frame_index(),
            request.path(),
            Some(request.file_version()),
        );

        assert_eq!(failed, DecodeApplyOutcome::Applied);
        assert_eq!(app.pending_animation_frame, None);
        assert_eq!(app.pending_animation_state, None);
        let playback = app
            .current_animation_playback()
            .expect("animation playback");
        assert_eq!(playback.current_frame_index(), 0);
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Paused);
        assert_eq!(app.animation_timer_interval_ms(), None);

        let resumed = app.handle_animation_command(AnimationCommand::TogglePlayback);

        assert_eq!(resumed, AnimationFrameOutcome::StateChanged);
        let playback = app
            .current_animation_playback()
            .expect("animation playback");
        assert_eq!(playback.current_frame_index(), 0);
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Playing);
        assert_eq!(app.animation_timer_interval_ms(), Some(100));
    }

    #[test]
    fn replacing_image_clears_pending_animation_frame_decode() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());
        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };

        app.replace_loaded_image(test_image("static.png", 2, 1), ImageFolder::empty());
        let frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let outcome = app.apply_animation_frame(
            request.generation(),
            request.frame_index(),
            request.path(),
            Some(request.file_version()),
            frame,
        );

        assert_eq!(outcome, DecodeApplyOutcome::Stale);
        assert!(!app.has_animation());
    }

    #[test]
    fn stale_animation_frame_result_for_previous_path_does_not_clear_new_pending_frame() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("first.gif", 2, 1), ImageFolder::empty());
        let first_outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let first_request = match first_outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected first animation outcome: {outcome:?}"),
        };

        app.replace_loaded_image(
            animated_test_image("second.gif", 2, 1),
            ImageFolder::empty(),
        );
        let second_outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let second_request = match second_outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected second animation outcome: {outcome:?}"),
        };
        assert_eq!(first_request.generation(), second_request.generation());
        assert_eq!(first_request.frame_index(), second_request.frame_index());

        let late_first_frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        let late_first = app.apply_animation_frame(
            first_request.generation(),
            first_request.frame_index(),
            first_request.path(),
            Some(first_request.file_version()),
            late_first_frame,
        );

        assert_eq!(late_first, DecodeApplyOutcome::Stale);
        assert_eq!(
            app.pending_animation_frame,
            Some(second_request.frame_index())
        );

        let late_first_failure = app.finish_failed_animation_frame_decode(
            first_request.generation(),
            first_request.frame_index(),
            first_request.path(),
            Some(first_request.file_version()),
        );
        assert_eq!(late_first_failure, DecodeApplyOutcome::Stale);
        assert_eq!(
            app.pending_animation_frame,
            Some(second_request.frame_index())
        );

        let second_frame = Rgba8Image::new(2, 1, vec![7, 7, 7, 255, 6, 6, 6, 255]);
        let applied = app.apply_animation_frame(
            second_request.generation(),
            second_request.frame_index(),
            second_request.path(),
            Some(second_request.file_version()),
            second_frame,
        );

        assert_eq!(applied, DecodeApplyOutcome::Applied);
        assert_eq!(
            app.display_rgba8().expect("display frame").pixels(),
            &[7, 7, 7, 255, 6, 6, 6, 255]
        );
    }

    #[test]
    fn replacing_image_clears_cached_animation_frames() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("anim.gif", 2, 1), ImageFolder::empty());
        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        let frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        assert_eq!(
            app.apply_animation_frame(
                request.generation(),
                request.frame_index(),
                request.path(),
                Some(request.file_version()),
                frame
            ),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(app.animation_frame_cache.len(), 1);

        app.replace_loaded_image(animated_test_image("next.gif", 2, 1), ImageFolder::empty());

        assert!(app.animation_frame_cache.is_empty());
    }

    #[test]
    fn replacing_animation_starts_from_new_first_frame_without_previous_state_leak() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(animated_test_image("first.gif", 2, 1), ImageFolder::empty());
        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        let second_frame = Rgba8Image::new(2, 1, vec![9, 9, 9, 255, 8, 8, 8, 255]);
        assert_eq!(
            app.apply_animation_frame(
                request.generation(),
                request.frame_index(),
                request.path(),
                Some(request.file_version()),
                second_frame
            ),
            DecodeApplyOutcome::Applied
        );
        assert_eq!(
            app.current_animation_playback()
                .expect("first animation playback")
                .current_frame_index(),
            1
        );
        assert_eq!(app.animation_frame_cache.len(), 1);

        app.replace_loaded_image(
            animated_test_image("second.gif", 2, 1),
            ImageFolder::empty(),
        );

        let playback = app
            .current_animation_playback()
            .expect("second animation playback");
        assert_eq!(playback.current_frame_index(), 0);
        assert_eq!(playback.completed_loops(), 0);
        assert_eq!(playback.playback_state(), AnimationPlaybackState::Playing);
        assert_eq!(app.animation_timer_interval_ms(), Some(100));
        assert_eq!(app.pending_animation_frame, None);
        assert_eq!(app.pending_animation_state, None);
        assert!(app.animation_frame_cache.is_empty());
        assert_eq!(
            app.display_rgba8().expect("second first frame").pixels(),
            &[0, 0, 0, 255, 1, 1, 1, 255]
        );
    }

    #[test]
    fn applying_animation_frame_invalidates_scaled_cache() {
        let mut app = ViewerApp::new();
        app.replace_loaded_image(
            animated_test_image("anim-large.gif", 1000, 800),
            ImageFolder::empty(),
        );
        app.render_rgba8(ViewportSize::from_client_size(500, 400))
            .expect("render image");
        assert!(app.render_cache.has_scaled_image());

        let outcome = app.handle_animation_command(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ));
        let request = match outcome {
            AnimationFrameOutcome::NeedsDecode(request) => request,
            outcome => panic!("unexpected animation outcome: {outcome:?}"),
        };
        let frame = test_rgba8_image(1000, 800);
        assert_eq!(
            app.apply_animation_frame(
                request.generation(),
                request.frame_index(),
                request.path(),
                Some(request.file_version()),
                frame
            ),
            DecodeApplyOutcome::Applied
        );

        assert!(!app.render_cache.has_scaled_image());
    }

    fn test_image(name: &str, width: u32, height: u32) -> LoadedImage {
        test_image_from_path(PathBuf::from(name), width, height)
    }

    fn test_image_from_path(path: PathBuf, width: u32, height: u32) -> LoadedImage {
        test_image_with_metadata(
            width,
            height,
            ImageMetadata::with_file_version(path, test_file_version(0), SupportedImageFormat::Png),
        )
    }

    fn animated_test_image(name: &str, width: u32, height: u32) -> LoadedImage {
        animated_test_image_with_delays(name, width, height, vec![100, 120])
    }

    fn animated_test_image_with_delays(
        name: &str,
        width: u32,
        height: u32,
        frame_delays_ms: Vec<u32>,
    ) -> LoadedImage {
        let animation = AnimationPlayback::new(frame_delays_ms, AnimationLoopPolicy::Infinite)
            .expect("animation playback");
        LoadedImage::from_animation(
            test_rgba8_image(width, height),
            ImageSize::new(width, height),
            ImageBufferKind::FullResolution,
            ImageMetadata::with_file_version(
                PathBuf::from(name),
                test_file_version(0),
                SupportedImageFormat::Gif,
            ),
            animation,
        )
    }

    fn preview_test_image(
        name: &str,
        preview_width: u32,
        preview_height: u32,
        source_width: u32,
        source_height: u32,
    ) -> LoadedImage {
        LoadedImage::from_preview(
            test_rgba8_image(preview_width, preview_height),
            ImageSize::new(source_width, source_height),
            ImageMetadata::with_file_version(
                PathBuf::from(name),
                test_file_version(0),
                SupportedImageFormat::Png,
            ),
        )
    }

    fn test_image_with_exif(
        name: &str,
        width: u32,
        height: u32,
        exif_orientation: ImageOrientation,
    ) -> LoadedImage {
        test_image_with_metadata(
            width,
            height,
            ImageMetadata::with_exif_orientation(
                PathBuf::from(name),
                0,
                SupportedImageFormat::Jpeg,
                exif_orientation,
            ),
        )
    }

    fn test_rgb8_image(name: &str, width: u32, height: u32) -> LoadedImage {
        test_rgb8_image_with_metadata(
            width,
            height,
            ImageMetadata::with_file_version(
                PathBuf::from(name),
                test_file_version(0),
                SupportedImageFormat::Jpeg,
            ),
        )
    }

    fn test_rgb8_image_with_exif(
        name: &str,
        width: u32,
        height: u32,
        exif_orientation: ImageOrientation,
    ) -> LoadedImage {
        test_rgb8_image_with_metadata(
            width,
            height,
            ImageMetadata::with_exif_orientation(
                PathBuf::from(name),
                0,
                SupportedImageFormat::Jpeg,
                exif_orientation,
            ),
        )
    }

    fn test_rgb8_image_with_metadata(
        width: u32,
        height: u32,
        metadata: ImageMetadata,
    ) -> LoadedImage {
        LoadedImage::from_pixels(PixelImage::from(test_rgb8_pixels(width, height)), metadata)
    }

    fn test_image_with_metadata(width: u32, height: u32, metadata: ImageMetadata) -> LoadedImage {
        LoadedImage::new(test_rgba8_image(width, height), metadata)
    }

    fn test_file_version(file_size: u64) -> ImageFileVersion {
        ImageFileVersion::new(file_size, UNIX_EPOCH)
    }

    fn test_rgba8_image(width: u32, height: u32) -> Rgba8Image {
        let pixels = (0..width * height)
            .flat_map(|index| {
                let value = index as u8;
                [value, value, value, 255]
            })
            .collect::<Vec<_>>();

        Rgba8Image::new(width, height, pixels)
    }

    fn test_rgb8_pixels(width: u32, height: u32) -> Rgb8Image {
        let pixels = (0..width * height)
            .flat_map(|index| {
                let value = index as u8;
                [value, value.wrapping_mul(3), value.wrapping_mul(7)]
            })
            .collect::<Vec<_>>();

        Rgb8Image::new(width, height, pixels)
    }

    fn write_png_fixture(path: &Path, width: u32, height: u32) {
        export_rgba8_image(
            path,
            &test_rgba8_image(width, height),
            ExportOptions::new(ExportFormat::Png, None),
        )
        .expect("write png fixture");
    }

    fn unique_temp_dir(name: &str) -> PathBuf {
        let nanos = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("system clock")
            .as_nanos();
        std::env::temp_dir().join(format!("j3pic-{name}-{}-{nanos}", std::process::id()))
    }

    fn folder_file_names(folder: &ImageFolder) -> Vec<String> {
        folder
            .paths()
            .iter()
            .filter_map(|path| {
                path.file_name()
                    .and_then(|name| name.to_str())
                    .map(str::to_owned)
            })
            .collect()
    }

    fn assert_render_rect_visible(rect: ImageDisplayRect, viewport: ViewportSize) {
        assert_axis_visible("x", rect.x(), rect.width(), viewport.width());
        assert_axis_visible("y", rect.y(), rect.height(), viewport.height());
    }

    fn assert_axis_visible(axis: &str, origin: i32, extent: i32, viewport_extent: u32) {
        let origin = i64::from(origin);
        let extent = i64::from(extent);
        let end = origin + extent;
        let viewport_extent = i64::from(viewport_extent);

        if extent >= viewport_extent {
            assert!(
                origin <= 0 && end >= viewport_extent,
                "{axis}-axis oversized rect [{origin}, {end}) does not cover viewport 0..{viewport_extent}"
            );
        } else {
            assert!(
                origin >= 0 && end <= viewport_extent,
                "{axis}-axis fitting rect [{origin}, {end}) is outside viewport 0..{viewport_extent}"
            );
        }
    }

    fn assert_approx_eq(left: f64, right: f64) {
        let diff = (left - right).abs();
        assert!(
            diff < 0.000_001,
            "left {left} differs from right {right} by {diff}"
        );
    }

    fn assert_rgb_close(actual: &[u8], expected: [u8; 3], tolerance: u8) {
        assert_eq!(actual.len(), expected.len());
        for (index, (actual, expected)) in actual.iter().zip(expected).enumerate() {
            let diff = actual.abs_diff(expected);
            assert!(
                diff <= tolerance,
                "rgb channel {index}: actual {actual} differs from expected {expected} by {diff}"
            );
        }
    }
}
