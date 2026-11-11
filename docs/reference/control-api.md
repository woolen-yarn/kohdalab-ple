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

PLE retains its command-acknowledgement/run-job boundary: an acknowledged
command is not an acquisition result. Poll `control.result` until the linked
job reaches a terminal state. Existing shutter, cleanup-barrier, and explicit
confirmation policies are unchanged.
