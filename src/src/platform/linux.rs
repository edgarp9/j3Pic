#![allow(deprecated)]

use std::cell::{Cell, RefCell};
use std::error::Error;
use std::fmt;
use std::path::{Path, PathBuf};
use std::rc::Rc;
use std::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use std::sync::{mpsc, Arc};
use std::thread::{self, JoinHandle};
use std::time::Duration;

use gtk::prelude::*;
use gtk::{cairo, gdk, gio, glib, pango};
use gtk4 as gtk;

use super::{
    corrected_export_overwrite_message, corrected_export_path_requires_overwrite_confirmation,
    paths_refer_to_same_existing_file, same_source_export_message, ExportFileSelection,
    PROJECT_LINK_URL,
};
use crate::app::{
    AnimationFrameDecodeRequest, AnimationFrameOutcome, AppCommandOutcome, DecodeApplyOutcome,
    DecodeFailurePresentation, ImageDecodePurpose, ImageDecodeRequest, ImageExportRequest,
    ImagePreloadRequest, NavigationStartOutcome, ViewerApp, ViewerAppError,
};
use crate::domain::{
    command_for_key_input_with_context, export_format_extensions,
    export_path_with_format_extension, export_size_from_height_preserving_aspect,
    export_size_from_width_preserving_aspect, first_supported_image_path,
    validate_export_filename_suffix, AnimationCommand, AnimationTimingSettings, AppConfig, Command,
    CommandContext, DefaultExportFormatPolicy, ExportFilenameSuffixValidationError, ExportFormat,
    ExportOptions, ImageFileVersion, ImageFolder, ImageMemoryPolicy, ImageNavigationDirection,
    ImageRotation, ImageSize, InteractionSettings, KeyCode, KeyInput, KeyModifiers, LoadedImage,
    MemoryPolicySettings, MouseShortcut, PixelImage, RgbColor, ScalingQuality, StatusUiSettings,
    SupportedImageFormat, UiLanguage, ViewMode, ViewportPoint, ViewportSize, WindowBounds,
    ZoomSettings, DEFAULT_EXPORT_FILENAME_SUFFIX, MAX_CONFIG_ANIMATION_DELAY_MS,
    MAX_CONFIG_CACHE_ENTRIES, MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE, MAX_CONFIG_IMAGE_PIXELS,
    MAX_CONFIG_MAX_ZOOM_SCALE, MAX_CONFIG_MEMORY_MIB, MAX_CONFIG_MIN_ZOOM_SCALE,
    MAX_CONFIG_PREVIEW_OVERSAMPLE, MAX_CONFIG_ZOOM_STEP_FACTOR, MAX_EXPORT_FILENAME_SUFFIX_CHARS,
    MAX_EXPORT_QUALITY, MIN_CONFIG_ANIMATION_DELAY_MS, MIN_CONFIG_CACHE_ENTRIES,
    MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE, MIN_CONFIG_MAX_ZOOM_SCALE, MIN_CONFIG_MEMORY_MIB,
    MIN_CONFIG_MIN_ZOOM_SCALE, MIN_CONFIG_PIXEL_LIMIT, MIN_CONFIG_PREVIEW_OVERSAMPLE,
    MIN_CONFIG_ZOOM_STEP_FACTOR, MIN_EXPORT_QUALITY,
};
use crate::infra::{
    cached_animation_frame_pixels_for_loaded_image,
    load_animation_frame_for_view_with_prefetch_and_file_version,
    load_full_resolution_image_with_file_version, load_image_file_for_view_with_timing,
    preload_image_file_for_view_with_timing, save_app_config,
    scan_image_folder_for_file_with_cancellation, AnimationFramePixels, LoadImageError,
    ScanImageFolderError,
};
use crate::ui_text;

const APPLICATION_ID: &str = "io.github.j3pic.viewer";
const APP_ICON_BYTES: &[u8] = include_bytes!("../../icon.ico");
const DEFAULT_WINDOW_WIDTH: i32 = 960;
const DEFAULT_WINDOW_HEIGHT: i32 = 640;
const STATUS_BAR_HEIGHT: i32 = 28;
const STATUS_TEXT_HORIZONTAL_PADDING: i32 = 10;
const DECODE_POLL_INTERVAL: Duration = Duration::from_millis(16);
const EXPORT_POLL_INTERVAL: Duration = Duration::from_millis(50);
const INTERACTIVE_RENDER_SETTLE_INTERVAL: Duration = Duration::from_millis(90);
const MAX_IN_FLIGHT_DECODE_WORKERS: usize = 3;
const MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS: usize = MAX_IN_FLIGHT_DECODE_WORKERS;
const MAX_NAVIGATION_PRELOAD_WORKERS: usize = 2;
const NO_FOLLOW_UP_ANIMATION_FRAME: usize = usize::MAX;
const APPLICATION_FLAGS: gio::ApplicationFlags = gio::ApplicationFlags::NON_UNIQUE;
const OPEN_FILE_FILTER_PATTERNS: &str =
    "*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga";
const OPEN_IMAGE_SUFFIXES: &[&str] = &[
    "jpg", "jpeg", "png", "bmp", "gif", "webp", "ico", "tif", "tiff", "tga",
];
const SUPPORTED_FORMATS_TEXT: &str = "jpg, jpeg, png, bmp, gif, webp, ico, tif, tiff, tga";
const FILE_DROP_MIME_TYPES: &[&str] = &[
    "text/uri-list",
    "x-special/gnome-copied-files",
    "x-special/gnome-icon-list",
    "x-special/mate-icon-list",
    "x-special/nautilus-clipboard",
    "application/x-kde4-urilist",
    "text/plain;charset=utf-8",
    "text/plain",
    "UTF8_STRING",
    "STRING",
];
const DROP_TEXT_READ_CHUNK_BYTES: usize = 64 * 1024;
const DROP_TEXT_READ_MAX_BYTES: usize = 1024 * 1024;
const EXPORT_FORMAT_LABELS: &[&str] = &["PNG", "JPEG", "BMP", "WebP", "ICO"];

#[derive(Debug)]
pub struct GtkError {
    message: String,
}

impl fmt::Display for GtkError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl Error for GtkError {}

pub fn run_native_viewer(
    app: ViewerApp,
    save_config_on_destroy: bool,
    initial_image_path: Option<PathBuf>,
) -> Result<i32, GtkError> {
    let gtk_app = gtk::Application::builder()
        .application_id(APPLICATION_ID)
        .flags(APPLICATION_FLAGS)
        .build();
    let initial_app = RefCell::new(Some(app));
    let initial_path = RefCell::new(initial_image_path);
    let active_viewer: Rc<RefCell<Option<Rc<GtkViewer>>>> = Rc::new(RefCell::new(None));

    let active_viewer_for_activate = Rc::clone(&active_viewer);
    gtk_app.connect_activate(move |gtk_app| {
        let Some(app) = initial_app.borrow_mut().take() else {
            return;
        };
        let startup_path = initial_path.borrow_mut().take();
        let viewer = GtkViewer::new(gtk_app, app, save_config_on_destroy, startup_path);
        viewer.install();
        viewer.present();
        active_viewer_for_activate
            .borrow_mut()
            .replace(Rc::clone(&viewer));
    });

    let active_viewer_for_shutdown = Rc::clone(&active_viewer);
    gtk_app.connect_shutdown(move |_| {
        active_viewer_for_shutdown.borrow_mut().take();
    });

    Ok(i32::from(gtk_app.run_with_args(&["j3pic"]).get()))
}

struct GtkViewer {
    window: gtk::ApplicationWindow,
    root: gtk::Box,
    drawing_area: gtk::DrawingArea,
    status_label: gtk::Label,
    app: RefCell<ViewerApp>,
    decoder: RefCell<GtkDecodeController>,
    exporter: RefCell<GtkExportController>,
    paint_cache: RefCell<CairoSurfaceCache>,
    startup_image_path: RefCell<Option<PathBuf>>,
    save_config_on_destroy: Cell<bool>,
    last_pointer_position: RefCell<Option<(f64, f64)>>,
    animation_timer: RefCell<Option<glib::SourceId>>,
    decode_poll_timer: RefCell<Option<glib::SourceId>>,
    export_poll_timer: RefCell<Option<glib::SourceId>>,
    export_shutdown_poll_timer: RefCell<Option<glib::SourceId>>,
    render_settle_timer: RefCell<Option<glib::SourceId>>,
    windowed_bounds_before_fullscreen: RefCell<Option<WindowBounds>>,
    shutdown_started: Cell<bool>,
    shutdown_can_close: Cell<bool>,
}

impl GtkViewer {
    fn new(
        gtk_app: &gtk::Application,
        app: ViewerApp,
        save_config_on_destroy: bool,
        startup_image_path: Option<PathBuf>,
    ) -> Rc<Self> {
        let bounds = app.window_bounds();
        let width = bounds
            .map(WindowBounds::width)
            .unwrap_or(DEFAULT_WINDOW_WIDTH);
        let height = bounds
            .map(WindowBounds::height)
            .unwrap_or(DEFAULT_WINDOW_HEIGHT)
            .max(STATUS_BAR_HEIGHT + 1);

        let window = gtk::ApplicationWindow::builder()
            .application(gtk_app)
            .title(app.title())
            .icon_name(APPLICATION_ID)
            .default_width(width)
            .default_height(height)
            .build();
        let root = gtk::Box::new(gtk::Orientation::Vertical, 0);
        root.set_can_target(true);
        let drawing_area = gtk::DrawingArea::builder()
            .hexpand(true)
            .vexpand(true)
            .focusable(true)
            .can_target(true)
            .build();
        let status_label = gtk::Label::new(None);
        status_label.set_can_target(true);
        status_label.set_xalign(0.0);
        status_label.set_valign(gtk::Align::Center);
        status_label.set_height_request(STATUS_BAR_HEIGHT);
        status_label.set_margin_start(STATUS_TEXT_HORIZONTAL_PADDING);
        status_label.set_margin_end(STATUS_TEXT_HORIZONTAL_PADDING);
        status_label.set_single_line_mode(true);
        status_label.set_ellipsize(pango::EllipsizeMode::End);
        status_label.add_css_class("dim-label");

        root.append(&drawing_area);
        root.append(&status_label);
        window.set_child(Some(&root));

        Rc::new(Self {
            window,
            root,
            drawing_area,
            status_label,
            app: RefCell::new(app),
            decoder: RefCell::new(GtkDecodeController::new()),
            exporter: RefCell::new(GtkExportController::new()),
            paint_cache: RefCell::new(CairoSurfaceCache::new()),
            startup_image_path: RefCell::new(startup_image_path),
            save_config_on_destroy: Cell::new(save_config_on_destroy),
            last_pointer_position: RefCell::new(None),
            animation_timer: RefCell::new(None),
            decode_poll_timer: RefCell::new(None),
            export_poll_timer: RefCell::new(None),
            export_shutdown_poll_timer: RefCell::new(None),
            render_settle_timer: RefCell::new(None),
            windowed_bounds_before_fullscreen: RefCell::new(None),
            shutdown_started: Cell::new(false),
            shutdown_can_close: Cell::new(false),
        })
    }

    fn install(self: &Rc<Self>) {
        self.install_draw_handler();
        self.install_window_icon();
        self.install_key_handler();
        self.install_scroll_handler();
        self.install_mouse_handlers();
        self.install_drop_handlers();
        self.install_close_handler();
        self.update_status_bar();
        self.queue_startup_image_open();
    }

    fn present(&self) {
        self.window.present();
        self.drawing_area.grab_focus();
    }

    fn install_window_icon(self: &Rc<Self>) {
        self.window.connect_realize(move |window| {
            let Some(surface) = window.surface() else {
                return;
            };
            let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() else {
                return;
            };
            let icons = app_icon_textures();
            if !icons.is_empty() {
                toplevel.set_icon_list(&icons);
            }
        });
    }

    fn install_draw_handler(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.drawing_area
            .set_draw_func(move |_, context, width, height| {
                if let Some(viewer) = weak.upgrade() {
                    viewer.draw(context, width, height);
                }
            });
    }

    fn draw(self: &Rc<Self>, context: &cairo::Context, width: i32, height: i32) {
        context.set_source_rgb(1.0, 1.0, 1.0);
        let _ = context.paint();

        let mut app = self.app.borrow_mut();
        app.handle_resize(width, height);
        let viewport = ViewportSize::from_client_size(width, height);
        let max_cache_entry_bytes = app.memory_policy().max_cache_entry_bytes();
        let Some(render) = app.render_rgba8_for_paint(viewport) else {
            self.paint_cache.borrow_mut().invalidate();
            app.handle_paint();
            self.update_status_bar_from_app(&app);
            return;
        };
        let rect = render.rect();
        let Some(placement) = cairo_paint_placement(
            rect,
            render.pixels().width(),
            render.pixels().height(),
            viewport,
        ) else {
            app.handle_paint();
            self.update_status_bar_from_app(&app);
            return;
        };
        let Some(full_source_rect) =
            CairoSurfaceSourceRect::full(render.pixels().width(), render.pixels().height())
        else {
            app.handle_paint();
            self.update_status_bar_from_app(&app);
            return;
        };
        let cache_bytes = cairo_surface_cache_budget_for_paint_placement(
            max_cache_entry_bytes,
            full_source_rect,
            placement,
        );
        let Some(paint_surface) = self.paint_cache.borrow_mut().surface_for_paint_pixel_rect(
            render.cache_key(),
            render.pixels(),
            placement.source_rect,
            render.scaling_quality(),
            cache_bytes,
        ) else {
            app.handle_paint();
            self.update_status_bar_from_app(&app);
            return;
        };
        let Some(surface_rect) = cairo_surface_dest_rect_for_source_rect(
            rect,
            render.pixels().width(),
            render.pixels().height(),
            paint_surface.source_rect,
        ) else {
            app.handle_paint();
            self.update_status_bar_from_app(&app);
            return;
        };
        let pattern = cairo::SurfacePattern::create(&paint_surface.surface);
        pattern.set_filter(match render.scaling_quality() {
            ScalingQuality::Nearest => cairo::Filter::Nearest,
            ScalingQuality::Balanced => cairo::Filter::Good,
            ScalingQuality::HighQuality => cairo::Filter::Best,
        });
        let _ = context.save();
        context.translate(f64::from(surface_rect.x), f64::from(surface_rect.y));
        let scale_x = f64::from(surface_rect.width) / f64::from(paint_surface.source_rect.width);
        let scale_y = f64::from(surface_rect.height) / f64::from(paint_surface.source_rect.height);
        context.scale(scale_x, scale_y);
        let _ = context.set_source(&pattern);
        let _ = context.paint();
        let _ = context.restore();
        app.handle_paint();
        self.update_status_bar_from_app(&app);

        if app.has_deferred_scaling_cache_rebuild() {
            drop(app);
            self.schedule_interactive_render_settle();
        }
    }

    fn install_key_handler(self: &Rc<Self>) {
        let controller = gtk::EventControllerKey::new();
        let weak = Rc::downgrade(self);
        controller.connect_key_pressed(move |_, key, _, state| {
            let Some(viewer) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            if context_menu_key_from_gdk(key, state) {
                viewer.show_keyboard_context_menu();
                return glib::Propagation::Stop;
            }
            let Some(command) = viewer.command_from_key(key, state) else {
                return glib::Propagation::Proceed;
            };
            viewer.handle_key_command(command);
            glib::Propagation::Stop
        });
        self.window.add_controller(controller);
    }

    fn command_from_key(&self, key: gdk::Key, state: gdk::ModifierType) -> Option<Command> {
        let key = key_code_from_gdk_key(key)?;
        let input = KeyInput::new(key, key_modifiers_from_gdk(state));
        let context = if self.app.borrow().has_animation() {
            CommandContext::AnimationImage
        } else {
            CommandContext::StaticImage
        };
        command_for_key_input_with_context(input, context)
    }

    fn install_scroll_handler(self: &Rc<Self>) {
        let scroll = gtk::EventControllerScroll::new(
            gtk::EventControllerScrollFlags::VERTICAL | gtk::EventControllerScrollFlags::DISCRETE,
        );
        let weak = Rc::downgrade(self);
        scroll.connect_scroll(move |controller, _dx, dy| {
            let Some(viewer) = weak.upgrade() else {
                return glib::Propagation::Proceed;
            };
            let signed_steps = signed_scroll_steps_from_gtk_delta(dy);
            if signed_steps == 0 {
                return glib::Propagation::Proceed;
            }
            let state = controller.current_event_state();
            let anchor = controller
                .current_event()
                .and_then(|event| event.position());
            viewer.handle_mouse_wheel(signed_steps, state, anchor);
            glib::Propagation::Stop
        });
        self.drawing_area.add_controller(scroll);
    }

    fn install_mouse_handlers(self: &Rc<Self>) {
        let click = gtk::GestureClick::new();
        click.set_button(0);
        let weak = Rc::downgrade(self);
        click.connect_pressed(move |gesture, _n_press, x, y| {
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            match gesture.current_button() {
                1 => viewer.handle_left_button_press(gesture, x, y),
                3 => viewer.show_context_menu(x, y),
                _ => {}
            }
        });
        let weak = Rc::downgrade(self);
        click.connect_released(move |gesture, _n_press, _x, _y| {
            if gesture.current_button() == 1 {
                if let Some(viewer) = weak.upgrade() {
                    viewer.cancel_active_pan();
                }
            }
        });
        self.drawing_area.add_controller(click);

        let drag = gtk::GestureDrag::new();
        drag.set_button(1);
        let weak = Rc::downgrade(self);
        drag.connect_drag_begin(move |gesture, x, y| {
            if let Some(viewer) = weak.upgrade() {
                viewer.handle_image_pan_drag_begin(gesture, x, y);
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_update(move |gesture, offset_x, offset_y| {
            if let Some(viewer) = weak.upgrade() {
                viewer.handle_image_pan_drag_update(gesture, offset_x, offset_y);
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_drag_end(move |_, _offset_x, _offset_y| {
            if let Some(viewer) = weak.upgrade() {
                viewer.cancel_active_pan();
            }
        });
        let weak = Rc::downgrade(self);
        drag.connect_cancel(move |_, _sequence| {
            if let Some(viewer) = weak.upgrade() {
                viewer.cancel_active_pan();
            }
        });
        self.drawing_area.add_controller(drag);

        let motion = gtk::EventControllerMotion::new();
        let weak = Rc::downgrade(self);
        motion.connect_motion(move |_, x, y| {
            if let Some(viewer) = weak.upgrade() {
                viewer.handle_mouse_move(x, y);
            }
        });
        self.drawing_area.add_controller(motion);
    }

    fn install_drop_handlers(self: &Rc<Self>) {
        self.install_direct_file_drop_handlers_on(&self.root);
        self.install_text_file_drop_handler_on(&self.root);
    }

    fn install_direct_file_drop_handlers_on<W: IsA<gtk::Widget>>(self: &Rc<Self>, widget: &W) {
        let drop_file_list =
            gtk::DropTarget::new(gdk::FileList::static_type(), gdk::DragAction::COPY);
        drop_file_list.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        drop_file_list.connect_drop(move |_, value, _, _| {
            let Some(viewer) = weak.upgrade() else {
                return false;
            };
            let Ok(file_list) = value.get::<gdk::FileList>() else {
                return false;
            };
            viewer.handle_dropped_image_path(first_supported_path_from_file_list(&file_list))
        });
        widget.add_controller(drop_file_list);

        let drop_file = gtk::DropTarget::new(gio::File::static_type(), gdk::DragAction::COPY);
        drop_file.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        drop_file.connect_drop(move |_, value, _, _| {
            let Some(viewer) = weak.upgrade() else {
                return false;
            };
            let Ok(file) = value.get::<gio::File>() else {
                return false;
            };
            viewer.handle_dropped_image_path(first_supported_path_from_gio_file(&file))
        });
        widget.add_controller(drop_file);

        let drop_text = gtk::DropTarget::new(String::static_type(), gdk::DragAction::COPY);
        drop_text.set_propagation_phase(gtk::PropagationPhase::Capture);
        let weak = Rc::downgrade(self);
        drop_text.connect_drop(move |_, value, _, _| {
            let Some(viewer) = weak.upgrade() else {
                return false;
            };
            let Ok(text) = value.get::<String>() else {
                return false;
            };
            viewer.handle_dropped_image_path(first_supported_path_from_drop_text(&text))
        });
        widget.add_controller(drop_text);
    }

    fn install_text_file_drop_handler_on<W: IsA<gtk::Widget>>(self: &Rc<Self>, widget: &W) {
        let drop = gtk::DropTargetAsync::new(
            Some(file_drop_text_content_formats()),
            gdk::DragAction::COPY,
        );
        drop.set_propagation_phase(gtk::PropagationPhase::Capture);
        drop.connect_accept(|_, drop| drop_contains_text_file_payload(drop));
        drop.connect_drag_enter(|_, drop, _, _| file_drop_action_for_text_drop(drop));
        drop.connect_drag_motion(|_, drop, _, _| file_drop_action_for_text_drop(drop));
        let weak = Rc::downgrade(self);
        drop.connect_drop(move |_, drop, _, _| {
            let drop = drop.clone();
            let weak = weak.clone();
            glib::MainContext::default().spawn_local(async move {
                let handled = match first_supported_path_from_text_drop(&drop).await {
                    Ok(path) => {
                        let Some(viewer) = weak.upgrade() else {
                            drop.finish(gdk::DragAction::empty());
                            return;
                        };
                        viewer.handle_dropped_image_path(path)
                    }
                    Err(error) => {
                        if let Some(viewer) = weak.upgrade() {
                            eprintln!("j3Pic drop read failed: {error}");
                            viewer.show_error("드롭된 파일 경로를 읽을 수 없습니다.");
                        }
                        false
                    }
                };
                drop.finish(if handled {
                    gdk::DragAction::COPY
                } else {
                    gdk::DragAction::empty()
                });
            });
            true
        });
        widget.add_controller(drop);
    }

    fn install_close_handler(self: &Rc<Self>) {
        let weak = Rc::downgrade(self);
        self.window.connect_close_request(move |_| {
            weak.upgrade()
                .map(|viewer| viewer.handle_close_request())
                .unwrap_or(glib::Propagation::Proceed)
        });
    }

    fn queue_startup_image_open(self: &Rc<Self>) {
        if self.startup_image_path.borrow().is_none() {
            return;
        }
        let weak = Rc::downgrade(self);
        glib::idle_add_local_once(move || {
            if let Some(viewer) = weak.upgrade() {
                if let Some(path) = viewer.startup_image_path.borrow_mut().take() {
                    viewer.load_image_path(path);
                }
            }
        });
    }

    fn handle_close_request(self: &Rc<Self>) -> glib::Propagation {
        if self.shutdown_can_close.get() {
            return glib::Propagation::Proceed;
        }
        if self.shutdown_started.replace(true) {
            return glib::Propagation::Stop;
        }

        match self.shutdown() {
            GtkExportShutdownOutcome::Complete => {
                self.shutdown_can_close.set(true);
                glib::Propagation::Proceed
            }
            GtkExportShutdownOutcome::WaitingForWorker(receiver) => {
                self.ensure_export_shutdown_poll_timer(receiver);
                self.window.set_sensitive(false);
                self.window.set_visible(false);
                glib::Propagation::Stop
            }
        }
    }

    fn shutdown(&self) -> GtkExportShutdownOutcome {
        self.cancel_active_pan();
        self.kill_animation_timer();
        self.kill_decode_poll_timer();
        self.kill_export_poll_timer();
        self.kill_render_settle_timer();
        self.decoder.borrow_mut().shutdown();
        let export_shutdown = self.exporter.borrow_mut().shutdown();
        self.save_window_bounds();
        if self.save_config_on_destroy.get() {
            let config = self.app.borrow().config_snapshot();
            if let Err(error) = save_app_config(&config) {
                eprintln!("j3Pic config save failed: {error}");
            }
        }
        export_shutdown
    }

    fn ensure_export_shutdown_poll_timer(self: &Rc<Self>, receiver: mpsc::Receiver<()>) {
        if self.export_shutdown_poll_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source =
            glib::timeout_add_local(EXPORT_POLL_INTERVAL, move || match receiver.try_recv() {
                Ok(()) | Err(mpsc::TryRecvError::Disconnected) => {
                    if let Some(viewer) = weak.upgrade() {
                        viewer.export_shutdown_poll_timer.borrow_mut().take();
                        viewer.shutdown_can_close.set(true);
                        viewer.window.close();
                    }
                    glib::ControlFlow::Break
                }
                Err(mpsc::TryRecvError::Empty) => glib::ControlFlow::Continue,
            });
        self.export_shutdown_poll_timer.borrow_mut().replace(source);
    }

    fn save_window_bounds(&self) {
        let fullscreen_bounds = if self.window.is_fullscreen() {
            *self.windowed_bounds_before_fullscreen.borrow()
        } else {
            None
        };
        let saved_bounds = saved_window_bounds_for_config(
            self.app.borrow().window_bounds(),
            self.window.width(),
            self.window.height(),
            fullscreen_bounds,
        );
        if let Some(bounds) = saved_bounds {
            self.app.borrow_mut().set_window_bounds(Some(bounds));
        }
    }

    fn handle_key_command(self: &Rc<Self>, command: Command) {
        if matches!(
            command,
            Command::ZoomIn
                | Command::ZoomOut
                | Command::ActualSize
                | Command::FitToWindow
                | Command::RotateClockwise
                | Command::RotateCounterClockwise
        ) {
            self.handle_app_command_with_deferred_render(command);
        } else {
            self.handle_command(command);
        }
    }

    fn handle_command(self: &Rc<Self>, command: Command) {
        match command {
            Command::OpenImage => self.open_image_dialog(),
            Command::ExportImage => self.export_image_dialog(),
            Command::CopyImageToClipboard => self.copy_current_image_to_clipboard(),
            Command::ToggleFullscreen => self.toggle_fullscreen(),
            Command::OpenSettings => self.open_settings_dialog(),
            Command::ExitFullscreenOrQuit => self.exit_fullscreen_or_quit(),
            Command::Quit => self.window.close(),
            Command::Navigate(direction) => self.navigate_image(direction),
            Command::Animation(command) => self.handle_animation_command(command),
            Command::ContextualSpace => self.navigate_image(ImageNavigationDirection::Next),
            Command::ZoomIn
            | Command::ZoomOut
            | Command::ActualSize
            | Command::FitToWindow
            | Command::RotateClockwise
            | Command::RotateCounterClockwise => self.handle_app_command(command),
        }
    }

    fn handle_app_command(self: &Rc<Self>, command: Command) {
        self.handle_app_command_impl(command, false);
    }

    fn handle_app_command_with_deferred_render(self: &Rc<Self>, command: Command) {
        self.handle_app_command_impl(command, true);
    }

    fn handle_app_command_impl(self: &Rc<Self>, command: Command, defer_render: bool) {
        let result = {
            let mut app = self.app.borrow_mut();
            app.handle_command(command)
                .map(|outcome| (outcome, app.title().to_owned()))
        };
        match result {
            Ok((AppCommandOutcome::Changed, title)) => {
                self.window.set_title(Some(&title));
                if defer_render {
                    self.app.borrow_mut().defer_scaling_cache_rebuilds();
                    self.schedule_interactive_render_settle();
                } else {
                    self.cancel_deferred_render_settle();
                }
                self.queue_draw();
                self.start_full_resolution_decode_if_needed();
            }
            Ok((AppCommandOutcome::Unchanged | AppCommandOutcome::Unhandled, _)) => {}
            Err(error) => {
                debug_log_viewer_error("app command failed", &error);
                self.show_error(&error.user_message_for(self.ui_language()));
            }
        }
    }

    fn handle_animation_command(self: &Rc<Self>, command: AnimationCommand) {
        self.kill_animation_timer();
        let outcome = self.app.borrow_mut().handle_animation_command(command);
        self.handle_animation_frame_outcome(outcome);
    }

    fn handle_animation_timer(self: &Rc<Self>) {
        self.kill_animation_timer();
        let outcome = self.app.borrow_mut().handle_animation_timer();
        self.handle_animation_frame_outcome(outcome);
    }

    fn handle_animation_frame_outcome(self: &Rc<Self>, outcome: AnimationFrameOutcome) {
        match outcome {
            AnimationFrameOutcome::Updated => {
                self.queue_draw_after_image_content_change();
                self.update_animation_timer();
            }
            AnimationFrameOutcome::StateChanged => self.update_animation_timer(),
            AnimationFrameOutcome::NeedsDecode(request) => {
                self.start_animation_frame_decode(request)
            }
            AnimationFrameOutcome::Unchanged => self.update_animation_timer(),
        }
    }

    fn handle_mouse_wheel(
        self: &Rc<Self>,
        steps: i32,
        state: gdk::ModifierType,
        anchor: Option<(f64, f64)>,
    ) {
        let modifiers = key_modifiers_from_gdk(state);
        let action = {
            let app = self.app.borrow();
            let interaction = app.config().interaction_settings();
            if mouse_event_matches(interaction.zoom_shortcut(), modifiers) {
                Some(MouseWheelAction::Zoom)
            } else if mouse_event_matches(interaction.image_navigation_shortcut(), modifiers) {
                wheel_navigation_direction_from_steps(steps).map(MouseWheelAction::Navigate)
            } else {
                None
            }
        };

        match action {
            Some(MouseWheelAction::Zoom) => {
                let (x, y) = anchor.unwrap_or_else(|| self.last_pointer_or_viewport_center());
                let changed = {
                    let mut app = self.app.borrow_mut();
                    let Some(factor) = wheel_zoom_factor_from_steps(app.zoom_step_factor(), steps)
                    else {
                        return;
                    };
                    app.zoom_at(
                        factor,
                        ViewportPoint::from_client_position(x as i32, y as i32),
                    )
                };
                if changed {
                    self.app.borrow_mut().defer_scaling_cache_rebuilds();
                    self.schedule_interactive_render_settle();
                    self.queue_draw();
                    self.start_full_resolution_decode_if_needed();
                }
            }
            Some(MouseWheelAction::Navigate(direction)) => self.navigate_image(direction),
            None => {}
        }
    }

    fn handle_left_button_press(self: &Rc<Self>, gesture: &gtk::GestureClick, x: f64, y: f64) {
        self.store_pointer_position(x, y);
        let state = gesture.current_event_state();
        let modifiers = key_modifiers_from_gdk(state);
        let action = {
            let app = self.app.borrow();
            let interaction = app.config().interaction_settings();
            if mouse_event_matches(interaction.image_pan_shortcut(), modifiers) {
                Some(LeftButtonAction::ImagePan)
            } else if mouse_event_matches(interaction.window_move_shortcut(), modifiers) {
                Some(LeftButtonAction::WindowMove)
            } else {
                None
            }
        };

        match action {
            Some(LeftButtonAction::ImagePan) => {}
            Some(LeftButtonAction::WindowMove) => self.start_window_move(gesture, x, y),
            None => {}
        }
    }

    fn start_window_move(&self, gesture: &gtk::GestureClick, x: f64, y: f64) {
        self.cancel_active_pan();
        let Some(surface) = self.window.surface() else {
            return;
        };
        let Ok(toplevel) = surface.downcast::<gdk::Toplevel>() else {
            return;
        };
        let Some(event) = gesture.current_event() else {
            return;
        };
        let Some(device) = event.device() else {
            return;
        };
        toplevel.begin_move(&device, 1, x, y, gesture.current_event_time());
    }

    fn handle_image_pan_drag_begin(self: &Rc<Self>, gesture: &gtk::GestureDrag, x: f64, y: f64) {
        self.store_pointer_position(x, y);
        let modifiers = key_modifiers_from_gdk(gesture.current_event_state());
        let should_pan = {
            let app = self.app.borrow();
            mouse_event_matches(
                app.config().interaction_settings().image_pan_shortcut(),
                modifiers,
            )
        };
        if should_pan {
            let point = ViewportPoint::from_client_position(x as i32, y as i32);
            let _ = self.app.borrow_mut().begin_pan(point);
        }
    }

    fn handle_image_pan_drag_update(
        self: &Rc<Self>,
        gesture: &gtk::GestureDrag,
        offset_x: f64,
        offset_y: f64,
    ) {
        let Some((start_x, start_y)) = gesture.start_point() else {
            return;
        };
        let x = start_x + offset_x;
        let y = start_y + offset_y;
        self.store_pointer_position(x, y);
        self.update_active_pan_to_position(x, y);
    }

    fn handle_mouse_move(self: &Rc<Self>, x: f64, y: f64) {
        self.store_pointer_position(x, y);
    }

    fn update_active_pan_to_position(self: &Rc<Self>, x: f64, y: f64) {
        let point = ViewportPoint::from_client_position(x as i32, y as i32);
        if self.app.borrow_mut().update_pan(point) {
            self.app.borrow_mut().defer_scaling_cache_rebuilds();
            self.schedule_interactive_render_settle();
            self.queue_draw();
        }
    }

    fn store_pointer_position(&self, x: f64, y: f64) {
        if x.is_finite() && y.is_finite() {
            self.last_pointer_position.borrow_mut().replace((x, y));
        }
    }

    fn last_pointer_or_viewport_center(&self) -> (f64, f64) {
        (*self.last_pointer_position.borrow()).unwrap_or_else(|| {
            (
                f64::from(self.drawing_area.width()) / 2.0,
                f64::from(self.drawing_area.height()) / 2.0,
            )
        })
    }

    fn cancel_active_pan(&self) {
        let _ = self.app.borrow_mut().end_pan();
    }

    fn navigate_image(self: &Rc<Self>, direction: ImageNavigationDirection) {
        self.cancel_active_pan();
        self.kill_animation_timer();
        self.cancel_deferred_render_settle();
        let outcome = self
            .app
            .borrow_mut()
            .begin_navigation_or_use_preloaded(direction);
        match outcome {
            NavigationStartOutcome::Decode(request) => self.start_initial_decode(request),
            NavigationStartOutcome::AppliedPreloaded => {
                let title = self.app.borrow().title().to_owned();
                self.window.set_title(Some(&title));
                self.queue_draw_after_image_content_change();
                self.update_animation_timer();
                self.start_full_resolution_decode_if_needed();
                self.start_navigation_preloads_if_possible();
            }
            NavigationStartOutcome::Noop => {}
        }
    }

    fn load_image_path(self: &Rc<Self>, path: PathBuf) {
        self.cancel_active_pan();
        self.kill_animation_timer();
        self.cancel_deferred_render_settle();
        let request = self.app.borrow_mut().begin_image_decode(path);
        self.start_initial_decode(request);
    }

    fn handle_dropped_image_path(self: &Rc<Self>, path: Option<PathBuf>) -> bool {
        match path {
            Some(path) => {
                self.load_image_path(path);
                true
            }
            None => {
                self.show_error(&unsupported_drop_message());
                false
            }
        }
    }

    fn start_initial_decode(self: &Rc<Self>, request: ImageDecodeRequest) {
        let generation = request.generation();
        match self.decoder.borrow_mut().start_initial_decode(request) {
            Ok(()) => self.ensure_decode_poll_timer(),
            Err(error) => self.handle_initial_decode_start_error(generation, error),
        }
    }

    fn start_full_resolution_decode_if_needed(self: &Rc<Self>) {
        let request = self.app.borrow_mut().begin_full_resolution_decode();
        if let Some(request) = request {
            let generation = request.generation();
            let file_version = request.file_version();
            match self
                .decoder
                .borrow_mut()
                .start_full_resolution_decode(request)
            {
                Ok(()) => self.ensure_decode_poll_timer(),
                Err(error) => {
                    self.handle_full_resolution_decode_start_error(generation, file_version, error)
                }
            }
        }
    }

    fn start_animation_frame_decode(self: &Rc<Self>, request: AnimationFrameDecodeRequest) {
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
            Ok(Some(frame)) => {
                self.handle_animation_frame_decode_message(
                    generation,
                    path,
                    Some(file_version),
                    frame_index,
                    Ok(frame),
                );
                return;
            }
            Ok(None) => {}
            Err(error) => {
                self.handle_animation_frame_decode_message(
                    generation,
                    path,
                    Some(file_version),
                    frame_index,
                    Err(error),
                );
                return;
            }
        }
        match self
            .decoder
            .borrow_mut()
            .start_animation_frame_decode(request)
        {
            Ok(()) => self.ensure_decode_poll_timer(),
            Err(error) => self.handle_animation_frame_decode_start_error(
                generation,
                path,
                file_version,
                frame_index,
                error,
            ),
        }
    }

    fn start_navigation_preloads_if_possible(self: &Rc<Self>) {
        let requests = self.app.borrow().navigation_preload_requests();
        if requests.is_empty() {
            return;
        }
        self.decoder
            .borrow_mut()
            .start_navigation_preloads(requests);
        self.ensure_decode_poll_timer();
    }

    fn ensure_decode_poll_timer(self: &Rc<Self>) {
        if self.decode_poll_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(DECODE_POLL_INTERVAL, move || {
            let Some(viewer) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let keep = viewer.drain_decode_messages();
            if keep {
                glib::ControlFlow::Continue
            } else {
                viewer.decode_poll_timer.borrow_mut().take();
                glib::ControlFlow::Break
            }
        });
        self.decode_poll_timer.borrow_mut().replace(source);
    }

    fn drain_decode_messages(self: &Rc<Self>) -> bool {
        let drain = self.decoder.borrow_mut().drain_messages();
        for message in drain.messages {
            match message {
                DecodeWorkerMessage::Initial { generation, result } => {
                    self.handle_initial_decode_message(generation, result);
                }
                DecodeWorkerMessage::InitialDecodeCompleted { .. } => {}
                DecodeWorkerMessage::FolderScanned {
                    generation,
                    path,
                    result,
                } => self.handle_folder_scan_message(generation, path, result),
                DecodeWorkerMessage::FolderScanSkipped { generation, path } => {
                    self.handle_folder_scan_skipped_message(generation, path);
                }
                DecodeWorkerMessage::FullResolution {
                    generation,
                    file_version,
                    result,
                } => self.handle_full_resolution_decode_message(generation, file_version, result),
                DecodeWorkerMessage::AnimationFrame {
                    generation,
                    path,
                    file_version,
                    frame_index,
                    result,
                } => self.handle_animation_frame_decode_message(
                    generation,
                    path,
                    file_version,
                    frame_index,
                    result,
                ),
                DecodeWorkerMessage::NavigationPreload { request, result } => {
                    if let Ok(image) = result {
                        let _ = self
                            .app
                            .borrow_mut()
                            .store_preloaded_navigation_image(&request, image);
                    }
                }
            }
        }
        for failure in drain.start_failures {
            match failure {
                DecodeStartFailure::Initial { generation, error } => {
                    self.handle_initial_decode_start_error(generation, error)
                }
                DecodeStartFailure::FullResolution {
                    generation,
                    file_version,
                    error,
                } => {
                    self.handle_full_resolution_decode_start_error(generation, file_version, error)
                }
                DecodeStartFailure::AnimationFrame {
                    generation,
                    path,
                    file_version,
                    frame_index,
                    error,
                } => self.handle_animation_frame_decode_start_error(
                    generation,
                    path,
                    file_version,
                    frame_index,
                    error,
                ),
            }
        }
        self.decoder.borrow_mut().has_live_work()
    }

    fn handle_initial_decode_start_error(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        error: ViewerAppError,
    ) {
        let presentation = self
            .app
            .borrow_mut()
            .finish_failed_initial_decode(generation, &error);
        self.present_decode_failure(presentation, error);
    }

    fn handle_initial_decode_message(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        result: Result<(LoadedImage, ImageFolder), ViewerAppError>,
    ) {
        match result {
            Ok((image, folder)) => {
                let outcome = self
                    .app
                    .borrow_mut()
                    .apply_decoded_image(generation, image, folder);
                if outcome == DecodeApplyOutcome::Applied {
                    let title = self.app.borrow().title().to_owned();
                    self.window.set_title(Some(&title));
                    self.queue_draw_after_image_content_change();
                    self.update_animation_timer();
                    self.start_full_resolution_decode_if_needed();
                    self.start_navigation_preloads_if_possible();
                }
            }
            Err(error) => {
                let presentation = self
                    .app
                    .borrow_mut()
                    .finish_failed_initial_decode(generation, &error);
                self.present_decode_failure(presentation, error);
            }
        }
    }

    fn present_decode_failure(
        self: &Rc<Self>,
        presentation: DecodeFailurePresentation,
        error: ViewerAppError,
    ) {
        match presentation {
            DecodeFailurePresentation::MessageBox => {
                debug_log_viewer_error("image decode failed", &error);
                self.show_error(&error.user_message_for(self.ui_language()));
                self.update_animation_timer();
            }
            DecodeFailurePresentation::StatusMessage => {
                debug_log_viewer_error("navigation decode failed", &error);
                self.queue_draw();
                self.update_animation_timer();
            }
            DecodeFailurePresentation::RetryNavigation(request) => {
                self.start_initial_decode(request)
            }
            DecodeFailurePresentation::Stale => {}
        }
    }

    fn handle_folder_scan_message(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        result: Result<ImageFolder, ScanImageFolderError>,
    ) {
        match result {
            Ok(folder) => {
                let pending = {
                    let mut app = self.app.borrow_mut();
                    let outcome = app.apply_scanned_image_folder(generation, &path, folder);
                    if outcome == DecodeApplyOutcome::Applied {
                        app.take_pending_navigation_after_folder_scan()
                    } else {
                        None
                    }
                };
                if let Some(request) = pending {
                    self.cancel_deferred_render_settle();
                    self.start_initial_decode(request);
                } else {
                    self.start_navigation_preloads_if_possible();
                }
            }
            Err(error) => {
                let outcome = self
                    .app
                    .borrow_mut()
                    .finish_pending_folder_scan_without_update(generation, &path);
                if outcome == DecodeApplyOutcome::Applied {
                    self.start_navigation_preloads_if_possible();
                }
                eprintln!("j3Pic folder scan failed: {error}");
            }
        }
    }

    fn handle_folder_scan_skipped_message(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
    ) {
        let outcome = self
            .app
            .borrow_mut()
            .finish_pending_folder_scan_without_update(generation, &path);
        if outcome == DecodeApplyOutcome::Applied {
            self.start_navigation_preloads_if_possible();
        }
    }

    fn handle_full_resolution_decode_start_error(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        file_version: Option<ImageFileVersion>,
        error: ViewerAppError,
    ) {
        let outcome = self
            .app
            .borrow_mut()
            .finish_failed_decode(generation, file_version);
        if outcome == DecodeApplyOutcome::Applied {
            debug_log_viewer_error("full-resolution decode worker start failed", &error);
            self.show_error(&error.user_message_for(self.ui_language()));
        }
    }

    fn handle_full_resolution_decode_message(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        file_version: Option<ImageFileVersion>,
        result: Result<PixelImage, LoadImageError>,
    ) {
        match result {
            Ok(pixels) => {
                let outcome = self.app.borrow_mut().apply_full_resolution_image(
                    generation,
                    file_version,
                    pixels,
                );
                if outcome == DecodeApplyOutcome::Applied {
                    let title = self.app.borrow().title().to_owned();
                    self.window.set_title(Some(&title));
                    self.queue_draw_after_image_content_change();
                    self.start_navigation_preloads_if_possible();
                }
            }
            Err(error) => {
                let outcome = self
                    .app
                    .borrow_mut()
                    .finish_failed_decode(generation, file_version);
                if outcome == DecodeApplyOutcome::Applied && !error.is_canceled() {
                    let error = ViewerAppError::LoadImage(error);
                    debug_log_viewer_error("full-resolution decode failed", &error);
                    self.show_error(&error.user_message_for(self.ui_language()));
                }
                if outcome == DecodeApplyOutcome::Applied {
                    self.start_navigation_preloads_if_possible();
                }
            }
        }
    }

    fn handle_animation_frame_decode_start_error(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        file_version: ImageFileVersion,
        frame_index: usize,
        error: ViewerAppError,
    ) {
        let outcome = self.app.borrow_mut().finish_failed_animation_frame_decode(
            generation,
            frame_index,
            &path,
            Some(file_version),
        );
        if outcome == DecodeApplyOutcome::Applied {
            debug_log_viewer_error("animation frame decode worker start failed", &error);
            self.show_error(&error.user_message_for(self.ui_language()));
        }
        self.update_animation_timer();
    }

    fn handle_animation_frame_decode_message(
        self: &Rc<Self>,
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        file_version: Option<ImageFileVersion>,
        frame_index: usize,
        result: Result<AnimationFramePixels, LoadImageError>,
    ) {
        match result {
            Ok(frame) => {
                let outcome = self.app.borrow_mut().apply_animation_frame_pixels(
                    generation,
                    frame_index,
                    &path,
                    file_version,
                    frame,
                );
                if outcome == DecodeApplyOutcome::Applied {
                    self.queue_draw_after_image_content_change();
                }
                self.update_animation_timer();
            }
            Err(error) => {
                let outcome = self.app.borrow_mut().finish_failed_animation_frame_decode(
                    generation,
                    frame_index,
                    &path,
                    file_version,
                );
                if outcome == DecodeApplyOutcome::Applied && !error.is_canceled() {
                    let error = ViewerAppError::LoadImage(error);
                    debug_log_viewer_error("animation frame decode failed", &error);
                    self.show_error(&error.user_message_for(self.ui_language()));
                }
                self.update_animation_timer();
            }
        }
    }

    fn update_animation_timer(self: &Rc<Self>) {
        self.kill_animation_timer();
        let Some(interval) = self.app.borrow().animation_timer_interval_ms() else {
            return;
        };
        let weak = Rc::downgrade(self);
        let source =
            glib::timeout_add_local(Duration::from_millis(u64::from(interval)), move || {
                let Some(viewer) = weak.upgrade() else {
                    return glib::ControlFlow::Break;
                };
                viewer.handle_animation_timer();
                glib::ControlFlow::Break
            });
        self.animation_timer.borrow_mut().replace(source);
    }

    fn kill_animation_timer(&self) {
        if let Some(source) = self.animation_timer.borrow_mut().take() {
            source.remove();
        }
    }

    fn kill_decode_poll_timer(&self) {
        if let Some(source) = self.decode_poll_timer.borrow_mut().take() {
            source.remove();
        }
    }

    fn kill_export_poll_timer(&self) {
        if let Some(source) = self.export_poll_timer.borrow_mut().take() {
            source.remove();
        }
    }

    fn schedule_interactive_render_settle(self: &Rc<Self>) {
        self.kill_render_settle_timer();
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(INTERACTIVE_RENDER_SETTLE_INTERVAL, move || {
            let Some(viewer) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            viewer.render_settle_timer.borrow_mut().take();
            if viewer.app.borrow_mut().resume_scaling_cache_rebuilds() {
                viewer.queue_draw();
            }
            glib::ControlFlow::Break
        });
        self.render_settle_timer.borrow_mut().replace(source);
    }

    fn kill_render_settle_timer(&self) {
        if let Some(source) = self.render_settle_timer.borrow_mut().take() {
            source.remove();
        }
    }

    fn cancel_deferred_render_settle(&self) {
        self.kill_render_settle_timer();
        self.app
            .borrow_mut()
            .cancel_deferred_scaling_cache_rebuild();
    }

    fn queue_draw(&self) {
        self.drawing_area.queue_draw();
        self.update_status_bar();
    }

    fn queue_draw_after_image_content_change(&self) {
        self.cancel_deferred_render_settle();
        self.paint_cache.borrow_mut().invalidate();
        self.queue_draw();
    }

    fn update_status_bar(&self) {
        let app = self.app.borrow();
        self.update_status_bar_from_app(&app);
    }

    fn update_status_bar_from_app(&self, app: &ViewerApp) {
        if let Some(text) = app.image_info_text() {
            self.status_label.set_text(&text);
            self.status_label.set_visible(true);
        } else {
            self.status_label.set_text("");
            self.status_label.set_visible(false);
        }
    }

    fn show_context_menu(self: &Rc<Self>, x: f64, y: f64) {
        let popover = gtk::Popover::new();
        popover.set_has_arrow(false);
        popover.set_parent(&self.drawing_area);
        let drawing_area = self.drawing_area.clone();
        popover.connect_closed(move |popover| {
            if popover.parent().is_some() {
                popover.unparent();
            }
            let _ = drawing_area.grab_focus();
        });
        let rect = gdk::Rectangle::new(x as i32, y as i32, 1, 1);
        popover.set_pointing_to(Some(&rect));
        let list = gtk::Box::new(gtk::Orientation::Vertical, 0);
        let has_image = self.app.borrow().image_state().has_image();
        let language = self.ui_language();
        for &entry in context_menu_entries() {
            match entry {
                ContextMenuEntry::Separator => {
                    let separator = gtk::Separator::new(gtk::Orientation::Horizontal);
                    list.append(&separator);
                }
                ContextMenuEntry::Command {
                    command,
                    requires_image,
                } => {
                    let button =
                        gtk::Button::with_label(ui_text::context_menu_label(language, command));
                    button.set_halign(gtk::Align::Fill);
                    button.set_sensitive(!requires_image || has_image);
                    let weak = Rc::downgrade(self);
                    let popover = popover.downgrade();
                    button.connect_clicked(move |_| {
                        if let Some(popover) = popover.upgrade() {
                            popover.popdown();
                        }
                        if let Some(viewer) = weak.upgrade() {
                            viewer.handle_command(command);
                        }
                    });
                    list.append(&button);
                }
            }
        }
        popover.set_child(Some(&list));
        popover.popup();
    }

    fn ui_language(&self) -> UiLanguage {
        self.app.borrow().config().ui_language()
    }

    fn show_keyboard_context_menu(self: &Rc<Self>) {
        let width = self.drawing_area.width().max(1);
        let height = self.drawing_area.height().max(1);
        self.show_context_menu(f64::from(width / 2), f64::from(height / 2));
    }

    fn open_image_dialog(self: &Rc<Self>) {
        let dialog = gtk::FileDialog::new();
        dialog.set_title("Open Image");
        dialog.set_modal(true);
        dialog.set_accept_label(Some("열기"));
        dialog.set_filters(Some(&open_file_filters()));
        if let Some(folder) = self.app.borrow().recent_folder() {
            dialog.set_initial_folder(Some(&gio::File::for_path(folder)));
        }
        let weak = Rc::downgrade(self);
        let window = self.window.clone();
        glib::MainContext::default().spawn_local(async move {
            let result = dialog.open_future(Some(&window)).await;
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            match result {
                Ok(file) => {
                    if let Some(path) = file.path() {
                        viewer.load_image_path(path);
                    } else {
                        viewer.show_error("선택한 파일 경로를 읽을 수 없습니다.");
                    }
                }
                Err(error) if gtk_file_dialog_error_is_cancelled(&error) => {}
                Err(error) => {
                    debug_log_gtk_dialog_error("open file dialog failed", &error);
                    viewer.show_error("파일 열기 대화상자를 열 수 없습니다.");
                }
            }
        });
    }

    fn export_image_dialog(self: &Rc<Self>) {
        let Some(defaults) = self.current_export_defaults() else {
            self.show_error("내보낼 이미지가 없습니다.");
            return;
        };
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            let language = viewer.ui_language();
            let Some(options) =
                show_export_options_dialog(&viewer.window, defaults.clone(), language).await
            else {
                return;
            };
            let suggested =
                export_path_with_format_extension(&defaults.suggested_path, options.format());
            let dialog = gtk::FileDialog::new();
            dialog.set_title("Export Image");
            dialog.set_modal(true);
            dialog.set_accept_label(Some(match language {
                UiLanguage::English => "Save",
                UiLanguage::Korean => "저장",
            }));
            dialog.set_filters(Some(&export_file_filters(options.format())));
            if let Some(parent) = suggested
                .parent()
                .filter(|path| !path.as_os_str().is_empty())
            {
                dialog.set_initial_folder(Some(&gio::File::for_path(parent)));
            }
            if let Some(name) = gtk_initial_file_name(&suggested) {
                dialog.set_initial_name(Some(&name));
            }
            let file = match dialog.save_future(Some(&viewer.window)).await {
                Ok(file) => file,
                Err(error) if gtk_file_dialog_error_is_cancelled(&error) => return,
                Err(error) => {
                    debug_log_gtk_dialog_error("save file dialog failed", &error);
                    viewer.show_error("파일 저장 대화상자를 열 수 없습니다.");
                    return;
                }
            };
            let Some(path) = file.path() else {
                viewer.show_error("저장할 파일 경로를 읽을 수 없습니다.");
                return;
            };
            let selection = ExportFileSelection::from_selected_path(path, options.format());
            if paths_refer_to_same_existing_file(&defaults.source_path, selection.path()) {
                viewer.show_error(same_source_export_message(language));
                return;
            }
            if selection.selected_path().exists()
                && !confirm_overwrite_export_path(
                    &viewer.window,
                    selection.selected_path(),
                    language,
                )
                .await
            {
                return;
            }
            if corrected_export_path_requires_overwrite_confirmation(
                selection.selected_path(),
                selection.path(),
            ) && !confirm_overwrite_corrected_export_path(
                &viewer.window,
                selection.path(),
                language,
            )
            .await
            {
                return;
            }
            viewer.start_image_export(selection.path().to_path_buf(), options);
        });
    }

    fn current_export_defaults(&self) -> Option<ExportDialogDefaults> {
        let app = self.app.borrow();
        let source_path = app.current_source_path()?.to_path_buf();
        let source_format = app.current_source_format()?;
        let source_size = app.current_export_source_size()?;
        let format = app.default_export_format_for_source(source_format);
        let suggested_path = app.suggested_export_path(&source_path, source_format);
        Some(ExportDialogDefaults {
            source_path,
            source_size,
            format,
            quality: app.export_default_quality(),
            suggested_path,
        })
    }

    fn start_image_export(self: &Rc<Self>, path: PathBuf, options: ExportOptions) {
        let quality = options.quality();
        if self.exporter.borrow_mut().is_busy() {
            self.show_error(
                "이미지 내보내기가 아직 진행 중입니다.\n\n완료된 뒤 다시 시도해 주세요.",
            );
            return;
        }
        let request = match self
            .app
            .borrow_mut()
            .begin_current_image_export(&path, options)
        {
            Ok(request) => request,
            Err(error) => {
                debug_log_viewer_error("image export request failed", &error);
                self.show_error(&error.user_message_for(self.ui_language()));
                return;
            }
        };
        match self.exporter.borrow_mut().start_export(request, quality) {
            Ok(()) => self.ensure_export_poll_timer(),
            Err(error) => {
                debug_log_viewer_error("image export worker start failed", &error);
                self.show_error(&error.user_message_for(self.ui_language()));
            }
        }
    }

    fn ensure_export_poll_timer(self: &Rc<Self>) {
        if self.export_poll_timer.borrow().is_some() {
            return;
        }
        let weak = Rc::downgrade(self);
        let source = glib::timeout_add_local(EXPORT_POLL_INTERVAL, move || {
            let Some(viewer) = weak.upgrade() else {
                return glib::ControlFlow::Break;
            };
            let keep = viewer.drain_export_messages();
            if keep {
                glib::ControlFlow::Continue
            } else {
                viewer.export_poll_timer.borrow_mut().take();
                glib::ControlFlow::Break
            }
        });
        self.export_poll_timer.borrow_mut().replace(source);
    }

    fn drain_export_messages(&self) -> bool {
        for message in self.exporter.borrow_mut().drain_messages() {
            match message {
                ExportWorkerMessage::Completed {
                    path,
                    options,
                    quality,
                    result,
                } => match result {
                    Ok(()) => {
                        let mut app = self.app.borrow_mut();
                        app.finish_current_image_export(&path, options);
                        if let Some(quality) = quality {
                            app.set_export_default_quality(quality);
                        }
                        drop(app);
                        self.queue_draw();
                    }
                    Err(error) => {
                        debug_log_viewer_error("image export failed", &error);
                        self.show_error(&error.user_message_for(self.ui_language()));
                    }
                },
            }
        }
        self.exporter.borrow_mut().has_live_work()
    }

    fn copy_current_image_to_clipboard(&self) {
        let texture = {
            let mut app = self.app.borrow_mut();
            let has_image = app.image_state().has_image();
            let Some(pixels) = app.display_pixels() else {
                if has_image {
                    self.show_error(
                        "표시 중인 이미지 데이터를 클립보드용으로 준비하지 못했습니다.",
                    );
                }
                return;
            };
            let Some(texture) = texture_for_pixels(pixels) else {
                self.show_error(
                    "이미지 픽셀 데이터가 올바르지 않아 클립보드에 복사하지 못했습니다.",
                );
                return;
            };
            texture
        };
        self.drawing_area.clipboard().set(&texture);
    }

    fn toggle_fullscreen(&self) {
        self.cancel_active_pan();
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
            self.windowed_bounds_before_fullscreen.borrow_mut().take();
        } else {
            if let Some(bounds) = self.current_window_bounds_for_config() {
                self.windowed_bounds_before_fullscreen
                    .borrow_mut()
                    .replace(bounds);
            }
            self.window.fullscreen();
        }
    }

    fn exit_fullscreen_or_quit(&self) {
        self.cancel_active_pan();
        if self.window.is_fullscreen() {
            self.window.unfullscreen();
            self.windowed_bounds_before_fullscreen.borrow_mut().take();
        } else {
            self.window.close();
        }
    }

    fn current_window_bounds_for_config(&self) -> Option<WindowBounds> {
        window_bounds_for_config(
            self.app.borrow().window_bounds(),
            self.window.width(),
            self.window.height(),
        )
    }

    fn open_settings_dialog(self: &Rc<Self>) {
        let base = self.app.borrow().config_snapshot();
        let weak = Rc::downgrade(self);
        glib::MainContext::default().spawn_local(async move {
            let Some(viewer) = weak.upgrade() else {
                return;
            };
            let Some(config) = show_settings_dialog(&viewer.window, base).await else {
                return;
            };
            viewer.apply_settings_config(config);
        });
    }

    fn apply_settings_config(self: &Rc<Self>, config: AppConfig) {
        let changed = self.app.borrow_mut().apply_config(config.clone());
        match save_app_config(&config) {
            Ok(()) => self.save_config_on_destroy.set(true),
            Err(error) => {
                eprintln!("j3Pic settings save failed: {error}");
                self.show_error(
                    "설정을 저장하지 못했습니다. 현재 실행 중인 창에는 적용되었습니다.",
                );
            }
        }
        if changed {
            self.cancel_deferred_render_settle();
            self.queue_draw();
            self.update_animation_timer();
            self.start_full_resolution_decode_if_needed();
        }
    }

    fn show_error(&self, message: &str) {
        let dialog = gtk::MessageDialog::builder()
            .transient_for(&self.window)
            .modal(true)
            .message_type(gtk::MessageType::Error)
            .buttons(gtk::ButtonsType::Ok)
            .text(message)
            .build();
        glib::MainContext::default().spawn_local(async move {
            let _ = dialog.run_future().await;
            dialog.close();
        });
    }
}

fn saved_window_bounds_for_config(
    existing: Option<WindowBounds>,
    current_width: i32,
    current_height: i32,
    fullscreen_windowed_bounds: Option<WindowBounds>,
) -> Option<WindowBounds> {
    let (width, height) = fullscreen_windowed_bounds
        .map(|bounds| (bounds.width(), bounds.height()))
        .unwrap_or((current_width, current_height));
    window_bounds_for_config(existing, width, height)
}

fn window_bounds_for_config(
    existing: Option<WindowBounds>,
    width: i32,
    height: i32,
) -> Option<WindowBounds> {
    let x = existing.map(WindowBounds::x).unwrap_or(0);
    let y = existing.map(WindowBounds::y).unwrap_or(0);
    WindowBounds::new(x, y, width, height)
}

fn debug_log_viewer_error(context: &str, error: &ViewerAppError) {
    eprintln!(
        "[j3Pic] {context}; category={}; user_message={:?}; internal={}; source={}",
        viewer_error_category(error),
        error.brief_user_message(),
        error,
        error_source_text(error)
    );
}

fn viewer_error_category(error: &ViewerAppError) -> &'static str {
    match error {
        ViewerAppError::LoadImage(_) => "LoadImage",
        ViewerAppError::ScanImageFolder(_) => "ScanImageFolder",
        ViewerAppError::ExportImage(_) => "ExportImage",
        ViewerAppError::DecodeWorkerStart { .. } => "DecodeWorkerStart",
        ViewerAppError::ExportWorkerStart { .. } => "ExportWorkerStart",
        ViewerAppError::NoImageToExport => "NoImageToExport",
    }
}

fn debug_log_gtk_dialog_error(context: &str, error: &glib::Error) {
    eprintln!("[j3Pic] {context}; internal={error}");
}

fn gtk_file_dialog_error_is_cancelled(error: &glib::Error) -> bool {
    error.matches(gtk::DialogError::Cancelled)
        || error.matches(gtk::DialogError::Dismissed)
        || error.matches(gio::IOErrorEnum::Cancelled)
}

fn error_source_text(error: &(dyn Error + 'static)) -> String {
    let mut sources = Vec::new();
    let mut current = error.source();
    while let Some(source) = current {
        sources.push(source.to_string());
        current = source.source();
    }
    if sources.is_empty() {
        "none".to_owned()
    } else {
        sources.join(" -> ")
    }
}

#[derive(Clone, Copy)]
enum MouseWheelAction {
    Zoom,
    Navigate(ImageNavigationDirection),
}

fn signed_scroll_steps_from_gtk_delta(dy: f64) -> i32 {
    let steps = dy.abs().round().max(1.0).min(f64::from(i32::MAX)) as i32;
    if dy < 0.0 {
        steps
    } else if dy > 0.0 {
        -steps
    } else {
        0
    }
}

fn wheel_zoom_factor_from_steps(zoom_step_factor: f64, steps: i32) -> Option<f64> {
    if steps == 0 || !zoom_step_factor.is_finite() || zoom_step_factor <= 0.0 {
        return None;
    }
    Some(zoom_step_factor.powi(steps))
}

fn wheel_navigation_direction_from_steps(steps: i32) -> Option<ImageNavigationDirection> {
    if steps > 0 {
        Some(ImageNavigationDirection::Previous)
    } else if steps < 0 {
        Some(ImageNavigationDirection::Next)
    } else {
        None
    }
}

#[derive(Clone, Copy)]
enum LeftButtonAction {
    ImagePan,
    WindowMove,
}

#[derive(Clone, Copy)]
enum ContextMenuEntry {
    Command {
        command: Command,
        requires_image: bool,
    },
    Separator,
}

fn context_menu_entries() -> &'static [ContextMenuEntry] {
    &[
        ContextMenuEntry::Command {
            command: Command::OpenImage,
            requires_image: false,
        },
        ContextMenuEntry::Command {
            command: Command::ExportImage,
            requires_image: true,
        },
        ContextMenuEntry::Command {
            command: Command::CopyImageToClipboard,
            requires_image: true,
        },
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command {
            command: Command::ActualSize,
            requires_image: true,
        },
        ContextMenuEntry::Command {
            command: Command::FitToWindow,
            requires_image: true,
        },
        ContextMenuEntry::Command {
            command: Command::RotateClockwise,
            requires_image: true,
        },
        ContextMenuEntry::Command {
            command: Command::RotateCounterClockwise,
            requires_image: true,
        },
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command {
            command: Command::ToggleFullscreen,
            requires_image: false,
        },
        ContextMenuEntry::Separator,
        ContextMenuEntry::Command {
            command: Command::OpenSettings,
            requires_image: false,
        },
    ]
}

fn key_code_from_gdk_key(key: gdk::Key) -> Option<KeyCode> {
    match key {
        gdk::Key::c | gdk::Key::C => Some(KeyCode::C),
        gdk::Key::o | gdk::Key::O => Some(KeyCode::O),
        gdk::Key::p | gdk::Key::P => Some(KeyCode::P),
        gdk::Key::q | gdk::Key::Q => Some(KeyCode::Q),
        gdk::Key::r | gdk::Key::R => Some(KeyCode::R),
        gdk::Key::s | gdk::Key::S => Some(KeyCode::S),
        gdk::Key::_0 | gdk::Key::parenright => Some(KeyCode::Digit0),
        gdk::Key::_1 | gdk::Key::exclam => Some(KeyCode::Digit1),
        gdk::Key::equal | gdk::Key::plus => Some(KeyCode::Equals),
        gdk::Key::minus | gdk::Key::underscore => Some(KeyCode::Minus),
        gdk::Key::KP_Add => Some(KeyCode::NumpadAdd),
        gdk::Key::KP_Subtract => Some(KeyCode::NumpadSubtract),
        gdk::Key::Left => Some(KeyCode::Left),
        gdk::Key::Right => Some(KeyCode::Right),
        gdk::Key::space => Some(KeyCode::Space),
        gdk::Key::BackSpace => Some(KeyCode::Backspace),
        gdk::Key::Page_Up => Some(KeyCode::PageUp),
        gdk::Key::Page_Down => Some(KeyCode::PageDown),
        gdk::Key::Home => Some(KeyCode::Home),
        gdk::Key::bracketleft | gdk::Key::braceleft => Some(KeyCode::BracketLeft),
        gdk::Key::bracketright | gdk::Key::braceright => Some(KeyCode::BracketRight),
        gdk::Key::F4 => Some(KeyCode::F4),
        gdk::Key::F11 => Some(KeyCode::F11),
        gdk::Key::Return | gdk::Key::KP_Enter => Some(KeyCode::Enter),
        gdk::Key::Escape => Some(KeyCode::Escape),
        _ => None,
    }
}

fn key_modifiers_from_gdk(state: gdk::ModifierType) -> KeyModifiers {
    KeyModifiers::new(
        state.contains(gdk::ModifierType::CONTROL_MASK),
        state.contains(gdk::ModifierType::SHIFT_MASK),
        state.contains(gdk::ModifierType::ALT_MASK),
    )
}

fn context_menu_key_from_gdk(key: gdk::Key, state: gdk::ModifierType) -> bool {
    let modifiers = key_modifiers_from_gdk(state);
    match key {
        gdk::Key::Menu => !modifiers.control() && !modifiers.alt(),
        gdk::Key::F10 => modifiers == KeyModifiers::new(false, true, false),
        _ => false,
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

enum DecodeWorkerMessage {
    Initial {
        generation: crate::domain::DecodeGeneration,
        result: Result<(LoadedImage, ImageFolder), ViewerAppError>,
    },
    InitialDecodeCompleted {
        generation: crate::domain::DecodeGeneration,
    },
    FolderScanned {
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        result: Result<ImageFolder, ScanImageFolderError>,
    },
    FolderScanSkipped {
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
    },
    FullResolution {
        generation: crate::domain::DecodeGeneration,
        file_version: Option<ImageFileVersion>,
        result: Result<PixelImage, LoadImageError>,
    },
    AnimationFrame {
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        file_version: Option<ImageFileVersion>,
        frame_index: usize,
        result: Result<AnimationFramePixels, LoadImageError>,
    },
    NavigationPreload {
        request: ImagePreloadRequest,
        result: Result<LoadedImage, LoadImageError>,
    },
}

#[derive(Debug)]
enum DecodeStartFailure {
    Initial {
        generation: crate::domain::DecodeGeneration,
        error: ViewerAppError,
    },
    FullResolution {
        generation: crate::domain::DecodeGeneration,
        file_version: Option<ImageFileVersion>,
        error: ViewerAppError,
    },
    AnimationFrame {
        generation: crate::domain::DecodeGeneration,
        path: PathBuf,
        file_version: ImageFileVersion,
        frame_index: usize,
        error: ViewerAppError,
    },
}

struct DecodeControllerDrain {
    messages: Vec<DecodeWorkerMessage>,
    start_failures: Vec<DecodeStartFailure>,
}

struct GtkDecodeController {
    sender: mpsc::Sender<DecodeWorkerMessage>,
    receiver: mpsc::Receiver<DecodeWorkerMessage>,
    folder_scan_worker_count: Arc<AtomicUsize>,
    active_worker: Option<DecodeWorker>,
    retired_workers: Vec<DecodeWorker>,
    folder_scan_workers: Vec<DecodeWorker>,
    navigation_preload_workers: Vec<NavigationPreloadWorker>,
    pending_decode: Option<PendingDecodeRequest>,
}

impl GtkDecodeController {
    fn new() -> Self {
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

    fn start_initial_decode(&mut self, request: ImageDecodeRequest) -> Result<(), ViewerAppError> {
        self.join_finished_workers();
        self.start_decode_or_queue(PendingDecodeRequest::Initial { request })
    }

    fn start_full_resolution_decode(
        &mut self,
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
        self.start_decode_or_queue(PendingDecodeRequest::FullResolution { request })
    }

    fn start_animation_frame_decode(
        &mut self,
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
                self.queue_pending_animation_frame_decode_without_cancel(request);
                return Ok(());
            }
            None => {}
        }
        self.start_decode_or_queue(PendingDecodeRequest::AnimationFrame { request })
    }

    fn start_navigation_preloads(&mut self, requests: Vec<ImagePreloadRequest>) {
        self.join_finished_workers();
        self.cancel_obsolete_navigation_preloads(&requests);
        for request in requests {
            if self
                .navigation_preload_workers
                .iter()
                .any(|worker| worker.request == request && !worker.cancel.load(Ordering::Acquire))
            {
                continue;
            }
            if self.navigation_preload_workers.len() >= MAX_NAVIGATION_PRELOAD_WORKERS {
                break;
            }
            match spawn_navigation_preload_worker(self.sender.clone(), request) {
                Ok(worker) => self.navigation_preload_workers.push(worker),
                Err(error) => eprintln!("j3Pic navigation preload worker start failed: {error}"),
            }
        }
    }

    fn drain_messages(&mut self) -> DecodeControllerDrain {
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

    fn has_live_work(&self) -> bool {
        self.active_worker.is_some()
            || !self.retired_workers.is_empty()
            || !self.folder_scan_workers.is_empty()
            || !self.navigation_preload_workers.is_empty()
            || self.pending_decode.is_some()
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
                .map_err(|failure| decode_start_failure_into_error(*failure))?;
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
        if crate::infra::animation_frame_prefetch_for_loaded_image_covers(
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
        request: AnimationFrameDecodeRequest,
    ) {
        self.pending_decode = Some(PendingDecodeRequest::AnimationFrame { request });
        self.cancel_folder_scan_workers();
        self.cancel_navigation_preload_workers();
    }

    fn clear_pending_animation_frame_decode(
        &mut self,
        generation: crate::domain::DecodeGeneration,
        path: &Path,
        file_version: Option<ImageFileVersion>,
        frame_index: usize,
    ) {
        let should_clear = self.pending_decode.as_ref().is_some_and(|pending| {
            matches!(
                pending,
                PendingDecodeRequest::AnimationFrame { request }
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
            Err(failure) => Some(*failure),
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
        self.active_worker.is_none() && self.inactive_worker_count() < MAX_IN_FLIGHT_DECODE_WORKERS
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

    fn release_initial_worker_for_folder_scan(
        &mut self,
        generation: crate::domain::DecodeGeneration,
    ) {
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

    fn shutdown(&mut self) {
        self.pending_decode = None;
        let mut decode_workers = Vec::new();
        if let Some(worker) = self.active_worker.take() {
            decode_workers.push(worker);
        }
        decode_workers.append(&mut self.retired_workers);
        decode_workers.append(&mut self.folder_scan_workers);
        let navigation_preload_workers = std::mem::take(&mut self.navigation_preload_workers);

        for worker in &decode_workers {
            worker.cancel.store(true, Ordering::Release);
        }
        for worker in &navigation_preload_workers {
            worker.cancel.store(true, Ordering::Release);
        }
        spawn_decode_shutdown_joiner(decode_workers, navigation_preload_workers);
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
        join_finished_worker_list(&mut self.retired_workers);
        join_finished_worker_list(&mut self.folder_scan_workers);
        join_finished_navigation_preload_workers(&mut self.navigation_preload_workers);
    }
}

enum PendingDecodeRequest {
    Initial {
        request: ImageDecodeRequest,
    },
    FullResolution {
        request: ImageDecodeRequest,
    },
    AnimationFrame {
        request: AnimationFrameDecodeRequest,
    },
}

impl PendingDecodeRequest {
    fn spawn_worker(
        self,
        sender: mpsc::Sender<DecodeWorkerMessage>,
        folder_scan_worker_count: &Arc<AtomicUsize>,
    ) -> Result<DecodeWorker, Box<DecodeStartFailure>> {
        match self {
            Self::Initial { request } => {
                let generation = request.generation();
                spawn_decode_worker(
                    sender,
                    request,
                    ImageDecodeWorkerKind::Initial,
                    Arc::clone(folder_scan_worker_count),
                )
                .map_err(|error| Box::new(DecodeStartFailure::Initial { generation, error }))
            }
            Self::FullResolution { request } => {
                let generation = request.generation();
                let file_version = request.file_version();
                spawn_decode_worker(
                    sender,
                    request,
                    ImageDecodeWorkerKind::FullResolution,
                    Arc::clone(folder_scan_worker_count),
                )
                .map_err(|error| {
                    Box::new(DecodeStartFailure::FullResolution {
                        generation,
                        file_version,
                        error,
                    })
                })
            }
            Self::AnimationFrame { request } => {
                let generation = request.generation();
                let path = request.path().to_path_buf();
                let file_version = request.file_version();
                let frame_index = request.frame_index();
                spawn_animation_frame_decode_worker(sender, request).map_err(|error| {
                    Box::new(DecodeStartFailure::AnimationFrame {
                        generation,
                        path,
                        file_version,
                        frame_index,
                        error,
                    })
                })
            }
        }
    }
}

fn decode_start_failure_into_error(failure: DecodeStartFailure) -> ViewerAppError {
    match failure {
        DecodeStartFailure::Initial { error, .. }
        | DecodeStartFailure::FullResolution { error, .. }
        | DecodeStartFailure::AnimationFrame { error, .. } => error,
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum DecodeWorkerKind {
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

struct DecodeWorker {
    generation: crate::domain::DecodeGeneration,
    kind: DecodeWorkerKind,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
    animation_frame: Option<ActiveAnimationFrameDecode>,
}

struct NavigationPreloadWorker {
    request: ImagePreloadRequest,
    cancel: Arc<AtomicBool>,
    handle: JoinHandle<()>,
}

struct ActiveAnimationFrameDecode {
    path: PathBuf,
    file_version: ImageFileVersion,
    format: SupportedImageFormat,
    source_size: ImageSize,
    frame_index: usize,
    viewport: ViewportSize,
    memory_policy: ImageMemoryPolicy,
    follow_up_frame_index: Arc<AtomicUsize>,
}

enum ActiveAnimationFrameRequest {
    InFlight {
        follow_up_frame_index: Arc<AtomicUsize>,
    },
    FollowUp {
        follow_up_frame_index: Arc<AtomicUsize>,
    },
}

struct FolderScanPermit {
    active_count: Arc<AtomicUsize>,
}

impl FolderScanPermit {
    fn try_acquire(active_count: &Arc<AtomicUsize>) -> Option<Self> {
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
        let _ = self.active_count.fetch_sub(1, Ordering::AcqRel);
    }
}

fn spawn_decode_worker(
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
    let should_scan_folder = !matches!(request.purpose(), ImageDecodePurpose::FolderNavigation(_));
    let worker_kind = kind.worker_kind();
    let cancel = Arc::new(AtomicBool::new(false));
    let worker_cancel = Arc::clone(&cancel);
    let handle = thread::Builder::new()
        .name("j3pic-linux-decode".to_owned())
        .spawn(move || match kind {
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
                let initial_sent = sender
                    .send(DecodeWorkerMessage::Initial { generation, result })
                    .is_ok();
                if initial_sent && should_scan_folder {
                    if let Some(scan_path) = scan_path {
                        let scan_released = sender
                            .send(DecodeWorkerMessage::InitialDecodeCompleted { generation })
                            .is_ok();
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
                            let _ = sender.send(message);
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
                .map(|(pixels, decoded_file_version)| (decoded_file_version, pixels));
                let (file_version, result) = match result {
                    Ok((decoded_file_version, pixels)) => (decoded_file_version, Ok(pixels)),
                    Err(error) => (file_version, Err(error)),
                };
                let _ = sender.send(DecodeWorkerMessage::FullResolution {
                    generation,
                    file_version,
                    result,
                });
            }
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
    let handle = thread::Builder::new()
        .name("j3pic-linux-preload".to_owned())
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
            let _ = sender.send(DecodeWorkerMessage::NavigationPreload {
                request: worker_request,
                result,
            });
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

fn spawn_animation_frame_decode_worker(
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
    let handle = thread::Builder::new()
        .name("j3pic-linux-animation".to_owned())
        .spawn(move || {
            let mut delivered = false;
            let result = load_animation_frame_for_view_with_prefetch_and_file_version(
                &path,
                frame_index,
                viewport,
                memory_policy,
                |decoded_file_version, frame| {
                    delivered = true;
                    sender
                        .send(DecodeWorkerMessage::AnimationFrame {
                            generation,
                            path: path.clone(),
                            file_version: decoded_file_version,
                            frame_index,
                            result: Ok(frame),
                        })
                        .is_ok()
                },
                |decoded_file_version, prefetched_frame_index, frame| {
                    if worker_follow_up_frame_index.load(Ordering::Acquire)
                        != prefetched_frame_index
                    {
                        return true;
                    }
                    let sent = sender
                        .send(DecodeWorkerMessage::AnimationFrame {
                            generation,
                            path: path.clone(),
                            file_version: decoded_file_version,
                            frame_index: prefetched_frame_index,
                            result: Ok(frame),
                        })
                        .is_ok();
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
                let _ = sender.send(DecodeWorkerMessage::AnimationFrame {
                    generation,
                    path,
                    file_version: Some(file_version),
                    frame_index,
                    result,
                });
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

fn spawn_decode_shutdown_joiner(
    decode_workers: Vec<DecodeWorker>,
    navigation_preload_workers: Vec<NavigationPreloadWorker>,
) {
    if decode_workers.is_empty() && navigation_preload_workers.is_empty() {
        return;
    }
    if let Err(error) = thread::Builder::new()
        .name("j3pic-linux-decode-shutdown".to_owned())
        .spawn(move || {
            join_decode_workers(decode_workers);
            join_navigation_preload_worker_vec(navigation_preload_workers);
        })
    {
        eprintln!("j3Pic decode shutdown worker start failed: {error}");
    }
}

fn join_decode_workers(workers: Vec<DecodeWorker>) {
    for worker in workers {
        let _ = worker.handle.join();
    }
}

fn join_navigation_preload_worker_vec(workers: Vec<NavigationPreloadWorker>) {
    for worker in workers {
        let _ = worker.handle.join();
    }
}

enum ExportWorkerMessage {
    Completed {
        path: PathBuf,
        options: ExportOptions,
        quality: Option<u8>,
        result: Result<(), ViewerAppError>,
    },
}

struct GtkExportController {
    sender: mpsc::Sender<ExportWorkerMessage>,
    receiver: mpsc::Receiver<ExportWorkerMessage>,
    active_worker: Option<ExportWorker>,
}

struct ExportWorker {
    handle: JoinHandle<()>,
}

enum GtkExportShutdownOutcome {
    Complete,
    WaitingForWorker(mpsc::Receiver<()>),
}

impl GtkExportController {
    fn new() -> Self {
        let (sender, receiver) = mpsc::channel();
        Self {
            sender,
            receiver,
            active_worker: None,
        }
    }

    fn start_export(
        &mut self,
        request: ImageExportRequest,
        quality: Option<u8>,
    ) -> Result<(), ViewerAppError> {
        self.join_finished_worker();
        if self.active_worker.is_some() {
            return Err(ViewerAppError::ExportWorkerStart {
                path: request.path().to_path_buf(),
                source: std::io::Error::new(
                    std::io::ErrorKind::WouldBlock,
                    "another export is still running",
                ),
            });
        }
        let path = request.path().to_path_buf();
        let options = request.options();
        let sender = self.sender.clone();
        let error_path = path.clone();
        let handle = thread::Builder::new()
            .name("j3pic-linux-export".to_owned())
            .spawn(move || {
                let result = request.export();
                let _ = sender.send(ExportWorkerMessage::Completed {
                    path,
                    options,
                    quality,
                    result,
                });
            })
            .map_err(|source| ViewerAppError::ExportWorkerStart {
                path: error_path,
                source,
            })?;
        self.active_worker = Some(ExportWorker { handle });
        Ok(())
    }

    fn is_busy(&mut self) -> bool {
        self.join_finished_worker();
        self.active_worker.is_some()
    }

    fn drain_messages(&mut self) -> Vec<ExportWorkerMessage> {
        self.join_finished_worker();
        let mut messages = Vec::new();
        while let Ok(message) = self.receiver.try_recv() {
            messages.push(message);
        }
        self.join_finished_worker();
        messages
    }

    fn has_live_work(&self) -> bool {
        self.active_worker.is_some()
    }

    fn shutdown(&mut self) -> GtkExportShutdownOutcome {
        self.join_finished_worker();
        let worker = self.active_worker.take();
        self.close_worker_message_channel();
        let Some(worker) = worker else {
            return GtkExportShutdownOutcome::Complete;
        };

        match spawn_export_shutdown_joiner(worker) {
            Ok(receiver) => GtkExportShutdownOutcome::WaitingForWorker(receiver),
            Err(worker) => {
                join_export_worker(worker);
                GtkExportShutdownOutcome::Complete
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

    fn join_finished_worker(&mut self) {
        if self
            .active_worker
            .as_ref()
            .is_some_and(|worker| worker.handle.is_finished())
        {
            if let Some(worker) = self.active_worker.take() {
                let _ = worker.handle.join();
            }
        }
    }
}

fn spawn_export_shutdown_joiner(worker: ExportWorker) -> Result<mpsc::Receiver<()>, ExportWorker> {
    let (worker_sender, worker_receiver) = mpsc::sync_channel::<ExportWorker>(1);
    let (completion_sender, completion_receiver) = mpsc::channel();
    let joiner = thread::Builder::new()
        .name("j3pic-linux-export-shutdown".to_owned())
        .spawn(move || {
            if let Ok(worker) = worker_receiver.recv() {
                join_export_worker(worker);
                let _ = completion_sender.send(());
            }
        });

    match joiner {
        Ok(handle) => {
            drop(handle);
            worker_sender
                .send(worker)
                .map_err(|send_error| send_error.0)?;
            Ok(completion_receiver)
        }
        Err(_) => Err(worker),
    }
}

fn join_export_worker(worker: ExportWorker) {
    let _ = worker.handle.join();
}

#[derive(Debug, Clone)]
struct ExportDialogDefaults {
    source_path: PathBuf,
    source_size: ImageSize,
    format: ExportFormat,
    quality: u8,
    suggested_path: PathBuf,
}

#[derive(Debug, Clone, Copy)]
enum SizeAxis {
    Width,
    Height,
}

async fn show_export_options_dialog(
    parent: &gtk::ApplicationWindow,
    defaults: ExportDialogDefaults,
    language: UiLanguage,
) -> Option<ExportOptions> {
    let dialog = gtk::Dialog::builder()
        .title(ui_text::export_dialog_title(language))
        .transient_for(parent)
        .modal(true)
        .default_width(390)
        .build();
    dialog.add_button(
        match language {
            UiLanguage::English => "Cancel",
            UiLanguage::Korean => "취소",
        },
        gtk::ResponseType::Cancel,
    );
    dialog.add_button(
        match language {
            UiLanguage::English => "OK",
            UiLanguage::Korean => "확인",
        },
        gtk::ResponseType::Ok,
    );
    dialog.set_default_response(gtk::ResponseType::Ok);

    let area = dialog.content_area();
    area.set_spacing(8);
    area.set_margin_top(12);
    area.set_margin_bottom(12);
    area.set_margin_start(12);
    area.set_margin_end(12);

    let root = gtk::Box::new(gtk::Orientation::Vertical, 8);
    area.append(&root);

    let file_grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(8)
        .build();
    let file_frame = gtk::Frame::new(Some(match language {
        UiLanguage::English => "File",
        UiLanguage::Korean => "파일",
    }));
    file_frame.set_child(Some(&file_grid));
    root.append(&file_frame);

    let size_grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(8)
        .build();
    let size_frame = gtk::Frame::new(Some(match language {
        UiLanguage::English => "Size",
        UiLanguage::Korean => "크기",
    }));
    size_frame.set_child(Some(&size_grid));
    root.append(&size_frame);

    let format_combo = combo_box(EXPORT_FORMAT_LABELS);
    set_combo_index(&format_combo, export_format_index(defaults.format));
    let quality = gtk::Entry::new();
    quality.set_text(&defaults.quality.to_string());
    let remove_metadata = gtk::CheckButton::with_label(match language {
        UiLanguage::English => "Remove Metadata",
        UiLanguage::Korean => "메타데이터 제거",
    });
    let rotation_combo = combo_box(ui_text::export_rotation_labels(language));
    let aspect = gtk::CheckButton::with_label(match language {
        UiLanguage::English => "Keep Aspect Ratio",
        UiLanguage::Korean => "가로세로 비율 유지",
    });
    aspect.set_active(true);
    let width_entry = gtk::Entry::new();
    let height_entry = gtk::Entry::new();
    let source_size_label = gtk::Label::new(None);
    source_size_label.set_xalign(0.0);
    let reset_size = gtk::Button::with_label(match language {
        UiLanguage::English => "Original Size",
        UiLanguage::Korean => "원본 크기",
    });
    let size_actions = gtk::Box::new(gtk::Orientation::Horizontal, 8);
    size_actions.append(&aspect);
    size_actions.append(&reset_size);

    attach_labeled(
        &file_grid,
        0,
        match language {
            UiLanguage::English => "Format",
            UiLanguage::Korean => "포맷",
        },
        &format_combo,
    );
    attach_labeled(
        &file_grid,
        1,
        match language {
            UiLanguage::English => "JPEG Quality",
            UiLanguage::Korean => "JPEG 품질",
        },
        &quality,
    );
    attach_labeled(
        &file_grid,
        2,
        match language {
            UiLanguage::English => "Rotation",
            UiLanguage::Korean => "회전",
        },
        &rotation_combo,
    );
    file_grid.attach(&remove_metadata, 1, 3, 1, 1);
    size_grid.attach(&source_size_label, 0, 0, 2, 1);
    attach_labeled(
        &size_grid,
        1,
        match language {
            UiLanguage::English => "Width",
            UiLanguage::Korean => "너비",
        },
        &width_entry,
    );
    attach_labeled(
        &size_grid,
        2,
        match language {
            UiLanguage::English => "Height",
            UiLanguage::Korean => "높이",
        },
        &height_entry,
    );
    size_grid.attach(&size_actions, 1, 3, 1, 1);

    let source_size = Rc::new(Cell::new(defaults.source_size));
    let updating = Rc::new(Cell::new(false));
    let last_size_axis = Rc::new(Cell::new(SizeAxis::Width));
    reset_export_size_fields(
        source_size.get(),
        &source_size_label,
        &width_entry,
        &height_entry,
        &updating,
        language,
    );
    update_export_field_state(
        ExportFieldStateWidgets {
            format_combo: &format_combo,
            quality: &quality,
            width_entry: &width_entry,
            height_entry: &height_entry,
            aspect: &aspect,
            reset_size: &reset_size,
            source_size_label: &source_size_label,
        },
        source_size.get(),
        language,
    );

    {
        let source_size = Rc::clone(&source_size);
        let source_size_label = source_size_label.clone();
        let width_entry = width_entry.clone();
        let height_entry = height_entry.clone();
        let format_combo = format_combo.clone();
        let quality = quality.clone();
        let aspect = aspect.clone();
        let reset_size = reset_size.clone();
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        let defaults = defaults.clone();
        rotation_combo.connect_changed(move |combo| {
            let rotation = rotation_from_combo(combo).unwrap_or(ImageRotation::ZERO);
            let rotated = defaults.source_size.with_rotation(rotation);
            source_size.set(rotated);
            last_size_axis.set(SizeAxis::Width);
            reset_export_size_fields(
                rotated,
                &source_size_label,
                &width_entry,
                &height_entry,
                &updating,
                language,
            );
            update_export_field_state(
                ExportFieldStateWidgets {
                    format_combo: &format_combo,
                    quality: &quality,
                    width_entry: &width_entry,
                    height_entry: &height_entry,
                    aspect: &aspect,
                    reset_size: &reset_size,
                    source_size_label: &source_size_label,
                },
                rotated,
                language,
            );
        });
    }
    {
        let quality = quality.clone();
        let width_entry = width_entry.clone();
        let height_entry = height_entry.clone();
        let aspect = aspect.clone();
        let reset_size = reset_size.clone();
        let source_size_label = source_size_label.clone();
        let source_size = Rc::clone(&source_size);
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        format_combo.connect_changed(move |combo| {
            let format = export_format_from_combo(combo).unwrap_or(ExportFormat::Png);
            update_export_field_state(
                ExportFieldStateWidgets {
                    format_combo: combo,
                    quality: &quality,
                    width_entry: &width_entry,
                    height_entry: &height_entry,
                    aspect: &aspect,
                    reset_size: &reset_size,
                    source_size_label: &source_size_label,
                },
                source_size.get(),
                language,
            );
            if format != ExportFormat::Ico {
                last_size_axis.set(SizeAxis::Width);
                reset_export_size_fields(
                    source_size.get(),
                    &source_size_label,
                    &width_entry,
                    &height_entry,
                    &updating,
                    language,
                );
            }
        });
    }
    {
        let source_size = Rc::clone(&source_size);
        let source_size_label = source_size_label.clone();
        let width_entry = width_entry.clone();
        let height_entry = height_entry.clone();
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        reset_size.connect_clicked(move |_| {
            last_size_axis.set(SizeAxis::Width);
            reset_export_size_fields(
                source_size.get(),
                &source_size_label,
                &width_entry,
                &height_entry,
                &updating,
                language,
            );
        });
    }
    {
        let height_entry = height_entry.clone();
        let aspect = aspect.clone();
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        let source_size = Rc::clone(&source_size);
        width_entry.connect_changed(move |entry| {
            last_size_axis.set(SizeAxis::Width);
            if updating.get() || !aspect.is_active() {
                return;
            }
            let text = entry.text();
            let Some(width) = parse_u32_text(text.as_str()) else {
                return;
            };
            sync_export_size_preserving_aspect(
                source_size.get(),
                SizeAxis::Width,
                width,
                None,
                Some(&height_entry),
                &updating,
            );
        });
    }
    {
        let width_entry = width_entry.clone();
        let aspect = aspect.clone();
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        let source_size = Rc::clone(&source_size);
        height_entry.connect_changed(move |entry| {
            last_size_axis.set(SizeAxis::Height);
            if updating.get() || !aspect.is_active() {
                return;
            }
            let text = entry.text();
            let Some(height) = parse_u32_text(text.as_str()) else {
                return;
            };
            sync_export_size_preserving_aspect(
                source_size.get(),
                SizeAxis::Height,
                height,
                Some(&width_entry),
                None,
                &updating,
            );
        });
    }
    {
        let width_entry = width_entry.clone();
        let height_entry = height_entry.clone();
        let updating = Rc::clone(&updating);
        let last_size_axis = Rc::clone(&last_size_axis);
        let source_size = Rc::clone(&source_size);
        aspect.connect_toggled(move |check| {
            if !check.is_active() {
                return;
            }
            let axis = last_size_axis.get();
            let value = match axis {
                SizeAxis::Width => {
                    let text = width_entry.text();
                    parse_u32_text(text.as_str())
                }
                SizeAxis::Height => {
                    let text = height_entry.text();
                    parse_u32_text(text.as_str())
                }
            };
            if let Some(value) = value {
                sync_export_size_preserving_aspect(
                    source_size.get(),
                    axis,
                    value,
                    Some(&width_entry),
                    Some(&height_entry),
                    &updating,
                );
            }
        });
    }

    loop {
        let response = dialog.run_future().await;
        if response != gtk::ResponseType::Ok {
            dialog.close();
            return None;
        }
        match read_export_options_from_dialog(
            ExportOptionsDialogFields {
                format_combo: &format_combo,
                quality: &quality,
                remove_metadata: &remove_metadata,
                rotation_combo: &rotation_combo,
                width_entry: &width_entry,
                height_entry: &height_entry,
                aspect: &aspect,
            },
            ExportSizeSelection {
                source_size: source_size.get(),
                last_axis: last_size_axis.get(),
            },
        ) {
            Ok(options) => {
                dialog.close();
                return Some(options);
            }
            Err(message) => show_modal_warning(&dialog, &message).await,
        }
    }
}

fn reset_export_size_fields(
    size: ImageSize,
    label: &gtk::Label,
    width_entry: &gtk::Entry,
    height_entry: &gtk::Entry,
    updating: &Cell<bool>,
    language: UiLanguage,
) {
    label.set_text(&match language {
        UiLanguage::English => format!("Original size: {} x {}", size.width(), size.height()),
        UiLanguage::Korean => format!("원본 크기: {} x {}", size.width(), size.height()),
    });
    updating.set(true);
    width_entry.set_text(&size.width().to_string());
    height_entry.set_text(&size.height().to_string());
    updating.set(false);
}

struct ExportFieldStateWidgets<'a> {
    format_combo: &'a gtk::ComboBoxText,
    quality: &'a gtk::Entry,
    width_entry: &'a gtk::Entry,
    height_entry: &'a gtk::Entry,
    aspect: &'a gtk::CheckButton,
    reset_size: &'a gtk::Button,
    source_size_label: &'a gtk::Label,
}

fn update_export_field_state(
    fields: ExportFieldStateWidgets<'_>,
    source_size: ImageSize,
    language: UiLanguage,
) {
    let format = export_format_from_combo(fields.format_combo).unwrap_or(ExportFormat::Png);
    fields.quality.set_sensitive(format == ExportFormat::Jpeg);
    let resizable = format != ExportFormat::Ico;
    fields.width_entry.set_sensitive(resizable);
    fields.height_entry.set_sensitive(resizable);
    fields.aspect.set_sensitive(resizable);
    fields.reset_size.set_sensitive(resizable);
    if format == ExportFormat::Ico {
        fields.source_size_label.set_text(match language {
            UiLanguage::English => "ICO sizes: 16, 32, 48, 256 px",
            UiLanguage::Korean => "ICO 크기: 16, 32, 48, 256 px",
        });
    } else {
        fields.source_size_label.set_text(&match language {
            UiLanguage::English => {
                format!(
                    "Original size: {} x {}",
                    source_size.width(),
                    source_size.height()
                )
            }
            UiLanguage::Korean => {
                format!(
                    "원본 크기: {} x {}",
                    source_size.width(),
                    source_size.height()
                )
            }
        });
    }
}

fn sync_export_size_preserving_aspect(
    source_size: ImageSize,
    axis: SizeAxis,
    value: u32,
    width_entry: Option<&gtk::Entry>,
    height_entry: Option<&gtk::Entry>,
    updating: &Cell<bool>,
) {
    let size = export_live_synced_size_for_win32_reference(source_size, axis, value);
    let Some(size) = size else {
        return;
    };
    updating.set(true);
    match axis {
        SizeAxis::Width => {
            if let Some(height_entry) = height_entry {
                height_entry.set_text(&size.height().to_string());
            }
        }
        SizeAxis::Height => {
            if let Some(width_entry) = width_entry {
                width_entry.set_text(&size.width().to_string());
            }
        }
    }
    updating.set(false);
}

fn export_live_synced_size_for_win32_reference(
    original_size: ImageSize,
    axis: SizeAxis,
    value: u32,
) -> Option<ImageSize> {
    match axis {
        SizeAxis::Width => export_size_from_width_preserving_aspect(original_size, value),
        SizeAxis::Height => export_size_from_height_preserving_aspect(original_size, value),
    }
}

fn export_target_size_from_dialog_values(
    format: ExportFormat,
    source_size: ImageSize,
    width: u32,
    height: u32,
    aspect_active: bool,
    last_axis: SizeAxis,
) -> Result<Option<ImageSize>, String> {
    if format == ExportFormat::Ico {
        return Ok(None);
    }
    let size = if aspect_active {
        let axis_value = match last_axis {
            SizeAxis::Width => width,
            SizeAxis::Height => height,
        };
        match last_axis {
            SizeAxis::Width => export_size_from_width_preserving_aspect(source_size, axis_value),
            SizeAxis::Height => export_size_from_height_preserving_aspect(source_size, axis_value),
        }
        .ok_or_else(|| "내보내기 크기를 계산할 수 없습니다.".to_owned())?
    } else {
        ImageSize::new(width, height)
    };
    let pixel_count = size
        .pixel_count()
        .ok_or_else(|| "내보내기 크기가 너무 큽니다.".to_owned())?;
    if pixel_count > MAX_CONFIG_IMAGE_PIXELS {
        return Err(format!(
            "내보내기 크기는 최대 {} 픽셀까지 가능합니다.",
            MAX_CONFIG_IMAGE_PIXELS
        ));
    }
    if size == source_size {
        Ok(None)
    } else {
        Ok(Some(size))
    }
}

struct ExportOptionsDialogFields<'a> {
    format_combo: &'a gtk::ComboBoxText,
    quality: &'a gtk::Entry,
    remove_metadata: &'a gtk::CheckButton,
    rotation_combo: &'a gtk::ComboBoxText,
    width_entry: &'a gtk::Entry,
    height_entry: &'a gtk::Entry,
    aspect: &'a gtk::CheckButton,
}

struct ExportSizeSelection {
    source_size: ImageSize,
    last_axis: SizeAxis,
}

fn read_export_options_from_dialog(
    fields: ExportOptionsDialogFields<'_>,
    size_selection: ExportSizeSelection,
) -> Result<ExportOptions, String> {
    let format = export_format_from_combo(fields.format_combo)
        .ok_or_else(|| "내보내기 형식을 선택할 수 없습니다.".to_owned())?;
    let quality = if format == ExportFormat::Jpeg {
        Some(parse_u8_entry(
            fields.quality,
            "JPEG 품질",
            MIN_EXPORT_QUALITY,
            MAX_EXPORT_QUALITY,
        )?)
    } else {
        None
    };
    let rotation = rotation_from_combo(fields.rotation_combo)
        .ok_or_else(|| "회전 값을 선택할 수 없습니다.".to_owned())?;
    let target_size = if format == ExportFormat::Ico {
        None
    } else {
        let width = parse_u32_entry(fields.width_entry, "너비", 1, u32::MAX)?;
        let height = parse_u32_entry(fields.height_entry, "높이", 1, u32::MAX)?;
        export_target_size_from_dialog_values(
            format,
            size_selection.source_size,
            width,
            height,
            fields.aspect.is_active(),
            size_selection.last_axis,
        )?
    };
    Ok(ExportOptions::new(format, quality)
        .with_rotation(rotation)
        .with_target_size(target_size)
        .with_remove_metadata(fields.remove_metadata.is_active()))
}

async fn confirm_overwrite_corrected_export_path(
    parent: &gtk::ApplicationWindow,
    path: &Path,
    language: UiLanguage,
) -> bool {
    let message = corrected_export_overwrite_message(language, path);
    confirm_export_overwrite(parent, &message, language).await
}

async fn confirm_overwrite_export_path(
    parent: &gtk::ApplicationWindow,
    path: &Path,
    language: UiLanguage,
) -> bool {
    let message = ui_text::export_overwrite_message(language, path);
    confirm_export_overwrite(parent, &message, language).await
}

async fn confirm_export_overwrite(
    parent: &gtk::ApplicationWindow,
    message: &str,
    language: UiLanguage,
) -> bool {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .message_type(gtk::MessageType::Warning)
        .text(message)
        .build();
    for (label, is_yes) in ui_text::yes_no_buttons(language) {
        dialog.add_button(
            label,
            if is_yes {
                gtk::ResponseType::Yes
            } else {
                gtk::ResponseType::No
            },
        );
    }
    dialog.set_default_response(export_overwrite_default_response());
    let response = dialog.run_future().await;
    dialog.close();
    response == gtk::ResponseType::Yes
}

fn export_overwrite_default_response() -> gtk::ResponseType {
    gtk::ResponseType::Yes
}

async fn show_settings_dialog(
    parent: &gtk::ApplicationWindow,
    base: AppConfig,
) -> Option<AppConfig> {
    let language = base.ui_language();
    let dialog = gtk::Dialog::builder()
        .title(ui_text::settings_title(language))
        .transient_for(parent)
        .modal(true)
        .default_width(780)
        .default_height(610)
        .build();
    dialog.add_button(
        match language {
            UiLanguage::English => "Defaults",
            UiLanguage::Korean => "기본값",
        },
        gtk::ResponseType::Other(1),
    );
    dialog.add_button(
        match language {
            UiLanguage::English => "Cancel",
            UiLanguage::Korean => "취소",
        },
        gtk::ResponseType::Cancel,
    );
    dialog.add_button(
        match language {
            UiLanguage::English => "OK",
            UiLanguage::Korean => "확인",
        },
        gtk::ResponseType::Ok,
    );
    dialog.set_default_response(gtk::ResponseType::Ok);

    let widgets = SettingsWidgets::new(&base, language);
    dialog.content_area().append(&widgets.root);

    loop {
        let response = dialog.run_future().await;
        match response {
            gtk::ResponseType::Ok => match widgets.read_config(&base) {
                Ok(config) => {
                    dialog.close();
                    return Some(config);
                }
                Err(message) => show_modal_warning(&dialog, &message).await,
            },
            gtk::ResponseType::Other(1) => widgets.write_config(&AppConfig::default()),
            _ => {
                dialog.close();
                return None;
            }
        }
    }
}

struct SettingsWidgets {
    root: gtk::Grid,
    ui_language: gtk::ComboBoxText,
    default_view_mode: gtk::ComboBoxText,
    scaling_quality: gtk::ComboBoxText,
    show_status_bar: gtk::CheckButton,
    detailed_status_text: gtk::CheckButton,
    animation_autoplay: gtk::CheckButton,
    min_zoom_scale: gtk::Entry,
    max_zoom_scale: gtk::Entry,
    zoom_step_factor: gtk::Entry,
    large_image_pixel_threshold: gtk::Entry,
    max_image_pixels: gtk::Entry,
    preview_max_pixels: gtk::Entry,
    preview_oversample: gtk::Entry,
    full_resolution_request_scale: gtk::Entry,
    max_resident_mib: gtk::Entry,
    max_cache_entry_mib: gtk::Entry,
    max_cache_entries: gtk::Entry,
    default_frame_delay_ms: gtk::Entry,
    min_frame_delay_ms: gtk::Entry,
    max_frame_delay_ms: gtk::Entry,
    wrap_navigation: gtk::CheckButton,
    auto_skip_navigation: gtk::CheckButton,
    navigation_attempts: gtk::Entry,
    export_format_policy: gtk::ComboBoxText,
    export_quality: gtk::Entry,
    export_suffix: gtk::Entry,
    jpeg_alpha_background_rgb: gtk::Entry,
    zoom_shortcut: gtk::ComboBoxText,
    image_navigation_shortcut: gtk::ComboBoxText,
    image_pan_shortcut: gtk::ComboBoxText,
    window_move_shortcut: gtk::ComboBoxText,
}

impl SettingsWidgets {
    fn new(config: &AppConfig, language: UiLanguage) -> Self {
        let root = gtk::Grid::builder()
            .column_homogeneous(true)
            .column_spacing(16)
            .margin_top(12)
            .margin_bottom(12)
            .margin_start(12)
            .margin_end(12)
            .build();
        let left = gtk::Box::new(gtk::Orientation::Vertical, 10);
        let right = gtk::Box::new(gtk::Orientation::Vertical, 10);
        root.attach(&left, 0, 0, 1, 1);
        root.attach(&right, 1, 0, 1, 1);

        let groups = ui_text::settings_group_titles(language);
        let (general_frame, general) = settings_group(groups[0]);
        let (zoom_frame, zoom_grid) = settings_group(groups[1]);
        let (animation_frame, animation) = settings_group(groups[2]);
        let (navigation_frame, navigation_grid) = settings_group(groups[3]);
        let (memory_frame, memory) = settings_group(groups[4]);
        let (export_frame, export) = settings_group(groups[5]);
        let (shortcuts_frame, shortcuts) = settings_group(groups[6]);
        left.append(&general_frame);
        left.append(&zoom_frame);
        left.append(&animation_frame);
        left.append(&navigation_frame);
        right.append(&memory_frame);
        right.append(&export_frame);
        right.append(&shortcuts_frame);

        let ui_language = combo_box(ui_text::UI_LANGUAGE_LABELS);
        let default_view_mode = combo_box(ui_text::settings_view_mode_labels(language));
        let scaling_quality = combo_box(ui_text::settings_scaling_quality_labels(language));
        let show_status_bar = gtk::CheckButton::with_label(match language {
            UiLanguage::English => "Show Status Bar",
            UiLanguage::Korean => "상태바 표시",
        });
        let detailed_status_text = gtk::CheckButton::with_label(match language {
            UiLanguage::English => "Detailed Status Text",
            UiLanguage::Korean => "자세한 상태 텍스트",
        });
        let min_zoom_scale = gtk::Entry::new();
        let max_zoom_scale = gtk::Entry::new();
        let zoom_step_factor = gtk::Entry::new();
        attach_labeled(
            &general,
            0,
            match language {
                UiLanguage::English => "Language",
                UiLanguage::Korean => "언어",
            },
            &ui_language,
        );
        attach_labeled(
            &general,
            1,
            match language {
                UiLanguage::English => "Default View Mode",
                UiLanguage::Korean => "기본 보기 모드",
            },
            &default_view_mode,
        );
        attach_labeled(
            &general,
            2,
            match language {
                UiLanguage::English => "Scaling Quality",
                UiLanguage::Korean => "스케일링 품질",
            },
            &scaling_quality,
        );
        general.attach(&show_status_bar, 1, 3, 1, 1);
        general.attach(&detailed_status_text, 1, 4, 1, 1);
        attach_labeled(
            &zoom_grid,
            0,
            match language {
                UiLanguage::English => "Minimum Zoom",
                UiLanguage::Korean => "최소 줌",
            },
            &min_zoom_scale,
        );
        attach_labeled(
            &zoom_grid,
            1,
            match language {
                UiLanguage::English => "Maximum Zoom",
                UiLanguage::Korean => "최대 줌",
            },
            &max_zoom_scale,
        );
        attach_labeled(
            &zoom_grid,
            2,
            match language {
                UiLanguage::English => "Zoom Step Factor",
                UiLanguage::Korean => "줌 단계 배율",
            },
            &zoom_step_factor,
        );

        let large_image_pixel_threshold = gtk::Entry::new();
        let max_image_pixels = gtk::Entry::new();
        let preview_max_pixels = gtk::Entry::new();
        let preview_oversample = gtk::Entry::new();
        let full_resolution_request_scale = gtk::Entry::new();
        let max_resident_mib = gtk::Entry::new();
        let max_cache_entry_mib = gtk::Entry::new();
        let max_cache_entries = gtk::Entry::new();
        let default_frame_delay_ms = gtk::Entry::new();
        let min_frame_delay_ms = gtk::Entry::new();
        let max_frame_delay_ms = gtk::Entry::new();
        let animation_autoplay = gtk::CheckButton::with_label(match language {
            UiLanguage::English => "Autoplay",
            UiLanguage::Korean => "자동재생",
        });
        let wrap_navigation = gtk::CheckButton::with_label(match language {
            UiLanguage::English => "Wrap Navigation",
            UiLanguage::Korean => "순환 이동",
        });
        let auto_skip_navigation = gtk::CheckButton::with_label(match language {
            UiLanguage::English => "Auto-skip Failed Files",
            UiLanguage::Korean => "실패 파일 자동 스킵",
        });
        let navigation_attempts = gtk::Entry::new();
        attach_labeled(
            &memory,
            0,
            match language {
                UiLanguage::English => "Large Pixel Threshold",
                UiLanguage::Korean => "대용량 픽셀 기준",
            },
            &large_image_pixel_threshold,
        );
        attach_labeled(
            &memory,
            1,
            match language {
                UiLanguage::English => "Maximum Image Pixels",
                UiLanguage::Korean => "최대 이미지 픽셀",
            },
            &max_image_pixels,
        );
        attach_labeled(
            &memory,
            2,
            match language {
                UiLanguage::English => "Preview Maximum Pixels",
                UiLanguage::Korean => "프리뷰 최대 픽셀",
            },
            &preview_max_pixels,
        );
        attach_labeled(
            &memory,
            3,
            match language {
                UiLanguage::English => "Preview Oversample",
                UiLanguage::Korean => "프리뷰 배율",
            },
            &preview_oversample,
        );
        attach_labeled(
            &memory,
            4,
            match language {
                UiLanguage::English => "Full-res Request Scale",
                UiLanguage::Korean => "전체 해상도 요청 배율",
            },
            &full_resolution_request_scale,
        );
        attach_labeled(
            &memory,
            5,
            match language {
                UiLanguage::English => "Total Cache (MiB)",
                UiLanguage::Korean => "캐시 총량(MiB)",
            },
            &max_resident_mib,
        );
        attach_labeled(
            &memory,
            6,
            match language {
                UiLanguage::English => "Entry Cache Limit (MiB)",
                UiLanguage::Korean => "캐시 항목 한도(MiB)",
            },
            &max_cache_entry_mib,
        );
        attach_labeled(
            &memory,
            7,
            match language {
                UiLanguage::English => "Cache Entries",
                UiLanguage::Korean => "캐시 항목 수",
            },
            &max_cache_entries,
        );
        animation.attach(&animation_autoplay, 1, 0, 1, 1);
        attach_labeled(
            &animation,
            1,
            match language {
                UiLanguage::English => "Default Frame Delay (ms)",
                UiLanguage::Korean => "기본 프레임 지연(ms)",
            },
            &default_frame_delay_ms,
        );
        attach_labeled(
            &animation,
            2,
            match language {
                UiLanguage::English => "Minimum Frame Delay (ms)",
                UiLanguage::Korean => "최소 프레임 지연(ms)",
            },
            &min_frame_delay_ms,
        );
        attach_labeled(
            &animation,
            3,
            match language {
                UiLanguage::English => "Maximum Frame Delay (ms)",
                UiLanguage::Korean => "최대 프레임 지연(ms)",
            },
            &max_frame_delay_ms,
        );
        navigation_grid.attach(&wrap_navigation, 1, 0, 1, 1);
        navigation_grid.attach(&auto_skip_navigation, 1, 1, 1, 1);
        attach_labeled(
            &navigation_grid,
            2,
            match language {
                UiLanguage::English => "Maximum Attempts",
                UiLanguage::Korean => "최대 시도 횟수",
            },
            &navigation_attempts,
        );

        let export_format_policy = combo_box(ui_text::settings_export_policy_labels(language));
        let export_quality = gtk::Entry::new();
        let export_suffix = gtk::Entry::new();
        let jpeg_alpha_background_rgb = gtk::Entry::new();
        attach_labeled(
            &export,
            0,
            match language {
                UiLanguage::English => "Default Format Policy",
                UiLanguage::Korean => "기본 포맷 정책",
            },
            &export_format_policy,
        );
        attach_labeled(
            &export,
            1,
            match language {
                UiLanguage::English => "JPEG Quality",
                UiLanguage::Korean => "JPEG 품질",
            },
            &export_quality,
        );
        attach_labeled(
            &export,
            2,
            match language {
                UiLanguage::English => "Filename Suffix",
                UiLanguage::Korean => "파일명 suffix",
            },
            &export_suffix,
        );
        attach_labeled(
            &export,
            3,
            match language {
                UiLanguage::English => "JPEG Alpha RGB",
                UiLanguage::Korean => "JPEG 투명 배경 RGB",
            },
            &jpeg_alpha_background_rgb,
        );

        let zoom_shortcut = combo_box(ui_text::settings_wheel_shortcut_labels(language));
        let image_navigation_shortcut =
            combo_box(ui_text::settings_wheel_shortcut_labels(language));
        let image_pan_shortcut = combo_box(ui_text::settings_drag_shortcut_labels(language));
        let window_move_shortcut = combo_box(ui_text::settings_drag_shortcut_labels(language));
        attach_labeled(
            &shortcuts,
            0,
            match language {
                UiLanguage::English => "Zoom",
                UiLanguage::Korean => "확대/축소",
            },
            &zoom_shortcut,
        );
        attach_labeled(
            &shortcuts,
            1,
            match language {
                UiLanguage::English => "Previous/Next Image",
                UiLanguage::Korean => "이전/다음 이미지",
            },
            &image_navigation_shortcut,
        );
        attach_labeled(
            &shortcuts,
            2,
            match language {
                UiLanguage::English => "Image Pan",
                UiLanguage::Korean => "이미지 이동",
            },
            &image_pan_shortcut,
        );
        attach_labeled(
            &shortcuts,
            3,
            match language {
                UiLanguage::English => "Window Move",
                UiLanguage::Korean => "창 이동",
            },
            &window_move_shortcut,
        );

        let project_link = gtk::LinkButton::with_label(PROJECT_LINK_URL, PROJECT_LINK_URL);
        project_link.set_halign(gtk::Align::Start);
        root.attach(&project_link, 0, 1, 2, 1);

        let widgets = Self {
            root,
            ui_language,
            default_view_mode,
            scaling_quality,
            show_status_bar,
            detailed_status_text,
            animation_autoplay,
            min_zoom_scale,
            max_zoom_scale,
            zoom_step_factor,
            large_image_pixel_threshold,
            max_image_pixels,
            preview_max_pixels,
            preview_oversample,
            full_resolution_request_scale,
            max_resident_mib,
            max_cache_entry_mib,
            max_cache_entries,
            default_frame_delay_ms,
            min_frame_delay_ms,
            max_frame_delay_ms,
            wrap_navigation,
            auto_skip_navigation,
            navigation_attempts,
            export_format_policy,
            export_quality,
            export_suffix,
            jpeg_alpha_background_rgb,
            zoom_shortcut,
            image_navigation_shortcut,
            image_pan_shortcut,
            window_move_shortcut,
        };
        widgets.write_config(config);
        widgets
    }

    fn write_config(&self, config: &AppConfig) {
        set_combo_index(
            &self.ui_language,
            ui_text::ui_language_index(config.ui_language()) as u32,
        );
        set_combo_index(
            &self.default_view_mode,
            view_mode_index(config.default_view_mode()),
        );
        set_combo_index(
            &self.scaling_quality,
            scaling_quality_index(config.scaling_quality()),
        );
        let status = config.status_ui_settings();
        self.show_status_bar.set_active(status.show_status_bar());
        self.detailed_status_text
            .set_active(status.detailed_status_text());
        self.animation_autoplay
            .set_active(config.animation_autoplay());
        let zoom = config.zoom_settings();
        self.min_zoom_scale
            .set_text(&zoom.min_zoom_scale().to_string());
        self.max_zoom_scale
            .set_text(&zoom.max_zoom_scale().to_string());
        self.zoom_step_factor
            .set_text(&zoom.zoom_step_factor().to_string());
        let memory = config.memory_policy_settings();
        self.large_image_pixel_threshold
            .set_text(&memory.large_image_pixel_threshold().to_string());
        self.max_image_pixels
            .set_text(&memory.max_image_pixels().to_string());
        self.preview_max_pixels
            .set_text(&memory.preview_max_pixels().to_string());
        self.preview_oversample
            .set_text(&memory.preview_oversample().to_string());
        self.full_resolution_request_scale
            .set_text(&memory.full_resolution_request_scale().to_string());
        self.max_resident_mib
            .set_text(&memory.max_resident_mib().to_string());
        self.max_cache_entry_mib
            .set_text(&memory.max_cache_entry_mib().to_string());
        self.max_cache_entries
            .set_text(&memory.max_cache_entries().to_string());
        let timing = config.animation_timing_settings();
        self.default_frame_delay_ms
            .set_text(&timing.default_frame_delay_ms().to_string());
        self.min_frame_delay_ms
            .set_text(&timing.min_frame_delay_ms().to_string());
        self.max_frame_delay_ms
            .set_text(&timing.max_frame_delay_ms().to_string());
        let navigation = config.navigation_settings();
        self.wrap_navigation
            .set_active(navigation.wrap_navigation());
        self.auto_skip_navigation
            .set_active(navigation.auto_skip_failed_navigation());
        self.navigation_attempts
            .set_text(&navigation.max_navigation_attempts_per_command().to_string());
        let export = config.export_settings();
        set_combo_index(
            &self.export_format_policy,
            export_policy_index(export.default_export_format_policy()),
        );
        self.export_quality
            .set_text(&config.export_default_quality().to_string());
        self.export_suffix.set_text(export.export_filename_suffix());
        let color = export.jpeg_alpha_background_rgb();
        self.jpeg_alpha_background_rgb.set_text(&format!(
            "{},{},{}",
            color.red(),
            color.green(),
            color.blue()
        ));
        let interaction = config.interaction_settings();
        set_combo_index(
            &self.zoom_shortcut,
            wheel_shortcut_index(interaction.zoom_shortcut()),
        );
        set_combo_index(
            &self.image_navigation_shortcut,
            wheel_shortcut_index(interaction.image_navigation_shortcut()),
        );
        set_combo_index(
            &self.image_pan_shortcut,
            drag_shortcut_index(interaction.image_pan_shortcut()),
        );
        set_combo_index(
            &self.window_move_shortcut,
            drag_shortcut_index(interaction.window_move_shortcut()),
        );
    }

    fn read_config(&self, base: &AppConfig) -> Result<AppConfig, String> {
        let mut config = base.clone();
        config.set_ui_language(
            ui_text::ui_language_from_index(combo_index(&self.ui_language)? as usize)
                .ok_or_else(|| "UI language could not be selected.".to_owned())?,
        );
        config.set_default_view_mode(
            view_mode_from_index(combo_index(&self.default_view_mode)?)
                .ok_or_else(|| "기본 보기 값을 선택할 수 없습니다.".to_owned())?,
        );
        config.set_scaling_quality(
            scaling_quality_from_index(combo_index(&self.scaling_quality)?)
                .ok_or_else(|| "스케일링 품질 값을 선택할 수 없습니다.".to_owned())?,
        );
        config.set_animation_autoplay(self.animation_autoplay.is_active());
        let mut status = StatusUiSettings::default();
        status.set_show_status_bar(self.show_status_bar.is_active());
        status.set_detailed_status_text(self.detailed_status_text.is_active());
        config.set_status_ui_settings(status);
        config.set_zoom_settings(self.read_zoom_settings()?);
        config.set_memory_policy_settings(
            self.read_memory_policy_settings(base.memory_policy_settings())?,
        );
        config.set_animation_timing_settings(self.read_animation_timing_settings()?);
        config.set_navigation_settings(self.read_navigation_settings()?);
        config.set_export_default_quality(parse_u8_entry(
            &self.export_quality,
            "JPEG 품질",
            MIN_EXPORT_QUALITY,
            MAX_EXPORT_QUALITY,
        )?);
        config.set_export_settings(self.read_export_settings(base.export_settings())?);
        config.set_interaction_settings(self.read_interaction_settings()?);
        Ok(config)
    }

    fn read_zoom_settings(&self) -> Result<ZoomSettings, String> {
        let min = parse_f64_entry(
            &self.min_zoom_scale,
            "최소 줌",
            MIN_CONFIG_MIN_ZOOM_SCALE,
            MAX_CONFIG_MIN_ZOOM_SCALE,
        )?;
        let max = parse_f64_entry(
            &self.max_zoom_scale,
            "최대 줌",
            MIN_CONFIG_MAX_ZOOM_SCALE,
            MAX_CONFIG_MAX_ZOOM_SCALE,
        )?;
        if min > max {
            return Err("최소 줌 값은 최대 줌 값보다 클 수 없습니다.".to_owned());
        }
        let step = parse_f64_entry(
            &self.zoom_step_factor,
            "줌 단계 배율",
            MIN_CONFIG_ZOOM_STEP_FACTOR,
            MAX_CONFIG_ZOOM_STEP_FACTOR,
        )?;
        Ok(ZoomSettings::new(min, max, step))
    }

    fn read_memory_policy_settings(
        &self,
        base: MemoryPolicySettings,
    ) -> Result<MemoryPolicySettings, String> {
        let large = parse_u64_entry(
            &self.large_image_pixel_threshold,
            "대용량 픽셀 기준",
            MIN_CONFIG_PIXEL_LIMIT,
            MAX_CONFIG_IMAGE_PIXELS,
        )?;
        let max_pixels = parse_u64_entry(
            &self.max_image_pixels,
            "최대 이미지 픽셀",
            MIN_CONFIG_PIXEL_LIMIT,
            MAX_CONFIG_IMAGE_PIXELS,
        )?;
        if large > max_pixels {
            return Err("대용량 픽셀 기준 값은 최대 이미지 픽셀 값보다 클 수 없습니다.".to_owned());
        }
        let preview = parse_u64_entry(
            &self.preview_max_pixels,
            "프리뷰 최대 픽셀",
            MIN_CONFIG_PIXEL_LIMIT,
            MAX_CONFIG_IMAGE_PIXELS,
        )?;
        if preview > max_pixels {
            return Err("프리뷰 최대 픽셀 값은 최대 이미지 픽셀 값보다 클 수 없습니다.".to_owned());
        }
        let oversample = parse_u32_entry(
            &self.preview_oversample,
            "프리뷰 배율",
            MIN_CONFIG_PREVIEW_OVERSAMPLE,
            MAX_CONFIG_PREVIEW_OVERSAMPLE,
        )?;
        let request_scale = parse_f64_entry(
            &self.full_resolution_request_scale,
            "전체 해상도 요청 배율",
            MIN_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
            MAX_CONFIG_FULL_RESOLUTION_REQUEST_SCALE,
        )?;
        let resident = parse_u32_entry(
            &self.max_resident_mib,
            "캐시 총량(MiB)",
            MIN_CONFIG_MEMORY_MIB,
            MAX_CONFIG_MEMORY_MIB,
        )?;
        let entry = parse_u32_entry(
            &self.max_cache_entry_mib,
            "캐시 항목 한도(MiB)",
            MIN_CONFIG_MEMORY_MIB,
            MAX_CONFIG_MEMORY_MIB,
        )?;
        if entry > resident {
            return Err("캐시 항목 한도(MiB)는 캐시 총량(MiB)보다 클 수 없습니다.".to_owned());
        }
        let entries = parse_usize_entry(
            &self.max_cache_entries,
            "캐시 항목 수",
            MIN_CONFIG_CACHE_ENTRIES,
            MAX_CONFIG_CACHE_ENTRIES,
        )?;
        let mut memory = base;
        memory.set_large_image_pixel_threshold(large);
        memory.set_max_image_pixels(max_pixels);
        memory.set_preview_max_pixels(preview);
        memory.set_preview_oversample(oversample);
        memory.set_full_resolution_request_scale(request_scale);
        memory.set_max_resident_mib(resident);
        memory.set_max_cache_entry_mib(entry);
        memory.set_max_cache_entries(entries);
        Ok(memory)
    }

    fn read_animation_timing_settings(&self) -> Result<AnimationTimingSettings, String> {
        let default = parse_u32_entry(
            &self.default_frame_delay_ms,
            "기본 프레임 지연",
            MIN_CONFIG_ANIMATION_DELAY_MS,
            MAX_CONFIG_ANIMATION_DELAY_MS,
        )?;
        let min = parse_u32_entry(
            &self.min_frame_delay_ms,
            "최소 프레임 지연",
            MIN_CONFIG_ANIMATION_DELAY_MS,
            MAX_CONFIG_ANIMATION_DELAY_MS,
        )?;
        let max = parse_u32_entry(
            &self.max_frame_delay_ms,
            "최대 프레임 지연",
            MIN_CONFIG_ANIMATION_DELAY_MS,
            MAX_CONFIG_ANIMATION_DELAY_MS,
        )?;
        if min > max {
            return Err("최소 프레임 지연 값은 최대 프레임 지연 값보다 클 수 없습니다.".to_owned());
        }
        if default < min || default > max {
            return Err(
                "기본 프레임 지연 값은 최소/최대 프레임 지연 범위 안에 있어야 합니다.".to_owned(),
            );
        }
        let mut timing = AnimationTimingSettings::default();
        timing.set_min_frame_delay_ms(min);
        timing.set_max_frame_delay_ms(max);
        timing.set_default_frame_delay_ms(default);
        Ok(timing)
    }

    fn read_navigation_settings(&self) -> Result<crate::domain::NavigationSettings, String> {
        let mut navigation = crate::domain::NavigationSettings::default();
        navigation.set_wrap_navigation(self.wrap_navigation.is_active());
        navigation.set_auto_skip_failed_navigation(self.auto_skip_navigation.is_active());
        navigation.set_max_navigation_attempts_per_command(parse_usize_entry(
            &self.navigation_attempts,
            "최대 시도 횟수",
            1,
            100,
        )?);
        Ok(navigation)
    }

    fn read_export_settings(
        &self,
        base: &crate::domain::ExportSettings,
    ) -> Result<crate::domain::ExportSettings, String> {
        let mut export = base.clone();
        export.set_default_export_format_policy(
            export_policy_from_index(combo_index(&self.export_format_policy)?)
                .ok_or_else(|| "내보내기 형식 값을 선택할 수 없습니다.".to_owned())?,
        );
        export.set_export_filename_suffix(read_export_suffix(&self.export_suffix)?);
        export.set_jpeg_alpha_background_rgb(read_rgb_color_entry(
            &self.jpeg_alpha_background_rgb,
            "JPEG 투명 배경 RGB",
        )?);
        Ok(export)
    }

    fn read_interaction_settings(&self) -> Result<InteractionSettings, String> {
        let mut interaction = InteractionSettings::default();
        interaction.set_zoom_shortcut(read_wheel_shortcut(&self.zoom_shortcut, "확대/축소")?);
        interaction.set_image_navigation_shortcut(read_wheel_shortcut(
            &self.image_navigation_shortcut,
            "이전/다음 이미지",
        )?);
        interaction
            .set_image_pan_shortcut(read_drag_shortcut(&self.image_pan_shortcut, "이미지 이동")?);
        interaction
            .set_window_move_shortcut(read_drag_shortcut(&self.window_move_shortcut, "창 이동")?);
        Ok(interaction)
    }
}

fn combo_box(items: &[&str]) -> gtk::ComboBoxText {
    let combo = gtk::ComboBoxText::new();
    for item in items {
        combo.append_text(item);
    }
    combo.set_active(Some(0));
    combo
}

fn settings_group(title: &str) -> (gtk::Frame, gtk::Grid) {
    let frame = gtk::Frame::new(Some(title));
    let grid = gtk::Grid::builder()
        .row_spacing(8)
        .column_spacing(8)
        .margin_top(12)
        .margin_bottom(12)
        .margin_start(12)
        .margin_end(12)
        .build();
    frame.set_child(Some(&grid));
    (frame, grid)
}

fn attach_labeled<W>(grid: &gtk::Grid, row: i32, label: &str, widget: &W)
where
    W: IsA<gtk::Widget>,
{
    let label = gtk::Label::new(Some(label));
    label.set_xalign(0.0);
    grid.attach(&label, 0, row, 1, 1);
    grid.attach(widget, 1, row, 1, 1);
}

fn set_combo_index(combo: &gtk::ComboBoxText, index: u32) {
    combo.set_active(Some(index));
}

fn combo_index(combo: &gtk::ComboBoxText) -> Result<u32, String> {
    combo
        .active()
        .ok_or_else(|| "콤보박스 값을 선택할 수 없습니다.".to_owned())
}

fn view_mode_index(view_mode: ViewMode) -> u32 {
    match view_mode {
        ViewMode::FitToWindow | ViewMode::ManualZoom => 0,
        ViewMode::ActualSize => 1,
    }
}

fn view_mode_from_index(index: u32) -> Option<ViewMode> {
    match index {
        0 => Some(ViewMode::FitToWindow),
        1 => Some(ViewMode::ActualSize),
        _ => None,
    }
}

fn scaling_quality_index(quality: ScalingQuality) -> u32 {
    match quality {
        ScalingQuality::Nearest => 0,
        ScalingQuality::Balanced => 1,
        ScalingQuality::HighQuality => 2,
    }
}

fn scaling_quality_from_index(index: u32) -> Option<ScalingQuality> {
    match index {
        0 => Some(ScalingQuality::Nearest),
        1 => Some(ScalingQuality::Balanced),
        2 => Some(ScalingQuality::HighQuality),
        _ => None,
    }
}

fn export_policy_index(policy: DefaultExportFormatPolicy) -> u32 {
    match policy {
        DefaultExportFormatPolicy::Source => 0,
        DefaultExportFormatPolicy::Png => 1,
        DefaultExportFormatPolicy::Jpeg => 2,
        DefaultExportFormatPolicy::Bmp => 3,
        DefaultExportFormatPolicy::Webp => 4,
        DefaultExportFormatPolicy::Ico => 5,
    }
}

fn export_policy_from_index(index: u32) -> Option<DefaultExportFormatPolicy> {
    match index {
        0 => Some(DefaultExportFormatPolicy::Source),
        1 => Some(DefaultExportFormatPolicy::Png),
        2 => Some(DefaultExportFormatPolicy::Jpeg),
        3 => Some(DefaultExportFormatPolicy::Bmp),
        4 => Some(DefaultExportFormatPolicy::Webp),
        5 => Some(DefaultExportFormatPolicy::Ico),
        _ => None,
    }
}

fn export_format_index(format: ExportFormat) -> u32 {
    match format {
        ExportFormat::Png => 0,
        ExportFormat::Jpeg => 1,
        ExportFormat::Bmp => 2,
        ExportFormat::Webp => 3,
        ExportFormat::Ico => 4,
    }
}

fn export_format_from_combo(combo: &gtk::ComboBoxText) -> Option<ExportFormat> {
    match combo.active()? {
        0 => Some(ExportFormat::Png),
        1 => Some(ExportFormat::Jpeg),
        2 => Some(ExportFormat::Bmp),
        3 => Some(ExportFormat::Webp),
        4 => Some(ExportFormat::Ico),
        _ => None,
    }
}

fn rotation_from_combo(combo: &gtk::ComboBoxText) -> Option<ImageRotation> {
    match combo.active()? {
        0 => Some(ImageRotation::Degrees0),
        1 => Some(ImageRotation::Degrees90),
        2 => Some(ImageRotation::Degrees180),
        3 => Some(ImageRotation::Degrees270),
        _ => None,
    }
}

fn wheel_shortcut_index(shortcut: MouseShortcut) -> u32 {
    match shortcut {
        MouseShortcut::MouseWheel => 0,
        MouseShortcut::CtrlMouseWheel => 1,
        MouseShortcut::LeftButtonDrag | MouseShortcut::CtrlLeftButtonDrag => 0,
    }
}

fn drag_shortcut_index(shortcut: MouseShortcut) -> u32 {
    match shortcut {
        MouseShortcut::LeftButtonDrag => 0,
        MouseShortcut::CtrlLeftButtonDrag => 1,
        MouseShortcut::MouseWheel | MouseShortcut::CtrlMouseWheel => 0,
    }
}

fn read_wheel_shortcut(combo: &gtk::ComboBoxText, label: &str) -> Result<MouseShortcut, String> {
    match combo_index(combo)? {
        0 => Ok(MouseShortcut::MouseWheel),
        1 => Ok(MouseShortcut::CtrlMouseWheel),
        _ => Err(format!("{label} 단축키 값을 선택할 수 없습니다.")),
    }
}

fn read_drag_shortcut(combo: &gtk::ComboBoxText, label: &str) -> Result<MouseShortcut, String> {
    match combo_index(combo)? {
        0 => Ok(MouseShortcut::LeftButtonDrag),
        1 => Ok(MouseShortcut::CtrlLeftButtonDrag),
        _ => Err(format!("{label} 단축키 값을 선택할 수 없습니다.")),
    }
}

fn parse_f64_entry(entry: &gtk::Entry, label: &str, min: f64, max: f64) -> Result<f64, String> {
    let value = entry
        .text()
        .trim()
        .parse::<f64>()
        .map_err(|_| format!("{label} 값은 숫자여야 합니다."))?;
    if !value.is_finite() || value < min || value > max {
        return Err(format!("{label} 값은 {min} 이상 {max} 이하이어야 합니다."));
    }
    Ok(value)
}

fn parse_u8_entry(entry: &gtk::Entry, label: &str, min: u8, max: u8) -> Result<u8, String> {
    parse_u32_entry(entry, label, u32::from(min), u32::from(max)).and_then(|value| {
        u8::try_from(value).map_err(|_| format!("{label} 값은 {min} 이상 {max} 이하이어야 합니다."))
    })
}

fn parse_u32_entry(entry: &gtk::Entry, label: &str, min: u32, max: u32) -> Result<u32, String> {
    let text = entry.text();
    let value =
        parse_u32_text(text.as_str()).ok_or_else(|| format!("{label} 값은 정수여야 합니다."))?;
    if value < min || value > max {
        return Err(format!("{label} 값은 {min} 이상 {max} 이하이어야 합니다."));
    }
    Ok(value)
}

fn parse_u32_text(text: &str) -> Option<u32> {
    text.trim().parse::<u32>().ok()
}

fn parse_usize_text(text: &str) -> Option<usize> {
    text.trim().parse::<usize>().ok()
}

fn parse_u64_entry(entry: &gtk::Entry, label: &str, min: u64, max: u64) -> Result<u64, String> {
    let value = entry
        .text()
        .trim()
        .parse::<u64>()
        .map_err(|_| format!("{label} 값은 정수여야 합니다."))?;
    if value < min || value > max {
        return Err(format!("{label} 값은 {min} 이상 {max} 이하이어야 합니다."));
    }
    Ok(value)
}

fn parse_usize_entry(
    entry: &gtk::Entry,
    label: &str,
    min: usize,
    max: usize,
) -> Result<usize, String> {
    let text = entry.text();
    let value =
        parse_usize_text(text.as_str()).ok_or_else(|| format!("{label} 값은 정수여야 합니다."))?;
    if value < min || value > max {
        return Err(format!("{label} 값은 {min} 이상 {max} 이하이어야 합니다."));
    }
    Ok(value)
}

fn read_export_suffix(entry: &gtk::Entry) -> Result<String, String> {
    let suffix = entry.text();
    match validate_export_filename_suffix(&suffix) {
        Ok(suffix) => Ok(suffix.to_owned()),
        Err(ExportFilenameSuffixValidationError::Empty) => Err(format!(
            "파일명 suffix 값은 비워 둘 수 없습니다. 기본값은 {DEFAULT_EXPORT_FILENAME_SUFFIX}입니다."
        )),
        Err(ExportFilenameSuffixValidationError::TooLong) => Err(format!(
            "파일명 suffix 값은 최대 {MAX_EXPORT_FILENAME_SUFFIX_CHARS}자까지 입력할 수 있습니다."
        )),
        Err(ExportFilenameSuffixValidationError::InvalidCharacter) => Err(
            "파일명 suffix 값에는 \\ / : * ? \" < > | 또는 제어 문자를 사용할 수 없습니다."
                .to_owned(),
        ),
    }
}

fn read_rgb_color_entry(entry: &gtk::Entry, label: &str) -> Result<RgbColor, String> {
    let text = entry.text();
    let mut parts = text.split(',');
    let red = parse_rgb_component(parts.next(), label, "R")?;
    let green = parse_rgb_component(parts.next(), label, "G")?;
    let blue = parse_rgb_component(parts.next(), label, "B")?;
    if parts.next().is_some() {
        return Err(format!("{label} 값은 R,G,B 형식으로 입력해야 합니다."));
    }
    Ok(RgbColor::new(red, green, blue))
}

fn parse_rgb_component(component: Option<&str>, label: &str, name: &str) -> Result<u8, String> {
    let Some(component) = component else {
        return Err(format!("{label} 값은 R,G,B 형식으로 입력해야 합니다."));
    };
    component
        .trim()
        .parse::<u8>()
        .map_err(|_| format!("{label}의 {name} 값은 0 이상 255 이하 정수여야 합니다."))
}

async fn show_modal_warning(parent: &impl IsA<gtk::Window>, message: &str) {
    let dialog = gtk::MessageDialog::builder()
        .transient_for(parent)
        .modal(true)
        .message_type(gtk::MessageType::Warning)
        .buttons(gtk::ButtonsType::Ok)
        .text(message)
        .build();
    let _ = dialog.run_future().await;
    dialog.close();
}

fn open_file_filters() -> gio::ListStore {
    let store = gio::ListStore::new::<gtk::FileFilter>();
    let filter = gtk::FileFilter::new();
    filter.set_name(Some(&format!(
        "Supported Images ({OPEN_FILE_FILTER_PATTERNS})"
    )));
    for suffix in OPEN_IMAGE_SUFFIXES {
        filter.add_suffix(suffix);
    }
    store.append(&filter);
    store
}

fn export_file_filters(format: ExportFormat) -> gio::ListStore {
    let store = gio::ListStore::new::<gtk::FileFilter>();
    let filter = gtk::FileFilter::new();
    let (label, _) = export_file_filter_label_and_pattern(format);
    filter.set_name(Some(label));
    for suffix in export_format_extensions(format) {
        filter.add_suffix(suffix);
    }
    store.append(&filter);
    store
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

fn gtk_initial_file_name(path: &Path) -> Option<String> {
    path.file_name()
        .map(|name| name.to_string_lossy().into_owned())
        .filter(|name| !name.is_empty())
}

fn file_drop_text_content_formats() -> gdk::ContentFormats {
    gdk::ContentFormats::new(FILE_DROP_MIME_TYPES)
}

fn drop_contains_text_file_payload(drop: &gdk::Drop) -> bool {
    drop_formats_contain_text_payload(&drop.formats())
}

fn drop_formats_contain_text_payload(formats: &gdk::ContentFormats) -> bool {
    FILE_DROP_MIME_TYPES
        .iter()
        .any(|mime_type| formats.contain_mime_type(mime_type))
}

fn file_drop_action_for_text_drop(drop: &gdk::Drop) -> gdk::DragAction {
    if drop_contains_text_file_payload(drop) {
        gdk::DragAction::COPY
    } else {
        gdk::DragAction::empty()
    }
}

fn first_supported_path_from_gio_file(file: &gio::File) -> Option<PathBuf> {
    let path = file.path()?;
    first_supported_image_path([path.as_path()]).map(Path::to_path_buf)
}

async fn first_supported_path_from_text_drop(
    drop: &gdk::Drop,
) -> Result<Option<PathBuf>, glib::Error> {
    let (stream, _mime_type) = drop
        .read_future(FILE_DROP_MIME_TYPES, glib::Priority::default())
        .await?;
    let text = read_drop_text_stream(stream).await?;
    Ok(first_supported_path_from_drop_text(&text))
}

async fn read_drop_text_stream(stream: gio::InputStream) -> Result<String, glib::Error> {
    let mut bytes = Vec::new();

    loop {
        let chunk = stream
            .read_bytes_future(DROP_TEXT_READ_CHUNK_BYTES, glib::Priority::default())
            .await?;
        let chunk = chunk.as_ref();
        if chunk.is_empty() {
            break;
        }
        if bytes.len().saturating_add(chunk.len()) > DROP_TEXT_READ_MAX_BYTES {
            break;
        }
        bytes.extend_from_slice(chunk);
    }

    Ok(String::from_utf8_lossy(&bytes).into_owned())
}

fn first_supported_path_from_uri_list(text: &str) -> Option<PathBuf> {
    let paths = text
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty() && !line.starts_with('#'))
        .filter_map(|uri| glib::filename_from_uri(uri).ok().map(|(path, _)| path))
        .collect::<Vec<_>>();
    first_supported_image_path(paths.iter().map(PathBuf::as_path)).map(Path::to_path_buf)
}

fn first_supported_path_from_drop_text(text: &str) -> Option<PathBuf> {
    if let Some(path) = first_supported_path_from_uri_list(text) {
        return Some(path);
    }

    let paths = text
        .lines()
        .filter_map(drop_text_line_to_local_path)
        .collect::<Vec<_>>();
    first_supported_image_path(paths.iter().map(PathBuf::as_path)).map(Path::to_path_buf)
}

fn drop_text_line_to_local_path(line: &str) -> Option<PathBuf> {
    let line = line.trim_matches('\0').trim();
    if line.is_empty() || line.starts_with('#') {
        return None;
    }
    if let Some(token) = line.split_whitespace().next() {
        if token != line {
            if let Ok((path, _)) = glib::filename_from_uri(token) {
                return Some(path);
            }
            let path = PathBuf::from(token);
            if path.is_absolute() {
                return Some(path);
            }
        }
    }
    if let Ok((path, _)) = glib::filename_from_uri(line) {
        return Some(path);
    }
    let path = PathBuf::from(line);
    path.is_absolute().then_some(path)
}

fn first_supported_path_from_file_list(file_list: &gdk::FileList) -> Option<PathBuf> {
    let paths = file_list
        .files()
        .into_iter()
        .filter_map(|file| file.path())
        .collect::<Vec<_>>();
    first_supported_image_path(paths.iter().map(PathBuf::as_path)).map(Path::to_path_buf)
}

fn unsupported_drop_message() -> String {
    format!(
        "드롭한 항목에서 지원하는 이미지 파일을 찾지 못했습니다.\n\n지원 형식: {SUPPORTED_FORMATS_TEXT}"
    )
}

const CAIRO_ARGB32_BYTES_PER_PIXEL: usize = 4;
const CAIRO_SURFACE_CACHE_VISIBLE_SOURCE_MULTIPLIER: usize = 16;

#[derive(Debug)]
struct CairoSurfaceCache {
    key: Option<CairoSurfaceCacheKey>,
    surface: Option<cairo::ImageSurface>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CairoSurfaceCacheKey {
    render_key: crate::app::RenderImageCacheKey,
    source_rect: CairoSurfaceSourceRect,
    scaling_quality: ScalingQuality,
}

#[derive(Debug, Clone)]
struct CairoPaintSurface {
    surface: cairo::ImageSurface,
    source_rect: CairoSurfaceSourceRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CairoSurfaceSourceRect {
    x: u32,
    y: u32,
    width: u32,
    height: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CairoPaintPlacement {
    source_rect: CairoSurfaceSourceRect,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CairoSurfaceDestRect {
    x: i32,
    y: i32,
    width: i32,
    height: i32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct CairoPaintAxisPlacement {
    source_start: u32,
    source_size: u32,
}

impl CairoSurfaceCache {
    fn new() -> Self {
        Self {
            key: None,
            surface: None,
        }
    }

    fn invalidate(&mut self) {
        self.key = None;
        self.surface = None;
    }

    fn release(&mut self) {
        self.invalidate();
    }

    fn surface_for_paint_pixel_rect(
        &mut self,
        render_key: crate::app::RenderImageCacheKey,
        pixels: &PixelImage,
        source_rect: CairoSurfaceSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<CairoPaintSurface> {
        if let Some(source_rect) = self.cache_source_rect_for_pixel_rect(
            render_key,
            pixels,
            source_rect,
            scaling_quality,
            max_cache_bytes,
        ) {
            return Some(CairoPaintSurface {
                surface: self.surface.as_ref()?.clone(),
                source_rect,
            });
        }

        let surface = cairo_surface_for_pixel_rect(pixels, source_rect)?;
        Some(CairoPaintSurface {
            surface,
            source_rect,
        })
    }

    fn cache_source_rect_for_pixel_rect(
        &mut self,
        render_key: crate::app::RenderImageCacheKey,
        pixels: &PixelImage,
        source_rect: CairoSurfaceSourceRect,
        scaling_quality: ScalingQuality,
        max_cache_bytes: usize,
    ) -> Option<CairoSurfaceSourceRect> {
        let full_source_rect = CairoSurfaceSourceRect::full(pixels.width(), pixels.height())?;
        if source_rect.width == 0
            || source_rect.height == 0
            || !full_source_rect.contains(source_rect)?
        {
            return None;
        }

        if full_source_rect.byte_len()? <= max_cache_bytes {
            let key = CairoSurfaceCacheKey {
                render_key,
                source_rect: full_source_rect,
                scaling_quality,
            };
            if self.key != Some(key) || self.surface.is_none() {
                self.surface = Some(cairo_surface_for_pixel_rect(pixels, full_source_rect)?);
                self.key = Some(key);
            }

            return Some(full_source_rect);
        }

        if let Some(key) = self.key {
            if key.render_key == render_key && key.scaling_quality == scaling_quality {
                let cached_source_rect = key.source_rect;
                if cached_source_rect.byte_len()? <= max_cache_bytes
                    && cached_source_rect.contains(source_rect)?
                    && self.surface.is_some()
                {
                    return Some(cached_source_rect);
                }
            }
        }

        let Some(cache_source_rect) = expanded_cairo_surface_cache_source_rect(
            full_source_rect,
            source_rect,
            max_cache_bytes,
        ) else {
            self.release();
            return None;
        };

        let key = CairoSurfaceCacheKey {
            render_key,
            source_rect: cache_source_rect,
            scaling_quality,
        };
        if self.key != Some(key) || self.surface.is_none() {
            self.surface = Some(cairo_surface_for_pixel_rect(pixels, cache_source_rect)?);
            self.key = Some(key);
        }

        Some(cache_source_rect)
    }
}

impl CairoSurfaceSourceRect {
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
        cairo_argb32_byte_len(self.width, self.height)
    }

    fn contains(self, other: Self) -> Option<bool> {
        let self_right = self.x.checked_add(self.width)?;
        let self_bottom = self.y.checked_add(self.height)?;
        let other_right = other.x.checked_add(other.width)?;
        let other_bottom = other.y.checked_add(other.height)?;

        Some(
            other.x >= self.x
                && other.y >= self.y
                && other_right <= self_right
                && other_bottom <= self_bottom,
        )
    }
}

fn cairo_paint_placement(
    rect: crate::domain::ImageDisplayRect,
    source_width: u32,
    source_height: u32,
    viewport: ViewportSize,
) -> Option<CairoPaintPlacement> {
    CairoSurfaceSourceRect::full(source_width, source_height)?;
    if rect.width() <= 0 || rect.height() <= 0 {
        return None;
    }

    let viewport_right = i32::try_from(viewport.width()).ok()?;
    let viewport_bottom = i32::try_from(viewport.height()).ok()?;
    let dest_left = rect.x();
    let dest_top = rect.y();
    let dest_right = dest_left.checked_add(rect.width())?;
    let dest_bottom = dest_top.checked_add(rect.height())?;
    let visible_left = dest_left.max(0);
    let visible_top = dest_top.max(0);
    let visible_right = dest_right.min(viewport_right);
    let visible_bottom = dest_bottom.min(viewport_bottom);
    if visible_left >= visible_right || visible_top >= visible_bottom {
        return None;
    }

    let one_to_one_width = i32::try_from(source_width).ok()?;
    let one_to_one_height = i32::try_from(source_height).ok()?;
    if rect.width() != one_to_one_width || rect.height() != one_to_one_height {
        let x_axis = scaled_cairo_surface_axis_placement(
            dest_left,
            rect.width(),
            visible_left,
            visible_right,
            source_width,
        )?;
        let y_axis = scaled_cairo_surface_axis_placement(
            dest_top,
            rect.height(),
            visible_top,
            visible_bottom,
            source_height,
        )?;
        return Some(CairoPaintPlacement {
            source_rect: CairoSurfaceSourceRect {
                x: x_axis.source_start,
                y: y_axis.source_start,
                width: x_axis.source_size,
                height: y_axis.source_size,
            },
        });
    }

    let source_x = u32::try_from(visible_left.checked_sub(dest_left)?).ok()?;
    let source_y = u32::try_from(visible_top.checked_sub(dest_top)?).ok()?;
    let width = u32::try_from(visible_right.checked_sub(visible_left)?).ok()?;
    let height = u32::try_from(visible_bottom.checked_sub(visible_top)?).ok()?;

    Some(CairoPaintPlacement {
        source_rect: CairoSurfaceSourceRect {
            x: source_x,
            y: source_y,
            width,
            height,
        },
    })
}

fn scaled_cairo_surface_axis_placement(
    dest_start: i32,
    dest_size: i32,
    visible_start: i32,
    visible_end: i32,
    source_size: u32,
) -> Option<CairoPaintAxisPlacement> {
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

    Some(CairoPaintAxisPlacement {
        source_start,
        source_size: source_end.checked_sub(source_start)?,
    })
}

fn cairo_surface_dest_rect_for_source_rect(
    rect: crate::domain::ImageDisplayRect,
    source_width: u32,
    source_height: u32,
    source_rect: CairoSurfaceSourceRect,
) -> Option<CairoSurfaceDestRect> {
    let x_axis = cairo_surface_dest_axis_for_source_rect(
        rect.x(),
        rect.width(),
        source_rect.x,
        source_rect.width,
        source_width,
    )?;
    let y_axis = cairo_surface_dest_axis_for_source_rect(
        rect.y(),
        rect.height(),
        source_rect.y,
        source_rect.height,
        source_height,
    )?;

    Some(CairoSurfaceDestRect {
        x: x_axis.0,
        y: y_axis.0,
        width: x_axis.1,
        height: y_axis.1,
    })
}

fn cairo_surface_dest_axis_for_source_rect(
    dest_start: i32,
    dest_size: i32,
    source_start: u32,
    source_size: u32,
    full_source_size: u32,
) -> Option<(i32, i32)> {
    if dest_size <= 0 || source_size == 0 || full_source_size == 0 {
        return None;
    }
    let source_end = source_start.checked_add(source_size)?;
    if source_end > full_source_size {
        return None;
    }

    let dest_size = u32::try_from(dest_size).ok()?;
    let dest_offset_start = floor_mul_div_i32(source_start, dest_size, full_source_size)?;
    let dest_offset_end = ceil_mul_div_i32(source_end, dest_size, full_source_size)?;
    let mapped_dest_start = dest_start.checked_add(dest_offset_start)?;
    let mapped_dest_end = dest_start.checked_add(dest_offset_end)?;
    if mapped_dest_start >= mapped_dest_end {
        return None;
    }

    Some((
        mapped_dest_start,
        mapped_dest_end.checked_sub(mapped_dest_start)?,
    ))
}

fn cairo_surface_cache_budget_for_paint_placement(
    max_cache_bytes: usize,
    full_source_rect: CairoSurfaceSourceRect,
    placement: CairoPaintPlacement,
) -> usize {
    if placement.source_rect == full_source_rect {
        return placement
            .source_rect
            .byte_len()
            .map_or(0, |source_bytes| max_cache_bytes.min(source_bytes));
    }

    cairo_surface_cache_budget_for_placement(max_cache_bytes, full_source_rect, placement)
}

fn cairo_surface_cache_budget_for_placement(
    max_cache_bytes: usize,
    full_source_rect: CairoSurfaceSourceRect,
    placement: CairoPaintPlacement,
) -> usize {
    let Some(source_bytes) = placement.source_rect.byte_len() else {
        return 0;
    };
    let Some(full_source_bytes) = full_source_rect.byte_len() else {
        return 0;
    };
    let expanded_budget =
        source_bytes.saturating_mul(CAIRO_SURFACE_CACHE_VISIBLE_SOURCE_MULTIPLIER);

    max_cache_bytes.min(full_source_bytes).min(expanded_budget)
}

fn expanded_cairo_surface_cache_source_rect(
    full_source_rect: CairoSurfaceSourceRect,
    source_rect: CairoSurfaceSourceRect,
    max_cache_bytes: usize,
) -> Option<CairoSurfaceSourceRect> {
    if source_rect.byte_len()? > max_cache_bytes {
        return None;
    }

    let max_pixels = max_cache_bytes / CAIRO_ARGB32_BYTES_PER_PIXEL;
    let full_width = usize::try_from(full_source_rect.width).ok()?;
    let full_height = usize::try_from(full_source_rect.height).ok()?;
    let source_width = usize::try_from(source_rect.width).ok()?;
    let source_height = usize::try_from(source_rect.height).ok()?;
    if max_pixels == 0 || source_width == 0 || source_height == 0 {
        return None;
    }

    let full_pixels = full_width.checked_mul(full_height)?;
    if full_pixels <= max_pixels {
        return Some(full_source_rect);
    }

    let source_pixels = source_width.checked_mul(source_height)?;
    let scale = ((max_pixels as f64) / (source_pixels as f64)).sqrt();
    if !scale.is_finite() || scale < 1.0 {
        return None;
    }

    let cache_width = ((source_width as f64) * scale).floor() as usize;
    let cache_height = ((source_height as f64) * scale).floor() as usize;
    let mut cache_width = cache_width.clamp(source_width, full_width);
    let mut cache_height = cache_height.clamp(source_height, full_height);
    while cache_width.checked_mul(cache_height)? > max_pixels {
        if cache_width.saturating_sub(source_width) >= cache_height.saturating_sub(source_height)
            && cache_width > source_width
        {
            cache_width = cache_width.saturating_sub(1);
        } else if cache_height > source_height {
            cache_height = cache_height.saturating_sub(1);
        } else {
            return None;
        }
    }
    if cache_width == full_width {
        cache_height = full_height.min(max_pixels.checked_div(cache_width)?);
    }
    if cache_height == full_height {
        cache_width = full_width.min(max_pixels.checked_div(cache_height)?);
    }
    if cache_width < source_width || cache_height < source_height {
        return None;
    }

    let cache_width = u32::try_from(cache_width).ok()?;
    let cache_height = u32::try_from(cache_height).ok()?;
    let cache_x = expanded_cairo_surface_cache_axis_start(
        full_source_rect.x,
        full_source_rect.width,
        source_rect.x,
        source_rect.width,
        cache_width,
    )?;
    let cache_y = expanded_cairo_surface_cache_axis_start(
        full_source_rect.y,
        full_source_rect.height,
        source_rect.y,
        source_rect.height,
        cache_height,
    )?;

    Some(CairoSurfaceSourceRect {
        x: cache_x,
        y: cache_y,
        width: cache_width,
        height: cache_height,
    })
}

fn expanded_cairo_surface_cache_axis_start(
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

fn cairo_argb32_byte_len(width: u32, height: u32) -> Option<usize> {
    let pixels = width.checked_mul(height)?;
    let bytes = pixels.checked_mul(CAIRO_ARGB32_BYTES_PER_PIXEL as u32)?;
    usize::try_from(bytes).ok()
}

fn cairo_surface_for_pixel_rect(
    pixels: &PixelImage,
    source_rect: CairoSurfaceSourceRect,
) -> Option<cairo::ImageSurface> {
    let full_source_rect = CairoSurfaceSourceRect::full(pixels.width(), pixels.height())?;
    if !full_source_rect.contains(source_rect)? {
        return None;
    }
    if pixels.expected_byte_len()? != pixels.pixels().len() {
        return None;
    }

    let width = i32::try_from(source_rect.width).ok()?;
    let height = i32::try_from(source_rect.height).ok()?;
    let mut surface = cairo::ImageSurface::create(cairo::Format::ARgb32, width, height).ok()?;
    let stride = usize::try_from(surface.stride()).ok()?;
    {
        let mut data = surface.data().ok()?;
        write_pixel_rect_to_cairo_argb32(pixels, source_rect, stride, &mut data)?;
    }
    surface.mark_dirty();
    Some(surface)
}

fn write_pixel_rect_to_cairo_argb32(
    pixels: &PixelImage,
    source_rect: CairoSurfaceSourceRect,
    stride: usize,
    destination: &mut [u8],
) -> Option<()> {
    let row_bytes = usize::try_from(source_rect.width)
        .ok()?
        .checked_mul(CAIRO_ARGB32_BYTES_PER_PIXEL)?;
    if stride < row_bytes {
        return None;
    }
    let height = usize::try_from(source_rect.height).ok()?;
    if destination.len() < stride.checked_mul(height)? {
        return None;
    }

    for y in 0..height {
        let source_y = source_rect.y.checked_add(u32::try_from(y).ok()?)?;
        let destination_start = y.checked_mul(stride)?;
        let destination_row = &mut destination[destination_start..destination_start + row_bytes];
        match pixels {
            PixelImage::Rgb8(image) => {
                let range = pixel_row_range(
                    image.width(),
                    source_rect.x,
                    source_y,
                    source_rect.width,
                    crate::domain::RGB8_BYTES_PER_PIXEL,
                )?;
                write_rgb8_row_to_cairo_argb32(&image.pixels()[range], destination_row)?;
            }
            PixelImage::Rgba8(image) => {
                let range = pixel_row_range(
                    image.width(),
                    source_rect.x,
                    source_y,
                    source_rect.width,
                    crate::domain::RGBA8_BYTES_PER_PIXEL,
                )?;
                write_rgba8_row_to_cairo_argb32(&image.pixels()[range], destination_row)?;
            }
            PixelImage::Bgra8(image) => {
                let range = pixel_row_range(
                    image.width(),
                    source_rect.x,
                    source_y,
                    source_rect.width,
                    crate::domain::BGRA8_BYTES_PER_PIXEL,
                )?;
                write_bgra8_row_to_cairo_argb32(&image.pixels()[range], destination_row)?;
            }
        }
    }

    Some(())
}

fn pixel_row_range(
    image_width: u32,
    x: u32,
    y: u32,
    width: u32,
    bytes_per_pixel: usize,
) -> Option<std::ops::Range<usize>> {
    let image_width = usize::try_from(image_width).ok()?;
    let x = usize::try_from(x).ok()?;
    let y = usize::try_from(y).ok()?;
    let width = usize::try_from(width).ok()?;
    let start_pixel = y.checked_mul(image_width)?.checked_add(x)?;
    let start = start_pixel.checked_mul(bytes_per_pixel)?;
    let len = width.checked_mul(bytes_per_pixel)?;
    let end = start.checked_add(len)?;
    Some(start..end)
}

fn texture_for_pixels(pixels: &PixelImage) -> Option<gdk::MemoryTexture> {
    let rgba = pixels.to_rgba8()?;
    let width = i32::try_from(rgba.width()).ok()?;
    let height = i32::try_from(rgba.height()).ok()?;
    let stride = usize::try_from(rgba.width()).ok()?.checked_mul(4)?;
    let bytes = glib::Bytes::from_owned(rgba8_flattened_over_white_bytes(&rgba)?);
    Some(gdk::MemoryTexture::new(
        width,
        height,
        gdk::MemoryFormat::R8g8b8a8,
        &bytes,
        stride,
    ))
}

fn rgba8_flattened_over_white_bytes(rgba: &crate::domain::Rgba8Image) -> Option<Vec<u8>> {
    let width = usize::try_from(rgba.width()).ok()?;
    let height = usize::try_from(rgba.height()).ok()?;
    let len = width.checked_mul(height)?.checked_mul(4)?;
    if rgba.pixels().len() != len {
        return None;
    }
    let mut flattened = Vec::new();
    flattened.try_reserve_exact(len).ok()?;
    for pixel in rgba.pixels().chunks_exact(4) {
        let alpha = u16::from(pixel[3]);
        flattened.push(blend_channel_over_white(pixel[0], alpha));
        flattened.push(blend_channel_over_white(pixel[1], alpha));
        flattened.push(blend_channel_over_white(pixel[2], alpha));
        flattened.push(255);
    }
    Some(flattened)
}

fn app_icon_textures() -> Vec<gdk::Texture> {
    let Ok(icon) = image::load_from_memory_with_format(APP_ICON_BYTES, image::ImageFormat::Ico)
    else {
        return Vec::new();
    };
    let rgba = icon.into_rgba8();
    let width = match i32::try_from(rgba.width()) {
        Ok(width) => width,
        Err(_) => return Vec::new(),
    };
    let height = match i32::try_from(rgba.height()) {
        Ok(height) => height,
        Err(_) => return Vec::new(),
    };
    let stride = match usize::try_from(rgba.width())
        .ok()
        .and_then(|width| width.checked_mul(4))
    {
        Some(stride) => stride,
        None => return Vec::new(),
    };
    let bytes = glib::Bytes::from_owned(rgba.into_raw());
    let texture =
        gdk::MemoryTexture::new(width, height, gdk::MemoryFormat::R8g8b8a8, &bytes, stride);
    vec![texture.upcast()]
}

#[cfg(test)]
fn write_rgba_to_cairo_argb32(
    rgba: &[u8],
    width: u32,
    height: u32,
    stride: usize,
    destination: &mut [u8],
) -> Option<()> {
    let width = usize::try_from(width).ok()?;
    let height = usize::try_from(height).ok()?;
    let row_bytes = width.checked_mul(4)?;
    if rgba.len() != row_bytes.checked_mul(height)? {
        return None;
    }
    if stride < row_bytes {
        return None;
    }
    let required_len = if height == 0 {
        0
    } else {
        stride
            .checked_mul(height.saturating_sub(1))?
            .checked_add(row_bytes)?
    };
    if destination.len() < required_len {
        return None;
    }
    for y in 0..height {
        let source_row = &rgba[y * row_bytes..(y + 1) * row_bytes];
        let dest_row = &mut destination[y * stride..y * stride + row_bytes];
        write_rgba8_row_to_cairo_argb32(source_row, dest_row)?;
    }
    Some(())
}

fn write_rgb8_row_to_cairo_argb32(rgb: &[u8], destination: &mut [u8]) -> Option<()> {
    let pixel_count = rgb.len().checked_div(crate::domain::RGB8_BYTES_PER_PIXEL)?;
    if pixel_count.checked_mul(crate::domain::RGB8_BYTES_PER_PIXEL) != Some(rgb.len())
        || destination.len() != pixel_count.checked_mul(CAIRO_ARGB32_BYTES_PER_PIXEL)?
    {
        return None;
    }

    for (source, dest) in rgb.chunks_exact(3).zip(destination.chunks_exact_mut(4)) {
        dest[0] = source[2];
        dest[1] = source[1];
        dest[2] = source[0];
        dest[3] = 255;
    }

    Some(())
}

fn write_rgba8_row_to_cairo_argb32(rgba: &[u8], destination: &mut [u8]) -> Option<()> {
    let pixel_count = rgba
        .len()
        .checked_div(crate::domain::RGBA8_BYTES_PER_PIXEL)?;
    if pixel_count.checked_mul(crate::domain::RGBA8_BYTES_PER_PIXEL) != Some(rgba.len())
        || destination.len() != pixel_count.checked_mul(CAIRO_ARGB32_BYTES_PER_PIXEL)?
    {
        return None;
    }

    for (source, dest) in rgba.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
        let alpha = u16::from(source[3]);
        dest[0] = blend_channel_over_white(source[2], alpha);
        dest[1] = blend_channel_over_white(source[1], alpha);
        dest[2] = blend_channel_over_white(source[0], alpha);
        dest[3] = 255;
    }

    Some(())
}

fn write_bgra8_row_to_cairo_argb32(bgra: &[u8], destination: &mut [u8]) -> Option<()> {
    let pixel_count = bgra
        .len()
        .checked_div(crate::domain::BGRA8_BYTES_PER_PIXEL)?;
    if pixel_count.checked_mul(crate::domain::BGRA8_BYTES_PER_PIXEL) != Some(bgra.len())
        || destination.len() != pixel_count.checked_mul(CAIRO_ARGB32_BYTES_PER_PIXEL)?
    {
        return None;
    }

    for (source, dest) in bgra.chunks_exact(4).zip(destination.chunks_exact_mut(4)) {
        let alpha = u16::from(source[3]);
        dest[0] = blend_channel_over_white(source[0], alpha);
        dest[1] = blend_channel_over_white(source[1], alpha);
        dest[2] = blend_channel_over_white(source[2], alpha);
        dest[3] = 255;
    }

    Some(())
}

fn blend_channel_over_white(channel: u8, alpha: u16) -> u8 {
    let inverse_alpha = 255u16.saturating_sub(alpha);
    ((u16::from(channel) * alpha + 255 * inverse_alpha) / 255) as u8
}

#[cfg(test)]
mod tests {
    use std::sync::{Arc, Condvar, Mutex};

    use super::*;

    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    enum ExpectedContextMenuEntry {
        Command {
            command: Command,
            requires_image: bool,
        },
        Separator,
    }

    #[test]
    fn context_menu_matches_win32_reference_order() {
        let expected = [
            ExpectedContextMenuEntry::Command {
                command: Command::OpenImage,
                requires_image: false,
            },
            ExpectedContextMenuEntry::Command {
                command: Command::ExportImage,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Command {
                command: Command::CopyImageToClipboard,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Separator,
            ExpectedContextMenuEntry::Command {
                command: Command::ActualSize,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Command {
                command: Command::FitToWindow,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Command {
                command: Command::RotateClockwise,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Command {
                command: Command::RotateCounterClockwise,
                requires_image: true,
            },
            ExpectedContextMenuEntry::Separator,
            ExpectedContextMenuEntry::Command {
                command: Command::ToggleFullscreen,
                requires_image: false,
            },
            ExpectedContextMenuEntry::Separator,
            ExpectedContextMenuEntry::Command {
                command: Command::OpenSettings,
                requires_image: false,
            },
        ];
        let actual = context_menu_entries()
            .iter()
            .map(|entry| match entry {
                ContextMenuEntry::Command {
                    command,
                    requires_image,
                } => ExpectedContextMenuEntry::Command {
                    command: *command,
                    requires_image: *requires_image,
                },
                ContextMenuEntry::Separator => ExpectedContextMenuEntry::Separator,
            })
            .collect::<Vec<_>>();

        assert_eq!(actual, expected);
        assert_eq!(
            ui_text::context_menu_label(UiLanguage::English, Command::OpenImage),
            "Open..."
        );
        assert_eq!(
            ui_text::context_menu_label(UiLanguage::Korean, Command::OpenImage),
            "열기..."
        );
    }

    #[test]
    fn gdk_key_mapping_covers_shared_command_keys() {
        let cases = [
            (gdk::Key::o, KeyCode::O),
            (gdk::Key::O, KeyCode::O),
            (gdk::Key::s, KeyCode::S),
            (gdk::Key::c, KeyCode::C),
            (gdk::Key::Right, KeyCode::Right),
            (gdk::Key::Left, KeyCode::Left),
            (gdk::Key::Page_Down, KeyCode::PageDown),
            (gdk::Key::Page_Up, KeyCode::PageUp),
            (gdk::Key::BackSpace, KeyCode::Backspace),
            (gdk::Key::space, KeyCode::Space),
            (gdk::Key::p, KeyCode::P),
            (gdk::Key::bracketleft, KeyCode::BracketLeft),
            (gdk::Key::bracketright, KeyCode::BracketRight),
            (gdk::Key::Home, KeyCode::Home),
            (gdk::Key::equal, KeyCode::Equals),
            (gdk::Key::plus, KeyCode::Equals),
            (gdk::Key::KP_Add, KeyCode::NumpadAdd),
            (gdk::Key::minus, KeyCode::Minus),
            (gdk::Key::KP_Subtract, KeyCode::NumpadSubtract),
            (gdk::Key::_1, KeyCode::Digit1),
            (gdk::Key::exclam, KeyCode::Digit1),
            (gdk::Key::_0, KeyCode::Digit0),
            (gdk::Key::parenright, KeyCode::Digit0),
            (gdk::Key::underscore, KeyCode::Minus),
            (gdk::Key::r, KeyCode::R),
            (gdk::Key::braceleft, KeyCode::BracketLeft),
            (gdk::Key::braceright, KeyCode::BracketRight),
            (gdk::Key::F11, KeyCode::F11),
            (gdk::Key::Return, KeyCode::Enter),
            (gdk::Key::KP_Enter, KeyCode::Enter),
            (gdk::Key::Escape, KeyCode::Escape),
            (gdk::Key::q, KeyCode::Q),
            (gdk::Key::F4, KeyCode::F4),
        ];

        for (gdk_key, key_code) in cases {
            assert_eq!(key_code_from_gdk_key(gdk_key), Some(key_code));
        }
    }

    #[test]
    fn gdk_modifier_mapping_keeps_command_contract_modifiers() {
        let modifiers = key_modifiers_from_gdk(
            gdk::ModifierType::CONTROL_MASK
                | gdk::ModifierType::SHIFT_MASK
                | gdk::ModifierType::ALT_MASK,
        );

        assert!(modifiers.control());
        assert!(modifiers.shift());
        assert!(modifiers.alt());
    }

    #[test]
    fn gtk_context_menu_keys_match_windows_keyboard_invocation() {
        let none = gdk::ModifierType::empty();
        let shift = gdk::ModifierType::SHIFT_MASK;
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let alt = gdk::ModifierType::ALT_MASK;

        assert!(context_menu_key_from_gdk(gdk::Key::Menu, none));
        assert!(context_menu_key_from_gdk(gdk::Key::Menu, shift));
        assert!(context_menu_key_from_gdk(gdk::Key::F10, shift));
        assert!(!context_menu_key_from_gdk(gdk::Key::F10, none));
        assert!(!context_menu_key_from_gdk(gdk::Key::F10, ctrl | shift));
        assert!(!context_menu_key_from_gdk(gdk::Key::Menu, alt));
    }

    #[test]
    fn gtk_application_is_non_unique_like_win32_launches() {
        assert!(APPLICATION_FLAGS.contains(gio::ApplicationFlags::NON_UNIQUE));
    }

    #[test]
    fn gtk_file_dialog_cancel_errors_are_not_reported_as_failures() {
        let cancelled = glib::Error::new(gtk::DialogError::Cancelled, "cancelled");
        let dismissed = glib::Error::new(gtk::DialogError::Dismissed, "dismissed");
        let gio_cancelled = glib::Error::new(gio::IOErrorEnum::Cancelled, "task cancelled");
        let failed = glib::Error::new(gtk::DialogError::Failed, "failed");

        assert!(gtk_file_dialog_error_is_cancelled(&cancelled));
        assert!(gtk_file_dialog_error_is_cancelled(&dismissed));
        assert!(gtk_file_dialog_error_is_cancelled(&gio_cancelled));
        assert!(!gtk_file_dialog_error_is_cancelled(&failed));
    }

    #[test]
    fn decode_worker_limits_match_win32_reference() {
        assert_eq!(MAX_IN_FLIGHT_DECODE_WORKERS, 3);
        assert_eq!(
            MAX_IN_FLIGHT_FOLDER_SCAN_WORKERS,
            MAX_IN_FLIGHT_DECODE_WORKERS
        );
        assert_eq!(MAX_NAVIGATION_PRELOAD_WORKERS, 2);
    }

    #[test]
    fn numeric_entry_text_parsing_trims_like_win32_dialogs() {
        assert_eq!(parse_u32_text("90"), Some(90));
        assert_eq!(parse_u32_text(" 90 "), Some(90));
        assert_eq!(parse_u32_text("\t400\n"), Some(400));
        assert_eq!(parse_u32_text(""), None);
        assert_eq!(parse_u32_text("  "), None);
        assert_eq!(parse_u32_text("-1"), None);

        assert_eq!(parse_usize_text("2"), Some(2));
        assert_eq!(parse_usize_text(" 2 "), Some(2));
        assert_eq!(parse_usize_text("\t8\n"), Some(8));
        assert_eq!(parse_usize_text(""), None);
        assert_eq!(parse_usize_text("  "), None);
        assert_eq!(parse_usize_text("-1"), None);
    }

    #[test]
    fn gtk_initial_file_name_keeps_lossy_non_utf8_names() {
        use std::ffi::OsString;
        use std::os::unix::ffi::OsStringExt;

        assert_eq!(
            gtk_initial_file_name(Path::new("/tmp/export.png")),
            Some("export.png".to_owned())
        );

        let mut path = PathBuf::from("/tmp");
        path.push(OsString::from_vec(vec![b'p', 0xff, b'.', b'p', b'n', b'g']));

        assert_eq!(
            gtk_initial_file_name(&path),
            Some(format!("p{}.png", char::REPLACEMENT_CHARACTER))
        );
    }

    #[test]
    fn gtk_key_events_resolve_to_windows_shortcut_commands() {
        let none = gdk::ModifierType::empty();
        let ctrl = gdk::ModifierType::CONTROL_MASK;
        let shift = gdk::ModifierType::SHIFT_MASK;
        let alt = gdk::ModifierType::ALT_MASK;
        let static_context = CommandContext::StaticImage;
        let animation_context = CommandContext::AnimationImage;
        let cases = [
            (gdk::Key::o, ctrl, static_context, Some(Command::OpenImage)),
            (
                gdk::Key::s,
                ctrl,
                static_context,
                Some(Command::ExportImage),
            ),
            (
                gdk::Key::s,
                ctrl | shift,
                static_context,
                Some(Command::ExportImage),
            ),
            (
                gdk::Key::c,
                ctrl,
                static_context,
                Some(Command::CopyImageToClipboard),
            ),
            (
                gdk::Key::Right,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Next)),
            ),
            (
                gdk::Key::Page_Down,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Next)),
            ),
            (
                gdk::Key::space,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Next)),
            ),
            (
                gdk::Key::space,
                none,
                animation_context,
                Some(Command::Animation(AnimationCommand::TogglePlayback)),
            ),
            (
                gdk::Key::Left,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Previous)),
            ),
            (
                gdk::Key::BackSpace,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Previous)),
            ),
            (
                gdk::Key::Page_Up,
                none,
                static_context,
                Some(Command::Navigate(ImageNavigationDirection::Previous)),
            ),
            (
                gdk::Key::p,
                none,
                static_context,
                Some(Command::Animation(AnimationCommand::TogglePlayback)),
            ),
            (
                gdk::Key::bracketleft,
                none,
                static_context,
                Some(Command::Animation(AnimationCommand::StepFrame(
                    crate::domain::AnimationFrameStepDirection::Previous,
                ))),
            ),
            (
                gdk::Key::bracketright,
                none,
                static_context,
                Some(Command::Animation(AnimationCommand::StepFrame(
                    crate::domain::AnimationFrameStepDirection::Next,
                ))),
            ),
            (
                gdk::Key::Home,
                none,
                static_context,
                Some(Command::Animation(AnimationCommand::FirstFrame)),
            ),
            (gdk::Key::plus, shift, static_context, Some(Command::ZoomIn)),
            (
                gdk::Key::KP_Add,
                none,
                static_context,
                Some(Command::ZoomIn),
            ),
            (
                gdk::Key::minus,
                none,
                static_context,
                Some(Command::ZoomOut),
            ),
            (
                gdk::Key::KP_Subtract,
                none,
                static_context,
                Some(Command::ZoomOut),
            ),
            (
                gdk::Key::_1,
                none,
                static_context,
                Some(Command::ActualSize),
            ),
            (
                gdk::Key::_0,
                none,
                static_context,
                Some(Command::FitToWindow),
            ),
            (
                gdk::Key::r,
                none,
                static_context,
                Some(Command::RotateClockwise),
            ),
            (
                gdk::Key::R,
                shift,
                static_context,
                Some(Command::RotateCounterClockwise),
            ),
            (
                gdk::Key::F11,
                none,
                static_context,
                Some(Command::ToggleFullscreen),
            ),
            (
                gdk::Key::KP_Enter,
                alt,
                static_context,
                Some(Command::ToggleFullscreen),
            ),
            (
                gdk::Key::Escape,
                none,
                static_context,
                Some(Command::ExitFullscreenOrQuit),
            ),
            (gdk::Key::q, none, static_context, Some(Command::Quit)),
            (gdk::Key::F4, alt, static_context, Some(Command::Quit)),
            (gdk::Key::o, none, static_context, None),
            (gdk::Key::c, ctrl | shift, static_context, None),
            (gdk::Key::Right, ctrl, static_context, None),
        ];

        for (key, modifiers, context, expected) in cases {
            let actual = key_code_from_gdk_key(key)
                .map(|key| KeyInput::new(key, key_modifiers_from_gdk(modifiers)))
                .and_then(|input| command_for_key_input_with_context(input, context));
            assert_eq!(actual, expected, "key={key:?} modifiers={modifiers:?}");
        }
    }

    #[test]
    fn mouse_event_matching_accepts_configured_plain_and_ctrl_shortcuts_only() {
        let none = KeyModifiers::NONE;
        let ctrl = KeyModifiers::new(true, false, false);
        let shift = KeyModifiers::new(false, true, false);
        let ctrl_shift = KeyModifiers::new(true, true, false);

        assert!(mouse_event_matches(MouseShortcut::MouseWheel, none));
        assert!(mouse_event_matches(MouseShortcut::LeftButtonDrag, none));
        assert!(!mouse_event_matches(MouseShortcut::MouseWheel, ctrl));
        assert!(!mouse_event_matches(MouseShortcut::LeftButtonDrag, shift));

        assert!(mouse_event_matches(MouseShortcut::CtrlMouseWheel, ctrl));
        assert!(mouse_event_matches(MouseShortcut::CtrlLeftButtonDrag, ctrl));
        assert!(!mouse_event_matches(MouseShortcut::CtrlMouseWheel, none));
        assert!(!mouse_event_matches(
            MouseShortcut::CtrlLeftButtonDrag,
            ctrl_shift
        ));
    }

    #[test]
    fn gtk_scroll_delta_uses_windows_wheel_step_semantics() {
        assert_eq!(signed_scroll_steps_from_gtk_delta(-1.0), 1);
        assert_eq!(signed_scroll_steps_from_gtk_delta(1.0), -1);
        assert_eq!(signed_scroll_steps_from_gtk_delta(-2.4), 2);
        assert_eq!(signed_scroll_steps_from_gtk_delta(2.6), -3);
        assert_eq!(signed_scroll_steps_from_gtk_delta(0.0), 0);

        assert_eq!(wheel_zoom_factor_from_steps(2.0, 1), Some(2.0));
        assert_eq!(wheel_zoom_factor_from_steps(2.0, -1), Some(0.5));
        assert_eq!(wheel_zoom_factor_from_steps(2.0, 2), Some(4.0));
        assert_eq!(wheel_zoom_factor_from_steps(2.0, 0), None);
        assert_eq!(wheel_zoom_factor_from_steps(0.0, 1), None);
        assert_eq!(wheel_zoom_factor_from_steps(f64::NAN, 1), None);

        assert_eq!(
            wheel_navigation_direction_from_steps(1),
            Some(ImageNavigationDirection::Previous)
        );
        assert_eq!(
            wheel_navigation_direction_from_steps(-1),
            Some(ImageNavigationDirection::Next)
        );
        assert_eq!(wheel_navigation_direction_from_steps(0), None);
    }

    #[test]
    fn shortcut_combo_indices_preserve_windows_default_semantics() {
        assert_eq!(wheel_shortcut_index(MouseShortcut::MouseWheel), 0);
        assert_eq!(wheel_shortcut_index(MouseShortcut::CtrlMouseWheel), 1);
        assert_eq!(wheel_shortcut_index(MouseShortcut::LeftButtonDrag), 0);
        assert_eq!(wheel_shortcut_index(MouseShortcut::CtrlLeftButtonDrag), 0);

        assert_eq!(drag_shortcut_index(MouseShortcut::LeftButtonDrag), 0);
        assert_eq!(drag_shortcut_index(MouseShortcut::CtrlLeftButtonDrag), 1);
        assert_eq!(drag_shortcut_index(MouseShortcut::MouseWheel), 0);
        assert_eq!(drag_shortcut_index(MouseShortcut::CtrlMouseWheel), 0);
    }

    #[test]
    fn export_format_indices_match_export_dialog_order() {
        assert_eq!(EXPORT_FORMAT_LABELS, ["PNG", "JPEG", "BMP", "WebP", "ICO"]);
        assert_eq!(
            ui_text::export_rotation_labels(UiLanguage::Korean),
            ["0도", "90도 시계 방향", "180도", "270도 시계 방향"]
        );
        assert_eq!(
            ui_text::export_rotation_labels(UiLanguage::English),
            ["0 deg", "90 deg clockwise", "180 deg", "270 deg clockwise"]
        );
        assert_eq!(export_format_index(ExportFormat::Png), 0);
        assert_eq!(export_format_index(ExportFormat::Jpeg), 1);
        assert_eq!(export_format_index(ExportFormat::Bmp), 2);
        assert_eq!(export_format_index(ExportFormat::Webp), 3);
        assert_eq!(export_format_index(ExportFormat::Ico), 4);
    }

    #[test]
    fn export_dialog_size_values_follow_win32_resize_rules() {
        let source = ImageSize::new(400, 200);

        assert_eq!(
            export_target_size_from_dialog_values(
                ExportFormat::Png,
                source,
                400,
                200,
                true,
                SizeAxis::Width,
            )
            .expect("source size should be accepted"),
            None
        );
        assert_eq!(
            export_target_size_from_dialog_values(
                ExportFormat::Png,
                source,
                100,
                999,
                true,
                SizeAxis::Width,
            )
            .expect("aspect-preserved width should be accepted"),
            Some(ImageSize::new(100, 50))
        );
        assert_eq!(
            export_target_size_from_dialog_values(
                ExportFormat::Png,
                source,
                999,
                80,
                true,
                SizeAxis::Height,
            )
            .expect("aspect-preserved height should be accepted"),
            Some(ImageSize::new(160, 80))
        );
        assert_eq!(
            export_target_size_from_dialog_values(
                ExportFormat::Ico,
                source,
                u32::MAX,
                u32::MAX,
                false,
                SizeAxis::Width,
            )
            .expect("ICO uses fixed frame sizes"),
            None
        );
    }

    #[test]
    fn export_dialog_rotated_live_aspect_sync_matches_final_target_size() {
        let source = ImageSize::new(400, 200);
        let rotated = source.with_rotation(ImageRotation::Degrees90);
        let live_size = export_live_synced_size_for_win32_reference(rotated, SizeAxis::Width, 100)
            .expect("rotated live export size should be calculated");

        assert_eq!(live_size, ImageSize::new(100, 200));
        assert_eq!(
            export_target_size_from_dialog_values(
                ExportFormat::Png,
                rotated,
                live_size.width(),
                live_size.height(),
                true,
                SizeAxis::Width,
            )
            .expect("rotated export size should be accepted"),
            Some(ImageSize::new(100, 200))
        );
    }

    #[test]
    fn export_dialog_size_values_reject_oversized_non_ico_exports() {
        let source = ImageSize::new(400, 200);
        let error = export_target_size_from_dialog_values(
            ExportFormat::Png,
            source,
            u32::try_from(MAX_CONFIG_IMAGE_PIXELS + 1).expect("test pixel limit fits u32"),
            1,
            false,
            SizeAxis::Width,
        )
        .expect_err("oversized export should be rejected before worker start");

        assert!(error.contains(&MAX_CONFIG_IMAGE_PIXELS.to_string()));
    }

    #[test]
    fn fullscreen_save_uses_windowed_size_and_preserves_existing_position() {
        let existing = WindowBounds::new(31, 47, 960, 640);
        let windowed_before_fullscreen = WindowBounds::new(31, 47, 900, 650);

        assert_eq!(
            saved_window_bounds_for_config(existing, 1920, 1080, windowed_before_fullscreen),
            WindowBounds::new(31, 47, 900, 650)
        );
        assert_eq!(
            saved_window_bounds_for_config(existing, 1000, 700, None),
            WindowBounds::new(31, 47, 1000, 700)
        );
        assert_eq!(
            saved_window_bounds_for_config(None, 1000, 700, None),
            WindowBounds::new(0, 0, 1000, 700)
        );
    }

    #[test]
    fn settings_dialog_labels_match_win32_group_layout() {
        assert_eq!(
            ui_text::settings_group_titles(UiLanguage::Korean),
            [
                "일반",
                "줌",
                "애니메이션",
                "탐색",
                "대용량 이미지/메모리",
                "내보내기",
                "단축키"
            ]
        );
        assert_eq!(
            ui_text::settings_export_policy_labels(UiLanguage::Korean),
            ["원본 형식", "PNG", "JPEG", "BMP", "WebP", "ICO"]
        );
        assert_eq!(
            ui_text::settings_wheel_shortcut_labels(UiLanguage::Korean),
            ["마우스휠", "Ctrl+마우스휠"]
        );
        assert_eq!(
            ui_text::settings_drag_shortcut_labels(UiLanguage::Korean),
            ["마우스 왼쪽 클릭 이동", "Ctrl+마우스 왼쪽 클릭 이동"]
        );
        assert_eq!(
            ui_text::settings_group_titles(UiLanguage::English),
            [
                "General",
                "Zoom",
                "Animation",
                "Navigation",
                "Large Images / Memory",
                "Export",
                "Shortcuts"
            ]
        );
    }

    #[test]
    fn settings_combo_indices_match_win32_reference_order() {
        assert_eq!(view_mode_index(ViewMode::FitToWindow), 0);
        assert_eq!(view_mode_index(ViewMode::ManualZoom), 0);
        assert_eq!(view_mode_index(ViewMode::ActualSize), 1);
        assert_eq!(view_mode_from_index(0), Some(ViewMode::FitToWindow));
        assert_eq!(view_mode_from_index(1), Some(ViewMode::ActualSize));
        assert_eq!(view_mode_from_index(2), None);

        assert_eq!(scaling_quality_index(ScalingQuality::Nearest), 0);
        assert_eq!(scaling_quality_index(ScalingQuality::Balanced), 1);
        assert_eq!(scaling_quality_index(ScalingQuality::HighQuality), 2);
        assert_eq!(scaling_quality_from_index(0), Some(ScalingQuality::Nearest));
        assert_eq!(
            scaling_quality_from_index(1),
            Some(ScalingQuality::Balanced)
        );
        assert_eq!(
            scaling_quality_from_index(2),
            Some(ScalingQuality::HighQuality)
        );
        assert_eq!(scaling_quality_from_index(3), None);

        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Source), 0);
        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Png), 1);
        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Jpeg), 2);
        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Bmp), 3);
        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Webp), 4);
        assert_eq!(export_policy_index(DefaultExportFormatPolicy::Ico), 5);
        assert_eq!(
            export_policy_from_index(0),
            Some(DefaultExportFormatPolicy::Source)
        );
        assert_eq!(
            export_policy_from_index(1),
            Some(DefaultExportFormatPolicy::Png)
        );
        assert_eq!(
            export_policy_from_index(2),
            Some(DefaultExportFormatPolicy::Jpeg)
        );
        assert_eq!(
            export_policy_from_index(3),
            Some(DefaultExportFormatPolicy::Bmp)
        );
        assert_eq!(
            export_policy_from_index(4),
            Some(DefaultExportFormatPolicy::Webp)
        );
        assert_eq!(
            export_policy_from_index(5),
            Some(DefaultExportFormatPolicy::Ico)
        );
        assert_eq!(export_policy_from_index(6), None);
    }

    #[test]
    fn export_shutdown_waits_for_active_worker_before_completion_notification() {
        let mut controller = GtkExportController::new();
        let (worker, release_worker, worker_finished) = test_export_worker();
        controller.active_worker = Some(worker);
        let (shutdown_done_sender, shutdown_done_receiver) = mpsc::channel();

        let shutdown_thread = thread::spawn(move || {
            let outcome = controller.shutdown();
            let worker_removed = controller.active_worker.is_none();
            let completion_receiver = match outcome {
                GtkExportShutdownOutcome::WaitingForWorker(receiver) => Some(receiver),
                GtkExportShutdownOutcome::Complete => None,
            };
            let _ = shutdown_done_sender.send((completion_receiver, worker_removed));
        });

        let (completion_receiver, worker_removed) = shutdown_done_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("shutdown should return without waiting for the active export worker");
        let completion_receiver =
            completion_receiver.expect("active export worker should produce waiting outcome");
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

        assert!(worker_removed);
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

    #[test]
    fn decode_shutdown_cancels_owned_workers_without_waiting_for_join() {
        let mut controller = GtkDecodeController::new();
        let (active_worker, active_release, active_finished) =
            test_blocked_decode_worker(DecodeWorkerKind::Initial);
        let (retired_worker, retired_release, retired_finished) =
            test_blocked_decode_worker(DecodeWorkerKind::FullResolution);
        let (folder_scan_worker, folder_scan_release, folder_scan_finished) =
            test_blocked_decode_worker(DecodeWorkerKind::Initial);
        controller.active_worker = Some(active_worker);
        controller.retired_workers.push(retired_worker);
        controller.folder_scan_workers.push(folder_scan_worker);
        let (shutdown_done_sender, shutdown_done_receiver) = mpsc::channel();

        let shutdown_thread = thread::spawn(move || {
            controller.shutdown();
            let _ = shutdown_done_sender.send(!controller.has_live_work());
        });

        let no_live_work = shutdown_done_receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("decode shutdown should return without waiting for workers to exit");
        let active_finished_before_release = active_finished
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        let retired_finished_before_release = retired_finished
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        let folder_scan_finished_before_release = folder_scan_finished
            .recv_timeout(Duration::from_millis(100))
            .is_ok();
        active_release.release();
        retired_release.release();
        folder_scan_release.release();

        assert!(no_live_work);
        assert!(
            !active_finished_before_release,
            "active worker should still be blocked before release"
        );
        assert!(
            !retired_finished_before_release,
            "retired worker should still be blocked before release"
        );
        assert!(
            !folder_scan_finished_before_release,
            "folder scan worker should still be blocked before release"
        );
        assert!(active_finished.recv_timeout(Duration::from_secs(1)).is_ok());
        assert!(retired_finished
            .recv_timeout(Duration::from_secs(1))
            .is_ok());
        assert!(folder_scan_finished
            .recv_timeout(Duration::from_secs(1))
            .is_ok());
        assert!(
            shutdown_thread.join().is_ok(),
            "shutdown thread should not panic"
        );
    }

    #[test]
    fn uri_list_drop_uses_first_supported_image_path() {
        let text = "\
# ignored comment
file:///tmp/readme.txt
not a uri
file:///tmp/photo.png
file:///tmp/other.jpg
";

        assert_eq!(
            first_supported_path_from_uri_list(text),
            Some(PathBuf::from("/tmp/photo.png"))
        );
    }

    #[test]
    fn gnome_copied_files_drop_text_uses_first_supported_file_uri() {
        let text = "\
copy
file:///tmp/readme.txt
file:///tmp/photo.tiff
file:///tmp/other.png
";

        assert_eq!(
            first_supported_path_from_drop_text(text),
            Some(PathBuf::from("/tmp/photo.tiff"))
        );
    }

    #[test]
    fn plain_text_drop_uses_absolute_supported_path() {
        let text = "\
/tmp/readme.txt
/tmp/photo.TGA
relative.png
";

        assert_eq!(
            first_supported_path_from_drop_text(text),
            Some(PathBuf::from("/tmp/photo.TGA"))
        );
    }

    #[test]
    fn icon_list_drop_text_accepts_uri_or_path_token() {
        let text = "\
file:///tmp/readme.txt 10 20 32 32
file:///tmp/photo.webp 40 50 32 32
/tmp/other.png 60 70 32 32
";

        assert_eq!(
            first_supported_path_from_drop_text(text),
            Some(PathBuf::from("/tmp/photo.webp"))
        );
    }

    #[test]
    fn gdk_file_list_drop_uses_first_supported_image_path() {
        let files = [
            gio::File::for_path("/tmp/readme.txt"),
            gio::File::for_path("/tmp/photo.webp"),
            gio::File::for_path("/tmp/other.png"),
        ];
        let file_list = gdk::FileList::from_array(&files);

        assert_eq!(
            first_supported_path_from_file_list(&file_list),
            Some(PathBuf::from("/tmp/photo.webp"))
        );
    }

    #[test]
    fn gio_file_drop_uses_supported_image_path() {
        let image_file = gio::File::for_path("/tmp/photo.PNG");
        let text_file = gio::File::for_path("/tmp/readme.txt");

        assert_eq!(
            first_supported_path_from_gio_file(&image_file),
            Some(PathBuf::from("/tmp/photo.PNG"))
        );
        assert_eq!(first_supported_path_from_gio_file(&text_file), None);
    }

    #[test]
    fn open_filter_suffixes_match_win32_reference_order() {
        let win32_filter_patterns = OPEN_IMAGE_SUFFIXES
            .iter()
            .map(|suffix| format!("*.{suffix}"))
            .collect::<Vec<_>>()
            .join(";");

        assert_eq!(
            win32_filter_patterns,
            "*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga"
        );
        assert_eq!(OPEN_FILE_FILTER_PATTERNS, win32_filter_patterns);
        assert_eq!(
            format!("Supported Images ({OPEN_FILE_FILTER_PATTERNS})"),
            "Supported Images (*.jpg;*.jpeg;*.png;*.bmp;*.gif;*.webp;*.ico;*.tif;*.tiff;*.tga)"
        );
        assert_eq!(OPEN_IMAGE_SUFFIXES.join(", "), SUPPORTED_FORMATS_TEXT);
        assert!(unsupported_drop_message().contains(SUPPORTED_FORMATS_TEXT));
    }

    #[test]
    fn export_filter_labels_match_win32_reference_patterns() {
        let cases = [
            (ExportFormat::Png, "PNG image (*.png)", "*.png"),
            (
                ExportFormat::Jpeg,
                "JPEG image (*.jpg;*.jpeg)",
                "*.jpg;*.jpeg",
            ),
            (ExportFormat::Bmp, "Bitmap image (*.bmp)", "*.bmp"),
            (ExportFormat::Webp, "WebP image (*.webp)", "*.webp"),
            (ExportFormat::Ico, "Icon image (*.ico)", "*.ico"),
        ];

        for (format, label, pattern) in cases {
            assert_eq!(
                export_file_filter_label_and_pattern(format),
                (label, pattern)
            );
            let suffix_pattern = export_format_extensions(format)
                .iter()
                .map(|suffix| format!("*.{suffix}"))
                .collect::<Vec<_>>()
                .join(";");
            assert_eq!(suffix_pattern, pattern);
        }
    }

    #[test]
    fn export_overwrite_confirmation_defaults_to_win32_yes_button() {
        assert_eq!(
            ui_text::yes_no_buttons(UiLanguage::Korean),
            [("예", true), ("아니요", false)]
        );
        assert_eq!(export_overwrite_default_response(), gtk::ResponseType::Yes);
    }

    #[test]
    fn uri_list_drop_decodes_file_uris_and_keeps_drop_order() {
        let text = "\
file:///tmp/not-supported.txt
file:///tmp/encoded%20name.WEBP
file:///tmp/later.png
";

        assert_eq!(
            first_supported_path_from_uri_list(text),
            Some(PathBuf::from("/tmp/encoded name.WEBP"))
        );
    }

    #[test]
    fn uri_list_drop_returns_none_without_supported_file_uri() {
        let text = "\
# comment
https://example.invalid/photo.png
file:///tmp/archive.zip
not a uri
";

        assert_eq!(first_supported_path_from_uri_list(text), None);
    }

    #[test]
    fn embedded_app_icon_decodes_to_texture() {
        let textures = app_icon_textures();

        assert_eq!(textures.len(), 1);
        assert!(textures[0].width() > 0);
        assert!(textures[0].height() > 0);
    }

    #[test]
    fn cairo_argb32_conversion_flattens_rgba_over_white_without_touching_padding() {
        let rgba = [
            10, 20, 30, 255, //
            100, 50, 200, 128, //
            1, 2, 3, 0, //
            40, 80, 120, 64, //
        ];
        let mut destination = [0xEE; 24];

        write_rgba_to_cairo_argb32(&rgba, 2, 2, 12, &mut destination).expect("valid cairo buffer");

        assert_eq!(
            &destination,
            &[
                30, 20, 10, 255, //
                227, 152, 177, 255, //
                0xEE, 0xEE, 0xEE, 0xEE, //
                255, 255, 255, 255, //
                221, 211, 201, 255, //
                0xEE, 0xEE, 0xEE, 0xEE,
            ]
        );
    }

    #[test]
    fn cairo_paint_placement_clips_panned_source_to_visible_viewport() {
        let viewport = ViewportSize::from_client_size(960, 640);
        let image_size = ImageSize::new(6000, 4000);
        let transform = crate::domain::ViewTransform::ACTUAL_SIZE.pan_to_offset(
            viewport,
            image_size,
            crate::domain::ViewOffset::new(120.0, 80.0),
        );
        let rect = transform
            .display_rect(viewport, image_size)
            .expect("display rect");

        let placement =
            cairo_paint_placement(rect, image_size.width(), image_size.height(), viewport)
                .expect("paint placement");

        assert_eq!(
            placement.source_rect,
            CairoSurfaceSourceRect {
                x: 2400,
                y: 1600,
                width: 960,
                height: 640,
            }
        );
    }

    #[test]
    fn cairo_surface_cache_reuses_expanded_rect_for_nearby_pans() {
        let pixels = PixelImage::from(crate::domain::Rgb8Image::new(
            100,
            100,
            vec![128; 100 * 100 * crate::domain::RGB8_BYTES_PER_PIXEL],
        ));
        let render_key = crate::app::RenderImageCacheKey::new(
            1,
            crate::domain::ImageOrientation::NORMAL,
            ImageSize::new(100, 100),
        );
        let mut cache = CairoSurfaceCache::new();
        let max_cache_bytes = 20 * 10 * CAIRO_ARGB32_BYTES_PER_PIXEL;

        let first = cache
            .surface_for_paint_pixel_rect(
                render_key,
                &pixels,
                CairoSurfaceSourceRect {
                    x: 10,
                    y: 10,
                    width: 10,
                    height: 10,
                },
                ScalingQuality::Balanced,
                max_cache_bytes,
            )
            .expect("first cached surface");
        let first_surface = first.surface.to_raw_none();
        assert_eq!(
            first.source_rect,
            CairoSurfaceSourceRect {
                x: 8,
                y: 8,
                width: 14,
                height: 14,
            }
        );

        let second = cache
            .surface_for_paint_pixel_rect(
                render_key,
                &pixels,
                CairoSurfaceSourceRect {
                    x: 12,
                    y: 10,
                    width: 10,
                    height: 10,
                },
                ScalingQuality::Balanced,
                max_cache_bytes,
            )
            .expect("second cached surface");

        assert_eq!(second.source_rect, first.source_rect);
        assert_eq!(second.surface.to_raw_none(), first_surface);
    }

    #[test]
    fn cairo_surface_cache_uses_paint_budget_to_reuse_nearby_pans() {
        let pixels = PixelImage::from(crate::domain::Rgb8Image::new(
            100,
            100,
            vec![128; 100 * 100 * crate::domain::RGB8_BYTES_PER_PIXEL],
        ));
        let render_key = crate::app::RenderImageCacheKey::new(
            1,
            crate::domain::ImageOrientation::NORMAL,
            ImageSize::new(100, 100),
        );
        let full_source_rect = CairoSurfaceSourceRect::full(100, 100).expect("full source");
        let first_placement = CairoPaintPlacement {
            source_rect: CairoSurfaceSourceRect {
                x: 45,
                y: 45,
                width: 10,
                height: 10,
            },
        };
        let cache_budget = cairo_surface_cache_budget_for_paint_placement(
            usize::MAX,
            full_source_rect,
            first_placement,
        );
        let mut cache = CairoSurfaceCache::new();

        let first = cache
            .surface_for_paint_pixel_rect(
                render_key,
                &pixels,
                first_placement.source_rect,
                ScalingQuality::Balanced,
                cache_budget,
            )
            .expect("first cached surface");
        let first_surface = first.surface.to_raw_none();
        assert!(first.source_rect.width > first_placement.source_rect.width);
        assert!(first.source_rect.height > first_placement.source_rect.height);

        let second = cache
            .surface_for_paint_pixel_rect(
                render_key,
                &pixels,
                CairoSurfaceSourceRect {
                    x: 47,
                    y: 47,
                    width: 10,
                    height: 10,
                },
                ScalingQuality::Balanced,
                cache_budget,
            )
            .expect("second cached surface");

        assert_eq!(second.source_rect, first.source_rect);
        assert_eq!(second.surface.to_raw_none(), first_surface);
    }

    #[test]
    fn expanded_cairo_surface_cache_source_rect_grows_both_axes() {
        let full_source_rect = CairoSurfaceSourceRect::full(100, 100).expect("full source");
        let source_rect = CairoSurfaceSourceRect {
            x: 45,
            y: 45,
            width: 10,
            height: 10,
        };
        let max_cache_bytes = source_rect.byte_len().expect("source bytes") * 16;

        let expanded = expanded_cairo_surface_cache_source_rect(
            full_source_rect,
            source_rect,
            max_cache_bytes,
        )
        .expect("expanded cache rect");

        assert_eq!(
            expanded,
            CairoSurfaceSourceRect {
                x: 30,
                y: 30,
                width: 40,
                height: 40,
            }
        );
    }

    #[test]
    fn cairo_surface_for_pixel_rect_converts_rgb8_clip_without_full_surface() {
        let pixels = PixelImage::from(crate::domain::Rgb8Image::new(
            4,
            3,
            vec![
                0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, //
                4, 5, 6, 10, 20, 30, 40, 50, 60, 7, 8, 9, //
                0, 0, 0, 1, 1, 1, 2, 2, 2, 3, 3, 3, //
            ],
        ));

        let mut surface = cairo_surface_for_pixel_rect(
            &pixels,
            CairoSurfaceSourceRect {
                x: 1,
                y: 1,
                width: 2,
                height: 1,
            },
        )
        .expect("clipped surface");
        assert_eq!(surface.width(), 2);
        assert_eq!(surface.height(), 1);
        let stride = usize::try_from(surface.stride()).expect("stride");
        let data = surface.data().expect("surface data");

        assert_eq!(
            &data[..8],
            &[
                30, 20, 10, 255, //
                60, 50, 40, 255,
            ]
        );
        assert!(stride >= 8);
    }

    #[test]
    fn clipboard_texture_pixels_are_flattened_over_white() {
        let rgba = crate::domain::Rgba8Image::new(
            2,
            1,
            vec![
                10, 20, 30, 255, //
                100, 50, 200, 128, //
            ],
        );

        assert_eq!(
            rgba8_flattened_over_white_bytes(&rgba),
            Some(vec![
                10, 20, 30, 255, //
                177, 152, 227, 255,
            ])
        );
    }

    #[test]
    fn cairo_argb32_conversion_rejects_invalid_buffers_without_panicking() {
        let rgba = [0; 16];

        assert!(write_rgba_to_cairo_argb32(&rgba[..15], 2, 2, 8, &mut [0; 16]).is_none());
        assert!(write_rgba_to_cairo_argb32(&rgba, 2, 2, 7, &mut [0; 16]).is_none());
        assert!(write_rgba_to_cairo_argb32(&rgba, 2, 2, 8, &mut [0; 15]).is_none());
    }

    fn test_blocked_decode_worker(
        kind: DecodeWorkerKind,
    ) -> (DecodeWorker, TestWorkerRelease, mpsc::Receiver<()>) {
        let cancel = Arc::new(AtomicBool::new(false));
        let worker_cancel = Arc::clone(&cancel);
        let release = TestWorkerRelease::new();
        let worker_release = release.clone();
        let (finished_sender, finished_receiver) = mpsc::channel();
        let handle = thread::spawn(move || {
            while !worker_cancel.load(Ordering::Acquire) {
                thread::sleep(Duration::from_millis(5));
            }
            worker_release.wait();
            let _ = finished_sender.send(());
        });

        (
            DecodeWorker {
                generation: crate::domain::DecodeGeneration::ZERO,
                kind,
                cancel,
                handle,
                animation_frame: None,
            },
            release,
            finished_receiver,
        )
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
    struct TestWorkerRelease {
        state: Arc<(Mutex<bool>, Condvar)>,
    }

    impl TestWorkerRelease {
        fn new() -> Self {
            Self {
                state: Arc::new((Mutex::new(false), Condvar::new())),
            }
        }

        fn wait(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            while !*released {
                released = condition.wait(released).expect("test worker release wait");
            }
        }

        fn release(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            *released = true;
            condition.notify_all();
        }
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

        fn wait(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            while !*released {
                released = condition.wait(released).expect("test worker release wait");
            }
        }

        fn release(&self) {
            let (lock, condition) = &*self.state;
            let mut released = lock.lock().expect("test worker release lock");
            *released = true;
            condition.notify_all();
        }
    }
}
