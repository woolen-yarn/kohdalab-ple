//! PLE spectra in the shared TRKR/IV plot shell, with sequential intensity colors.
use super::*;
use crate::{
    precision::{self, rect},
    visuals::*,
};
use eframe::egui::{Align, Align2, Color32, FontId, Layout, Rect, Vec2};
use egui_plot::{Legend, Line, LineStyle, Plot, PlotImage, PlotMemory, PlotPoint, Points, VLine};
const COLORS: [Color32; 6] = [BLUE, ORANGE, PURPLE, TEAL, RED, GREEN];
// Exact 256-sample Matplotlib Inferno LUT, rounded to u8 from the official
// source at matplotlib/lib/matplotlib/_cm_listed.py (_inferno_data).
const INFERNO_HEX: &str = "00000401000501010601010802010a02020c02020e03021004031204031405041706041907051b08051d09061f0a07220b07240c08260d08290e092b10092d110a30120a32140b34150b37160b39180c3c190c3e1b0c411c0c431e0c451f0c48210c4a230c4c240c4f260c51280b53290b552b0b572d0b592f0a5b310a5c320a5e340a5f3609613809623909633b09643d09653e0966400a67420a68440a68450a69470b6a490b6a4a0c6b4c0c6b4d0d6c4f0d6c510e6c520e6d540f6d550f6d57106e59106e5a116e5c126e5d126e5f136e61136e62146e64156e65156e67166e69166e6a176e6c186e6d186e6f196e71196e721a6e741a6e751b6e771c6d781c6d7a1d6d7c1d6d7d1e6d7f1e6c801f6c82206c84206b85216b87216b88226a8a226a8c23698d23698f24699025689225689326679526679727669827669a28659b29649d29649f2a63a02a63a22b62a32c61a52c60a62d60a82e5fa92e5eab2f5ead305dae305cb0315bb1325ab3325ab43359b63458b73557b93556ba3655bc3754bd3853bf3952c03a51c13a50c33b4fc43c4ec63d4dc73e4cc83f4bca404acb4149cc4248ce4347cf4446d04545d24644d34743d44842d54a41d74b3fd84c3ed94d3dda4e3cdb503bdd513ade5238df5337e05536e15635e25734e35933e45a31e55c30e65d2fe75e2ee8602de9612bea632aeb6429eb6628ec6726ed6925ee6a24ef6c23ef6e21f06f20f1711ff1731df2741cf3761bf37819f47918f57b17f57d15f67e14f68013f78212f78410f8850ff8870ef8890cf98b0bf98c0af98e09fa9008fa9207fa9407fb9606fb9706fb9906fb9b06fb9d07fc9f07fca108fca309fca50afca60cfca80dfcaa0ffcac11fcae12fcb014fcb216fcb418fbb61afbb81dfbba1ffbbc21fbbe23fac026fac228fac42afac62df9c72ff9c932f9cb35f8cd37f8cf3af7d13df7d340f6d543f6d746f5d949f5db4cf4dd4ff4df53f4e156f3e35af3e55df2e661f2e865f2ea69f1ec6df1ed71f1ef75f1f179f2f27df2f482f3f586f3f68af4f88ef5f992f6fa96f8fb9af9fc9dfafda1fcffa4";
const fn hex_nibble(value: u8) -> u8 {
    match value {
        b'0'..=b'9' => value - b'0',
        b'a'..=b'f' => value - b'a' + 10,
        _ => 0,
    }
}
const fn inferno_lut() -> [[u8; 3]; 256] {
    let bytes = INFERNO_HEX.as_bytes();
    let mut lut = [[0; 3]; 256];
    let mut i = 0;
    while i < lut.len() {
        let offset = i * 6;
        lut[i] = [
            hex_nibble(bytes[offset]) * 16 + hex_nibble(bytes[offset + 1]),
            hex_nibble(bytes[offset + 2]) * 16 + hex_nibble(bytes[offset + 3]),
            hex_nibble(bytes[offset + 4]) * 16 + hex_nibble(bytes[offset + 5]),
        ];
        i += 1;
    }
    lut
}
const INFERNO_LUT: [[u8; 3]; 256] = inferno_lut();
fn inferno_color(value: f64, low: f64, high: f64) -> Color32 {
    let t = if high > low {
        ((value - low) / (high - low)).clamp(0., 1.)
    } else {
        0.5
    };
    let [r, g, b] = INFERNO_LUT[(t * 255.).round() as usize];
    Color32::from_rgb(r, g, b)
}

pub(super) struct HeatmapRaster {
    energy: bool,
    x_edges: [f64; 2],
    max_texture_side: usize,
    pages: Vec<HeatmapAtlasPage>,
    rows: Vec<HeatmapAtlasRow>,
}

struct HeatmapAtlasPage {
    texture: egui::TextureHandle,
}

struct HeatmapAtlasRow {
    page: usize,
    uv: Rect,
}

/// Pure page assignment, kept separate from egui so GPU bounds and UV seams
/// can be tested without a graphics context.
#[derive(Clone, Debug, PartialEq, Eq)]
struct AtlasLayout {
    width: usize,
    page_heights: Vec<usize>,
    rows: Vec<AtlasSlot>,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
struct AtlasSlot {
    page: usize,
    local_row: usize,
    page_height: usize,
}

impl AtlasSlot {
    fn uv(self) -> Rect {
        let top = self.local_row as f32 / self.page_height as f32;
        let bottom = (self.local_row + 1) as f32 / self.page_height as f32;
        Rect::from_min_max(egui::pos2(0., top), egui::pos2(1., bottom))
    }
}

fn atlas_layout(width: usize, row_count: usize, max_texture_side: usize) -> AtlasLayout {
    let limit = max_texture_side.max(1);
    let width = width.clamp(1, limit);
    let mut page_heights = Vec::new();
    let mut rows = Vec::with_capacity(row_count);
    for row in 0..row_count {
        let page = row / limit;
        let local_row = row % limit;
        if page == page_heights.len() {
            page_heights.push((row_count - row).min(limit));
        }
        rows.push(AtlasSlot {
            page,
            local_row,
            page_height: page_heights[page],
        });
    }
    AtlasLayout {
        width,
        page_heights,
        rows,
    }
}

#[derive(Clone, Copy, PartialEq, Eq)]
struct PlotDataKey {
    samples_len: usize,
    first_sample: usize,
    last_sample: usize,
    preview: usize,
    selected: Option<usize>,
    energy: bool,
    normalize: bool,
}

impl PlotDataKey {
    fn from_app(app: &PleApp) -> Self {
        Self {
            samples_len: app.samples.len(),
            first_sample: app
                .samples
                .first()
                .map(|sample| Arc::as_ptr(sample) as usize)
                .unwrap_or(0),
            last_sample: app
                .samples
                .last()
                .map(|sample| Arc::as_ptr(sample) as usize)
                .unwrap_or(0),
            preview: app
                .preview
                .as_ref()
                .map(|sample| Arc::as_ptr(sample) as usize)
                .unwrap_or(0),
            selected: app.selected,
            energy: app.energy,
            normalize: app.normalize,
        }
    }
}

struct OverlayPlotData {
    label: String,
    points: Vec<PlotPoint>,
    color: Color32,
    dashed: bool,
}

/// UI-only plot preparation cache.  It deliberately owns only reduced plot
/// vertices; source spectra remain in `Sample`/`Acquisition` unchanged.
pub(crate) struct PlotDataCache {
    key: PlotDataKey,
    current: Option<Vec<PlotPoint>>,
    preview: Option<Vec<PlotPoint>>,
    overlay: Vec<OverlayPlotData>,
    excitation: Vec<PlotPoint>,
    fit: Option<(String, Vec<PlotPoint>)>,
}

fn display_points(
    wavelength: &[f64],
    corrected_counts: &[f64],
    sample_power_w: f64,
    normalize: bool,
    energy: bool,
) -> Vec<PlotPoint> {
    let normalized;
    let values = if normalize {
        normalized = corrected_counts
            .iter()
            .map(|value| value / sample_power_w)
            .collect::<Vec<_>>();
        &normalized
    } else {
        corrected_counts
    };
    ple_core::display::display_spectrum_reduced(
        wavelength,
        values,
        energy,
        ple_core::display::MAX_DISPLAY_SPECTRUM_VERTICES,
        HC_EV_NM,
    )
    .into_iter()
    .map(|[x, y]| PlotPoint::new(x, y))
    .collect()
}

impl PlotDataCache {
    fn build(app: &PleApp, key: PlotDataKey) -> Self {
        let current = app
            .selected
            .and_then(|index| app.samples.get(index))
            .or_else(|| app.samples.last())
            .map(|sample| {
                display_points(
                    &sample.spectrum.wavelength_nm,
                    &sample.corrected_counts,
                    sample.sample_power_w,
                    app.normalize,
                    app.energy,
                )
            });
        let preview = app.preview.as_ref().map(|sample| {
            display_points(
                &sample.spectrum.wavelength_nm,
                &sample.corrected_counts,
                sample.sample_power_w,
                app.normalize,
                app.energy,
            )
        });
        let overlay = ple_core::display::representative_indices(app.samples.len(), 8)
            .into_iter()
            .enumerate()
            .map(|(ordinal, index)| {
                let sample = &app.samples[index];
                OverlayPlotData {
                    label: format!("{:0.4} mm · {}", sample.actual_mm, sample.repeat_index + 1),
                    points: display_points(
                        &sample.spectrum.wavelength_nm,
                        &sample.corrected_counts,
                        sample.sample_power_w,
                        app.normalize,
                        app.energy,
                    ),
                    color: COLORS[ordinal % COLORS.len()],
                    dashed: ordinal >= 6,
                }
            })
            .collect();
        let excitation = app
            .samples
            .iter()
            .map(|sample| {
                PlotPoint::new(
                    sample.actual_mm,
                    if app.energy {
                        HC_EV_NM / sample.excitation_nm
                    } else {
                        sample.excitation_nm
                    },
                )
            })
            .collect();
        let fit = match (app.samples.first(), app.samples.last()) {
            (Some(first), Some(last)) => ple_core::display::ols_wavelength_fit(
                ple_core::display::position_wavelength_means(app.samples.iter().map(Arc::as_ref)),
            )
            .map(|fit| {
                (
                    format!(
                        "λ[nm] = {:.4} x[mm] {:+.4}; R² {}",
                        fit.slope_nm_per_mm,
                        fit.intercept_nm,
                        fit.r_squared
                            .map(|value| format!("{value:.4}"))
                            .unwrap_or_else(|| "n/a".into())
                    ),
                    ple_core::display::wavelength_fit_curve(
                        fit,
                        first.actual_mm,
                        last.actual_mm,
                        app.energy,
                        64,
                        HC_EV_NM,
                    )
                    .into_iter()
                    .map(|[x, y]| PlotPoint::new(x, y))
                    .collect(),
                )
            }),
            _ => None,
        };
        Self {
            key,
            current,
            preview,
            overlay,
            excitation,
            fit,
        }
    }
}
fn raster_indices(wavelength: &[f64], energy: bool, width: usize) -> ([f64; 2], Vec<usize>) {
    let left = wavelength[0] - (wavelength[1] - wavelength[0]) * 0.5;
    let n = wavelength.len();
    let right = wavelength[n - 1] + (wavelength[n - 1] - wavelength[n - 2]) * 0.5;
    let edges = if energy {
        [HC_EV_NM / right, HC_EV_NM / left]
    } else {
        [left, right]
    };
    let indices = (0..width)
        .map(|i| {
            let x = edges[0] + (edges[1] - edges[0]) * (i as f64 + 0.5) / width as f64;
            let nm = if energy { HC_EV_NM / x } else { x };
            let next = wavelength.partition_point(|w| *w < nm);
            if next == 0 {
                0
            } else if next == n {
                n - 1
            } else if (nm - wavelength[next - 1]).abs() <= (nm - wavelength[next]).abs() {
                next - 1
            } else {
                next
            }
        })
        .collect();
    (edges, indices)
}
pub(super) fn plot_rects(area: Rect) -> [Rect; 4] {
    let gap = 8.;
    let w = ((area.width() - gap) * 0.5).ceil();
    let h = ((area.height() - gap) * 0.5).ceil();
    [
        rect(area.left(), area.top(), w, h),
        rect(area.left() + w + gap, area.top(), area.width() - w - gap, h),
        rect(
            area.left(),
            area.top() + h + gap,
            w,
            area.height() - h - gap,
        ),
        rect(
            area.left() + w + gap,
            area.top() + h + gap,
            area.width() - w - gap,
            area.height() - h - gap,
        ),
    ]
}
fn chart(ui: &mut egui::Ui, area: Rect, title: &str, add: impl FnOnce(&mut egui::Ui, Rect)) {
    ui.painter().text(
        area.left_top(),
        Align2::LEFT_TOP,
        title,
        FontId::proportional(12.),
        TEXT,
    );
    let inner = Rect::from_min_max(area.min + Vec2::new(0., 18.), area.max);
    ui.scope_builder(
        egui::UiBuilder::new()
            .max_rect(inner)
            .layout(Layout::top_down(Align::Min)),
        |ui| {
            ui.set_clip_rect(ui.clip_rect().intersect(inner));
            add(ui, inner);
        },
    );
}
impl PleApp {
    fn refresh_plot_cache(&mut self) {
        let key = PlotDataKey::from_app(self);
        if self
            .plot_cache
            .as_ref()
            .is_none_or(|cache| cache.key != key)
        {
            self.plot_cache = Some(PlotDataCache::build(self, key));
        }
    }
    fn intensity_scale(&self) -> AxisScale {
        let key = (
            self.samples.len(),
            self.samples
                .first()
                .map(|s| Arc::as_ptr(s) as usize)
                .unwrap_or(0),
            self.normalize,
        );
        if self.preview.is_none()
            && let Some(c) = &self.heatmap_cache
            && c.key == key
        {
            return AxisScale::for_magnitude(c.signal_magnitude);
        }
        let sample_magnitude = self
            .samples
            .iter()
            .flat_map(|s| {
                s.corrected_counts
                    .iter()
                    .map(move |v| v.abs() / if self.normalize { s.sample_power_w } else { 1. })
            })
            .fold(0_f64, f64::max);
        let preview_magnitude = self.preview.as_ref().map_or(0., |preview| {
            preview
                .corrected_counts
                .iter()
                .map(|value| {
                    value.abs()
                        / if self.normalize {
                            preview.sample_power_w
                        } else {
                            1.
                        }
                })
                .fold(0_f64, f64::max)
        });
        AxisScale::for_magnitude(sample_magnitude.max(preview_magnitude))
    }
    fn spectral_plot(
        &self,
        ctx: &egui::Context,
        id: &str,
        area: Rect,
        mut scale: AxisScale,
    ) -> Plot<'static> {
        let has_data = !self.samples.is_empty() || self.preview.is_some();
        let x_label = if self.energy {
            "Emission energy (eV)"
        } else {
            "Emission wavelength (nm)"
        };
        let unit = if self.normalize { "counts/W" } else { "counts" };
        let plot_id = egui::Id::new((
            "ple",
            id,
            self.energy,
            self.normalize,
            self.plot_epoch,
            has_data,
        ));
        if let Some(memory) = PlotMemory::load(ctx, plot_id)
            && !memory.auto_bounds[1]
        {
            let bounds = memory.bounds();
            scale = AxisScale::for_magnitude(bounds.min()[1].abs().max(bounds.max()[1].abs()));
        }
        let p = Plot::new(id)
            .id(plot_id)
            .width(area.width())
            .height(area.height())
            .show_background(true)
            .x_axis_label(x_label)
            .y_axis_label(format!("PL − BG ({}{unit})", scale.prefix))
            .y_axis_formatter(move |m, _| scale.tick(m.value, m.step_size))
            .allow_boxed_zoom(true)
            .allow_scroll(false)
            .allow_drag(true)
            .label_formatter(move |name, p| {
                format!(
                    "{name}\nEmission: {:0.6}\nPL − BG: {:0.6e} {unit}",
                    p.x, p.y
                )
            });
        if has_data {
            p.auto_bounds(true)
        } else {
            p.default_x_bounds(0., 1.)
                .default_y_bounds(0., 1.)
                .auto_bounds(false)
        }
    }
    pub(super) fn plots(&mut self, ui: &mut egui::Ui, area: Rect) {
        self.refresh_plot_cache();
        let areas = plot_rects(area);
        let scale = self.intensity_scale();
        let title = if self.awaiting == 3 && self.preview.is_some() {
            "Preview · click to place PL peak"
        } else if self.selected.is_some() {
            "Selected PL spectrum"
        } else {
            "Current PL spectrum"
        };
        for (name, area) in [
            "plot_current",
            "plot_overlay",
            "plot_excitation",
            "plot_heatmap",
        ]
        .into_iter()
        .zip(areas)
        {
            precision::record(&mut self.geometry, name, area);
        }
        chart(ui, areas[0], title, |ui, r| {
            let response = self
                .spectral_plot(ui.ctx(), "current", r, scale)
                .show(ui, |p| {
                    if self.awaiting == 3
                        && let Some(preview) = self
                            .plot_cache
                            .as_ref()
                            .and_then(|cache| cache.preview.as_deref())
                    {
                        p.line(
                            Line::new("Preview PL − BG", preview)
                                .color(BLUE)
                                .width(1.7_f32),
                        );
                        let x = |nm: f64| if self.energy { HC_EV_NM / nm } else { nm };
                        for (label, nm, color) in [
                            ("PL peak", self.analysis_draft.peak_nm, ORANGE),
                            (
                                "Peak window min",
                                self.analysis_draft.peak_nm
                                    - self.analysis_draft.peak_half_width_nm,
                                MUTED,
                            ),
                            (
                                "Peak window max",
                                self.analysis_draft.peak_nm
                                    + self.analysis_draft.peak_half_width_nm,
                                MUTED,
                            ),
                            ("Integral min", self.analysis_draft.integral_min_nm, GREEN),
                            ("Integral max", self.analysis_draft.integral_max_nm, GREEN),
                        ] {
                            p.vline(VLine::new(label, x(nm)).color(color).width(1_f32));
                        }
                    }
                    if let Some(current) = self
                        .plot_cache
                        .as_ref()
                        .and_then(|cache| cache.current.as_deref())
                    {
                        p.line(Line::new("PL − BG", current).color(BLUE).width(1.7_f32));
                    }
                });
            if self.awaiting == 3
                && response.response.clicked()
                && let Some(pointer) = response.response.interact_pointer_pos()
            {
                let point = response.transform.value_from_position(pointer);
                let wavelength_nm = if self.energy {
                    HC_EV_NM / point.x
                } else {
                    point.x
                };
                if wavelength_nm.is_finite() && wavelength_nm > 0. {
                    self.analysis_draft.peak_nm = wavelength_nm;
                }
            }
        });
        chart(ui, areas[1], "Spectral overlay", |ui, r| {
            self.spectral_plot(ui.ctx(), "overlay", r, scale)
                .legend(Legend::default().background_alpha(0.45))
                .show(ui, |p| {
                    for line in &self.plot_cache.as_ref().unwrap().overlay {
                        p.line(
                            Line::new(&line.label, line.points.as_slice())
                                .color(line.color)
                                .width(1.7_f32)
                                .style(if line.dashed {
                                    LineStyle::dashed_dense()
                                } else {
                                    LineStyle::Solid
                                }),
                        );
                    }
                });
        });
        chart(ui, areas[2], "CONEX CC vs excitation", |ui, r| {
            let has_data = !self.samples.is_empty();
            let plot = Plot::new(("ple_excitation", self.energy, self.plot_epoch, has_data))
                .width(r.width())
                .height(r.height())
                .show_background(true)
                .x_axis_label("CONEX position (mm)")
                .y_axis_label(if self.energy {
                    "Excitation energy (eV)"
                } else {
                    "Excitation wavelength (nm)"
                })
                .legend(
                    Legend::default()
                        .position(if self.energy {
                            egui_plot::Corner::RightTop
                        } else {
                            egui_plot::Corner::LeftTop
                        })
                        .background_alpha(0.45),
                )
                .allow_boxed_zoom(true)
                .allow_scroll(false)
                .allow_drag(true);
            let plot = if has_data {
                plot.auto_bounds(true)
            } else {
                plot.default_x_bounds(0., 1.)
                    .default_y_bounds(0., 1.)
                    .auto_bounds(false)
            };
            let response = plot.show(ui, |p| {
                p.points(
                    Points::new(
                        "Acquired excitation",
                        self.plot_cache.as_ref().unwrap().excitation.as_slice(),
                    )
                    .color(BLUE)
                    .radius(2.5_f32),
                );
                if let Some((label, points)) = &self.plot_cache.as_ref().unwrap().fit {
                    p.line(
                        Line::new(label, points.as_slice())
                            .color(ORANGE)
                            .width(1.4_f32),
                    );
                }
            });
            if response.response.clicked()
                && let Some(pointer) = response.response.interact_pointer_pos()
            {
                let q = response.transform.value_from_position(pointer);
                self.selected = self
                    .samples
                    .iter()
                    .enumerate()
                    .min_by(|(_, a), (_, b)| {
                        (a.actual_mm - q.x)
                            .abs()
                            .total_cmp(&(b.actual_mm - q.x).abs())
                    })
                    .map(|(i, _)| i);
            }
        });
        chart(
            ui,
            areas[3],
            "PLE heatmap · mean at each excitation",
            |ui, r| self.heatmap(ui, r),
        );
    }
    fn heatmap(&mut self, ui: &mut egui::Ui, area: Rect) {
        let key = (
            self.samples.len(),
            self.samples
                .first()
                .map(|s| Arc::as_ptr(s) as usize)
                .unwrap_or(0),
            self.normalize,
        );
        if self.heatmap_cache.as_ref().is_none_or(|c| c.key != key) {
            let rows = ple_core::display::heatmap_rows(
                self.samples
                    .iter()
                    .enumerate()
                    .map(|(i, s)| (i, s.as_ref())),
                self.normalize,
            );
            let low = rows
                .iter()
                .flat_map(|r| r.values.iter())
                .copied()
                .fold(f64::INFINITY, f64::min);
            let high = rows
                .iter()
                .flat_map(|r| r.values.iter())
                .copied()
                .fold(f64::NEG_INFINITY, f64::max);
            let normalize = self.normalize;
            let signal_magnitude = self
                .samples
                .iter()
                .flat_map(|s| {
                    s.corrected_counts
                        .iter()
                        .map(move |v| v.abs() / if normalize { s.sample_power_w } else { 1. })
                })
                .fold(0_f64, f64::max);
            self.heatmap_cache = Some(HeatmapCache {
                key,
                rows,
                low,
                high,
                signal_magnitude,
                raster: None,
            });
        }
        let cache = self.heatmap_cache.as_mut().unwrap();
        let max_texture_side = ui.ctx().input(|input| input.max_texture_side).max(1);
        if cache
            .raster
            .as_ref()
            .is_none_or(|r| r.energy != self.energy || r.max_texture_side != max_texture_side)
            && !cache.rows.is_empty()
        {
            let first = &self.samples[cache.rows[0].sample_index]
                .spectrum
                .wavelength_nm;
            let width = first.len().min(4096).min(max_texture_side).max(1);
            let (x_edges, indices) = raster_indices(first, self.energy, width);
            let layout = atlas_layout(width, cache.rows.len(), max_texture_side);
            let pages = layout
                .page_heights
                .iter()
                .enumerate()
                .map(|(page, height)| {
                    let start = page * max_texture_side;
                    let end = start + height;
                    let pixels = cache.rows[start..end]
                        .iter()
                        .flat_map(|row| {
                            indices
                                .iter()
                                .map(|ix| inferno_color(row.values[*ix], cache.low, cache.high))
                        })
                        .collect();
                    HeatmapAtlasPage {
                        texture: ui.ctx().load_texture(
                            format!("ple-map-atlas-{page}"),
                            egui::ColorImage::new([layout.width, *height], pixels),
                            egui::TextureOptions::NEAREST,
                        ),
                    }
                })
                .collect();
            cache.raster = Some(HeatmapRaster {
                energy: self.energy,
                x_edges,
                max_texture_side,
                pages,
                rows: layout
                    .rows
                    .into_iter()
                    .map(|slot| HeatmapAtlasRow {
                        page: slot.page,
                        uv: slot.uv(),
                    })
                    .collect(),
            });
        }
        let cache = self.heatmap_cache.as_ref().unwrap();
        let rows = &cache.rows;
        let unit = if self.normalize { "counts/W" } else { "counts" };
        let has_data = !rows.is_empty();
        let height = area.height() - 16.;
        let plot = Plot::new((
            "ple_heatmap",
            self.energy,
            self.normalize,
            self.plot_epoch,
            has_data,
        ))
        .width(area.width())
        .height(height)
        .show_background(true)
        .x_axis_label(if self.energy {
            "Emission energy (eV)"
        } else {
            "Emission wavelength (nm)"
        })
        .y_axis_label(if self.energy {
            "Excitation energy (eV)"
        } else {
            "Excitation wavelength (nm)"
        })
        .allow_boxed_zoom(true)
        .allow_scroll(false)
        .allow_drag(true)
        .label_formatter(|_, _| String::new());
        let plot = if has_data {
            plot.auto_bounds(true)
        } else {
            plot.default_x_bounds(0., 1.)
                .default_y_bounds(0., 1.)
                .auto_bounds(false)
        };
        let response = plot.show(ui, |p| {
            for (r, row) in rows.iter().enumerate() {
                let y = row.excitation_nm;
                let yl = if r > 0 {
                    (rows[r - 1].excitation_nm + y) * 0.5
                } else {
                    y - rows.get(1).map(|n| n.excitation_nm - y).unwrap_or(0.5) * 0.5
                };
                let yh = if r + 1 < rows.len() {
                    (rows[r + 1].excitation_nm + y) * 0.5
                } else {
                    y + rows
                        .get(r.wrapping_sub(1))
                        .map(|n| y - n.excitation_nm)
                        .unwrap_or(0.5)
                        * 0.5
                };
                let transform = |v: f64| if self.energy { HC_EV_NM / v } else { v };
                let (y0, y1) = (transform(yl), transform(yh));
                if let Some(raster) = &cache.raster {
                    let [x0, x1] = raster.x_edges;
                    let atlas_row = &raster.rows[r];
                    p.image(
                        PlotImage::new(
                            "PL − BG",
                            raster.pages[atlas_row.page].texture.id(),
                            PlotPoint::new((x0 + x1) * 0.5, (y0 + y1) * 0.5),
                            Vec2::new((x1 - x0).abs() as f32, (y1 - y0).abs() as f32),
                        )
                        .uv(atlas_row.uv),
                    );
                }
            }
        });
        if response.response.clicked()
            && let Some(pointer) = response.response.interact_pointer_pos()
        {
            let q = response.transform.value_from_position(pointer);
            let target = if self.energy { HC_EV_NM / q.y } else { q.y };
            self.selected = rows
                .iter()
                .min_by(|a, b| {
                    (a.excitation_nm - target)
                        .abs()
                        .total_cmp(&(b.excitation_nm - target).abs())
                })
                .map(|r| r.sample_index);
        }
        if has_data {
            let bar = rect(area.left() + 4., area.bottom() - 10., 76., 6.);
            for i in 0..64 {
                let x = bar.left() + bar.width() * i as f32 / 64.;
                ui.painter().rect_filled(
                    rect(x, bar.top(), bar.width() / 64. + 0.1, bar.height()),
                    0.,
                    inferno_color(
                        cache.low + (cache.high - cache.low) * i as f64 / 63.,
                        cache.low,
                        cache.high,
                    ),
                );
            }
            ui.painter().text(
                egui::pos2(bar.right() + 8., area.bottom() - 1.),
                Align2::LEFT_BOTTOM,
                format!("{:0.3e} … {:0.3e} {unit}", cache.low, cache.high),
                FontId::proportional(9.),
                MUTED,
            );
        }
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn palette_is_official_sequential_inferno_and_preserves_negative_range() {
        assert_eq!(inferno_color(-2., -2., 2.), Color32::from_rgb(0, 0, 4));
        assert_eq!(inferno_color(2., -2., 2.), Color32::from_rgb(252, 255, 164));
        assert_ne!(inferno_color(-1., -2., 2.), inferno_color(1., -2., 2.));
    }
    #[test]
    fn compile_time_inferno_lut_matches_every_hex_triplet() {
        assert_eq!(INFERNO_HEX.len(), 256 * 6);
        for (index, rgb) in INFERNO_LUT.iter().enumerate() {
            let offset = index * 6;
            let decode = |at| {
                hex_nibble(INFERNO_HEX.as_bytes()[offset + at]) * 16
                    + hex_nibble(INFERNO_HEX.as_bytes()[offset + at + 1])
            };
            assert_eq!(*rgb, [decode(0), decode(2), decode(4)]);
        }
    }
    #[test]
    fn energy_raster_preserves_physical_order_and_only_indexes_raw_bins() {
        let w = [650., 700., 780., 850.];
        let (nm, n) = raster_indices(&w, false, 64);
        let (ev, e) = raster_indices(&w, true, 64);
        assert!(nm[0] < nm[1] && ev[0] < ev[1]);
        assert_eq!(n[0], 0);
        assert_eq!(n[63], 3);
        assert_eq!(e[0], 3);
        assert_eq!(e[63], 0);
        assert!(n.iter().chain(&e).all(|i| *i < w.len()));
    }
    #[test]
    fn atlas_layout_bounds_each_page_by_gpu_limit() {
        let layout = atlas_layout(4096, 5000, 2048);
        assert_eq!(layout.width, 2048);
        assert_eq!(layout.page_heights, vec![2048, 2048, 904]);
        assert!(layout.page_heights.iter().all(|height| *height <= 2048));
        assert_eq!(layout.rows[0].page, 0);
        assert_eq!(layout.rows[2047].page, 0);
        assert_eq!(layout.rows[2048].page, 1);
        assert_eq!(layout.rows[4999].page, 2);
    }
    #[test]
    fn atlas_row_uv_crops_exact_rows_at_page_boundaries() {
        let layout = atlas_layout(32, 5, 2);
        let first = layout.rows[0].uv();
        let last_first_page = layout.rows[1].uv();
        let first_second_page = layout.rows[2].uv();
        assert_eq!(layout.rows[2].page, 1);
        assert_eq!(first.min.y, 0.);
        assert_eq!(first.max.y, 0.5);
        assert_eq!(last_first_page.min.y, 0.5);
        assert_eq!(last_first_page.max.y, 1.);
        assert_eq!(first_second_page.min.y, 0.);
        assert_eq!(first_second_page.max.y, 0.5);
    }
    #[test]
    fn four_plots_fit_without_overlap() {
        let outer = rect(0., 0., 832., 600.);
        let r = plot_rects(outer);
        for a in r {
            assert!(outer.contains_rect(a));
        }
        for i in 0..4 {
            for j in i + 1..4 {
                assert!(!r[i].intersects(r[j]));
            }
        }
    }
}
