//! Tray icon drawn programmatically as ARGB32 (no image dependencies).
//!
//! A vertical "runway meter": dark track, coloured fill for the worst provider
//! percent, colour bands green/amber/red, grey when unknown.

pub const ICON_SIZE: i32 = 32;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Band {
    Green,
    Amber,
    Red,
    Unknown,
}

/// Glanceable colour bands. These intentionally differ from the notification
/// levels (Warning ≥ 80 / Critical ≥ 100 in `state.rs:126`): colour is a
/// continuous at-a-glance signal, notify is a transitions-only policy — the
/// colour-vs-notify split, not an accidental non-unification.
pub fn band_for(percent: Option<f64>) -> Band {
    match percent {
        Some(v) if v >= 85.0 => Band::Red,
        Some(v) if v >= 60.0 => Band::Amber,
        Some(_) => Band::Green,
        None => Band::Unknown,
    }
}

fn band_rgb(b: Band) -> (u8, u8, u8) {
    match b {
        Band::Green => (76, 175, 80),
        Band::Amber => (255, 179, 0),
        Band::Red => (229, 57, 53),
        Band::Unknown => (130, 134, 140),
    }
}

/// Draw the meter. `percent`: worst used percent (clamped 0–100 for the fill);
/// None → empty rounded track in grey with a centred dot (no coloured fill).
pub fn draw(percent: Option<f64>) -> ksni::Icon {
    let size = ICON_SIZE;
    let track = (22u8, 26u8, 32u8);
    let border = (70u8, 76u8, 84u8);
    let highlight = (86u8, 90u8, 96u8);
    let dot = (170u8, 174u8, 180u8);
    let band = band_for(percent);
    let fill = band_rgb(band);
    // Meter geometry: vertical bar centred horizontally, 4px margins + 2px border.
    let x0 = 11;
    let x1 = 20;
    let y0 = 3;
    let y1 = size - 4;
    let mut data = vec![0u8; (size * size * 4) as usize];
    let fill_rows = match percent {
        Some(p) => ((p.clamp(0.0, 100.0) / 100.0) * ((y1 - y0 - 2) as f64)).round() as i32,
        None => 0,
    };
    // Unknown dot: radius-3 filled circle at the meter centre (legible at 32px
    // where `%` text would not be).
    let cx = (x0 + x1) / 2;
    let cy = (y0 + y1) / 2;
    for y in 0..size {
        for x in 0..size {
            let idx = ((y * size + x) * 4) as usize;
            let in_meter = x >= x0 && x <= x1 && y >= y0 && y <= y1;
            // Rounded track: the 2×2 corner blocks stay transparent.
            let corner = in_meter && (x - x0 < 2 || x1 - x < 2) && (y - y0 < 2 || y1 - y < 2);
            let (r, g, b, a) = if !in_meter || corner {
                // outside the meter (or rounded off): transparent
                (0, 0, 0, 0)
            } else if x == x0 || x == x1 || y == y0 || y == y1 {
                (border.0, border.1, border.2, 255)
            } else if percent.is_none() && (x - cx) * (x - cx) + (y - cy) * (y - cy) <= 9 {
                (dot.0, dot.1, dot.2, 255)
            } else {
                let inner_bottom = y1 - 1;
                let rows_from_bottom = inner_bottom - y + 1; // 1 at the lowest inner row
                let (br, bg, bb) = if rows_from_bottom <= fill_rows {
                    fill
                } else {
                    track
                };
                // Inner highlight: one lightened row just under the top border.
                if y == y0 + 1 && br == track.0 && bg == track.1 && bb == track.2 {
                    (highlight.0, highlight.1, highlight.2, 255)
                } else {
                    (br, bg, bb, 255)
                }
            };
            // ksni Icon data is ARGB32, network byte order: A, R, G, B per pixel.
            data[idx] = a;
            data[idx + 1] = r;
            data[idx + 2] = g;
            data[idx + 3] = b;
        }
    }
    ksni::Icon {
        width: size,
        height: size,
        data,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn geometry_and_argb_layout() {
        let icon = draw(Some(50.0));
        assert_eq!(icon.width, ICON_SIZE);
        assert_eq!(icon.height, ICON_SIZE);
        assert_eq!(icon.data.len(), (ICON_SIZE * ICON_SIZE * 4) as usize);
        // A centre pixel inside the track is opaque (alpha = 255 at first byte)
        let idx = ((16 * ICON_SIZE + 16) * 4) as usize;
        assert_eq!(icon.data[idx], 255, "alpha byte");
    }

    #[test]
    fn bands_map_thresholds() {
        assert_eq!(band_for(Some(0.0)), Band::Green);
        assert_eq!(band_for(Some(59.9)), Band::Green);
        assert_eq!(band_for(Some(60.0)), Band::Amber);
        assert_eq!(band_for(Some(84.9)), Band::Amber);
        assert_eq!(band_for(Some(85.0)), Band::Red);
        assert_eq!(band_for(Some(130.0)), Band::Red);
        assert_eq!(band_for(None), Band::Unknown);
    }

    #[test]
    fn fill_grows_monotonically() {
        let count_band = |icon: &ksni::Icon, rgb: (u8, u8, u8)| -> i64 {
            let mut n = 0i64;
            let mut i = 0;
            while i + 4 <= icon.data.len() {
                let px = &icon.data[i..i + 4];
                if px[0] == 255 && px[1] == rgb.0 && px[2] == rgb.1 && px[3] == rgb.2 {
                    n += 1;
                }
                i += 4;
            }
            n
        };
        let low = count_band(&draw(Some(10.0)), band_rgb(Band::Green));
        let mid = count_band(&draw(Some(50.0)), band_rgb(Band::Green));
        let high = count_band(&draw(Some(90.0)), band_rgb(Band::Red));
        assert!(low < mid, "green fill grows {low} < {mid}");
        assert!(
            high > mid,
            "red fill at 90%% dominates: {high} vs green-at-50 {mid}"
        );
        // unknown: no coloured fill at all
        for band in [Band::Green, Band::Amber, Band::Red] {
            assert_eq!(count_band(&draw(None), band_rgb(band)), 0);
        }
    }

    fn px(icon: &ksni::Icon, x: i32, y: i32) -> [u8; 4] {
        let idx = ((y * ICON_SIZE + x) * 4) as usize;
        // ARGB32 network order: A, R, G, B per pixel.
        [
            icon.data[idx],
            icon.data[idx + 1],
            icon.data[idx + 2],
            icon.data[idx + 3],
        ]
    }

    #[test]
    fn rounded_corners_are_transparent() {
        let icon = draw(Some(50.0));
        assert_eq!(px(&icon, 11, 3)[0], 0, "outer corner cut");
        assert_eq!(px(&icon, 12, 4)[0], 0, "inner corner cut");
        assert_eq!(px(&icon, 20, 28)[0], 0, "opposite corner cut");
        // Adjacent edge pixels stay opaque border.
        assert_eq!(px(&icon, 11, 5), [255, 70, 76, 84]);
        assert_eq!(px(&icon, 13, 3), [255, 70, 76, 84]);
    }

    #[test]
    fn inner_highlight_row() {
        // (15,4): top inner row, track zone at 50% → lightened highlight.
        assert_eq!(px(&draw(Some(50.0)), 15, 4), [255, 86, 90, 96]);
    }

    #[test]
    fn unknown_dot() {
        // Centre pixel is the grey dot, opaque; bands stay unmapped.
        assert_eq!(px(&draw(None), 15, 15), [255, 170, 174, 180]);
        assert_eq!(band_for(None), Band::Unknown);
    }
}
