# Common control API validation

Date: 2026-10-29. Target: 0.3.5. Host: macOS ARM64.

- Device-free Rust tests: 108 passed across the selected application/workspace and common contract suites.
- Shared Python SDK: 9 tests passed, including authenticated loopback errors, exact job links, ambiguous timeout boundaries and pending cleanup.
- Actual Demo GUI: native full measurement/stop/lease acceptance and common read/ownership/connect/release passed. Common cleanup failure, active-release lifecycle and preservation of native Stop ownership have focused regression coverage. The final native Demo completed six rows with 641 spectral bins.
- Physical instruments were not opened or qualified for this change. Windows packaging/CI evidence belongs to the matching PR/release; these local tests are not a Windows hardware pass.

The existing native API, scientific storage and product-specific safety policy remain separate from the additive common namespace. See [the common contract](../reference/common-control-api.md).
