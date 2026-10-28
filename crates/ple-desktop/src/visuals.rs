//! Shared KohdaLab IV/TRKR palette. See third_party/NOTICE.md.
use eframe::egui::Color32;
pub const BG: Color32 = Color32::from_rgb(18, 19, 23);
pub const PANEL: Color32 = Color32::from_rgb(26, 27, 32);
pub const BORDER: Color32 = Color32::from_rgb(56, 58, 67);
pub const TEXT: Color32 = Color32::from_rgb(235, 236, 241);
pub const MUTED: Color32 = Color32::from_rgb(160, 163, 176);
pub const BLUE: Color32 = Color32::from_rgb(102, 174, 255);
pub const ORANGE: Color32 = Color32::from_rgb(245, 183, 82);
pub const PURPLE: Color32 = Color32::from_rgb(179, 155, 248);
pub const TEAL: Color32 = Color32::from_rgb(63, 215, 194);
pub const RED: Color32 = Color32::from_rgb(255, 123, 130);
pub const GREEN: Color32 = Color32::from_rgb(100, 196, 146);

#[derive(Clone, Copy, Debug, PartialEq)]
pub struct AxisScale {
    pub factor: f64,
    pub prefix: &'static str,
}
impl AxisScale {
    pub fn for_magnitude(m: f64) -> Self {
        if !m.is_finite() || m <= 0. {
            return Self {
                factor: 1.,
                prefix: "",
            };
        }
        for (factor, prefix) in [
            (1e12, "T"),
            (1e9, "G"),
            (1e6, "M"),
            (1e3, "k"),
            (1., ""),
            (1e-3, "m"),
            (1e-6, "µ"),
            (1e-9, "n"),
            (1e-12, "p"),
        ] {
            if m >= factor * (1. - 1e-12) {
                return Self { factor, prefix };
            }
        }
        Self {
            factor: 1e-12,
            prefix: "p",
        }
    }
    pub fn tick(self, value: f64, spacing: f64) -> String {
        let step = (spacing.abs() / self.factor).max(f64::MIN_POSITIVE);
        let decimals = (-step.log10()).ceil().clamp(0., 6.) as usize;
        format!("{:.*}", decimals, value / self.factor)
    }
}
