//! Pure-Rust anti-aliased rasterizer for the tray icon (ring and bar styles)
//! and pixel helpers. Text (number style) is rendered by the platform layer
//! into a coverage mask and composited here.

use crate::palette::Rgba;
use memmanager_core::state::PressureState;

/// Premultiplied BGRA canvas, top-down.
pub struct Canvas {
    pub size: usize,
    /// Premultiplied colour, f32 per channel: [b, g, r, a].
    px: Vec<[f32; 4]>,
}

const SS: usize = 4; // 4×4 supersampling

impl Canvas {
    pub fn new(size: usize) -> Canvas {
        Canvas {
            size,
            px: vec![[0.0; 4]; size * size],
        }
    }

    /// Composites `color` over the canvas with per-pixel coverage from `cov`.
    pub fn fill(&mut self, color: Rgba, mut cov: impl FnMut(f32, f32) -> bool) {
        let n = self.size;
        let inv = 1.0 / (SS * SS) as f32;
        for y in 0..n {
            for x in 0..n {
                let mut hits = 0;
                for sy in 0..SS {
                    for sx in 0..SS {
                        let fx = x as f32 + (sx as f32 + 0.5) / SS as f32;
                        let fy = y as f32 + (sy as f32 + 0.5) / SS as f32;
                        if cov(fx, fy) {
                            hits += 1;
                        }
                    }
                }
                if hits > 0 {
                    self.blend(x, y, color, hits as f32 * inv);
                }
            }
        }
    }

    /// Composites with an explicit coverage mask (0..=1 per pixel).
    pub fn fill_mask(&mut self, color: Rgba, mask: &[f32]) {
        for (i, &m) in mask.iter().enumerate().take(self.size * self.size) {
            if m > 0.0 {
                self.blend(i % self.size, i / self.size, color, m);
            }
        }
    }

    fn blend(&mut self, x: usize, y: usize, c: Rgba, coverage: f32) {
        let a = c.a * coverage;
        let d = &mut self.px[y * self.size + x];
        let inv = 1.0 - a;
        d[0] = c.b * a + d[0] * inv;
        d[1] = c.g * a + d[1] * inv;
        d[2] = c.r * a + d[2] * inv;
        d[3] = a + d[3] * inv;
    }

    /// Straight-alpha BGRA bytes (what 32-bpp icons expect).
    pub fn to_bgra_straight(&self) -> Vec<u8> {
        let mut out = Vec::with_capacity(self.px.len() * 4);
        for p in &self.px {
            let a = p[3].clamp(0.0, 1.0);
            let un = |c: f32| {
                if a > 0.0 {
                    (c / a).clamp(0.0, 1.0)
                } else {
                    0.0
                }
            };
            out.push((un(p[0]) * 255.0).round() as u8);
            out.push((un(p[1]) * 255.0).round() as u8);
            out.push((un(p[2]) * 255.0).round() as u8);
            out.push((a * 255.0).round() as u8);
        }
        out
    }

    pub fn max_alpha(&self) -> f32 {
        self.px.iter().map(|p| p[3]).fold(0.0, f32::max)
    }
}

/// Angle of point (x, y) around centre, clockwise from 12 o'clock, in turns 0..1.
fn turn(cx: f32, cy: f32, x: f32, y: f32) -> f32 {
    let a = (x - cx).atan2(cy - y); // 0 at top, positive clockwise
    let t = a / std::f32::consts::TAU;
    if t < 0.0 { t + 1.0 } else { t }
}

pub struct IconSpec {
    pub size: usize,
    /// Metric as a fraction 0..1.
    pub fraction: f32,
    pub state: PressureState,
    pub color: Rgba,
    /// Colour of the unfilled track / outline.
    pub track: Rgba,
    pub paused: bool,
    /// Small "!" badge (no privileges).
    pub limited: bool,
}

/// Ring gauge: track + filled arc; High adds a notch gap at 12 o'clock,
/// Critical adds a filled centre dot; paused draws a dashed track.
pub fn draw_ring(spec: &IconSpec) -> Canvas {
    let n = spec.size as f32;
    let mut c = Canvas::new(spec.size);
    let cx = n / 2.0;
    let cy = n / 2.0;
    let thick = (n * 0.16).max(2.0);
    let r_out = n / 2.0 - 0.6;
    let r_in = r_out - thick;
    let frac = spec.fraction.clamp(0.0, 1.0);
    let notch = if spec.state >= PressureState::High {
        0.035
    } else {
        0.0
    };
    let in_ring = |x: f32, y: f32| {
        let d = ((x - cx).powi(2) + (y - cy).powi(2)).sqrt();
        d <= r_out && d >= r_in
    };
    let paused = spec.paused;
    c.fill(spec.track, |x, y| {
        in_ring(x, y) && (!paused || ((turn(cx, cy, x, y) * 12.0).floor() as i32) % 2 == 0)
    });
    if !paused {
        c.fill(spec.color, |x, y| {
            if !in_ring(x, y) {
                return false;
            }
            let t = turn(cx, cy, x, y);
            t >= notch && t <= frac.max(notch)
        });
    }
    if spec.state == PressureState::Critical {
        let r_dot = r_in * 0.55;
        c.fill(spec.color, |x, y| {
            ((x - cx).powi(2) + (y - cy).powi(2)).sqrt() <= r_dot
        });
    }
    if spec.limited {
        draw_badge(&mut c, spec.color);
    }
    c
}

/// Vertical bar inside a rounded outline, filling bottom → top.
pub fn draw_bar(spec: &IconSpec) -> Canvas {
    let n = spec.size as f32;
    let mut c = Canvas::new(spec.size);
    let w = n * 0.56;
    let x0 = (n - w) / 2.0;
    let x1 = x0 + w;
    let y0 = 0.5;
    let y1 = n - 0.5;
    let stroke = (n * 0.09).max(1.2);
    let rad = (n * 0.12).max(1.5);
    let rounded = move |x: f32, y: f32, ax: f32, ay: f32, bx: f32, by: f32, r: f32| {
        if x < ax || x > bx || y < ay || y > by {
            return false;
        }
        let qx = if x < ax + r {
            ax + r
        } else if x > bx - r {
            bx - r
        } else {
            x
        };
        let qy = if y < ay + r {
            ay + r
        } else if y > by - r {
            by - r
        } else {
            y
        };
        (x - qx).powi(2) + (y - qy).powi(2) <= r * r
    };
    let outline_color = if spec.state >= PressureState::Elevated {
        spec.color
    } else {
        spec.track
    };
    c.fill(outline_color, |x, y| {
        rounded(x, y, x0, y0, x1, y1, rad)
            && !rounded(
                x,
                y,
                x0 + stroke,
                y0 + stroke,
                x1 - stroke,
                y1 - stroke,
                (rad - stroke).max(0.5),
            )
    });
    let inset = stroke + (n * 0.06).max(0.8);
    let top = y0 + inset;
    let bottom = y1 - inset;
    let fill_top = bottom - (bottom - top) * spec.fraction.clamp(0.0, 1.0);
    if !spec.paused {
        c.fill(spec.color, |x, y| {
            x >= x0 + inset && x <= x1 - inset && y >= fill_top && y <= bottom
        });
    }
    if spec.limited {
        draw_badge(&mut c, spec.color);
    }
    c
}

/// Small dot at the top-right corner.
fn draw_badge(c: &mut Canvas, color: Rgba) {
    let n = c.size as f32;
    let r = (n * 0.17).max(1.6);
    let (bx, by) = (n - r - 0.3, r + 0.3);
    c.fill(Rgba::rgb(0x000000).with_alpha(0.55), |x, y| {
        (x - bx).powi(2) + (y - by).powi(2) <= (r + 0.9).powi(2)
    });
    c.fill(color.mix(Rgba::rgb(0xFFC83D), 0.7), |x, y| {
        (x - bx).powi(2) + (y - by).powi(2) <= r * r
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    fn spec(state: PressureState) -> IconSpec {
        IconSpec {
            size: 32,
            fraction: 0.62,
            state,
            color: Rgba::rgb(0x0078D4),
            track: Rgba::with_a(0xFFFFFF, 0.3),
            paused: false,
            limited: false,
        }
    }

    #[test]
    fn ring_draws_arc_and_dot() {
        let c = draw_ring(&spec(PressureState::Critical));
        assert!(c.max_alpha() > 0.99);
        let px = c.to_bgra_straight();
        // Centre pixel is the critical dot.
        let i = (16 * 32 + 16) * 4;
        assert!(px[i + 3] > 200);
        let n = draw_ring(&spec(PressureState::Normal)).to_bgra_straight();
        assert_eq!(n[i + 3], 0);
    }

    #[test]
    fn bar_fills_from_bottom() {
        let c = draw_bar(&spec(PressureState::Normal)).to_bgra_straight();
        let alpha = |x: usize, y: usize| c[(y * 32 + x) * 4 + 3];
        assert!(alpha(16, 24) > 200); // filled near the bottom
        assert_eq!(alpha(16, 6), 0); // empty near the top (62%)
    }
}
