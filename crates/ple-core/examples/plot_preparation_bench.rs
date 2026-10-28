//! Reproducible CPU preparation comparison; this does not measure GPU/FPS.
use ple_core::{
    HC_EV_NM,
    display::{MAX_DISPLAY_SPECTRUM_VERTICES, display_spectrum_reduced},
    display_spectrum,
};
use std::{hint::black_box, time::Instant};
fn main() {
    println!("bins,curves,frames,old_prepare_ms,cached_prepare_ms,reduced_vertices_per_curve");
    for (bins, frames) in [(4_096, 120), (65_536, 60), (1_000_000, 10)] {
        let wavelength: Vec<_> = (0..bins)
            .map(|i| 640. + 240. * i as f64 / (bins - 1) as f64)
            .collect();
        let counts: Vec<_> = (0..bins)
            .map(|i| {
                let x = wavelength[i];
                900. * (-0.5 * ((x - 780.) / 7.).powi(2)).exp() + (i as f64).sin()
            })
            .collect();
        let started = Instant::now();
        for _ in 0..frames {
            for _ in 0..8 {
                let normalized: Vec<_> = black_box(&counts).iter().map(|v| v / 20e-6).collect();
                black_box(display_spectrum(black_box(&wavelength), &normalized, true));
            }
        }
        let old = started.elapsed().as_secs_f64() * 1000.;
        let started = Instant::now();
        let cached: Vec<_> = (0..8)
            .map(|_| {
                let normalized: Vec<_> = counts.iter().map(|v| v / 20e-6).collect();
                display_spectrum_reduced(
                    &wavelength,
                    &normalized,
                    true,
                    MAX_DISPLAY_SPECTRUM_VERTICES,
                    HC_EV_NM,
                )
            })
            .collect();
        for _ in 0..frames {
            for line in &cached {
                black_box(line.as_slice());
            }
        }
        let new = started.elapsed().as_secs_f64() * 1000.;
        println!("{bins},8,{frames},{old:.3},{new:.3},{}", cached[0].len());
    }
}
