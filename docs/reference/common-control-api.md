# KohdaLab Control Protocol 1 / 共通GUI API

This additive API standardizes local GUI discovery, authentication, ownership,
status, job receipts and errors. Scientific settings and operation arguments
remain application-specific; ask `describe()` before invoking an operation.
Existing native clients and their safety defaults remain available.

## Python / Notebook

Install the separate dependency-free SDK from `bindings/control` in the source
or the common SDK directory in the package. This is the same distribution in all
five applications, not five overlapping Python modules.

```sh
python -m pip install ./bindings/control
```

```python
from kohdalab_control import ControlClient, discover

instances = discover("kohdalab-iv")
app = ControlClient("kohdalab-iv")  # exactly one responsive instance required
print(app.describe())              # no ownership or instrument connection
print(app.status())
# Review the GUI profile before acquiring or connecting devices.
with app:
    job = app.invoke("connect")
    app.wait(job["job_id"])
    job = app.invoke("start")
    finished = app.wait(job["job_id"])
```

For multiple instances choose an explicit private descriptor with `endpoint=`.
Camera currently offers read-only `describe/status/config`; inspect it without a
context manager because context entry requests ownership. Camera capture/start/
stop/ownership are not exposed by this adapter. Do not substitute internal camera
helper IPC for a public GUI API.

## Contract / 通信仕様

| Area | Common behavior |
| --- | --- |
| Discovery | One atomic per-instance descriptor in `~/.kohdalab/<app>/run` (Windows `%USERPROFILE%\.kohdalab\<app>\run`); app, instance UUID, PID, loopback host/port, private token, protocol version |
| Wire | Local TCP JSON Lines, `control_protocol: 1`, `method: control.<name>` |
| Success | `ok: true`, `request_id`, `result`, `error: null` |
| Error | `ok: false`, `request_id`, `error: {code, application_code, message, retryable}` |
| Ownership | `client_id` UUID plus opaque `lease`; heartbeat interval and timeout come from `describe()`/`acquire()` |
| Status | `phase`, actual admission `busy`, `ownership`, `cleanup`, app snapshot under `native` |
| Job receipt | `job_id`, `kind`, `status`, `terminal`, `cleanup`, optional `linked_job_id`, original receipt under `native` |
| Completion | Only `completed/stopped/error` are terminal; a command ACK can link to a separate acquisition job |
| Cleanup | `verified/pending/not_required/unknown`; ACK or output command alone is not verified cleanup |

Read endpoints are token-authenticated without acquiring ownership. Mutations
use fresh UUID request IDs and the app's existing admission/duplicate tracking.
A transport timeout means the outcome may be unknown: inspect status and the
specific receipt before retrying. The SDK never replays mutations automatically,
chooses a process by recency, or opens a second instrument connection.

`wait(job_id)` follows explicit ACK-to-job links, never the latest job.
`stop_and_wait()` requires actual cleanup confirmation. `release()` waits for
confirmed release and cleanup; a timeout is reported rather than hidden. Finish
or explicitly stop work before releasing when required by the app policy.

## Application boundaries / アプリ別の動作

| App | Public scope | Safety policy retained |
| --- | --- | --- |
| IV | Measurement, configuration, source/gate operations through GUI | Existing protections and fresh output-OFF confirmation on release/takeover |
| TRKR | Measurement, configuration and supported motion through GUI | Existing continuation options and motion/stop admission rules |
| PLE | Measurement/configuration through GUI; explicit confirmations retained | Optical cleanup barrier and command-ACK/run-job distinction |
| Shutter | Supported channel operations through GUI | Hold last state; commanded position is not measured optical transmission |
| Camera | Read-only GUI status/configuration | Acquisition helper remains private; no new device commands |

The common contract does not make device stop policies identical. Read the
advertised methods and safety fields from the running release. Native API
versions, app versions and common protocol version are separate.

GUI経由の共通窓口を追加しました。既存API・測定・CSV・停止方針は維持します。
対応操作は `describe()` で確認し、複数起動時は対象を明示してください。
通信失敗時に操作を自動再送せず、受付応答を停止・測定の完了とは判断しません。
Cameraは読み取り専用、Shutterは状態保持など、アプリごとの違いを維持します。
