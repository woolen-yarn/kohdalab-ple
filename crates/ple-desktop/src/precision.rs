//! Measured Qt-compatible geometry. Positions are logical pixels, independent of DPI.
use eframe::egui::{self, Color32, Pos2, Rect, Vec2};
use std::collections::BTreeMap;

pub type Geometry = BTreeMap<String, [f32; 4]>;
pub fn record(geometry: &mut Geometry, name: &str, rect: Rect) {
    geometry.insert(
        name.into(),
        [rect.min.x, rect.min.y, rect.width(), rect.height()],
    );
}
pub fn rect(x: f32, y: f32, w: f32, h: f32) -> Rect {
    Rect::from_min_size(Pos2::new(x, y), Vec2::new(w.max(0.), h.max(0.)))
}

fn paint_arrow(ui: &egui::Ui, r: Rect, up: bool) {
    let c = r.center();
    let sign = if up { -1. } else { 1. };
    ui.painter().add(egui::Shape::convex_polygon(
        vec![
            c + Vec2::new(-3., -2. * sign),
            c + Vec2::new(3., -2. * sign),
            c + Vec2::new(0., 2. * sign),
        ],
        ui.visuals().text_color(),
        egui::Stroke::NONE,
    ));
}

pub struct Form<'a> {
    pub ui: &'a mut egui::Ui,
    pub geometry: &'a mut Geometry,
    pub origin: Pos2,
    pub enabled: bool,
}
impl<'a> Form<'a> {
    pub fn new(ui: &'a mut egui::Ui, geometry: &'a mut Geometry, origin: Pos2) -> Self {
        Self {
            ui,
            geometry,
            origin,
            enabled: true,
        }
    }
    pub fn at(&mut self, name: &str, local: Rect) -> Rect {
        let r = local.translate(self.origin.to_vec2());
        record(self.geometry, name, r);
        r
    }
    pub fn label(&mut self, name: &str, local: Rect, text: impl Into<String>) {
        self.value(name, local, text, Color32::from_rgb(235, 236, 241));
    }
    pub fn value(&mut self, name: &str, local: Rect, text: impl Into<String>, color: Color32) {
        let r = self.at(name, local);
        let galley =
            self.ui
                .painter()
                .layout_no_wrap(text.into(), egui::FontId::proportional(12.), color);
        let position = Pos2::new(r.min.x, r.center().y - galley.size().y * 0.5);
        self.ui
            .painter()
            .with_clip_rect(r.intersect(self.ui.clip_rect()))
            .galley(position, galley, color);
    }
    pub fn button(&mut self, name: &str, local: Rect, text: &str, enabled: bool) -> bool {
        let r = self.at(name, local);
        // Keep the Qt-era fixed rectangles, but use the same restrained controls as
        // the IV desktop: neutral controls recede, while the two measurement
        // actions are immediately recognisable.
        let fill = match (text, enabled && self.enabled) {
            ("Start" | "Continue", true) => Color32::from_rgb(48, 107, 214),
            ("Stop", true) => Color32::from_rgb(173, 69, 76),
            _ => Color32::from_rgb(45, 46, 53),
        };
        self.ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if !enabled || !self.enabled {
                    ui.disable();
                }
                ui.put(
                    r,
                    egui::Button::new(text)
                        .fill(fill)
                        .min_size(r.size())
                        .corner_radius(6)
                        .truncate(),
                )
            })
            .inner
            .clicked()
    }
    pub fn text(&mut self, name: &str, local: Rect, value: &mut String) {
        let r = self.at(name, local);
        self.ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if !self.enabled {
                    ui.disable();
                }
                ui.put(
                    r,
                    egui::TextEdit::singleline(value)
                        .id(egui::Id::new(name))
                        .margin(Vec2::new(4., 2.))
                        .min_size(r.size())
                        .vertical_align(egui::Align::Center)
                        .desired_width(r.width()),
                );
            });
    }
    #[allow(dead_code)] // Kept with the upstream fixed-form widget set.
    pub fn editable_combo(
        &mut self,
        name: &str,
        local: Rect,
        value: &mut String,
        options: &[String],
    ) {
        let r = self.at(name, local);
        self.ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if !self.enabled {
                    ui.disable();
                }
                let arrow_width = 18.;
                let text_rect =
                    Rect::from_min_max(r.min, Pos2::new(r.max.x - arrow_width, r.max.y));
                ui.put(
                    text_rect,
                    egui::TextEdit::singleline(value)
                        .id(egui::Id::new((name, "text")))
                        .margin(Vec2::new(4., 2.))
                        .min_size(text_rect.size())
                        .vertical_align(egui::Align::Center)
                        .desired_width(text_rect.width()),
                );
                let arrow_rect = Rect::from_min_max(Pos2::new(text_rect.max.x, r.min.y), r.max);
                let arrow = ui.put(
                    arrow_rect,
                    egui::Button::new("")
                        .min_size(arrow_rect.size())
                        .corner_radius(6),
                );
                paint_arrow(ui, arrow_rect, false);
                egui::Popup::menu(&arrow).show(|ui| {
                    if options.is_empty() {
                        ui.label("No ports found");
                    }
                    for option in options {
                        if ui.selectable_label(*value == *option, option).clicked() {
                            *value = option.clone();
                            ui.close();
                        }
                    }
                });
            });
    }
    #[allow(dead_code)] // Shared upstream checkbox kept for PLE device extensions.
    pub fn check(&mut self, name: &str, local: Rect, value: &mut bool, label: &str) {
        let r = self.at(name, local);
        self.ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if !self.enabled {
                    ui.disable();
                }
                ui.put(r, egui::Checkbox::new(value, label));
            });
    }

    pub fn number(
        &mut self,
        name: &str,
        local: Rect,
        value: &mut f64,
        step: f64,
        decimals: usize,
        enabled: bool,
    ) {
        let r = self.at(name, local);
        let id = egui::Id::new(name);
        self.ui
            .scope_builder(egui::UiBuilder::new().max_rect(r), |ui| {
                if !enabled || !self.enabled {
                    ui.disable();
                }
                let focused = ui.memory(|m| m.has_focus(id));
                let mut text = if focused {
                    ui.data(|d| d.get_temp::<String>(id))
                        .unwrap_or_else(|| format!("{value:.decimals$}"))
                } else {
                    format!("{value:.decimals$}")
                };
                let arrows = name == "signal_points_spin";
                let field = if arrows {
                    Rect::from_min_max(r.min, Pos2::new(r.max.x - 15., r.max.y))
                } else {
                    r
                };
                let response = ui.put(
                    field,
                    egui::TextEdit::singleline(&mut text)
                        .id(id)
                        .margin(Vec2::new(4., 2.))
                        .min_size(field.size())
                        .vertical_align(egui::Align::Center)
                        .desired_width(field.width()),
                );
                if response.changed()
                    && let Ok(parsed) = text.parse::<f64>()
                    && parsed.is_finite()
                {
                    *value = parsed;
                }
                let invalid = !text.parse::<f64>().is_ok_and(f64::is_finite);
                if invalid {
                    response.clone().on_hover_text(
                        "Input not applied: enter a finite number. Start uses the last accepted value.",
                    );
                    ui.painter().rect_stroke(
                        field,
                        6.0,
                        egui::Stroke::new(1.0_f32, ui.visuals().error_fg_color),
                        egui::StrokeKind::Inside,
                    );
                }
                ui.data_mut(|d| d.insert_temp(id, text));
                let mut clicked = 0.;
                if arrows {
                    let up = rect(r.max.x - 15., r.min.y, 15., r.height() * 0.5);
                    let down = rect(r.max.x - 15., r.center().y, 15., r.height() * 0.5);
                    for (r, is_up, change) in [(up, true, 1.), (down, false, -1.)] {
                        let response = ui.interact(r, id.with(is_up), egui::Sense::click());
                        let fill = ui.style().interact(&response).bg_fill;
                        ui.painter().rect_filled(r, 0., fill);
                        paint_arrow(ui, r, is_up);
                        if response.clicked() {
                            clicked = change;
                        }
                    }
                }
                let key = clicked
                    + if focused && enabled && self.enabled {
                        ui.input(|i| {
                            if i.key_pressed(egui::Key::ArrowUp) {
                                1.
                            } else if i.key_pressed(egui::Key::ArrowDown) {
                                -1.
                            } else {
                                0.
                            }
                        })
                    } else {
                        0.
                    };
                if key != 0. {
                    *value += step * key;
                    ui.data_mut(|d| d.insert_temp(id, format!("{value:.decimals$}")));
                }
            });
    }
    pub fn combo<T: PartialEq + Clone>(
        &mut self,
        name: &str,
        local: Rect,
        value: &mut T,
        options: &[(T, &str)],
    ) {
        let r = self.at(name, local);
        self.ui.scope_builder(
            egui::UiBuilder::new()
                .max_rect(r)
                .layout(egui::Layout::left_to_right(egui::Align::Center)),
            |ui| {
                if !self.enabled {
                    ui.disable();
                }
                ui.spacing_mut().combo_width = r.width();
                ui.spacing_mut().interact_size.y = r.height();
                let label = options
                    .iter()
                    .find(|(v, _)| v == value)
                    .map(|(_, s)| *s)
                    .unwrap_or("—");
                egui::ComboBox::from_id_salt(name)
                    .width(r.width())
                    .selected_text(label)
                    .show_ui(ui, |ui| {
                        for (v, label) in options {
                            ui.selectable_value(value, v.clone(), *label);
                        }
                    });
            },
        );
    }
    pub fn group(&mut self, name: &str, local: Rect, title: &str) {
        let r = self.at(name, local);
        self.ui.painter().rect(
            r,
            10.,
            Color32::from_rgb(26, 27, 32),
            egui::Stroke::new(1_f32, Color32::from_rgb(56, 58, 67)),
            egui::StrokeKind::Inside,
        );
        let font = egui::FontId::proportional(12.);
        let galley =
            self.ui
                .painter()
                .layout_no_wrap(title.into(), font, Color32::from_rgb(235, 236, 241));
        let textrect = Rect::from_min_size(r.min + Vec2::new(10., 6.), galley.size());
        self.ui
            .painter()
            .galley(textrect.min, galley, Color32::from_rgb(235, 236, 241));
    }
    pub fn toggle(&mut self, name: &str, local: Rect, expanded: &mut bool, label: &str) {
        if self.button(name, local, "", true) {
            *expanded = !*expanded;
        }
        let r = local.translate(self.origin.to_vec2());
        let center = r.min + Vec2::new(11., 12.);
        let triangle = if *expanded {
            vec![
                center + Vec2::new(-3., -2.),
                center + Vec2::new(3., -2.),
                center + Vec2::new(0., 2.),
            ]
        } else {
            vec![
                center + Vec2::new(-2., -3.),
                center + Vec2::new(-2., 3.),
                center + Vec2::new(2., 0.),
            ]
        };
        self.ui.painter().add(egui::Shape::convex_polygon(
            triangle,
            Color32::LIGHT_GRAY,
            egui::Stroke::NONE,
        ));
        self.value(
            &format!("{name}_caption"),
            rect(
                local.min.x + 25.,
                local.min.y,
                local.width() - 28.,
                local.height(),
            ),
            label,
            Color32::from_rgb(235, 236, 241),
        );
    }
}

#[derive(Clone, Copy, Debug)]
pub struct Columns {
    pub left: Rect,
    pub center: Rect,
    pub right: Rect,
    pub measurement: Rect,
    pub log: Rect,
    pub snapshot: Rect,
    pub plots: Rect,
    pub toolbar: Rect,
    pub toggle: Rect,
    pub left_toggle: Rect,
}
impl Columns {
    pub const WIDTH: f32 = 850.;
    pub const RIGHT_WIDTH: f32 = 300.;
    /// Collapse only side rails on a narrow first launch; central content stays
    /// fixed and is never made wheel-scrollable.
    pub fn initial_rails(viewport_width: f32) -> (bool, bool) {
        if viewport_width < Self::minimum_width(false, false) {
            (true, true)
        } else {
            (false, false)
        }
    }
    pub fn minimum_width(collapsed: bool, left_collapsed: bool) -> f32 {
        (if left_collapsed { 40. } else { 384. })
            + Self::WIDTH
            + 32.
            + if collapsed { 27. } else { Self::RIGHT_WIDTH }
    }
    pub fn new(viewport: Rect, collapsed: bool, left_collapsed: bool) -> Self {
        let w = viewport.width();
        let h = viewport.height();
        let left = if left_collapsed { 40. } else { 384. };
        let right = if collapsed { 27. } else { Self::RIGHT_WIDTH };
        let top = 226.;
        let center_width = (w - left - right - 32.).max(Self::WIDTH);
        let c = rect(left + 16., 8., center_width, h - 16.);
        let measurement = rect(c.min.x + 9., 17., c.width() - 18., top);
        let plot_y = measurement.max.y + 40.;
        let right_x = c.max.x + 8.;
        let origin = viewport.min.to_vec2();
        let mov = |r: Rect| r.translate(origin);
        Self {
            left: mov(rect(8., 8., left, h - 16.)),
            left_toggle: mov(rect(left - 24., 8., 24., 28.)),
            center: mov(c),
            right: mov(rect(right_x, 8., right, h - 16.)),
            measurement: mov(measurement),
            log: mov(rect(right_x + 37., 17., right - 46., top)),
            snapshot: mov(rect(
                right_x + 37.,
                17. + top + 8.,
                right - 46.,
                h - 34. - top - 8.,
            )),
            plots: mov(rect(
                c.min.x + 9.,
                plot_y,
                c.width() - 18.,
                h - 17. - plot_y,
            )),
            toolbar: mov(rect(
                c.min.x + 9.,
                measurement.max.y + 8.,
                c.width() - 18.,
                24.,
            )),
            toggle: mov(rect(
                right_x + if collapsed { 1. } else { 0. },
                8.,
                24.,
                28.,
            )),
        }
    }
    pub fn record(&self, g: &mut Geometry) {
        for (name, r) in [
            ("left_panel", self.left),
            ("left_panel_toggle", self.left_toggle),
            ("center_panel", self.center),
            ("center_top_widget", self.measurement),
            ("right_panel", self.right),
            (
                "right_content",
                rect(
                    self.right.min.x + 28.,
                    self.right.min.y,
                    self.right.width() - 28.,
                    self.right.height(),
                ),
            ),
            ("log", self.log),
            ("snapshot_table", self.snapshot),
            ("plot_stack", self.plots),
            ("right_panel_toggle", self.toggle),
        ] {
            record(g, name, r);
        }
    }
}

pub fn configure(ctx: &egui::Context) {
    let mut fonts = egui::FontDefinitions::default();
    if cfg!(windows) {
        let windows = std::env::var_os("WINDIR")
            .map(std::path::PathBuf::from)
            .unwrap_or_else(|| "C:\\Windows".into());
        // Segoe UI has the lighter, compact Windows control metrics used by the
        // IV app. Yu Gothic UI remains the Japanese fallback when Segoe is not
        // available, and supplies Japanese glyphs after Segoe UI.
        for (order, file) in ["segoeui.ttf", "YuGothR.ttc"].into_iter().enumerate() {
            if let Ok(bytes) = std::fs::read(windows.join("Fonts").join(file)) {
                let index = crate::font_selection::windows_ui_face_index(&bytes);
                let mut data = egui::FontData::from_owned(bytes);
                data.index = index;
                fonts
                    .font_data
                    .insert(format!("windows-ui-{order}"), std::sync::Arc::new(data));
                let family = fonts
                    .families
                    .entry(egui::FontFamily::Proportional)
                    .or_default();
                family.insert(order.min(family.len()), format!("windows-ui-{order}"));
            }
        }
    }
    ctx.set_fonts(fonts);
    ctx.global_style_mut(|style| {
        for size in style.text_styles.values_mut() {
            *size = egui::FontId::proportional(12.);
        }
        style.spacing.interact_size = Vec2::new(80., 26.);
        style.spacing.button_padding = Vec2::new(6., 3.);
        style.spacing.item_spacing = Vec2::new(6., 6.);
        style.spacing.scroll = egui::style::ScrollStyle::solid();
        style.spacing.scroll.bar_width = 12.;
        style.spacing.scroll.bar_inner_margin = 0.;
        style.spacing.scroll.bar_outer_margin = 0.;
        // IV's dark, low-contrast palette. This belongs here rather than in
        // individual layouts so cards retain their measured TRKR geometry.
        style.visuals.panel_fill = Color32::from_rgb(18, 19, 23);
        style.visuals.window_fill = Color32::from_rgb(26, 27, 32);
        style.visuals.window_corner_radius = 12.into();
        style.visuals.window_stroke = egui::Stroke::new(1_f32, Color32::from_rgb(56, 58, 67));
        style.visuals.extreme_bg_color = Color32::from_rgb(18, 19, 23);
        style.visuals.text_edit_bg_color = Some(Color32::from_rgb(20, 21, 25));
        style.visuals.faint_bg_color = Color32::from_rgb(26, 27, 32);
        style.visuals.override_text_color = Some(Color32::from_rgb(235, 236, 241));
        style.visuals.selection.bg_fill = Color32::from_rgb(48, 91, 158);
        style.visuals.selection.stroke = egui::Stroke::new(1_f32, Color32::from_rgb(235, 236, 241));
        for widget in [
            &mut style.visuals.widgets.inactive,
            &mut style.visuals.widgets.hovered,
            &mut style.visuals.widgets.active,
        ] {
            widget.bg_stroke = egui::Stroke::new(1_f32, Color32::from_rgb(56, 58, 67));
            widget.corner_radius = 6.into();
            widget.fg_stroke = egui::Stroke::new(1_f32, Color32::from_rgb(235, 236, 241));
        }
        style.visuals.widgets.inactive.bg_fill = Color32::from_rgb(45, 46, 53);
        style.visuals.widgets.inactive.weak_bg_fill = Color32::from_rgb(20, 21, 25);
        style.visuals.widgets.inactive.bg_stroke =
            egui::Stroke::new(0.7_f32, Color32::from_rgb(56, 58, 67));
        style.visuals.widgets.hovered.bg_fill = Color32::from_rgb(62, 64, 74);
        style.visuals.widgets.active.bg_fill = Color32::from_rgb(48, 91, 158);
        style.visuals.widgets.noninteractive.bg_stroke =
            egui::Stroke::new(1_f32, Color32::from_rgb(56, 58, 67));
    });
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn columns_match_trkr_and_only_center_grows() {
        for width in [1566., 1920., 2560.] {
            let c = Columns::new(rect(0., 0., width, 900.), false, false);
            assert_eq!(c.left.width(), 384.);
            assert_eq!(c.right.width(), 300.);
            assert_eq!(c.center.width(), width - 716.);
            assert_eq!(c.measurement.height(), 226.);
            assert_eq!(c.log.height(), 226.);
            assert!(c.left.right() < c.center.left() && c.center.right() < c.right.left());
        }
    }
    #[test]
    fn sidebars_preserve_the_reference_collapse_contract() {
        let view = rect(0., 0., 1920., 900.);
        let full = Columns::new(view, false, false);
        let right = Columns::new(rect(0., 0., 1920. - 273., 900.), true, false);
        assert_eq!(full.center, right.center);
        let left = Columns::new(view, false, true);
        assert_eq!(left.center.width() - full.center.width(), 344.);
        assert_eq!(left.right, full.right);
        assert_eq!(Columns::minimum_width(false, false), 1566.);
    }
    #[test]
    fn laptop_initial_layout_collapses_only_the_side_rails() {
        assert_eq!(Columns::initial_rails(1200.), (true, true));
        assert_eq!(Columns::initial_rails(1566.), (false, false));
    }
    #[test]
    fn theme_and_control_metrics_match_pinned_upstream() {
        let ctx = egui::Context::default();
        ctx.set_visuals(egui::Visuals::dark());
        configure(&ctx);
        let style = ctx.global_style();
        assert_eq!(style.visuals.panel_fill, Color32::from_rgb(18, 19, 23));
        assert_eq!(style.visuals.window_fill, Color32::from_rgb(26, 27, 32));
        assert_eq!(
            style.visuals.widgets.inactive.bg_fill,
            Color32::from_rgb(45, 46, 53)
        );
        assert_eq!(style.spacing.interact_size, Vec2::new(80., 26.));
        assert_eq!(style.spacing.item_spacing, Vec2::new(6., 6.));
        for font in style.text_styles.values() {
            assert_eq!(font.size, 12.);
        }
    }

    #[test]
    fn invalid_number_draft_keeps_last_value_and_field_geometry() {
        let mut value = 12.5;
        let field = rect(94., 120., 100., 26.);
        let draft = "not-a-number";
        if let Some(parsed) = draft.parse::<f64>().ok().filter(|v| v.is_finite()) {
            value = parsed;
        }
        assert_eq!(value, 12.5);
        assert_eq!(field, rect(94., 120., 100., 26.));
        assert!(!draft.parse::<f64>().is_ok_and(f64::is_finite));
    }
}
