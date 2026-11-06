# Upstream UI provenance

The KohdaLab PLE desktop UI deliberately shares the design and interaction
contracts of these MIT-licensed applications:

- KohdaLab TRKR, woolen-yarn/kohdalab-trkr, commit
  `066f89686efc1155156b0d079fcb1766d897d0f4`.
  `crates/ple-desktop/src/precision.rs` adapts its measured form geometry,
  control rendering, column dimensions, font setup and palette.
  `file_dialog.rs` uses its asynchronous native file chooser.
  `remote_gui.rs` adapts its bounded, opt-in loopback GUI transport; PLE ownership, receipts and workflow dispatch are implemented separately. The original signed palette asset remains as reference; the PLE heatmap uses Inferno.
  `live_monitor.rs` adapts its `live.rs` and `live_view.rs` scalar monitor.
- KohdaLab IV, woolen-yarn/kohdalab-iv, commit
  `2908b47d54a0dc35927f508e51345bb568ec83df`.
  The shared palette, engineering-axis formatting, compact fields and
  log/snapshot presentation follow `ui/widgets.rs` and `ui/formatting.rs`.

The upstream copyright and MIT notices are preserved in the adjacent
`kohdalab-trkr/LICENSE` and `kohdalab-iv/LICENSE` files. PLE measurement
planning, spectral reduction and instrument execution remain product-specific.

The sequential PLE heatmap palette is the 256-entry Inferno lookup table from
Matplotlib's official `lib/matplotlib/_cm_listed.py` (`_inferno_data`), rounded
to 8-bit RGB in `crates/ple-desktop/src/presentation.rs`. Only the LUT is adapted, by conversion to 8-bit RGB; no Matplotlib runtime is bundled. Copyright (c) 2012- Matplotlib Development Team; All Rights Reserved. Its license is retained in `matplotlib/LICENSE`. Source: https://github.com/matplotlib/matplotlib/blob/v3.10.7/lib/matplotlib/_cm_listed.py.

## Locked Rust dependencies

Binary distributions include `rust/NOTICE.json` and preserved dependency license texts.
For crates whose published archives omit texts, `rust-upstream/provenance.json`
records the fixed upstream source and SHA-256, or the upstream SPDX assignment
and standard SPDX text. Original metadata is retained for the latter.
This inventory does not include vendor SDK DLLs or their redistribution rights.

The documentation link checker (`scripts/check_docs.py`) is adapted from the
MIT-licensed KohdaLab TRKR repository at commit `066f8968`. The upstream MIT
notice remains in `kohdalab-trkr/LICENSE`.

## kohdalab-shutter Windows companion

GitHub Windows releases may include the separately built GUI/CLI of MIT-licensed [kohdalab-shutter](https://github.com/woolen-yarn/kohdalab-shutter) at commit `7e8d1c953905ad92a49a07cd75b916c38a3a518d` (package 0.6.6, GUI API protocol 2). Its upstream LICENSE, matching source archive and dependency license texts are retained under `companions/kohdalab-shutter`. The PLE executable communicates with this program; it does not incorporate its Rust implementation or flash firmware.

Windows配布版の同梱Shutterは上記commitの0.6.6（GUI API protocol 2）です。対応するソース・MITライセンス・依存ライブラリのライセンス文書を`companions/kohdalab-shutter`に保持します。PLEとは別プログラムとして通信し、firmwareの書込みは行いません。

## Windows Setup

The NSIS installer follows TRKR's Setup distribution and IV's suite-standard application directory, staged replacement and data-preservation approach. The NSIS template was adapted from KohdaLab Shutter (MIT, Kohdalab contributors). The compiler is pinned to NSIS 3.12 using the archive hash recorded by IV at commit `5572ee8a1c08c54dc3def5460b2a719cb4e76e52`; the unchanged archive `COPYING` and provenance are retained in `nsis-3.12/`. Setup uses zlib compression. Compiler/build tools are not bundled.
