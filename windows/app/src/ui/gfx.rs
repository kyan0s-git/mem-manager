//! Thin Direct2D / DirectWrite wrapper: device resources are created when the
//! flyout opens and dropped when it closes.

use crate::palette::Rgba;
use windows::Win32::Foundation::HWND;
use windows::Win32::Graphics::Direct2D::Common::{
    D2D_RECT_F, D2D_SIZE_F, D2D_SIZE_U, D2D1_ALPHA_MODE_PREMULTIPLIED, D2D1_COLOR_F,
    D2D1_FIGURE_BEGIN_FILLED, D2D1_FIGURE_BEGIN_HOLLOW, D2D1_FIGURE_END_CLOSED,
    D2D1_FIGURE_END_OPEN, D2D1_PIXEL_FORMAT,
};
use windows::Win32::Graphics::Direct2D::{
    D2D1_ANTIALIAS_MODE_PER_PRIMITIVE, D2D1_ARC_SEGMENT, D2D1_ARC_SIZE_LARGE, D2D1_ARC_SIZE_SMALL,
    D2D1_CAP_STYLE_ROUND, D2D1_DRAW_TEXT_OPTIONS_CLIP, D2D1_ELLIPSE,
    D2D1_FACTORY_TYPE_SINGLE_THREADED, D2D1_HWND_RENDER_TARGET_PROPERTIES, D2D1_LINE_JOIN_ROUND,
    D2D1_PRESENT_OPTIONS_NONE, D2D1_RENDER_TARGET_PROPERTIES, D2D1_RENDER_TARGET_TYPE_DEFAULT,
    D2D1_ROUNDED_RECT, D2D1_STROKE_STYLE_PROPERTIES, D2D1_SWEEP_DIRECTION_CLOCKWISE,
    D2D1CreateFactory, ID2D1Factory, ID2D1HwndRenderTarget, ID2D1SolidColorBrush, ID2D1StrokeStyle,
};
use windows::Win32::Graphics::DirectWrite::{
    DWRITE_FACTORY_TYPE_SHARED, DWRITE_FONT_STRETCH_NORMAL, DWRITE_FONT_STYLE_NORMAL,
    DWRITE_FONT_WEIGHT, DWRITE_FONT_WEIGHT_NORMAL, DWRITE_FONT_WEIGHT_SEMI_BOLD,
    DWRITE_MEASURING_MODE_NATURAL, DWRITE_PARAGRAPH_ALIGNMENT_CENTER,
    DWRITE_PARAGRAPH_ALIGNMENT_NEAR, DWRITE_TEXT_ALIGNMENT, DWRITE_TEXT_ALIGNMENT_CENTER,
    DWRITE_TEXT_ALIGNMENT_LEADING, DWRITE_TEXT_ALIGNMENT_TRAILING, DWRITE_TEXT_METRICS,
    DWRITE_TRIMMING, DWRITE_TRIMMING_GRANULARITY_CHARACTER, DWRITE_WORD_WRAPPING_NO_WRAP,
    DWRITE_WORD_WRAPPING_WRAP, DWriteCreateFactory, IDWriteFactory, IDWriteFontCollection,
    IDWriteTextFormat,
};
use windows::Win32::Graphics::Dxgi::Common::DXGI_FORMAT_B8G8R8A8_UNORM;
use windows::core::{BOOL, HSTRING, Result};
use windows_numerics::Vector2;

#[derive(Clone, Copy, Debug, PartialEq, Default)]
pub struct Rect {
    pub x: f32,
    pub y: f32,
    pub w: f32,
    pub h: f32,
}

impl Rect {
    pub const fn new(x: f32, y: f32, w: f32, h: f32) -> Rect {
        Rect { x, y, w, h }
    }
    pub fn contains(&self, px: f32, py: f32) -> bool {
        px >= self.x && px < self.x + self.w && py >= self.y && py < self.y + self.h
    }
    pub fn right(&self) -> f32 {
        self.x + self.w
    }
    pub fn bottom(&self) -> f32 {
        self.y + self.h
    }
    pub fn inset(&self, d: f32) -> Rect {
        Rect::new(
            self.x + d,
            self.y + d,
            (self.w - 2.0 * d).max(0.0),
            (self.h - 2.0 * d).max(0.0),
        )
    }
    fn d2d(&self) -> D2D_RECT_F {
        D2D_RECT_F {
            left: self.x,
            top: self.y,
            right: self.x + self.w,
            bottom: self.y + self.h,
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Font {
    Caption,
    CaptionStrong,
    Body,
    BodyStrong,
    Title,
    Header,
    Icon,
    IconSmall,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Align {
    Left,
    Center,
    Right,
}

fn color(c: Rgba) -> D2D1_COLOR_F {
    D2D1_COLOR_F {
        r: c.r,
        g: c.g,
        b: c.b,
        a: c.a,
    }
}

fn v2(x: f32, y: f32) -> Vector2 {
    Vector2 { X: x, Y: y }
}

pub struct Gfx {
    factory: ID2D1Factory,
    dwrite: IDWriteFactory,
    pub rt: ID2D1HwndRenderTarget,
    brush: ID2D1SolidColorBrush,
    round: ID2D1StrokeStyle,
    formats: Vec<(Font, IDWriteTextFormat)>,
}

fn family_exists(coll: &IDWriteFontCollection, name: &str) -> bool {
    let mut idx = 0u32;
    let mut exists = BOOL(0);
    unsafe {
        coll.FindFamilyName(&HSTRING::from(name), &mut idx, &mut exists)
            .is_ok()
            && exists.as_bool()
    }
}

impl Gfx {
    pub fn new(hwnd: HWND, width_px: u32, height_px: u32, dpi: f32) -> Result<Gfx> {
        unsafe {
            let factory: ID2D1Factory = D2D1CreateFactory(D2D1_FACTORY_TYPE_SINGLE_THREADED, None)?;
            let dwrite: IDWriteFactory = DWriteCreateFactory(DWRITE_FACTORY_TYPE_SHARED)?;
            let props = D2D1_RENDER_TARGET_PROPERTIES {
                r#type: D2D1_RENDER_TARGET_TYPE_DEFAULT,
                pixelFormat: D2D1_PIXEL_FORMAT {
                    format: DXGI_FORMAT_B8G8R8A8_UNORM,
                    alphaMode: D2D1_ALPHA_MODE_PREMULTIPLIED,
                },
                dpiX: dpi,
                dpiY: dpi,
                ..Default::default()
            };
            let hprops = D2D1_HWND_RENDER_TARGET_PROPERTIES {
                hwnd,
                pixelSize: D2D_SIZE_U {
                    width: width_px,
                    height: height_px,
                },
                presentOptions: D2D1_PRESENT_OPTIONS_NONE,
            };
            let rt = factory.CreateHwndRenderTarget(&props, &hprops)?;
            let brush = rt.CreateSolidColorBrush(&color(Rgba::rgb(0)), None)?;
            let round = factory.CreateStrokeStyle(
                &D2D1_STROKE_STYLE_PROPERTIES {
                    startCap: D2D1_CAP_STYLE_ROUND,
                    endCap: D2D1_CAP_STYLE_ROUND,
                    lineJoin: D2D1_LINE_JOIN_ROUND,
                    ..Default::default()
                },
                None,
            )?;

            let mut coll: Option<IDWriteFontCollection> = None;
            dwrite.GetSystemFontCollection(&mut coll, false)?;
            let coll = coll.ok_or_else(windows::core::Error::empty)?;
            let text_family = if family_exists(&coll, "Segoe UI Variable Text") {
                "Segoe UI Variable Text"
            } else {
                "Segoe UI"
            };
            let display_family = if family_exists(&coll, "Segoe UI Variable Display") {
                "Segoe UI Variable Display"
            } else {
                "Segoe UI"
            };
            let icon_family = if family_exists(&coll, "Segoe Fluent Icons") {
                "Segoe Fluent Icons"
            } else {
                "Segoe MDL2 Assets"
            };
            let mk = |family: &str,
                      weight: DWRITE_FONT_WEIGHT,
                      size: f32|
             -> Result<IDWriteTextFormat> {
                let f = dwrite.CreateTextFormat(
                    &HSTRING::from(family),
                    None,
                    weight,
                    DWRITE_FONT_STYLE_NORMAL,
                    DWRITE_FONT_STRETCH_NORMAL,
                    size,
                    &HSTRING::from("en-us"),
                )?;
                f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP)?;
                f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER)?;
                let sign = dwrite.CreateEllipsisTrimmingSign(&f)?;
                let trim = DWRITE_TRIMMING {
                    granularity: DWRITE_TRIMMING_GRANULARITY_CHARACTER,
                    delimiter: 0,
                    delimiterCount: 0,
                };
                f.SetTrimming(&trim, &sign)?;
                Ok(f)
            };
            let formats = vec![
                (
                    Font::Caption,
                    mk(text_family, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
                ),
                (
                    Font::CaptionStrong,
                    mk(text_family, DWRITE_FONT_WEIGHT_SEMI_BOLD, 12.0)?,
                ),
                (
                    Font::Body,
                    mk(text_family, DWRITE_FONT_WEIGHT_NORMAL, 14.0)?,
                ),
                (
                    Font::BodyStrong,
                    mk(text_family, DWRITE_FONT_WEIGHT_SEMI_BOLD, 14.0)?,
                ),
                (
                    Font::Title,
                    mk(display_family, DWRITE_FONT_WEIGHT_SEMI_BOLD, 16.0)?,
                ),
                (
                    Font::Header,
                    mk(display_family, DWRITE_FONT_WEIGHT_SEMI_BOLD, 18.0)?,
                ),
                (
                    Font::Icon,
                    mk(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 16.0)?,
                ),
                (
                    Font::IconSmall,
                    mk(icon_family, DWRITE_FONT_WEIGHT_NORMAL, 12.0)?,
                ),
            ];
            Ok(Gfx {
                factory,
                dwrite,
                rt,
                brush,
                round,
                formats,
            })
        }
    }

    fn fmt(&self, f: Font) -> &IDWriteTextFormat {
        &self
            .formats
            .iter()
            .find(|(k, _)| *k == f)
            .expect("all fonts created")
            .1
    }

    pub fn resize(&self, w: u32, h: u32) {
        unsafe {
            let _ = self.rt.Resize(&D2D_SIZE_U {
                width: w,
                height: h,
            });
        }
    }

    pub fn set_dpi(&self, dpi: f32) {
        unsafe { self.rt.SetDpi(dpi, dpi) };
    }

    pub fn begin(&self, clear: Rgba) {
        unsafe {
            self.rt.BeginDraw();
            self.rt.Clear(Some(&color(clear)));
        }
    }

    /// Returns false when the device was lost and resources must be recreated.
    pub fn end(&self) -> bool {
        unsafe { self.rt.EndDraw(None, None).is_ok() }
    }

    fn set(&self, c: Rgba) {
        unsafe { self.brush.SetColor(&color(c)) };
    }

    pub fn fill_round(&self, r: Rect, radius: f32, c: Rgba) {
        self.set(c);
        let rr = D2D1_ROUNDED_RECT {
            rect: r.d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.rt.FillRoundedRectangle(&rr, &self.brush) };
    }

    pub fn stroke_round(&self, r: Rect, radius: f32, c: Rgba, width: f32) {
        self.set(c);
        let h = width / 2.0;
        let rr = D2D1_ROUNDED_RECT {
            rect: Rect::new(r.x + h, r.y + h, r.w - width, r.h - width).d2d(),
            radiusX: radius,
            radiusY: radius,
        };
        unsafe { self.rt.DrawRoundedRectangle(&rr, &self.brush, width, None) };
    }

    pub fn line(&self, x0: f32, y0: f32, x1: f32, y1: f32, c: Rgba, width: f32) {
        self.set(c);
        unsafe {
            self.rt
                .DrawLine(v2(x0, y0), v2(x1, y1), &self.brush, width, &self.round)
        };
    }

    pub fn circle(&self, cx: f32, cy: f32, r: f32, c: Rgba) {
        self.set(c);
        let e = D2D1_ELLIPSE {
            point: v2(cx, cy),
            radiusX: r,
            radiusY: r,
        };
        unsafe { self.rt.FillEllipse(&e, &self.brush) };
    }

    /// Arc from 12 o'clock clockwise covering `frac` of the circle.
    pub fn arc(&self, cx: f32, cy: f32, r: f32, frac: f32, c: Rgba, width: f32) {
        let frac = frac.clamp(0.0, 0.9999);
        if frac <= 0.001 {
            return;
        }
        let a = frac * std::f32::consts::TAU;
        let (sx, sy) = (cx, cy - r);
        let (ex, ey) = (cx + r * a.sin(), cy - r * a.cos());
        unsafe {
            let Ok(geo) = self.factory.CreatePathGeometry() else {
                return;
            };
            let Ok(sink) = geo.Open() else { return };
            sink.BeginFigure(v2(sx, sy), D2D1_FIGURE_BEGIN_HOLLOW);
            sink.AddArc(&D2D1_ARC_SEGMENT {
                point: v2(ex, ey),
                size: D2D_SIZE_F {
                    width: r,
                    height: r,
                },
                rotationAngle: 0.0,
                sweepDirection: D2D1_SWEEP_DIRECTION_CLOCKWISE,
                arcSize: if frac > 0.5 {
                    D2D1_ARC_SIZE_LARGE
                } else {
                    D2D1_ARC_SIZE_SMALL
                },
            });
            sink.EndFigure(D2D1_FIGURE_END_OPEN);
            if sink.Close().is_ok() {
                self.set(c);
                self.rt.DrawGeometry(&geo, &self.brush, width, &self.round);
            }
        }
    }

    /// Polyline sparkline with a soft fill underneath.
    pub fn sparkline(&self, r: Rect, values: &[f32], line: Rgba, fill: Rgba) {
        if values.len() < 2 {
            return;
        }
        let n = values.len() as f32 - 1.0;
        let pt = |i: usize, v: f32| {
            v2(
                r.x + r.w * i as f32 / n,
                r.bottom() - r.h * v.clamp(0.0, 1.0),
            )
        };
        unsafe {
            if let Ok(geo) = self.factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(v2(r.x, r.bottom()), D2D1_FIGURE_BEGIN_FILLED);
                    for (i, &v) in values.iter().enumerate() {
                        sink.AddLine(pt(i, v));
                    }
                    sink.AddLine(v2(r.right(), r.bottom()));
                    sink.EndFigure(D2D1_FIGURE_END_CLOSED);
                    if sink.Close().is_ok() {
                        self.set(fill);
                        self.rt.FillGeometry(&geo, &self.brush, None);
                    }
                }
            }
            if let Ok(geo) = self.factory.CreatePathGeometry() {
                if let Ok(sink) = geo.Open() {
                    sink.BeginFigure(pt(0, values[0]), D2D1_FIGURE_BEGIN_HOLLOW);
                    for (i, &v) in values.iter().enumerate().skip(1) {
                        sink.AddLine(pt(i, v));
                    }
                    sink.EndFigure(D2D1_FIGURE_END_OPEN);
                    if sink.Close().is_ok() {
                        self.set(line);
                        self.rt.DrawGeometry(&geo, &self.brush, 1.5, &self.round);
                    }
                }
            }
        }
    }

    pub fn text(&self, s: &str, r: Rect, font: Font, c: Rgba, align: Align) {
        let f = self.fmt(font);
        let a: DWRITE_TEXT_ALIGNMENT = match align {
            Align::Left => DWRITE_TEXT_ALIGNMENT_LEADING,
            Align::Center => DWRITE_TEXT_ALIGNMENT_CENTER,
            Align::Right => DWRITE_TEXT_ALIGNMENT_TRAILING,
        };
        let w: Vec<u16> = s.encode_utf16().collect();
        self.set(c);
        unsafe {
            let _ = f.SetTextAlignment(a);
            self.rt.DrawText(
                &w,
                f,
                &r.d2d(),
                &self.brush,
                D2D1_DRAW_TEXT_OPTIONS_CLIP,
                DWRITE_MEASURING_MODE_NATURAL,
            );
        }
    }

    /// Wrapped multi-line text; returns the height used.
    pub fn text_wrapped(&self, s: &str, r: Rect, font: Font, c: Rgba) -> f32 {
        let f = self.fmt(font);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_NEAR);
            let _ = f.SetTextAlignment(DWRITE_TEXT_ALIGNMENT_LEADING);
            let mut h = 0.0;
            if let Ok(layout) = self.dwrite.CreateTextLayout(&w, f, r.w, 1000.0) {
                let mut m = DWRITE_TEXT_METRICS::default();
                if layout.GetMetrics(&mut m).is_ok() {
                    h = m.height;
                }
                self.set(c);
                self.rt.DrawTextLayout(
                    v2(r.x, r.y),
                    &layout,
                    &self.brush,
                    D2D1_DRAW_TEXT_OPTIONS_CLIP,
                );
            }
            let _ = f.SetWordWrapping(DWRITE_WORD_WRAPPING_NO_WRAP);
            let _ = f.SetParagraphAlignment(DWRITE_PARAGRAPH_ALIGNMENT_CENTER);
            h
        }
    }

    pub fn text_width(&self, s: &str, font: Font) -> f32 {
        let f = self.fmt(font);
        let w: Vec<u16> = s.encode_utf16().collect();
        unsafe {
            match self.dwrite.CreateTextLayout(&w, f, 10_000.0, 100.0) {
                Ok(layout) => {
                    let mut m = DWRITE_TEXT_METRICS::default();
                    if layout.GetMetrics(&mut m).is_ok() {
                        m.widthIncludingTrailingWhitespace
                    } else {
                        0.0
                    }
                }
                Err(_) => 0.0,
            }
        }
    }

    pub fn clip(&self, r: Rect) {
        unsafe {
            self.rt
                .PushAxisAlignedClip(&r.d2d(), D2D1_ANTIALIAS_MODE_PER_PRIMITIVE)
        };
    }

    pub fn unclip(&self) {
        unsafe { self.rt.PopAxisAlignedClip() };
    }
}
