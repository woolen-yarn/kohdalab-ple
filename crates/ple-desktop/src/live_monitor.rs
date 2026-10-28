//! Display-only scalar monitors matching the fixed TRKR live-view language.
//!
//! Histories are intentionally reset at connection and acquisition-source
//! boundaries so a chart never joins simulated and hardware measurements.

use eframe::egui::{self, Align, Color32, Layout, Rect, RichText, Stroke, Ui};
use egui_plot::{Line, Plot};
use std::{collections::VecDeque, time::Instant};

pub const BLUE: Color32 = Color32::from_rgb(102, 174, 255);
pub const TEAL: Color32 = Color32::from_rgb(63, 215, 194);
pub const ORANGE: Color32 = Color32::from_rgb(245, 183, 82);
pub const MUTED: Color32 = Color32::from_rgb(146, 150, 164);
pub const BORDER: Color32 = Color32::from_rgb(56, 58, 67);

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Origin {
    Demo,
    Hardware,
}

impl Origin {
    const fn label(self) -> &'static str {
        match self {
            Self::Demo => "SIM",
            Self::Hardware => "MEASURED",
        }
    }
}

/// A monotonic local history for one scalar telemetry channel.
pub struct History {
    samples: VecDeque<(f64, f64)>,
    origin: Option<Origin>,
    started: Instant,
}

impl Default for History {
    fn default() -> Self {
        Self {
            samples: VecDeque::new(),
            origin: None,
            started: Instant::now(),
        }
    }
}

impl History {
    pub fn clear(&mut self) {
        self.samples.clear();
        self.origin = None;
        self.started = Instant::now();
    }

    /// A device reconnect creates a new plotting epoch, just as in TRKR.
    pub fn connection_changed(&mut self, previous: bool, current: bool) {
        if previous != current {
            self.clear();
        }
    }

    /// Appends a scalar from one acquisition provenance.
    ///
    /// Switching demo/hardware input starts a new epoch rather than producing
    /// an ambiguous connected line in the monitor.
    pub fn push(&mut self, value: f64, origin: Origin) {
        if self.origin != Some(origin) {
            self.clear();
            self.origin = Some(origin);
        }
        let now = self.started.elapsed().as_secs_f64();
        self.samples.push_back((now, value));
        while self.samples.len() > 1024
            || self
                .samples
                .front()
                .is_some_and(|(time, _)| now - time > 30.)
        {
            self.samples.pop_front();
        }
    }

    pub fn latest(&self) -> Option<f64> {
        self.samples.back().map(|(_, value)| *value)
    }

    pub fn age(&self) -> Option<f64> {
        self.samples
            .back()
            .map(|(time, _)| (self.started.elapsed().as_secs_f64() - time).max(0.))
    }

    pub fn origin(&self) -> Option<Origin> {
        self.origin
    }

    /// `LIVE` means the latest value arrived in the last two seconds.
    pub fn state(&self, active: bool) -> &'static str {
        match (self.latest(), active, self.recent()) {
            (None, _, _) => "WAITING",
            (Some(_), true, true) => "LIVE",
            (Some(_), true, false) => "STALE",
            (Some(_), false, _) => "HELD",
        }
    }

    fn newest(&self, active: bool) -> f64 {
        if active {
            self.started.elapsed().as_secs_f64()
        } else {
            self.samples
                .back()
                .map(|(time, _)| *time)
                .unwrap_or_else(|| self.started.elapsed().as_secs_f64())
        }
    }

    fn recent(&self) -> bool {
        self.samples
            .back()
            .is_some_and(|(time, _)| self.started.elapsed().as_secs_f64() - time < 2.)
    }

    fn points_at(&self, newest: f64) -> Vec<[f64; 2]> {
        self.samples
            .iter()
            .filter(|(time, _)| newest - time <= 30.)
            .map(|(time, value)| [time - newest, *value])
            .collect()
    }
}

#[derive(Clone, Copy, Debug)]
pub struct LiveChannel {
    pub id: &'static str,
    pub label: &'static str,
    pub unit: &'static str,
    pub color: Color32,
    /// Use SI-prefix engineering units for the axis and numeric readout.
    pub engineering: bool,
}

pub const STAGE_CHANNEL: LiveChannel = LiveChannel {
    id: "ple_conex_live",
    label: "CONEX position",
    unit: "mm",
    color: TEAL,
    engineering: false,
};

pub const POWER_CHANNEL: LiveChannel = LiveChannel {
    id: "ple_power_live",
    label: "Power",
    unit: "W",
    color: ORANGE,
    engineering: true,
};

#[derive(Clone, Copy)]
struct Scale {
    factor: f64,
    prefix: &'static str,
}

impl Scale {
    fn for_magnitude(value: f64) -> Self {
        if !value.is_finite() || value <= 0. {
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
            if value >= factor * (1. - 1e-12) {
                return Self { factor, prefix };
            }
        }
        Self {
            factor: 1e-12,
            prefix: "p",
        }
    }

    fn tick(self, value: f64, spacing: f64) -> String {
        let step = (spacing.abs() / self.factor).max(f64::MIN_POSITIVE);
        let decimals = (-step.log10()).ceil().clamp(0., 6.) as usize;
        format!("{:.prec$}", value / self.factor, prec = decimals)
    }
}

/// Draw one compact, fixed-geometry display-only monitor.
///
/// `active` signals that new telemetry is expected. When false the plot holds
/// its final timestamp and reports `HELD`, without accepting any interaction.
pub fn draw(
    ui: &mut Ui,
    area: Rect,
    history: &History,
    channel: LiveChannel,
    active: bool,
) -> Rect {
    let newest = history.newest(active);
    let window = newest.clamp(2., 30.);
    let points = history.points_at(newest);
    let extent = points
        .iter()
        .fold(0_f64, |max, point| max.max(point[1].abs()));
    let scale = if channel.engineering {
        // Retain the last measured unit while an active stream is stale.
        Scale::for_magnitude(extent.max(history.latest().map_or(0., f64::abs)))
    } else {
        Scale {
            factor: 1.,
            prefix: "",
        }
    };
    let current = history.latest();
    let state = history.state(active);

    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(area)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(area));
            ui.spacing_mut().item_spacing.y = 1.;
            ui.set_min_width(area.width());
            ui.horizontal(|ui| {
                ui.label(RichText::new(channel.label).strong().color(channel.color));
                ui.label(
                    RichText::new(history.origin().map_or("MEASURED", Origin::label))
                        .size(9.)
                        .color(MUTED),
                );
                ui.label(
                    RichText::new(state)
                        .size(9.)
                        .color(if active { BLUE } else { MUTED }),
                );
            });
            let display = current.map_or_else(
                || format!("— {}", channel.unit),
                |value| {
                    format!(
                        "{:.4} {}{}",
                        value / scale.factor,
                        scale.prefix,
                        channel.unit
                    )
                },
            );
            ui.label(
                RichText::new(display)
                    .monospace()
                    .size(22.)
                    .color(channel.color),
            );

            let (bar, _) =
                ui.allocate_exact_size(egui::vec2(area.width(), 6.), egui::Sense::hover());
            let fraction = current
                .filter(|_| extent > 0.)
                .map_or(0., |value| (value / extent).clamp(-1., 1.))
                as f32;
            let center = bar.center().x;
            let end = center + fraction * bar.width() * 0.5;
            let painter = ui.painter();
            painter.rect_filled(bar, 2., BORDER);
            painter.rect_filled(
                Rect::from_min_max(
                    egui::pos2(center.min(end), bar.top()),
                    egui::pos2(center.max(end), bar.bottom()),
                ),
                0.,
                channel.color,
            );
            painter.line_segment(
                [
                    egui::pos2(center, bar.top()),
                    egui::pos2(center, bar.bottom()),
                ],
                Stroke::new(1_f32, Color32::WHITE),
            );

            Plot::new(("ple_live", channel.id, history.origin()))
                .width(area.width())
                .height(55.)
                .show_background(false)
                .show_axes([true, true])
                .show_grid([true, true])
                .allow_drag(false)
                .allow_zoom(false)
                .allow_scroll(false)
                .include_x(-window)
                .include_x(0.)
                .include_y(0.)
                .y_axis_formatter(move |mark, _| scale.tick(mark.value, mark.step_size))
                .label_formatter(|_, _| String::new())
                .show(ui, |plot| {
                    plot.line(
                        Line::new("trend", points)
                            .color(channel.color)
                            .width(1.5_f32)
                            .fill(0_f32)
                            .fill_alpha(0.75_f32),
                    );
                });
            let age = history
                .age()
                .map_or_else(|| "—".into(), |seconds| format!("{seconds:.1} s"));
            ui.painter().text(
                egui::pos2(area.left(), area.bottom()),
                egui::Align2::LEFT_BOTTOM,
                format!(
                    "Time · s ago · {}{} · {age} ago",
                    scale.prefix, channel.unit
                ),
                egui::FontId::proportional(9.),
                MUTED,
            );
        },
    )
    .response
    .rect
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    #[test]
    fn provenance_and_connection_epochs_cannot_mix_history() {
        let mut history = History::default();
        history.push(1., Origin::Demo);
        history.push(2., Origin::Demo);
        assert_eq!(history.samples.len(), 2);

        history.push(3., Origin::Hardware);
        assert_eq!(history.samples.len(), 1);
        assert_eq!(history.latest(), Some(3.));
        assert_eq!(history.origin(), Some(Origin::Hardware));

        history.connection_changed(true, true);
        assert_eq!(history.samples.len(), 1);
        history.connection_changed(true, false);
        assert_eq!(history.latest(), None);
        history.push(4., Origin::Demo);
        history.connection_changed(false, true);
        assert_eq!(history.latest(), None);
    }

    #[test]
    fn history_is_bounded_and_held_uses_its_last_timestamp() {
        let mut history = History::default();
        for value in 0..1030 {
            history.push(f64::from(value), Origin::Hardware);
        }
        assert_eq!(history.samples.len(), 1024);
        let held = history.newest(false);
        assert_eq!(history.points_at(held).len(), 1024);

        history.started -= Duration::from_secs(60);
        assert_eq!(history.state(false), "HELD");
        assert!(history.age().is_some_and(|age| age >= 60.));
        assert!(history.points_at(history.newest(true)).is_empty());
    }

    #[test]
    fn monitor_stays_inside_the_exact_allocated_area() {
        let context = egui::Context::default();
        let mut history = History::default();
        history.push(1.25e-6, Origin::Hardware);
        let _ = context.run(
            egui::RawInput {
                screen_rect: Some(Rect::from_min_size(
                    egui::Pos2::ZERO,
                    egui::vec2(400., 300.),
                )),
                ..Default::default()
            },
            |ctx| {
                egui::CentralPanel::default().show(ctx, |ui| {
                    let area = Rect::from_min_size(ui.max_rect().min, egui::vec2(167., 140.));
                    let used = draw(ui, area, &history, POWER_CHANNEL, false);
                    assert_eq!(used.min, area.min);
                    assert!(used.width() <= area.width() + 0.1, "overflow: {used:?}");
                    assert!(used.height() <= area.height() + 0.1, "overflow: {used:?}");
                });
            },
        );
    }
}
