# Instrument operation / 機器操作

Use [physical qualification](docs/guides/hardware-qualification.md) and laboratory procedures before acquisition. Verify 0–25 mm travel, optical shutter mapping, SDK version and Stop readback. Manual BG requires real operator beam-block confirmation. / 測定前に実機確認手順と研究室の手順に従い、可動域・遮光状態・SDK・停止読戻しを確認してください。手動BGは実際の遮光操作と確認が必要です。

GUI Stop/API cleanup is not a hardware interlock. Tests/CI never operate real hardware. / GUI StopやAPIの停止確認はハードウェアインターロックではありません。テスト・CIで実機を操作しません。
