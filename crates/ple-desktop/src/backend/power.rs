use super::RpcClient;
use crate::engine::Event;
use ple_core::{Config, Mode};
use serde_json::{Value, json};
use std::{
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};
type PowerReading = (f64, f64, f64, Value, Value);
fn native_power_observation(
    reply: &Value,
    converted_w: f64,
    wavelength_nm: f64,
) -> Result<Value, String> {
    let native_value = reply
        .get("native_value")
        .and_then(Value::as_f64)
        .filter(|value| value.is_finite())
        .ok_or("Power read did not contain a finite native_value")?;
    let native_unit = reply
        .get("native_unit")
        .and_then(Value::as_str)
        .ok_or("Power read did not contain native_unit")?;
    let expected_w = match native_unit {
        "W" => native_value,
        "dBm" => 1e-3 * 10_f64.powf(native_value / 10.),
        _ => return Err(format!("Unsupported native power unit {native_unit:?}")),
    };
    if !expected_w.is_finite()
        || !converted_w.is_finite()
        || (expected_w != converted_w
            && (expected_w - converted_w).abs() > expected_w.abs() * 1e-12)
    {
        return Err("Power read converted W disagrees with native observation".into());
    }
    Ok(
        json!({"native_value":native_value,"native_unit":native_unit,"converted_w":converted_w,"wavelength_nm":wavelength_nm,"hardware":reply.get("hardware")}),
    )
}
pub enum PowerCommand {
    Connect(Box<Config>, mpsc::Sender<Result<(), String>>),
    Disconnect,
    Read(mpsc::Sender<Result<PowerReading, String>>),
    Inventory(mpsc::Sender<Result<Value, String>>),
    ReadAt(f64, mpsc::Sender<Result<PowerReading, String>>),
    Shutdown(mpsc::Sender<()>),
}
#[derive(Clone)]
pub struct PowerClient {
    tx: mpsc::Sender<PowerCommand>,
    connected: Arc<AtomicBool>,
}
impl PowerClient {
    pub fn spawn(events: mpsc::Sender<Event>) -> Self {
        let (tx, rx) = mpsc::channel();
        let connected = Arc::new(AtomicBool::new(false));
        let worker_connected = connected.clone();
        thread::spawn(move || {
            let mut power: Option<Power> = None;
            loop {
                match rx.recv_timeout(Duration::from_millis(500)) {
                    Ok(PowerCommand::Connect(c, reply)) => {
                        let result = Power::connect(&c, events.clone());
                        match result {
                            Ok(p) => {
                                let _ = events.send(Event::PowerWavelength(p.wavelength_nm));
                                power = Some(p);
                                worker_connected.store(true, Ordering::Release);
                                let _ = reply.send(Ok(()));
                            }
                            Err(e) => {
                                power = None;
                                worker_connected.store(false, Ordering::Release);
                                let _ = reply.send(Err(e));
                            }
                        }
                    }
                    Ok(PowerCommand::Disconnect) => {
                        power = None;
                        worker_connected.store(false, Ordering::Release);
                    }
                    Ok(PowerCommand::Inventory(reply)) => {
                        let _ = reply.send(
                            power
                                .as_ref()
                                .map(|p| p.hardware.clone())
                                .ok_or("Power meter not connected".into()),
                        );
                    }
                    Ok(PowerCommand::ReadAt(wavelength, reply)) => {
                        let result = power
                            .as_mut()
                            .ok_or("Power meter not connected".into())
                            .and_then(|p| {
                                p.read_at(wavelength).map(|(w, t, nm, observation)| {
                                    (w, t, nm, p.hardware.clone(), observation)
                                })
                            });
                        match &result {
                            Ok((value, timestamp, actual, _, _)) => {
                                let _ = events.send(Event::PowerWavelength(*actual));
                                let _ = events.send(Event::Telemetry {
                                    device: "power",
                                    value: *value,
                                    timestamp: *timestamp,
                                });
                            }
                            Err(_) => {
                                power = None;
                                worker_connected.store(false, Ordering::Release);
                                let _ = events.send(Event::Device {
                                    device: "power",
                                    connected: false,
                                });
                            }
                        }
                        let _ = reply.send(result);
                    }
                    Ok(PowerCommand::Read(reply)) => {
                        let result = power
                            .as_mut()
                            .ok_or("Power meter not connected".into())
                            .and_then(|p| {
                                p.read().map(|(w, t, observation)| {
                                    (w, t, p.wavelength_nm, p.hardware.clone(), observation)
                                })
                            });
                        if let Ok((value, timestamp, actual, _, _)) = &result {
                            let _ = events.send(Event::PowerWavelength(*actual));
                            let _ = events.send(Event::Telemetry {
                                device: "power",
                                value: *value,
                                timestamp: *timestamp,
                            });
                        } else {
                            worker_connected.store(false, Ordering::Release);
                            power = None;
                            let _ = events.send(Event::Device {
                                device: "power",
                                connected: false,
                            });
                        }
                        let _ = reply.send(result);
                    }
                    Ok(PowerCommand::Shutdown(done)) => {
                        drop(power.take());
                        let _ = done.send(());
                        break;
                    }
                    Err(mpsc::RecvTimeoutError::Disconnected) => break,
                    Err(mpsc::RecvTimeoutError::Timeout) => {
                        if let Some(p) = &mut power {
                            match p.read() {
                                Ok((w, timestamp, _)) => {
                                    let _ = events.send(Event::Telemetry {
                                        device: "power",
                                        value: w,
                                        timestamp,
                                    });
                                }
                                Err(e) => {
                                    let _ =
                                        events.send(Event::Log(format!("Power telemetry: {e}")));
                                    let _ = events.send(Event::Device {
                                        device: "power",
                                        connected: false,
                                    });
                                    power = None;
                                    worker_connected.store(false, Ordering::Release);
                                }
                            }
                        }
                    }
                }
            }
        });
        Self { tx, connected }
    }
    pub fn is_connected(&self) -> bool {
        self.connected.load(Ordering::Acquire)
    }
    pub fn connect(&self, c: Config) -> Result<(), String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Connect(Box::new(c), tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn read_at(&self, wavelength: f64) -> Result<PowerReading, String> {
        if !wavelength.is_finite() || wavelength <= 0. {
            return Err("Invalid power sensor wavelength".into());
        }
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::ReadAt(wavelength, tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn hardware(&self) -> Result<Value, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Inventory(tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn read_with_wavelength(&self) -> Result<PowerReading, String> {
        let (tx, rx) = mpsc::channel();
        self.tx
            .send(PowerCommand::Read(tx))
            .map_err(|e| e.to_string())?;
        rx.recv().map_err(|e| e.to_string())?
    }
    pub fn disconnect(&self) {
        self.connected.store(false, Ordering::Release);
        let _ = self.tx.send(PowerCommand::Disconnect);
    }
    pub fn shutdown(&self) {
        self.connected.store(false, Ordering::Release);
        let (tx, rx) = mpsc::channel();
        if self.tx.send(PowerCommand::Shutdown(tx)).is_ok() {
            let _ = rx.recv_timeout(Duration::from_secs(21));
        }
    }
}
struct Power {
    hardware: Value,
    events: mpsc::Sender<Event>,
    wavelength_tolerance_nm: f64,
    wavelength_nm: f64,
    rpc: Option<RpcClient>,
    start: Instant,
}
impl Drop for Power {
    fn drop(&mut self) {
        if let Some(rpc) = &self.rpc {
            let _ = rpc.request("shutdown", json!({}), Duration::from_secs(5));
        }
    }
}
impl Power {
    fn connect(c: &Config, events: mpsc::Sender<Event>) -> Result<Self, String> {
        let mut hardware = json!({"demo":true,"source":"simulated"});
        let mut actual_wavelength = c.power.wavelength_nm;
        let rpc = if c.mode == Mode::Hardware {
            let rpc = RpcClient::start(c, events.clone())?;
            let state = rpc.request(
                "connect_power",
                serde_json::to_value(&c.power).map_err(|e| e.to_string())?,
                Duration::from_secs(30),
            )?;
            hardware = state.get("hardware").cloned().unwrap_or_else(
                || json!({"available":false,"errors":["Bridge did not provide power inventory"]}),
            );
            actual_wavelength = state["wavelength_nm"]
                .as_f64()
                .ok_or("Missing power wavelength readback")?;
            if !actual_wavelength.is_finite()
                || (actual_wavelength - c.power.wavelength_nm).abs()
                    > c.power.wavelength_tolerance_nm
            {
                return Err("Power sensor wavelength was not accepted".into());
            }
            Some(rpc)
        } else {
            None
        };
        let _ = events.send(Event::PowerHardware(hardware.clone()));
        Ok(Self {
            hardware,
            events,
            wavelength_nm: actual_wavelength,
            wavelength_tolerance_nm: c.power.wavelength_tolerance_nm,
            rpc,
            start: Instant::now(),
        })
    }
    fn read_at(&mut self, wavelength: f64) -> Result<(f64, f64, f64, Value), String> {
        let actual = if let Some(rpc) = &self.rpc {
            let r = rpc.request(
                "power_set_wavelength",
                json!({"wavelength_nm":wavelength,"wavelength_tolerance_nm":self.wavelength_tolerance_nm}),
                Duration::from_secs(10),
            )?;
            r["wavelength_nm"]
                .as_f64()
                .ok_or("Missing power wavelength readback")?
        } else {
            wavelength
        };
        if !actual.is_finite() || (actual - wavelength).abs() > self.wavelength_tolerance_nm {
            return Err("Power sensor rejected the requested wavelength".into());
        }
        self.wavelength_nm = actual;
        let (value, time, observation) = self.read()?;
        Ok((value, time, actual, observation))
    }
    fn read(&mut self) -> Result<(f64, f64, Value), String> {
        let (value, observation) = if let Some(rpc) = &self.rpc {
            let r = rpc.request("power_read", json!({}), Duration::from_secs(15))?;
            if let Some(hardware) = r.get("hardware")
                && *hardware != self.hardware
            {
                self.hardware = hardware.clone();
                let _ = self
                    .events
                    .send(Event::PowerHardware(self.hardware.clone()));
            }
            let readback = r["wavelength_nm"]
                .as_f64()
                .ok_or("Missing actual power wavelength")?;
            if !readback.is_finite() || (readback - self.wavelength_nm).abs() > 1e-6 {
                return Err("Power sensor wavelength changed outside the application".into());
            }
            let value = r["power_w"]
                .as_f64()
                .ok_or("Power read did not contain power_w")?;
            (value, native_power_observation(&r, value, readback)?)
        } else {
            let value = 20e-6 * (1. + 0.02 * (self.start.elapsed().as_secs_f64() * 0.4).sin());
            (
                value,
                json!({"native_value":value,"native_unit":"W","converted_w":value,"wavelength_nm":self.wavelength_nm,"source":"simulated"}),
            )
        };
        if !value.is_finite() {
            return Err("Non-finite power reading".into());
        }
        Ok((value, ple_core::recording::unix_s(), observation))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn native_power_provenance_requires_finite_known_unit_and_consistent_conversion() {
        let valid = json!({"native_value": -30., "native_unit":"dBm", "hardware":{}});
        assert!(native_power_observation(&valid, 1e-6, 700.).is_ok());
        for invalid in [
            json!({"native_unit":"W"}),
            json!({"native_value":"1e-6","native_unit":"W"}),
            json!({"native_value":1e-6,"native_unit":"mW"}),
        ] {
            assert!(native_power_observation(&invalid, 1e-6, 700.).is_err());
        }
        assert!(
            native_power_observation(&json!({"native_value":1e-6,"native_unit":"W"}), 2e-6, 700.)
                .is_err()
        );
    }
    #[test]
    fn explicit_power_reads_update_live_trace_and_connection_state() {
        let (tx, rx) = mpsc::channel();
        let power = PowerClient::spawn(tx);
        power.connect(Config::default()).unwrap();
        assert!(power.is_connected());
        let (w, _, _, _, _) = power.read_with_wavelength().unwrap();
        assert!(w > 0.);
        let inventory = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(inventory, Event::PowerHardware(v) if v["source"] == "simulated"));
        let wavelength = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            matches!(wavelength, Event::PowerWavelength(nm) if nm == Config::default().power.wavelength_nm)
        );
        let read_wavelength = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(
            matches!(read_wavelength, Event::PowerWavelength(nm) if nm == Config::default().power.wavelength_nm)
        );
        let event = rx.recv_timeout(Duration::from_secs(1)).unwrap();
        assert!(matches!(
            event,
            Event::Telemetry {
                device: "power",
                ..
            }
        ));
        power.disconnect();
        assert!(!power.is_connected());
        power.shutdown();
    }
}
