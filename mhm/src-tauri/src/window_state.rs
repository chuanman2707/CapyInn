//! Window sizing and state restoration.
//!
//! Two concerns live here:
//!
//! 1. `tauri-plugin-window-state` persists size, position, and maximized state across
//!    launches. `plugin()` and `state_flags()` own its configuration so the plugin
//!    registration and the manual save on exit can never disagree about the flags.
//! 2. `apply_startup_geometry` owns what the window looks like at launch: sized to the
//!    display on the first run, restored to its last geometry on every run after.
//!
//! Every step is best-effort: any failure logs at `warn` and leaves the window at the
//! size Tauri already gave it. Window geometry must never keep the app from starting.

use log::warn;
use tauri::plugin::TauriPlugin;
use tauri::{AppHandle, LogicalPosition, LogicalSize, Manager, PhysicalRect, Runtime};
use tauri_plugin_window_state::{StateFlags, WindowExt};

/// Smallest window the Dashboard layout holds together at.
const MIN_WIDTH: f64 = 1100.0;
const MIN_HEIGHT: f64 = 700.0;

/// Fraction of the monitor work area the window occupies on first launch.
const FIRST_LAUNCH_RATIO: f64 = 0.85;

/// Pinned explicitly so first-launch detection and the plugin agree on the path.
const STATE_FILENAME: &str = ".window-state.json";

/// Label of the window declared in `tauri.conf.json`.
const MAIN_WINDOW_LABEL: &str = "main";

/// State persisted across launches.
///
/// `VISIBLE`, `FULLSCREEN`, and `DECORATIONS` are deliberately left off: restoring a
/// hidden window strands the user with no way back, fullscreen restore reopens into a
/// separate macOS Space, and CapyInn never changes decorations.
pub fn state_flags() -> StateFlags {
    StateFlags::SIZE | StateFlags::POSITION | StateFlags::MAXIMIZED
}

pub fn plugin<R: Runtime>() -> TauriPlugin<R> {
    tauri_plugin_window_state::Builder::new()
        .with_state_flags(state_flags())
        .with_filename(STATE_FILENAME)
        // The plugin's own restore runs after setup(), which would overwrite the minimum
        // we enforce there. apply_startup_geometry() calls restore_state() itself so the
        // restore and the clamp happen in a known order.
        .skip_initial_state(MAIN_WINDOW_LABEL)
        .build()
}

/// Sizes the main window at startup.
///
/// On first launch there is no saved state, so the window is sized relative to its
/// monitor. On every later launch this restores the saved geometry and then enforces the
/// minimum on top of it.
///
/// The restore is driven from here rather than left to the plugin because the plugin's
/// own restore runs *after* `setup()`, so anything done here would be overwritten.
///
/// The minimum needs enforcing on top of the restore because `minWidth`/`minHeight` become
/// `NSWindow.contentMinSize`, which AppKit applies to user drags but *not* to the
/// programmatic `setFrame:` a restore uses. Without it, a state file holding a sub-minimum
/// size — written by a build that predates the minimum, or left behind by a display change
/// — reopens the window too small to use, with no way out but a manual resize.
pub fn apply_startup_geometry(app: &AppHandle) {
    let Some(window) = app.get_webview_window(MAIN_WINDOW_LABEL) else {
        warn!("Skipping window sizing, no '{MAIN_WINDOW_LABEL}' window");
        return;
    };

    let monitor = match window.current_monitor() {
        Ok(Some(monitor)) => monitor,
        Ok(None) => {
            warn!("Skipping window sizing, no current monitor");
            return;
        }
        Err(error) => {
            warn!("Skipping window sizing, monitor lookup failed: {error}");
            return;
        }
    };
    let work_area = monitor.work_area();
    let scale_factor = monitor.scale_factor();

    if is_first_launch(app) {
        let (size, position) = compute_first_launch_geometry(work_area, scale_factor);
        if let Err(error) = window.set_size(size) {
            warn!("Failed to set first-launch window size: {error}");
        } else if let Err(error) = window.set_position(position) {
            warn!("Failed to set first-launch window position: {error}");
        }
        // The computed size already honors the minimum, so there is nothing to clamp.
        return;
    }

    let undersized = saved_size_below_minimum(app, work_area, scale_factor);
    if let Err(error) = window.restore_state(state_flags()) {
        warn!("Failed to restore saved window state: {error}");
    }
    // Ordered after the restore on purpose: both are asynchronous `setFrame:` calls, so
    // the later one is the size that sticks.
    if let Some(size) = undersized {
        if let Err(error) = window.set_size(size) {
            warn!("Failed to grow the restored window to the minimum size: {error}");
        }
    }
}

/// Whether there is no saved state to restore.
///
/// An unresolvable config dir is treated as a returning launch: guessing "first launch"
/// would resize a window the user had already placed, while guessing "returning" costs
/// nothing worse than skipping the initial sizing.
fn is_first_launch(app: &AppHandle) -> bool {
    match app.path().app_config_dir() {
        Ok(dir) => !dir.join(STATE_FILENAME).exists(),
        Err(error) => {
            warn!("Cannot resolve config dir, treating as a returning launch: {error}");
            false
        }
    }
}

/// The size the window should end up at when the saved state is below the minimum.
///
/// Reads the size off the state file rather than off the window. Asking the window is
/// useless here: `restore_state` applies its size asynchronously, so a read taken right
/// after it still reports the pre-restore size. The file is the only source that is
/// accurate at this point in startup.
///
/// Returns `None` when the saved size is already large enough, or when the file cannot be
/// read or understood — in every one of those cases the restore is left untouched.
fn saved_size_below_minimum(
    app: &AppHandle,
    work_area: &PhysicalRect<i32, u32>,
    scale_factor: f64,
) -> Option<LogicalSize<f64>> {
    let path = app.path().app_config_dir().ok()?.join(STATE_FILENAME);
    let raw = std::fs::read_to_string(&path)
        .inspect_err(|error| warn!("Cannot read saved window state: {error}"))
        .ok()?;
    let root: serde_json::Value = serde_json::from_str(&raw)
        .inspect_err(|error| warn!("Saved window state is not valid JSON: {error}"))
        .ok()?;

    let entry = root.get(MAIN_WINDOW_LABEL)?;
    let width = entry.get("width")?.as_f64()?;
    let height = entry.get("height")?.as_f64()?;

    // The plugin stores physical pixels; the clamp works in logical ones.
    let scale = normalize_scale(scale_factor);
    clamp_to_minimum(
        LogicalSize::new(width / scale, height / scale),
        LogicalSize::new(
            f64::from(work_area.size.width) / scale,
            f64::from(work_area.size.height) / scale,
        ),
    )
}

/// The size a window should be grown to, or `None` when it is already large enough.
///
/// Same clamp order as first-launch sizing: floor at the minimum, then cap at the work
/// area so a display smaller than the minimum still gets a window that fits.
fn clamp_to_minimum(
    current: LogicalSize<f64>,
    work_area: LogicalSize<f64>,
) -> Option<LogicalSize<f64>> {
    let width = current.width.max(MIN_WIDTH).min(work_area.width).round();
    let height = current.height.max(MIN_HEIGHT).min(work_area.height).round();

    // Sub-pixel drift from the scale-factor round trip is not a resize worth doing.
    if (width - current.width).abs() < 1.0 && (height - current.height).abs() < 1.0 {
        return None;
    }
    Some(LogicalSize::new(width, height))
}

/// A zero or nonsensical scale factor would poison every division that uses it.
fn normalize_scale(scale_factor: f64) -> f64 {
    if scale_factor.is_finite() && scale_factor > 0.0 {
        scale_factor
    } else {
        1.0
    }
}

/// Centers a window of `FIRST_LAUNCH_RATIO` of the work area within that work area.
///
/// All arithmetic is in logical pixels, so a Retina display does not get a double-size
/// window. The size is floored at the minimum and then capped at the work area itself —
/// the cap is applied second and wins, so a display smaller than the minimum gets a
/// window that fits rather than one that overflows. The position is offset by the work
/// area's own origin so a window destined for a second display lands there.
fn compute_first_launch_geometry(
    work_area: &PhysicalRect<i32, u32>,
    scale_factor: f64,
) -> (LogicalSize<f64>, LogicalPosition<f64>) {
    let scale = normalize_scale(scale_factor);

    let area_width = f64::from(work_area.size.width) / scale;
    let area_height = f64::from(work_area.size.height) / scale;
    let area_x = f64::from(work_area.position.x) / scale;
    let area_y = f64::from(work_area.position.y) / scale;

    // Floor at the minimum first, then cap at the work area. The cap wins.
    let width = (area_width * FIRST_LAUNCH_RATIO)
        .max(MIN_WIDTH)
        .min(area_width)
        .round();
    let height = (area_height * FIRST_LAUNCH_RATIO)
        .max(MIN_HEIGHT)
        .min(area_height)
        .round();

    let x = (area_x + (area_width - width) / 2.0).round();
    let y = (area_y + (area_height - height) / 2.0).round();

    (LogicalSize::new(width, height), LogicalPosition::new(x, y))
}

#[cfg(test)]
mod tests {
    use super::*;
    use tauri::{PhysicalPosition, PhysicalSize};

    fn work_area(x: i32, y: i32, width: u32, height: u32) -> PhysicalRect<i32, u32> {
        PhysicalRect {
            position: PhysicalPosition::new(x, y),
            size: PhysicalSize::new(width, height),
        }
    }

    #[test]
    fn retina_work_area_is_measured_in_logical_pixels() {
        let (size, _) = compute_first_launch_geometry(&work_area(0, 0, 3024, 1890), 2.0);

        // 3024x1890 physical at 2x is 1512x945 logical, and 85% of that is 1285x803.
        assert_eq!(size.width, 1285.0);
        assert_eq!(size.height, 803.0);
    }

    #[test]
    fn size_is_floored_at_the_minimum_window_size() {
        let (size, _) = compute_first_launch_geometry(&work_area(0, 0, 1280, 800), 1.0);

        // 85% would be 1088x680, below the 1100x700 floor.
        assert_eq!(size.width, MIN_WIDTH);
        assert_eq!(size.height, MIN_HEIGHT);
    }

    #[test]
    fn work_area_ceiling_beats_the_minimum_floor() {
        let (size, _) = compute_first_launch_geometry(&work_area(0, 0, 1000, 600), 1.0);

        // The window must never exceed the display, even to honor the minimum.
        assert_eq!(size.width, 1000.0);
        assert_eq!(size.height, 600.0);
    }

    #[test]
    fn a_restored_window_below_the_minimum_is_grown_back() {
        // The plugin restores geometry with a programmatic setFrame:, which AppKit does
        // not constrain to contentMinSize — so this is the only thing standing between a
        // stale state file and the window reopening too small to use.
        let grown = clamp_to_minimum(
            LogicalSize::new(600.0, 400.0),
            LogicalSize::new(2560.0, 1285.0),
        );

        assert_eq!(grown, Some(LogicalSize::new(MIN_WIDTH, MIN_HEIGHT)));
    }

    #[test]
    fn a_restored_window_at_or_above_the_minimum_is_left_alone() {
        let unchanged = clamp_to_minimum(
            LogicalSize::new(1400.0, 900.0),
            LogicalSize::new(2560.0, 1285.0),
        );

        assert_eq!(unchanged, None);
    }

    #[test]
    fn growing_to_the_minimum_never_exceeds_the_work_area() {
        let grown =
            clamp_to_minimum(LogicalSize::new(600.0, 400.0), LogicalSize::new(1000.0, 600.0));

        assert_eq!(grown, Some(LogicalSize::new(1000.0, 600.0)));
    }

    #[test]
    fn position_is_offset_by_the_monitor_origin() {
        let (size, position) = compute_first_launch_geometry(&work_area(1920, 0, 1920, 1080), 1.0);

        assert_eq!(size.width, 1632.0);
        assert_eq!(size.height, 918.0);
        // Centered on the second monitor: 1920 + (1920 - 1632) / 2.
        assert_eq!(position.x, 2064.0);
        assert_eq!(position.y, 81.0);
    }
}
