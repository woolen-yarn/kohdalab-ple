# KohdaLab PLE control API

PLE publishes a token-authenticated, loopback-only JSON-lines control endpoint
during ordinary GUI startup. Use `--no-gui-api` to disable it. Discovery does
not acquire control or connect hardware. The descriptor is published atomically in
`~/.kohdalab/ple/run`; it contains a per-instance token and must never be
shared outside the local user account. On Windows the default descriptor relies
on the user profile's access permissions. If you use `--api-dir`, choose a
directory accessible only to the same user.

The additive `control.*` namespace follows **KohdaLab Control Protocol 1**.
Use `control.describe` to discover operations and safety policy, then
`control.acquire` with a fresh UUID `client_id`. Mutating calls require the
returned opaque `lease`, renewed with `control.heartbeat`. A valid endpoint
token suffices for `control.stop` without a lease.

`control.stop` without `job_id` (or with `job_id: null`) stops all current
activity and requests optical cleanup even when no job is active. When a
`job_id` is supplied it is validated before anything is stopped: it must be a
UUID naming the active job, or a retained command acknowledgement whose
`run_job` is the active job. A malformed ID returns `validation`; an ID that
was never retained returns `unknown_job`; a retained ID that is no longer
active returns `invalid_state`. In those cases nothing is stopped. The native
`stop` action is unchanged and ignores identifiers.

While Python holds control, or Stop cleanup is outstanding, the GUI shows a
nonmodal **External control** strip across the top of the window. It names the
client type (Python), says what is actually happening (measuring, moving the
stage, waiting for a confirmation, stopping), and offers **Stop** followed by
**Take control**. Take control also requests Stop and revokes the lease.

PLE retains its command-acknowledgement/run-job boundary: an acknowledged
command is not an acquisition result. Poll `control.result` until the linked
job reaches a terminal state. Existing shutter, cleanup-barrier, and explicit
confirmation policies are unchanged.
