//! Display reductions never modify recorded acquisitions.
use crate::Sample;
use std::collections::BTreeMap;

/// Ordinary least-squares fit of excitation wavelength (nm) against stage
/// position (mm).  The fit itself is always performed in wavelength space.
#[derive(Clone, Copy, Debug, PartialEq)]
pub struct LinearFit {
    pub slope_nm_per_mm: f64,
    pub intercept_nm: f64,
    pub r_squared: Option<f64>,
    pub n: usize,
}

pub fn ols_wavelength_fit(points: impl IntoIterator<Item = (f64, f64)>) -> Option<LinearFit> {
    let points: Vec<_> = points
        .into_iter()
        .filter(|(x, y)| x.is_finite() && y.is_finite())
        .collect();
    if points.len() < 2 {
        return None;
    }
    let n = points.len() as f64;
    let x_mean = points.iter().map(|(x, _)| x).sum::<f64>() / n;
    let y_mean = points.iter().map(|(_, y)| y).sum::<f64>() / n;
    let (sxx, sxy) = points.iter().fold((0., 0.), |(sxx, sxy), (x, y)| {
        let dx = x - x_mean;
        (sxx + dx * dx, sxy + dx * (y - y_mean))
    });
    if sxx <= f64::EPSILON {
        return None;
    }
    let slope = sxy / sxx;
    let intercept = y_mean - slope * x_mean;
    let sst = points
        .iter()
        .map(|(_, y)| (y - y_mean).powi(2))
        .sum::<f64>();
    let sse = points
        .iter()
        .map(|(x, y)| (y - (slope * x + intercept)).powi(2))
        .sum::<f64>();
    Some(LinearFit {
        slope_nm_per_mm: slope,
        intercept_nm: intercept,
        r_squared: if sst <= f64::EPSILON {
            None
        } else {
            Some((1. - sse / sst).clamp(0., 1.))
        },
        n: points.len(),
    })
}

/// Average repeats by planned position so unequal repeat counts do not weight the fit.
pub fn position_wavelength_means<'a>(
    samples: impl IntoIterator<Item = &'a Sample>,
) -> Vec<(f64, f64)> {
    let mut points: BTreeMap<usize, (f64, f64, usize)> = BTreeMap::new();
    for s in samples {
        let v = points.entry(s.point_index).or_insert((0., 0., 0));
        v.0 += s.actual_mm;
        v.1 += s.excitation_nm;
        v.2 += 1;
    }
    points
        .into_values()
        .map(|(x, y, n)| (x / n as f64, y / n as f64))
        .collect()
}
/// Uniformly select at most `limit` representative samples, always retaining
/// the first and last acquisition when more than one is present.
pub fn representative_indices(count: usize, limit: usize) -> Vec<usize> {
    if count == 0 || limit == 0 {
        return Vec::new();
    }
    let shown = count.min(limit);
    if shown == 1 {
        return vec![0];
    }
    (0..shown).map(|k| k * (count - 1) / (shown - 1)).collect()
}

/// Largest number of vertices sent to an interactive spectral line.  This is
/// strictly a display reduction: acquisition, analysis, and exported data
/// retain every detector bin.
pub const MAX_DISPLAY_SPECTRUM_VERTICES: usize = 8_192;

/// Build a spectral polyline suitable for interactive display.
///
/// Long spectra are reduced in source-index buckets, retaining the extrema of
/// each bucket in their original order.  Therefore a one-bin emission peak or
/// dip remains visible while bounding UI work.  The first and last detector
/// bins are always retained.  Energy display reverses the wavelength order so
/// x remains increasing, just like [`display_spectrum`].
pub fn display_spectrum_reduced(
    wavelength: &[f64],
    counts: &[f64],
    energy: bool,
    max_vertices: usize,
    hc_ev_nm: f64,
) -> Vec<[f64; 2]> {
    let len = wavelength.len().min(counts.len());
    if len == 0 || max_vertices == 0 {
        return Vec::new();
    }
    let point = |i: usize| {
        [
            if energy {
                hc_ev_nm / wavelength[i]
            } else {
                wavelength[i]
            },
            counts[i],
        ]
    };
    if max_vertices == 1 {
        return vec![point(if energy { len - 1 } else { 0 })];
    }
    if max_vertices == 2 && len > 2 {
        let mut endpoints = vec![point(0), point(len - 1)];
        if energy {
            endpoints.reverse();
        }
        return endpoints;
    }
    if len <= max_vertices {
        let mut points: Vec<_> = (0..len).map(point).collect();
        if energy {
            points.reverse();
        }
        return points;
    }

    // Two extrema per bucket plus the two endpoints is the hard vertex bound.
    let buckets = ((max_vertices - 2) / 2).max(1).min(len - 2);
    let inner_len = len - 2;
    let mut indices = Vec::with_capacity(max_vertices);
    indices.push(0);
    for bucket in 0..buckets {
        let start = 1 + bucket * inner_len / buckets;
        let end = 1 + (bucket + 1) * inner_len / buckets;
        if start >= end {
            continue;
        }
        let mut min = start;
        let mut max = start;
        for i in start + 1..end {
            if counts[i].total_cmp(&counts[min]).is_lt() {
                min = i;
            }
            if counts[i].total_cmp(&counts[max]).is_gt() {
                max = i;
            }
        }
        if min <= max {
            indices.push(min);
            if max != min {
                indices.push(max);
            }
        } else {
            indices.push(max);
            indices.push(min);
        }
    }
    indices.push(len - 1);
    let mut points: Vec<_> = indices.into_iter().map(point).collect();
    if energy {
        points.reverse();
    }
    points
}

/// Build a smooth display curve from a wavelength-space linear fit.  For the
/// energy view only the plotted ordinate is converted: it is not an eV fit.
pub fn wavelength_fit_curve(
    fit: LinearFit,
    start_mm: f64,
    end_mm: f64,
    energy: bool,
    segments: usize,
    hc_ev_nm: f64,
) -> Vec<[f64; 2]> {
    let segments = segments.max(1);
    (0..=segments)
        .filter_map(|i| {
            let x = start_mm + (end_mm - start_mm) * i as f64 / segments as f64;
            let nm = fit.slope_nm_per_mm * x + fit.intercept_nm;
            let y = if energy { hc_ev_nm / nm } else { nm };
            (x.is_finite() && y.is_finite() && y > 0.).then_some([x, y])
        })
        .collect()
}
#[derive(Clone, Debug)]
pub struct HeatRow {
    pub sample_index: usize,
    pub excitation_nm: f64,
    pub values: Vec<f64>,
    pub acquisitions: usize,
}
/// Mean repeats at a point, then combine coincident excitation wavelengths.
/// Detector-bin quantization can give several stage positions the same peak.
pub fn heatmap_rows<'a>(
    samples: impl IntoIterator<Item = (usize, &'a Sample)>,
    normalize: bool,
) -> Vec<HeatRow> {
    let mut points: BTreeMap<usize, HeatRow> = BTreeMap::new();
    for (i, s) in samples {
        let scale = if normalize { 1. / s.sample_power_w } else { 1. };
        let row = points.entry(s.point_index).or_insert_with(|| HeatRow {
            sample_index: i,
            excitation_nm: 0.,
            values: vec![0.; s.corrected_counts.len()],
            acquisitions: 0,
        });
        if row.values.len() != s.corrected_counts.len() {
            continue;
        }
        row.excitation_nm += s.excitation_nm;
        row.acquisitions += 1;
        for (dst, src) in row.values.iter_mut().zip(&s.corrected_counts) {
            *dst += src * scale;
        }
    }
    let mut rows: Vec<_> = points.into_values().collect();
    for r in &mut rows {
        r.excitation_nm /= r.acquisitions as f64;
    }
    rows.sort_by(|a, b| a.excitation_nm.total_cmp(&b.excitation_nm));
    let mut unique: Vec<HeatRow> = Vec::new();
    for row in rows {
        if let Some(last) = unique
            .last_mut()
            .filter(|r| (r.excitation_nm - row.excitation_nm).abs() < 1e-8)
        {
            for (a, b) in last.values.iter_mut().zip(row.values) {
                *a += b;
            }
            last.acquisitions += row.acquisitions;
        } else {
            unique.push(row);
        }
    }
    for r in &mut unique {
        for value in &mut r.values {
            *value /= r.acquisitions as f64;
        }
    }
    unique
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Spectrum;
    fn sample(point: usize, exc: f64, value: f64) -> Sample {
        Sample {
            file_base: String::new(),
            point_index: point,
            repeat_index: 0,
            target_mm: 18.,
            actual_mm: 18.,
            excitation_nm: exc,
            excitation_source: "test".into(),
            power_w: 2.,
            sample_power_w: 2.,
            power_timestamp_unix_s: 0.,
            power_sensor_wavelength_nm: None,
            acquisition_started_unix_s: 0.,
            timestamp_unix_s: 0.,
            pl_peak_nm: Some(780.),
            pl_integral_counts_nm: value,
            spectrum: Spectrum {
                wavelength_nm: vec![770., 780.],
                counts: vec![value; 2],
                spe_path: None,
                settings: serde_json::Value::Null,
            },
            corrected_counts: vec![value; 2],
        }
    }
    #[test]
    fn regression_does_not_claim_correlation_for_constant_wavelength() {
        let fit = ols_wavelength_fit([(18., 700.), (19., 700.)]).unwrap();
        assert_eq!(fit.r_squared, None);
        assert!(ols_wavelength_fit([(18., 700.), (18., 710.)]).is_none());
    }
    #[test]
    fn fit_averages_each_position_before_regression() {
        let mut a = sample(0, 700., 1.);
        a.actual_mm = 18.;
        let mut b = sample(1, 710., 1.);
        b.actual_mm = 19.;
        let points = position_wavelength_means([&a, &a, &a, &b]);
        assert_eq!(points, vec![(18., 700.), (19., 710.)]);
    }
    #[test]
    fn repeated_and_coincident_excitation_rows_are_weighted() {
        let samples = [
            sample(0, 700., 2.),
            sample(0, 700., 4.),
            sample(1, 700., 12.),
            sample(2, 701., 20.),
        ];
        let rows = heatmap_rows(samples.iter().enumerate(), false);
        assert_eq!(rows.len(), 2);
        assert_eq!(rows[0].acquisitions, 3);
        assert_eq!(rows[0].values, vec![6., 6.]);
        let normalized = heatmap_rows(samples.iter().enumerate(), true);
        assert_eq!(normalized[0].values, vec![3., 3.]);
    }
    #[test]
    fn ols_fit_and_energy_curve_do_not_refit_in_energy() {
        let fit = ols_wavelength_fit([(0., 700.), (1., 710.), (2., 720.)]).unwrap();
        assert_eq!(fit.slope_nm_per_mm, 10.);
        assert_eq!(fit.intercept_nm, 700.);
        assert_eq!(fit.r_squared, Some(1.));
        let curve = wavelength_fit_curve(fit, 0., 2., true, 2, crate::HC_EV_NM);
        assert!((curve[1][1] - crate::HC_EV_NM / 710.).abs() < 1e-12);
    }
    #[test]
    fn representatives_keep_endpoints_and_are_uniform() {
        assert_eq!(representative_indices(0, 8), Vec::<usize>::new());
        assert_eq!(representative_indices(1, 8), vec![0]);
        assert_eq!(
            representative_indices(13, 8),
            vec![0, 1, 3, 5, 6, 8, 10, 12]
        );
    }
    #[test]
    fn display_reduction_preserves_narrow_positive_and_negative_features() {
        let wavelength: Vec<_> = (0..100_000).map(|i| 700. + i as f64 * 0.001).collect();
        let mut counts = vec![1.; wavelength.len()];
        counts[12_345] = 99.;
        counts[67_890] = -41.;
        let reduced = display_spectrum_reduced(
            &wavelength,
            &counts,
            false,
            MAX_DISPLAY_SPECTRUM_VERTICES,
            crate::HC_EV_NM,
        );
        assert!(reduced.len() <= MAX_DISPLAY_SPECTRUM_VERTICES);
        assert_eq!(reduced.first(), Some(&[wavelength[0], 1.]));
        assert_eq!(
            reduced.last(),
            Some(&[wavelength[wavelength.len() - 1], 1.])
        );
        assert!(reduced.contains(&[wavelength[12_345], 99.]));
        assert!(reduced.contains(&[wavelength[67_890], -41.]));
    }
    #[test]
    fn reduced_energy_spectrum_is_ordered_and_uses_original_coordinates() {
        let wavelength = [700., 701., 702., 703., 704., 705.];
        let counts = [0., -2., 5., 0., 1., 0.];
        let points = display_spectrum_reduced(&wavelength, &counts, true, 4, crate::HC_EV_NM);
        assert!(points.windows(2).all(|pair| pair[0][0] <= pair[1][0]));
        assert!(points.contains(&[crate::HC_EV_NM / 701., -2.]));
        assert!(points.contains(&[crate::HC_EV_NM / 702., 5.]));
    }
}
