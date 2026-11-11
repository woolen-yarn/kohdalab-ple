//! Immutable run inputs, raw/processed CSV and crash-readable run status.
use crate::{Config, HC_EV_NM, Plan, Sample, Spectrum};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File, OpenOptions},
    io::{BufWriter, Read, Write},
    path::{Path, PathBuf},
    time::{SystemTime, UNIX_EPOCH},
};
pub fn unix_s() -> f64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs_f64()
}
fn sha256(path: &Path) -> Result<String, String> {
    let mut file = File::open(path).map_err(|e| e.to_string())?;
    let mut hash = Sha256::new();
    let mut buffer = [0_u8; 65_536];
    loop {
        let count = file.read(&mut buffer).map_err(|e| e.to_string())?;
        if count == 0 {
            break;
        }
        hash.update(&buffer[..count]);
    }
    Ok(format!("{:x}", hash.finalize()))
}
pub fn atomic_json(path: &Path, value: &impl serde::Serialize) -> Result<(), String> {
    let temp = path.with_extension("json.tmp");
    let file = File::create(&temp).map_err(|e| e.to_string())?;
    let mut w = BufWriter::new(file);
    serde_json::to_writer_pretty(&mut w, value).map_err(|e| e.to_string())?;
    w.write_all(b"\n").map_err(|e| e.to_string())?;
    w.flush().map_err(|e| e.to_string())?;
    w.get_ref().sync_all().map_err(|e| e.to_string())?;
    // Windows rename cannot replace an existing destination. ReplaceFileW is not
    // required here: original status remains recoverable as status.previous.json.
    if path.exists() {
        let previous = path.with_extension("previous.json");
        if previous.exists() {
            fs::remove_file(&previous).map_err(|e| e.to_string())?;
        }
        fs::rename(path, &previous).map_err(|e| e.to_string())?;
    }
    fs::rename(temp, path).map_err(|e| e.to_string())
}
pub struct Recording {
    pub directory: PathBuf,
    events: File,
    summary: csv::Writer<File>,
    analysis: csv::Writer<File>,
    raw: csv::Writer<File>,
    log: File,
    raw_event_index: u64,
    completed: usize,
    planned: usize,
    preview_acquired: bool,
    raw_saved: std::collections::HashSet<String>,
    analysis_confirmed: bool,
    pub run_id: String,
}
impl Recording {
    pub fn create(c: &Config, p: &Plan) -> Result<Self, String> {
        let root = Path::new(&c.output_dir);
        fs::create_dir_all(root).map_err(|e| e.to_string())?;
        let root = fs::canonicalize(root).map_err(|e| e.to_string())?;
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_err(|e| e.to_string())?
            .as_nanos();
        let slug: String = c
            .sample
            .chars()
            .map(|x| {
                if x.is_ascii_alphanumeric() || x == '-' || x == '_' {
                    x
                } else {
                    '_'
                }
            })
            .take(48)
            .collect();
        let run_id = format!("{stamp}-{}", std::process::id());
        let directory = root.join(format!("{slug}_{run_id}"));
        fs::create_dir(&directory).map_err(|e| e.to_string())?;
        for folder in ["raw", "processed", "metadata"] {
            fs::create_dir(directory.join(folder)).map_err(|e| e.to_string())?;
        }
        atomic_json(
            &directory.join("settings.json"),
            &json!({"schema_version":1,"application":"kohdalab-ple","application_version":env!("CARGO_PKG_VERSION"),"run_id":run_id,"created_unix_s":unix_s(),"config":c,"plan":p,"background_policy":"fresh matched BG before each run; mandatory subtraction; no clipping","excitation_method":c.excitation_method,"energy_display":"bin counts preserved; no spectral-density Jacobian","raw_spe_available":c.mode==crate::Mode::Hardware}),
        )?;
        let events = OpenOptions::new()
            .create_new(true)
            .append(true)
            .open(directory.join("events.jsonl"))
            .map_err(|e| e.to_string())?;
        let analysis_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("analysis.csv"))
            .map_err(|e| e.to_string())?;
        let mut analysis = csv::Writer::from_writer(analysis_file);
        analysis
            .write_record([
                "run_id",
                "point_index",
                "repeat_index",
                "target_mm",
                "actual_mm",
                "acquisition_completed_unix_s",
                "excitation_nm",
                "emission_nm",
                "corrected_counts",
                "corrected_counts_per_W",
                "meter_power_W",
                "sample_power_W",
                "PL_peak_nm",
                "PL_integral_counts_nm",
                "raw_spectrum_csv",
                "acquisition_metadata_json",
            ])
            .map_err(|e| e.to_string())?;
        analysis.flush().map_err(|e| e.to_string())?;
        let raw_file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("analysis.raw.csv"))
            .map_err(|e| e.to_string())?;
        let mut raw = csv::Writer::from_writer(raw_file);
        raw.write_record([
            "run_id",
            "event_index",
            "event_type",
            "point_index",
            "repeat_index",
            "host_time_unix_s",
            "result",
            "error",
            "details_json",
        ])
        .map_err(|e| e.to_string())?;
        raw.flush().map_err(|e| e.to_string())?;
        let log = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(directory.join("analysis.log"))
            .map_err(|e| e.to_string())?;
        let mut summary =
            csv::Writer::from_path(directory.join("points.csv")).map_err(|e| e.to_string())?;
        summary
            .write_record([
                "point_index",
                "repeat_index",
                "target_mm",
                "actual_mm",
                "excitation_nm",
                "excitation_eV",
                "power_W",
                "sample_power_W",
                "power_timestamp_unix_s",
                "acquisition_started_unix_s",
                "acquisition_completed_unix_s",
                "PL_peak_nm",
                "PL_integral_counts_nm",
                "spectrum_base",
                "power_sensor_wavelength_nm",
            ])
            .map_err(|e| e.to_string())?;
        summary.flush().map_err(|e| e.to_string())?;
        let mut r = Self {
            directory,
            events,
            summary,
            analysis,
            raw,
            log,
            raw_event_index: 0,
            completed: 0,
            planned: p.acquisition_count,
            preview_acquired: false,
            raw_saved: Default::default(),
            analysis_confirmed: false,
            run_id,
        };
        r.log("INFO", "run_created", json!({"planned_acquisitions": p.acquisition_count, "simulation": c.mode == crate::Mode::Demo}))?;
        atomic_json(
            &r.directory.join("analysis.run.json"),
            &bundle_manifest(c, p, &r.run_id, "awaiting_background", true, 0, None),
        )?;
        r.event("run_created", json!({}))?;
        r.status("awaiting_background", None)?;
        Ok(r)
    }
    pub fn base_path(&self, name: &str) -> String {
        self.directory
            .join("raw")
            .join(name)
            .to_string_lossy()
            .into_owned()
    }
    pub fn event(&mut self, kind: &str, data: Value) -> Result<(), String> {
        let at = unix_s();
        serde_json::to_writer(
            &mut self.events,
            &json!({"time_unix_s":at,"kind":kind,"data":data}),
        )
        .map_err(|e| e.to_string())?;
        self.events
            .write_all(b"\n")
            .and_then(|_| self.events.flush())
            .map_err(|e| e.to_string())?;
        self.raw_event_index += 1;
        let point = data
            .get("point_index")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .unwrap_or_default();
        let repeat = data
            .get("repeat_index")
            .and_then(Value::as_u64)
            .map(|v| v.to_string())
            .unwrap_or_default();
        let error = data.get("error").and_then(Value::as_str).unwrap_or("");
        let result = if kind == "run_finished" {
            data.get("state")
                .and_then(Value::as_str)
                .unwrap_or("unknown")
        } else if error.is_empty() {
            "ok"
        } else {
            "error"
        };
        self.raw
            .write_record([
                self.run_id.as_str(),
                &self.raw_event_index.to_string(),
                kind,
                &point,
                &repeat,
                &at.to_string(),
                result,
                error,
                &serde_json::to_string(&data).map_err(|e| e.to_string())?,
            ])
            .map_err(|e| e.to_string())?;
        self.raw.flush().map_err(|e| e.to_string())
    }
    fn log(&mut self, level: &str, event: &str, context: Value) -> Result<(), String> {
        writeln!(
            self.log,
            "{}\t{}\t{}\t{}\t{}",
            unix_s(),
            level,
            self.run_id,
            event,
            serde_json::to_string(&context).map_err(|e| e.to_string())?
        )
        .map_err(|e| e.to_string())?;
        self.log.flush().map_err(|e| e.to_string())
    }
    pub fn excitation_extrapolated(
        &mut self,
        point_index: usize,
        repeat_index: u32,
        actual_mm: f64,
    ) -> Result<(), String> {
        self.log(
            "INFO",
            "excitation_extrapolated",
            json!({"point_index":point_index,"repeat_index":repeat_index,"actual_mm":actual_mm}),
        )
    }
    pub fn hardware(&mut self, hardware: Value) -> Result<(), String> {
        atomic_json(
            &self.directory.join("hardware.json"),
            &json!({"schema_version":1,"captured_unix_s":unix_s(),"hardware":hardware}),
        )?;
        self.event("hardware_recorded", hardware)
    }
    pub fn background(&mut self, b: &Spectrum) -> Result<(), String> {
        b.validate()?;
        write_raw_csv(&self.directory.join("raw/background.csv"), b)?;
        atomic_json(&self.directory.join("metadata/background.json"), b)?;
        self.event("background_saved", json!({"spe_path":b.spe_path}))?;
        self.status("awaiting_signal_confirmation", None)
    }
    pub fn acquired(&mut self, a: &crate::Acquisition) -> Result<(), String> {
        if self.raw_saved.contains(&a.file_base) {
            return Err("Duplicate raw acquisition basename".into());
        }
        write_raw_csv(
            &self
                .directory
                .join("raw")
                .join(format!("{}.csv", a.file_base)),
            &a.spectrum,
        )?;
        self.raw_saved.insert(a.file_base.clone());
        atomic_json(
            &self.directory.join("metadata").join(format!(
                "acquisition_{:05}_{:03}.json",
                a.point_index, a.repeat_index
            )),
            &json!({"file_base":a.file_base,"point_index":a.point_index,"repeat_index":a.repeat_index,"target_mm":a.target_mm,"actual_mm":a.actual_mm,"power_w":a.power_w,"sample_power_w":a.sample_power_w,"power_sensor_wavelength_nm":a.power_sensor_wavelength_nm,"power_observation":a.power_observation,"power_timestamp_unix_s":a.power_timestamp_unix_s,"acquisition_started_unix_s":a.acquisition_started_unix_s,"timestamp_unix_s":a.timestamp_unix_s,"spe_path":a.spectrum.spe_path,"settings":a.spectrum.settings,"background":"metadata/background.json"}),
        )?;
        self.event("raw_acquired",json!({"file_base":a.file_base,"point_index":a.point_index,"repeat_index":a.repeat_index,
            "target_mm":a.target_mm,"actual_mm":a.actual_mm,"power_w":a.power_w,"sample_power_w":a.sample_power_w,
            "power_observation":a.power_observation,"raw_spectrum_csv":format!("raw/{}.csv", a.file_base),"spe_path":a.spectrum.spe_path}))
    }
    pub fn preview(&mut self, a: &crate::Acquisition) -> Result<(), String> {
        if !self.raw_saved.contains(&a.file_base) {
            self.acquired(a)?;
        }
        atomic_json(&self.directory.join("metadata/preview.json"), a)?;
        self.preview_acquired = true;
        self.event("preview_saved",json!({"file_base":a.file_base,"point_index":a.point_index,"repeat_index":a.repeat_index}))?;
        self.status("awaiting_analysis_selection", None)
    }
    pub fn analysis(&mut self, c: &Config, p: &Plan) -> Result<(), String> {
        atomic_json(
            &self.directory.join("analysis.json"),
            &json!({"schema_version":1,"confirmed_unix_s":unix_s(),"analysis":c.initial_analysis(),"excitation_method":c.excitation_method,"excitation_min_nm":c.excitation_min_nm,"excitation_max_nm":c.excitation_max_nm,"excitation_min_snr":c.excitation_min_snr,"preview":"metadata/preview.json"}),
        )?;
        atomic_json(
            &self.directory.join("accepted_settings.json"),
            &json!({"schema_version":1,"config":c,"plan":p,"analysis_confirmed_unix_s":unix_s()}),
        )?;
        self.analysis_confirmed = true;
        self.event(
            "analysis_confirmed",
            json!({"analysis":c.initial_analysis()}),
        )?;
        self.status("running", None)
    }
    pub fn sample(&mut self, s: &Sample, bg: &Spectrum) -> Result<(), String> {
        let name = if s.file_base.is_empty() {
            format!("point_{:05}_repeat_{:03}", s.point_index, s.repeat_index)
        } else {
            s.file_base.clone()
        };
        let raw = self.directory.join("raw").join(format!("{name}.csv"));
        if !self.raw_saved.contains(&name) {
            write_raw_csv(&raw, &s.spectrum)?;
            self.raw_saved.insert(name.clone());
        } else if !raw.is_file() {
            return Err("Saved preview raw CSV is missing".into());
        }
        let mut csv =
            csv::Writer::from_path(self.directory.join("processed").join(format!("{name}.csv")))
                .map_err(|e| e.to_string())?;
        csv.write_record([
            "emission_nm",
            "emission_eV",
            "raw_counts",
            "background_counts",
            "corrected_counts",
            "corrected_counts_per_W",
        ])
        .map_err(|e| e.to_string())?;
        for i in 0..s.spectrum.counts.len() {
            csv.write_record([
                s.spectrum.wavelength_nm[i].to_string(),
                (HC_EV_NM / s.spectrum.wavelength_nm[i]).to_string(),
                s.spectrum.counts[i].to_string(),
                bg.counts[i].to_string(),
                s.corrected_counts[i].to_string(),
                (s.corrected_counts[i] / s.sample_power_w).to_string(),
            ])
            .map_err(|e| e.to_string())?;
        }
        csv.flush().map_err(|e| e.to_string())?;
        csv.get_ref().sync_all().map_err(|e| e.to_string())?;
        let metadata_path = self.directory.join("metadata").join(format!("{name}.json"));
        atomic_json(&metadata_path, s)?;
        let raw_reference = format!("raw/{name}.csv");
        let metadata_reference = format!("metadata/{name}.json");
        for i in 0..s.spectrum.counts.len() {
            self.analysis
                .write_record([
                    self.run_id.as_str(),
                    &s.point_index.to_string(),
                    &s.repeat_index.to_string(),
                    &s.target_mm.to_string(),
                    &s.actual_mm.to_string(),
                    &s.timestamp_unix_s.to_string(),
                    &s.excitation_nm.to_string(),
                    &s.spectrum.wavelength_nm[i].to_string(),
                    &s.corrected_counts[i].to_string(),
                    &(s.corrected_counts[i] / s.sample_power_w).to_string(),
                    &s.power_w.to_string(),
                    &s.sample_power_w.to_string(),
                    &s.pl_peak_nm.map(|v| v.to_string()).unwrap_or_default(),
                    &s.pl_integral_counts_nm.to_string(),
                    &raw_reference,
                    &metadata_reference,
                ])
                .map_err(|e| e.to_string())?;
        }
        self.analysis.flush().map_err(|e| e.to_string())?;
        self.analysis
            .get_ref()
            .sync_all()
            .map_err(|e| e.to_string())?;
        self.summary
            .write_record([
                s.point_index.to_string(),
                s.repeat_index.to_string(),
                s.target_mm.to_string(),
                s.actual_mm.to_string(),
                s.excitation_nm.to_string(),
                (HC_EV_NM / s.excitation_nm).to_string(),
                s.power_w.to_string(),
                s.sample_power_w.to_string(),
                s.power_timestamp_unix_s.to_string(),
                s.acquisition_started_unix_s.to_string(),
                s.timestamp_unix_s.to_string(),
                s.pl_peak_nm.map(|v| v.to_string()).unwrap_or_default(),
                s.pl_integral_counts_nm.to_string(),
                name,
                s.power_sensor_wavelength_nm
                    .map(|v| v.to_string())
                    .unwrap_or_default(),
            ])
            .map_err(|e| e.to_string())?;
        self.summary.flush().map_err(|e| e.to_string())?;
        self.summary
            .get_ref()
            .sync_all()
            .map_err(|e| e.to_string())?;
        self.completed += 1;
        self.log("INFO", "acquisition_saved", json!({"point_index":s.point_index,"repeat_index":s.repeat_index,"completed":self.completed}))?;
        self.event("acquisition_saved", json!({
            "point_index":s.point_index,"repeat_index":s.repeat_index,"completed":self.completed,
            "target_mm":s.target_mm,"actual_mm":s.actual_mm,
            "meter_power_W":s.power_w,"sample_power_W":s.sample_power_w,
            "power_sensor_wavelength_nm":s.power_sensor_wavelength_nm,
            "power_observation":s.power_observation,
            "power_timestamp_unix_s":s.power_timestamp_unix_s,
            "acquisition_started_unix_s":s.acquisition_started_unix_s,
            "acquisition_completed_unix_s":s.timestamp_unix_s,
            "raw_spectrum_csv":raw_reference,"acquisition_metadata_json":metadata_reference
        }))?;
        self.status("running", None)
    }
    pub fn status(&self, state: &str, error: Option<&str>) -> Result<(), String> {
        atomic_json(
            &self.directory.join("status.json"),
            &json!({"schema_version":1,"run_id":self.run_id,"state":state,"completed_acquisitions":self.completed,"preview_acquired":self.preview_acquired,"analysis_confirmed":self.analysis_confirmed,"planned_acquisitions":self.planned,"updated_unix_s":unix_s(),"error":error}),
        )
    }
    pub fn finish(&mut self, state: &str, error: Option<&str>) -> Result<(), String> {
        self.event("run_finished", json!({"state":state,"error":error}))?;
        self.events.sync_all().map_err(|e| e.to_string())?;
        self.raw.flush().map_err(|e| e.to_string())?;
        self.raw.get_ref().sync_all().map_err(|e| e.to_string())?;
        self.log(
            if error.is_some() || matches!(state, "failed" | "aborted") {
                "ERROR"
            } else {
                "INFO"
            },
            "run_finished",
            json!({"state":state,"completed":self.completed,"error":error}),
        )?;
        self.log.sync_all().map_err(|e| e.to_string())?;
        self.status(state, error)?;
        let settings: Value = serde_json::from_reader(
            File::open(self.directory.join("settings.json")).map_err(|e| e.to_string())?,
        )
        .map_err(|e| e.to_string())?;
        let config: Config = serde_json::from_value(
            settings
                .get("config")
                .cloned()
                .ok_or("Missing run config")?,
        )
        .map_err(|e| e.to_string())?;
        let plan: Plan =
            serde_json::from_value(settings.get("plan").cloned().ok_or("Missing run plan")?)
                .map_err(|e| e.to_string())?;
        let mut manifest = bundle_manifest(
            &config,
            &plan,
            &self.run_id,
            state,
            false,
            self.completed,
            error,
        );
        if self.directory.join("accepted_settings.json").is_file() {
            manifest["accepted_settings"]["value"] = serde_json::from_reader(
                File::open(self.directory.join("accepted_settings.json"))
                    .map_err(|e| e.to_string())?,
            )
            .map_err(|e| e.to_string())?;
        }
        manifest["file_hashes"] = json!({"csv_sha256":sha256(&self.directory.join("analysis.csv"))?,
            "raw_csv_sha256":sha256(&self.directory.join("analysis.raw.csv"))?,
            "log_sha256":sha256(&self.directory.join("analysis.log"))?});
        manifest["hash_verification"] =
            json!({"status":"verified","algorithm":"sha256","files":"analysis_csv,raw_csv,log"});
        atomic_json(&self.directory.join("analysis.run.json"), &manifest)
    }
}
fn bundle_manifest(
    c: &Config,
    p: &Plan,
    run_id: &str,
    status: &str,
    in_progress: bool,
    completed: usize,
    error: Option<&str>,
) -> Value {
    json!({"schema_version":1,"application":"kohdalab-ple","application_version":env!("CARGO_PKG_VERSION"),"run_id":run_id,
        "csv_schema":"ple_spectral_bins_v1","raw_schema":"ple_raw_events_v1","log":{"schema":"kohdalab_run_log_v1","timestamp_unit":"unix_s","filename":"analysis.log"},
        "in_progress":in_progress,"final_status":status,"completed_acquisitions":completed,"error":error,
        "requested_settings":{"config":c,"plan":p},"accepted_settings":{"path":"accepted_settings.json","value":null},
        "hardware_observations":"hardware.json","native_artifacts":{"settings":"settings.json","status":"status.json","points":"points.csv","raw":"raw/","processed":"processed/","metadata":"metadata/"},
        "run_bundle":{"schema":"kohdalab_run_bundle_v1","filenames":{"analysis_csv":"analysis.csv","raw_csv":"analysis.raw.csv","log":"analysis.log","conditions_json":"analysis.run.json"}},
        "file_hashes":null,"hash_verification":{"status":"pending","algorithm":"sha256"}})
}
fn write_raw_csv(path: &Path, s: &Spectrum) -> Result<(), String> {
    if path.exists() {
        return Err(format!("Refusing to replace raw CSV {}", path.display()));
    }
    let temp = path.with_extension("csv.tmp");
    let file = OpenOptions::new()
        .write(true)
        .create_new(true)
        .open(&temp)
        .map_err(|e| e.to_string())?;
    let mut w = csv::Writer::from_writer(file);
    w.write_record(["wavelength_nm", "counts"])
        .map_err(|e| e.to_string())?;
    for (x, y) in s.wavelength_nm.iter().zip(&s.counts) {
        w.write_record([x.to_string(), y.to_string()])
            .map_err(|e| e.to_string())?;
    }
    w.flush().map_err(|e| e.to_string())?;
    w.get_ref().sync_all().map_err(|e| e.to_string())?;
    drop(w);
    fs::rename(temp, path).map_err(|e| e.to_string())
}
pub fn load_run(path: &Path) -> Result<(Config, Vec<Sample>), String> {
    let value: Value = serde_json::from_reader(
        File::open(path.join(if path.join("accepted_settings.json").is_file() {
            "accepted_settings.json"
        } else {
            "settings.json"
        }))
        .map_err(|e| e.to_string())?,
    )
    .map_err(|e| e.to_string())?;
    let c: Config =
        serde_json::from_value(value.get("config").cloned().ok_or("Missing run config")?)
            .map_err(|e| e.to_string())?;
    let plan = c.plan()?;
    let mut seen = std::collections::HashSet::new();
    let mut files: Vec<_> = fs::read_dir(path.join("metadata"))
        .map_err(|e| e.to_string())?
        .filter_map(Result::ok)
        .map(|e| e.path())
        .filter(|p| {
            p.file_name().and_then(|s| s.to_str()).is_some_and(|s| {
                (s.starts_with("point_") || s.starts_with("Y_")) && s.ends_with(".json")
            })
        })
        .collect();
    files.sort();
    let mut samples = Vec::new();
    let mut total_bins = 0_usize;
    for p in files {
        let sample: Sample = serde_json::from_reader(File::open(p).map_err(|e| e.to_string())?)
            .map_err(|e| e.to_string())?;
        sample.spectrum.validate()?;
        total_bins = total_bins.saturating_add(sample.spectrum.counts.len());
        if total_bins > 8_000_000 {
            return Err("Run exceeds live-view memory budget".into());
        }
        if sample.corrected_counts.len() != sample.spectrum.counts.len()
            || sample.corrected_counts.iter().any(|x| !x.is_finite())
            || !sample.sample_power_w.is_finite()
            || sample.sample_power_w <= 0.
        {
            return Err("Invalid processed spectrum in saved run".into());
        }
        if sample.point_index >= plan.positions_mm.len()
            || sample.repeat_index >= c.scan.repeats
            || !seen.insert((sample.point_index, sample.repeat_index))
            || !sample.excitation_nm.is_finite()
            || sample.excitation_nm <= 0.
            || !sample.pl_integral_counts_nm.is_finite()
            || !sample.actual_mm.is_finite()
            || !(0. ..=25.).contains(&sample.actual_mm)
            || sample
                .power_sensor_wavelength_nm
                .is_some_and(|v| !v.is_finite() || v <= 0.)
            || sample.pl_peak_nm.is_some_and(|v| !v.is_finite() || v <= 0.)
            || sample
                .corrected_counts
                .iter()
                .any(|v| !(v / sample.sample_power_w).is_finite())
        {
            return Err(
                "Saved acquisition has invalid indices, coordinates or normalization".into(),
            );
        }
        samples.push(sample);
        if samples.len() > crate::MAX_ACQUISITIONS {
            return Err("Run exceeds acquisition limit".into());
        }
    }
    samples.sort_by_key(|s| (s.point_index, s.repeat_index));
    Ok((c, samples))
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn writing_raw_never_truncates_a_saved_file() {
        let dir = tempfile::tempdir().unwrap();
        let path = dir.path().join("Y_18.6mm_40uW.csv");
        let s = Spectrum {
            wavelength_nm: vec![700., 710.],
            counts: vec![1., 2.],
            spe_path: None,
            settings: Value::Null,
        };
        write_raw_csv(&path, &s).unwrap();
        let saved = fs::read(&path).unwrap();
        assert!(write_raw_csv(&path, &s).is_err());
        assert_eq!(fs::read(&path).unwrap(), saved);
    }
    #[test]
    fn raw_data_and_status_survive_stop() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config {
            output_dir: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        let p = c.plan().unwrap();
        let mut r = Recording::create(&c, &p).unwrap();
        let pending: Value =
            serde_json::from_reader(File::open(r.directory.join("analysis.run.json")).unwrap())
                .unwrap();
        assert_eq!(pending["in_progress"], true);
        assert_eq!(pending["hash_verification"]["status"], "pending");
        let b = Spectrum {
            wavelength_nm: vec![750., 800.],
            counts: vec![4., 5.],
            spe_path: None,
            settings: Value::Null,
        };
        r.background(&b).unwrap();
        let s = Sample {
            file_base: String::new(),
            point_index: 0,
            repeat_index: 0,
            target_mm: 18.,
            actual_mm: 18.,
            excitation_nm: 700.,
            excitation_source: "calibration".into(),
            power_w: 1e-6,
            sample_power_w: 1e-6,
            power_timestamp_unix_s: 1.,
            power_sensor_wavelength_nm: Some(800.),
            power_observation: json!({"native_value": -30.0, "native_unit": "dBm", "power_W": 1e-6}),
            acquisition_started_unix_s: 2.,
            timestamp_unix_s: 3.,
            pl_peak_nm: Some(800.),
            pl_integral_counts_nm: 100.,
            spectrum: Spectrum {
                counts: vec![3., 10.],
                ..b.clone()
            },
            corrected_counts: vec![-1., 5.],
        };
        r.sample(&s, &b).unwrap();
        r.finish("stopped", None).unwrap();
        let v: Value =
            serde_json::from_reader(File::open(r.directory.join("status.json")).unwrap()).unwrap();
        assert_eq!(v["completed_acquisitions"], 1);
        assert_eq!(v["state"], "stopped");
        let (_, loaded) = load_run(&r.directory).unwrap();
        assert_eq!(loaded[0].corrected_counts, vec![-1., 5.]);
        let analysis = fs::read_to_string(r.directory.join("analysis.csv")).unwrap();
        assert_eq!(analysis.lines().count(), 3);
        assert!(analysis.contains("corrected_counts_per_W"));
        assert!(analysis.contains("raw/point_00000_repeat_000.csv"));
        assert!(analysis.contains("metadata/point_00000_repeat_000.json"));
        let raw = fs::read_to_string(r.directory.join("analysis.raw.csv")).unwrap();
        assert!(raw.contains("acquisition_saved") && raw.contains("run_finished"));
        let mut reader = csv::Reader::from_reader(raw.as_bytes());
        let saved = reader
            .records()
            .map(Result::unwrap)
            .find(|row| &row[2] == "acquisition_saved")
            .unwrap();
        let details: Value = serde_json::from_str(&saved[8]).unwrap();
        assert_eq!(details["actual_mm"], s.actual_mm);
        assert_eq!(details["meter_power_W"], s.power_w);
        assert_eq!(details["power_observation"], s.power_observation);
        assert_eq!(details["power_sensor_wavelength_nm"], 800.0);
        assert_eq!(details["sample_power_W"], s.sample_power_w);
        assert_eq!(details["power_timestamp_unix_s"], s.power_timestamp_unix_s);
        assert_eq!(details["acquisition_completed_unix_s"], s.timestamp_unix_s);
        assert_eq!(
            details["raw_spectrum_csv"],
            "raw/point_00000_repeat_000.csv"
        );
        assert_eq!(
            details["acquisition_metadata_json"],
            "metadata/point_00000_repeat_000.json"
        );
        let manifest: Value =
            serde_json::from_reader(File::open(r.directory.join("analysis.run.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["run_bundle"]["schema"], "kohdalab_run_bundle_v1");
        assert_eq!(
            manifest["run_bundle"]["filenames"]["analysis_csv"],
            "analysis.csv"
        );
        assert_eq!(manifest["hash_verification"]["status"], "verified");
        assert_eq!(
            manifest["file_hashes"]["csv_sha256"],
            sha256(&r.directory.join("analysis.csv")).unwrap()
        );
        assert!(
            fs::read_to_string(r.directory.join("analysis.log"))
                .unwrap()
                .contains("run_finished")
        );
        let raw = fs::read_to_string(r.directory.join("raw/point_00000_repeat_000.csv")).unwrap();
        assert!(raw.contains("750,3"));
        assert!(!r.directory.join("raw/background.spe").exists());
    }
    #[test]
    fn failed_bundle_records_error_and_hashes() {
        let dir = tempfile::tempdir().unwrap();
        let c = Config {
            output_dir: dir.path().to_string_lossy().into(),
            ..Default::default()
        };
        let p = c.plan().unwrap();
        let mut r = Recording::create(&c, &p).unwrap();
        r.finish("failed", Some("simulated storage error")).unwrap();
        let manifest: Value =
            serde_json::from_reader(File::open(r.directory.join("analysis.run.json")).unwrap())
                .unwrap();
        assert_eq!(manifest["in_progress"], false);
        assert_eq!(manifest["final_status"], "failed");
        assert_eq!(manifest["error"], "simulated storage error");
        assert_eq!(manifest["hash_verification"]["status"], "verified");
        assert_eq!(
            manifest["file_hashes"]["log_sha256"],
            sha256(&r.directory.join("analysis.log")).unwrap()
        );
        let mut raw = csv::Reader::from_path(r.directory.join("analysis.raw.csv")).unwrap();
        let final_row = raw
            .records()
            .filter_map(Result::ok)
            .find(|row| row.get(2) == Some("run_finished"))
            .unwrap();
        assert_eq!(final_row.get(6), Some("failed"));
        assert_eq!(final_row.get(7), Some("simulated storage error"));
        assert!(
            fs::read_to_string(r.directory.join("analysis.log"))
                .unwrap()
                .contains("\tERROR\t")
        );
    }
}
