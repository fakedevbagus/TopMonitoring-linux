//! Bar positioning: Wayland layer-shell and X11 strut/dock hints.
//!
//! Extracted from `main.rs` during v4 Fase A (Step A.1) to keep window-manager
//! glue together and to shrink `main.rs` (< 1200 LOC target).

use gtk::prelude::*;
use gtk::ApplicationWindow;
#[cfg(feature = "wayland")]
use gtk4_layer_shell::{Edge, Layer, LayerShell};

use crate::config::Config;

/// Retrieve the geometry (x, y, width, height) of the monitor at `idx`.
/// Falls back to the primary monitor when `idx` is negative.
pub fn monitor_geometry(idx: i32) -> Option<(i32, i32, i32, i32)> {
    let display = gtk::gdk::Display::default()?;
    let monitors = display.monitors();
    let obj = monitors.item(if idx >= 0 { idx as u32 } else { 0 })?;
    let mon = obj.downcast::<gtk::gdk::Monitor>().ok()?;
    let geometry = mon.geometry();
    Some((
        geometry.x(),
        geometry.y(),
        geometry.width(),
        geometry.height(),
    ))
}

/// Result of computing the dock geometry for one monitor.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct X11DockGeometry {
    pub x: i32,
    pub y: i32,
    pub width: u32,
    pub height: u32,
    pub strut_partial: [u32; 12],
    pub strut: [u32; 4],
}

/// Wayland layer-shell configuration. Compiled out when `wayland` is off.
#[cfg(feature = "wayland")]
pub fn configure_wayland(window: &ApplicationWindow, cfg: &Config) {
    let bottom = cfg.position == "bottom";
    window.set_layer(Layer::Top);
    window.set_anchor(Edge::Top, !bottom);
    window.set_anchor(Edge::Bottom, bottom);
    window.set_anchor(Edge::Left, true);
    window.set_anchor(Edge::Right, true);
    window.set_margin(Edge::Top, if bottom { 0 } else { cfg.margin_top });
    window.set_margin(Edge::Bottom, if bottom { cfg.margin_top } else { 0 });
    // The exclusive zone is what actually reserves screen space and
    // forces other windows (including fullscreen ones) to shrink around it.
    window.set_exclusive_zone(cfg.height + cfg.margin_top);
    if cfg.monitor >= 0 {
        if let Some(display) = gtk::gdk::Display::default() {
            if let Some(obj) = display.monitors().item(cfg.monitor as u32) {
                if let Ok(mon) = obj.downcast::<gtk::gdk::Monitor>() {
                    window.set_monitor(&mon);
                }
            }
        }
    }
}

/// X11 dock/strut hints. Casts the window surface to `X11Surface`, computes
/// the dock geometry, reserves screen space, and pins the window position.
// Note: intentionally NOT cfg-gated on `not(wayland)` — `gdk4-x11` is always
// available and `build_bar` calls this on every non-Wayland display path.
pub fn configure_x11(window: &ApplicationWindow, cfg: &Config) {
    if let Some(surface) = window.surface() {
        if let Ok(x11) = surface.downcast::<gdk4_x11::X11Surface>() {
            let geo = monitor_geometry(cfg.monitor);
            match apply_x11_dock(
                x11.xid() as u32,
                geo,
                cfg.margin_top as u32,
                cfg.height as u32,
                cfg.position == "bottom",
            ) {
                Ok(w) => window.set_size_request(w as i32, cfg.height),
                Err(e) => eprintln!("[TopMonitoring] Failed to apply X11 dock hints: {e}"),
            }
        }
    }
}

/// Pure geometry helper. No X11 calls; fully unit-testable.
pub fn calculate_x11_dock_geometry(
    screen_width: i32,
    screen_height: i32,
    monitor: Option<(i32, i32, i32, i32)>,
    offset: u32,
    bar_height: u32,
    bottom: bool,
) -> Option<X11DockGeometry> {
    if screen_width <= 0 || screen_height <= 0 || bar_height == 0 {
        return None;
    }
    let (monitor_x, monitor_y, monitor_width, monitor_height) =
        monitor.unwrap_or((0, 0, screen_width, screen_height));
    if monitor_width <= 0 || monitor_height <= 0 {
        return None;
    }

    let width = monitor_width.clamp(1, screen_width) as u32;
    let height = (bar_height as i32).clamp(1, screen_height) as u32;
    let width_i = width as i32;
    let height_i = height as i32;
    let x = monitor_x.clamp(0, screen_width - width_i);
    let x_start = x as u32;
    let x_end = (x + width_i - 1).clamp(0, screen_width - 1) as u32;
    let monitor_top = monitor_y.clamp(0, screen_height);
    let monitor_bottom = monitor_y
        .saturating_add(monitor_height)
        .clamp(0, screen_height);
    let offset_i = (offset.min(screen_height as u32)) as i32;

    let (y, reserve) = if bottom {
        let outside = (screen_height - monitor_bottom).max(0);
        let reserve = outside
            .saturating_add(offset_i)
            .saturating_add(height_i)
            .clamp(0, screen_height) as u32;
        let y = monitor_bottom
            .saturating_sub(offset_i)
            .saturating_sub(height_i)
            .clamp(0, screen_height - height_i);
        (y, reserve)
    } else {
        let reserve = monitor_top
            .saturating_add(offset_i)
            .saturating_add(height_i)
            .clamp(0, screen_height) as u32;
        let y = monitor_top
            .saturating_add(offset_i)
            .clamp(0, screen_height - height_i);
        (y, reserve)
    };

    let (strut_partial, strut) = if bottom {
        (
            [0, 0, 0, reserve, 0, 0, 0, 0, 0, 0, x_start, x_end],
            [0, 0, 0, reserve],
        )
    } else {
        (
            [0, 0, reserve, 0, 0, 0, 0, 0, x_start, x_end, 0, 0],
            [0, 0, reserve, 0],
        )
    };
    Some(X11DockGeometry {
        x,
        y,
        width,
        height,
        strut_partial,
        strut,
    })
}

/// Turns a plain X11 window into a dock: sets `_NET_WM_WINDOW_TYPE_DOCK`,
/// reserves space via `_NET_WM_STRUT_PARTIAL`/`_NET_WM_STRUT`, keeps it on
/// all workspaces, and pins its exact geometry. Returns the applied width.
pub fn apply_x11_dock(
    xid: u32,
    geo: Option<(i32, i32, i32, i32)>,
    offset: u32,
    bar_height: u32,
    bottom: bool,
) -> Result<u32, Box<dyn std::error::Error>> {
    use x11rb::connection::Connection;
    use x11rb::protocol::xproto::{
        AtomEnum, ConfigureWindowAux, ConnectionExt as XProto, PropMode,
    };
    use x11rb::wrapper::ConnectionExt as Wrapper;

    let (conn, screen_num) = x11rb::connect(None)?;
    let screen = &conn.setup().roots[screen_num];
    let sw = screen.width_in_pixels as i32;
    let sh = screen.height_in_pixels as i32;
    let geometry = calculate_x11_dock_geometry(sw, sh, geo, offset, bar_height, bottom)
        .ok_or_else(|| {
            std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "invalid X11 dock geometry",
            )
        })?;

    let atom = |name: &[u8]| -> Result<u32, Box<dyn std::error::Error>> {
        Ok(conn.intern_atom(false, name)?.reply()?.atom)
    };

    let wtype = atom(b"_NET_WM_WINDOW_TYPE")?;
    let dock = atom(b"_NET_WM_WINDOW_TYPE_DOCK")?;
    conn.change_property32(PropMode::REPLACE, xid, wtype, AtomEnum::ATOM, &[dock])?;

    let strut_p = atom(b"_NET_WM_STRUT_PARTIAL")?;
    let strut = atom(b"_NET_WM_STRUT")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut_p,
        AtomEnum::CARDINAL,
        &geometry.strut_partial,
    )?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        strut,
        AtomEnum::CARDINAL,
        &geometry.strut,
    )?;

    let desktop = atom(b"_NET_WM_DESKTOP")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        desktop,
        AtomEnum::CARDINAL,
        &[0xFFFF_FFFF],
    )?;

    let state = atom(b"_NET_WM_STATE")?;
    let sticky = atom(b"_NET_WM_STATE_STICKY")?;
    let above = atom(b"_NET_WM_STATE_ABOVE")?;
    let skip_tb = atom(b"_NET_WM_STATE_SKIP_TASKBAR")?;
    let skip_pg = atom(b"_NET_WM_STATE_SKIP_PAGER")?;
    conn.change_property32(
        PropMode::REPLACE,
        xid,
        state,
        AtomEnum::ATOM,
        &[sticky, above, skip_tb, skip_pg],
    )?;

    conn.configure_window(
        xid,
        &ConfigureWindowAux::new()
            .x(geometry.x)
            .y(geometry.y)
            .width(geometry.width)
            .height(geometry.height),
    )?;
    conn.flush()?;
    Ok(geometry.width)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dock_geometry_primary_top() {
        let geo = calculate_x11_dock_geometry(1920, 1080, None, 4, 40, false).expect("valid");
        assert_eq!(geo.width, 1920);
        assert_eq!(geo.height, 40);
        assert_eq!(geo.x, 0);
        assert_eq!(geo.y, 4);
        assert_eq!(geo.strut[2], 44); // reserve = top(0) + offset(4) + height(40)
        assert_eq!(geo.strut_partial[8], 0); // x_start
        assert_eq!(geo.strut_partial[9], 1919); // x_end
    }

    #[test]
    fn dock_geometry_primary_bottom() {
        let geo = calculate_x11_dock_geometry(1920, 1080, None, 0, 36, true).expect("valid");
        assert_eq!(geo.y, 1080 - 36);
        assert_eq!(geo.strut[3], 36); // reserve = bottom(0) + offset(0) + height(36)
    }

    #[test]
    fn dock_geometry_rejects_zero() {
        assert!(calculate_x11_dock_geometry(0, 1080, None, 0, 40, false).is_none());
        assert!(calculate_x11_dock_geometry(1920, 1080, None, 0, 0, false).is_none());
        assert!(
            calculate_x11_dock_geometry(1920, 1080, Some((0, 0, 0, 1080)), 0, 40, false).is_none()
        );
    }
}
