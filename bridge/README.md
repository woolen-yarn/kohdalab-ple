# PLE Windows hardware bridge

`ple_bridge.py` is a JSON-lines process launched by the Rust PLE desktop app.
It is the only component that imports the Windows-only LightField or Thorlabs
.NET APIs. Its standard output is protocol data only; diagnostics go to standard
error.

## Protocol revision 1

Connection/readback replies include optional automatic hardware inventory. LightField
settings carry `hardware.lightfield` with attached device type/model/serial and
exact grating capability strings. Power replies carry `hardware` with meter and
current sensor identities, calibration text and freshly queried wavelength limits.
Missing optional SDK metadata is explicit (`available`, `null`, `errors`) and is
never replaced with the manually declared apparatus labels. Sensor metadata is
refreshed per power read. No inventory query changes optical settings.

Each request is one line: `{"revision":1,"id":"unique","op":"state","args":{}}`.
Each reply is one line with `id`, `ok`, `result`, and `error`. Calls are executed
serially. `stop` is read on a separate thread so it can request cancellation while
an acquisition is waiting; it immediately replies with `stop_requested`. The
LightField SDK call itself remains on the serial worker thread.

Supported operations are `protocol`, `connect_spectrometer`,
`configure_spectrometer`, `acquire`, `disconnect_spectrometer`, `connect_power`,
`power_read`, `disconnect_power`, `shutter_connect`, `shutter_set`,
`disconnect_shutter`, `state`, `stop`, and `shutdown`.

`connect_spectrometer` and `configure_spectrometer` accept `experiment`,
`visible`, `exposure_ms`, `frames_combined`, `center_nm`, and the exact
LightField `grating` name. An empty `grating` retains the reviewed experiment
template grating and records its readback. `acquire` accepts `path_base`,
`background`, and `timeout_s`. It refuses to overwrite output, keeps the raw
`.spe`, waits for both `.spe` and exported `.csv` to become stable, then moves
the vendor export to `<path_base>.lightfield.csv` before returning one
ROI/combined-frame spectrum. This keeps it distinct from the Rust application's
processed `background.csv` and other derived CSV files. It never retries an
uncertain acquisition. The timeout covers acquisition and export together. When
the installed SDK returns a Task from `ExportAsync`, the bridge waits for that
Task; it also requires stable, exclusively-readable Windows files and a complete
CSV parse before accepting an export.

`path_base` may contain decimal sample coordinates such as
`Y_18.6mm_40uW_p00000_r000`; the bridge appends `.spe`, `.csv`, and
`.lightfield.csv` rather than replacing the final filename component. Only a
terminal `.spe` or `.csv` is rejected.

For automatic background acquisition, `shutter_connect` accepts `serial`,
`closed_state`, `open_state`, and optional `timeout_s`; the two explicitly
provided states must differ. It only reads the MFF state when connecting and
never assumes that state 0 or 1 means open/closed. `shutter_set` accepts
`closed` and optional `timeout_s`. A latched `stop` prevents opening but still
permits closing so the caller can fail closed. `shutdown` attempts to close an
attached automatic shutter before disconnecting it; a close readback failure is
reported as shutdown failure rather than claimed as a safe closed beam.

`connect_power` accepts `resource`, `wavelength_nm`, `average_s`, and
`driver_path`. The last is a user-installed folder containing the existing
`ThorlabsPowerMeter.py` and its `Thorlabs_DotNet_dll/` subfolder with the
official Thorlabs DLL; this repository does not redistribute the DLL.
`power_read` returns `power_w`, or an error—never a fake zero reading.

## Windows setup

Use 64-bit Python on the acquisition PC. Install LightField with its Automation
SDK, define `LIGHTFIELD_ROOT` to its installation directory, and make a reviewed
LightField experiment available under the selected name. Install pythonnet and
`pylablib` for an automatic MFF shutter, plus the dependencies required by the existing Thorlabs helper in the selected
`driver_path`. Start the Rust desktop app and bridge at the same desktop privilege
level as LightField; this bridge never requests administrator elevation.

A stop request remains latched until `configure_spectrometer` begins a reviewed
new run (or a new spectrometer connection is made). This prevents a stop queued
immediately before `acquire` from being lost.

The LightField SDK varies by version. Before production use, qualify: experiment
loading, the exact grating names, one ROI/combined-frame export, `Stop`, SPE/CSV
completion, and cancellation behavior on the intended camera. The bridge keeps
the raw background spectrum; background subtraction is deliberately performed by
the Rust application so every raw acquisition remains reproducible.

## Remote CONEX-CC adapter

`conex_server.py` is a separate, Windows-only process for a CONEX-CC connected
to a lab PC. It listens on `127.0.0.1:9000` by default, accepts exactly one
newline-terminated JSON request per TCP connection, replies with one JSON line,
then closes the connection. Start it on the controller PC, for example:

```powershell
python bridge/conex_server.py --port 9000 --token $env:KOHDA_CONEX_TOKEN
```

Use `--host 0.0.0.0` only deliberately on a protected laboratory network. A
configured token (CLI or `KOHDA_CONEX_TOKEN`) is compared in constant time.
Requests have `{id, revision:1, op, args, token?}` and replies have
`{id, ok, result, error}`. Operations are `hello`, `connect`, `status`, `move`,
and `stop`. `hello` reports protocol `kohdalab-conex`, revision 1,
`can_stop:true`, and `can_home:false`.

`connect` takes `com_port` and optional `velocity_mm_s`, and only opens the
configured COM port then reads limits, position, and state. It never issues
`OR` (home) or `MM_Set` (enable). `move` takes `position_mm`, `tolerance_mm`,
and `timeout_s`; it uses `PA_Set`, confirms both ready state and final position,
and refuses overlapping moves. `stop` calls the Newport `ST(device, "")`
signature and aborts the active waiter. The adapter intentionally does not retry,
home, or recover a controller. Confirm the installed Newport
`CommandInterfaceConexCC` version exposes the documented `SL_Get`, `SR_Get`,
`TP`, `TS`, `PA_Set`, and `ST` signatures before production use.
