//! Cache monitor geometry, never the cursor: local mice and agents can move it.
use core_graphics::display::{CGDisplayBounds, CGGetActiveDisplayList};
use std::{
    sync::mpsc::{Receiver, SyncSender, sync_channel},
    time::{Duration, Instant},
};

#[derive(Clone, Copy)]
struct Bounds {
    x: f64,
    y: f64,
    width: f64,
    height: f64,
}

impl Bounds {
    fn contains(&self, x: f64, y: f64) -> bool {
        x >= self.x && x < self.x + self.width && y >= self.y && y < self.y + self.height
    }
}

fn read_displays() -> Vec<Bounds> {
    let mut displays = [0; 32];
    let mut count = 0;
    let error = unsafe { CGGetActiveDisplayList(32, displays.as_mut_ptr(), &mut count) };
    if error != 0 {
        return Vec::new();
    }
    displays[..(count as usize).min(displays.len())]
        .iter()
        .filter_map(|id| {
            let rect = unsafe { CGDisplayBounds(*id) };
            (rect.size.width >= 1. && rect.size.height >= 1.).then_some(Bounds {
                x: rect.origin.x,
                y: rect.origin.y,
                width: rect.size.width,
                height: rect.size.height,
            })
        })
        .collect()
}

pub(super) struct DisplayLayout {
    bounds: Vec<Bounds>,
    request: SyncSender<()>,
    result: Receiver<Vec<Bounds>>,
    pending: bool,
    requested: Instant,
}

impl DisplayLayout {
    pub fn new() -> Self {
        // One initial snapshot at backend creation, before emulation is enabled.
        let bounds = read_displays();
        let (request, requests) = sync_channel(1);
        let (results, result) = sync_channel(1);
        if let Err(error) = std::thread::Builder::new()
            .name("mouse-display-layout".into())
            .spawn(move || {
                while requests.recv().is_ok() {
                    if results.send(read_displays()).is_err() {
                        break;
                    }
                }
            })
        {
            log::warn!("display geometry worker unavailable: {error}");
        }
        Self {
            bounds,
            request,
            result,
            pending: false,
            requested: Instant::now(),
        }
    }

    pub fn clamp(&mut self, x: f64, y: f64, dx: f64, dy: f64) -> (f64, f64) {
        if let Ok(bounds) = self.result.try_recv() {
            if !bounds.is_empty() {
                self.bounds = bounds;
            }
            self.pending = false;
        }
        // Only request while input is actually arriving; no idle polling.
        if !self.pending
            && self.requested.elapsed() >= Duration::from_secs(1)
            && self.request.try_send(()).is_ok()
        {
            self.pending = true;
            self.requested = Instant::now();
        }
        clamp(&self.bounds, x, y, dx, dy)
    }
}

fn clamp(displays: &[Bounds], x: f64, y: f64, dx: f64, dy: f64) -> (f64, f64) {
    let Some(current) = displays.iter().find(|d| d.contains(x, y)) else {
        // A topology change can move the OS cursor before the new snapshot.
        // Let Quartz constrain the event rather than snapping to an old screen.
        return (x + dx, y + dy);
    };
    let target = displays
        .iter()
        .find(|d| d.contains(x + dx, y + dy))
        .unwrap_or(current);
    (
        (x + dx).clamp(target.x, target.x + target.width - 1.),
        (y + dy).clamp(target.y, target.y + target.height - 1.),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn layouts_support_negative_origins_crossing_edges_and_display_gaps() {
        let displays = [
            Bounds {
                x: -1920.,
                y: 0.,
                width: 1920.,
                height: 1080.,
            },
            Bounds {
                x: 0.,
                y: 0.,
                width: 1440.,
                height: 900.,
            },
        ];
        assert_eq!(clamp(&displays, -1., 30., 2., 0.), (1., 30.));
        assert_eq!(clamp(&displays, 1., 30., -2., 0.), (-1., 30.));
        assert_eq!(clamp(&displays, -1., 1000., 20., 0.), (-1., 1000.));
        assert_eq!(clamp(&displays, 1439., 899., 30., 30.), (1439., 899.));
    }

    #[test]
    fn external_cursor_moves_and_topology_changes_never_restore_a_cached_cursor() {
        let screens = [Bounds {
            x: 0.,
            y: 0.,
            width: 1440.,
            height: 900.,
        }];
        assert_eq!(clamp(&screens, 500., 200., 2., 3.), (502., 203.));
        assert_eq!(clamp(&screens, 900., 600., 2., 3.), (902., 603.));
        assert_eq!(clamp(&screens, -50., 10., 2., 3.), (-48., 13.));
    }
}
