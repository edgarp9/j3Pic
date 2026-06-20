use std::env;
use std::ffi::OsString;
use std::path::PathBuf;
use std::time::{Duration, Instant};

use j3pic::app::ViewerApp;
use j3pic::domain::ViewportSize;
use j3pic::infra::ImageOpenProfile;
#[cfg(target_os = "windows")]
use j3pic::platform::win32::{profile_win32_paint_prepare, Win32PaintPrepareProfile};

const PROFILE_VIEWPORT_WIDTH: i32 = 960;
const PROFILE_VIEWPORT_HEIGHT: i32 = 640;
const DEFAULT_PROFILE_IMAGE_PATH: &str = r"C:\Users\dolco\Desktop\111.jpg";

fn main() {
    std::process::exit(run());
}

fn run() -> i32 {
    let mut args = env::args_os().skip(1);
    let path = match args.next() {
        Some(path) => path,
        None => OsString::from(DEFAULT_PROFILE_IMAGE_PATH),
    };
    if args.next().is_some() {
        eprintln!("usage: cargo run --bin profile_open -- [image-path]");
        return 2;
    }

    match profile_open(PathBuf::from(path)) {
        Ok(()) => 0,
        Err(error) => {
            eprintln!("{error}");
            1
        }
    }
}

fn profile_open(path: PathBuf) -> Result<(), String> {
    let mut app = ViewerApp::new();
    app.handle_resize(PROFILE_VIEWPORT_WIDTH, PROFILE_VIEWPORT_HEIGHT);
    let viewport = ViewportSize::from_client_size(PROFILE_VIEWPORT_WIDTH, PROFILE_VIEWPORT_HEIGHT);
    #[cfg(target_os = "windows")]
    let max_cache_bytes = app.memory_policy().max_cache_entry_bytes();

    let profile = app
        .load_image_with_profile(&path)
        .map_err(|error| format!("failed to open {}: {error}", path.display()))?;
    let render_started = Instant::now();
    let render = app.prepare_first_render(viewport);
    let render_duration = render_started.elapsed();
    let mut rows = profile_table_rows(&profile);
    push_profile_table_row(&mut rows, "app.prepare_first_render", render_duration);

    #[cfg(target_os = "windows")]
    let paint_prepare = {
        if let Some(render) = render.as_ref() {
            let started = Instant::now();
            let paint_prepare = profile_win32_paint_prepare(render, viewport, max_cache_bytes);
            push_profile_table_row(&mut rows, "win32.prepare_paint_dib", started.elapsed());
            paint_prepare
        } else {
            None
        }
    };
    #[cfg(not(target_os = "windows"))]
    let paint_prepare: Option<()> = None;

    println!("j3Pic image open profile");
    println!("path: {}", display_os_string(path.into_os_string()));
    println!("viewport: {PROFILE_VIEWPORT_WIDTH}x{PROFILE_VIEWPORT_HEIGHT}");
    println!();
    print!("{}", format_profile_table(&rows));

    if let Some(render) = render {
        let rect = render.rect();
        println!();
        println!(
            "render: source={}x{} {:?}, rect={}x{} at {},{}, quality={:?}",
            render.pixels().width(),
            render.pixels().height(),
            render.pixels().pixel_format(),
            rect.width(),
            rect.height(),
            rect.x(),
            rect.y(),
            render.scaling_quality()
        );
    }
    print_paint_prepare_summary(paint_prepare);
    println!();
    println!(
        "open_total_ms: {:.3}",
        duration_ms(profile.total_duration())
    );
    Ok(())
}

fn duration_ms(duration: Duration) -> f64 {
    duration.as_secs_f64() * 1000.0
}

#[derive(Debug, Clone, PartialEq)]
struct ProfileTableRow {
    stage: &'static str,
    delta: Duration,
    total: Duration,
}

impl ProfileTableRow {
    fn new(stage: &'static str, delta: Duration, total: Duration) -> Self {
        Self {
            stage,
            delta,
            total,
        }
    }
}

fn profile_table_rows(profile: &ImageOpenProfile) -> Vec<ProfileTableRow> {
    profile
        .stages()
        .iter()
        .map(|stage| ProfileTableRow::new(stage.name(), stage.duration(), stage.total_duration()))
        .collect()
}

fn push_profile_table_row(rows: &mut Vec<ProfileTableRow>, stage: &'static str, delta: Duration) {
    let total = rows.last().map(|row| row.total + delta).unwrap_or(delta);
    rows.push(ProfileTableRow::new(stage, delta, total));
}

fn format_profile_table(rows: &[ProfileTableRow]) -> String {
    let mut output = format!("{:<42} {:>12} {:>12}\n", "stage", "delta_ms", "total_ms");
    for row in rows {
        output.push_str(&format!(
            "{:<42} {:>12.3} {:>12.3}\n",
            row.stage,
            duration_ms(row.delta),
            duration_ms(row.total)
        ));
    }
    output
}

#[cfg(target_os = "windows")]
fn print_paint_prepare_summary(paint_prepare: Option<Win32PaintPrepareProfile>) {
    if let Some(paint_prepare) = paint_prepare {
        println!(
            "win32_paint_prepare: dib_bytes={}, source={}x{} at {},{}, cached={}",
            paint_prepare.dib_bytes(),
            paint_prepare.source_width(),
            paint_prepare.source_height(),
            paint_prepare.source_x(),
            paint_prepare.source_y(),
            paint_prepare.cached()
        );
    }
}

#[cfg(not(target_os = "windows"))]
fn print_paint_prepare_summary(_paint_prepare: Option<()>) {}

fn display_os_string(value: OsString) -> String {
    value.to_string_lossy().into_owned()
}

#[cfg(test)]
mod tests {
    use std::time::Duration;

    use super::{format_profile_table, push_profile_table_row, ProfileTableRow};

    #[test]
    fn profile_table_contains_stage_delta_and_total_columns() {
        let rows = vec![ProfileTableRow::new(
            "open.format_detection",
            Duration::from_millis(1),
            Duration::from_millis(1),
        )];

        let table = format_profile_table(&rows);

        assert!(table.starts_with("stage"));
        assert!(table.contains("delta_ms"));
        assert!(table.contains("total_ms"));
        assert!(table.contains("open.format_detection"));
        assert!(table.contains("1.000"));
    }

    #[test]
    fn profile_table_rows_accumulate_totals_without_time_thresholds() {
        let mut rows = vec![ProfileTableRow::new(
            "open.complete",
            Duration::from_millis(3),
            Duration::from_millis(5),
        )];

        push_profile_table_row(
            &mut rows,
            "app.prepare_first_render",
            Duration::from_millis(7),
        );
        push_profile_table_row(
            &mut rows,
            "win32.prepare_paint_dib",
            Duration::from_millis(11),
        );

        assert_eq!(rows[1].total, Duration::from_millis(12));
        assert_eq!(rows[2].total, Duration::from_millis(23));
    }
}
