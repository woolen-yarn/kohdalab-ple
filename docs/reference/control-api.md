# KohdaLab PLE control API

PLE publishes a token-authenticated, loopback-only JSON-lines control endpoint
during ordinary GUI startup. Use `--no-gui-api` to disable it. Discovery does
not acquire control or connect hardware. The descriptor is published atomically in
`~/.kohdalab/ple/run`; it contains a per-instance token and must never be
shared outside the local user account.

The additive `control.*` namespace follows **KohdaLab Control Protocol 1**.
Use `control.describe` to discover operations and safety policy, then
`control.acquire` with a fresh UUID `client_id`. Mutating calls require the
returned opaque `lease`; renew it with `control.heartbeat`.

PLE retains its command-acknowledgement/run-job boundary: an acknowledged
command is not an acquisition result. Poll `control.result` until the linked
job reaches a terminal state. Existing shutter, cleanup-barrier, and explicit
confirmation policies are unchanged.
