//! Colour tokens and presets for the status icon and flyout (docs/architecture/ui-design.md §1.3).

use crate::config::ColorPreset;
use memmanager_core::state::PressureState;

/// Straight-alpha colour, components 0..=1.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Rgba {
    pub r: f32,
    pub g: f32,
    pub b: f32,
    pub a: f32,
}

impl Rgba {
    pub const fn rgb(hex: u32) -> Rgba {
        Rgba {
            r: ((hex >> 16) & 0xFF) as f32 / 255.0,
            g: ((hex >> 8) & 0xFF) as f32 / 255.0,
            b: (hex & 0xFF) as f32 / 255.0,
            a: 1.0,
        }
    }
    pub const fn with_a(hex: u32, a: f32) -> Rgba {
        let c = Rgba::rgb(hex);
        Rgba { a, ..c }
    }
    pub fn with_alpha(self, a: f32) -> Rgba {
        Rgba { a, ..self }
    }
    pub fn to_hex(self) -> u32 {
        let c = |v: f32| (v.clamp(0.0, 1.0) * 255.0).round() as u32;
        (c(self.r) << 16) | (c(self.g) << 8) | c(self.b)
    }
    /// Relative luminance (sRGB approximation).
    pub fn luminance(self) -> f32 {
        let lin = |c: f32| {
            if c <= 0.03928 {
                c / 12.92
            } else {
                ((c + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * lin(self.r) + 0.7152 * lin(self.g) + 0.0722 * lin(self.b)
    }
    pub fn mix(self, other: Rgba, t: f32) -> Rgba {
        Rgba {
            r: self.r + (other.r - self.r) * t,
            g: self.g + (other.g - self.g) * t,
            b: self.b + (other.b - self.b) * t,
            a: self.a + (other.a - self.a) * t,
        }
    }
}

/// System appearance relevant to drawing.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct Appearance {
    /// Taskbar uses the light theme (icon background is light).
    pub taskbar_light: bool,
    /// Apps use the light theme (flyout palette).
    pub apps_light: bool,
    pub accent: Rgba,
    pub high_contrast: bool,
}

impl Default for Appearance {
    fn default() -> Self {
        Appearance {
            taskbar_light: false,
            apps_light: false,
            accent: Rgba::rgb(0x0078D4),
            high_contrast: false,
        }
    }
}

/// Icon/state colour for `state` given the preset and the taskbar theme.
pub fn state_color(
    preset: ColorPreset,
    custom: &[u32; 4],
    state: PressureState,
    ap: &Appearance,
) -> Rgba {
    let light = ap.taskbar_light;
    let fg = if light {
        Rgba::rgb(0x1A1A1A)
    } else {
        Rgba::rgb(0xFFFFFF)
    };
    if ap.high_contrast {
        return fg;
    }
    let pick = |l: u32, d: u32| Rgba::rgb(if light { l } else { d });
    let accent = accent_for(ap.accent, light);
    match preset {
        ColorPreset::Mono => fg,
        ColorPreset::Custom => Rgba::rgb(custom[state as usize]),
        ColorPreset::ColorBlind => match state {
            PressureState::Normal => pick(0x0A6CD6, 0x56A8FF),
            PressureState::Elevated => pick(0xB07800, 0xE69F00),
            PressureState::High | PressureState::Critical => pick(0xC04E00, 0xFF7A33),
        },
        ColorPreset::Traffic => match state {
            PressureState::Normal => pick(0x107C10, 0x6CCB5F),
            PressureState::Elevated => pick(0xB58400, 0xF2C744),
            PressureState::High => pick(0xC8551B, 0xFF8F4D),
            PressureState::Critical => pick(0xC42B1C, 0xFF6B5E),
        },
        ColorPreset::System => match state {
            PressureState::Normal => accent,
            PressureState::Elevated => pick(0xB58400, 0xF2C744),
            PressureState::High => pick(0xC8551B, 0xFF8F4D),
            PressureState::Critical => pick(0xC42B1C, 0xFF6B5E),
        },
    }
}

/// Adjusts the accent so it keeps ≥ 3:1 contrast on the taskbar.
pub fn accent_for(accent: Rgba, light_bg: bool) -> Rgba {
    let bg_l = if light_bg { 0.87 } else { 0.015 };
    let mut c = accent;
    for _ in 0..8 {
        let l = c.luminance();
        let (hi, lo) = if l > bg_l { (l, bg_l) } else { (bg_l, l) };
        if (hi + 0.05) / (lo + 0.05) >= 3.0 {
            break;
        }
        c = if light_bg {
            c.mix(Rgba::rgb(0x000000), 0.25)
        } else {
            c.mix(Rgba::rgb(0xFFFFFF), 0.25)
        };
    }
    c
}

/// Flyout palette.
#[derive(Clone, Copy, Debug)]
pub struct Palette {
    pub bg: Rgba,
    pub bg_solid: Rgba,
    pub text: Rgba,
    pub text2: Rgba,
    pub text3: Rgba,
    pub divider: Rgba,
    pub control: Rgba,
    pub control_hover: Rgba,
    pub border: Rgba,
    pub accent: Rgba,
    pub on_accent: Rgba,
    pub seg_modified: Rgba,
    pub seg_standby: Rgba,
    pub track: Rgba,
    pub warn: Rgba,
    pub danger: Rgba,
    pub good: Rgba,
}

impl Palette {
    pub fn new(ap: &Appearance) -> Palette {
        if ap.apps_light {
            let accent = accent_for(ap.accent, true);
            Palette {
                bg: Rgba::with_a(0xF3F3F3, 0.90),
                bg_solid: Rgba::rgb(0xF3F3F3),
                text: Rgba::rgb(0x1A1A1A),
                text2: Rgba::rgb(0x5D5D5D),
                text3: Rgba::rgb(0x8A8A8A),
                divider: Rgba::with_a(0x000000, 0.08),
                control: Rgba::with_a(0x000000, 0.045),
                control_hover: Rgba::with_a(0x000000, 0.08),
                border: Rgba::with_a(0x000000, 0.12),
                accent,
                on_accent: Rgba::rgb(0xFFFFFF),
                seg_modified: Rgba::rgb(0xD47A00),
                seg_standby: Rgba::rgb(0x7FA7D9),
                track: Rgba::with_a(0x000000, 0.07),
                warn: Rgba::rgb(0xB58400),
                danger: Rgba::rgb(0xC42B1C),
                good: Rgba::rgb(0x107C10),
            }
        } else {
            let accent = accent_for(ap.accent, false);
            Palette {
                bg: Rgba::with_a(0x202020, 0.86),
                bg_solid: Rgba::rgb(0x202020),
                text: Rgba::rgb(0xFFFFFF),
                text2: Rgba::rgb(0xC8C8C8),
                text3: Rgba::rgb(0x9A9A9A),
                divider: Rgba::with_a(0xFFFFFF, 0.08),
                control: Rgba::with_a(0xFFFFFF, 0.06),
                control_hover: Rgba::with_a(0xFFFFFF, 0.10),
                border: Rgba::with_a(0xFFFFFF, 0.10),
                accent,
                on_accent: if accent.luminance() > 0.45 {
                    Rgba::rgb(0x000000)
                } else {
                    Rgba::rgb(0xFFFFFF)
                },
                seg_modified: Rgba::rgb(0xF7A33A),
                seg_standby: Rgba::rgb(0x6F94C4),
                track: Rgba::with_a(0xFFFFFF, 0.08),
                warn: Rgba::rgb(0xF2C744),
                danger: Rgba::rgb(0xFF6B5E),
                good: Rgba::rgb(0x6CCB5F),
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn accent_contrast_is_enforced() {
        let dark_accent = Rgba::rgb(0x003060);
        let adj = accent_for(dark_accent, false);
        assert!(adj.luminance() > dark_accent.luminance());
        let pale = Rgba::rgb(0xCCE4FF);
        assert!(accent_for(pale, true).luminance() < pale.luminance());
    }

    #[test]
    fn hex_round_trip() {
        assert_eq!(Rgba::rgb(0x12AB34).to_hex(), 0x12AB34);
    }
}
