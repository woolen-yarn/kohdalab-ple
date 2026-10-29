//! Hardware-independent PLE planning, reduction and run recording.
#![forbid(unsafe_code)]
pub mod display;
pub mod recording;
use serde::{Deserialize, Serialize};

pub const HC_EV_NM: f64 = 1239.8419843320026;
pub const MAX_ACQUISITIONS: usize = 20_000;

#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum Mode {
    #[default]
    Demo,
    Hardware,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct SpectrometerConfig {
    pub python: String,
    pub bridge_script: String,
    pub experiment: String,
    pub visible: bool,
    pub exposure_ms: f64,
    pub frames_combined: u32,
    pub center_nm: f64,
    pub grating: String,
    pub timeout_s: f64,
}
impl Default for SpectrometerConfig {
    fn default() -> Self {
        Self {
            python: if cfg!(windows) { "python" } else { "python3" }.into(),
            bridge_script: "bridge/ple_bridge.py".into(),
            experiment: "".into(),
            visible: true,
            exposure_ms: 1000.,
            frames_combined: 1,
            center_nm: 780.,
            grating: String::new(),
            timeout_s: 120.,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum StageProtocol {
    #[default]
    LegacyTcp,
    RemoteJson,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum BackgroundMode {
    #[default]
    Manual,
    Automatic,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ShutterBackend {
    #[default]
    Mff,
    KohdalabShutter,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct BackgroundConfig {
    pub mode: BackgroundMode,
    pub backend: ShutterBackend,
    pub endpoint: String,
    pub channel: usize,
    pub shutter_serial: String,
    pub closed_state: u8,
    pub open_state: u8,
    pub timeout_s: f64,
}
impl Default for BackgroundConfig {
    fn default() -> Self {
        Self {
            mode: BackgroundMode::Manual,
            backend: ShutterBackend::Mff,
            endpoint: String::new(),
            channel: 0,
            shutter_serial: String::new(),
            closed_state: 1,
            open_state: 0,
            timeout_s: 10.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
#[serde(deny_unknown_fields)]
pub struct AnalysisConfig {
    pub peak_nm: f64,
    pub peak_half_width_nm: f64,
    pub integral_min_nm: f64,
    pub integral_max_nm: f64,
}
impl AnalysisConfig {
    pub fn validate(&self, wavelength: &[f64]) -> Result<(), String> {
        if wavelength.len() < 2 {
            return Err("Acquire the first spectrum before choosing analysis".into());
        }
        for (v, n) in [
            (self.peak_nm, "PL peak"),
            (self.peak_half_width_nm, "PL peak search half width"),
            (self.integral_min_nm, "Integration minimum"),
            (self.integral_max_nm, "Integration maximum"),
        ] {
            finite(v, n)?;
        }
        let min = wavelength[0];
        let max = wavelength[wavelength.len() - 1];
        if self.peak_half_width_nm <= 0.
            || self.peak_nm - self.peak_half_width_nm < min
            || self.peak_nm + self.peak_half_width_nm > max
        {
            return Err(format!(
                "PL peak search window must lie inside {min:.3}–{max:.3} nm"
            ));
        }
        if self.integral_min_nm < min
            || self.integral_max_nm > max
            || self.integral_min_nm >= self.integral_max_nm
        {
            return Err(format!(
                "Choose an increasing integration interval inside {min:.3}–{max:.3} nm"
            ));
        }
        if !wavelength
            .iter()
            .any(|w| (*w - self.peak_nm).abs() <= self.peak_half_width_nm)
        {
            return Err("PL peak search window contains no detector bins".into());
        }
        Ok(())
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct StageConfig {
    pub protocol: StageProtocol,
    pub token_env: String,
    pub host: String,
    pub port: u16,
    pub com_port: String,
    pub allow_legacy_initialize: bool,
    pub min_mm: f64,
    pub max_mm: f64,
    pub tolerance_mm: f64,
    pub timeout_s: f64,
}
impl StageConfig {
    pub fn validate_limits(&self) -> Result<(), String> {
        if !self.min_mm.is_finite()
            || !self.max_mm.is_finite()
            || self.min_mm < 0.
            || self.max_mm > 25.
            || self.min_mm >= self.max_mm
        {
            return Err("CONEX software limits must lie within 0–25 mm".into());
        }
        Ok(())
    }
}
impl Default for StageConfig {
    fn default() -> Self {
        Self {
            protocol: StageProtocol::LegacyTcp,
            token_env: "PLE_CONEX_TOKEN".into(),
            host: "scallop.local".into(),
            port: 9000,
            com_port: "COM4".into(),
            allow_legacy_initialize: false,
            min_mm: 0.,
            max_mm: 25.,
            tolerance_mm: 0.001,
            timeout_s: 90.,
        }
    }
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum PowerWavelengthMode {
    #[default]
    Fixed,
    PositionCalibration,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct PowerConfig {
    pub wavelength_mode: PowerWavelengthMode,
    pub wavelength_tolerance_nm: f64,
    pub resource: String,
    pub driver_path: String,
    pub wavelength_nm: f64,
    pub correction_factor: f64,
    pub average_s: f64,
}
impl Default for PowerConfig {
    fn default() -> Self {
        Self {
            wavelength_mode: PowerWavelengthMode::Fixed,
            wavelength_tolerance_nm: 1.,
            resource: String::new(),
            driver_path: String::new(),
            wavelength_nm: 720.,
            correction_factor: 1.,
            average_s: 0.2,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize, PartialEq)]
pub struct PeakAnchor {
    pub position_mm: f64,
    pub wavelength_nm: f64,
}
#[derive(Clone, Copy, Debug, Default, Serialize, Deserialize, PartialEq, Eq)]
#[serde(rename_all = "snake_case")]
pub enum ExcitationMethod {
    #[default]
    SpectrumPeak,
    Calibration,
}

#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ScanConfig {
    pub start_mm: f64,
    pub stop_mm: f64,
    pub step_mm: f64,
    pub repeats: u32,
    pub settle_s: f64,
    pub peak_min_nm: f64,
    pub peak_max_nm: f64,
}
impl Default for ScanConfig {
    fn default() -> Self {
        Self {
            start_mm: 18.,
            stop_mm: 18.2,
            step_mm: 0.02,
            repeats: 2,
            settle_s: 0.5,
            peak_min_nm: 765.,
            peak_max_nm: 795.,
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct ApparatusConfig {
    pub laser_model: String,
    pub spectrometer_model: String,
    pub camera_model: String,
    pub laser_optics: String,
    pub power_meter_model: String,
    pub power_sensor_model: String,
    pub setup_notes: String,
    pub power_meter_candidates: Vec<String>,
    pub conex_x_default_port: String,
    pub conex_y_default_port: String,
}
impl Default for ApparatusConfig {
    fn default() -> Self {
        Self {
            laser_model: "3900S".into(),
            spectrometer_model: "SpectraPro HRS-300".into(),
            camera_model: String::new(),
            laser_optics: String::new(),
            power_meter_model: String::new(),
            power_sensor_model: String::new(),
            setup_notes: String::new(),
            power_meter_candidates: vec!["PM100USB".into(), "PM100D".into()],
            conex_x_default_port: "COM3".into(),
            conex_y_default_port: "COM4".into(),
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
#[serde(default, deny_unknown_fields)]
pub struct Config {
    pub schema_version: u32,
    pub mode: Mode,
    pub sample: String,
    pub output_dir: String,
    pub apparatus: ApparatusConfig,
    pub spectrometer: SpectrometerConfig,
    pub background: BackgroundConfig,
    pub analysis: Option<AnalysisConfig>,
    pub stage: StageConfig,
    pub power: PowerConfig,
    pub scan: ScanConfig,
    pub excitation_peaks: Vec<PeakAnchor>,
    pub excitation_method: ExcitationMethod,
    pub excitation_min_nm: f64,
    pub excitation_max_nm: f64,
    pub excitation_min_snr: f64,
}
impl Default for Config {
    fn default() -> Self {
        Self {
            schema_version: 1,
            mode: Mode::Demo,
            sample: "sample".into(),
            output_dir: "output".into(),
            apparatus: Default::default(),
            spectrometer: Default::default(),
            background: Default::default(),
            analysis: None,
            stage: Default::default(),
            power: Default::default(),
            scan: Default::default(),
            excitation_method: ExcitationMethod::SpectrumPeak,
            excitation_min_nm: 680.,
            excitation_max_nm: 740.,
            excitation_min_snr: 5.,
            excitation_peaks: vec![
                PeakAnchor {
                    position_mm: 18.,
                    wavelength_nm: 700.,
                },
                PeakAnchor {
                    position_mm: 19.2,
                    wavelength_nm: 760.,
                },
            ],
        }
    }
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Plan {
    pub positions_mm: Vec<f64>,
    pub acquisition_count: usize,
    pub excitation_anchors: Vec<PeakAnchor>,
}
fn finite(v: f64, name: &str) -> Result<(), String> {
    if v.is_finite() {
        Ok(())
    } else {
        Err(format!("{name} must be finite"))
    }
}
impl Config {
    pub fn initial_analysis(&self) -> AnalysisConfig {
        self.analysis.clone().unwrap_or(AnalysisConfig {
            peak_nm: (self.scan.peak_min_nm + self.scan.peak_max_nm) * 0.5,
            peak_half_width_nm: (self.scan.peak_max_nm - self.scan.peak_min_nm) * 0.5,
            integral_min_nm: self.scan.peak_min_nm,
            integral_max_nm: self.scan.peak_max_nm,
        })
    }
    pub fn plan(&self) -> Result<Plan, String> {
        if self.schema_version != 1 {
            return Err("Unsupported configuration schema version".into());
        }
        if !self.background.timeout_s.is_finite()
            || self.background.timeout_s <= 0.
            || self.background.timeout_s > 120.
        {
            return Err("Shutter timeout must be finite and between 0 and 120 seconds".into());
        }
        if self.background.mode == BackgroundMode::Automatic {
            if self.background.backend == ShutterBackend::Mff
                && self.background.closed_state == self.background.open_state
            {
                return Err("Automatic shutter needs distinct open and closed states".into());
            }
            if self.mode == Mode::Hardware
                && self.background.backend == ShutterBackend::Mff
                && self.background.shutter_serial.trim().is_empty()
            {
                return Err("Enter the shutter serial for automatic BG acquisition".into());
            }
        }
        if self.background.channel > 255 {
            return Err(
                "Shutter channel must be within 0–255; actual channels are verified on connection"
                    .into(),
            );
        }
        if !self.power.wavelength_tolerance_nm.is_finite()
            || self.power.wavelength_tolerance_nm <= 0.
        {
            return Err("Power wavelength tolerance must be finite and positive".into());
        }
        let s = &self.scan;
        let h = &self.stage;
        h.validate_limits()?;
        for (v, n) in [
            (s.start_mm, "start"),
            (s.stop_mm, "stop"),
            (s.step_mm, "step"),
            (s.settle_s, "settle"),
            (h.min_mm, "stage minimum"),
            (h.max_mm, "stage maximum"),
            (h.tolerance_mm, "position tolerance"),
            (h.timeout_s, "stage timeout"),
            (self.spectrometer.exposure_ms, "exposure"),
            (self.spectrometer.center_nm, "spectrometer center"),
            (self.spectrometer.timeout_s, "acquisition timeout"),
            (self.power.correction_factor, "power correction"),
            (self.power.wavelength_nm, "power sensor wavelength"),
            (self.power.average_s, "power averaging"),
            (s.peak_min_nm, "peak minimum"),
            (s.peak_max_nm, "peak maximum"),
        ] {
            finite(v, n)?;
        }
        if s.step_mm == 0. || (s.stop_mm - s.start_mm) * s.step_mm < 0. {
            return Err("Step must point from start toward stop and cannot be zero".into());
        }
        if h.min_mm >= h.max_mm
            || s.start_mm.min(s.stop_mm) < h.min_mm
            || s.start_mm.max(s.stop_mm) > h.max_mm
        {
            return Err("Scan is outside configured stage limits".into());
        }
        if h.tolerance_mm <= 0.
            || h.timeout_s <= 0.
            || h.timeout_s > 3600.
            || self.spectrometer.timeout_s <= 0.
            || self.spectrometer.timeout_s > 86400.
        {
            return Err("Tolerances/timeouts are outside valid bounds".into());
        }
        if s.repeats == 0
            || s.settle_s < 0.
            || s.settle_s > 3600.
            || self.spectrometer.exposure_ms <= 0.
            || self.spectrometer.frames_combined == 0
            || self.spectrometer.center_nm <= 0.
            || self.power.correction_factor <= 0.
            || self.power.wavelength_nm <= 0.
            || self.power.average_s <= 0.
            || self.power.average_s > 60.
        {
            return Err("Exposure, repeats, averaging and correction must be positive".into());
        }
        let exposure_s =
            self.spectrometer.exposure_ms * self.spectrometer.frames_combined as f64 / 1000.;
        if !exposure_s.is_finite() || exposure_s + 5. > self.spectrometer.timeout_s {
            return Err(
                "Acquisition timeout must exceed combined exposure by at least 5 seconds".into(),
            );
        }
        if s.peak_min_nm <= 0. || s.peak_max_nm <= s.peak_min_nm {
            return Err("PL peak search interval is invalid".into());
        }
        let intervals = ((s.stop_mm - s.start_mm) / s.step_mm).abs();
        if intervals > MAX_ACQUISITIONS as f64 {
            return Err("Too many scan points".into());
        }
        let n = (intervals + 1e-8).floor() as usize + 1;
        let count = n
            .checked_mul(s.repeats as usize)
            .ok_or("Too many acquisitions")?;
        if count > MAX_ACQUISITIONS {
            return Err(format!(
                "At most {MAX_ACQUISITIONS} acquisitions are allowed"
            ));
        }
        if self.sample.trim().is_empty() || self.output_dir.trim().is_empty() {
            return Err("Sample and output directory are required".into());
        }
        let mut anchors = self.excitation_peaks.clone();
        if !self.excitation_min_snr.is_finite() || self.excitation_min_snr <= 0. {
            return Err("Excitation minimum SNR must be positive".into());
        }
        if self.excitation_min_nm <= 0.
            || !self.excitation_min_nm.is_finite()
            || !self.excitation_max_nm.is_finite()
            || self.excitation_max_nm <= self.excitation_min_nm
        {
            return Err("Invalid excitation peak search window".into());
        }
        if self.excitation_method == ExcitationMethod::Calibration
            || self.power.wavelength_mode == PowerWavelengthMode::PositionCalibration
        {
            if anchors.len() < 2 {
                return Err(
                    "Enter at least two CONEX position / excitation wavelength anchors".into(),
                );
            }
            for a in &anchors {
                finite(a.position_mm, "anchor position")?;
                finite(a.wavelength_nm, "anchor wavelength")?;
                if a.wavelength_nm <= 0. {
                    return Err("Excitation wavelengths must be positive".into());
                }
            }
            anchors.sort_by(|a, b| a.position_mm.total_cmp(&b.position_mm));
            let direction = (anchors[1].wavelength_nm - anchors[0].wavelength_nm).signum();
            if direction == 0.
                || anchors.windows(2).any(|p| {
                    p[1].position_mm <= p[0].position_mm
                        || (p[1].wavelength_nm - p[0].wavelength_nm) * direction <= 0.
                })
            {
                return Err(
                    "Anchor positions must be unique and wavelengths strictly monotonic".into(),
                );
            }
        }
        let positions: Vec<_> = (0..n).map(|i| s.start_mm + i as f64 * s.step_mm).collect();
        if self.excitation_method == ExcitationMethod::Calibration
            || self.power.wavelength_mode == PowerWavelengthMode::PositionCalibration
        {
            for &p in &positions {
                interpolate_excitation(&anchors, p)?;
            }
        }
        if self.mode == Mode::Hardware && self.spectrometer.experiment.trim().is_empty() {
            return Err("Enter a LightField experiment name".into());
        }
        Ok(Plan {
            positions_mm: positions,
            acquisition_count: count,
            excitation_anchors: anchors,
        })
    }
}
pub fn interpolate_excitation(anchors: &[PeakAnchor], p: f64) -> Result<f64, String> {
    if anchors.len() < 2 || !p.is_finite() {
        return Err("Missing or invalid excitation calibration".into());
    }
    let lo = anchors[0].position_mm;
    let hi = anchors[anchors.len() - 1].position_mm;
    if p < lo - 1e-9 || p > hi + 1e-9 {
        return Err(format!(
            "CONEX {p:0.5} mm lies outside excitation calibration ({lo:0.5}–{hi:0.5} mm)"
        ));
    }
    let p = p.clamp(lo, hi);
    let upper = anchors
        .partition_point(|a| a.position_mm < p)
        .clamp(1, anchors.len() - 1);
    let pair = &anchors[upper - 1..=upper];
    Ok(pair[0].wavelength_nm
        + (pair[1].wavelength_nm - pair[0].wavelength_nm) * (p - pair[0].position_mm)
            / (pair[1].position_mm - pair[0].position_mm))
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Spectrum {
    pub wavelength_nm: Vec<f64>,
    pub counts: Vec<f64>,
    pub spe_path: Option<String>,
    #[serde(default)]
    pub settings: serde_json::Value,
}
impl Spectrum {
    pub fn validate(&self) -> Result<(), String> {
        let n = self.wavelength_nm.len();
        if n < 2 || n != self.counts.len() || n > 1_000_000 {
            return Err("Invalid spectrum dimensions".into());
        }
        if self
            .wavelength_nm
            .iter()
            .any(|x| !x.is_finite() || *x <= 0.)
            || self.counts.iter().any(|x| !x.is_finite())
            || self.wavelength_nm.windows(2).any(|w| w[0] >= w[1])
        {
            return Err("Invalid spectrum values or non-increasing wavelength axis".into());
        }
        Ok(())
    }
}
pub fn subtract_background(signal: &Spectrum, bg: &Spectrum) -> Result<Vec<f64>, String> {
    signal.validate()?;
    bg.validate()?;
    if signal.wavelength_nm.len() != bg.wavelength_nm.len()
        || signal
            .wavelength_nm
            .iter()
            .zip(&bg.wavelength_nm)
            .any(|(a, b)| (a - b).abs() > 1e-7)
    {
        return Err(
            "Background wavelength axis does not match acquisition; acquire a new BG".into(),
        );
    }
    // LightField setting snapshots are compared in the acquisition adapter as well.
    let corrected: Vec<_> = signal
        .counts
        .iter()
        .zip(&bg.counts)
        .map(|(s, b)| s - b)
        .collect();
    if corrected.iter().any(|v| !v.is_finite()) {
        return Err("BG subtraction overflowed".into());
    }
    Ok(corrected)
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Sample {
    #[serde(default)]
    pub file_base: String,
    pub point_index: usize,
    pub repeat_index: u32,
    pub target_mm: f64,
    pub actual_mm: f64,
    pub excitation_nm: f64,
    pub excitation_source: String,
    pub power_w: f64,
    pub sample_power_w: f64,
    pub power_timestamp_unix_s: f64,
    #[serde(default)]
    pub power_sensor_wavelength_nm: Option<f64>,
    pub acquisition_started_unix_s: f64,
    pub timestamp_unix_s: f64,
    pub pl_peak_nm: Option<f64>,
    pub pl_integral_counts_nm: f64,
    pub spectrum: Spectrum,
    pub corrected_counts: Vec<f64>,
}
#[derive(Clone, Debug, Serialize, Deserialize)]
pub struct Acquisition {
    pub file_base: String,
    pub point_index: usize,
    pub repeat_index: u32,
    pub target_mm: f64,
    pub actual_mm: f64,
    pub power_w: f64,
    pub sample_power_w: f64,
    pub power_timestamp_unix_s: f64,
    #[serde(default)]
    pub power_sensor_wavelength_nm: Option<f64>,
    pub acquisition_started_unix_s: f64,
    pub timestamp_unix_s: f64,
    pub spectrum: Spectrum,
    pub corrected_counts: Vec<f64>,
}
impl Acquisition {
    pub fn analyze(
        self,
        c: &Config,
        plan: &Plan,
        analysis: &AnalysisConfig,
    ) -> Result<Sample, String> {
        analysis.validate(&self.spectrum.wavelength_nm)?;
        let excitation_nm = match c.excitation_method {
            ExcitationMethod::Calibration => {
                interpolate_excitation(&plan.excitation_anchors, self.actual_mm)?
            }
            ExcitationMethod::SpectrumPeak => excitation_peak(
                &self.spectrum.wavelength_nm,
                &self.corrected_counts,
                c.excitation_min_nm,
                c.excitation_max_nm,
                c.excitation_min_snr,
            )?,
        };
        if c.power.wavelength_mode == PowerWavelengthMode::PositionCalibration {
            let sensor = self
                .power_sensor_wavelength_nm
                .ok_or("Missing power sensor wavelength readback")?;
            if !sensor.is_finite()
                || (sensor - excitation_nm).abs() > c.power.wavelength_tolerance_nm
            {
                return Err(format!(
                    "Power sensor wavelength {sensor:.6} nm disagrees with excitation {excitation_nm:.6} nm; recalibrate position/wavelength before retrying"
                ));
            }
        }
        let peak = peak_nm(
            &self.spectrum.wavelength_nm,
            &self.corrected_counts,
            analysis.peak_nm - analysis.peak_half_width_nm,
            analysis.peak_nm + analysis.peak_half_width_nm,
        );
        let integral = integrate_nm(
            &self.spectrum.wavelength_nm,
            &self.corrected_counts,
            analysis.integral_min_nm,
            analysis.integral_max_nm,
        )?;
        Ok(Sample {
            file_base: self.file_base,
            point_index: self.point_index,
            repeat_index: self.repeat_index,
            target_mm: self.target_mm,
            actual_mm: self.actual_mm,
            excitation_nm,
            excitation_source: match c.excitation_method {
                ExcitationMethod::Calibration => "user_calibration_piecewise_linear",
                ExcitationMethod::SpectrumPeak => "maximum_bin_median_MAD_SNR_screened",
            }
            .into(),
            power_w: self.power_w,
            sample_power_w: self.sample_power_w,
            power_timestamp_unix_s: self.power_timestamp_unix_s,
            power_sensor_wavelength_nm: self.power_sensor_wavelength_nm,
            acquisition_started_unix_s: self.acquisition_started_unix_s,
            timestamp_unix_s: self.timestamp_unix_s,
            pl_peak_nm: peak,
            pl_integral_counts_nm: integral,
            spectrum: self.spectrum,
            corrected_counts: self.corrected_counts,
        })
    }
}
/// Readable physical names with stable indices to prevent rounding collisions.
pub fn acquisition_name(
    position_mm: f64,
    sample_power_w: f64,
    point: usize,
    repeat: u32,
) -> Result<String, String> {
    finite(position_mm, "file position")?;
    finite(sample_power_w, "file power")?;
    if sample_power_w <= 0. || (sample_power_w * 1e6).is_infinite() {
        return Err("Invalid power for filename".into());
    }
    let trim = |s: String| {
        let s = s.trim_end_matches('0').trim_end_matches('.');
        if s == "-0" { "0".into() } else { s.to_string() }
    };
    Ok(format!(
        "Y_{}mm_{}uW_p{point:05}_r{repeat:03}",
        trim(format!("{position_mm:.6}")),
        trim(format!("{:.4}", sample_power_w * 1e6))
    ))
}
pub fn peak_nm(wavelength: &[f64], counts: &[f64], min: f64, max: f64) -> Option<f64> {
    wavelength
        .iter()
        .zip(counts)
        .filter(|(w, c)| **w >= min && **w <= max && c.is_finite())
        .max_by(|a, b| a.1.total_cmp(b.1))
        .map(|(w, _)| *w)
}
/// Maximum detector bin with robust median/MAD noise screening.
pub fn excitation_peak(
    wavelength: &[f64],
    counts: &[f64],
    min: f64,
    max: f64,
    min_snr: f64,
) -> Result<f64, String> {
    let candidates: Vec<_> = wavelength
        .iter()
        .zip(counts)
        .enumerate()
        .filter(|(_, (w, _))| **w >= min && **w <= max)
        .collect();
    if candidates.len() < 5 {
        return Err("Excitation window needs at least five spectral bins".into());
    }
    let (index, (_, maximum)) = candidates
        .iter()
        .copied()
        .max_by(|a, b| a.1.1.total_cmp(b.1.1))
        .ok_or("No excitation bins")?;
    if index == candidates[0].0 || index == candidates[candidates.len() - 1].0 {
        return Err("Excitation peak is at search-window boundary".into());
    }
    let mut values: Vec<_> = candidates.iter().map(|(_, (_, c))| **c).collect();
    values.sort_by(f64::total_cmp);
    let baseline = values[values.len() / 2];
    let mut deviations: Vec<_> = values.iter().map(|v| (v - baseline).abs()).collect();
    deviations.sort_by(f64::total_cmp);
    let noise = (1.4826 * deviations[deviations.len() / 2]).max(1e-12);
    if *maximum <= 0. || (*maximum - baseline) / noise < min_snr {
        return Err("No excitation peak above configured median/MAD SNR threshold".into());
    }
    Ok(wavelength[index])
}
pub fn integrate_nm(wavelength: &[f64], counts: &[f64], min: f64, max: f64) -> Result<f64, String> {
    if wavelength.len() != counts.len() || wavelength.len() < 2 {
        return Err("Invalid integration arrays".into());
    }
    if min < wavelength[0] || max > wavelength[wavelength.len() - 1] || min >= max {
        return Err("PL integration window lies outside spectrum".into());
    }
    let mut sum = 0.;
    for i in 1..wavelength.len() {
        let a = wavelength[i - 1];
        let b = wavelength[i];
        let lo = a.max(min);
        let hi = b.min(max);
        if hi > lo {
            let slope = (counts[i] - counts[i - 1]) / (b - a);
            let y0 = counts[i - 1] + slope * (lo - a);
            let y1 = counts[i - 1] + slope * (hi - a);
            sum += (y0 + y1) * 0.5 * (hi - lo);
        }
    }
    if !sum.is_finite() {
        return Err("PL integral overflowed".into());
    }
    Ok(sum)
}
pub fn display_spectrum(wavelength: &[f64], counts: &[f64], energy: bool) -> Vec<[f64; 2]> {
    let mut points: Vec<_> = wavelength
        .iter()
        .zip(counts)
        .map(|(w, c)| [if energy { HC_EV_NM / w } else { *w }, *c])
        .collect();
    if energy {
        points.reverse();
    }
    points
}
#[cfg(test)]
mod tests {
    #[test]
    fn apparatus_defaults_are_saved_without_changing_the_active_port() {
        let mut profile = serde_json::to_value(super::Config::default()).unwrap();
        profile.as_object_mut().unwrap().remove("apparatus");
        profile["stage"]["com_port"] = serde_json::json!("COM8");
        let config: super::Config = serde_json::from_value(profile).unwrap();
        assert_eq!(config.stage.com_port, "COM8");
        assert_eq!(config.apparatus.conex_x_default_port, "COM3");
        assert_eq!(config.apparatus.conex_y_default_port, "COM4");
        let saved = serde_json::to_value(&config).unwrap();
        assert_eq!(saved["apparatus"]["laser_model"], "3900S");
        assert_eq!(
            saved["apparatus"]["spectrometer_model"],
            "SpectraPro HRS-300"
        );
        assert_eq!(
            saved["apparatus"]["power_meter_candidates"],
            serde_json::json!(["PM100USB", "PM100D"])
        );
    }
    use super::*;
    #[test]
    fn external_shutter_profile_does_not_require_mff_serial_or_states() {
        let mut c = Config {
            mode: Mode::Hardware,
            ..Default::default()
        };
        c.background.mode = BackgroundMode::Automatic;
        c.background.backend = ShutterBackend::KohdalabShutter;
        c.background.closed_state = 0;
        c.background.open_state = 0;
        c.spectrometer.experiment = "test".into();
        c.power.resource = "USB::test".into();
        c.power.driver_path = "driver".into();
        assert!(c.plan().is_ok());
        c.background.channel = 256;
        assert!(c.plan().is_err());
        let legacy: BackgroundConfig =
            serde_json::from_str(r#"{"mode":"automatic","shutter_serial":"123"}"#).unwrap();
        assert_eq!(legacy.backend, ShutterBackend::Mff);
    }
    #[test]
    fn shutter_states_are_explicit_device_values_and_not_assumed_zero_one() {
        let mut c = Config::default();
        c.background.mode = BackgroundMode::Automatic;
        c.background.closed_state = 7;
        c.background.open_state = 3;
        assert!(c.plan().is_ok());
        c.background.open_state = 7;
        assert!(c.plan().is_err());
    }
    #[test]
    fn conex_travel_cannot_be_widened_beyond_zero_to_twenty_five() {
        let mut c = Config::default();
        c.stage.min_mm = -0.001;
        assert!(c.plan().is_err());
        c.stage.min_mm = 0.;
        c.stage.max_mm = 25.001;
        assert!(c.plan().is_err());
        c.stage.max_mm = 25.;
        assert!(c.plan().is_ok());
    }
    #[test]
    fn physical_filename_keeps_decimal_and_uses_indices_for_uniqueness() {
        assert_eq!(
            acquisition_name(18.6, 40e-6, 0, 0).unwrap(),
            "Y_18.6mm_40uW_p00000_r000"
        );
        assert_ne!(
            acquisition_name(18.6, 40e-6, 0, 0).unwrap(),
            acquisition_name(18.6, 40e-6, 0, 1).unwrap()
        );
        assert!(acquisition_name(18.6, 0., 0, 0).is_err());
        assert!(acquisition_name(f64::NAN, 40e-6, 0, 0).is_err());
    }
    #[test]
    fn analysis_has_independent_peak_search_and_integral_bounds() {
        let a = AnalysisConfig {
            peak_nm: 780.,
            peak_half_width_nm: 2.,
            integral_min_nm: 750.,
            integral_max_nm: 790.,
        };
        assert!(a.validate(&[740., 760., 780., 800.]).is_ok());
        let mut invalid = a.clone();
        invalid.integral_max_nm = 810.;
        assert!(invalid.validate(&[740., 760., 780., 800.]).is_err());
        invalid = a;
        invalid.peak_half_width_nm = 0.;
        assert!(invalid.validate(&[740., 760., 780., 800.]).is_err());
    }
    #[test]
    fn scan_directions_and_endpoint() {
        let mut c = Config::default();
        assert_eq!(c.plan().unwrap().positions_mm.len(), 11);
        c.scan.start_mm = 18.2;
        c.scan.stop_mm = 18.;
        c.scan.step_mm = -0.02;
        assert_eq!(c.plan().unwrap().positions_mm.len(), 11);
        c.scan.step_mm = 0.02;
        assert!(c.plan().is_err());
        c.scan.start_mm = 18.;
        c.scan.stop_mm = 18.21;
        c.scan.step_mm = 0.02;
        assert_eq!(*c.plan().unwrap().positions_mm.last().unwrap(), 18.2);
    }
    #[test]
    fn reject_nonfinite_limits_and_large_jobs() {
        let mut c = Config::default();
        c.scan.step_mm = f64::NAN;
        assert!(c.plan().is_err());
        c = Config::default();
        c.scan.repeats = 10000;
        assert!(c.plan().is_err());
        c = Config::default();
        c.scan.stop_mm = 26.;
        assert!(c.plan().is_err());
    }
    #[test]
    fn duration_bounds_and_unknown_settings_are_rejected() {
        let mut c = Config::default();
        c.scan.settle_s = f64::MAX;
        assert!(c.plan().is_err());
        c = Config::default();
        c.spectrometer.exposure_ms = 200000.;
        assert!(c.plan().is_err());
        assert!(serde_json::from_str::<Config>(r#"{"scan_ragne":{}}"#).is_err());
    }
    #[test]
    fn calibration_no_extrapolation_and_duplicates() {
        let mut c = Config::default();
        let p = c.plan().unwrap();
        assert!((interpolate_excitation(&p.excitation_anchors, 18.1).unwrap() - 705.).abs() < 1e-8);
        assert!(interpolate_excitation(&p.excitation_anchors, 17.).is_err());
        c.excitation_method = ExcitationMethod::Calibration;
        c.excitation_peaks[1].position_mm = 18.;
        assert!(c.plan().is_err());
    }
    #[test]
    fn bg_keeps_negative_and_checks_axis() {
        let s = Spectrum {
            wavelength_nm: vec![700., 710.],
            counts: vec![10., 20.],
            spe_path: None,
            settings: serde_json::Value::Null,
        };
        let mut b = s.clone();
        b.counts = vec![12., 3.];
        assert_eq!(subtract_background(&s, &b).unwrap(), vec![-2., 17.]);
        b.wavelength_nm[1] = 711.;
        assert!(subtract_background(&s, &b).is_err());
    }
    #[test]
    fn excitation_detection_rejects_flat_noise_and_boundary() {
        let w: Vec<_> = (0..21).map(|i| 690. + i as f64).collect();
        let mut y = vec![0.; 21];
        assert!(excitation_peak(&w, &y, 690., 710., 5.).is_err());
        for (i, v) in y.iter_mut().enumerate() {
            *v = (i as f64).sin();
        }
        assert!(excitation_peak(&w, &y, 690., 710., 5.).is_err());
        y[10] = 100.;
        assert_eq!(excitation_peak(&w, &y, 690., 710., 5.).unwrap(), 700.);
        y[0] = 1000.;
        assert!(excitation_peak(&w, &y, 690., 710., 5.).is_err());
    }
    #[test]
    fn energy_conversion_preserves_bins() {
        let p = display_spectrum(&[700., 800.], &[1., 2.], true);
        assert!(p[0][0] < p[1][0]);
        assert_eq!(p[0][1], 2.);
        assert!((p[0][0] - HC_EV_NM / 800.).abs() < 1e-12);
    }
}
