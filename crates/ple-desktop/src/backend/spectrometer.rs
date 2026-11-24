use super::{RpcClient, timeout};
use crate::{
    engine::{CleanupCause, Event},
    shutter_client::ExternalShutter,
};
use ple_core::{Config, Mode, Spectrum};
use serde_json::{Value, json};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Mutex, atomic::AtomicBool, mpsc},
    thread,
    time::Duration,
};
pub struct Spectrometer {
    rpc: Option<RpcClient>,
    mode: Mode,
    config: Config,
    acquisitions: usize,
    actual_settings: Value,
    shutter_attached: bool,
    shutter_readback: Mutex<Value>,
    external_shutter: Arc<Mutex<Option<Arc<ExternalShutter>>>>,
    cancel: Arc<AtomicBool>,
}
impl Drop for Spectrometer {
    fn drop(&mut self) {
        if let Some(shutter) = self.external_shutter.lock().unwrap().as_ref() {
            shutter.request_close();
        }
        if let Some(rpc) = &self.rpc {
            let _ = rpc.request("shutdown", json!({}), Duration::from_secs(5));
        }
    }
}
impl Spectrometer {
    pub fn connect(
        c: &Config,
        events: mpsc::Sender<Event>,
        stop_handle: &Mutex<Option<RpcClient>>,
        external_shutter: Arc<Mutex<Option<Arc<ExternalShutter>>>>,
        cancel: Arc<AtomicBool>,
    ) -> Result<Self, String> {
        let mut actual_settings = json!({"demo":true,"exposure_ms":c.spectrometer.exposure_ms,"frames_combined":c.spectrometer.frames_combined,"center_nm":c.spectrometer.center_nm,"grating":c.spectrometer.grating});
        let rpc = if c.mode == Mode::Hardware {
            let client = RpcClient::start(c, events)?;
            // Retain ownership even if the connect acknowledgement fails.
            *stop_handle.lock().unwrap() = Some(client.clone());
            let state = client.request(
                "connect_spectrometer",
                serde_json::to_value(&c.spectrometer).map_err(|e| e.to_string())?,
                Duration::from_secs(90),
            )?;
            actual_settings = state["settings"].clone();
            Some(client)
        } else {
            None
        };
        Ok(Self {
            rpc,
            mode: c.mode,
            config: c.clone(),
            acquisitions: 0,
            actual_settings,
            shutter_attached: false,
            shutter_readback: Mutex::new(Value::Null),
            external_shutter,
            cancel,
        })
    }
    pub fn disconnect(
        &mut self,
        events: &mpsc::Sender<Event>,
    ) -> Result<(), (CleanupCause, String)> {
        let external = self.external_shutter.lock().unwrap().clone();
        if let Some(shutter) = external {
            shutter.shutdown().map_err(|e| (CleanupCause::Shutter, e))?;
            *self.external_shutter.lock().unwrap() = None;
            self.shutter_attached = false;
            let _ = events.send(Event::Cleanup {
                cause: CleanupCause::Shutter,
                result: Ok(()),
            });
        }
        if self.shutter_attached {
            self.set_shutter(true)
                .map_err(|e| (CleanupCause::Shutter, e))?;
            let _ = events.send(Event::Cleanup {
                cause: CleanupCause::Shutter,
                result: Ok(()),
            });
        }
        if let Some(rpc) = &self.rpc {
            let result = rpc
                .request("shutdown", json!({}), Duration::from_secs(25))
                .map_err(|e| (CleanupCause::SpectrometerStop, e))?;
            if result["shutdown"].as_bool() != Some(true) {
                return Err((
                    CleanupCause::SpectrometerStop,
                    "Bridge shutdown was not confirmed".into(),
                ));
            }
        }
        self.shutter_attached = false;
        self.rpc = None;
        Ok(())
    }
    pub fn cancel_handle(&self) -> Option<RpcClient> {
        self.rpc.clone()
    }
    pub fn configure(&mut self, c: &Config) -> Result<(), String> {
        if self.mode != c.mode {
            return Err("Reconnect all devices after changing mode".into());
        }
        if let Some(rpc) = &self.rpc {
            self.actual_settings = rpc.request(
                "configure_spectrometer",
                serde_json::to_value(&c.spectrometer).map_err(|e| e.to_string())?,
                Duration::from_secs(30),
            )?;
        }
        if self.rpc.is_none() {
            self.actual_settings = json!({"demo":true,"exposure_ms":c.spectrometer.exposure_ms,"frames_combined":c.spectrometer.frames_combined,"center_nm":c.spectrometer.center_nm,"grating":c.spectrometer.grating});
        }
        self.config = c.clone();
        Ok(())
    }
    pub fn settings(&self) -> Value {
        self.actual_settings.clone()
    }
    pub fn prepare_shutter(&mut self, c: &Config) -> Result<(), String> {
        let previous = self.external_shutter.lock().unwrap().clone();
        if let Some(shutter) = previous {
            shutter.shutdown()?;
            *self.external_shutter.lock().unwrap() = None;
            self.shutter_attached = false;
        }
        if self.shutter_attached {
            self.set_shutter(true)?;
            if let Some(rpc) = &self.rpc {
                rpc.request("disconnect_shutter", json!({}), Duration::from_secs(10))?;
            }
            self.shutter_attached = false;
        }
        if c.background.mode == ple_core::BackgroundMode::Automatic {
            if c.mode == Mode::Hardware
                && c.background.backend == ple_core::ShutterBackend::KohdalabShutter
            {
                let endpoint = (!c.background.endpoint.trim().is_empty())
                    .then(|| PathBuf::from(&c.background.endpoint));
                let shutter = match ExternalShutter::connect(
                    endpoint,
                    c.background.channel,
                    c.background.timeout_s,
                    self.cancel.clone(),
                ) {
                    Ok(shutter) => shutter,
                    Err(error) => {
                        if let Some(owner) = error.unconfirmed_owner {
                            *self.external_shutter.lock().unwrap() = Some(owner);
                            self.shutter_attached = true;
                        }
                        return Err(error.message);
                    }
                };
                if let Some(rpc) = self.rpc.clone() {
                    shutter.set_cancel_hook(Arc::new(move || rpc.stop()));
                }
                *self.external_shutter.lock().unwrap() = Some(shutter);
            } else if let Some(rpc) = &self.rpc {
                rpc.request("shutter_connect",json!({"serial":c.background.shutter_serial,"closed_state":c.background.closed_state,"open_state":c.background.open_state,"timeout_s":c.background.timeout_s}),timeout(c.background.timeout_s + 5.))?;
            }
            self.shutter_attached = true;
        }
        Ok(())
    }
    pub fn shutter_attached(&self) -> bool {
        self.shutter_attached
    }
    pub fn finish_shutter(&mut self) -> Result<(), String> {
        let external = self.external_shutter.lock().unwrap().clone();
        if let Some(shutter) = external {
            shutter.shutdown()?;
            let mut snapshot = shutter.snapshot();
            snapshot["control_released"] = json!(true);
            *self.shutter_readback.lock().unwrap() = snapshot;
            *self.external_shutter.lock().unwrap() = None;
            self.shutter_attached = false;
        }
        Ok(())
    }
    pub fn shutter_snapshot(&self) -> Value {
        if let Some(shutter) = self.external_shutter.lock().unwrap().as_ref() {
            return shutter.snapshot();
        }
        if self.mode == Mode::Hardware
            && self.config.background.backend == ple_core::ShutterBackend::KohdalabShutter
        {
            let cached = self.shutter_readback.lock().unwrap().clone();
            if !cached.is_null() {
                return cached;
            }
        }
        json!({"connected":self.shutter_attached,"backend":self.config.background.backend,"mode":self.mode,"state":self.shutter_readback.lock().unwrap().clone(),"feedback":if self.mode==Mode::Demo {"simulated"} else {"device_state_not_optical_sensor"}})
    }
    pub fn set_shutter(&self, closed: bool) -> Result<(), String> {
        if !self.shutter_attached {
            return Err("Automatic shutter is not connected".into());
        }
        let external = self.external_shutter.lock().unwrap().clone();
        if let Some(shutter) = external {
            *self.shutter_readback.lock().unwrap() = shutter.set_closed(closed)?;
            return Ok(());
        }
        if let Some(rpc) = &self.rpc {
            let r = rpc.request(
                "shutter_set",
                json!({"closed":closed,"timeout_s":self.config.background.timeout_s}),
                timeout(self.config.background.timeout_s + 5.),
            )?;
            if r["closed"].as_bool() != Some(closed) {
                return Err("Shutter readback did not confirm requested optical state".into());
            }
            *self.shutter_readback.lock().unwrap() = r;
        } else {
            *self.shutter_readback.lock().unwrap() = json!({"closed":closed,"simulated":true});
        }
        Ok(())
    }
    pub fn acquire(
        &mut self,
        path: &str,
        background: bool,
        position: f64,
    ) -> Result<Spectrum, String> {
        if let Some(rpc) = &self.rpc {
            let v=rpc.request("acquire",json!({"path_base":path,"background":background,"timeout_s":self.config.spectrometer.timeout_s}),timeout(self.config.spectrometer.timeout_s+10.))?;
            let spectrum: Spectrum =
                serde_json::from_value(v).map_err(|e| format!("Invalid spectrum response: {e}"))?;
            spectrum.validate()?;
            let path = spectrum
                .spe_path
                .as_ref()
                .ok_or("LightField did not report a raw SPE file")?;
            if !Path::new(path).is_file() {
                return Err("Raw SPE file is not available locally".into());
            }
            Ok(spectrum)
        } else {
            thread::sleep(Duration::from_millis(80));
            self.acquisitions += 1;
            let wavelength_nm: Vec<_> = (0..641).map(|i| 640. + i as f64 * 0.375).collect();
            let exc = 700. + (position - 18.) * 50.;
            let response = 0.5 + 1.4 * (-((exc - 707.) / 5.).powi(2)).exp();
            let counts = wavelength_nm
                .iter()
                .map(|w| {
                    let bg = 70. + 0.04 * (w - 640.);
                    if background {
                        bg
                    } else {
                        let pl = 900.
                            * response
                            * (-0.5 * ((w - (780. + (exc - 700.) * 0.1)) / 7.).powi(2)).exp();
                        let laser = 1800. * (-0.5 * ((w - exc) / 0.8).powi(2)).exp();
                        let noise = 2. * ((w * 13. + self.acquisitions as f64).sin());
                        bg + pl + laser + noise
                    }
                })
                .collect();
            Ok(Spectrum {
                wavelength_nm,
                counts,
                spe_path: None,
                settings: self.actual_settings.clone(),
            })
        }
    }
}
