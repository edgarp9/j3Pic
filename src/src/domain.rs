use std::collections::{hash_map::DefaultHasher, HashMap};
use std::error::Error;
use std::fmt;
use std::hash::{Hash, Hasher};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::SystemTime;

pub const RGB8_BYTES_PER_PIXEL: usize = 3;
pub const RGBA8_BYTES_PER_PIXEL: usize = 4;
pub const BGRA8_BYTES_PER_PIXEL: usize = 4;
const BYTES_PER_MIB: usize = 1024 * 1024;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ViewportSize {
    width: u32,
    height: u32,
}

impl ViewportSize {
    pub const EMPTY: Self = Self {
        width: 0,
        height: 0,
    };

    pub fn from_client_size(width: i32, height: i32) -> Self {
        Self {
            width: width.max(0) as u32,
            height: height.max(0) as u32,
        }
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }
}

pub const MIN_ZOOM_SCALE: f64 = 0.05;
pub const MAX_ZOOM_SCALE: f64 = 32.0;
pub const ZOOM_STEP_FACTOR: f64 = 1.25;
pub const DEFAULT_ANIMATION_FRAME_DELAY_MS: u32 = 100;
pub const MIN_ANIMATION_FRAME_DELAY_MS: u32 = 10;
pub const MAX_ANIMATION_FRAME_DELAY_MS: u32 = 60_000;

const DEFAULT_LARGE_IMAGE_PIXEL_THRESHOLD: u64 = 24_000_000;
const DEFAULT_MAX_IMAGE_PIXELS: u64 = 160_000_000;
const DEFAULT_PREVIEW_MAX_PIXELS: u64 = 8_000_000;
const DEFAULT_PREVIEW_OVERSAMPLE: u32 = 2;
const DEFAULT_FALLBACK_VIEWPORT_WIDTH: u32 = 1920;
const DEFAULT_FALLBACK_VIEWPORT_HEIGHT: u32 = 1080;
const DEFAULT_MAX_TRANSIENT_DECODE_MIB: u32 = 640;
const DEFAULT_MAX_FULL_RESOLUTION_MIB: u32 = 128;
const DEFAULT_MAX_RESIDENT_MIB: u32 = 256;
const DEFAULT_MAX_CACHE_ENTRY_MIB: u32 = 128;
const DEFAULT_MAX_CACHE_ENTRIES: usize = 2;
const DEFAULT_MAX_ANIMATION_METADATA_FRAMES: usize = 10_000;
const DEFAULT_FULL_RESOLUTION_REQUEST_SCALE: f64 = 0.75;

pub(crate) const MIN_CONFIG_MIN_ZOOM_SCALE: f64 = 0.01;
pub(crate) const MAX_CONFIG_MIN_ZOOM_SCALE: f64 = 1.0;
pub(crate) const MIN_CONFIG_MAX_ZOOM_SCALE: f64 = 1.0;
pub(crate) const MAX_CONFIG_MAX_ZOOM_SCALE: f64 = 128.0;
pub(crate) const MIN_CONFIG_ZOOM_STEP_FACTOR: f64 = 1.01;
pub(crate) const MAX_CONFIG_ZOOM_STEP_FACTOR: f64 = 8.0;
pub(crate) const MIN_CONFIG_PIXEL_LIMIT: u64 = 1;
pub(crate) const MAX_CONFIG_IMAGE_PIXELS: u64 = 1_000_000_000;
pub(crate) const MIN_CONFIG_PREVIEW_OVERSAMPLE: u32 = 1;
pub(crate) const MAX_CONFIG_PREVIEW_OVERSAMPLE: u32 = 8;
const MIN_CONFIG_VIEWPORT_EDGE: u32 = 1;
const MAX_CONFIG_VIEWPORT_EDGE: u32 = 16_384;
pub(crate) const MIN_CONFIG_MEMORY_MIB: u32 = 1;
pub(crate) const MAX_CONFIG_MEMORY_MIB: u32 = 4096;
pub(crate) const MIN_CONFIG_CACHE_ENTRIES: usize = 0;
pub(crate) const MAX_CONFIG_CACHE_ENTRIES: usize = 64;
const MIN_CONFIG_ANIMATION_METADATA_FRAMES: usize = 1;
const MAX_CONFIG_ANIMATION_METADATA_FRAMES: usize = 1_000_000;
pub(crate) const MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE: f64 = MIN_ZOOM_SCALE;
pub(crate) const MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE: f64 = MAX_ZOOM_SCALE;
pub(crate) const MIN_CONFIG_ANIMATION_DELAY_MS: u32 = 1;
pub(crate) const MAX_CONFIG_ANIMATION_DELAY_MS: u32 = 600_000;
pub(crate) const DEFAULT_EXPORT_FILENAME_SUFFIX: &str = "-export";
pub(crate) const MAX_EXPORT_FILENAME_SUFFIX_CHARS: usize = 64;

#[derive(Debug, Clone, Copy, Default, PartialEq, Eq, Hash)]
pub enum UiLanguage {
    #[default]
    English,
    Korean,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageOpenErrorCategory {
    UnsupportedFormat,
    CorruptOrDecodingFailed,
    PermissionDenied,
    FileNotFoundOrMoved,
    FileLocked,
    ImageTooLargeOrOutOfMemory,
    UnknownIo,
    NotAFile,
    Canceled,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageLoadFailureStage {
    FormatDetection,
    FileIo,
    Decoder,
    PixelConversion,
    Win32Rendering,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ExportSaveErrorCategory {
    PermissionDenied,
    PathNotFound,
    FileLocked,
    EncodingFailed,
    ImageDataInvalid,
    ImageTooLargeOrOutOfMemory,
    UnknownIo,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NavigationFailurePolicy {
    KeepCurrentAndReport,
}

impl NavigationFailurePolicy {
    pub fn auto_skips_failed_files(self) -> bool {
        match self {
            Self::KeepCurrentAndReport => false,
        }
    }

    pub fn max_attempts_per_command(self) -> usize {
        match self {
            Self::KeepCurrentAndReport => 1,
        }
    }
}

pub const NAVIGATION_FAILURE_POLICY: NavigationFailurePolicy =
    NavigationFailurePolicy::KeepCurrentAndReport;

pub fn navigation_failure_policy() -> NavigationFailurePolicy {
    NAVIGATION_FAILURE_POLICY
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ImageSize {
    width: u32,
    height: u32,
}

impl ImageSize {
    pub fn new(width: u32, height: u32) -> Self {
        Self { width, height }
    }

    pub fn width(self) -> u32 {
        self.width
    }

    pub fn height(self) -> u32 {
        self.height
    }

    pub fn is_empty(self) -> bool {
        self.width == 0 || self.height == 0
    }

    pub fn pixel_count(self) -> Option<u64> {
        image_pixel_count(self)
    }

    pub fn rgba8_byte_len(self) -> Option<usize> {
        rgba8_byte_len(self)
    }

    pub fn pixel_byte_len(self, format: PixelFormat) -> Option<usize> {
        pixel_byte_len(self, format)
    }

    pub fn with_right_angle_rotation(self, degrees: u16) -> Self {
        if degrees % 180 == 90 {
            Self {
                width: self.height,
                height: self.width,
            }
        } else {
            self
        }
    }

    pub fn with_rotation(self, rotation: ImageRotation) -> Self {
        self.with_right_angle_rotation(rotation.degrees())
    }

    pub fn with_orientation(self, orientation: ImageOrientation) -> Self {
        if orientation.swaps_axes() {
            Self {
                width: self.height,
                height: self.width,
            }
        } else {
            self
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PixelFormat {
    Rgb8,
    Rgba8,
    Bgra8,
}

impl PixelFormat {
    pub fn bytes_per_pixel(self) -> usize {
        match self {
            Self::Rgb8 => RGB8_BYTES_PER_PIXEL,
            Self::Rgba8 => RGBA8_BYTES_PER_PIXEL,
            Self::Bgra8 => BGRA8_BYTES_PER_PIXEL,
        }
    }

    pub fn has_alpha(self) -> bool {
        match self {
            Self::Rgb8 => false,
            Self::Rgba8 | Self::Bgra8 => true,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ImageMemoryPolicy {
    large_image_pixel_threshold: u64,
    max_image_pixels: u64,
    preview_max_pixels: u64,
    preview_oversample: u32,
    fallback_viewport: ImageSize,
    max_transient_decode_bytes: usize,
    max_full_resolution_bytes: usize,
    max_resident_bytes: usize,
    max_cache_entry_bytes: usize,
    max_cache_entries: usize,
    max_animation_metadata_frames: usize,
    full_resolution_request_scale: f64,
}

impl ImageMemoryPolicy {
    pub const DEFAULT: Self = Self {
        large_image_pixel_threshold: DEFAULT_LARGE_IMAGE_PIXEL_THRESHOLD,
        max_image_pixels: DEFAULT_MAX_IMAGE_PIXELS,
        preview_max_pixels: DEFAULT_PREVIEW_MAX_PIXELS,
        preview_oversample: DEFAULT_PREVIEW_OVERSAMPLE,
        fallback_viewport: ImageSize {
            width: DEFAULT_FALLBACK_VIEWPORT_WIDTH,
            height: DEFAULT_FALLBACK_VIEWPORT_HEIGHT,
        },
        max_transient_decode_bytes: DEFAULT_MAX_TRANSIENT_DECODE_MIB as usize * BYTES_PER_MIB,
        max_full_resolution_bytes: DEFAULT_MAX_FULL_RESOLUTION_MIB as usize * BYTES_PER_MIB,
        max_resident_bytes: DEFAULT_MAX_RESIDENT_MIB as usize * BYTES_PER_MIB,
        max_cache_entry_bytes: DEFAULT_MAX_CACHE_ENTRY_MIB as usize * BYTES_PER_MIB,
        max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        max_animation_metadata_frames: DEFAULT_MAX_ANIMATION_METADATA_FRAMES,
        full_resolution_request_scale: DEFAULT_FULL_RESOLUTION_REQUEST_SCALE,
    };

    pub fn large_image_pixel_threshold(self) -> u64 {
        self.large_image_pixel_threshold
    }

    pub fn max_image_pixels(self) -> u64 {
        self.max_image_pixels
    }

    pub fn preview_max_pixels(self) -> u64 {
        self.preview_max_pixels
    }

    pub fn preview_oversample(self) -> u32 {
        self.preview_oversample
    }

    pub fn fallback_viewport(self) -> ImageSize {
        self.fallback_viewport
    }

    pub fn max_transient_decode_bytes(self) -> usize {
        self.max_transient_decode_bytes
    }

    pub fn max_full_resolution_bytes(self) -> usize {
        self.max_full_resolution_bytes
    }

    pub fn max_resident_bytes(self) -> usize {
        self.max_resident_bytes
    }

    pub fn max_cache_entry_bytes(self) -> usize {
        self.max_cache_entry_bytes
    }

    pub fn max_cache_entries(self) -> usize {
        self.max_cache_entries
    }

    pub fn max_animation_metadata_frames(self) -> usize {
        self.max_animation_metadata_frames
    }

    pub fn full_resolution_request_scale(self) -> f64 {
        self.full_resolution_request_scale
    }
}

impl Default for ImageMemoryPolicy {
    fn default() -> Self {
        Self::DEFAULT
    }
}

pub const DEFAULT_IMAGE_MEMORY_POLICY: ImageMemoryPolicy = ImageMemoryPolicy::DEFAULT;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageBufferKind {
    Preview,
    FullResolution,
}

pub fn image_pixel_count(size: ImageSize) -> Option<u64> {
    u64::from(size.width()).checked_mul(u64::from(size.height()))
}

pub fn rgba8_byte_len(size: ImageSize) -> Option<usize> {
    pixel_byte_len(size, PixelFormat::Rgba8)
}

pub fn pixel_byte_len(size: ImageSize, format: PixelFormat) -> Option<usize> {
    let pixels = image_pixel_count(size)?;
    let bytes = pixels.checked_mul(format.bytes_per_pixel() as u64)?;
    usize::try_from(bytes).ok()
}

pub fn is_large_image(size: ImageSize, policy: ImageMemoryPolicy) -> bool {
    image_pixel_count(size).is_none_or(|pixels| pixels > policy.large_image_pixel_threshold())
        || rgba8_byte_len(size).is_none_or(|bytes| bytes > policy.max_full_resolution_bytes())
}

pub fn is_image_too_large(size: ImageSize, policy: ImageMemoryPolicy) -> bool {
    image_pixel_count(size).is_none_or(|pixels| pixels > policy.max_image_pixels())
        || rgba8_byte_len(size).is_none_or(|bytes| bytes > policy.max_transient_decode_bytes())
}

pub fn should_retain_full_resolution(size: ImageSize, policy: ImageMemoryPolicy) -> bool {
    rgba8_byte_len(size).is_some_and(|bytes| bytes <= policy.max_full_resolution_bytes())
}

pub fn should_request_full_resolution_for_view(
    buffer_kind: ImageBufferKind,
    effective_scale: f64,
    source_size: ImageSize,
    policy: ImageMemoryPolicy,
) -> bool {
    buffer_kind == ImageBufferKind::Preview
        && effective_scale.is_finite()
        && effective_scale >= policy.full_resolution_request_scale()
        && should_retain_full_resolution(source_size, policy)
}

pub fn should_load_static_preview_first(
    format: SupportedImageFormat,
    source_size: ImageSize,
    viewport: ViewportSize,
    preview_size: ImageSize,
    policy: ImageMemoryPolicy,
) -> bool {
    if is_large_image(source_size, policy) {
        return true;
    }

    format == SupportedImageFormat::Jpeg
        && !viewport.is_empty()
        && is_meaningfully_smaller_preview(source_size, preview_size)
}

fn is_meaningfully_smaller_preview(source_size: ImageSize, preview_size: ImageSize) -> bool {
    let Some(source_pixels) = image_pixel_count(source_size) else {
        return false;
    };
    let Some(preview_pixels) = image_pixel_count(preview_size) else {
        return false;
    };
    preview_pixels > 0
        && preview_pixels < source_pixels
        && source_pixels >= preview_pixels.saturating_mul(2)
}

pub fn preview_size_for_viewport(
    source_size: ImageSize,
    viewport: ViewportSize,
    policy: ImageMemoryPolicy,
) -> ImageSize {
    if source_size.is_empty() {
        return source_size;
    }

    let bounds = preview_bounds_for_viewport(viewport, policy);
    let fitted = fit_size_within_bounds(source_size, bounds);
    fit_size_to_pixel_limit(fitted, policy.preview_max_pixels())
}

fn preview_bounds_for_viewport(viewport: ViewportSize, policy: ImageMemoryPolicy) -> ImageSize {
    if viewport.is_empty() {
        return policy.fallback_viewport();
    }

    ImageSize::new(
        viewport
            .width()
            .saturating_mul(policy.preview_oversample())
            .max(1),
        viewport
            .height()
            .saturating_mul(policy.preview_oversample())
            .max(1),
    )
}

fn fit_size_within_bounds(source_size: ImageSize, bounds: ImageSize) -> ImageSize {
    if bounds.is_empty() {
        return ImageSize::new(1, 1);
    }

    let width_scale = f64::from(bounds.width()) / f64::from(source_size.width());
    let height_scale = f64::from(bounds.height()) / f64::from(source_size.height());
    let scale = width_scale.min(height_scale).min(1.0);

    scaled_size(source_size, scale)
}

fn fit_size_to_pixel_limit(size: ImageSize, max_pixels: u64) -> ImageSize {
    let Some(pixel_count) = image_pixel_count(size) else {
        return ImageSize::new(1, 1);
    };
    if max_pixels == 0 {
        return ImageSize::new(1, 1);
    }
    if pixel_count <= max_pixels {
        return size;
    }

    let scale = ((max_pixels as f64) / (pixel_count as f64)).sqrt();
    scaled_size(size, scale)
}

fn scaled_size(size: ImageSize, scale: f64) -> ImageSize {
    if !scale.is_finite() || scale <= 0.0 {
        return ImageSize::new(1, 1);
    }

    ImageSize::new(
        (f64::from(size.width()) * scale)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32,
        (f64::from(size.height()) * scale)
            .round()
            .clamp(1.0, f64::from(u32::MAX)) as u32,
    )
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageCacheSlot {
    OrientedImage,
    ScaledImage,
    AnimationFrame { frame_index: Option<usize> },
    NavigationPreload { path_key: u64 },
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MemoryCacheEntry {
    slot: ImageCacheSlot,
    bytes: usize,
    last_used: u64,
}

impl MemoryCacheEntry {
    pub fn new(slot: ImageCacheSlot, bytes: usize, last_used: u64) -> Self {
        Self {
            slot,
            bytes,
            last_used,
        }
    }

    pub fn slot(self) -> ImageCacheSlot {
        self.slot
    }

    pub fn bytes(self) -> usize {
        self.bytes
    }

    pub fn last_used(self) -> u64 {
        self.last_used
    }
}

pub fn memory_cache_slots_to_evict(
    base_bytes: usize,
    entries: &[MemoryCacheEntry],
    policy: ImageMemoryPolicy,
) -> Vec<ImageCacheSlot> {
    let mut retained = entries.to_vec();
    retained.sort_by(|left, right| {
        left.last_used()
            .cmp(&right.last_used())
            .then_with(|| right.bytes().cmp(&left.bytes()))
    });

    let mut total_bytes = entries.iter().fold(base_bytes, |total, entry| {
        total.saturating_add(entry.bytes())
    });
    let mut entry_count = entries.len();
    let mut evicted = Vec::new();

    for entry in retained {
        let over_entry_limit = entry.bytes() > policy.max_cache_entry_bytes();
        let over_total_limit = total_bytes > policy.max_resident_bytes();
        let over_count_limit = entry_count > policy.max_cache_entries();

        if over_entry_limit || over_total_limit || over_count_limit {
            evicted.push(entry.slot());
            total_bytes = total_bytes.saturating_sub(entry.bytes());
            entry_count = entry_count.saturating_sub(1);
        }
    }

    evicted
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct DecodeGeneration(u64);

impl DecodeGeneration {
    pub const ZERO: Self = Self(0);

    pub fn next(self) -> Self {
        Self(self.0.wrapping_add(1))
    }

    pub fn value(self) -> u64 {
        self.0
    }
}

impl Default for DecodeGeneration {
    fn default() -> Self {
        Self::ZERO
    }
}

pub fn is_stale_decode_generation(
    active_generation: DecodeGeneration,
    result_generation: DecodeGeneration,
) -> bool {
    active_generation != result_generation
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationLoopPolicy {
    Infinite,
    Finite { repeat_count: u32 },
}

impl AnimationLoopPolicy {
    pub fn finite(repeat_count: u32) -> Self {
        if repeat_count == 0 {
            Self::Infinite
        } else {
            Self::Finite { repeat_count }
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationPlaybackState {
    Playing,
    Paused,
    Finished,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationFrameStepDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationPlayback {
    source_frame_delays_ms: Arc<[u32]>,
    frame_delays_ms: Arc<[u32]>,
    current_frame_index: usize,
    loop_policy: AnimationLoopPolicy,
    playback_state: AnimationPlaybackState,
    completed_loops: u32,
}

impl AnimationPlayback {
    pub fn new(frame_delays_ms: Vec<u32>, loop_policy: AnimationLoopPolicy) -> Option<Self> {
        Self::new_with_timing(
            frame_delays_ms,
            loop_policy,
            AnimationTimingSettings::default(),
        )
    }

    pub fn new_with_timing(
        frame_delays_ms: Vec<u32>,
        loop_policy: AnimationLoopPolicy,
        timing: AnimationTimingSettings,
    ) -> Option<Self> {
        if frame_delays_ms.len() < 2 {
            return None;
        }

        let normalized_frame_delays_ms: Vec<u32> = frame_delays_ms
            .iter()
            .copied()
            .map(|delay_ms| normalize_animation_frame_delay_ms_with_settings(delay_ms, timing))
            .collect();

        Some(Self {
            source_frame_delays_ms: frame_delays_ms.into(),
            frame_delays_ms: normalized_frame_delays_ms.into(),
            current_frame_index: 0,
            loop_policy,
            playback_state: AnimationPlaybackState::Playing,
            completed_loops: 0,
        })
    }

    pub fn frame_count(&self) -> usize {
        self.frame_delays_ms.len()
    }

    pub fn current_frame_index(&self) -> usize {
        self.current_frame_index
    }

    pub fn loop_policy(&self) -> AnimationLoopPolicy {
        self.loop_policy
    }

    pub fn playback_state(&self) -> AnimationPlaybackState {
        self.playback_state
    }

    pub fn completed_loops(&self) -> u32 {
        self.completed_loops
    }

    pub fn frame_delay_ms(&self, frame_index: usize) -> Option<u32> {
        self.frame_delays_ms.get(frame_index).copied()
    }

    pub fn frame_delays_ms(&self) -> &[u32] {
        self.frame_delays_ms.as_ref()
    }

    pub fn with_timing_settings(&self, timing: AnimationTimingSettings) -> Self {
        let mut next = self.clone();
        let frame_delays_ms: Vec<u32> = next
            .source_frame_delays_ms
            .iter()
            .copied()
            .map(|delay_ms| normalize_animation_frame_delay_ms_with_settings(delay_ms, timing))
            .collect();
        next.frame_delays_ms = frame_delays_ms.into();
        next
    }

    fn with_current_frame(
        &self,
        current_frame_index: usize,
        playback_state: AnimationPlaybackState,
        completed_loops: u32,
    ) -> Self {
        let mut next = self.clone();
        next.current_frame_index = current_frame_index.min(next.frame_count().saturating_sub(1));
        next.playback_state = playback_state;
        next.completed_loops = completed_loops;
        next
    }

    fn with_playback_state(&self, playback_state: AnimationPlaybackState) -> Self {
        self.with_current_frame(
            self.current_frame_index,
            playback_state,
            self.completed_loops,
        )
    }

    pub fn with_autoplay(&self, autoplay: bool) -> Self {
        let playback_state = if autoplay {
            AnimationPlaybackState::Playing
        } else {
            AnimationPlaybackState::Paused
        };
        self.with_current_frame(
            self.current_frame_index,
            playback_state,
            self.completed_loops,
        )
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AnimationPlaybackTransition {
    state: AnimationPlayback,
    frame_index: Option<usize>,
}

impl AnimationPlaybackTransition {
    pub fn state(&self) -> &AnimationPlayback {
        &self.state
    }

    pub fn frame_index(&self) -> Option<usize> {
        self.frame_index
    }

    pub fn into_parts(self) -> (AnimationPlayback, Option<usize>) {
        (self.state, self.frame_index)
    }
}

pub fn normalize_animation_frame_delay_ms(delay_ms: u32) -> u32 {
    normalize_animation_frame_delay_ms_with_settings(delay_ms, AnimationTimingSettings::default())
}

pub fn normalize_animation_frame_delay_ms_with_settings(
    delay_ms: u32,
    timing: AnimationTimingSettings,
) -> u32 {
    timing.normalize_frame_delay_ms(delay_ms)
}

pub fn animation_timer_interval_ms(playback: &AnimationPlayback) -> Option<u32> {
    if playback.playback_state() != AnimationPlaybackState::Playing || playback.frame_count() < 2 {
        return None;
    }

    playback.frame_delay_ms(playback.current_frame_index())
}

pub fn animation_state_after_timer_tick(
    playback: &AnimationPlayback,
) -> AnimationPlaybackTransition {
    if playback.playback_state() != AnimationPlaybackState::Playing || playback.frame_count() < 2 {
        return animation_transition(playback.clone(), None);
    }

    let current = playback.current_frame_index();
    if current + 1 < playback.frame_count() {
        let next_frame = current + 1;
        return animation_transition(
            playback.with_current_frame(
                next_frame,
                AnimationPlaybackState::Playing,
                playback.completed_loops(),
            ),
            Some(next_frame),
        );
    }

    match playback.loop_policy() {
        AnimationLoopPolicy::Infinite => {
            let completed_loops = playback.completed_loops().saturating_add(1);
            animation_transition(
                playback.with_current_frame(0, AnimationPlaybackState::Playing, completed_loops),
                Some(0),
            )
        }
        AnimationLoopPolicy::Finite { repeat_count } => {
            let completed_loops = playback.completed_loops().saturating_add(1);
            if completed_loops >= repeat_count {
                animation_transition(
                    playback.with_current_frame(
                        current,
                        AnimationPlaybackState::Finished,
                        completed_loops,
                    ),
                    None,
                )
            } else {
                animation_transition(
                    playback.with_current_frame(
                        0,
                        AnimationPlaybackState::Playing,
                        completed_loops,
                    ),
                    Some(0),
                )
            }
        }
    }
}

pub fn animation_state_after_toggle(playback: &AnimationPlayback) -> AnimationPlaybackTransition {
    match playback.playback_state() {
        AnimationPlaybackState::Playing => animation_transition(
            playback.with_playback_state(AnimationPlaybackState::Paused),
            None,
        ),
        AnimationPlaybackState::Paused => animation_transition(
            playback.with_playback_state(AnimationPlaybackState::Playing),
            None,
        ),
        AnimationPlaybackState::Finished => {
            let frame_index = (playback.current_frame_index() != 0).then_some(0);
            animation_transition(
                playback.with_current_frame(0, AnimationPlaybackState::Playing, 0),
                frame_index,
            )
        }
    }
}

pub fn animation_state_after_manual_step(
    playback: &AnimationPlayback,
    direction: AnimationFrameStepDirection,
) -> AnimationPlaybackTransition {
    let current = playback.current_frame_index();
    let next_frame = match direction {
        AnimationFrameStepDirection::Previous => current.saturating_sub(1),
        AnimationFrameStepDirection::Next => (current + 1).min(playback.frame_count() - 1),
    };
    let frame_index = (next_frame != current).then_some(next_frame);
    animation_transition(
        playback.with_current_frame(
            next_frame,
            AnimationPlaybackState::Paused,
            playback.completed_loops(),
        ),
        frame_index,
    )
}

pub fn animation_state_after_home(playback: &AnimationPlayback) -> AnimationPlaybackTransition {
    let frame_index = (playback.current_frame_index() != 0).then_some(0);
    animation_transition(
        playback.with_current_frame(
            0,
            AnimationPlaybackState::Paused,
            playback.completed_loops(),
        ),
        frame_index,
    )
}

fn animation_transition(
    state: AnimationPlayback,
    frame_index: Option<usize>,
) -> AnimationPlaybackTransition {
    AnimationPlaybackTransition { state, frame_index }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageRotation {
    Degrees0,
    Degrees90,
    Degrees180,
    Degrees270,
}

impl ImageRotation {
    pub const ZERO: Self = Self::Degrees0;

    pub fn degrees(self) -> u16 {
        match self {
            Self::Degrees0 => 0,
            Self::Degrees90 => 90,
            Self::Degrees180 => 180,
            Self::Degrees270 => 270,
        }
    }

    pub fn clockwise(self) -> Self {
        match self {
            Self::Degrees0 => Self::Degrees90,
            Self::Degrees90 => Self::Degrees180,
            Self::Degrees180 => Self::Degrees270,
            Self::Degrees270 => Self::Degrees0,
        }
    }

    pub fn counter_clockwise(self) -> Self {
        match self {
            Self::Degrees0 => Self::Degrees270,
            Self::Degrees90 => Self::Degrees0,
            Self::Degrees180 => Self::Degrees90,
            Self::Degrees270 => Self::Degrees180,
        }
    }

    pub fn is_identity(self) -> bool {
        self == Self::Degrees0
    }

    pub fn quarter_turns_clockwise(self) -> u8 {
        match self {
            Self::Degrees0 => 0,
            Self::Degrees90 => 1,
            Self::Degrees180 => 2,
            Self::Degrees270 => 3,
        }
    }
}

impl Default for ImageRotation {
    fn default() -> Self {
        Self::ZERO
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ImageOrientation {
    Normal,
    FlipHorizontal,
    Rotate180,
    FlipVertical,
    Rotate90FlipHorizontal,
    Rotate90,
    Rotate270FlipHorizontal,
    Rotate270,
}

impl ImageOrientation {
    pub const NORMAL: Self = Self::Normal;

    pub fn from_exif_value(value: u8) -> Option<Self> {
        match value {
            1 => Some(Self::Normal),
            2 => Some(Self::FlipHorizontal),
            3 => Some(Self::Rotate180),
            4 => Some(Self::FlipVertical),
            5 => Some(Self::Rotate90FlipHorizontal),
            6 => Some(Self::Rotate90),
            7 => Some(Self::Rotate270FlipHorizontal),
            8 => Some(Self::Rotate270),
            0 | 9.. => None,
        }
    }

    pub fn exif_value(self) -> u8 {
        match self {
            Self::Normal => 1,
            Self::FlipHorizontal => 2,
            Self::Rotate180 => 3,
            Self::FlipVertical => 4,
            Self::Rotate90FlipHorizontal => 5,
            Self::Rotate90 => 6,
            Self::Rotate270FlipHorizontal => 7,
            Self::Rotate270 => 8,
        }
    }

    pub fn is_identity(self) -> bool {
        self == Self::Normal
    }

    pub fn swaps_axes(self) -> bool {
        matches!(
            self,
            Self::Rotate90
                | Self::Rotate270
                | Self::Rotate90FlipHorizontal
                | Self::Rotate270FlipHorizontal
        )
    }

    pub fn then_rotation(self, rotation: ImageRotation) -> Self {
        let mut orientation = self;
        for _ in 0..rotation.quarter_turns_clockwise() {
            orientation = orientation.then_clockwise();
        }
        orientation
    }

    fn then_clockwise(self) -> Self {
        match self {
            Self::Normal => Self::Rotate90,
            Self::Rotate90 => Self::Rotate180,
            Self::Rotate180 => Self::Rotate270,
            Self::Rotate270 => Self::Normal,
            Self::FlipHorizontal => Self::Rotate270FlipHorizontal,
            Self::Rotate270FlipHorizontal => Self::FlipVertical,
            Self::FlipVertical => Self::Rotate90FlipHorizontal,
            Self::Rotate90FlipHorizontal => Self::FlipHorizontal,
        }
    }
}

impl Default for ImageOrientation {
    fn default() -> Self {
        Self::NORMAL
    }
}

pub fn display_orientation(
    exif_orientation: ImageOrientation,
    user_rotation: ImageRotation,
) -> ImageOrientation {
    exif_orientation.then_rotation(user_rotation)
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ViewMode {
    FitToWindow,
    ActualSize,
    ManualZoom,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct RenderReadySpec {
    viewport: ViewportSize,
    view_mode: ViewMode,
    scaling_quality: ScalingQuality,
}

impl RenderReadySpec {
    pub fn new(
        viewport: ViewportSize,
        view_mode: ViewMode,
        scaling_quality: ScalingQuality,
    ) -> Self {
        Self {
            viewport,
            view_mode,
            scaling_quality,
        }
    }

    pub fn viewport(self) -> ViewportSize {
        self.viewport
    }

    pub fn view_mode(self) -> ViewMode {
        self.view_mode
    }

    pub fn scaling_quality(self) -> ScalingQuality {
        self.scaling_quality
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewportPoint {
    x: f64,
    y: f64,
}

impl ViewportPoint {
    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn from_client_position(x: i32, y: i32) -> Self {
        Self {
            x: f64::from(x),
            y: f64::from(y),
        }
    }

    pub fn center(viewport: ViewportSize) -> Option<Self> {
        if viewport.is_empty() {
            None
        } else {
            Some(Self {
                x: f64::from(viewport.width()) / 2.0,
                y: f64::from(viewport.height()) / 2.0,
            })
        }
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewOffset {
    x: f64,
    y: f64,
}

impl ViewOffset {
    pub const ZERO: Self = Self { x: 0.0, y: 0.0 };

    pub fn new(x: f64, y: f64) -> Self {
        Self { x, y }
    }

    pub fn x(self) -> f64 {
        self.x
    }

    pub fn y(self) -> f64 {
        self.y
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageDisplayRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl ImageDisplayRect {
    pub fn x(self) -> i32 {
        self.x
    }

    pub fn y(self) -> i32 {
        self.y
    }

    pub fn width(self) -> i32 {
        self.width
    }

    pub fn height(self) -> i32 {
        self.height
    }

    pub fn size(self) -> Option<ImageSize> {
        let width = u32::try_from(self.width).ok()?;
        let height = u32::try_from(self.height).ok()?;
        if width == 0 || height == 0 {
            None
        } else {
            Some(ImageSize::new(width, height))
        }
    }
}

pub const DEFAULT_SCALING_QUALITY: ScalingQuality = ScalingQuality::Balanced;
pub const DEFAULT_EXPORT_QUALITY: u8 = 90;

pub(crate) const MIN_EXPORT_QUALITY: u8 = 1;
pub(crate) const MAX_EXPORT_QUALITY: u8 = 100;
const MIN_SAVED_WINDOW_WIDTH: i32 = 320;
const MIN_SAVED_WINDOW_HEIGHT: i32 = 240;
const MAX_SAVED_WINDOW_EXTENT: i32 = 32_767;
const MIN_SAVED_WINDOW_ORIGIN: i32 = -32_768;
const MAX_SAVED_WINDOW_ORIGIN: i32 = 32_767;

const SCALING_CACHE_TARGET_BUCKET_PIXELS: u32 = 16;
const BALANCED_SOFTWARE_DOWNSCALE_THRESHOLD: f64 = 0.98;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum ScalingQuality {
    Nearest,
    Balanced,
    HighQuality,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct WindowBounds {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

impl WindowBounds {
    pub fn new(x: i32, y: i32, width: i32, height: i32) -> Option<Self> {
        if !valid_window_origin(x)
            || !valid_window_origin(y)
            || !(MIN_SAVED_WINDOW_WIDTH..=MAX_SAVED_WINDOW_EXTENT).contains(&width)
            || !(MIN_SAVED_WINDOW_HEIGHT..=MAX_SAVED_WINDOW_EXTENT).contains(&height)
        {
            return None;
        }

        Some(Self {
            x,
            y,
            width,
            height,
        })
    }

    pub fn x(self) -> i32 {
        self.x
    }

    pub fn y(self) -> i32 {
        self.y
    }

    pub fn width(self) -> i32 {
        self.width
    }

    pub fn height(self) -> i32 {
        self.height
    }
}

fn valid_window_origin(value: i32) -> bool {
    (MIN_SAVED_WINDOW_ORIGIN..=MAX_SAVED_WINDOW_ORIGIN).contains(&value)
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ZoomSettings {
    min_zoom_scale: f64,
    max_zoom_scale: f64,
    zoom_step_factor: f64,
}

impl ZoomSettings {
    pub const DEFAULT: Self = Self {
        min_zoom_scale: MIN_ZOOM_SCALE,
        max_zoom_scale: MAX_ZOOM_SCALE,
        zoom_step_factor: ZOOM_STEP_FACTOR,
    };

    pub fn new(min_zoom_scale: f64, max_zoom_scale: f64, zoom_step_factor: f64) -> Self {
        let mut settings = Self::DEFAULT;
        settings.set_min_zoom_scale(min_zoom_scale);
        settings.set_max_zoom_scale(max_zoom_scale);
        settings.set_zoom_step_factor(zoom_step_factor);
        settings
    }

    pub fn min_zoom_scale(self) -> f64 {
        self.min_zoom_scale
    }

    pub fn set_min_zoom_scale(&mut self, min_zoom_scale: f64) {
        self.min_zoom_scale = sanitize_config_f64(
            min_zoom_scale,
            MIN_CONFIG_MIN_ZOOM_SCALE,
            MAX_CONFIG_MIN_ZOOM_SCALE,
            MIN_ZOOM_SCALE,
        );
    }

    pub fn max_zoom_scale(self) -> f64 {
        self.max_zoom_scale
    }

    pub fn set_max_zoom_scale(&mut self, max_zoom_scale: f64) {
        self.max_zoom_scale = sanitize_config_f64(
            max_zoom_scale,
            MIN_CONFIG_MAX_ZOOM_SCALE,
            MAX_CONFIG_MAX_ZOOM_SCALE,
            MAX_ZOOM_SCALE,
        );
    }

    pub fn zoom_step_factor(self) -> f64 {
        self.zoom_step_factor
    }

    pub fn set_zoom_step_factor(&mut self, zoom_step_factor: f64) {
        self.zoom_step_factor = sanitize_config_f64(
            zoom_step_factor,
            MIN_CONFIG_ZOOM_STEP_FACTOR,
            MAX_CONFIG_ZOOM_STEP_FACTOR,
            ZOOM_STEP_FACTOR,
        );
    }
}

impl Default for ZoomSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Eq for ZoomSettings {}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct MemoryPolicySettings {
    large_image_pixel_threshold: u64,
    max_image_pixels: u64,
    preview_max_pixels: u64,
    preview_oversample: u32,
    fallback_viewport_width: u32,
    fallback_viewport_height: u32,
    max_transient_decode_mib: u32,
    max_full_resolution_mib: u32,
    max_resident_mib: u32,
    max_cache_entry_mib: u32,
    max_cache_entries: usize,
    max_animation_metadata_frames: usize,
    full_resolution_request_scale: f64,
}

impl MemoryPolicySettings {
    pub const DEFAULT: Self = Self {
        large_image_pixel_threshold: DEFAULT_LARGE_IMAGE_PIXEL_THRESHOLD,
        max_image_pixels: DEFAULT_MAX_IMAGE_PIXELS,
        preview_max_pixels: DEFAULT_PREVIEW_MAX_PIXELS,
        preview_oversample: DEFAULT_PREVIEW_OVERSAMPLE,
        fallback_viewport_width: DEFAULT_FALLBACK_VIEWPORT_WIDTH,
        fallback_viewport_height: DEFAULT_FALLBACK_VIEWPORT_HEIGHT,
        max_transient_decode_mib: DEFAULT_MAX_TRANSIENT_DECODE_MIB,
        max_full_resolution_mib: DEFAULT_MAX_FULL_RESOLUTION_MIB,
        max_resident_mib: DEFAULT_MAX_RESIDENT_MIB,
        max_cache_entry_mib: DEFAULT_MAX_CACHE_ENTRY_MIB,
        max_cache_entries: DEFAULT_MAX_CACHE_ENTRIES,
        max_animation_metadata_frames: DEFAULT_MAX_ANIMATION_METADATA_FRAMES,
        full_resolution_request_scale: DEFAULT_FULL_RESOLUTION_REQUEST_SCALE,
    };

    pub fn large_image_pixel_threshold(self) -> u64 {
        self.large_image_pixel_threshold
    }

    pub fn set_large_image_pixel_threshold(&mut self, threshold: u64) {
        self.large_image_pixel_threshold =
            threshold.clamp(MIN_CONFIG_PIXEL_LIMIT, MAX_CONFIG_IMAGE_PIXELS);
        self.normalize_relationships();
    }

    pub fn max_image_pixels(self) -> u64 {
        self.max_image_pixels
    }

    pub fn set_max_image_pixels(&mut self, max_image_pixels: u64) {
        self.max_image_pixels =
            max_image_pixels.clamp(MIN_CONFIG_PIXEL_LIMIT, MAX_CONFIG_IMAGE_PIXELS);
        self.normalize_relationships();
    }

    pub fn preview_max_pixels(self) -> u64 {
        self.preview_max_pixels
    }

    pub fn set_preview_max_pixels(&mut self, preview_max_pixels: u64) {
        self.preview_max_pixels =
            preview_max_pixels.clamp(MIN_CONFIG_PIXEL_LIMIT, MAX_CONFIG_IMAGE_PIXELS);
        self.normalize_relationships();
    }

    pub fn preview_oversample(self) -> u32 {
        self.preview_oversample
    }

    pub fn set_preview_oversample(&mut self, preview_oversample: u32) {
        self.preview_oversample =
            preview_oversample.clamp(MIN_CONFIG_PREVIEW_OVERSAMPLE, MAX_CONFIG_PREVIEW_OVERSAMPLE);
    }

    pub fn fallback_viewport_width(self) -> u32 {
        self.fallback_viewport_width
    }

    pub fn set_fallback_viewport_width(&mut self, width: u32) {
        self.fallback_viewport_width =
            width.clamp(MIN_CONFIG_VIEWPORT_EDGE, MAX_CONFIG_VIEWPORT_EDGE);
    }

    pub fn fallback_viewport_height(self) -> u32 {
        self.fallback_viewport_height
    }

    pub fn set_fallback_viewport_height(&mut self, height: u32) {
        self.fallback_viewport_height =
            height.clamp(MIN_CONFIG_VIEWPORT_EDGE, MAX_CONFIG_VIEWPORT_EDGE);
    }

    pub fn max_transient_decode_mib(self) -> u32 {
        self.max_transient_decode_mib
    }

    pub fn set_max_transient_decode_mib(&mut self, max_mib: u32) {
        self.max_transient_decode_mib = max_mib.clamp(MIN_CONFIG_MEMORY_MIB, MAX_CONFIG_MEMORY_MIB);
        self.normalize_relationships();
    }

    pub fn max_full_resolution_mib(self) -> u32 {
        self.max_full_resolution_mib
    }

    pub fn set_max_full_resolution_mib(&mut self, max_mib: u32) {
        self.max_full_resolution_mib = max_mib.clamp(MIN_CONFIG_MEMORY_MIB, MAX_CONFIG_MEMORY_MIB);
        self.normalize_relationships();
    }

    pub fn max_resident_mib(self) -> u32 {
        self.max_resident_mib
    }

    pub fn set_max_resident_mib(&mut self, max_mib: u32) {
        self.max_resident_mib = max_mib.clamp(MIN_CONFIG_MEMORY_MIB, MAX_CONFIG_MEMORY_MIB);
        self.normalize_relationships();
    }

    pub fn max_cache_entry_mib(self) -> u32 {
        self.max_cache_entry_mib
    }

    pub fn set_max_cache_entry_mib(&mut self, max_mib: u32) {
        self.max_cache_entry_mib = max_mib.clamp(MIN_CONFIG_MEMORY_MIB, MAX_CONFIG_MEMORY_MIB);
        self.normalize_relationships();
    }

    pub fn max_cache_entries(self) -> usize {
        self.max_cache_entries
    }

    pub fn set_max_cache_entries(&mut self, max_cache_entries: usize) {
        self.max_cache_entries =
            max_cache_entries.clamp(MIN_CONFIG_CACHE_ENTRIES, MAX_CONFIG_CACHE_ENTRIES);
    }

    pub fn max_animation_metadata_frames(self) -> usize {
        self.max_animation_metadata_frames
    }

    pub fn set_max_animation_metadata_frames(&mut self, max_frames: usize) {
        self.max_animation_metadata_frames = max_frames.clamp(
            MIN_CONFIG_ANIMATION_METADATA_FRAMES,
            MAX_CONFIG_ANIMATION_METADATA_FRAMES,
        );
    }

    pub fn full_resolution_request_scale(self) -> f64 {
        self.full_resolution_request_scale
    }

    pub fn set_full_resolution_request_scale(&mut self, scale: f64) {
        self.full_resolution_request_scale = sanitize_config_f64(
            scale,
            MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
            MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
            DEFAULT_FULL_RESOLUTION_REQUEST_SCALE,
        );
    }

    pub fn image_memory_policy(self) -> ImageMemoryPolicy {
        ImageMemoryPolicy {
            large_image_pixel_threshold: self.large_image_pixel_threshold,
            max_image_pixels: self.max_image_pixels,
            preview_max_pixels: self.preview_max_pixels,
            preview_oversample: self.preview_oversample,
            fallback_viewport: ImageSize::new(
                self.fallback_viewport_width,
                self.fallback_viewport_height,
            ),
            max_transient_decode_bytes: mib_to_bytes(self.max_transient_decode_mib),
            max_full_resolution_bytes: mib_to_bytes(self.max_full_resolution_mib),
            max_resident_bytes: mib_to_bytes(self.max_resident_mib),
            max_cache_entry_bytes: mib_to_bytes(self.max_cache_entry_mib),
            max_cache_entries: self.max_cache_entries,
            max_animation_metadata_frames: self.max_animation_metadata_frames,
            full_resolution_request_scale: self.full_resolution_request_scale,
        }
    }

    fn normalize_relationships(&mut self) {
        self.large_image_pixel_threshold =
            self.large_image_pixel_threshold.min(self.max_image_pixels);
        self.preview_max_pixels = self.preview_max_pixels.min(self.max_image_pixels);
        self.max_full_resolution_mib = self
            .max_full_resolution_mib
            .min(self.max_transient_decode_mib);
        self.max_cache_entry_mib = self.max_cache_entry_mib.min(self.max_resident_mib);
    }
}

impl Default for MemoryPolicySettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

impl Eq for MemoryPolicySettings {}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct AnimationTimingSettings {
    default_frame_delay_ms: u32,
    min_frame_delay_ms: u32,
    max_frame_delay_ms: u32,
}

impl AnimationTimingSettings {
    pub const DEFAULT: Self = Self {
        default_frame_delay_ms: DEFAULT_ANIMATION_FRAME_DELAY_MS,
        min_frame_delay_ms: MIN_ANIMATION_FRAME_DELAY_MS,
        max_frame_delay_ms: MAX_ANIMATION_FRAME_DELAY_MS,
    };

    pub fn default_frame_delay_ms(self) -> u32 {
        self.default_frame_delay_ms
    }

    pub fn set_default_frame_delay_ms(&mut self, delay_ms: u32) {
        self.default_frame_delay_ms =
            delay_ms.clamp(MIN_CONFIG_ANIMATION_DELAY_MS, MAX_CONFIG_ANIMATION_DELAY_MS);
        self.normalize_relationships();
    }

    pub fn min_frame_delay_ms(self) -> u32 {
        self.min_frame_delay_ms
    }

    pub fn set_min_frame_delay_ms(&mut self, delay_ms: u32) {
        self.min_frame_delay_ms =
            delay_ms.clamp(MIN_CONFIG_ANIMATION_DELAY_MS, MAX_CONFIG_ANIMATION_DELAY_MS);
        self.normalize_relationships();
    }

    pub fn max_frame_delay_ms(self) -> u32 {
        self.max_frame_delay_ms
    }

    pub fn set_max_frame_delay_ms(&mut self, delay_ms: u32) {
        self.max_frame_delay_ms =
            delay_ms.clamp(MIN_CONFIG_ANIMATION_DELAY_MS, MAX_CONFIG_ANIMATION_DELAY_MS);
        self.normalize_relationships();
    }

    pub fn normalize_frame_delay_ms(self, delay_ms: u32) -> u32 {
        if delay_ms == 0 {
            self.default_frame_delay_ms
        } else {
            delay_ms.clamp(self.min_frame_delay_ms, self.max_frame_delay_ms)
        }
    }

    fn normalize_relationships(&mut self) {
        if self.min_frame_delay_ms > self.max_frame_delay_ms {
            self.max_frame_delay_ms = self.min_frame_delay_ms;
        }
        self.default_frame_delay_ms = self
            .default_frame_delay_ms
            .clamp(self.min_frame_delay_ms, self.max_frame_delay_ms);
    }
}

impl Default for AnimationTimingSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NavigationSettings {
    wrap_navigation: bool,
    auto_skip_failed_navigation: bool,
    max_navigation_attempts_per_command: usize,
}

impl NavigationSettings {
    pub const DEFAULT: Self = Self {
        wrap_navigation: true,
        auto_skip_failed_navigation: false,
        max_navigation_attempts_per_command: 1,
    };

    pub fn wrap_navigation(self) -> bool {
        self.wrap_navigation
    }

    pub fn set_wrap_navigation(&mut self, wrap_navigation: bool) {
        self.wrap_navigation = wrap_navigation;
    }

    pub fn auto_skip_failed_navigation(self) -> bool {
        self.auto_skip_failed_navigation
    }

    pub fn set_auto_skip_failed_navigation(&mut self, auto_skip_failed_navigation: bool) {
        self.auto_skip_failed_navigation = auto_skip_failed_navigation;
    }

    pub fn max_navigation_attempts_per_command(self) -> usize {
        self.max_navigation_attempts_per_command
    }

    pub fn set_max_navigation_attempts_per_command(&mut self, max_attempts: usize) {
        self.max_navigation_attempts_per_command = max_attempts.clamp(1, 100);
    }
}

impl Default for NavigationSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

mod export_settings {
    use super::{
        default_export_format_for_source_format, export_suffix::sanitize_export_filename_suffix,
        ExportFormat, SupportedImageFormat, DEFAULT_EXPORT_FILENAME_SUFFIX,
    };

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum DefaultExportFormatPolicy {
        Source,
        Png,
        Jpeg,
        Bmp,
        Webp,
        Ico,
    }

    impl DefaultExportFormatPolicy {
        pub fn export_format_for_source(self, source_format: SupportedImageFormat) -> ExportFormat {
            match self {
                Self::Source => default_export_format_for_source_format(source_format),
                Self::Png => ExportFormat::Png,
                Self::Jpeg => ExportFormat::Jpeg,
                Self::Bmp => ExportFormat::Bmp,
                Self::Webp => ExportFormat::Webp,
                Self::Ico => ExportFormat::Ico,
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct RgbColor {
        red: u8,
        green: u8,
        blue: u8,
    }

    impl RgbColor {
        pub const WHITE: Self = Self {
            red: 255,
            green: 255,
            blue: 255,
        };

        pub const fn new(red: u8, green: u8, blue: u8) -> Self {
            Self { red, green, blue }
        }

        pub fn red(self) -> u8 {
            self.red
        }

        pub fn green(self) -> u8 {
            self.green
        }

        pub fn blue(self) -> u8 {
            self.blue
        }
    }

    #[derive(Debug, Clone, PartialEq, Eq)]
    pub struct ExportSettings {
        default_export_format_policy: DefaultExportFormatPolicy,
        export_filename_suffix: String,
        jpeg_alpha_background_rgb: RgbColor,
    }

    impl ExportSettings {
        pub fn new(
            default_export_format_policy: DefaultExportFormatPolicy,
            export_filename_suffix: impl Into<String>,
            jpeg_alpha_background_rgb: RgbColor,
        ) -> Self {
            let mut settings = Self::default();
            settings.set_default_export_format_policy(default_export_format_policy);
            settings.set_export_filename_suffix(export_filename_suffix);
            settings.set_jpeg_alpha_background_rgb(jpeg_alpha_background_rgb);
            settings
        }

        pub fn default_export_format_policy(&self) -> DefaultExportFormatPolicy {
            self.default_export_format_policy
        }

        pub fn set_default_export_format_policy(&mut self, policy: DefaultExportFormatPolicy) {
            self.default_export_format_policy = policy;
        }

        pub fn export_filename_suffix(&self) -> &str {
            &self.export_filename_suffix
        }

        pub fn set_export_filename_suffix(&mut self, suffix: impl Into<String>) {
            self.export_filename_suffix = sanitize_export_filename_suffix(suffix.into());
        }

        pub fn jpeg_alpha_background_rgb(&self) -> RgbColor {
            self.jpeg_alpha_background_rgb
        }

        pub fn set_jpeg_alpha_background_rgb(&mut self, color: RgbColor) {
            self.jpeg_alpha_background_rgb = color;
        }
    }

    impl Default for ExportSettings {
        fn default() -> Self {
            Self {
                default_export_format_policy: DefaultExportFormatPolicy::Png,
                export_filename_suffix: DEFAULT_EXPORT_FILENAME_SUFFIX.to_owned(),
                jpeg_alpha_background_rgb: RgbColor::WHITE,
            }
        }
    }
}

pub use export_settings::{DefaultExportFormatPolicy, ExportSettings, RgbColor};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct StatusUiSettings {
    show_status_bar: bool,
    detailed_status_text: bool,
}

impl StatusUiSettings {
    pub const DEFAULT: Self = Self {
        show_status_bar: true,
        detailed_status_text: true,
    };

    pub fn show_status_bar(self) -> bool {
        self.show_status_bar
    }

    pub fn set_show_status_bar(&mut self, show_status_bar: bool) {
        self.show_status_bar = show_status_bar;
    }

    pub fn detailed_status_text(self) -> bool {
        self.detailed_status_text
    }

    pub fn set_detailed_status_text(&mut self, detailed_status_text: bool) {
        self.detailed_status_text = detailed_status_text;
    }
}

impl Default for StatusUiSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MouseShortcut {
    MouseWheel,
    CtrlMouseWheel,
    LeftButtonDrag,
    CtrlLeftButtonDrag,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct InteractionSettings {
    zoom_shortcut: MouseShortcut,
    image_navigation_shortcut: MouseShortcut,
    image_pan_shortcut: MouseShortcut,
    window_move_shortcut: MouseShortcut,
}

impl InteractionSettings {
    pub const DEFAULT: Self = Self {
        zoom_shortcut: MouseShortcut::CtrlMouseWheel,
        image_navigation_shortcut: MouseShortcut::MouseWheel,
        image_pan_shortcut: MouseShortcut::CtrlLeftButtonDrag,
        window_move_shortcut: MouseShortcut::LeftButtonDrag,
    };

    pub fn zoom_shortcut(self) -> MouseShortcut {
        self.zoom_shortcut
    }

    pub fn set_zoom_shortcut(&mut self, shortcut: MouseShortcut) {
        self.zoom_shortcut = shortcut;
    }

    pub fn image_navigation_shortcut(self) -> MouseShortcut {
        self.image_navigation_shortcut
    }

    pub fn set_image_navigation_shortcut(&mut self, shortcut: MouseShortcut) {
        self.image_navigation_shortcut = shortcut;
    }

    pub fn image_pan_shortcut(self) -> MouseShortcut {
        self.image_pan_shortcut
    }

    pub fn set_image_pan_shortcut(&mut self, shortcut: MouseShortcut) {
        self.image_pan_shortcut = shortcut;
    }

    pub fn window_move_shortcut(self) -> MouseShortcut {
        self.window_move_shortcut
    }

    pub fn set_window_move_shortcut(&mut self, shortcut: MouseShortcut) {
        self.window_move_shortcut = shortcut;
    }
}

impl Default for InteractionSettings {
    fn default() -> Self {
        Self::DEFAULT
    }
}

#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct UserSettings {
    zoom: ZoomSettings,
    memory_policy: MemoryPolicySettings,
    animation_timing: AnimationTimingSettings,
    navigation: NavigationSettings,
    export: ExportSettings,
    status_ui: StatusUiSettings,
    interaction: InteractionSettings,
}

impl UserSettings {
    pub fn zoom(&self) -> ZoomSettings {
        self.zoom
    }

    pub fn set_zoom(&mut self, zoom: ZoomSettings) {
        self.zoom = zoom;
    }

    pub fn memory_policy(&self) -> MemoryPolicySettings {
        self.memory_policy
    }

    pub fn set_memory_policy(&mut self, memory_policy: MemoryPolicySettings) {
        self.memory_policy = memory_policy;
    }

    pub fn animation_timing(&self) -> AnimationTimingSettings {
        self.animation_timing
    }

    pub fn set_animation_timing(&mut self, animation_timing: AnimationTimingSettings) {
        self.animation_timing = animation_timing;
    }

    pub fn navigation(&self) -> NavigationSettings {
        self.navigation
    }

    pub fn set_navigation(&mut self, navigation: NavigationSettings) {
        self.navigation = navigation;
    }

    pub fn export(&self) -> &ExportSettings {
        &self.export
    }

    pub fn set_export(&mut self, export: ExportSettings) {
        self.export = export;
    }

    pub fn status_ui(&self) -> StatusUiSettings {
        self.status_ui
    }

    pub fn set_status_ui(&mut self, status_ui: StatusUiSettings) {
        self.status_ui = status_ui;
    }

    pub fn interaction(&self) -> InteractionSettings {
        self.interaction
    }

    pub fn set_interaction(&mut self, interaction: InteractionSettings) {
        self.interaction = interaction;
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppConfig {
    window_bounds: Option<WindowBounds>,
    ui_language: UiLanguage,
    default_view_mode: ViewMode,
    scaling_quality: ScalingQuality,
    recent_folder: Option<PathBuf>,
    export_default_quality: u8,
    animation_autoplay: bool,
    user_settings: UserSettings,
}

impl AppConfig {
    pub fn new(
        window_bounds: Option<WindowBounds>,
        default_view_mode: ViewMode,
        scaling_quality: ScalingQuality,
        recent_folder: Option<PathBuf>,
        export_default_quality: u8,
        animation_autoplay: bool,
    ) -> Self {
        let mut config = Self {
            window_bounds,
            animation_autoplay,
            ..Self::default()
        };
        config.set_default_view_mode(default_view_mode);
        config.set_scaling_quality(scaling_quality);
        config.set_recent_folder(recent_folder);
        config.set_export_default_quality(export_default_quality);
        config
    }

    pub fn window_bounds(&self) -> Option<WindowBounds> {
        self.window_bounds
    }

    pub fn set_window_bounds(&mut self, window_bounds: Option<WindowBounds>) {
        self.window_bounds = window_bounds;
    }

    pub fn ui_language(&self) -> UiLanguage {
        self.ui_language
    }

    pub fn set_ui_language(&mut self, ui_language: UiLanguage) {
        self.ui_language = ui_language;
    }

    pub fn default_view_mode(&self) -> ViewMode {
        self.default_view_mode
    }

    pub fn default_view_transform(&self) -> ViewTransform {
        view_transform_for_default_mode(self.default_view_mode)
    }

    pub fn set_default_view_mode(&mut self, view_mode: ViewMode) {
        self.default_view_mode = default_config_view_mode(view_mode);
    }

    pub fn scaling_quality(&self) -> ScalingQuality {
        self.scaling_quality
    }

    pub fn set_scaling_quality(&mut self, scaling_quality: ScalingQuality) {
        self.scaling_quality = scaling_quality;
    }

    pub fn recent_folder(&self) -> Option<&Path> {
        self.recent_folder.as_deref()
    }

    pub fn set_recent_folder(&mut self, recent_folder: Option<PathBuf>) {
        self.recent_folder = recent_folder.filter(|path| !path.as_os_str().is_empty());
    }

    pub fn export_default_quality(&self) -> u8 {
        self.export_default_quality
    }

    pub fn set_export_default_quality(&mut self, quality: u8) {
        self.export_default_quality = clamp_config_export_quality(quality);
    }

    pub fn animation_autoplay(&self) -> bool {
        self.animation_autoplay
    }

    pub fn set_animation_autoplay(&mut self, animation_autoplay: bool) {
        self.animation_autoplay = animation_autoplay;
    }

    pub fn user_settings(&self) -> &UserSettings {
        &self.user_settings
    }

    pub fn set_user_settings(&mut self, user_settings: UserSettings) {
        self.user_settings = user_settings;
    }

    pub fn zoom_settings(&self) -> ZoomSettings {
        self.user_settings.zoom()
    }

    pub fn set_zoom_settings(&mut self, zoom: ZoomSettings) {
        self.user_settings.set_zoom(zoom);
    }

    pub fn memory_policy_settings(&self) -> MemoryPolicySettings {
        self.user_settings.memory_policy()
    }

    pub fn set_memory_policy_settings(&mut self, memory_policy: MemoryPolicySettings) {
        self.user_settings.set_memory_policy(memory_policy);
    }

    pub fn image_memory_policy(&self) -> ImageMemoryPolicy {
        self.user_settings.memory_policy().image_memory_policy()
    }

    pub fn animation_timing_settings(&self) -> AnimationTimingSettings {
        self.user_settings.animation_timing()
    }

    pub fn set_animation_timing_settings(&mut self, animation_timing: AnimationTimingSettings) {
        self.user_settings.set_animation_timing(animation_timing);
    }

    pub fn navigation_settings(&self) -> NavigationSettings {
        self.user_settings.navigation()
    }

    pub fn set_navigation_settings(&mut self, navigation: NavigationSettings) {
        self.user_settings.set_navigation(navigation);
    }

    pub fn export_settings(&self) -> &ExportSettings {
        self.user_settings.export()
    }

    pub fn set_export_settings(&mut self, export: ExportSettings) {
        self.user_settings.set_export(export);
    }

    pub fn status_ui_settings(&self) -> StatusUiSettings {
        self.user_settings.status_ui()
    }

    pub fn set_status_ui_settings(&mut self, status_ui: StatusUiSettings) {
        self.user_settings.set_status_ui(status_ui);
    }

    pub fn interaction_settings(&self) -> InteractionSettings {
        self.user_settings.interaction()
    }

    pub fn set_interaction_settings(&mut self, interaction: InteractionSettings) {
        self.user_settings.set_interaction(interaction);
    }
}

impl Default for AppConfig {
    fn default() -> Self {
        Self {
            window_bounds: None,
            ui_language: UiLanguage::English,
            default_view_mode: ViewMode::FitToWindow,
            scaling_quality: DEFAULT_SCALING_QUALITY,
            recent_folder: None,
            export_default_quality: DEFAULT_EXPORT_QUALITY,
            animation_autoplay: true,
            user_settings: UserSettings::default(),
        }
    }
}

fn default_config_view_mode(view_mode: ViewMode) -> ViewMode {
    match view_mode {
        ViewMode::FitToWindow | ViewMode::ManualZoom => ViewMode::FitToWindow,
        ViewMode::ActualSize => ViewMode::ActualSize,
    }
}

fn view_transform_for_default_mode(view_mode: ViewMode) -> ViewTransform {
    match default_config_view_mode(view_mode) {
        ViewMode::FitToWindow | ViewMode::ManualZoom => ViewTransform::FIT_TO_WINDOW,
        ViewMode::ActualSize => ViewTransform::ACTUAL_SIZE,
    }
}

fn clamp_config_export_quality(quality: u8) -> u8 {
    quality.clamp(MIN_EXPORT_QUALITY, MAX_EXPORT_QUALITY)
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AppConfigParseError {
    MalformedLine { line: usize },
    UnsupportedVersion { line: usize },
    InvalidEscape { line: usize },
}

impl fmt::Display for AppConfigParseError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MalformedLine { line } => {
                write!(formatter, "malformed app config line: {line}")
            }
            Self::UnsupportedVersion { line } => {
                write!(formatter, "unsupported app config version: line {line}")
            }
            Self::InvalidEscape { line } => {
                write!(formatter, "invalid app config escape: line {line}")
            }
        }
    }
}

impl Error for AppConfigParseError {}

pub fn serialize_app_config(config: &AppConfig) -> String {
    let mut output = String::new();
    output.push_str("version=1\n");

    if let Some(bounds) = config.window_bounds() {
        output.push_str(&format!("window.x={}\n", bounds.x()));
        output.push_str(&format!("window.y={}\n", bounds.y()));
        output.push_str(&format!("window.width={}\n", bounds.width()));
        output.push_str(&format!("window.height={}\n", bounds.height()));
    }

    output.push_str(&format!(
        "ui_language={}\n",
        config_string_value(config_ui_language_name(config.ui_language()))
    ));
    output.push_str(&format!(
        "default_view_mode={}\n",
        config_string_value(config_view_mode_name(config.default_view_mode()))
    ));
    output.push_str(&format!(
        "scaling_quality={}\n",
        config_string_value(config_scaling_quality_name(config.scaling_quality()))
    ));

    if let Some(folder) = config.recent_folder() {
        output.push_str("recent_folder=");
        output.push_str(&config_string_value(&folder.to_string_lossy()));
        output.push('\n');
    }

    output.push_str(&format!(
        "export_default_quality={}\n",
        config.export_default_quality()
    ));
    output.push_str(&format!(
        "animation_autoplay={}\n",
        config.animation_autoplay()
    ));

    let zoom = config.zoom_settings();
    output.push_str(&format!("min_zoom_scale={}\n", zoom.min_zoom_scale()));
    output.push_str(&format!("max_zoom_scale={}\n", zoom.max_zoom_scale()));
    output.push_str(&format!("zoom_step_factor={}\n", zoom.zoom_step_factor()));

    let memory = config.memory_policy_settings();
    output.push_str(&format!(
        "large_image_pixel_threshold={}\n",
        memory.large_image_pixel_threshold()
    ));
    output.push_str(&format!("max_image_pixels={}\n", memory.max_image_pixels()));
    output.push_str(&format!(
        "preview_max_pixels={}\n",
        memory.preview_max_pixels()
    ));
    output.push_str(&format!(
        "preview_oversample={}\n",
        memory.preview_oversample()
    ));
    output.push_str(&format!(
        "fallback_viewport_width={}\n",
        memory.fallback_viewport_width()
    ));
    output.push_str(&format!(
        "fallback_viewport_height={}\n",
        memory.fallback_viewport_height()
    ));
    output.push_str(&format!(
        "max_transient_decode_mib={}\n",
        memory.max_transient_decode_mib()
    ));
    output.push_str(&format!(
        "max_full_resolution_mib={}\n",
        memory.max_full_resolution_mib()
    ));
    output.push_str(&format!("max_resident_mib={}\n", memory.max_resident_mib()));
    output.push_str(&format!(
        "max_cache_entry_mib={}\n",
        memory.max_cache_entry_mib()
    ));
    output.push_str(&format!(
        "max_cache_entries={}\n",
        memory.max_cache_entries()
    ));
    output.push_str(&format!(
        "max_animation_metadata_frames={}\n",
        memory.max_animation_metadata_frames()
    ));
    output.push_str(&format!(
        "full_resolution_request_scale={}\n",
        memory.full_resolution_request_scale()
    ));

    let animation_timing = config.animation_timing_settings();
    output.push_str(&format!(
        "default_frame_delay_ms={}\n",
        animation_timing.default_frame_delay_ms()
    ));
    output.push_str(&format!(
        "min_frame_delay_ms={}\n",
        animation_timing.min_frame_delay_ms()
    ));
    output.push_str(&format!(
        "max_frame_delay_ms={}\n",
        animation_timing.max_frame_delay_ms()
    ));

    let navigation = config.navigation_settings();
    output.push_str(&format!(
        "wrap_navigation={}\n",
        navigation.wrap_navigation()
    ));
    output.push_str(&format!(
        "auto_skip_failed_navigation={}\n",
        navigation.auto_skip_failed_navigation()
    ));
    output.push_str(&format!(
        "max_navigation_attempts_per_command={}\n",
        navigation.max_navigation_attempts_per_command()
    ));

    let export = config.export_settings();
    output.push_str(&format!(
        "default_export_format_policy={}\n",
        config_string_value(config_default_export_format_policy_name(
            export.default_export_format_policy()
        ))
    ));
    output.push_str("export_filename_suffix=");
    output.push_str(&config_string_value(export.export_filename_suffix()));
    output.push('\n');
    output.push_str(&format!(
        "jpeg_alpha_background_rgb={}\n",
        config_string_value(&config_rgb_color_value(export.jpeg_alpha_background_rgb()))
    ));

    let status_ui = config.status_ui_settings();
    output.push_str(&format!(
        "show_status_bar={}\n",
        status_ui.show_status_bar()
    ));
    output.push_str(&format!(
        "detailed_status_text={}\n",
        status_ui.detailed_status_text()
    ));

    let interaction = config.interaction_settings();
    output.push_str(&format!(
        "zoom_shortcut={}\n",
        config_string_value(config_mouse_shortcut_name(interaction.zoom_shortcut()))
    ));
    output.push_str(&format!(
        "image_navigation_shortcut={}\n",
        config_string_value(config_mouse_shortcut_name(
            interaction.image_navigation_shortcut()
        ))
    ));
    output.push_str(&format!(
        "image_pan_shortcut={}\n",
        config_string_value(config_mouse_shortcut_name(interaction.image_pan_shortcut()))
    ));
    output.push_str(&format!(
        "window_move_shortcut={}\n",
        config_string_value(config_mouse_shortcut_name(
            interaction.window_move_shortcut()
        ))
    ));
    output
}

pub fn parse_app_config(contents: &str) -> Result<AppConfig, AppConfigParseError> {
    let mut parsed = ParsedAppConfig::default();
    let mut saw_entry = false;

    for (index, line) in contents.lines().enumerate() {
        let line_number = index + 1;
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') {
            continue;
        }

        let Some((key, value)) = line.split_once('=') else {
            return Err(AppConfigParseError::MalformedLine { line: line_number });
        };
        let key = key.trim();
        let value = value.trim();
        if key.is_empty() {
            return Err(AppConfigParseError::MalformedLine { line: line_number });
        }
        saw_entry = true;

        match key {
            "version" if value != "1" => {
                return Err(AppConfigParseError::UnsupportedVersion { line: line_number });
            }
            "version" => {}
            "window.x" => parsed.window_x = value.parse::<i32>().ok(),
            "window.y" => parsed.window_y = value.parse::<i32>().ok(),
            "window.width" => parsed.window_width = value.parse::<i32>().ok(),
            "window.height" => parsed.window_height = value.parse::<i32>().ok(),
            "ui_language" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.ui_language = config_ui_language_from_name(&decoded);
            }
            "default_view_mode" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.default_view_mode = config_view_mode_from_name(&decoded);
            }
            "scaling_quality" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.scaling_quality = config_scaling_quality_from_name(&decoded);
            }
            "recent_folder" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.recent_folder = Some(PathBuf::from(decoded));
            }
            "export_default_quality" => {
                parsed.export_default_quality = parse_config_i128(value);
            }
            "animation_autoplay" => {
                parsed.animation_autoplay = parse_config_bool(value);
            }
            "min_zoom_scale" => {
                parsed.user_settings.zoom.min_zoom_scale = value.parse::<f64>().ok();
            }
            "max_zoom_scale" => {
                parsed.user_settings.zoom.max_zoom_scale = value.parse::<f64>().ok();
            }
            "zoom_step_factor" => {
                parsed.user_settings.zoom.zoom_step_factor = value.parse::<f64>().ok();
            }
            "large_image_pixel_threshold" => {
                parsed
                    .user_settings
                    .memory_policy
                    .large_image_pixel_threshold = parse_config_i128(value);
            }
            "max_image_pixels" => {
                parsed.user_settings.memory_policy.max_image_pixels = parse_config_i128(value);
            }
            "preview_max_pixels" => {
                parsed.user_settings.memory_policy.preview_max_pixels = parse_config_i128(value);
            }
            "preview_oversample" => {
                parsed.user_settings.memory_policy.preview_oversample = parse_config_i128(value);
            }
            "fallback_viewport_width" => {
                parsed.user_settings.memory_policy.fallback_viewport_width =
                    parse_config_i128(value);
            }
            "fallback_viewport_height" => {
                parsed.user_settings.memory_policy.fallback_viewport_height =
                    parse_config_i128(value);
            }
            "max_transient_decode_mib" => {
                parsed.user_settings.memory_policy.max_transient_decode_mib =
                    parse_config_i128(value);
            }
            "max_full_resolution_mib" => {
                parsed.user_settings.memory_policy.max_full_resolution_mib =
                    parse_config_i128(value);
            }
            "max_resident_mib" => {
                parsed.user_settings.memory_policy.max_resident_mib = parse_config_i128(value);
            }
            "max_cache_entry_mib" => {
                parsed.user_settings.memory_policy.max_cache_entry_mib = parse_config_i128(value);
            }
            "max_cache_entries" => {
                parsed.user_settings.memory_policy.max_cache_entries = parse_config_i128(value);
            }
            "max_animation_metadata_frames" => {
                parsed
                    .user_settings
                    .memory_policy
                    .max_animation_metadata_frames = parse_config_i128(value);
            }
            "full_resolution_request_scale" => {
                parsed
                    .user_settings
                    .memory_policy
                    .full_resolution_request_scale = value.parse::<f64>().ok();
            }
            "default_frame_delay_ms" => {
                parsed.user_settings.animation_timing.default_frame_delay_ms =
                    parse_config_i128(value);
            }
            "min_frame_delay_ms" => {
                parsed.user_settings.animation_timing.min_frame_delay_ms = parse_config_i128(value);
            }
            "max_frame_delay_ms" => {
                parsed.user_settings.animation_timing.max_frame_delay_ms = parse_config_i128(value);
            }
            "wrap_navigation" => {
                parsed.user_settings.navigation.wrap_navigation = parse_config_bool(value);
            }
            "auto_skip_failed_navigation" => {
                parsed.user_settings.navigation.auto_skip_failed_navigation =
                    parse_config_bool(value);
            }
            "max_navigation_attempts_per_command" => {
                parsed
                    .user_settings
                    .navigation
                    .max_navigation_attempts_per_command = parse_config_i128(value);
            }
            "default_export_format_policy" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.user_settings.export.default_export_format_policy =
                    config_default_export_format_policy_from_name(&decoded);
            }
            "export_filename_suffix" => {
                parsed.user_settings.export.export_filename_suffix = Some(
                    parse_config_string_value(value, line_number)
                        .unwrap_or_else(|_| DEFAULT_EXPORT_FILENAME_SUFFIX.to_owned()),
                );
            }
            "jpeg_alpha_background_rgb" => {
                let decoded = parse_config_string_value(value, line_number)
                    .unwrap_or_else(|_| value.to_owned());
                parsed.user_settings.export.jpeg_alpha_background_rgb =
                    parse_config_rgb_color(&decoded);
            }
            "show_status_bar" => {
                parsed.user_settings.status_ui.show_status_bar = parse_config_bool(value);
            }
            "detailed_status_text" => {
                parsed.user_settings.status_ui.detailed_status_text = parse_config_bool(value);
            }
            "zoom_shortcut" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.user_settings.interaction.zoom_shortcut =
                    config_mouse_shortcut_from_name(&decoded);
            }
            "image_navigation_shortcut" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.user_settings.interaction.image_navigation_shortcut =
                    config_mouse_shortcut_from_name(&decoded);
            }
            "image_pan_shortcut" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.user_settings.interaction.image_pan_shortcut =
                    config_mouse_shortcut_from_name(&decoded);
            }
            "window_move_shortcut" => {
                let decoded = parse_config_string_value(value, line_number)?;
                parsed.user_settings.interaction.window_move_shortcut =
                    config_mouse_shortcut_from_name(&decoded);
            }
            _ => {}
        }
    }

    if !saw_entry {
        return Ok(AppConfig::default());
    }

    Ok(parsed.into_config())
}

#[derive(Default)]
struct ParsedAppConfig {
    window_x: Option<i32>,
    window_y: Option<i32>,
    window_width: Option<i32>,
    window_height: Option<i32>,
    ui_language: Option<UiLanguage>,
    default_view_mode: Option<ViewMode>,
    scaling_quality: Option<ScalingQuality>,
    recent_folder: Option<PathBuf>,
    export_default_quality: Option<i128>,
    animation_autoplay: Option<bool>,
    user_settings: ParsedUserSettings,
}

impl ParsedAppConfig {
    fn into_config(self) -> AppConfig {
        let window_bounds = match (
            self.window_x,
            self.window_y,
            self.window_width,
            self.window_height,
        ) {
            (Some(x), Some(y), Some(width), Some(height)) => WindowBounds::new(x, y, width, height),
            _ => None,
        };
        let export_default_quality = sanitize_optional_config_u8(
            self.export_default_quality,
            MIN_EXPORT_QUALITY,
            MAX_EXPORT_QUALITY,
            DEFAULT_EXPORT_QUALITY,
        );

        let mut config = AppConfig::new(
            window_bounds,
            self.default_view_mode.unwrap_or(ViewMode::FitToWindow),
            self.scaling_quality.unwrap_or(DEFAULT_SCALING_QUALITY),
            self.recent_folder,
            export_default_quality,
            self.animation_autoplay.unwrap_or(true),
        );
        config.set_ui_language(self.ui_language.unwrap_or_default());
        config.set_user_settings(self.user_settings.into_settings());
        config
    }
}

#[derive(Default)]
struct ParsedUserSettings {
    zoom: ParsedZoomSettings,
    memory_policy: ParsedMemoryPolicySettings,
    animation_timing: ParsedAnimationTimingSettings,
    navigation: ParsedNavigationSettings,
    export: ParsedExportSettings,
    status_ui: ParsedStatusUiSettings,
    interaction: ParsedInteractionSettings,
}

impl ParsedUserSettings {
    fn into_settings(self) -> UserSettings {
        let mut settings = UserSettings::default();
        settings.set_zoom(self.zoom.into_settings());
        settings.set_memory_policy(self.memory_policy.into_settings());
        settings.set_animation_timing(self.animation_timing.into_settings());
        settings.set_navigation(self.navigation.into_settings());
        settings.set_export(self.export.into_settings());
        settings.set_status_ui(self.status_ui.into_settings());
        settings.set_interaction(self.interaction.into_settings());
        settings
    }
}

#[derive(Default)]
struct ParsedZoomSettings {
    min_zoom_scale: Option<f64>,
    max_zoom_scale: Option<f64>,
    zoom_step_factor: Option<f64>,
}

impl ParsedZoomSettings {
    fn into_settings(self) -> ZoomSettings {
        ZoomSettings {
            min_zoom_scale: sanitize_optional_config_f64(
                self.min_zoom_scale,
                MIN_CONFIG_MIN_ZOOM_SCALE,
                MAX_CONFIG_MIN_ZOOM_SCALE,
                MIN_ZOOM_SCALE,
            ),
            max_zoom_scale: sanitize_optional_config_f64(
                self.max_zoom_scale,
                MIN_CONFIG_MAX_ZOOM_SCALE,
                MAX_CONFIG_MAX_ZOOM_SCALE,
                MAX_ZOOM_SCALE,
            ),
            zoom_step_factor: sanitize_optional_config_f64(
                self.zoom_step_factor,
                MIN_CONFIG_ZOOM_STEP_FACTOR,
                MAX_CONFIG_ZOOM_STEP_FACTOR,
                ZOOM_STEP_FACTOR,
            ),
        }
    }
}

#[derive(Default)]
struct ParsedMemoryPolicySettings {
    large_image_pixel_threshold: Option<i128>,
    max_image_pixels: Option<i128>,
    preview_max_pixels: Option<i128>,
    preview_oversample: Option<i128>,
    fallback_viewport_width: Option<i128>,
    fallback_viewport_height: Option<i128>,
    max_transient_decode_mib: Option<i128>,
    max_full_resolution_mib: Option<i128>,
    max_resident_mib: Option<i128>,
    max_cache_entry_mib: Option<i128>,
    max_cache_entries: Option<i128>,
    max_animation_metadata_frames: Option<i128>,
    full_resolution_request_scale: Option<f64>,
}

impl ParsedMemoryPolicySettings {
    fn into_settings(self) -> MemoryPolicySettings {
        let mut settings = MemoryPolicySettings {
            large_image_pixel_threshold: sanitize_optional_config_u64(
                self.large_image_pixel_threshold,
                MIN_CONFIG_PIXEL_LIMIT,
                MAX_CONFIG_IMAGE_PIXELS,
                DEFAULT_LARGE_IMAGE_PIXEL_THRESHOLD,
            ),
            max_image_pixels: sanitize_optional_config_u64(
                self.max_image_pixels,
                MIN_CONFIG_PIXEL_LIMIT,
                MAX_CONFIG_IMAGE_PIXELS,
                DEFAULT_MAX_IMAGE_PIXELS,
            ),
            preview_max_pixels: sanitize_optional_config_u64(
                self.preview_max_pixels,
                MIN_CONFIG_PIXEL_LIMIT,
                MAX_CONFIG_IMAGE_PIXELS,
                DEFAULT_PREVIEW_MAX_PIXELS,
            ),
            preview_oversample: sanitize_optional_config_u32(
                self.preview_oversample,
                MIN_CONFIG_PREVIEW_OVERSAMPLE,
                MAX_CONFIG_PREVIEW_OVERSAMPLE,
                DEFAULT_PREVIEW_OVERSAMPLE,
            ),
            fallback_viewport_width: sanitize_optional_config_u32(
                self.fallback_viewport_width,
                MIN_CONFIG_VIEWPORT_EDGE,
                MAX_CONFIG_VIEWPORT_EDGE,
                DEFAULT_FALLBACK_VIEWPORT_WIDTH,
            ),
            fallback_viewport_height: sanitize_optional_config_u32(
                self.fallback_viewport_height,
                MIN_CONFIG_VIEWPORT_EDGE,
                MAX_CONFIG_VIEWPORT_EDGE,
                DEFAULT_FALLBACK_VIEWPORT_HEIGHT,
            ),
            max_transient_decode_mib: sanitize_optional_config_u32(
                self.max_transient_decode_mib,
                MIN_CONFIG_MEMORY_MIB,
                MAX_CONFIG_MEMORY_MIB,
                DEFAULT_MAX_TRANSIENT_DECODE_MIB,
            ),
            max_full_resolution_mib: sanitize_optional_config_u32(
                self.max_full_resolution_mib,
                MIN_CONFIG_MEMORY_MIB,
                MAX_CONFIG_MEMORY_MIB,
                DEFAULT_MAX_FULL_RESOLUTION_MIB,
            ),
            max_resident_mib: sanitize_optional_config_u32(
                self.max_resident_mib,
                MIN_CONFIG_MEMORY_MIB,
                MAX_CONFIG_MEMORY_MIB,
                DEFAULT_MAX_RESIDENT_MIB,
            ),
            max_cache_entry_mib: sanitize_optional_config_u32(
                self.max_cache_entry_mib,
                MIN_CONFIG_MEMORY_MIB,
                MAX_CONFIG_MEMORY_MIB,
                DEFAULT_MAX_CACHE_ENTRY_MIB,
            ),
            max_cache_entries: sanitize_optional_config_usize(
                self.max_cache_entries,
                MIN_CONFIG_CACHE_ENTRIES,
                MAX_CONFIG_CACHE_ENTRIES,
                DEFAULT_MAX_CACHE_ENTRIES,
            ),
            max_animation_metadata_frames: sanitize_optional_config_usize(
                self.max_animation_metadata_frames,
                MIN_CONFIG_ANIMATION_METADATA_FRAMES,
                MAX_CONFIG_ANIMATION_METADATA_FRAMES,
                DEFAULT_MAX_ANIMATION_METADATA_FRAMES,
            ),
            full_resolution_request_scale: sanitize_optional_config_f64(
                self.full_resolution_request_scale,
                MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
                MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
                DEFAULT_FULL_RESOLUTION_REQUEST_SCALE,
            ),
        };
        settings.normalize_relationships();
        settings
    }
}

#[derive(Default)]
struct ParsedAnimationTimingSettings {
    default_frame_delay_ms: Option<i128>,
    min_frame_delay_ms: Option<i128>,
    max_frame_delay_ms: Option<i128>,
}

impl ParsedAnimationTimingSettings {
    fn into_settings(self) -> AnimationTimingSettings {
        let mut settings = AnimationTimingSettings {
            default_frame_delay_ms: sanitize_optional_config_u32(
                self.default_frame_delay_ms,
                MIN_CONFIG_ANIMATION_DELAY_MS,
                MAX_CONFIG_ANIMATION_DELAY_MS,
                DEFAULT_ANIMATION_FRAME_DELAY_MS,
            ),
            min_frame_delay_ms: sanitize_optional_config_u32(
                self.min_frame_delay_ms,
                MIN_CONFIG_ANIMATION_DELAY_MS,
                MAX_CONFIG_ANIMATION_DELAY_MS,
                MIN_ANIMATION_FRAME_DELAY_MS,
            ),
            max_frame_delay_ms: sanitize_optional_config_u32(
                self.max_frame_delay_ms,
                MIN_CONFIG_ANIMATION_DELAY_MS,
                MAX_CONFIG_ANIMATION_DELAY_MS,
                MAX_ANIMATION_FRAME_DELAY_MS,
            ),
        };
        settings.normalize_relationships();
        settings
    }
}

#[derive(Default)]
struct ParsedNavigationSettings {
    wrap_navigation: Option<bool>,
    auto_skip_failed_navigation: Option<bool>,
    max_navigation_attempts_per_command: Option<i128>,
}

impl ParsedNavigationSettings {
    fn into_settings(self) -> NavigationSettings {
        NavigationSettings {
            wrap_navigation: self.wrap_navigation.unwrap_or(true),
            auto_skip_failed_navigation: self.auto_skip_failed_navigation.unwrap_or(false),
            max_navigation_attempts_per_command: sanitize_optional_config_usize(
                self.max_navigation_attempts_per_command,
                1,
                100,
                1,
            ),
        }
    }
}

#[derive(Default)]
struct ParsedExportSettings {
    default_export_format_policy: Option<DefaultExportFormatPolicy>,
    export_filename_suffix: Option<String>,
    jpeg_alpha_background_rgb: Option<RgbColor>,
}

impl ParsedExportSettings {
    fn into_settings(self) -> ExportSettings {
        ExportSettings::new(
            self.default_export_format_policy
                .unwrap_or(DefaultExportFormatPolicy::Png),
            self.export_filename_suffix
                .unwrap_or_else(|| DEFAULT_EXPORT_FILENAME_SUFFIX.to_owned()),
            self.jpeg_alpha_background_rgb.unwrap_or(RgbColor::WHITE),
        )
    }
}

#[derive(Default)]
struct ParsedStatusUiSettings {
    show_status_bar: Option<bool>,
    detailed_status_text: Option<bool>,
}

impl ParsedStatusUiSettings {
    fn into_settings(self) -> StatusUiSettings {
        StatusUiSettings {
            show_status_bar: self.show_status_bar.unwrap_or(true),
            detailed_status_text: self.detailed_status_text.unwrap_or(true),
        }
    }
}

#[derive(Default)]
struct ParsedInteractionSettings {
    zoom_shortcut: Option<MouseShortcut>,
    image_navigation_shortcut: Option<MouseShortcut>,
    image_pan_shortcut: Option<MouseShortcut>,
    window_move_shortcut: Option<MouseShortcut>,
}

impl ParsedInteractionSettings {
    fn into_settings(self) -> InteractionSettings {
        let default = InteractionSettings::default();
        InteractionSettings {
            zoom_shortcut: self.zoom_shortcut.unwrap_or(default.zoom_shortcut()),
            image_navigation_shortcut: self
                .image_navigation_shortcut
                .unwrap_or(default.image_navigation_shortcut()),
            image_pan_shortcut: self
                .image_pan_shortcut
                .unwrap_or(default.image_pan_shortcut()),
            window_move_shortcut: self
                .window_move_shortcut
                .unwrap_or(default.window_move_shortcut()),
        }
    }
}

fn config_ui_language_name(language: UiLanguage) -> &'static str {
    match language {
        UiLanguage::English => "english",
        UiLanguage::Korean => "korean",
    }
}

fn config_ui_language_from_name(value: &str) -> Option<UiLanguage> {
    if value.eq_ignore_ascii_case("english") || value.eq_ignore_ascii_case("en") {
        Some(UiLanguage::English)
    } else if value.eq_ignore_ascii_case("korean")
        || value.eq_ignore_ascii_case("ko")
        || value.eq_ignore_ascii_case("kr")
    {
        Some(UiLanguage::Korean)
    } else {
        None
    }
}

fn config_view_mode_name(view_mode: ViewMode) -> &'static str {
    match default_config_view_mode(view_mode) {
        ViewMode::FitToWindow | ViewMode::ManualZoom => "fit_to_window",
        ViewMode::ActualSize => "actual_size",
    }
}

fn config_view_mode_from_name(value: &str) -> Option<ViewMode> {
    if value.eq_ignore_ascii_case("fit_to_window") || value.eq_ignore_ascii_case("fit") {
        Some(ViewMode::FitToWindow)
    } else if value.eq_ignore_ascii_case("actual_size") || value.eq_ignore_ascii_case("actual") {
        Some(ViewMode::ActualSize)
    } else {
        None
    }
}

fn config_scaling_quality_name(quality: ScalingQuality) -> &'static str {
    match quality {
        ScalingQuality::Nearest => "nearest",
        ScalingQuality::Balanced => "balanced",
        ScalingQuality::HighQuality => "high_quality",
    }
}

fn config_scaling_quality_from_name(value: &str) -> Option<ScalingQuality> {
    if value.eq_ignore_ascii_case("nearest") {
        Some(ScalingQuality::Nearest)
    } else if value.eq_ignore_ascii_case("balanced") {
        Some(ScalingQuality::Balanced)
    } else if value.eq_ignore_ascii_case("high_quality")
        || value.eq_ignore_ascii_case("high-quality")
        || value.eq_ignore_ascii_case("highquality")
    {
        Some(ScalingQuality::HighQuality)
    } else {
        None
    }
}

fn config_default_export_format_policy_name(policy: DefaultExportFormatPolicy) -> &'static str {
    match policy {
        DefaultExportFormatPolicy::Source => "source",
        DefaultExportFormatPolicy::Png => "png",
        DefaultExportFormatPolicy::Jpeg => "jpeg",
        DefaultExportFormatPolicy::Bmp => "bmp",
        DefaultExportFormatPolicy::Webp => "webp",
        DefaultExportFormatPolicy::Ico => "ico",
    }
}

fn config_default_export_format_policy_from_name(value: &str) -> Option<DefaultExportFormatPolicy> {
    if value.eq_ignore_ascii_case("source") {
        Some(DefaultExportFormatPolicy::Source)
    } else if value.eq_ignore_ascii_case("png") {
        Some(DefaultExportFormatPolicy::Png)
    } else if value.eq_ignore_ascii_case("jpeg") || value.eq_ignore_ascii_case("jpg") {
        Some(DefaultExportFormatPolicy::Jpeg)
    } else if value.eq_ignore_ascii_case("bmp") {
        Some(DefaultExportFormatPolicy::Bmp)
    } else if value.eq_ignore_ascii_case("webp") {
        Some(DefaultExportFormatPolicy::Webp)
    } else if value.eq_ignore_ascii_case("ico") {
        Some(DefaultExportFormatPolicy::Ico)
    } else {
        None
    }
}

pub fn config_mouse_shortcut_name(shortcut: MouseShortcut) -> &'static str {
    match shortcut {
        MouseShortcut::MouseWheel => "mouse_wheel",
        MouseShortcut::CtrlMouseWheel => "ctrl_mouse_wheel",
        MouseShortcut::LeftButtonDrag => "left_button_drag",
        MouseShortcut::CtrlLeftButtonDrag => "ctrl_left_button_drag",
    }
}

pub fn config_mouse_shortcut_from_name(value: &str) -> Option<MouseShortcut> {
    if value.eq_ignore_ascii_case("mouse_wheel") || value.eq_ignore_ascii_case("wheel") {
        Some(MouseShortcut::MouseWheel)
    } else if value.eq_ignore_ascii_case("ctrl_mouse_wheel")
        || value.eq_ignore_ascii_case("control_mouse_wheel")
        || value.eq_ignore_ascii_case("ctrl+wheel")
        || value.eq_ignore_ascii_case("ctrl_wheel")
    {
        Some(MouseShortcut::CtrlMouseWheel)
    } else if value.eq_ignore_ascii_case("left_button_drag")
        || value.eq_ignore_ascii_case("left_drag")
    {
        Some(MouseShortcut::LeftButtonDrag)
    } else if value.eq_ignore_ascii_case("ctrl_left_button_drag")
        || value.eq_ignore_ascii_case("control_left_button_drag")
        || value.eq_ignore_ascii_case("ctrl+left_drag")
        || value.eq_ignore_ascii_case("ctrl_left_drag")
    {
        Some(MouseShortcut::CtrlLeftButtonDrag)
    } else {
        None
    }
}

fn parse_config_bool(value: &str) -> Option<bool> {
    if value.eq_ignore_ascii_case("true") || value == "1" || value.eq_ignore_ascii_case("yes") {
        Some(true)
    } else if value.eq_ignore_ascii_case("false")
        || value == "0"
        || value.eq_ignore_ascii_case("no")
    {
        Some(false)
    } else {
        None
    }
}

fn parse_config_i128(value: &str) -> Option<i128> {
    match value.parse::<i128>() {
        Ok(value) => Some(value),
        Err(error) => match error.kind() {
            std::num::IntErrorKind::PosOverflow => Some(i128::MAX),
            std::num::IntErrorKind::NegOverflow => Some(i128::MIN),
            _ => None,
        },
    }
}

fn sanitize_optional_config_f64(value: Option<f64>, min: f64, max: f64, default: f64) -> f64 {
    value
        .map(|value| sanitize_config_f64(value, min, max, default))
        .unwrap_or(default)
}

fn sanitize_config_f64(value: f64, min: f64, max: f64, default: f64) -> f64 {
    if value.is_finite() {
        value.clamp(min, max)
    } else {
        default
    }
}

fn sanitize_optional_config_u64(value: Option<i128>, min: u64, max: u64, default: u64) -> u64 {
    let Some(value) = value else {
        return default;
    };
    let clamped = value.clamp(i128::from(min), i128::from(max));
    match u64::try_from(clamped) {
        Ok(value) => value,
        Err(_) => default,
    }
}

fn sanitize_optional_config_u32(value: Option<i128>, min: u32, max: u32, default: u32) -> u32 {
    let Some(value) = value else {
        return default;
    };
    let clamped = value.clamp(i128::from(min), i128::from(max));
    match u32::try_from(clamped) {
        Ok(value) => value,
        Err(_) => default,
    }
}

fn sanitize_optional_config_u8(value: Option<i128>, min: u8, max: u8, default: u8) -> u8 {
    let Some(value) = value else {
        return default;
    };
    let clamped = value.clamp(i128::from(min), i128::from(max));
    match u8::try_from(clamped) {
        Ok(value) => value,
        Err(_) => default,
    }
}

fn sanitize_optional_config_usize(
    value: Option<i128>,
    min: usize,
    max: usize,
    default: usize,
) -> usize {
    let Some(value) = value else {
        return default;
    };
    let clamped = value.clamp(min as i128, max as i128);
    match usize::try_from(clamped) {
        Ok(value) => value,
        Err(_) => default,
    }
}

fn mib_to_bytes(mib: u32) -> usize {
    let bytes = u64::from(mib).saturating_mul(BYTES_PER_MIB as u64);
    match usize::try_from(bytes) {
        Ok(bytes) => bytes,
        Err(_) => usize::MAX,
    }
}

mod export_suffix {
    use super::{DEFAULT_EXPORT_FILENAME_SUFFIX, MAX_EXPORT_FILENAME_SUFFIX_CHARS};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub(crate) enum ExportFilenameSuffixValidationError {
        Empty,
        TooLong,
        InvalidCharacter,
    }

    pub(crate) fn validate_export_filename_suffix(
        suffix: &str,
    ) -> Result<&str, ExportFilenameSuffixValidationError> {
        let suffix = suffix.trim();
        if suffix.is_empty() {
            return Err(ExportFilenameSuffixValidationError::Empty);
        }
        if suffix.chars().count() > MAX_EXPORT_FILENAME_SUFFIX_CHARS {
            return Err(ExportFilenameSuffixValidationError::TooLong);
        }
        if suffix.chars().any(is_invalid_export_suffix_character) {
            return Err(ExportFilenameSuffixValidationError::InvalidCharacter);
        }
        Ok(suffix)
    }

    pub(super) fn sanitize_export_filename_suffix(suffix: String) -> String {
        match validate_export_filename_suffix(&suffix) {
            Ok(suffix) => suffix.to_owned(),
            Err(_) => DEFAULT_EXPORT_FILENAME_SUFFIX.to_owned(),
        }
    }

    fn is_invalid_export_suffix_character(character: char) -> bool {
        character.is_control()
            || matches!(
                character,
                '\\' | '/' | ':' | '*' | '?' | '"' | '<' | '>' | '|'
            )
    }
}

pub(crate) use export_suffix::{
    validate_export_filename_suffix, ExportFilenameSuffixValidationError,
};

fn config_rgb_color_value(color: RgbColor) -> String {
    format!("{},{},{}", color.red(), color.green(), color.blue())
}

fn parse_config_rgb_color(value: &str) -> Option<RgbColor> {
    let mut components = value.split(',');
    let red = parse_config_rgb_component(components.next()?)?;
    let green = parse_config_rgb_component(components.next()?)?;
    let blue = parse_config_rgb_component(components.next()?)?;
    if components.next().is_some() {
        return None;
    }
    Some(RgbColor::new(red, green, blue))
}

fn parse_config_rgb_component(value: &str) -> Option<u8> {
    let value = parse_config_i128(value.trim())?;
    let clamped = value.clamp(0, i128::from(u8::MAX));
    u8::try_from(clamped).ok()
}

fn config_string_value(value: &str) -> String {
    let mut escaped = String::with_capacity(value.len() + 2);
    escaped.push('"');
    for character in value.chars() {
        match character {
            '\\' => escaped.push_str("\\\\"),
            '"' => escaped.push_str("\\\""),
            '\n' => escaped.push_str("\\n"),
            '\r' => escaped.push_str("\\r"),
            '\t' => escaped.push_str("\\t"),
            character if character.is_control() => {
                let code = character as u32;
                if code <= 0xffff {
                    escaped.push_str(&format!("\\u{code:04X}"));
                } else {
                    escaped.push_str(&format!("\\U{code:08X}"));
                }
            }
            _ => escaped.push(character),
        }
    }
    escaped.push('"');
    escaped
}

fn parse_config_string_value(
    value: &str,
    line_number: usize,
) -> Result<String, AppConfigParseError> {
    let value = if let Some(quoted) = value.strip_prefix('"') {
        quoted
            .strip_suffix('"')
            .ok_or(AppConfigParseError::InvalidEscape { line: line_number })?
    } else {
        value
    };

    unescape_config_value(value)
        .map_err(|()| AppConfigParseError::InvalidEscape { line: line_number })
}

fn unescape_config_value(value: &str) -> Result<String, ()> {
    let mut unescaped = String::new();
    let mut characters = value.chars();
    while let Some(character) = characters.next() {
        if character != '\\' {
            unescaped.push(character);
            continue;
        }

        match characters.next() {
            Some('\\') => unescaped.push('\\'),
            Some('"') => unescaped.push('"'),
            Some('n') => unescaped.push('\n'),
            Some('r') => unescaped.push('\r'),
            Some('t') => unescaped.push('\t'),
            Some('u') => unescaped.push(unescape_config_unicode(&mut characters, 4)?),
            Some('U') => unescaped.push(unescape_config_unicode(&mut characters, 8)?),
            Some(_) | None => return Err(()),
        }
    }
    Ok(unescaped)
}

fn unescape_config_unicode(
    characters: &mut std::str::Chars<'_>,
    digits: usize,
) -> Result<char, ()> {
    let mut value = 0;
    for _ in 0..digits {
        let digit = characters
            .next()
            .and_then(|character| character.to_digit(16));
        value = value * 16 + digit.ok_or(())?;
    }
    char::from_u32(value).ok_or(())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct ScalingCacheKey {
    image_revision: u64,
    orientation: ImageOrientation,
    target_size: ImageSize,
    quality: ScalingQuality,
}

impl ScalingCacheKey {
    pub fn new(
        image_revision: u64,
        orientation: ImageOrientation,
        target_size: ImageSize,
        quality: ScalingQuality,
    ) -> Self {
        Self {
            image_revision,
            orientation,
            target_size,
            quality,
        }
    }

    pub fn image_revision(self) -> u64 {
        self.image_revision
    }

    pub fn orientation(self) -> ImageOrientation {
        self.orientation
    }

    pub fn target_size(self) -> ImageSize {
        self.target_size
    }

    pub fn quality(self) -> ScalingQuality {
        self.quality
    }
}

pub fn scaling_quality_for_render(
    preferred_quality: ScalingQuality,
    effective_scale: f64,
) -> ScalingQuality {
    if preferred_quality == ScalingQuality::Nearest
        || !effective_scale.is_finite()
        || effective_scale <= 0.0
        || effective_scale == 1.0
    {
        ScalingQuality::Nearest
    } else {
        preferred_quality
    }
}

pub fn scaling_cache_key_for_render(
    image_revision: u64,
    orientation: ImageOrientation,
    source_size: ImageSize,
    display_size: ImageSize,
    quality: ScalingQuality,
) -> Option<ScalingCacheKey> {
    let target_size = scaling_cache_target_size(source_size, display_size, quality)?;
    Some(ScalingCacheKey::new(
        image_revision,
        orientation,
        target_size,
        quality,
    ))
}

pub fn scaling_cache_target_size(
    source_size: ImageSize,
    display_size: ImageSize,
    quality: ScalingQuality,
) -> Option<ImageSize> {
    if !should_use_software_resampling(source_size, display_size, quality) {
        return None;
    }

    let mut target_size = ImageSize::new(
        quantized_scaling_cache_extent(display_size.width()),
        quantized_scaling_cache_extent(display_size.height()),
    );

    if quality == ScalingQuality::Balanced {
        target_size = ImageSize::new(
            target_size.width().min(source_size.width()),
            target_size.height().min(source_size.height()),
        );
    }

    if target_size == source_size {
        None
    } else {
        Some(target_size)
    }
}

pub fn should_rebuild_scaling_cache(
    cached_key: Option<ScalingCacheKey>,
    next_key: ScalingCacheKey,
) -> bool {
    cached_key != Some(next_key)
}

pub fn should_use_software_resampling(
    source_size: ImageSize,
    target_size: ImageSize,
    quality: ScalingQuality,
) -> bool {
    if source_size.is_empty() || target_size.is_empty() || source_size == target_size {
        return false;
    }

    match quality {
        ScalingQuality::Nearest => false,
        ScalingQuality::Balanced => {
            axis_scale(target_size.width(), source_size.width())
                < BALANCED_SOFTWARE_DOWNSCALE_THRESHOLD
                || axis_scale(target_size.height(), source_size.height())
                    < BALANCED_SOFTWARE_DOWNSCALE_THRESHOLD
        }
        ScalingQuality::HighQuality => true,
    }
}

fn quantized_scaling_cache_extent(extent: u32) -> u32 {
    if extent <= SCALING_CACHE_TARGET_BUCKET_PIXELS {
        return extent.max(1);
    }

    let bucket_count = extent.saturating_add(SCALING_CACHE_TARGET_BUCKET_PIXELS - 1)
        / SCALING_CACHE_TARGET_BUCKET_PIXELS;
    bucket_count
        .saturating_mul(SCALING_CACHE_TARGET_BUCKET_PIXELS)
        .max(1)
}

fn axis_scale(target_extent: u32, source_extent: u32) -> f64 {
    if source_extent == 0 {
        0.0
    } else {
        f64::from(target_extent) / f64::from(source_extent)
    }
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub struct ViewTransform {
    mode: ViewMode,
    zoom_scale: f64,
    offset: ViewOffset,
}

impl ViewTransform {
    pub const FIT_TO_WINDOW: Self = Self {
        mode: ViewMode::FitToWindow,
        zoom_scale: 1.0,
        offset: ViewOffset::ZERO,
    };

    pub const ACTUAL_SIZE: Self = Self {
        mode: ViewMode::ActualSize,
        zoom_scale: 1.0,
        offset: ViewOffset::ZERO,
    };

    pub fn manual_zoom(zoom_scale: f64, offset: ViewOffset) -> Self {
        Self::manual_zoom_with_settings(zoom_scale, offset, ZoomSettings::default())
    }

    pub fn manual_zoom_with_settings(
        zoom_scale: f64,
        offset: ViewOffset,
        settings: ZoomSettings,
    ) -> Self {
        Self {
            mode: ViewMode::ManualZoom,
            zoom_scale: clamp_zoom_scale_with_settings(zoom_scale, settings),
            offset,
        }
    }

    pub fn mode(self) -> ViewMode {
        self.mode
    }

    pub fn zoom_scale(self) -> f64 {
        self.zoom_scale
    }

    pub fn offset(self) -> ViewOffset {
        self.offset
    }

    pub fn can_pan(self, viewport: ViewportSize, image_size: ImageSize) -> bool {
        self.can_pan_with_settings(viewport, image_size, ZoomSettings::default())
    }

    pub fn can_pan_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> bool {
        if self
            .panning_scale_with_settings(viewport, image_size, settings)
            .is_none()
        {
            return false;
        }

        let Some(geometry) = display_geometry_with_settings(viewport, image_size, self, settings)
        else {
            return false;
        };

        geometry.width > f64::from(viewport.width())
            || geometry.height > f64::from(viewport.height())
    }

    pub fn panning_start_offset(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
    ) -> Option<ViewOffset> {
        self.panning_start_offset_with_settings(viewport, image_size, ZoomSettings::default())
    }

    pub fn panning_start_offset_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Option<ViewOffset> {
        let scale = self.panning_scale_with_settings(viewport, image_size, settings)?;
        if !self.can_pan_with_settings(viewport, image_size, settings) {
            return None;
        }

        let offset = match self.mode {
            ViewMode::FitToWindow | ViewMode::ActualSize => ViewOffset::ZERO,
            ViewMode::ManualZoom => self.offset,
        };
        Some(clamp_view_offset_with_settings(
            viewport, image_size, scale, offset, settings,
        ))
    }

    pub fn pan_to_offset(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        offset: ViewOffset,
    ) -> Self {
        self.pan_to_offset_with_settings(viewport, image_size, offset, ZoomSettings::default())
    }

    pub fn pan_to_offset_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        offset: ViewOffset,
        settings: ZoomSettings,
    ) -> Self {
        let Some(scale) = self.panning_scale_with_settings(viewport, image_size, settings) else {
            return self.constrain_to_viewport_with_settings(viewport, image_size, settings);
        };
        if !self.can_pan_with_settings(viewport, image_size, settings) {
            return self.constrain_to_viewport_with_settings(viewport, image_size, settings);
        }

        let offset = clamp_view_offset_with_settings(viewport, image_size, scale, offset, settings);
        Self::manual_zoom_with_settings(scale, offset, settings)
    }

    pub fn effective_scale(self, viewport: ViewportSize, image_size: ImageSize) -> Option<f64> {
        self.effective_scale_with_settings(viewport, image_size, ZoomSettings::default())
    }

    pub fn effective_scale_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Option<f64> {
        match self.mode {
            ViewMode::FitToWindow => fit_to_window_scale(viewport, image_size),
            ViewMode::ActualSize => Some(1.0),
            ViewMode::ManualZoom => Some(clamp_zoom_scale_for_viewport_with_settings(
                self.zoom_scale,
                viewport,
                image_size,
                settings,
            )),
        }
    }

    pub fn display_rect(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
    ) -> Option<ImageDisplayRect> {
        self.display_rect_with_settings(viewport, image_size, ZoomSettings::default())
    }

    pub fn display_rect_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Option<ImageDisplayRect> {
        let geometry = display_geometry_with_settings(viewport, image_size, self, settings)?;

        Some(ImageDisplayRect {
            x: rounded_i32(geometry.left)?,
            y: rounded_i32(geometry.top)?,
            width: rounded_extent_i32(geometry.width)?,
            height: rounded_extent_i32(geometry.height)?,
        })
    }

    pub fn zoom_at(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        factor: f64,
        anchor: ViewportPoint,
    ) -> Self {
        self.zoom_at_with_settings(
            viewport,
            image_size,
            factor,
            anchor,
            ZoomSettings::default(),
        )
    }

    pub fn zoom_at_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        factor: f64,
        anchor: ViewportPoint,
        settings: ZoomSettings,
    ) -> Self {
        if viewport.is_empty() || image_size.is_empty() || !factor.is_finite() || factor <= 0.0 {
            return self;
        }

        let Some(old_geometry) =
            display_geometry_with_settings(viewport, image_size, self, settings)
        else {
            return self;
        };
        if old_geometry.scale <= 0.0 {
            return self;
        }

        let image_anchor_x = (anchor.x() - old_geometry.left) / old_geometry.scale;
        let image_anchor_y = (anchor.y() - old_geometry.top) / old_geometry.scale;
        let next_scale = clamp_zoom_scale_for_viewport_with_settings(
            old_geometry.scale * factor,
            viewport,
            image_size,
            settings,
        );
        if zoom_scales_equivalent(old_geometry.scale, next_scale) {
            return self.constrain_to_viewport_with_settings(viewport, image_size, settings);
        }

        manual_transform_for_anchor_with_settings(
            viewport,
            image_size,
            next_scale,
            anchor,
            image_anchor_x,
            image_anchor_y,
            settings,
        )
    }

    pub fn resize_viewport(
        self,
        old_viewport: ViewportSize,
        new_viewport: ViewportSize,
        image_size: ImageSize,
    ) -> Self {
        self.resize_viewport_with_settings(
            old_viewport,
            new_viewport,
            image_size,
            ZoomSettings::default(),
        )
    }

    pub fn resize_viewport_with_settings(
        self,
        old_viewport: ViewportSize,
        new_viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Self {
        if self.mode != ViewMode::ManualZoom || old_viewport.is_empty() || new_viewport.is_empty() {
            return self.constrain_to_viewport_with_settings(new_viewport, image_size, settings);
        }

        let Some(old_anchor) = ViewportPoint::center(old_viewport) else {
            return self.constrain_to_viewport_with_settings(new_viewport, image_size, settings);
        };
        let Some(new_anchor) = ViewportPoint::center(new_viewport) else {
            return self.constrain_to_viewport_with_settings(new_viewport, image_size, settings);
        };
        let Some(old_geometry) =
            display_geometry_with_settings(old_viewport, image_size, self, settings)
        else {
            return self.constrain_to_viewport_with_settings(new_viewport, image_size, settings);
        };
        if old_geometry.scale <= 0.0 {
            return self.constrain_to_viewport_with_settings(new_viewport, image_size, settings);
        }

        let image_anchor_x = (old_anchor.x() - old_geometry.left) / old_geometry.scale;
        let image_anchor_y = (old_anchor.y() - old_geometry.top) / old_geometry.scale;

        manual_transform_for_anchor_with_settings(
            new_viewport,
            image_size,
            clamp_zoom_scale_for_viewport_with_settings(
                self.zoom_scale,
                new_viewport,
                image_size,
                settings,
            ),
            new_anchor,
            image_anchor_x,
            image_anchor_y,
            settings,
        )
    }

    pub fn constrain_to_viewport(self, viewport: ViewportSize, image_size: ImageSize) -> Self {
        self.constrain_to_viewport_with_settings(viewport, image_size, ZoomSettings::default())
    }

    pub fn constrain_to_viewport_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Self {
        if self.mode != ViewMode::ManualZoom {
            return match self.mode {
                ViewMode::FitToWindow => Self::FIT_TO_WINDOW,
                ViewMode::ActualSize => Self::ACTUAL_SIZE,
                ViewMode::ManualZoom => self,
            };
        }

        let scale = clamp_zoom_scale_for_viewport_with_settings(
            self.zoom_scale,
            viewport,
            image_size,
            settings,
        );
        let Some((width, height)) = scaled_image_extents(image_size, scale) else {
            return Self::manual_zoom_with_settings(scale, ViewOffset::ZERO, settings);
        };
        let offset = clamp_display_offset(viewport, width, height, self.offset);
        Self::manual_zoom_with_settings(scale, offset, settings)
    }

    fn panning_scale_with_settings(
        self,
        viewport: ViewportSize,
        image_size: ImageSize,
        settings: ZoomSettings,
    ) -> Option<f64> {
        match self.mode {
            ViewMode::FitToWindow => None,
            ViewMode::ActualSize => Some(1.0),
            ViewMode::ManualZoom => Some(clamp_zoom_scale_for_viewport_with_settings(
                self.zoom_scale,
                viewport,
                image_size,
                settings,
            )),
        }
    }
}

pub fn rotate_rgba8_image(image: &Rgba8Image, rotation: ImageRotation) -> Option<Rgba8Image> {
    orient_rgba8_image(image, ImageOrientation::NORMAL.then_rotation(rotation))
}

pub fn rotate_pixel_image(image: &PixelImage, rotation: ImageRotation) -> Option<PixelImage> {
    orient_pixel_image(image, ImageOrientation::NORMAL.then_rotation(rotation))
}

pub fn orient_pixel_image(image: &PixelImage, orientation: ImageOrientation) -> Option<PixelImage> {
    match image {
        PixelImage::Rgb8(image) => orient_rgb8_image(image, orientation).map(PixelImage::Rgb8),
        PixelImage::Rgba8(image) => orient_rgba8_image(image, orientation).map(PixelImage::Rgba8),
        PixelImage::Bgra8(image) => orient_bgra8_image(image, orientation).map(PixelImage::Bgra8),
    }
}

pub fn orient_rgb8_image(image: &Rgb8Image, orientation: ImageOrientation) -> Option<Rgb8Image> {
    if orientation.is_identity() {
        return Some(image.clone());
    }

    let (output_size, output) =
        orient_pixel_bytes(image.size(), image.pixels(), PixelFormat::Rgb8, orientation)?;
    Some(Rgb8Image::new(
        output_size.width(),
        output_size.height(),
        output,
    ))
}

pub fn orient_rgba8_image(image: &Rgba8Image, orientation: ImageOrientation) -> Option<Rgba8Image> {
    if orientation.is_identity() {
        return Some(image.clone());
    }

    let (output_size, output) = orient_pixel_bytes(
        image.size(),
        image.pixels(),
        PixelFormat::Rgba8,
        orientation,
    )?;
    Some(Rgba8Image::new(
        output_size.width(),
        output_size.height(),
        output,
    ))
}

pub fn orient_bgra8_image(image: &Bgra8Image, orientation: ImageOrientation) -> Option<Bgra8Image> {
    if orientation.is_identity() {
        return Some(image.clone());
    }

    let (output_size, output) = orient_pixel_bytes(
        image.size(),
        image.pixels(),
        PixelFormat::Bgra8,
        orientation,
    )?;
    Some(Bgra8Image::new(
        output_size.width(),
        output_size.height(),
        output,
    ))
}

fn orient_pixel_bytes(
    source_size: ImageSize,
    source_pixels: &[u8],
    format: PixelFormat,
    orientation: ImageOrientation,
) -> Option<(ImageSize, Vec<u8>)> {
    let expected_source_len = source_size.pixel_byte_len(format)?;
    if source_pixels.len() != expected_source_len {
        return None;
    }

    if orientation.is_identity() {
        return Some((source_size, source_pixels.to_vec()));
    }

    let output_size = source_size.with_orientation(orientation);
    let output_len = output_size.pixel_byte_len(format)?;
    let mut output = zeroed_pixel_buffer(output_len)?;
    let bytes_per_pixel = format.bytes_per_pixel();
    let source_width = usize::try_from(source_size.width()).ok()?;
    let source_height = usize::try_from(source_size.height()).ok()?;
    let output_width = usize::try_from(output_size.width()).ok()?;

    for source_y in 0..source_height {
        for source_x in 0..source_width {
            let (output_x, output_y) = oriented_pixel_position(
                source_x,
                source_y,
                source_width,
                source_height,
                orientation,
            );
            let source_index = pixel_byte_index(source_width, source_x, source_y, bytes_per_pixel);
            let output_index = pixel_byte_index(output_width, output_x, output_y, bytes_per_pixel);
            output[output_index..output_index + bytes_per_pixel]
                .copy_from_slice(&source_pixels[source_index..source_index + bytes_per_pixel]);
        }
    }

    Some((output_size, output))
}

fn zeroed_pixel_buffer(len: usize) -> Option<Vec<u8>> {
    let mut buffer = Vec::new();
    buffer.try_reserve_exact(len).ok()?;
    buffer.resize(len, 0);
    Some(buffer)
}

fn oriented_pixel_position(
    source_x: usize,
    source_y: usize,
    source_width: usize,
    source_height: usize,
    orientation: ImageOrientation,
) -> (usize, usize) {
    match orientation {
        ImageOrientation::Normal => (source_x, source_y),
        ImageOrientation::FlipHorizontal => (source_width - 1 - source_x, source_y),
        ImageOrientation::Rotate180 => (source_width - 1 - source_x, source_height - 1 - source_y),
        ImageOrientation::FlipVertical => (source_x, source_height - 1 - source_y),
        ImageOrientation::Rotate90FlipHorizontal => (source_y, source_x),
        ImageOrientation::Rotate90 => (source_height - 1 - source_y, source_x),
        ImageOrientation::Rotate270FlipHorizontal => {
            (source_height - 1 - source_y, source_width - 1 - source_x)
        }
        ImageOrientation::Rotate270 => (source_y, source_width - 1 - source_x),
    }
}

fn pixel_byte_index(width: usize, x: usize, y: usize, bytes_per_pixel: usize) -> usize {
    (y * width + x) * bytes_per_pixel
}

pub fn clamp_zoom_scale(scale: f64) -> f64 {
    clamp_zoom_scale_with_settings(scale, ZoomSettings::default())
}

pub fn clamp_zoom_scale_with_settings(scale: f64, settings: ZoomSettings) -> f64 {
    let min_zoom_scale = settings.min_zoom_scale();
    let max_zoom_scale = settings.max_zoom_scale().max(min_zoom_scale);
    if scale.is_nan() || scale.is_sign_negative() {
        min_zoom_scale
    } else if scale.is_infinite() {
        max_zoom_scale
    } else {
        scale.clamp(min_zoom_scale, max_zoom_scale)
    }
}

fn clamp_zoom_scale_for_viewport_with_settings(
    scale: f64,
    viewport: ViewportSize,
    image_size: ImageSize,
    settings: ZoomSettings,
) -> f64 {
    let configured_scale = clamp_zoom_scale_with_settings(scale, settings);
    let Some(viewport_min_scale) =
        minimum_zoom_scale_for_viewport_with_settings(viewport, image_size, settings)
    else {
        return configured_scale;
    };

    configured_scale.max(viewport_min_scale)
}

fn minimum_zoom_scale_for_viewport_with_settings(
    viewport: ViewportSize,
    image_size: ImageSize,
    settings: ZoomSettings,
) -> Option<f64> {
    let fit_scale = fit_to_window_scale(viewport, image_size)?;
    let min_zoom_scale = settings.min_zoom_scale();
    let max_zoom_scale = settings.max_zoom_scale().max(min_zoom_scale);
    let shrink_floor = fit_scale.min(1.0);

    Some(min_zoom_scale.max(shrink_floor).min(max_zoom_scale))
}

pub fn fit_to_window_scale(viewport: ViewportSize, image_size: ImageSize) -> Option<f64> {
    if viewport.is_empty() || image_size.is_empty() {
        return None;
    }

    let width_scale = f64::from(viewport.width()) / f64::from(image_size.width());
    let height_scale = f64::from(viewport.height()) / f64::from(image_size.height());
    Some(width_scale.min(height_scale))
}

pub fn zoom_status_text(transform: ViewTransform) -> String {
    zoom_status_text_with_settings(transform, ZoomSettings::default())
}

pub fn zoom_status_text_with_settings(transform: ViewTransform, settings: ZoomSettings) -> String {
    match transform.mode() {
        ViewMode::FitToWindow => "Fit".to_owned(),
        ViewMode::ActualSize => zoom_percentage_text_with_settings(1.0, settings),
        ViewMode::ManualZoom => {
            zoom_percentage_text_with_settings(transform.zoom_scale(), settings)
        }
    }
}

fn zoom_percentage_text_with_settings(scale: f64, settings: ZoomSettings) -> String {
    let percent = (clamp_zoom_scale_with_settings(scale, settings) * 100.0).round() as u32;
    format!("{percent}%")
}

pub fn clamp_view_offset(
    viewport: ViewportSize,
    image_size: ImageSize,
    scale: f64,
    offset: ViewOffset,
) -> ViewOffset {
    clamp_view_offset_with_settings(viewport, image_size, scale, offset, ZoomSettings::default())
}

pub fn clamp_view_offset_with_settings(
    viewport: ViewportSize,
    image_size: ImageSize,
    scale: f64,
    offset: ViewOffset,
    settings: ZoomSettings,
) -> ViewOffset {
    if viewport.is_empty() || image_size.is_empty() {
        return ViewOffset::ZERO;
    }

    let Some((width, height)) = scaled_image_extents(
        image_size,
        clamp_zoom_scale_for_viewport_with_settings(scale, viewport, image_size, settings),
    ) else {
        return ViewOffset::ZERO;
    };

    clamp_display_offset(viewport, width, height, offset)
}

fn manual_transform_for_anchor_with_settings(
    viewport: ViewportSize,
    image_size: ImageSize,
    scale: f64,
    anchor: ViewportPoint,
    image_anchor_x: f64,
    image_anchor_y: f64,
    settings: ZoomSettings,
) -> ViewTransform {
    let scale = clamp_zoom_scale_for_viewport_with_settings(scale, viewport, image_size, settings);
    let Some((width, height)) = scaled_image_extents(image_size, scale) else {
        return ViewTransform::manual_zoom_with_settings(scale, ViewOffset::ZERO, settings);
    };

    let base_left = centered_axis_origin(f64::from(viewport.width()), width);
    let base_top = centered_axis_origin(f64::from(viewport.height()), height);
    let desired_left = anchor.x() - image_anchor_x * scale;
    let desired_top = anchor.y() - image_anchor_y * scale;
    let offset = ViewOffset::new(desired_left - base_left, desired_top - base_top);

    ViewTransform::manual_zoom_with_settings(
        scale,
        clamp_display_offset(viewport, width, height, offset),
        settings,
    )
}

fn display_geometry_with_settings(
    viewport: ViewportSize,
    image_size: ImageSize,
    transform: ViewTransform,
    settings: ZoomSettings,
) -> Option<DisplayGeometry> {
    if viewport.is_empty() || image_size.is_empty() {
        return None;
    }

    let scale = transform.effective_scale_with_settings(viewport, image_size, settings)?;
    if !scale.is_finite() || scale <= 0.0 {
        return None;
    }

    let (width, height) = scaled_image_extents(image_size, scale)?;
    let offset = if transform.mode == ViewMode::ManualZoom {
        clamp_display_offset(viewport, width, height, transform.offset)
    } else {
        ViewOffset::ZERO
    };

    Some(DisplayGeometry {
        left: centered_axis_origin(f64::from(viewport.width()), width) + offset.x(),
        top: centered_axis_origin(f64::from(viewport.height()), height) + offset.y(),
        width,
        height,
        scale,
    })
}

fn scaled_image_extents(image_size: ImageSize, scale: f64) -> Option<(f64, f64)> {
    let width = f64::from(image_size.width()) * scale;
    let height = f64::from(image_size.height()) * scale;

    if width.is_finite() && height.is_finite() && width > 0.0 && height > 0.0 {
        Some((width.max(1.0), height.max(1.0)))
    } else {
        None
    }
}

fn clamp_display_offset(
    viewport: ViewportSize,
    image_width: f64,
    image_height: f64,
    offset: ViewOffset,
) -> ViewOffset {
    ViewOffset {
        x: constrain_axis_offset(f64::from(viewport.width()), image_width, offset.x()),
        y: constrain_axis_offset(f64::from(viewport.height()), image_height, offset.y()),
    }
}

fn constrain_axis_offset(viewport_extent: f64, image_extent: f64, offset: f64) -> f64 {
    if !image_extent.is_finite() || image_extent <= viewport_extent {
        return 0.0;
    }

    let base_origin = centered_axis_origin(viewport_extent, image_extent);
    let min_origin = viewport_extent - image_extent;
    let max_origin = 0.0;
    let offset = if offset.is_finite() { offset } else { 0.0 };
    let origin = (base_origin + offset).max(min_origin).min(max_origin);
    origin - base_origin
}

fn centered_axis_origin(viewport_extent: f64, image_extent: f64) -> f64 {
    (viewport_extent - image_extent) / 2.0
}

fn zoom_scales_equivalent(left: f64, right: f64) -> bool {
    let tolerance = f64::EPSILON * left.abs().max(right.abs()).max(1.0);
    (left - right).abs() <= tolerance
}

fn rounded_i32(value: f64) -> Option<i32> {
    if value.is_finite() && value >= f64::from(i32::MIN) && value <= f64::from(i32::MAX) {
        Some(value.round() as i32)
    } else {
        None
    }
}

fn rounded_extent_i32(value: f64) -> Option<i32> {
    if value.is_finite() && value > 0.0 && value <= f64::from(i32::MAX) {
        Some((value.round() as i32).max(1))
    } else {
        None
    }
}

#[derive(Debug, Clone, Copy)]
struct DisplayGeometry {
    left: f64,
    top: f64,
    width: f64,
    height: f64,
    scale: f64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum ImageState {
    Empty,
    Loaded(LoadedImage),
}

impl ImageState {
    pub fn has_image(&self) -> bool {
        !matches!(self, Self::Empty)
    }

    pub fn has_animation(&self) -> bool {
        matches!(self, Self::Loaded(image) if image.is_animated())
    }
}

mod export_rules {
    use super::{
        export_suffix::sanitize_export_filename_suffix, ImageRotation, ImageSize, RgbColor,
        DEFAULT_EXPORT_FILENAME_SUFFIX, DEFAULT_EXPORT_QUALITY,
    };
    use std::path::{Path, PathBuf};

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum SupportedImageFormat {
        Jpeg,
        Png,
        Bmp,
        Gif,
        Webp,
        Ico,
        Tiff,
        Tga,
    }

    impl SupportedImageFormat {
        pub const fn display_name(self) -> &'static str {
            match self {
                Self::Jpeg => "JPEG",
                Self::Png => "PNG",
                Self::Bmp => "BMP",
                Self::Gif => "GIF",
                Self::Webp => "WebP",
                Self::Ico => "ICO",
                Self::Tiff => "TIFF",
                Self::Tga => "TGA",
            }
        }

        pub fn from_extension(extension: &str) -> Option<Self> {
            let extension = extension.strip_prefix('.').unwrap_or(extension);

            if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
                Some(Self::Jpeg)
            } else if extension.eq_ignore_ascii_case("png") {
                Some(Self::Png)
            } else if extension.eq_ignore_ascii_case("bmp") {
                Some(Self::Bmp)
            } else if extension.eq_ignore_ascii_case("gif") {
                Some(Self::Gif)
            } else if extension.eq_ignore_ascii_case("webp") {
                Some(Self::Webp)
            } else if extension.eq_ignore_ascii_case("ico") {
                Some(Self::Ico)
            } else if extension.eq_ignore_ascii_case("tif")
                || extension.eq_ignore_ascii_case("tiff")
            {
                Some(Self::Tiff)
            } else if extension.eq_ignore_ascii_case("tga") {
                Some(Self::Tga)
            } else {
                None
            }
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ExportFormat {
        Jpeg,
        Png,
        Bmp,
        Webp,
        Ico,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ExportQualityRange {
        min: u8,
        max: u8,
        default: u8,
    }

    impl ExportQualityRange {
        pub const fn new(min: u8, max: u8, default: u8) -> Self {
            Self { min, max, default }
        }

        pub fn min(self) -> u8 {
            self.min
        }

        pub fn max(self) -> u8 {
            self.max
        }

        pub fn default(self) -> u8 {
            self.default
        }

        pub fn clamp(self, quality: u8) -> u8 {
            quality.clamp(self.min, self.max)
        }
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ExportOrientationPolicy {
        Display,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum ExportAnimationPolicy {
        CurrentFrame,
    }

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub struct ExportOptions {
        format: ExportFormat,
        quality: Option<u8>,
        orientation_policy: ExportOrientationPolicy,
        animation_policy: ExportAnimationPolicy,
        rotation: ImageRotation,
        target_size: Option<ImageSize>,
        remove_metadata: bool,
        jpeg_alpha_background_rgb: RgbColor,
    }

    impl ExportOptions {
        pub fn new(format: ExportFormat, requested_quality: Option<u8>) -> Self {
            let quality = export_quality_range(format)
                .map(|range| range.clamp(requested_quality.unwrap_or_else(|| range.default())));

            Self {
                format,
                quality,
                orientation_policy: ExportOrientationPolicy::Display,
                animation_policy: ExportAnimationPolicy::CurrentFrame,
                rotation: ImageRotation::ZERO,
                target_size: None,
                remove_metadata: false,
                jpeg_alpha_background_rgb: RgbColor::WHITE,
            }
        }

        pub fn with_rotation(mut self, rotation: ImageRotation) -> Self {
            self.rotation = rotation;
            self
        }

        pub fn with_target_size(mut self, target_size: Option<ImageSize>) -> Self {
            self.target_size = if self.format == ExportFormat::Ico {
                None
            } else {
                target_size.filter(|size| !size.is_empty())
            };
            self
        }

        pub fn with_remove_metadata(mut self, remove_metadata: bool) -> Self {
            self.remove_metadata = remove_metadata;
            self
        }

        pub fn with_jpeg_alpha_background_rgb(mut self, color: RgbColor) -> Self {
            self.jpeg_alpha_background_rgb = color;
            self
        }

        pub fn format(self) -> ExportFormat {
            self.format
        }

        pub fn quality(self) -> Option<u8> {
            self.quality
        }

        pub fn orientation_policy(self) -> ExportOrientationPolicy {
            self.orientation_policy
        }

        pub fn animation_policy(self) -> ExportAnimationPolicy {
            self.animation_policy
        }

        pub fn rotation(self) -> ImageRotation {
            self.rotation
        }

        pub fn target_size(self) -> Option<ImageSize> {
            self.target_size
        }

        pub fn remove_metadata(self) -> bool {
            self.remove_metadata
        }

        pub fn jpeg_alpha_background_rgb(self) -> RgbColor {
            self.jpeg_alpha_background_rgb
        }
    }

    pub fn export_size_from_width_preserving_aspect(
        source_size: ImageSize,
        width: u32,
    ) -> Option<ImageSize> {
        let height =
            scaled_dimension_preserving_aspect(source_size.height(), width, source_size.width())?;
        Some(ImageSize::new(width, height))
    }

    pub fn export_size_from_height_preserving_aspect(
        source_size: ImageSize,
        height: u32,
    ) -> Option<ImageSize> {
        let width =
            scaled_dimension_preserving_aspect(source_size.width(), height, source_size.height())?;
        Some(ImageSize::new(width, height))
    }

    fn scaled_dimension_preserving_aspect(
        source_other_axis: u32,
        target_axis: u32,
        source_axis: u32,
    ) -> Option<u32> {
        if source_axis == 0 || source_other_axis == 0 || target_axis == 0 {
            return None;
        }

        let numerator = u64::from(source_other_axis) * u64::from(target_axis);
        let rounded = numerator.saturating_add(u64::from(source_axis) / 2) / u64::from(source_axis);
        u32::try_from(rounded.max(1)).ok()
    }

    pub fn export_format_display_name(format: ExportFormat) -> &'static str {
        match format {
            ExportFormat::Jpeg => "JPEG",
            ExportFormat::Png => "PNG",
            ExportFormat::Bmp => "BMP",
            ExportFormat::Webp => "WebP",
            ExportFormat::Ico => "ICO",
        }
    }

    pub fn export_format_mime_type(format: ExportFormat) -> &'static str {
        match format {
            ExportFormat::Jpeg => "image/jpeg",
            ExportFormat::Png => "image/png",
            ExportFormat::Bmp => "image/bmp",
            ExportFormat::Webp => "image/webp",
            ExportFormat::Ico => "image/vnd.microsoft.icon",
        }
    }

    pub fn export_format_default_extension(format: ExportFormat) -> &'static str {
        match format {
            ExportFormat::Jpeg => "jpg",
            ExportFormat::Png => "png",
            ExportFormat::Bmp => "bmp",
            ExportFormat::Webp => "webp",
            ExportFormat::Ico => "ico",
        }
    }

    pub fn export_format_extensions(format: ExportFormat) -> &'static [&'static str] {
        match format {
            ExportFormat::Jpeg => &["jpg", "jpeg"],
            ExportFormat::Png => &["png"],
            ExportFormat::Bmp => &["bmp"],
            ExportFormat::Webp => &["webp"],
            ExportFormat::Ico => &["ico"],
        }
    }

    pub fn export_quality_range(format: ExportFormat) -> Option<ExportQualityRange> {
        match format {
            ExportFormat::Jpeg => Some(ExportQualityRange::new(1, 100, DEFAULT_EXPORT_QUALITY)),
            ExportFormat::Png | ExportFormat::Bmp | ExportFormat::Webp | ExportFormat::Ico => None,
        }
    }

    pub fn clamp_export_quality(format: ExportFormat, quality: u8) -> Option<u8> {
        export_quality_range(format).map(|range| range.clamp(quality))
    }

    pub fn export_format_for_extension(extension: &str) -> Option<ExportFormat> {
        let extension = extension.strip_prefix('.').unwrap_or(extension);

        if extension.eq_ignore_ascii_case("jpg") || extension.eq_ignore_ascii_case("jpeg") {
            Some(ExportFormat::Jpeg)
        } else if extension.eq_ignore_ascii_case("png") {
            Some(ExportFormat::Png)
        } else if extension.eq_ignore_ascii_case("bmp") {
            Some(ExportFormat::Bmp)
        } else if extension.eq_ignore_ascii_case("webp") {
            Some(ExportFormat::Webp)
        } else if extension.eq_ignore_ascii_case("ico") {
            Some(ExportFormat::Ico)
        } else {
            None
        }
    }

    pub fn export_format_for_path(path: impl AsRef<Path>) -> Option<ExportFormat> {
        path.as_ref()
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(export_format_for_extension)
    }

    pub fn default_export_format_for_source_format(format: SupportedImageFormat) -> ExportFormat {
        match format {
            SupportedImageFormat::Jpeg => ExportFormat::Jpeg,
            SupportedImageFormat::Png => ExportFormat::Png,
            SupportedImageFormat::Bmp => ExportFormat::Bmp,
            SupportedImageFormat::Gif => ExportFormat::Png,
            SupportedImageFormat::Webp => ExportFormat::Webp,
            SupportedImageFormat::Ico => ExportFormat::Ico,
            SupportedImageFormat::Tiff | SupportedImageFormat::Tga => ExportFormat::Png,
        }
    }

    pub fn export_path_with_format_extension(
        path: impl AsRef<Path>,
        format: ExportFormat,
    ) -> PathBuf {
        let path = path.as_ref();
        let extension_matches = path
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(export_format_for_extension)
            .is_some_and(|path_format| path_format == format);

        if extension_matches {
            return path.to_path_buf();
        }

        let mut corrected = path.to_path_buf();
        corrected.set_extension(export_format_default_extension(format));
        corrected
    }

    pub fn suggested_export_path(source_path: impl AsRef<Path>, format: ExportFormat) -> PathBuf {
        suggested_export_path_with_suffix(source_path, format, DEFAULT_EXPORT_FILENAME_SUFFIX)
    }

    pub fn suggested_export_path_with_suffix(
        source_path: impl AsRef<Path>,
        format: ExportFormat,
        suffix: &str,
    ) -> PathBuf {
        let source_path = source_path.as_ref();
        let suffix = sanitize_export_filename_suffix(suffix.to_owned());
        let stem = source_path
            .file_stem()
            .map(|stem| stem.to_string_lossy())
            .filter(|stem| !stem.is_empty())
            .unwrap_or_else(|| "image".into());
        let file_name = format!(
            "{}{}.{}",
            stem,
            suffix,
            export_format_default_extension(format)
        );

        if let Some(parent) = source_path
            .parent()
            .filter(|parent| !parent.as_os_str().is_empty())
        {
            parent.join(file_name)
        } else {
            PathBuf::from(file_name)
        }
    }

    pub fn supported_image_format_for_path(path: impl AsRef<Path>) -> Option<SupportedImageFormat> {
        path.as_ref()
            .extension()
            .and_then(|extension| extension.to_str())
            .and_then(SupportedImageFormat::from_extension)
    }

    pub fn is_supported_image_path(path: impl AsRef<Path>) -> bool {
        supported_image_format_for_path(path).is_some()
    }

    pub fn first_supported_image_path<'a>(
        paths: impl IntoIterator<Item = &'a Path>,
    ) -> Option<&'a Path> {
        paths.into_iter().find(|path| is_supported_image_path(path))
    }
}

pub use export_rules::{
    clamp_export_quality, default_export_format_for_source_format, export_format_default_extension,
    export_format_display_name, export_format_extensions, export_format_for_extension,
    export_format_for_path, export_format_mime_type, export_path_with_format_extension,
    export_quality_range, export_size_from_height_preserving_aspect,
    export_size_from_width_preserving_aspect, first_supported_image_path, is_supported_image_path,
    suggested_export_path, suggested_export_path_with_suffix, supported_image_format_for_path,
    ExportAnimationPolicy, ExportFormat, ExportOptions, ExportOrientationPolicy,
    ExportQualityRange, SupportedImageFormat,
};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ImageNavigationDirection {
    Previous,
    Next,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum AnimationCommand {
    TogglePlayback,
    StepFrame(AnimationFrameStepDirection),
    FirstFrame,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum CommandContext {
    StaticImage,
    AnimationImage,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Command {
    OpenImage,
    ExportImage,
    CopyImageToClipboard,
    Navigate(ImageNavigationDirection),
    Animation(AnimationCommand),
    ContextualSpace,
    ZoomIn,
    ZoomOut,
    ActualSize,
    FitToWindow,
    RotateClockwise,
    RotateCounterClockwise,
    ToggleFullscreen,
    OpenAbout,
    OpenSettings,
    ExitFullscreenOrQuit,
    Quit,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyCode {
    C,
    O,
    P,
    Q,
    R,
    S,
    Digit0,
    Digit1,
    Equals,
    Minus,
    NumpadAdd,
    NumpadSubtract,
    Left,
    Right,
    Space,
    Backspace,
    PageUp,
    PageDown,
    Home,
    BracketLeft,
    BracketRight,
    F4,
    F11,
    Enter,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyModifiers {
    control: bool,
    shift: bool,
    alt: bool,
}

impl KeyModifiers {
    pub const NONE: Self = Self {
        control: false,
        shift: false,
        alt: false,
    };

    pub const fn new(control: bool, shift: bool, alt: bool) -> Self {
        Self {
            control,
            shift,
            alt,
        }
    }

    pub fn control(self) -> bool {
        self.control
    }

    pub fn shift(self) -> bool {
        self.shift
    }

    pub fn alt(self) -> bool {
        self.alt
    }

    fn only_control(self) -> bool {
        self.control && !self.shift && !self.alt
    }

    fn only_alt(self) -> bool {
        !self.control && !self.shift && self.alt
    }

    fn control_without_alt(self) -> bool {
        self.control && !self.alt
    }

    fn has_command_modifier(self) -> bool {
        self.control || self.alt
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct KeyInput {
    key: KeyCode,
    modifiers: KeyModifiers,
}

impl KeyInput {
    pub const fn new(key: KeyCode, modifiers: KeyModifiers) -> Self {
        Self { key, modifiers }
    }

    pub fn key(self) -> KeyCode {
        self.key
    }

    pub fn modifiers(self) -> KeyModifiers {
        self.modifiers
    }
}

pub fn command_for_key_input(input: KeyInput) -> Option<Command> {
    command_for_key_input_with_context(input, CommandContext::StaticImage)
}

pub fn command_for_key_input_with_context(
    input: KeyInput,
    context: CommandContext,
) -> Option<Command> {
    command_for_key_input_raw(input).map(|command| resolve_contextual_command(command, context))
}

pub fn resolve_contextual_command(command: Command, context: CommandContext) -> Command {
    match (command, context) {
        (Command::ContextualSpace, CommandContext::AnimationImage) => {
            Command::Animation(AnimationCommand::TogglePlayback)
        }
        (Command::ContextualSpace, CommandContext::StaticImage) => {
            Command::Navigate(ImageNavigationDirection::Next)
        }
        (command, _) => command,
    }
}

fn command_for_key_input_raw(input: KeyInput) -> Option<Command> {
    let key = input.key();
    let modifiers = input.modifiers();

    if key == KeyCode::O && modifiers.only_control() {
        return Some(Command::OpenImage);
    }
    if key == KeyCode::S && modifiers.control_without_alt() {
        return Some(Command::ExportImage);
    }
    if key == KeyCode::C && modifiers.only_control() {
        return Some(Command::CopyImageToClipboard);
    }
    if key == KeyCode::Enter && modifiers.only_alt() {
        return Some(Command::ToggleFullscreen);
    }
    if key == KeyCode::F4 && modifiers.only_alt() {
        return Some(Command::Quit);
    }
    if modifiers.has_command_modifier() {
        return None;
    }

    match key {
        KeyCode::Right | KeyCode::PageDown => {
            Some(Command::Navigate(ImageNavigationDirection::Next))
        }
        KeyCode::Space => Some(Command::ContextualSpace),
        KeyCode::Left | KeyCode::Backspace | KeyCode::PageUp => {
            Some(Command::Navigate(ImageNavigationDirection::Previous))
        }
        KeyCode::P => Some(Command::Animation(AnimationCommand::TogglePlayback)),
        KeyCode::BracketLeft => Some(Command::Animation(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Previous,
        ))),
        KeyCode::BracketRight => Some(Command::Animation(AnimationCommand::StepFrame(
            AnimationFrameStepDirection::Next,
        ))),
        KeyCode::Home => Some(Command::Animation(AnimationCommand::FirstFrame)),
        KeyCode::Equals | KeyCode::NumpadAdd => Some(Command::ZoomIn),
        KeyCode::Minus | KeyCode::NumpadSubtract => Some(Command::ZoomOut),
        KeyCode::Digit1 => Some(Command::ActualSize),
        KeyCode::Digit0 => Some(Command::FitToWindow),
        KeyCode::R => {
            if modifiers.shift() {
                Some(Command::RotateCounterClockwise)
            } else {
                Some(Command::RotateClockwise)
            }
        }
        KeyCode::F11 => Some(Command::ToggleFullscreen),
        KeyCode::Escape => Some(Command::ExitFullscreenOrQuit),
        KeyCode::Q => Some(Command::Quit),
        KeyCode::C | KeyCode::O | KeyCode::S | KeyCode::F4 | KeyCode::Enter => None,
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageFolder {
    snapshot: Arc<ImageFolderSnapshot>,
    current_index: Option<usize>,
}

pub(crate) const MAX_IMAGE_FOLDER_SNAPSHOT_PATHS: usize = 20_000;

#[derive(Debug, PartialEq, Eq)]
struct ImageFolderSnapshot {
    paths: Vec<PathBuf>,
    path_indexes: HashMap<u64, PathIndex>,
    file_name_indexes: HashMap<String, usize>,
}

#[derive(Debug, PartialEq, Eq)]
enum PathIndex {
    Single(usize),
    Multiple(Vec<usize>),
}

impl ImageFolderSnapshot {
    fn empty() -> Self {
        Self {
            paths: Vec::new(),
            path_indexes: HashMap::new(),
            file_name_indexes: HashMap::new(),
        }
    }

    fn from_entries(entries: Vec<ImageFolderPathEntry>) -> Self {
        let mut paths = Vec::with_capacity(entries.len());
        let mut path_indexes = HashMap::with_capacity(entries.len());
        let mut file_name_indexes = HashMap::with_capacity(entries.len());

        for (index, entry) in entries.into_iter().enumerate() {
            insert_path_index(&mut path_indexes, &paths, &entry.path, index);
            if entry.has_file_name {
                file_name_indexes
                    .entry(entry.normalized_file_name)
                    .or_insert(index);
            }
            paths.push(entry.path);
        }

        Self {
            paths,
            path_indexes,
            file_name_indexes,
        }
    }
}

impl ImageFolder {
    pub fn empty() -> Self {
        Self {
            snapshot: Arc::new(ImageFolderSnapshot::empty()),
            current_index: None,
        }
    }

    pub fn from_paths(
        current_path: impl AsRef<Path>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let entries = sorted_supported_image_path_entries(paths);
        Self::from_sorted_entries(current_path.as_ref(), entries)
    }

    pub(crate) fn from_supported_paths(
        current_path: impl AsRef<Path>,
        paths: impl IntoIterator<Item = PathBuf>,
    ) -> Self {
        let entries = sorted_image_path_entries(paths);
        Self::from_sorted_entries(current_path.as_ref(), entries)
    }

    fn from_sorted_entries(current_path: &Path, entries: Vec<ImageFolderPathEntry>) -> Self {
        let snapshot = Arc::new(ImageFolderSnapshot::from_entries(entries));
        let current_index = current_image_index(
            &snapshot.paths,
            &snapshot.path_indexes,
            &snapshot.file_name_indexes,
            current_path,
        );

        Self {
            snapshot,
            current_index,
        }
    }

    pub fn retarget_current_path(&mut self, current_path: impl AsRef<Path>) -> bool {
        let Some(current_index) = current_image_index(
            &self.snapshot.paths,
            &self.snapshot.path_indexes,
            &self.snapshot.file_name_indexes,
            current_path.as_ref(),
        ) else {
            return false;
        };
        self.current_index = Some(current_index);
        true
    }

    pub fn retargeted_current_path(&self, current_path: impl AsRef<Path>) -> Option<Self> {
        let current_index = current_image_index(
            &self.snapshot.paths,
            &self.snapshot.path_indexes,
            &self.snapshot.file_name_indexes,
            current_path.as_ref(),
        )?;
        Some(Self {
            snapshot: Arc::clone(&self.snapshot),
            current_index: Some(current_index),
        })
    }

    pub fn paths(&self) -> &[PathBuf] {
        &self.snapshot.paths
    }

    pub fn current_index(&self) -> Option<usize> {
        self.current_index
    }

    pub fn len(&self) -> usize {
        self.snapshot.paths.len()
    }

    pub fn is_empty(&self) -> bool {
        self.snapshot.paths.is_empty()
    }

    pub fn navigation_path(&self, direction: ImageNavigationDirection) -> Option<&Path> {
        self.navigation_path_with_settings(direction, NavigationSettings::default())
    }

    pub fn navigation_path_with_settings(
        &self,
        direction: ImageNavigationDirection,
        settings: NavigationSettings,
    ) -> Option<&Path> {
        self.navigation_path_for_attempt(direction, settings, 0)
    }

    pub fn navigation_path_for_attempt(
        &self,
        direction: ImageNavigationDirection,
        settings: NavigationSettings,
        attempt_index: usize,
    ) -> Option<&Path> {
        let next_index = navigation_index_for_attempt(
            self.snapshot.paths.len(),
            self.current_index,
            direction,
            settings.wrap_navigation(),
            attempt_index,
        )?;
        self.snapshot.paths.get(next_index).map(PathBuf::as_path)
    }
}

pub fn sorted_supported_image_paths(paths: impl IntoIterator<Item = PathBuf>) -> Vec<PathBuf> {
    sorted_supported_image_path_entries(paths)
        .into_iter()
        .map(|entry| entry.path)
        .collect()
}

fn sorted_supported_image_path_entries(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<ImageFolderPathEntry> {
    sorted_image_path_entries(
        paths
            .into_iter()
            .filter(|path| is_supported_image_path(path)),
    )
}

fn sorted_image_path_entries(
    paths: impl IntoIterator<Item = PathBuf>,
) -> Vec<ImageFolderPathEntry> {
    let mut entries = Vec::new();
    for path in paths.into_iter().take(MAX_IMAGE_FOLDER_SNAPSHOT_PATHS) {
        entries.push(ImageFolderPathEntry::new(path));
    }
    entries.sort_by(|left, right| left.normalized_file_name.cmp(&right.normalized_file_name));
    entries
}

#[derive(Debug, PartialEq, Eq)]
struct ImageFolderPathEntry {
    path: PathBuf,
    normalized_file_name: String,
    has_file_name: bool,
}

impl ImageFolderPathEntry {
    fn new(path: PathBuf) -> Self {
        let file_name = path.file_name();
        Self {
            normalized_file_name: normalized_os_str(file_name.unwrap_or(path.as_os_str())),
            has_file_name: file_name.is_some(),
            path,
        }
    }
}

fn insert_path_index(
    path_indexes: &mut HashMap<u64, PathIndex>,
    paths: &[PathBuf],
    path: &Path,
    index: usize,
) {
    let key = path_index_key(path);
    match path_indexes.get_mut(&key) {
        Some(path_index) => match path_index {
            PathIndex::Single(existing_index) => {
                let first_index = *existing_index;
                let is_same_path = paths
                    .get(first_index)
                    .is_some_and(|existing_path| existing_path.as_path() == path);
                if !is_same_path {
                    *path_index = PathIndex::Multiple(vec![first_index, index]);
                }
            }
            PathIndex::Multiple(indexes) => {
                if !indexes.iter().any(|existing_index| {
                    paths
                        .get(*existing_index)
                        .is_some_and(|existing_path| existing_path.as_path() == path)
                }) {
                    indexes.push(index);
                }
            }
        },
        None => {
            path_indexes.insert(key, PathIndex::Single(index));
        }
    }
}

pub fn navigation_index(
    len: usize,
    current_index: Option<usize>,
    direction: ImageNavigationDirection,
) -> Option<usize> {
    navigation_index_with_settings(len, current_index, direction, NavigationSettings::default())
}

pub fn navigation_index_with_settings(
    len: usize,
    current_index: Option<usize>,
    direction: ImageNavigationDirection,
    settings: NavigationSettings,
) -> Option<usize> {
    navigation_index_for_attempt(len, current_index, direction, settings.wrap_navigation(), 0)
}

fn navigation_index_for_attempt(
    len: usize,
    current_index: Option<usize>,
    direction: ImageNavigationDirection,
    wrap_navigation: bool,
    attempt_index: usize,
) -> Option<usize> {
    let current_index = current_index?;
    if len <= 1 || current_index >= len {
        return None;
    }
    let step_count = attempt_index.checked_add(1)?;
    if step_count >= len {
        return None;
    }

    match direction {
        ImageNavigationDirection::Previous => {
            if wrap_navigation {
                Some((current_index + len - step_count) % len)
            } else {
                current_index.checked_sub(step_count)
            }
        }
        ImageNavigationDirection::Next => {
            if wrap_navigation {
                Some((current_index + step_count) % len)
            } else {
                current_index
                    .checked_add(step_count)
                    .filter(|index| *index < len)
            }
        }
    }
}

fn current_image_index(
    paths: &[PathBuf],
    path_indexes: &HashMap<u64, PathIndex>,
    file_name_indexes: &HashMap<String, usize>,
    current_path: &Path,
) -> Option<usize> {
    path_index_for_path(paths, path_indexes, current_path).or_else(|| {
        current_path.file_name().and_then(|name| {
            let current_name = normalized_os_str(name);
            file_name_indexes.get(&current_name).copied()
        })
    })
}

fn path_index_for_path(
    paths: &[PathBuf],
    path_indexes: &HashMap<u64, PathIndex>,
    current_path: &Path,
) -> Option<usize> {
    let key = path_index_key(current_path);
    match path_indexes.get(&key)? {
        PathIndex::Single(index) => paths
            .get(*index)
            .filter(|path| path.as_path() == current_path)
            .map(|_| *index),
        PathIndex::Multiple(indexes) => indexes.iter().copied().find(|index| {
            paths
                .get(*index)
                .is_some_and(|path| path.as_path() == current_path)
        }),
    }
}

fn path_index_key(path: &Path) -> u64 {
    let mut hasher = DefaultHasher::new();
    path.hash(&mut hasher);
    hasher.finish()
}

fn normalized_os_str(value: &std::ffi::OsStr) -> String {
    value.to_string_lossy().to_lowercase()
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LoadedImage {
    pixels: PixelImage,
    source_size: ImageSize,
    buffer_kind: ImageBufferKind,
    metadata: ImageMetadata,
    render_ready: Option<RenderReadyImage>,
    animation: Option<AnimationPlayback>,
}

impl LoadedImage {
    pub fn new(rgba8: Rgba8Image, metadata: ImageMetadata) -> Self {
        Self::from_pixels(PixelImage::from(rgba8), metadata)
    }

    pub fn from_pixels(pixels: PixelImage, metadata: ImageMetadata) -> Self {
        let source_size = pixels.size();
        Self {
            pixels,
            source_size,
            buffer_kind: ImageBufferKind::FullResolution,
            metadata,
            render_ready: None,
            animation: None,
        }
    }

    pub fn from_preview(
        rgba8: Rgba8Image,
        source_size: ImageSize,
        metadata: ImageMetadata,
    ) -> Self {
        Self::from_preview_pixels(PixelImage::from(rgba8), source_size, metadata)
    }

    pub fn from_preview_pixels(
        pixels: PixelImage,
        source_size: ImageSize,
        metadata: ImageMetadata,
    ) -> Self {
        Self {
            pixels,
            source_size,
            buffer_kind: ImageBufferKind::Preview,
            metadata,
            render_ready: None,
            animation: None,
        }
    }

    pub fn from_animation(
        rgba8: Rgba8Image,
        source_size: ImageSize,
        buffer_kind: ImageBufferKind,
        metadata: ImageMetadata,
        animation: AnimationPlayback,
    ) -> Self {
        Self {
            pixels: PixelImage::from(rgba8),
            source_size,
            buffer_kind,
            metadata,
            render_ready: None,
            animation: Some(animation),
        }
    }

    pub fn pixels(&self) -> &PixelImage {
        &self.pixels
    }

    pub fn rgba8(&self) -> Option<&Rgba8Image> {
        self.pixels.as_rgba8()
    }

    pub fn source_size(&self) -> ImageSize {
        self.source_size
    }

    pub fn buffer_kind(&self) -> ImageBufferKind {
        self.buffer_kind
    }

    pub fn has_full_resolution(&self) -> bool {
        self.buffer_kind == ImageBufferKind::FullResolution
    }

    pub fn render_ready_image(&self) -> Option<&RenderReadyImage> {
        self.render_ready.as_ref()
    }

    pub fn set_render_ready_image(&mut self, render_ready: Option<RenderReadyImage>) {
        self.render_ready = render_ready;
    }

    pub fn resident_byte_len(&self) -> usize {
        self.pixels.byte_len().saturating_add(
            self.render_ready
                .as_ref()
                .map(RenderReadyImage::byte_len)
                .unwrap_or(0),
        )
    }

    pub fn replace_with_full_resolution(&mut self, pixels: PixelImage) -> bool {
        if self.is_animated() {
            return false;
        }
        if pixels.size() != self.source_size {
            return false;
        }

        self.pixels = pixels;
        self.buffer_kind = ImageBufferKind::FullResolution;
        self.render_ready = None;
        true
    }

    pub fn metadata(&self) -> &ImageMetadata {
        &self.metadata
    }

    pub fn animation(&self) -> Option<&AnimationPlayback> {
        self.animation.as_ref()
    }

    pub fn is_animated(&self) -> bool {
        self.animation.is_some()
    }

    pub fn set_animation_playback(&mut self, playback: AnimationPlayback) -> bool {
        let Some(animation) = &self.animation else {
            return false;
        };
        if animation.frame_count() != playback.frame_count() {
            return false;
        }

        self.animation = Some(playback);
        true
    }

    pub fn replace_current_rgba8(&mut self, rgba8: Rgba8Image) -> Option<Rgba8Image> {
        if rgba8.size() != self.pixels.size() {
            return None;
        }

        self.render_ready = None;
        std::mem::replace(&mut self.pixels, PixelImage::from(rgba8)).into_rgba8()
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RenderReadyImage {
    pixels: PixelImage,
    viewport: ViewportSize,
    view_mode: ViewMode,
    preferred_scaling_quality: ScalingQuality,
    orientation: ImageOrientation,
    display_rect: ImageDisplayRect,
    scaling_quality: ScalingQuality,
}

impl RenderReadyImage {
    pub fn new(
        pixels: PixelImage,
        viewport: ViewportSize,
        view_mode: ViewMode,
        preferred_scaling_quality: ScalingQuality,
        orientation: ImageOrientation,
        display_rect: ImageDisplayRect,
        scaling_quality: ScalingQuality,
    ) -> Option<Self> {
        if display_rect.size()? != pixels.size() {
            return None;
        }

        Some(Self {
            pixels,
            viewport,
            view_mode,
            preferred_scaling_quality,
            orientation,
            display_rect,
            scaling_quality,
        })
    }

    pub fn pixels(&self) -> &PixelImage {
        &self.pixels
    }

    pub fn viewport(&self) -> ViewportSize {
        self.viewport
    }

    pub fn view_mode(&self) -> ViewMode {
        self.view_mode
    }

    pub fn preferred_scaling_quality(&self) -> ScalingQuality {
        self.preferred_scaling_quality
    }

    pub fn orientation(&self) -> ImageOrientation {
        self.orientation
    }

    pub fn display_rect(&self) -> ImageDisplayRect {
        self.display_rect
    }

    pub fn scaling_quality(&self) -> ScalingQuality {
        self.scaling_quality
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.byte_len()
    }

    pub fn matches_render(
        &self,
        viewport: ViewportSize,
        view_mode: ViewMode,
        preferred_scaling_quality: ScalingQuality,
        orientation: ImageOrientation,
        display_rect: ImageDisplayRect,
        scaling_quality: ScalingQuality,
    ) -> bool {
        self.viewport == viewport
            && self.view_mode == view_mode
            && self.preferred_scaling_quality == preferred_scaling_quality
            && self.orientation == orientation
            && self.display_rect == display_rect
            && self.scaling_quality == scaling_quality
            && display_rect.size() == Some(self.pixels.size())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgb8Image {
    width: u32,
    height: u32,
    pixels: Arc<Vec<u8>>,
}

impl Rgb8Image {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels: Arc::new(pixels),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> ImageSize {
        ImageSize::new(self.width, self.height)
    }

    pub fn pixel_format(&self) -> PixelFormat {
        PixelFormat::Rgb8
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_slice()
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    pub fn into_raw(self) -> Vec<u8> {
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => pixels,
            Err(pixels) => pixels.as_ref().clone(),
        }
    }

    pub fn try_into_raw(self) -> Result<Vec<u8>, Self> {
        let width = self.width;
        let height = self.height;
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => Ok(pixels),
            Err(pixels) => Err(Self {
                width,
                height,
                pixels,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Rgba8Image {
    width: u32,
    height: u32,
    pixels: Arc<Vec<u8>>,
}

impl Rgba8Image {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels: Arc::new(pixels),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> ImageSize {
        ImageSize::new(self.width, self.height)
    }

    pub fn pixel_format(&self) -> PixelFormat {
        PixelFormat::Rgba8
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_slice()
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    pub fn into_raw(self) -> Vec<u8> {
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => pixels,
            Err(pixels) => pixels.as_ref().clone(),
        }
    }

    pub fn try_into_raw(self) -> Result<Vec<u8>, Self> {
        let width = self.width;
        let height = self.height;
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => Ok(pixels),
            Err(pixels) => Err(Self {
                width,
                height,
                pixels,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Bgra8Image {
    width: u32,
    height: u32,
    pixels: Arc<Vec<u8>>,
}

impl Bgra8Image {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>) -> Self {
        Self {
            width,
            height,
            pixels: Arc::new(pixels),
        }
    }

    pub fn width(&self) -> u32 {
        self.width
    }

    pub fn height(&self) -> u32 {
        self.height
    }

    pub fn size(&self) -> ImageSize {
        ImageSize::new(self.width, self.height)
    }

    pub fn pixel_format(&self) -> PixelFormat {
        PixelFormat::Bgra8
    }

    pub fn pixels(&self) -> &[u8] {
        self.pixels.as_slice()
    }

    pub fn byte_len(&self) -> usize {
        self.pixels.len()
    }

    pub fn into_raw(self) -> Vec<u8> {
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => pixels,
            Err(pixels) => pixels.as_ref().clone(),
        }
    }

    pub fn try_into_raw(self) -> Result<Vec<u8>, Self> {
        let width = self.width;
        let height = self.height;
        match Arc::try_unwrap(self.pixels) {
            Ok(pixels) => Ok(pixels),
            Err(pixels) => Err(Self {
                width,
                height,
                pixels,
            }),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum PixelImage {
    Rgb8(Rgb8Image),
    Rgba8(Rgba8Image),
    Bgra8(Bgra8Image),
}

impl PixelImage {
    pub fn new(width: u32, height: u32, pixels: Vec<u8>, format: PixelFormat) -> Self {
        match format {
            PixelFormat::Rgb8 => Self::Rgb8(Rgb8Image::new(width, height, pixels)),
            PixelFormat::Rgba8 => Self::Rgba8(Rgba8Image::new(width, height, pixels)),
            PixelFormat::Bgra8 => Self::Bgra8(Bgra8Image::new(width, height, pixels)),
        }
    }

    pub fn pixel_format(&self) -> PixelFormat {
        match self {
            Self::Rgb8(_) => PixelFormat::Rgb8,
            Self::Rgba8(_) => PixelFormat::Rgba8,
            Self::Bgra8(_) => PixelFormat::Bgra8,
        }
    }

    pub fn width(&self) -> u32 {
        match self {
            Self::Rgb8(image) => image.width(),
            Self::Rgba8(image) => image.width(),
            Self::Bgra8(image) => image.width(),
        }
    }

    pub fn height(&self) -> u32 {
        match self {
            Self::Rgb8(image) => image.height(),
            Self::Rgba8(image) => image.height(),
            Self::Bgra8(image) => image.height(),
        }
    }

    pub fn size(&self) -> ImageSize {
        ImageSize::new(self.width(), self.height())
    }

    pub fn pixels(&self) -> &[u8] {
        match self {
            Self::Rgb8(image) => image.pixels(),
            Self::Rgba8(image) => image.pixels(),
            Self::Bgra8(image) => image.pixels(),
        }
    }

    pub fn byte_len(&self) -> usize {
        self.pixels().len()
    }

    pub fn expected_byte_len(&self) -> Option<usize> {
        self.size().pixel_byte_len(self.pixel_format())
    }

    pub fn has_alpha(&self) -> bool {
        self.pixel_format().has_alpha()
    }

    pub fn is_valid(&self) -> bool {
        self.expected_byte_len()
            .is_some_and(|expected_len| expected_len == self.pixels().len())
    }

    pub fn as_rgba8(&self) -> Option<&Rgba8Image> {
        match self {
            Self::Rgba8(image) => Some(image),
            Self::Rgb8(_) | Self::Bgra8(_) => None,
        }
    }

    pub fn to_rgba8(&self) -> Option<Rgba8Image> {
        match self {
            Self::Rgb8(image) => rgb8_slice_to_rgba8(image.width(), image.height(), image.pixels()),
            Self::Rgba8(image) => Some(image.clone()),
            Self::Bgra8(image) => {
                bgra8_slice_to_rgba8(image.width(), image.height(), image.pixels())
            }
        }
    }

    pub fn into_rgba8(self) -> Option<Rgba8Image> {
        match self {
            Self::Rgb8(image) => {
                let width = image.width();
                let height = image.height();
                rgb8_vec_to_rgba8(width, height, image.into_raw())
            }
            Self::Rgba8(image) => Some(image),
            Self::Bgra8(image) => {
                let width = image.width();
                let height = image.height();
                bgra8_vec_to_rgba8(width, height, image.into_raw())
            }
        }
    }

    pub fn into_raw(self) -> Vec<u8> {
        match self {
            Self::Rgb8(image) => image.into_raw(),
            Self::Rgba8(image) => image.into_raw(),
            Self::Bgra8(image) => image.into_raw(),
        }
    }
}

impl From<Rgb8Image> for PixelImage {
    fn from(image: Rgb8Image) -> Self {
        Self::Rgb8(image)
    }
}

impl From<Rgba8Image> for PixelImage {
    fn from(image: Rgba8Image) -> Self {
        Self::Rgba8(image)
    }
}

impl From<Bgra8Image> for PixelImage {
    fn from(image: Bgra8Image) -> Self {
        Self::Bgra8(image)
    }
}

fn rgb8_slice_to_rgba8(width: u32, height: u32, rgb8: &[u8]) -> Option<Rgba8Image> {
    let expected_len = ImageSize::new(width, height).pixel_byte_len(PixelFormat::Rgb8)?;
    if rgb8.len() != expected_len {
        return None;
    }
    let mut rgba8 = Vec::new();
    rgba8
        .try_reserve_exact(ImageSize::new(width, height).rgba8_byte_len()?)
        .ok()?;
    for pixel in rgb8.chunks_exact(RGB8_BYTES_PER_PIXEL) {
        rgba8.extend_from_slice(pixel);
        rgba8.push(255);
    }
    Some(Rgba8Image::new(width, height, rgba8))
}

fn rgb8_vec_to_rgba8(width: u32, height: u32, rgb8: Vec<u8>) -> Option<Rgba8Image> {
    rgb8_slice_to_rgba8(width, height, &rgb8)
}

fn bgra8_slice_to_rgba8(width: u32, height: u32, bgra8: &[u8]) -> Option<Rgba8Image> {
    let expected_len = ImageSize::new(width, height).pixel_byte_len(PixelFormat::Bgra8)?;
    if bgra8.len() != expected_len {
        return None;
    }
    let mut rgba8 = Vec::new();
    rgba8
        .try_reserve_exact(ImageSize::new(width, height).rgba8_byte_len()?)
        .ok()?;
    for pixel in bgra8.chunks_exact(BGRA8_BYTES_PER_PIXEL) {
        rgba8.push(pixel[2]);
        rgba8.push(pixel[1]);
        rgba8.push(pixel[0]);
        rgba8.push(pixel[3]);
    }
    Some(Rgba8Image::new(width, height, rgba8))
}

fn bgra8_vec_to_rgba8(width: u32, height: u32, mut bgra8: Vec<u8>) -> Option<Rgba8Image> {
    let expected_len = ImageSize::new(width, height).pixel_byte_len(PixelFormat::Bgra8)?;
    if bgra8.len() != expected_len {
        return None;
    }
    for pixel in bgra8.chunks_exact_mut(BGRA8_BYTES_PER_PIXEL) {
        pixel.swap(0, 2);
    }
    Some(Rgba8Image::new(width, height, bgra8))
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct ImageFileVersion {
    file_size: u64,
    modified: SystemTime,
}

impl ImageFileVersion {
    pub fn new(file_size: u64, modified: SystemTime) -> Self {
        Self {
            file_size,
            modified,
        }
    }

    pub fn file_size(self) -> u64 {
        self.file_size
    }

    pub fn modified(self) -> SystemTime {
        self.modified
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ImageMetadata {
    path: PathBuf,
    file_size: u64,
    file_version: Option<ImageFileVersion>,
    format: SupportedImageFormat,
    exif_orientation: ImageOrientation,
}

impl ImageMetadata {
    pub fn new(path: PathBuf, file_size: u64, format: SupportedImageFormat) -> Self {
        Self::with_exif_orientation(path, file_size, format, ImageOrientation::NORMAL)
    }

    pub fn with_file_version(
        path: PathBuf,
        file_version: ImageFileVersion,
        format: SupportedImageFormat,
    ) -> Self {
        Self::with_file_version_and_exif_orientation(
            path,
            file_version,
            format,
            ImageOrientation::NORMAL,
        )
    }

    pub fn with_exif_orientation(
        path: PathBuf,
        file_size: u64,
        format: SupportedImageFormat,
        exif_orientation: ImageOrientation,
    ) -> Self {
        Self {
            path,
            file_size,
            file_version: None,
            format,
            exif_orientation,
        }
    }

    pub fn with_file_version_and_exif_orientation(
        path: PathBuf,
        file_version: ImageFileVersion,
        format: SupportedImageFormat,
        exif_orientation: ImageOrientation,
    ) -> Self {
        Self {
            path,
            file_size: file_version.file_size(),
            file_version: Some(file_version),
            format,
            exif_orientation,
        }
    }

    pub fn path(&self) -> &Path {
        &self.path
    }

    pub fn file_size(&self) -> u64 {
        self.file_size
    }

    pub fn file_version(&self) -> Option<ImageFileVersion> {
        self.file_version
    }

    pub fn format(&self) -> SupportedImageFormat {
        self.format
    }

    pub fn exif_orientation(&self) -> ImageOrientation {
        self.exif_orientation
    }

    pub fn file_name(&self) -> Option<&str> {
        self.path.file_name().and_then(|name| name.to_str())
    }

    pub fn file_name_for_display(&self) -> String {
        file_name_for_display(&self.path)
    }
}

pub fn image_info_text(image: &LoadedImage) -> String {
    image_display_info_text(image, ImageRotation::ZERO)
}

pub fn image_status_text(
    image: &LoadedImage,
    transform: ViewTransform,
    user_rotation: ImageRotation,
) -> String {
    image_status_text_with_settings(
        image,
        transform,
        user_rotation,
        ZoomSettings::default(),
        true,
    )
}

pub fn image_status_text_with_settings(
    image: &LoadedImage,
    transform: ViewTransform,
    user_rotation: ImageRotation,
    zoom_settings: ZoomSettings,
    detailed: bool,
) -> String {
    if !detailed {
        return format!(
            "{} | {}",
            image.metadata().file_name_for_display(),
            zoom_status_text_with_settings(transform, zoom_settings)
        );
    }

    format!(
        "{} | {}",
        image_display_info_text(image, user_rotation),
        zoom_status_text_with_settings(transform, zoom_settings)
    )
}

pub fn image_metadata_info_text(metadata: &ImageMetadata, size: ImageSize) -> String {
    image_metadata_display_info_text(metadata, size, ImageRotation::ZERO)
}

pub fn image_display_info_text(image: &LoadedImage, user_rotation: ImageRotation) -> String {
    image_metadata_display_info_text(image.metadata(), image.source_size(), user_rotation)
}

pub fn image_metadata_display_info_text(
    metadata: &ImageMetadata,
    source_size: ImageSize,
    user_rotation: ImageRotation,
) -> String {
    let orientation = display_orientation(metadata.exif_orientation(), user_rotation);
    let display_size = source_size.with_orientation(orientation);
    let orientation_text = orientation_info_text(metadata.exif_orientation(), user_rotation);

    if let Some(orientation_text) = orientation_text {
        format!(
            "{} | source {}x{} | display {}x{} | {} | {} | {}",
            metadata.file_name_for_display(),
            source_size.width(),
            source_size.height(),
            display_size.width(),
            display_size.height(),
            orientation_text,
            format_file_size(metadata.file_size()),
            standard_image_format_name(metadata.format())
        )
    } else {
        image_metadata_plain_info_text(metadata, source_size)
    }
}

fn image_metadata_plain_info_text(metadata: &ImageMetadata, size: ImageSize) -> String {
    format!(
        "{} | {}x{} | {} | {}",
        metadata.file_name_for_display(),
        size.width(),
        size.height(),
        format_file_size(metadata.file_size()),
        standard_image_format_name(metadata.format())
    )
}

fn orientation_info_text(
    exif_orientation: ImageOrientation,
    user_rotation: ImageRotation,
) -> Option<String> {
    match (exif_orientation.is_identity(), user_rotation.is_identity()) {
        (true, true) => None,
        (false, true) => Some(format!("EXIF {}", exif_orientation.exif_value())),
        (true, false) => Some(format!("rotation {} deg", user_rotation.degrees())),
        (false, false) => Some(format!(
            "EXIF {} + rotation {} deg",
            exif_orientation.exif_value(),
            user_rotation.degrees()
        )),
    }
}

pub fn format_file_size(bytes: u64) -> String {
    const UNITS: [&str; 5] = ["B", "KB", "MB", "GB", "TB"];
    const UNIT_SIZE: f64 = 1024.0;

    if bytes < 1024 {
        return format!("{bytes} B");
    }

    let mut value = bytes as f64;
    let mut unit_index = 0usize;
    while value >= UNIT_SIZE && unit_index < UNITS.len() - 1 {
        value /= UNIT_SIZE;
        unit_index += 1;
    }

    format!("{value:.1} {}", UNITS[unit_index])
}

pub const fn standard_image_format_name(format: SupportedImageFormat) -> &'static str {
    format.display_name()
}

pub fn file_name_for_display(path: &Path) -> String {
    path.file_name()
        .filter(|name| !name.is_empty())
        .unwrap_or(path.as_os_str())
        .to_string_lossy()
        .into_owned()
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use super::{
        animation_state_after_home, animation_state_after_manual_step,
        animation_state_after_timer_tick, animation_state_after_toggle,
        animation_timer_interval_ms, clamp_export_quality, clamp_view_offset, clamp_zoom_scale,
        clamp_zoom_scale_with_settings, command_for_key_input, command_for_key_input_with_context,
        default_export_format_for_source_format, display_orientation,
        export_format_default_extension, export_format_display_name, export_format_extensions,
        export_format_for_extension, export_format_for_path, export_format_mime_type,
        export_path_with_format_extension, export_quality_range,
        export_size_from_height_preserving_aspect, export_size_from_width_preserving_aspect,
        first_supported_image_path, format_file_size, image_display_info_text, image_info_text,
        image_status_text, image_status_text_with_settings, is_image_too_large, is_large_image,
        is_stale_decode_generation, is_supported_image_path, memory_cache_slots_to_evict,
        navigation_failure_policy, navigation_index, navigation_index_with_settings,
        normalize_animation_frame_delay_ms, normalize_animation_frame_delay_ms_with_settings,
        orient_rgba8_image, parse_app_config, preview_size_for_viewport, rotate_rgba8_image,
        scaling_cache_key_for_render, scaling_cache_target_size, scaling_quality_for_render,
        serialize_app_config, should_rebuild_scaling_cache,
        should_request_full_resolution_for_view, should_retain_full_resolution,
        should_use_software_resampling, standard_image_format_name, suggested_export_path,
        suggested_export_path_with_suffix, supported_image_format_for_path, zoom_status_text,
        zoom_status_text_with_settings, AnimationCommand, AnimationFrameStepDirection,
        AnimationLoopPolicy, AnimationPlayback, AnimationPlaybackState, AnimationTimingSettings,
        AppConfig, AppConfigParseError, Command, CommandContext, DecodeGeneration,
        DefaultExportFormatPolicy, ExportAnimationPolicy, ExportFormat, ExportOptions,
        ExportOrientationPolicy, ExportSettings, ImageBufferKind, ImageCacheSlot, ImageDisplayRect,
        ImageFolder, ImageMemoryPolicy, ImageMetadata, ImageNavigationDirection, ImageOrientation,
        ImageRotation, ImageSize, ImageState, InteractionSettings, KeyCode, KeyInput, KeyModifiers,
        LoadedImage, MemoryCacheEntry, MemoryPolicySettings, MouseShortcut,
        NavigationFailurePolicy, NavigationSettings, RgbColor, Rgba8Image, ScalingCacheKey,
        ScalingQuality, StatusUiSettings, SupportedImageFormat, UiLanguage, ViewMode, ViewOffset,
        ViewTransform, ViewportPoint, ViewportSize, WindowBounds, ZoomSettings,
        DEFAULT_EXPORT_QUALITY, DEFAULT_SCALING_QUALITY, MAX_CONFIG_ANIMATION_DELAY_MS,
        MAX_CONFIG_IMAGE_PIXELS, MAX_CONFIG_MEMORY_MIB, MAX_EXPORT_QUALITY,
        MAX_IMAGE_FOLDER_SNAPSHOT_PATHS, MAX_ZOOM_SCALE, MIN_EXPORT_QUALITY, MIN_ZOOM_SCALE,
    };

    #[test]
    fn navigation_failure_policy_keeps_current_image_without_auto_skip() {
        let policy = navigation_failure_policy();

        assert_eq!(policy, NavigationFailurePolicy::KeepCurrentAndReport);
        assert!(!policy.auto_skips_failed_files());
        assert_eq!(policy.max_attempts_per_command(), 1);
    }

    #[test]
    fn app_config_defaults_are_safe() {
        let config = AppConfig::default();

        assert_eq!(config.window_bounds(), None);
        assert_eq!(config.ui_language(), UiLanguage::English);
        assert_eq!(config.default_view_mode(), ViewMode::FitToWindow);
        assert_eq!(
            config.default_view_transform(),
            ViewTransform::FIT_TO_WINDOW
        );
        assert_eq!(config.scaling_quality(), DEFAULT_SCALING_QUALITY);
        assert_eq!(config.recent_folder(), None);
        assert_eq!(config.export_default_quality(), DEFAULT_EXPORT_QUALITY);
        assert!(config.animation_autoplay());
        assert_eq!(config.zoom_settings(), ZoomSettings::default());
        assert_eq!(
            config.memory_policy_settings(),
            MemoryPolicySettings::default()
        );
        assert_eq!(config.image_memory_policy(), ImageMemoryPolicy::DEFAULT);
        assert_eq!(
            config.animation_timing_settings(),
            AnimationTimingSettings::default()
        );
        assert_eq!(config.navigation_settings(), NavigationSettings::default());
        assert_eq!(config.export_settings(), &ExportSettings::default());
        assert_eq!(
            config
                .export_settings()
                .default_export_format_policy()
                .export_format_for_source(SupportedImageFormat::Webp),
            ExportFormat::Png
        );
        assert_eq!(config.status_ui_settings(), StatusUiSettings::default());
        assert_eq!(
            config.interaction_settings(),
            InteractionSettings::default()
        );
    }

    #[test]
    fn app_config_validation_corrects_invalid_values() {
        let config = parse_app_config(
            "\
version=1
window.x=10
window.y=20
window.width=20
window.height=30
default_view_mode=manual_zoom
scaling_quality=not-a-quality
recent_folder=
export_default_quality=250
animation_autoplay=maybe
",
        )
        .expect("config parses with corrected values");

        assert_eq!(config.window_bounds(), None);
        assert_eq!(config.default_view_mode(), ViewMode::FitToWindow);
        assert_eq!(config.scaling_quality(), ScalingQuality::Balanced);
        assert_eq!(config.recent_folder(), None);
        assert_eq!(config.export_default_quality(), 100);
        assert!(config.animation_autoplay());
    }

    #[test]
    fn app_config_export_quality_clamps_wide_numeric_values() {
        let too_low = parse_app_config("version=1\nexport_default_quality=-25")
            .expect("negative quality parses and clamps");
        let too_high = parse_app_config("version=1\nexport_default_quality=99999")
            .expect("large quality parses and clamps");

        assert_eq!(too_low.export_default_quality(), 1);
        assert_eq!(too_high.export_default_quality(), 100);
    }

    #[test]
    fn app_config_clamps_integer_overflow_values() {
        let positive_overflow = "\
9999999999999999999999999999999999999999999999999999999999999999";
        let negative_overflow = "\
-9999999999999999999999999999999999999999999999999999999999999999";
        let config = parse_app_config(&format!(
            "\
version=1
export_default_quality={positive_overflow}
max_image_pixels={positive_overflow}
large_image_pixel_threshold={positive_overflow}
preview_max_pixels={positive_overflow}
max_transient_decode_mib={positive_overflow}
max_full_resolution_mib={positive_overflow}
max_resident_mib={positive_overflow}
max_cache_entry_mib={positive_overflow}
max_navigation_attempts_per_command={positive_overflow}
default_frame_delay_ms={positive_overflow}
min_frame_delay_ms={negative_overflow}
max_frame_delay_ms={positive_overflow}
jpeg_alpha_background_rgb={positive_overflow},{negative_overflow},{positive_overflow}
"
        ))
        .expect("overflowing numeric config values still parse");

        assert_eq!(config.export_default_quality(), MAX_EXPORT_QUALITY);

        let memory = config.memory_policy_settings();
        assert_eq!(memory.max_image_pixels(), MAX_CONFIG_IMAGE_PIXELS);
        assert_eq!(
            memory.large_image_pixel_threshold(),
            MAX_CONFIG_IMAGE_PIXELS
        );
        assert_eq!(memory.preview_max_pixels(), MAX_CONFIG_IMAGE_PIXELS);
        assert_eq!(memory.max_transient_decode_mib(), MAX_CONFIG_MEMORY_MIB);
        assert_eq!(memory.max_full_resolution_mib(), MAX_CONFIG_MEMORY_MIB);
        assert_eq!(memory.max_resident_mib(), MAX_CONFIG_MEMORY_MIB);
        assert_eq!(memory.max_cache_entry_mib(), MAX_CONFIG_MEMORY_MIB);
        assert_eq!(
            config
                .navigation_settings()
                .max_navigation_attempts_per_command(),
            100
        );

        let animation = config.animation_timing_settings();
        assert_eq!(
            animation.default_frame_delay_ms(),
            MAX_CONFIG_ANIMATION_DELAY_MS
        );
        assert_eq!(animation.min_frame_delay_ms(), 1);
        assert_eq!(
            animation.max_frame_delay_ms(),
            MAX_CONFIG_ANIMATION_DELAY_MS
        );

        assert_eq!(
            config.export_settings().jpeg_alpha_background_rgb(),
            RgbColor::new(u8::MAX, u8::MIN, u8::MAX)
        );

        let low_quality = parse_app_config(&format!(
            "version=1\nexport_default_quality={negative_overflow}"
        ))
        .expect("negative overflowing quality still parses");
        assert_eq!(low_quality.export_default_quality(), MIN_EXPORT_QUALITY);
    }

    #[test]
    fn app_config_parses_aliases_escaped_paths_and_bool_values() {
        let config = parse_app_config(
            "\
version=1
window.x=-32768
window.y=32767
window.width=320
window.height=240
ui_language=ko
default_view_mode=actual
scaling_quality=high-quality
recent_folder=C:\\\\Images\\\\Reviewed
export_default_quality=50
animation_autoplay=no
",
        )
        .expect("aliased config parses");

        assert_eq!(
            config.window_bounds(),
            WindowBounds::new(-32768, 32767, 320, 240)
        );
        assert_eq!(config.ui_language(), UiLanguage::Korean);
        assert_eq!(config.default_view_mode(), ViewMode::ActualSize);
        assert_eq!(config.default_view_transform(), ViewTransform::ACTUAL_SIZE);
        assert_eq!(config.scaling_quality(), ScalingQuality::HighQuality);
        assert_eq!(
            config.recent_folder(),
            Some(std::path::Path::new(r"C:\Images\Reviewed"))
        );
        assert_eq!(config.export_default_quality(), 50);
        assert!(!config.animation_autoplay());

        for value in ["true", "1", "yes"] {
            let parsed = parse_app_config(&format!("version=1\nanimation_autoplay={value}"))
                .expect("true boolean alias parses");
            assert!(parsed.animation_autoplay(), "{value}");
        }
    }

    #[test]
    fn app_config_rejects_invalid_recent_folder_escape_with_line_number() {
        assert_eq!(
            parse_app_config("version=1\nrecent_folder=C:\\q"),
            Err(AppConfigParseError::InvalidEscape { line: 2 })
        );
    }

    #[test]
    fn window_bounds_rejects_values_outside_persisted_range() {
        assert!(WindowBounds::new(-32768, 32767, 320, 240).is_some());
        assert!(WindowBounds::new(-32769, 0, 320, 240).is_none());
        assert!(WindowBounds::new(0, 32768, 320, 240).is_none());
        assert!(WindowBounds::new(0, 0, 319, 240).is_none());
        assert!(WindowBounds::new(0, 0, 320, 239).is_none());
        assert!(WindowBounds::new(0, 0, 32768, 240).is_none());
    }

    #[test]
    fn app_config_serializes_and_deserializes() {
        let config = AppConfig::new(
            WindowBounds::new(12, 34, 800, 600),
            ViewMode::ActualSize,
            ScalingQuality::HighQuality,
            Some(PathBuf::from(r"C:\Images\A=B")),
            95,
            false,
        );

        let serialized = serialize_app_config(&config);
        let parsed = parse_app_config(&serialized).expect("serialized config parses");

        assert!(serialized.contains("ui_language=\"english\"\n"));
        assert!(serialized.contains("default_view_mode=\"actual_size\"\n"));
        assert!(serialized.contains("scaling_quality=\"high_quality\"\n"));
        assert!(serialized.contains(r#"recent_folder="C:\\Images\\A=B""#));
        assert_eq!(parsed, config);
    }

    #[test]
    fn app_config_serialization_includes_all_current_user_setting_keys() {
        let serialized = serialize_app_config(&AppConfig::default());
        let required_keys = [
            "ui_language=",
            "min_zoom_scale=",
            "max_zoom_scale=",
            "zoom_step_factor=",
            "large_image_pixel_threshold=",
            "max_image_pixels=",
            "preview_max_pixels=",
            "preview_oversample=",
            "fallback_viewport_width=",
            "fallback_viewport_height=",
            "max_transient_decode_mib=",
            "max_full_resolution_mib=",
            "max_resident_mib=",
            "max_cache_entry_mib=",
            "max_cache_entries=",
            "max_animation_metadata_frames=",
            "full_resolution_request_scale=",
            "default_frame_delay_ms=",
            "min_frame_delay_ms=",
            "max_frame_delay_ms=",
            "wrap_navigation=",
            "auto_skip_failed_navigation=",
            "max_navigation_attempts_per_command=",
            "default_export_format_policy=",
            "export_filename_suffix=",
            "jpeg_alpha_background_rgb=",
            "show_status_bar=",
            "detailed_status_text=",
            "zoom_shortcut=",
            "image_navigation_shortcut=",
            "image_pan_shortcut=",
            "window_move_shortcut=",
        ];

        for key in required_keys {
            assert!(
                serialized.lines().any(|line| line.starts_with(key)),
                "missing serialized config key {key}"
            );
        }
    }

    #[test]
    fn legacy_version1_config_without_user_settings_keeps_safe_defaults() {
        let config = parse_app_config(
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
        .expect("legacy config parses");

        assert_eq!(config.default_view_mode(), ViewMode::ActualSize);
        assert_eq!(config.scaling_quality(), ScalingQuality::Nearest);
        assert_eq!(config.export_default_quality(), 85);
        assert!(!config.animation_autoplay());
        assert_eq!(config.ui_language(), UiLanguage::English);
        assert_eq!(config.zoom_settings(), ZoomSettings::default());
        assert_eq!(
            config.memory_policy_settings(),
            MemoryPolicySettings::default()
        );
        assert_eq!(config.image_memory_policy(), ImageMemoryPolicy::DEFAULT);
        assert_eq!(
            config.animation_timing_settings(),
            AnimationTimingSettings::default()
        );
        assert_eq!(config.navigation_settings(), NavigationSettings::default());
        assert_eq!(config.export_settings(), &ExportSettings::default());
        assert_eq!(config.status_ui_settings(), StatusUiSettings::default());
        assert_eq!(
            config.interaction_settings(),
            InteractionSettings::default()
        );
    }

    #[test]
    fn app_config_serializes_and_deserializes_user_settings() {
        let mut config = AppConfig::default();
        config.set_zoom_settings(ZoomSettings::new(0.125, 64.0, 1.5));

        let mut memory = MemoryPolicySettings::default();
        memory.set_large_image_pixel_threshold(50_000_000);
        memory.set_max_image_pixels(250_000_000);
        memory.set_preview_max_pixels(6_000_000);
        memory.set_preview_oversample(3);
        memory.set_fallback_viewport_width(1280);
        memory.set_fallback_viewport_height(720);
        memory.set_max_transient_decode_mib(512);
        memory.set_max_full_resolution_mib(64);
        memory.set_max_resident_mib(192);
        memory.set_max_cache_entry_mib(48);
        memory.set_max_cache_entries(4);
        memory.set_max_animation_metadata_frames(25_000);
        memory.set_full_resolution_request_scale(0.5);
        config.set_memory_policy_settings(memory);

        let mut animation = AnimationTimingSettings::default();
        animation.set_min_frame_delay_ms(20);
        animation.set_max_frame_delay_ms(5_000);
        animation.set_default_frame_delay_ms(120);
        config.set_animation_timing_settings(animation);

        let mut navigation = NavigationSettings::default();
        navigation.set_wrap_navigation(false);
        navigation.set_auto_skip_failed_navigation(true);
        navigation.set_max_navigation_attempts_per_command(5);
        config.set_navigation_settings(navigation);

        config.set_export_settings(ExportSettings::new(
            DefaultExportFormatPolicy::Jpeg,
            "_edited",
            RgbColor::new(12, 34, 56),
        ));

        let mut status_ui = StatusUiSettings::default();
        status_ui.set_show_status_bar(false);
        status_ui.set_detailed_status_text(false);
        config.set_status_ui_settings(status_ui);

        let mut interaction = InteractionSettings::default();
        interaction.set_zoom_shortcut(MouseShortcut::MouseWheel);
        interaction.set_image_navigation_shortcut(MouseShortcut::CtrlMouseWheel);
        interaction.set_image_pan_shortcut(MouseShortcut::LeftButtonDrag);
        interaction.set_window_move_shortcut(MouseShortcut::CtrlLeftButtonDrag);
        config.set_interaction_settings(interaction);

        let serialized = serialize_app_config(&config);
        let parsed = parse_app_config(&serialized).expect("serialized user settings parse");

        assert_eq!(parsed, config);
        assert_eq!(
            parsed.image_memory_policy().max_transient_decode_bytes(),
            512 * 1024 * 1024
        );
        assert_eq!(
            parsed
                .export_settings()
                .default_export_format_policy()
                .export_format_for_source(SupportedImageFormat::Png),
            ExportFormat::Jpeg
        );
    }

    #[test]
    fn app_config_user_setting_invalid_values_are_corrected() {
        let config = parse_app_config(
            "\
version=1
min_zoom_scale=NaN
max_zoom_scale=-2
zoom_step_factor=100
large_image_pixel_threshold=-9
max_image_pixels=0
preview_max_pixels=999999999999
preview_oversample=0
fallback_viewport_width=0
fallback_viewport_height=999999
max_transient_decode_mib=0
max_full_resolution_mib=99999
max_resident_mib=1
max_cache_entry_mib=4096
max_cache_entries=-3
max_animation_metadata_frames=0
full_resolution_request_scale=NaN
default_frame_delay_ms=not-a-number
min_frame_delay_ms=1000
max_frame_delay_ms=50
wrap_navigation=maybe
auto_skip_failed_navigation=yes
max_navigation_attempts_per_command=0
default_export_format_policy=tiff
export_filename_suffix=bad\\q
jpeg_alpha_background_rgb=-10,300,128
show_status_bar=no
detailed_status_text=maybe
",
        )
        .expect("config parses with corrected user settings");

        let zoom = config.zoom_settings();
        assert_eq!(zoom.min_zoom_scale(), MIN_ZOOM_SCALE);
        assert_eq!(zoom.max_zoom_scale(), 1.0);
        assert_eq!(zoom.zoom_step_factor(), 8.0);

        let memory = config.memory_policy_settings();
        assert_eq!(memory.large_image_pixel_threshold(), 1);
        assert_eq!(memory.max_image_pixels(), 1);
        assert_eq!(memory.preview_max_pixels(), 1);
        assert_eq!(memory.preview_oversample(), 1);
        assert_eq!(memory.fallback_viewport_width(), 1);
        assert_eq!(memory.fallback_viewport_height(), 16_384);
        assert_eq!(memory.max_transient_decode_mib(), 1);
        assert_eq!(memory.max_full_resolution_mib(), 1);
        assert_eq!(memory.max_resident_mib(), 1);
        assert_eq!(memory.max_cache_entry_mib(), 1);
        assert_eq!(memory.max_cache_entries(), 0);
        assert_eq!(memory.max_animation_metadata_frames(), 1);
        assert_eq!(
            memory.full_resolution_request_scale(),
            ImageMemoryPolicy::DEFAULT.full_resolution_request_scale()
        );
        assert_eq!(
            config.image_memory_policy().fallback_viewport(),
            ImageSize::new(1, 16_384)
        );

        let animation = config.animation_timing_settings();
        assert_eq!(animation.min_frame_delay_ms(), 1000);
        assert_eq!(animation.max_frame_delay_ms(), 1000);
        assert_eq!(animation.default_frame_delay_ms(), 1000);
        assert_eq!(animation.normalize_frame_delay_ms(0), 1000);

        let navigation = config.navigation_settings();
        assert!(navigation.wrap_navigation());
        assert!(navigation.auto_skip_failed_navigation());
        assert_eq!(navigation.max_navigation_attempts_per_command(), 1);

        let export = config.export_settings();
        assert_eq!(
            export.default_export_format_policy(),
            DefaultExportFormatPolicy::Png
        );
        assert_eq!(export.export_filename_suffix(), "-export");
        assert_eq!(
            export.jpeg_alpha_background_rgb(),
            RgbColor::new(0, 255, 128)
        );

        let status_ui = config.status_ui_settings();
        assert!(!status_ui.show_status_bar());
        assert!(status_ui.detailed_status_text());
        assert_eq!(
            config.interaction_settings(),
            InteractionSettings::default()
        );
    }

    #[test]
    fn app_config_parses_mouse_shortcut_settings() {
        let config = parse_app_config(
            "\
version=1
zoom_shortcut=mouse_wheel
image_navigation_shortcut=ctrl_mouse_wheel
image_pan_shortcut=left_button_drag
window_move_shortcut=ctrl_left_button_drag
",
        )
        .expect("config parses");

        let interaction = config.interaction_settings();
        assert_eq!(interaction.zoom_shortcut(), MouseShortcut::MouseWheel);
        assert_eq!(
            interaction.image_navigation_shortcut(),
            MouseShortcut::CtrlMouseWheel
        );
        assert_eq!(
            interaction.image_pan_shortcut(),
            MouseShortcut::LeftButtonDrag
        );
        assert_eq!(
            interaction.window_move_shortcut(),
            MouseShortcut::CtrlLeftButtonDrag
        );
    }

    #[test]
    fn app_config_range_correction_normalizes_dependent_memory_limits() {
        let config = parse_app_config(
            "\
version=1
large_image_pixel_threshold=900
max_image_pixels=100
preview_max_pixels=500
max_transient_decode_mib=64
max_full_resolution_mib=512
max_resident_mib=32
max_cache_entry_mib=64
",
        )
        .expect("config parses with normalized relationships");

        let memory = config.memory_policy_settings();
        assert_eq!(memory.max_image_pixels(), 100);
        assert_eq!(memory.large_image_pixel_threshold(), 100);
        assert_eq!(memory.preview_max_pixels(), 100);
        assert_eq!(memory.max_transient_decode_mib(), 64);
        assert_eq!(memory.max_full_resolution_mib(), 64);
        assert_eq!(memory.max_resident_mib(), 32);
        assert_eq!(memory.max_cache_entry_mib(), 32);
    }

    #[test]
    fn app_config_user_settings_drive_domain_policies() {
        let zoom = ZoomSettings::new(0.25, 4.0, 2.0);
        let transform = ViewTransform::manual_zoom_with_settings(99.0, ViewOffset::ZERO, zoom);
        assert_eq!(transform.zoom_scale(), 4.0);
        assert_eq!(zoom_status_text_with_settings(transform, zoom), "400%");

        let mut memory = MemoryPolicySettings::default();
        memory.set_large_image_pixel_threshold(100);
        memory.set_max_image_pixels(1_000);
        memory.set_max_full_resolution_mib(1);
        memory.set_full_resolution_request_scale(0.5);
        let policy = memory.image_memory_policy();
        assert!(is_large_image(ImageSize::new(11, 10), policy));
        assert!(is_image_too_large(ImageSize::new(40, 30), policy));
        assert!(should_request_full_resolution_for_view(
            ImageBufferKind::Preview,
            0.5,
            ImageSize::new(100, 100),
            policy
        ));
        assert!(!should_request_full_resolution_for_view(
            ImageBufferKind::FullResolution,
            1.0,
            ImageSize::new(100, 100),
            policy
        ));

        let mut animation = AnimationTimingSettings::default();
        animation.set_min_frame_delay_ms(20);
        animation.set_max_frame_delay_ms(200);
        animation.set_default_frame_delay_ms(80);
        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(0, animation),
            80
        );
        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(5, animation),
            20
        );
        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(500, animation),
            200
        );

        let mut navigation = NavigationSettings::default();
        navigation.set_wrap_navigation(false);
        navigation.set_auto_skip_failed_navigation(true);
        navigation.set_max_navigation_attempts_per_command(3);
        assert_eq!(
            navigation_index_with_settings(3, Some(2), ImageNavigationDirection::Next, navigation),
            None
        );
        assert!(navigation.auto_skip_failed_navigation());
        assert_eq!(navigation.max_navigation_attempts_per_command(), 3);

        let export = ExportSettings::new(
            DefaultExportFormatPolicy::Webp,
            "_review",
            RgbColor::new(12, 34, 56),
        );
        assert_eq!(
            export
                .default_export_format_policy()
                .export_format_for_source(SupportedImageFormat::Png),
            ExportFormat::Webp
        );
        assert_eq!(
            suggested_export_path_with_suffix("C:/Images/photo.png", ExportFormat::Webp, "_review"),
            PathBuf::from("C:/Images/photo_review.webp")
        );
        assert_eq!(
            export.jpeg_alpha_background_rgb(),
            RgbColor::new(12, 34, 56)
        );
    }

    #[test]
    fn corrupt_app_config_data_is_rejected() {
        assert!(parse_app_config("version=1\nthis line is broken").is_err());
        assert!(parse_app_config("version=2").is_err());
    }

    #[test]
    fn client_size_clamps_negative_values_to_empty_edges() {
        let size = ViewportSize::from_client_size(-10, 320);

        assert_eq!(size.width(), 0);
        assert_eq!(size.height(), 320);
        assert!(size.is_empty());
    }

    #[test]
    fn client_size_preserves_positive_dimensions() {
        let size = ViewportSize::from_client_size(800, 600);

        assert_eq!(size.width(), 800);
        assert_eq!(size.height(), 600);
        assert!(!size.is_empty());
    }

    #[test]
    fn right_angle_rotation_updates_display_size() {
        let size = ImageSize::new(3, 2);

        assert_eq!(
            size.with_rotation(ImageRotation::Degrees0),
            ImageSize::new(3, 2)
        );
        assert_eq!(
            size.with_rotation(ImageRotation::Degrees90),
            ImageSize::new(2, 3)
        );
        assert_eq!(
            size.with_rotation(ImageRotation::Degrees180),
            ImageSize::new(3, 2)
        );
        assert_eq!(
            size.with_rotation(ImageRotation::Degrees270),
            ImageSize::new(2, 3)
        );
    }

    #[test]
    fn exif_orientation_values_map_to_display_transforms() {
        let cases = [
            (1, ImageOrientation::Normal),
            (2, ImageOrientation::FlipHorizontal),
            (3, ImageOrientation::Rotate180),
            (4, ImageOrientation::FlipVertical),
            (5, ImageOrientation::Rotate90FlipHorizontal),
            (6, ImageOrientation::Rotate90),
            (7, ImageOrientation::Rotate270FlipHorizontal),
            (8, ImageOrientation::Rotate270),
        ];

        for (value, orientation) in cases {
            assert_eq!(ImageOrientation::from_exif_value(value), Some(orientation));
            assert_eq!(orientation.exif_value(), value);
        }
        assert_eq!(ImageOrientation::from_exif_value(0), None);
        assert_eq!(ImageOrientation::from_exif_value(9), None);
    }

    #[test]
    fn image_size_with_orientation_swaps_only_transposed_orientations() {
        let size = ImageSize::new(3, 2);
        let cases = [
            (ImageOrientation::Normal, ImageSize::new(3, 2)),
            (ImageOrientation::FlipHorizontal, ImageSize::new(3, 2)),
            (ImageOrientation::Rotate180, ImageSize::new(3, 2)),
            (ImageOrientation::FlipVertical, ImageSize::new(3, 2)),
            (
                ImageOrientation::Rotate90FlipHorizontal,
                ImageSize::new(2, 3),
            ),
            (ImageOrientation::Rotate90, ImageSize::new(2, 3)),
            (
                ImageOrientation::Rotate270FlipHorizontal,
                ImageSize::new(2, 3),
            ),
            (ImageOrientation::Rotate270, ImageSize::new(2, 3)),
        ];

        for (orientation, expected) in cases {
            assert_eq!(
                size.with_orientation(orientation),
                expected,
                "{orientation:?}"
            );
        }
    }

    #[test]
    fn user_rotation_is_composed_after_exif_orientation() {
        let clockwise_exif_values = [
            (1, 6),
            (2, 7),
            (3, 8),
            (4, 5),
            (5, 2),
            (6, 3),
            (7, 4),
            (8, 1),
        ];

        for (exif_value, expected_value) in clockwise_exif_values {
            let exif_orientation =
                ImageOrientation::from_exif_value(exif_value).expect("EXIF orientation");
            let expected =
                ImageOrientation::from_exif_value(expected_value).expect("expected orientation");

            assert_eq!(
                display_orientation(exif_orientation, ImageRotation::Degrees90),
                expected
            );
        }

        assert_eq!(
            display_orientation(ImageOrientation::Rotate90, ImageRotation::Degrees90),
            ImageOrientation::Rotate180
        );
        assert_eq!(
            display_orientation(ImageOrientation::FlipHorizontal, ImageRotation::Degrees270),
            ImageOrientation::Rotate90FlipHorizontal
        );
    }

    #[test]
    fn display_orientation_matches_exif_then_manual_pixel_transform_for_all_cases() {
        let source = numbered_rgba8_image_3x2();
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
        let rotations = [
            ImageRotation::Degrees0,
            ImageRotation::Degrees90,
            ImageRotation::Degrees180,
            ImageRotation::Degrees270,
        ];

        for orientation in orientations {
            for rotation in rotations {
                let exif_then_rotation = rotate_rgba8_image(
                    &orient_rgba8_image(&source, orientation).expect("EXIF-oriented image"),
                    rotation,
                )
                .expect("rotated image");
                let composed =
                    orient_rgba8_image(&source, display_orientation(orientation, rotation))
                        .expect("composed display image");

                assert_eq!(
                    composed.size(),
                    exif_then_rotation.size(),
                    "{orientation:?} + {rotation:?}"
                );
                assert_eq!(
                    pixel_ids(&composed),
                    pixel_ids(&exif_then_rotation),
                    "{orientation:?} + {rotation:?}"
                );
            }
        }
    }

    #[test]
    fn rgba8_rotation_90_maps_pixels_clockwise() {
        let rotated = rotate_rgba8_image(&numbered_rgba8_image_3x2(), ImageRotation::Degrees90)
            .expect("rotated image");

        assert_eq!(rotated.size(), ImageSize::new(2, 3));
        assert_eq!(pixel_ids(&rotated), vec![4, 1, 5, 2, 6, 3]);
    }

    #[test]
    fn rgba8_rotation_180_maps_pixels() {
        let rotated = rotate_rgba8_image(&numbered_rgba8_image_3x2(), ImageRotation::Degrees180)
            .expect("rotated image");

        assert_eq!(rotated.size(), ImageSize::new(3, 2));
        assert_eq!(pixel_ids(&rotated), vec![6, 5, 4, 3, 2, 1]);
    }

    #[test]
    fn rgba8_rotation_270_maps_pixels_clockwise() {
        let rotated = rotate_rgba8_image(&numbered_rgba8_image_3x2(), ImageRotation::Degrees270)
            .expect("rotated image");

        assert_eq!(rotated.size(), ImageSize::new(2, 3));
        assert_eq!(pixel_ids(&rotated), vec![3, 6, 2, 5, 1, 4]);
    }

    #[test]
    fn rgba8_orientation_maps_exif_1_through_8_pixels() {
        let cases = [
            (
                ImageOrientation::Normal,
                ImageSize::new(3, 2),
                vec![1, 2, 3, 4, 5, 6],
            ),
            (
                ImageOrientation::FlipHorizontal,
                ImageSize::new(3, 2),
                vec![3, 2, 1, 6, 5, 4],
            ),
            (
                ImageOrientation::Rotate180,
                ImageSize::new(3, 2),
                vec![6, 5, 4, 3, 2, 1],
            ),
            (
                ImageOrientation::FlipVertical,
                ImageSize::new(3, 2),
                vec![4, 5, 6, 1, 2, 3],
            ),
            (
                ImageOrientation::Rotate90FlipHorizontal,
                ImageSize::new(2, 3),
                vec![1, 4, 2, 5, 3, 6],
            ),
            (
                ImageOrientation::Rotate90,
                ImageSize::new(2, 3),
                vec![4, 1, 5, 2, 6, 3],
            ),
            (
                ImageOrientation::Rotate270FlipHorizontal,
                ImageSize::new(2, 3),
                vec![6, 3, 5, 2, 4, 1],
            ),
            (
                ImageOrientation::Rotate270,
                ImageSize::new(2, 3),
                vec![3, 6, 2, 5, 1, 4],
            ),
        ];

        for (orientation, expected_size, expected_pixels) in cases {
            let oriented =
                orient_rgba8_image(&numbered_rgba8_image_3x2(), orientation).expect("image");

            assert_eq!(oriented.size(), expected_size, "{orientation:?}");
            assert_eq!(pixel_ids(&oriented), expected_pixels, "{orientation:?}");
        }
    }

    #[test]
    fn rgba8_orientation_rejects_invalid_pixel_buffer_without_panicking() {
        let invalid = Rgba8Image::new(2, 2, vec![0, 0, 0, 255]);

        assert_eq!(
            orient_rgba8_image(&invalid, ImageOrientation::Rotate90),
            None
        );
        assert_eq!(
            rotate_rgba8_image(&invalid, ImageRotation::Degrees180),
            None
        );
    }

    #[test]
    fn rgba8_clone_shares_pixel_buffer() {
        let image = Rgba8Image::new(2, 1, vec![1, 2, 3, 4, 5, 6, 7, 8]);
        let cloned = image.clone();

        assert!(std::sync::Arc::ptr_eq(&image.pixels, &cloned.pixels));
        assert_eq!(cloned.pixels(), image.pixels());
    }

    #[test]
    fn fit_to_window_rect_scales_and_centers_image() {
        let rect = ViewTransform::FIT_TO_WINDOW
            .display_rect(
                ViewportSize::from_client_size(800, 600),
                ImageSize::new(1600, 800),
            )
            .expect("fit rect");

        assert_eq!(rect.x(), 0);
        assert_eq!(rect.y(), 100);
        assert_eq!(rect.width(), 800);
        assert_eq!(rect.height(), 400);
    }

    #[test]
    fn fit_to_window_uses_display_size_after_right_angle_rotation() {
        let viewport = ViewportSize::from_client_size(800, 600);
        let source_size = ImageSize::new(1600, 800);
        let cases = [
            (ImageRotation::Degrees90, (250, 0, 300, 600)),
            (ImageRotation::Degrees180, (0, 100, 800, 400)),
            (ImageRotation::Degrees270, (250, 0, 300, 600)),
        ];

        for (rotation, (x, y, width, height)) in cases {
            let rect = ViewTransform::FIT_TO_WINDOW
                .display_rect(viewport, source_size.with_rotation(rotation))
                .expect("rotated fit rect");

            assert_eq!(
                (rect.x(), rect.y(), rect.width(), rect.height()),
                (x, y, width, height),
                "{rotation:?}"
            );
        }
    }

    #[test]
    fn rotated_manual_zoom_offsets_clamp_to_display_axes() {
        let viewport = ViewportSize::from_client_size(800, 600);
        let source_size = ImageSize::new(1200, 300);
        let requested_offset = ViewOffset::new(10_000.0, 10_000.0);
        let cases = [
            (ImageRotation::Degrees90, (250, 0, 300, 1200)),
            (ImageRotation::Degrees180, (0, 150, 1200, 300)),
            (ImageRotation::Degrees270, (250, 0, 300, 1200)),
        ];

        for (rotation, (x, y, width, height)) in cases {
            let display_size = source_size.with_rotation(rotation);
            let offset = clamp_view_offset(viewport, display_size, 1.0, requested_offset);
            let rect = ViewTransform::manual_zoom(1.0, offset)
                .display_rect(viewport, display_size)
                .expect("rotated manual zoom rect");

            assert_eq!(
                (rect.x(), rect.y(), rect.width(), rect.height()),
                (x, y, width, height),
                "{rotation:?}"
            );
        }
    }

    #[test]
    fn rotated_manual_zoom_offsets_clamp_both_extremes() {
        let viewport = ViewportSize::from_client_size(800, 600);
        let source_size = ImageSize::new(1200, 300);
        let cases = [
            (ImageRotation::Degrees90, (250, 0, 250, -600)),
            (ImageRotation::Degrees180, (0, 150, -400, 150)),
            (ImageRotation::Degrees270, (250, 0, 250, -600)),
        ];

        for (rotation, (max_x, max_y, min_x, min_y)) in cases {
            let display_size = source_size.with_rotation(rotation);
            let max_offset = clamp_view_offset(
                viewport,
                display_size,
                1.0,
                ViewOffset::new(10_000.0, 10_000.0),
            );
            let min_offset = clamp_view_offset(
                viewport,
                display_size,
                1.0,
                ViewOffset::new(-10_000.0, -10_000.0),
            );
            let max_rect = ViewTransform::manual_zoom(1.0, max_offset)
                .display_rect(viewport, display_size)
                .expect("max clamped rect");
            let min_rect = ViewTransform::manual_zoom(1.0, min_offset)
                .display_rect(viewport, display_size)
                .expect("min clamped rect");

            assert_eq!((max_rect.x(), max_rect.y()), (max_x, max_y), "{rotation:?}");
            assert_eq!((min_rect.x(), min_rect.y()), (min_x, min_y), "{rotation:?}");
        }
    }

    #[test]
    fn display_rect_reports_positive_size() {
        let rect = ImageDisplayRect {
            x: 10,
            y: 20,
            width: 300,
            height: 200,
        };

        assert_eq!(rect.size(), Some(ImageSize::new(300, 200)));
        assert_eq!(
            ImageDisplayRect {
                x: 0,
                y: 0,
                width: 0,
                height: 200,
            }
            .size(),
            None
        );
    }

    #[test]
    fn scaling_quality_policy_uses_preferred_quality_except_actual_size() {
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::Balanced, 0.5),
            ScalingQuality::Balanced
        );
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::HighQuality, 2.0),
            ScalingQuality::HighQuality
        );
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::Balanced, 2.0),
            ScalingQuality::Balanced
        );
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::Balanced, 1.0),
            ScalingQuality::Nearest
        );
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::Nearest, 0.5),
            ScalingQuality::Nearest
        );
        assert_eq!(
            scaling_quality_for_render(ScalingQuality::Balanced, f64::NAN),
            ScalingQuality::Nearest
        );
    }

    #[test]
    fn scaling_cache_key_includes_display_orientation_target_size_and_quality() {
        let key = scaling_cache_key_for_render(
            7,
            ImageOrientation::Rotate90,
            ImageSize::new(1000, 800),
            ImageSize::new(501, 401),
            ScalingQuality::Balanced,
        )
        .expect("cache key");

        assert_eq!(key.image_revision(), 7);
        assert_eq!(key.orientation(), ImageOrientation::Rotate90);
        assert_eq!(key.target_size(), ImageSize::new(512, 416));
        assert_eq!(key.quality(), ScalingQuality::Balanced);
    }

    #[test]
    fn scaling_cache_rebuilds_when_revision_orientation_target_or_quality_changes() {
        let base = scaling_cache_key_for_render(
            1,
            ImageOrientation::Normal,
            ImageSize::new(1000, 800),
            ImageSize::new(501, 401),
            ScalingQuality::Balanced,
        )
        .expect("base cache key");
        let same_bucket = scaling_cache_key_for_render(
            1,
            ImageOrientation::Normal,
            ImageSize::new(1000, 800),
            ImageSize::new(510, 408),
            ScalingQuality::Balanced,
        )
        .expect("same target bucket");
        let next_revision = ScalingCacheKey::new(
            base.image_revision() + 1,
            base.orientation(),
            base.target_size(),
            base.quality(),
        );
        let next_orientation = ScalingCacheKey::new(
            base.image_revision(),
            ImageOrientation::Rotate90,
            base.target_size(),
            base.quality(),
        );
        let next_target = scaling_cache_key_for_render(
            1,
            ImageOrientation::Normal,
            ImageSize::new(1000, 800),
            ImageSize::new(520, 420),
            ScalingQuality::Balanced,
        )
        .expect("next target bucket");
        let next_quality = scaling_cache_key_for_render(
            1,
            ImageOrientation::Normal,
            ImageSize::new(1000, 800),
            ImageSize::new(500, 400),
            ScalingQuality::HighQuality,
        )
        .expect("next quality");

        assert!(!should_rebuild_scaling_cache(Some(base), same_bucket));
        assert!(should_rebuild_scaling_cache(Some(base), next_revision));
        assert!(should_rebuild_scaling_cache(Some(base), next_orientation));
        assert!(should_rebuild_scaling_cache(Some(base), next_target));
        assert!(should_rebuild_scaling_cache(Some(base), next_quality));
        assert!(should_rebuild_scaling_cache(None, base));
    }

    #[test]
    fn scaling_cache_target_skips_actual_size_nearest_and_small_balanced_changes() {
        let source_size = ImageSize::new(1000, 800);

        assert_eq!(
            scaling_cache_target_size(source_size, source_size, ScalingQuality::Balanced),
            None
        );
        assert_eq!(
            scaling_cache_target_size(
                source_size,
                ImageSize::new(500, 400),
                ScalingQuality::Nearest
            ),
            None
        );
        assert_eq!(
            scaling_cache_target_size(
                source_size,
                ImageSize::new(990, 792),
                ScalingQuality::Balanced
            ),
            None
        );
        assert_eq!(
            scaling_cache_target_size(
                source_size,
                ImageSize::new(970, 776),
                ScalingQuality::Balanced
            ),
            Some(ImageSize::new(976, 784))
        );
        assert_eq!(
            scaling_cache_target_size(
                ImageSize::new(20, 20),
                ImageSize::new(19, 19),
                ScalingQuality::Balanced
            ),
            None
        );
    }

    #[test]
    fn software_resampling_policy_prefers_balanced_downscale_and_high_quality_changes() {
        let source_size = ImageSize::new(1000, 800);

        assert!(should_use_software_resampling(
            source_size,
            ImageSize::new(500, 400),
            ScalingQuality::Balanced
        ));
        assert!(!should_use_software_resampling(
            source_size,
            ImageSize::new(1200, 960),
            ScalingQuality::Balanced
        ));
        assert!(should_use_software_resampling(
            source_size,
            ImageSize::new(1200, 960),
            ScalingQuality::HighQuality
        ));
    }

    #[test]
    fn large_image_policy_uses_pixels_and_retained_full_resolution_budget() {
        let policy = ImageMemoryPolicy::DEFAULT;

        assert!(!is_large_image(ImageSize::new(4000, 3000), policy));
        assert!(is_large_image(ImageSize::new(8000, 4000), policy));
        assert!(should_retain_full_resolution(
            ImageSize::new(4000, 3000),
            policy
        ));
        assert!(!should_retain_full_resolution(
            ImageSize::new(8000, 5000),
            policy
        ));
        assert!(is_image_too_large(ImageSize::new(20_000, 10_000), policy));
    }

    #[test]
    fn large_image_classification_separates_preview_retain_and_reject_cases() {
        let policy = ImageMemoryPolicy::DEFAULT;
        let preview_with_lazy_full_resolution = ImageSize::new(7000, 4000);
        let preview_only = ImageSize::new(9000, 5000);
        let rejected = ImageSize::new(20_000, 10_000);

        assert!(is_large_image(preview_with_lazy_full_resolution, policy));
        assert!(!is_image_too_large(
            preview_with_lazy_full_resolution,
            policy
        ));
        assert!(should_retain_full_resolution(
            preview_with_lazy_full_resolution,
            policy
        ));

        assert!(is_large_image(preview_only, policy));
        assert!(!is_image_too_large(preview_only, policy));
        assert!(!should_retain_full_resolution(preview_only, policy));

        assert!(is_image_too_large(rejected, policy));
        assert!(rejected.pixel_count().expect("pixel count") > policy.max_image_pixels());
    }

    #[test]
    fn large_image_classification_uses_exclusive_thresholds_and_byte_budget() {
        let policy = ImageMemoryPolicy::DEFAULT;
        let exactly_pixel_threshold = ImageSize::new(6000, 4000);
        let exactly_full_resolution_budget = ImageSize::new(8192, 4096);
        let over_full_resolution_budget = ImageSize::new(8193, 4096);

        assert_eq!(
            exactly_pixel_threshold.pixel_count(),
            Some(policy.large_image_pixel_threshold())
        );
        assert!(!is_large_image(exactly_pixel_threshold, policy));

        assert_eq!(
            exactly_full_resolution_budget.rgba8_byte_len(),
            Some(policy.max_full_resolution_bytes())
        );
        assert!(is_large_image(exactly_full_resolution_budget, policy));
        assert!(should_retain_full_resolution(
            exactly_full_resolution_budget,
            policy
        ));

        assert!(is_large_image(over_full_resolution_budget, policy));
        assert!(!should_retain_full_resolution(
            over_full_resolution_budget,
            policy
        ));
    }

    #[test]
    fn image_memory_calculations_reject_byte_length_overflow() {
        let size = ImageSize::new(u32::MAX, u32::MAX);

        assert_eq!(
            size.pixel_count(),
            Some(u64::from(u32::MAX) * u64::from(u32::MAX))
        );
        assert_eq!(size.rgba8_byte_len(), None);
        assert!(is_large_image(size, ImageMemoryPolicy::DEFAULT));
        assert!(is_image_too_large(size, ImageMemoryPolicy::DEFAULT));
    }

    #[test]
    fn preview_size_fits_viewport_with_oversample_and_pixel_cap() {
        let policy = ImageMemoryPolicy::DEFAULT;

        assert_eq!(
            preview_size_for_viewport(
                ImageSize::new(8000, 4000),
                ViewportSize::from_client_size(1000, 500),
                policy
            ),
            ImageSize::new(2000, 1000)
        );

        let preview = preview_size_for_viewport(
            ImageSize::new(20_000, 10_000),
            ViewportSize::from_client_size(4000, 3000),
            policy,
        );
        assert!(preview.pixel_count().expect("pixel count") <= policy.preview_max_pixels());
    }

    #[test]
    fn full_resolution_request_requires_preview_high_scale_and_memory_fit() {
        let policy = ImageMemoryPolicy::DEFAULT;

        assert!(should_request_full_resolution_for_view(
            ImageBufferKind::Preview,
            1.0,
            ImageSize::new(4000, 3000),
            policy
        ));
        assert!(!should_request_full_resolution_for_view(
            ImageBufferKind::Preview,
            0.25,
            ImageSize::new(4000, 3000),
            policy
        ));
        assert!(!should_request_full_resolution_for_view(
            ImageBufferKind::FullResolution,
            1.0,
            ImageSize::new(4000, 3000),
            policy
        ));
        assert!(!should_request_full_resolution_for_view(
            ImageBufferKind::Preview,
            1.0,
            ImageSize::new(9000, 5000),
            policy
        ));
    }

    #[test]
    fn memory_cache_eviction_uses_lru_count_entry_and_total_budgets() {
        let policy = ImageMemoryPolicy::DEFAULT;
        let entries = [
            MemoryCacheEntry::new(ImageCacheSlot::OrientedImage, 48 * 1024 * 1024, 3),
            MemoryCacheEntry::new(ImageCacheSlot::ScaledImage, 80 * 1024 * 1024, 2),
            MemoryCacheEntry::new(
                ImageCacheSlot::AnimationFrame {
                    frame_index: Some(7),
                },
                24 * 1024 * 1024,
                1,
            ),
        ];

        let evicted = memory_cache_slots_to_evict(160 * 1024 * 1024, &entries, policy);

        assert_eq!(
            evicted,
            vec![
                ImageCacheSlot::AnimationFrame {
                    frame_index: Some(7),
                },
                ImageCacheSlot::ScaledImage
            ]
        );
    }

    #[test]
    fn memory_cache_eviction_drops_oversized_entry_even_when_total_fits() {
        let policy = ImageMemoryPolicy::DEFAULT;
        let entries = [
            MemoryCacheEntry::new(
                ImageCacheSlot::OrientedImage,
                policy.max_cache_entry_bytes() + 1,
                10,
            ),
            MemoryCacheEntry::new(ImageCacheSlot::ScaledImage, 1, 1),
        ];

        let evicted = memory_cache_slots_to_evict(0, &entries, policy);

        assert_eq!(evicted, vec![ImageCacheSlot::OrientedImage]);
    }

    #[test]
    fn decode_generation_marks_only_non_active_results_as_stale() {
        let first = DecodeGeneration::ZERO.next();
        let second = first.next();

        assert!(!is_stale_decode_generation(first, first));
        assert!(is_stale_decode_generation(second, first));
        assert_ne!(first.value(), second.value());
    }

    #[test]
    fn animation_delay_normalization_clamps_zero_fast_and_extreme_values() {
        assert_eq!(
            normalize_animation_frame_delay_ms(0),
            super::DEFAULT_ANIMATION_FRAME_DELAY_MS
        );
        assert_eq!(
            normalize_animation_frame_delay_ms(1),
            super::MIN_ANIMATION_FRAME_DELAY_MS
        );
        assert_eq!(normalize_animation_frame_delay_ms(125), 125);
        assert_eq!(
            normalize_animation_frame_delay_ms(u32::MAX),
            super::MAX_ANIMATION_FRAME_DELAY_MS
        );
    }

    #[test]
    fn animation_timing_settings_normalize_playback_delays() {
        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(30);
        timing.set_max_frame_delay_ms(200);
        timing.set_default_frame_delay_ms(80);

        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(0, timing),
            80
        );
        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(1, timing),
            30
        );
        assert_eq!(
            normalize_animation_frame_delay_ms_with_settings(500, timing),
            200
        );

        let playback = AnimationPlayback::new_with_timing(
            vec![0, 1, 500],
            AnimationLoopPolicy::Infinite,
            timing,
        )
        .expect("custom timing playback");

        assert_eq!(playback.frame_delays_ms(), &[80, 30, 200]);
    }

    #[test]
    fn animation_playback_retiming_preserves_source_zero_delays() {
        let playback =
            AnimationPlayback::new(vec![0, 5, 500], AnimationLoopPolicy::Infinite).unwrap();
        assert_eq!(playback.frame_delays_ms(), &[100, 10, 500]);

        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(50);
        timing.set_max_frame_delay_ms(200);
        timing.set_default_frame_delay_ms(150);
        let retimed = playback.with_timing_settings(timing);

        assert_eq!(retimed.frame_delays_ms(), &[150, 50, 200]);
        assert_eq!(
            retimed.current_frame_index(),
            playback.current_frame_index()
        );
        assert_eq!(retimed.playback_state(), playback.playback_state());
    }

    #[test]
    fn animation_playback_clone_reuses_delay_storage() {
        let playback =
            AnimationPlayback::new(vec![0, 5, 500], AnimationLoopPolicy::Infinite).unwrap();
        let cloned = playback.clone();

        assert!(std::sync::Arc::ptr_eq(
            &playback.source_frame_delays_ms,
            &cloned.source_frame_delays_ms
        ));
        assert!(std::sync::Arc::ptr_eq(
            &playback.frame_delays_ms,
            &cloned.frame_delays_ms
        ));

        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(50);
        let retimed = playback.with_timing_settings(timing);

        assert!(std::sync::Arc::ptr_eq(
            &playback.source_frame_delays_ms,
            &retimed.source_frame_delays_ms
        ));
        assert!(!std::sync::Arc::ptr_eq(
            &playback.frame_delays_ms,
            &retimed.frame_delays_ms
        ));
    }

    #[test]
    fn animation_timer_advances_frames_and_wraps_infinite_loop() {
        let playback =
            AnimationPlayback::new(vec![0, 40, 50], AnimationLoopPolicy::Infinite).unwrap();

        assert_eq!(
            animation_timer_interval_ms(&playback),
            Some(super::DEFAULT_ANIMATION_FRAME_DELAY_MS)
        );

        let second = animation_state_after_timer_tick(&playback);
        assert_eq!(second.frame_index(), Some(1));
        assert_eq!(second.state().current_frame_index(), 1);

        let third = animation_state_after_timer_tick(second.state());
        assert_eq!(third.frame_index(), Some(2));
        assert_eq!(third.state().current_frame_index(), 2);

        let wrapped = animation_state_after_timer_tick(third.state());
        assert_eq!(wrapped.frame_index(), Some(0));
        assert_eq!(wrapped.state().current_frame_index(), 0);
        assert_eq!(wrapped.state().completed_loops(), 1);
        assert_eq!(
            wrapped.state().playback_state(),
            AnimationPlaybackState::Playing
        );
    }

    #[test]
    fn finite_animation_finishes_on_last_frame_after_requested_repeats() {
        let playback =
            AnimationPlayback::new(vec![30, 40], AnimationLoopPolicy::finite(1)).unwrap();
        let last = animation_state_after_timer_tick(&playback);
        let finished = animation_state_after_timer_tick(last.state());

        assert_eq!(finished.frame_index(), None);
        assert_eq!(finished.state().current_frame_index(), 1);
        assert_eq!(finished.state().completed_loops(), 1);
        assert_eq!(
            finished.state().playback_state(),
            AnimationPlaybackState::Finished
        );
        assert_eq!(animation_timer_interval_ms(finished.state()), None);
    }

    #[test]
    fn animation_toggle_pause_resume_and_restart_finished_state() {
        let playback =
            AnimationPlayback::new(vec![30, 40], AnimationLoopPolicy::finite(1)).unwrap();

        let paused = animation_state_after_toggle(&playback);
        assert_eq!(paused.frame_index(), None);
        assert_eq!(
            paused.state().playback_state(),
            AnimationPlaybackState::Paused
        );
        assert_eq!(animation_timer_interval_ms(paused.state()), None);

        let resumed = animation_state_after_toggle(paused.state());
        assert_eq!(
            resumed.state().playback_state(),
            AnimationPlaybackState::Playing
        );

        let last = animation_state_after_timer_tick(resumed.state());
        let finished = animation_state_after_timer_tick(last.state());
        let restarted = animation_state_after_toggle(finished.state());
        assert_eq!(restarted.frame_index(), Some(0));
        assert_eq!(restarted.state().current_frame_index(), 0);
        assert_eq!(restarted.state().completed_loops(), 0);
        assert_eq!(
            restarted.state().playback_state(),
            AnimationPlaybackState::Playing
        );
    }

    #[test]
    fn paused_animation_ignores_timer_and_resume_keeps_current_frame_delay() {
        let playback =
            AnimationPlayback::new(vec![25, 80, 120], AnimationLoopPolicy::Infinite).unwrap();
        let on_second_frame = animation_state_after_timer_tick(&playback);
        let paused = animation_state_after_toggle(on_second_frame.state());

        assert_eq!(paused.frame_index(), None);
        assert_eq!(paused.state().current_frame_index(), 1);
        assert_eq!(
            paused.state().playback_state(),
            AnimationPlaybackState::Paused
        );
        assert_eq!(animation_timer_interval_ms(paused.state()), None);

        let ignored = animation_state_after_timer_tick(paused.state());
        assert_eq!(ignored.frame_index(), None);
        assert_eq!(ignored.state().current_frame_index(), 1);
        assert_eq!(
            ignored.state().playback_state(),
            AnimationPlaybackState::Paused
        );

        let resumed = animation_state_after_toggle(ignored.state());
        assert_eq!(
            resumed.state().playback_state(),
            AnimationPlaybackState::Playing
        );
        assert_eq!(resumed.state().current_frame_index(), 1);
        assert_eq!(animation_timer_interval_ms(resumed.state()), Some(80));
    }

    #[test]
    fn manual_animation_frame_navigation_pauses_and_clamps() {
        let playback =
            AnimationPlayback::new(vec![30, 40, 50], AnimationLoopPolicy::Infinite).unwrap();

        let previous =
            animation_state_after_manual_step(&playback, AnimationFrameStepDirection::Previous);
        assert_eq!(previous.frame_index(), None);
        assert_eq!(previous.state().current_frame_index(), 0);
        assert_eq!(
            previous.state().playback_state(),
            AnimationPlaybackState::Paused
        );

        let next = animation_state_after_manual_step(&playback, AnimationFrameStepDirection::Next);
        assert_eq!(next.frame_index(), Some(1));
        assert_eq!(next.state().current_frame_index(), 1);
        assert_eq!(
            next.state().playback_state(),
            AnimationPlaybackState::Paused
        );

        let home = animation_state_after_home(next.state());
        assert_eq!(home.frame_index(), Some(0));
        assert_eq!(home.state().current_frame_index(), 0);
    }

    #[test]
    fn scaling_cache_invalidation_rebuilds_only_when_key_changes() {
        let key = ScalingCacheKey::new(
            1,
            ImageOrientation::Normal,
            ImageSize::new(512, 416),
            ScalingQuality::Balanced,
        );

        assert!(should_rebuild_scaling_cache(None, key));
        assert!(!should_rebuild_scaling_cache(Some(key), key));
        assert!(should_rebuild_scaling_cache(
            Some(key),
            ScalingCacheKey::new(
                2,
                ImageOrientation::Normal,
                ImageSize::new(512, 416),
                ScalingQuality::Balanced,
            )
        ));
        assert!(should_rebuild_scaling_cache(
            Some(key),
            ScalingCacheKey::new(
                1,
                ImageOrientation::Rotate90,
                ImageSize::new(512, 416),
                ScalingQuality::Balanced,
            )
        ));
        assert!(should_rebuild_scaling_cache(
            Some(key),
            ScalingCacheKey::new(
                1,
                ImageOrientation::Normal,
                ImageSize::new(528, 416),
                ScalingQuality::Balanced,
            )
        ));
        assert!(should_rebuild_scaling_cache(
            Some(key),
            ScalingCacheKey::new(
                1,
                ImageOrientation::Normal,
                ImageSize::new(512, 416),
                ScalingQuality::HighQuality,
            )
        ));
    }

    #[test]
    fn zoom_scale_clamps_to_supported_range() {
        assert_approx_eq(clamp_zoom_scale(0.001), MIN_ZOOM_SCALE);
        assert_approx_eq(clamp_zoom_scale(0.5), 0.5);
        assert_approx_eq(clamp_zoom_scale(100.0), MAX_ZOOM_SCALE);
        assert_approx_eq(clamp_zoom_scale(f64::INFINITY), MAX_ZOOM_SCALE);
        assert_approx_eq(clamp_zoom_scale(f64::NAN), MIN_ZOOM_SCALE);
    }

    #[test]
    fn zoom_settings_drive_clamping_transform_and_status_text() {
        let settings = ZoomSettings::new(0.25, 4.0, 2.0);
        let viewport = ViewportSize::from_client_size(100, 100);
        let image_size = ImageSize::new(100, 100);
        let anchor = ViewportPoint::from_client_position(50, 50);

        assert_approx_eq(clamp_zoom_scale_with_settings(0.001, settings), 0.25);
        assert_approx_eq(clamp_zoom_scale_with_settings(100.0, settings), 4.0);
        assert_approx_eq(
            ViewTransform::manual_zoom_with_settings(100.0, ViewOffset::ZERO, settings)
                .zoom_scale(),
            4.0,
        );

        let zoomed = ViewTransform::FIT_TO_WINDOW
            .zoom_at_with_settings(viewport, image_size, 100.0, anchor, settings);

        assert_approx_eq(zoomed.zoom_scale(), 4.0);
        assert_eq!(zoom_status_text_with_settings(zoomed, settings), "400%");
    }

    #[test]
    fn manual_zoom_out_stops_at_fit_to_window_scale() {
        let viewport = ViewportSize::from_client_size(800, 600);
        let image_size = ImageSize::new(1600, 1200);
        let anchor = ViewportPoint::from_client_position(400, 300);

        let unchanged_fit = ViewTransform::FIT_TO_WINDOW.zoom_at(viewport, image_size, 0.5, anchor);
        let zoomed_out = ViewTransform::manual_zoom(2.0, ViewOffset::ZERO)
            .zoom_at(viewport, image_size, 0.01, anchor);
        let rect = zoomed_out
            .display_rect(viewport, image_size)
            .expect("minimum zoom rect");

        assert_eq!(unchanged_fit, ViewTransform::FIT_TO_WINDOW);
        assert_eq!(zoomed_out.mode(), ViewMode::ManualZoom);
        assert_approx_eq(zoomed_out.zoom_scale(), 0.5);
        assert_eq!(
            (rect.x(), rect.y(), rect.width(), rect.height()),
            (0, 0, 800, 600)
        );
    }

    #[test]
    fn manual_zoom_out_does_not_shrink_below_actual_size_for_small_images() {
        let viewport = ViewportSize::from_client_size(800, 600);
        let image_size = ImageSize::new(100, 100);
        let anchor = ViewportPoint::from_client_position(400, 300);

        let transform = ViewTransform::ACTUAL_SIZE.zoom_at(viewport, image_size, 0.5, anchor);
        let rect = transform
            .display_rect(viewport, image_size)
            .expect("actual-size floor rect");

        assert_eq!(transform, ViewTransform::ACTUAL_SIZE);
        assert_eq!(
            (rect.x(), rect.y(), rect.width(), rect.height()),
            (350, 250, 100, 100)
        );
    }

    #[test]
    fn extreme_manual_zoom_clamps_scale_and_keeps_rect_covering_viewport() {
        let viewport = ViewportSize::from_client_size(100, 80);
        let image_size = ImageSize::new(10, 10);

        let max_offset = clamp_view_offset(
            viewport,
            image_size,
            1000.0,
            ViewOffset::new(100_000.0, 100_000.0),
        );
        let max_rect = ViewTransform::manual_zoom(1000.0, max_offset)
            .display_rect(viewport, image_size)
            .expect("max zoom rect");

        assert_approx_eq(
            ViewTransform::manual_zoom(1000.0, ViewOffset::ZERO).zoom_scale(),
            MAX_ZOOM_SCALE,
        );
        assert_eq!((max_rect.x(), max_rect.y()), (0, 0));
        assert_eq!((max_rect.width(), max_rect.height()), (320, 320));

        let min_offset = clamp_view_offset(
            viewport,
            image_size,
            1000.0,
            ViewOffset::new(-100_000.0, -100_000.0),
        );
        let min_rect = ViewTransform::manual_zoom(1000.0, min_offset)
            .display_rect(viewport, image_size)
            .expect("min zoom rect");

        assert_eq!((min_rect.x(), min_rect.y()), (-220, -240));
        assert_eq!((min_rect.width(), min_rect.height()), (320, 320));
    }

    #[test]
    fn cursor_anchored_zoom_preserves_image_point_when_unclamped() {
        let viewport = ViewportSize::from_client_size(500, 500);
        let image_size = ImageSize::new(1000, 1000);
        let anchor = ViewportPoint::new(125.0, 125.0);

        let transform = ViewTransform::FIT_TO_WINDOW.zoom_at(viewport, image_size, 2.0, anchor);
        let rect = transform
            .display_rect(viewport, image_size)
            .expect("zoom rect");

        assert_eq!(transform.mode(), ViewMode::ManualZoom);
        assert_approx_eq(transform.zoom_scale(), 1.0);
        assert_eq!(rect.x(), -125);
        assert_eq!(rect.y(), -125);
        assert_eq!(rect.width(), 1000);
        assert_eq!(rect.height(), 1000);
        assert_approx_eq(
            (anchor.x() - f64::from(rect.x())) / transform.zoom_scale(),
            250.0,
        );
        assert_approx_eq(
            (anchor.y() - f64::from(rect.y())) / transform.zoom_scale(),
            250.0,
        );
    }

    #[test]
    fn cursor_anchored_zoom_uses_rotated_display_size() {
        let viewport = ViewportSize::from_client_size(400, 300);
        let display_size = ImageSize::new(800, 200).with_rotation(ImageRotation::Degrees90);
        let anchor = ViewportPoint::new(200.0, 75.0);

        let transform = ViewTransform::FIT_TO_WINDOW.zoom_at(viewport, display_size, 2.0, anchor);
        let rect = transform
            .display_rect(viewport, display_size)
            .expect("rotated zoom rect");

        assert_eq!(transform.mode(), ViewMode::ManualZoom);
        assert_approx_eq(transform.zoom_scale(), 0.75);
        assert_eq!(
            (rect.x(), rect.y(), rect.width(), rect.height()),
            (125, -75, 150, 600)
        );
        assert_approx_eq(
            (anchor.y() - f64::from(rect.y())) / transform.zoom_scale(),
            200.0,
        );
    }

    #[test]
    fn manual_zoom_resize_preserves_center_anchor_and_clamps_offset() {
        let old_viewport = ViewportSize::from_client_size(400, 300);
        let new_viewport = ViewportSize::from_client_size(800, 600);
        let image_size = ImageSize::new(1000, 800);
        let transform = ViewTransform::manual_zoom(1.0, ViewOffset::new(100.0, 50.0));
        let old_rect = transform
            .display_rect(old_viewport, image_size)
            .expect("old rect");

        let resized = transform.resize_viewport(old_viewport, new_viewport, image_size);
        let new_rect = resized
            .display_rect(new_viewport, image_size)
            .expect("new rect");

        assert_eq!(resized.mode(), ViewMode::ManualZoom);
        assert_approx_eq(resized.zoom_scale(), 1.0);
        assert_eq!((new_rect.x(), new_rect.y()), (0, -50));
        assert_approx_eq(
            (f64::from(old_viewport.width()) / 2.0 - f64::from(old_rect.x()))
                / transform.zoom_scale(),
            (f64::from(new_viewport.width()) / 2.0 - f64::from(new_rect.x()))
                / resized.zoom_scale(),
        );
        assert_approx_eq(
            (f64::from(old_viewport.height()) / 2.0 - f64::from(old_rect.y()))
                / transform.zoom_scale(),
            (f64::from(new_viewport.height()) / 2.0 - f64::from(new_rect.y()))
                / resized.zoom_scale(),
        );
    }

    #[test]
    fn manual_zoom_resize_after_rotation_keeps_display_covering_viewport() {
        let old_viewport = ViewportSize::from_client_size(400, 300);
        let new_viewport = ViewportSize::from_client_size(700, 500);
        let display_size = ImageSize::new(300, 1200);
        let transform = ViewTransform::manual_zoom(1.0, ViewOffset::new(0.0, -350.0));

        let resized = transform.resize_viewport(old_viewport, new_viewport, display_size);
        let rect = resized
            .display_rect(new_viewport, display_size)
            .expect("resized rotated rect");

        assert_eq!(resized.mode(), ViewMode::ManualZoom);
        assert_eq!(rect.x(), 200);
        assert!(rect.y() <= 0, "{rect:?}");
        assert!(
            rect.y() + rect.height() >= new_viewport.height() as i32,
            "{rect:?}"
        );
    }

    #[test]
    fn clamp_view_offset_limits_large_image_to_viewport_edges() {
        let viewport = ViewportSize::from_client_size(500, 400);
        let image_size = ImageSize::new(1000, 800);

        let min_offset =
            clamp_view_offset(viewport, image_size, 1.0, ViewOffset::new(-1000.0, -1000.0));
        let max_offset =
            clamp_view_offset(viewport, image_size, 1.0, ViewOffset::new(1000.0, 1000.0));

        assert_approx_eq(min_offset.x(), -250.0);
        assert_approx_eq(min_offset.y(), -200.0);
        assert_approx_eq(max_offset.x(), 250.0);
        assert_approx_eq(max_offset.y(), 200.0);
    }

    #[test]
    fn clamp_view_offset_centers_axes_that_fit_inside_viewport() {
        let viewport = ViewportSize::from_client_size(500, 400);
        let image_size = ImageSize::new(1000, 200);

        let offset = clamp_view_offset(viewport, image_size, 1.0, ViewOffset::new(1000.0, 1000.0));
        let rect = ViewTransform::manual_zoom(1.0, offset)
            .display_rect(viewport, image_size)
            .expect("manual zoom rect");

        assert_approx_eq(offset.x(), 250.0);
        assert_approx_eq(offset.y(), 0.0);
        assert_eq!(rect.x(), 0);
        assert_eq!(rect.y(), 100);
        assert_eq!(rect.width(), 1000);
        assert_eq!(rect.height(), 200);
    }

    #[test]
    fn panning_converts_actual_size_to_manual_zoom_when_image_is_larger_than_viewport() {
        let viewport = ViewportSize::from_client_size(500, 400);
        let image_size = ImageSize::new(1000, 800);

        let transform = ViewTransform::ACTUAL_SIZE.pan_to_offset(
            viewport,
            image_size,
            ViewOffset::new(125.0, -80.0),
        );
        let rect = transform
            .display_rect(viewport, image_size)
            .expect("panned rect");

        assert_eq!(transform.mode(), ViewMode::ManualZoom);
        assert_approx_eq(transform.zoom_scale(), 1.0);
        assert_eq!(rect.x(), -125);
        assert_eq!(rect.y(), -280);
    }

    #[test]
    fn panning_is_disabled_when_displayed_image_fits_viewport() {
        let viewport = ViewportSize::from_client_size(500, 400);
        let image_size = ImageSize::new(300, 200);

        assert!(!ViewTransform::ACTUAL_SIZE.can_pan(viewport, image_size));
        assert_eq!(
            ViewTransform::ACTUAL_SIZE.panning_start_offset(viewport, image_size),
            None
        );
    }

    #[test]
    fn image_state_reports_loaded_status() {
        assert!(!ImageState::Empty.has_image());

        let image = LoadedImage::new(
            Rgba8Image::new(1, 1, vec![0, 0, 0, 255]),
            ImageMetadata::new(PathBuf::from("image.png"), 4, SupportedImageFormat::Png),
        );
        assert!(ImageState::Loaded(image).has_image());
    }

    #[test]
    fn image_info_text_uses_file_name_resolution_size_and_format() {
        let image = LoadedImage::new(
            Rgba8Image::new(1920, 1080, Vec::new()),
            ImageMetadata::new(
                PathBuf::from("C:/images/photo.JPG"),
                1_572_864,
                SupportedImageFormat::Jpeg,
            ),
        );

        assert_eq!(
            image_info_text(&image),
            "photo.JPG | 1920x1080 | 1.5 MB | JPEG"
        );
    }

    #[test]
    fn image_info_text_reports_exif_display_orientation_without_losing_source_size() {
        let image = LoadedImage::new(
            Rgba8Image::new(1920, 1080, Vec::new()),
            ImageMetadata::with_exif_orientation(
                PathBuf::from("C:/images/photo.JPG"),
                1_572_864,
                SupportedImageFormat::Jpeg,
                ImageOrientation::Rotate90,
            ),
        );

        assert_eq!(
            image_info_text(&image),
            "photo.JPG | source 1920x1080 | display 1080x1920 | EXIF 6 | 1.5 MB | JPEG"
        );
    }

    #[test]
    fn image_display_info_text_reports_exif_and_user_rotation_separately() {
        let image = LoadedImage::new(
            Rgba8Image::new(1920, 1080, Vec::new()),
            ImageMetadata::with_exif_orientation(
                PathBuf::from("C:/images/photo.JPG"),
                1_572_864,
                SupportedImageFormat::Jpeg,
                ImageOrientation::Rotate90,
            ),
        );

        assert_eq!(
            image_display_info_text(&image, ImageRotation::Degrees90),
            "photo.JPG | source 1920x1080 | display 1920x1080 | EXIF 6 + rotation 90 deg | 1.5 MB | JPEG"
        );
    }

    #[test]
    fn image_status_text_appends_zoom_text() {
        let image = LoadedImage::new(
            Rgba8Image::new(640, 480, Vec::new()),
            ImageMetadata::new(PathBuf::from("photo.png"), 1024, SupportedImageFormat::Png),
        );

        assert_eq!(
            image_status_text(&image, ViewTransform::FIT_TO_WINDOW, ImageRotation::ZERO),
            "photo.png | 640x480 | 1.0 KB | PNG | Fit"
        );
    }

    #[test]
    fn image_status_text_can_use_simple_configured_zoom_text() {
        let image = LoadedImage::new(
            Rgba8Image::new(640, 480, Vec::new()),
            ImageMetadata::new(PathBuf::from("photo.png"), 1024, SupportedImageFormat::Png),
        );
        let zoom = ZoomSettings::new(0.25, 4.0, 2.0);
        let transform = ViewTransform::manual_zoom_with_settings(10.0, ViewOffset::ZERO, zoom);

        assert_eq!(
            image_status_text_with_settings(&image, transform, ImageRotation::ZERO, zoom, false),
            "photo.png | 400%"
        );
    }

    #[test]
    fn zoom_status_text_uses_fit_or_rounded_percentage() {
        assert_eq!(zoom_status_text(ViewTransform::FIT_TO_WINDOW), "Fit");
        assert_eq!(zoom_status_text(ViewTransform::ACTUAL_SIZE), "100%");
        assert_eq!(
            zoom_status_text(ViewTransform::manual_zoom(3.2, ViewOffset::ZERO)),
            "320%"
        );
        assert_eq!(
            zoom_status_text(ViewTransform::manual_zoom(0.333, ViewOffset::ZERO)),
            "33%"
        );
    }

    #[test]
    fn zoom_status_text_uses_clamped_manual_zoom_bounds() {
        assert_eq!(
            zoom_status_text(ViewTransform::manual_zoom(0.001, ViewOffset::ZERO)),
            "5%"
        );
        assert_eq!(
            zoom_status_text(ViewTransform::manual_zoom(100.0, ViewOffset::ZERO)),
            "3200%"
        );
    }

    #[test]
    fn file_size_formatting_uses_readable_binary_units() {
        assert_eq!(format_file_size(0), "0 B");
        assert_eq!(format_file_size(1023), "1023 B");
        assert_eq!(format_file_size(1024), "1.0 KB");
        assert_eq!(format_file_size(1536), "1.5 KB");
        assert_eq!(format_file_size(1_048_576), "1.0 MB");
        assert_eq!(format_file_size(1_073_741_824), "1.0 GB");
        assert_eq!(format_file_size(1_099_511_627_776), "1.0 TB");
    }

    #[test]
    fn standard_format_names_are_canonical_for_display() {
        assert_eq!(
            standard_image_format_name(SupportedImageFormat::Jpeg),
            "JPEG"
        );
        assert_eq!(standard_image_format_name(SupportedImageFormat::Png), "PNG");
        assert_eq!(standard_image_format_name(SupportedImageFormat::Bmp), "BMP");
        assert_eq!(standard_image_format_name(SupportedImageFormat::Gif), "GIF");
        assert_eq!(
            standard_image_format_name(SupportedImageFormat::Webp),
            "WebP"
        );
        assert_eq!(standard_image_format_name(SupportedImageFormat::Ico), "ICO");
        assert_eq!(
            standard_image_format_name(SupportedImageFormat::Tiff),
            "TIFF"
        );
        assert_eq!(standard_image_format_name(SupportedImageFormat::Tga), "TGA");
    }

    #[test]
    fn export_format_detection_is_case_insensitive_and_excludes_gif() {
        let cases = [
            ("photo.JPG", ExportFormat::Jpeg),
            ("photo.jpeg", ExportFormat::Jpeg),
            ("icon.PnG", ExportFormat::Png),
            ("scan.BMP", ExportFormat::Bmp),
            ("poster.WEBP", ExportFormat::Webp),
            ("favicon.ICO", ExportFormat::Ico),
        ];

        for (path, expected) in cases {
            assert_eq!(export_format_for_path(path), Some(expected));
        }

        assert_eq!(
            export_format_for_extension(".JPEG"),
            Some(ExportFormat::Jpeg)
        );
        assert_eq!(export_format_for_path("anim.gif"), None);
    }

    #[test]
    fn export_format_metadata_is_canonical() {
        assert_eq!(export_format_display_name(ExportFormat::Jpeg), "JPEG");
        assert_eq!(export_format_mime_type(ExportFormat::Jpeg), "image/jpeg");
        assert_eq!(export_format_default_extension(ExportFormat::Jpeg), "jpg");
        assert_eq!(
            export_format_extensions(ExportFormat::Jpeg),
            ["jpg", "jpeg"]
        );
        assert_eq!(export_format_mime_type(ExportFormat::Png), "image/png");
        assert_eq!(export_format_mime_type(ExportFormat::Bmp), "image/bmp");
        assert_eq!(export_format_mime_type(ExportFormat::Webp), "image/webp");
        assert_eq!(
            export_format_mime_type(ExportFormat::Ico),
            "image/vnd.microsoft.icon"
        );
        assert_eq!(export_format_default_extension(ExportFormat::Ico), "ico");
        assert_eq!(export_format_extensions(ExportFormat::Ico), ["ico"]);
        assert_eq!(
            default_export_format_for_source_format(SupportedImageFormat::Gif),
            ExportFormat::Png
        );
        assert_eq!(
            default_export_format_for_source_format(SupportedImageFormat::Ico),
            ExportFormat::Ico
        );
        assert_eq!(
            default_export_format_for_source_format(SupportedImageFormat::Tiff),
            ExportFormat::Png
        );
        assert_eq!(
            default_export_format_for_source_format(SupportedImageFormat::Tga),
            ExportFormat::Png
        );
    }

    #[test]
    fn export_extension_correction_uses_selected_format() {
        assert_eq!(
            export_path_with_format_extension("C:/images/photo.png", ExportFormat::Jpeg),
            PathBuf::from("C:/images/photo.jpg")
        );
        assert_eq!(
            export_path_with_format_extension("C:/images/photo.jpeg", ExportFormat::Jpeg),
            PathBuf::from("C:/images/photo.jpeg")
        );
        assert_eq!(
            export_path_with_format_extension("C:/images/photo", ExportFormat::Png),
            PathBuf::from("C:/images/photo.png")
        );
        assert_eq!(
            export_path_with_format_extension("C:/images/photo.png", ExportFormat::Ico),
            PathBuf::from("C:/images/photo.ico")
        );
    }

    #[test]
    fn export_quality_range_clamps_lossy_formats_only() {
        let range = export_quality_range(ExportFormat::Jpeg).expect("jpeg range");
        assert_eq!(range.min(), 1);
        assert_eq!(range.max(), 100);
        assert_eq!(range.default(), 90);
        assert_eq!(clamp_export_quality(ExportFormat::Jpeg, 0), Some(1));
        assert_eq!(clamp_export_quality(ExportFormat::Jpeg, 85), Some(85));
        assert_eq!(clamp_export_quality(ExportFormat::Jpeg, 101), Some(100));
        assert_eq!(clamp_export_quality(ExportFormat::Png, 50), None);
        assert_eq!(clamp_export_quality(ExportFormat::Webp, 50), None);
        assert_eq!(clamp_export_quality(ExportFormat::Ico, 50), None);
    }

    #[test]
    fn export_options_creation_applies_default_policies_and_quality() {
        let jpeg_default = ExportOptions::new(ExportFormat::Jpeg, None);
        assert_eq!(jpeg_default.format(), ExportFormat::Jpeg);
        assert_eq!(jpeg_default.quality(), Some(90));
        assert_eq!(jpeg_default.jpeg_alpha_background_rgb(), RgbColor::WHITE);
        assert_eq!(
            jpeg_default.orientation_policy(),
            ExportOrientationPolicy::Display
        );
        assert_eq!(
            jpeg_default.animation_policy(),
            ExportAnimationPolicy::CurrentFrame
        );
        assert_eq!(jpeg_default.rotation(), ImageRotation::Degrees0);
        assert_eq!(jpeg_default.target_size(), None);
        assert!(!jpeg_default.remove_metadata());

        let jpeg_clamped = ExportOptions::new(ExportFormat::Jpeg, Some(200));
        assert_eq!(jpeg_clamped.quality(), Some(100));

        let jpeg_clamped_low = ExportOptions::new(ExportFormat::Jpeg, Some(0));
        assert_eq!(jpeg_clamped_low.quality(), Some(1));

        let png = ExportOptions::new(ExportFormat::Png, Some(10));
        assert_eq!(png.quality(), None);
        assert_eq!(png.orientation_policy(), ExportOrientationPolicy::Display);
        assert_eq!(png.animation_policy(), ExportAnimationPolicy::CurrentFrame);
        assert_eq!(
            png.with_rotation(ImageRotation::Degrees270).rotation(),
            ImageRotation::Degrees270
        );

        let jpeg_with_background =
            jpeg_default.with_jpeg_alpha_background_rgb(RgbColor::new(10, 20, 30));
        assert_eq!(
            jpeg_with_background.jpeg_alpha_background_rgb(),
            RgbColor::new(10, 20, 30)
        );
        assert!(jpeg_default.with_remove_metadata(true).remove_metadata());

        let resized = jpeg_default.with_target_size(Some(ImageSize::new(320, 180)));
        assert_eq!(resized.target_size(), Some(ImageSize::new(320, 180)));
        assert_eq!(
            jpeg_default
                .with_target_size(Some(ImageSize::new(0, 180)))
                .target_size(),
            None
        );
        assert_eq!(
            ExportOptions::new(ExportFormat::Ico, None)
                .with_target_size(Some(ImageSize::new(32, 32)))
                .target_size(),
            None
        );
    }

    #[test]
    fn export_resize_helpers_preserve_aspect_ratio_with_rounding() {
        let source = ImageSize::new(4000, 3000);

        assert_eq!(
            export_size_from_width_preserving_aspect(source, 1024),
            Some(ImageSize::new(1024, 768))
        );
        assert_eq!(
            export_size_from_height_preserving_aspect(source, 500),
            Some(ImageSize::new(667, 500))
        );
        assert_eq!(export_size_from_width_preserving_aspect(source, 0), None);
        assert_eq!(
            export_size_from_width_preserving_aspect(ImageSize::new(0, 3000), 1024),
            None
        );
    }

    #[test]
    fn suggested_export_path_adds_export_suffix() {
        assert_eq!(
            suggested_export_path("C:/images/photo.jpg", ExportFormat::Png),
            PathBuf::from("C:/images/photo-export.png")
        );
        assert_eq!(
            suggested_export_path("photo", ExportFormat::Jpeg),
            PathBuf::from("photo-export.jpg")
        );
    }

    #[test]
    fn suggested_export_path_uses_configured_safe_suffix() {
        assert_eq!(
            suggested_export_path_with_suffix("C:/images/photo.jpg", ExportFormat::Png, "_edited"),
            PathBuf::from("C:/images/photo_edited.png")
        );
        assert_eq!(
            suggested_export_path_with_suffix("photo", ExportFormat::Jpeg, "bad/name"),
            PathBuf::from("photo-export.jpg")
        );
    }

    #[test]
    fn key_input_maps_to_viewer_commands() {
        let cases = [
            (key(KeyCode::O).control(), Command::OpenImage),
            (key(KeyCode::S).control(), Command::ExportImage),
            (key(KeyCode::S).control().shift(), Command::ExportImage),
            (key(KeyCode::C).control(), Command::CopyImageToClipboard),
            (
                key(KeyCode::Right),
                Command::Navigate(ImageNavigationDirection::Next),
            ),
            (
                key(KeyCode::Space),
                Command::Navigate(ImageNavigationDirection::Next),
            ),
            (
                key(KeyCode::PageDown),
                Command::Navigate(ImageNavigationDirection::Next),
            ),
            (
                key(KeyCode::P),
                Command::Animation(AnimationCommand::TogglePlayback),
            ),
            (
                key(KeyCode::BracketLeft),
                Command::Animation(AnimationCommand::StepFrame(
                    AnimationFrameStepDirection::Previous,
                )),
            ),
            (
                key(KeyCode::BracketRight),
                Command::Animation(AnimationCommand::StepFrame(
                    AnimationFrameStepDirection::Next,
                )),
            ),
            (
                key(KeyCode::Home),
                Command::Animation(AnimationCommand::FirstFrame),
            ),
            (
                key(KeyCode::Left),
                Command::Navigate(ImageNavigationDirection::Previous),
            ),
            (
                key(KeyCode::Backspace),
                Command::Navigate(ImageNavigationDirection::Previous),
            ),
            (
                key(KeyCode::PageUp),
                Command::Navigate(ImageNavigationDirection::Previous),
            ),
            (key(KeyCode::Equals).shift(), Command::ZoomIn),
            (key(KeyCode::Equals), Command::ZoomIn),
            (key(KeyCode::NumpadAdd), Command::ZoomIn),
            (key(KeyCode::Minus), Command::ZoomOut),
            (key(KeyCode::NumpadSubtract), Command::ZoomOut),
            (key(KeyCode::Digit1), Command::ActualSize),
            (key(KeyCode::Digit0), Command::FitToWindow),
            (key(KeyCode::R), Command::RotateClockwise),
            (key(KeyCode::R).shift(), Command::RotateCounterClockwise),
            (key(KeyCode::F11), Command::ToggleFullscreen),
            (key(KeyCode::Enter).alt(), Command::ToggleFullscreen),
            (key(KeyCode::Escape), Command::ExitFullscreenOrQuit),
            (key(KeyCode::Q), Command::Quit),
            (key(KeyCode::F4).alt(), Command::Quit),
        ];

        for (input, expected) in cases {
            assert_eq!(command_for_key_input(input), Some(expected), "{input:?}");
        }
    }

    #[test]
    fn space_key_resolves_by_image_context() {
        assert_eq!(
            command_for_key_input_with_context(key(KeyCode::Space), CommandContext::StaticImage),
            Some(Command::Navigate(ImageNavigationDirection::Next))
        );
        assert_eq!(
            command_for_key_input_with_context(key(KeyCode::Space), CommandContext::AnimationImage),
            Some(Command::Animation(AnimationCommand::TogglePlayback))
        );
    }

    #[test]
    fn key_input_ignores_conflicting_modifier_combinations() {
        assert_eq!(command_for_key_input(key(KeyCode::O)), None);
        assert_eq!(
            command_for_key_input(key(KeyCode::O).control().shift()),
            None
        );
        assert_eq!(
            command_for_key_input(key(KeyCode::C).control().shift()),
            None
        );
        assert_eq!(command_for_key_input(key(KeyCode::S).control().alt()), None);
        assert_eq!(command_for_key_input(key(KeyCode::Right).control()), None);
        assert_eq!(command_for_key_input(key(KeyCode::Enter)), None);
        assert_eq!(command_for_key_input(key(KeyCode::F4)), None);
    }

    #[test]
    fn supported_format_detection_is_case_insensitive() {
        let cases = [
            ("photo.JPG", SupportedImageFormat::Jpeg),
            ("photo.jpeg", SupportedImageFormat::Jpeg),
            ("icon.PnG", SupportedImageFormat::Png),
            ("scan.BMP", SupportedImageFormat::Bmp),
            ("clip.Gif", SupportedImageFormat::Gif),
            ("poster.WEBP", SupportedImageFormat::Webp),
            ("favicon.ICO", SupportedImageFormat::Ico),
            ("scan.tif", SupportedImageFormat::Tiff),
            ("scan.TIFF", SupportedImageFormat::Tiff),
            ("sprite.TGA", SupportedImageFormat::Tga),
        ];

        for (path, expected) in cases {
            assert_eq!(supported_image_format_for_path(path), Some(expected));
            assert!(is_supported_image_path(path));
        }
    }

    #[test]
    fn unsupported_format_detection_rejects_unknown_extensions() {
        assert_eq!(supported_image_format_for_path("notes.txt"), None);
        assert!(!is_supported_image_path("archive.tar.gz"));
        assert!(!is_supported_image_path("no-extension"));
    }

    #[test]
    fn first_supported_image_path_keeps_drop_order() {
        let paths = [
            PathBuf::from("notes.txt"),
            PathBuf::from("photo.PNG"),
            PathBuf::from("later.jpg"),
        ];

        assert_eq!(
            first_supported_image_path(paths.iter().map(PathBuf::as_path)),
            Some(paths[1].as_path())
        );
    }

    #[test]
    fn image_folder_filters_supported_paths_and_sorts_by_case_insensitive_file_name() {
        let folder = ImageFolder::from_paths(
            "C:/images/photo.jpg",
            [
                "C:/images/zeta.PNG",
                "C:/images/Photo.JPG",
                "C:/images/notes.txt",
                "C:/images/alpha.bmp",
                "C:/images/photo.jpg",
                "C:/images/clip.webp",
                "C:/images/favicon.ico",
            ]
            .into_iter()
            .map(PathBuf::from),
        );

        assert_eq!(
            image_folder_file_names(&folder),
            [
                "alpha.bmp",
                "clip.webp",
                "favicon.ico",
                "Photo.JPG",
                "photo.jpg",
                "zeta.PNG"
            ]
        );
        assert_eq!(folder.current_index(), Some(4));
    }

    #[test]
    fn image_folder_limits_snapshot_paths() {
        let current_path = PathBuf::from("C:/images/current.png");
        let paths = std::iter::once(current_path.clone()).chain(
            (0..(MAX_IMAGE_FOLDER_SNAPSHOT_PATHS + 8))
                .map(|index| PathBuf::from(format!("C:/images/{index:05}.png"))),
        );

        let folder = ImageFolder::from_paths(&current_path, paths);

        assert_eq!(folder.len(), MAX_IMAGE_FOLDER_SNAPSHOT_PATHS);
        assert!(folder.paths().iter().any(|path| path == &current_path));
        assert!(folder.current_index().is_some());
    }

    #[test]
    fn image_folder_matches_current_index_by_file_name_when_paths_differ() {
        let folder = ImageFolder::from_paths(
            "C:/images/beta.png",
            ["beta.png", "alpha.png"].into_iter().map(PathBuf::from),
        );

        assert_eq!(image_folder_file_names(&folder), ["alpha.png", "beta.png"]);
        assert_eq!(folder.current_index(), Some(1));
    }

    #[test]
    fn image_folder_matches_current_index_by_case_insensitive_file_name() {
        let folder = ImageFolder::from_paths(
            "C:/images/photo.png",
            ["alpha.png", "PHOTO.PNG", "zeta.png"]
                .into_iter()
                .map(PathBuf::from),
        );

        assert_eq!(
            image_folder_file_names(&folder),
            ["alpha.png", "PHOTO.PNG", "zeta.png"]
        );
        assert_eq!(folder.current_index(), Some(1));
    }

    #[test]
    fn image_folder_retargets_current_path_without_resorting_paths() {
        let mut folder = ImageFolder::from_paths(
            "b.png",
            ["c.png", "a.png", "b.png"].into_iter().map(PathBuf::from),
        );

        assert!(folder.retarget_current_path("c.png"));

        assert_eq!(
            image_folder_file_names(&folder),
            ["a.png", "b.png", "c.png"]
        );
        assert_eq!(folder.current_index(), Some(2));
    }

    #[test]
    fn navigation_index_wraps_next_and_previous() {
        assert_eq!(
            navigation_index(3, Some(2), ImageNavigationDirection::Next),
            Some(0)
        );
        assert_eq!(
            navigation_index(3, Some(0), ImageNavigationDirection::Previous),
            Some(2)
        );
    }

    #[test]
    fn navigation_settings_can_disable_wrapping() {
        let mut settings = NavigationSettings::default();
        settings.set_wrap_navigation(false);

        assert_eq!(
            navigation_index_with_settings(3, Some(2), ImageNavigationDirection::Next, settings),
            None
        );
        assert_eq!(
            navigation_index_with_settings(
                3,
                Some(0),
                ImageNavigationDirection::Previous,
                settings
            ),
            None
        );
        assert_eq!(
            navigation_index_with_settings(3, Some(1), ImageNavigationDirection::Next, settings),
            Some(2)
        );
    }

    #[test]
    fn image_folder_navigation_selects_adjacent_paths() {
        let folder = ImageFolder::from_paths(
            "b.png",
            ["a.png", "b.png", "c.png"].into_iter().map(PathBuf::from),
        );

        assert_eq!(
            folder.navigation_path(ImageNavigationDirection::Next),
            Some(PathBuf::from("c.png").as_path())
        );
        assert_eq!(
            folder.navigation_path(ImageNavigationDirection::Previous),
            Some(PathBuf::from("a.png").as_path())
        );
    }

    #[test]
    fn image_folder_navigation_attempts_skip_over_failed_targets() {
        let folder = ImageFolder::from_paths(
            "b.png",
            ["a.png", "b.png", "c.png", "d.png"]
                .into_iter()
                .map(PathBuf::from),
        );
        let settings = NavigationSettings::default();

        assert_eq!(
            folder.navigation_path_for_attempt(ImageNavigationDirection::Next, settings, 0),
            Some(PathBuf::from("c.png").as_path())
        );
        assert_eq!(
            folder.navigation_path_for_attempt(ImageNavigationDirection::Next, settings, 1),
            Some(PathBuf::from("d.png").as_path())
        );
        assert_eq!(
            folder.navigation_path_for_attempt(ImageNavigationDirection::Next, settings, 2),
            Some(PathBuf::from("a.png").as_path())
        );
        assert_eq!(
            folder.navigation_path_for_attempt(ImageNavigationDirection::Next, settings, 3),
            None
        );
    }

    #[test]
    fn image_folder_navigation_is_noop_for_single_or_missing_current_image() {
        let single = ImageFolder::from_paths("only.png", [PathBuf::from("only.png")]);
        assert_eq!(single.len(), 1);
        assert_eq!(single.navigation_path(ImageNavigationDirection::Next), None);
        assert_eq!(
            single.navigation_path(ImageNavigationDirection::Previous),
            None
        );

        let missing = ImageFolder::from_paths(
            "missing.png",
            ["a.png", "b.png"].into_iter().map(PathBuf::from),
        );
        assert_eq!(missing.current_index(), None);
        assert_eq!(
            missing.navigation_path(ImageNavigationDirection::Next),
            None
        );
    }

    trait TestKeyInputModifiers {
        fn control(self) -> Self;
        fn shift(self) -> Self;
        fn alt(self) -> Self;
    }

    impl TestKeyInputModifiers for KeyInput {
        fn control(self) -> Self {
            let modifiers = self.modifiers();
            KeyInput::new(
                self.key(),
                KeyModifiers::new(true, modifiers.shift(), modifiers.alt()),
            )
        }

        fn shift(self) -> Self {
            let modifiers = self.modifiers();
            KeyInput::new(
                self.key(),
                KeyModifiers::new(modifiers.control(), true, modifiers.alt()),
            )
        }

        fn alt(self) -> Self {
            let modifiers = self.modifiers();
            KeyInput::new(
                self.key(),
                KeyModifiers::new(modifiers.control(), modifiers.shift(), true),
            )
        }
    }

    fn key(key: KeyCode) -> KeyInput {
        KeyInput::new(key, KeyModifiers::NONE)
    }

    fn image_folder_file_names(folder: &ImageFolder) -> Vec<String> {
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

    fn numbered_rgba8_image_3x2() -> Rgba8Image {
        let pixels = (1u8..=6)
            .flat_map(|id| [id, id, id, 255])
            .collect::<Vec<_>>();
        Rgba8Image::new(3, 2, pixels)
    }

    fn pixel_ids(image: &Rgba8Image) -> Vec<u8> {
        image
            .pixels()
            .chunks_exact(4)
            .map(|pixel| pixel[0])
            .collect()
    }

    fn assert_approx_eq(left: f64, right: f64) {
        let diff = (left - right).abs();
        assert!(
            diff < 0.000_001,
            "left {left} differs from right {right} by {diff}"
        );
    }
}
