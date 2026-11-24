//! Device boundaries. No GUI thread performs instrument I/O.
mod power;
mod rpc;
mod spectrometer;
mod stage;

pub use power::PowerClient;
pub use rpc::RpcClient;
pub use spectrometer::Spectrometer;
pub use stage::StageClient;

use std::time::Duration;

fn timeout(s: f64) -> Duration {
    Duration::from_secs_f64(s.clamp(0.1, 86400.))
}

#[derive(Default)]
struct StopBarrier {
    pending: bool,
    unconfirmed: Option<String>,
}
