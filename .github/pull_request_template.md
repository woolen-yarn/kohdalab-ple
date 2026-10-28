## Summary / 概要

## Verification / 確認

- [ ] `cargo fmt --all -- --check`
- [ ] `cargo clippy --workspace --all-targets --locked -- -D warnings`
- [ ] `cargo test --workspace --locked`
- [ ] `python -m unittest discover -s tests -v`
- [ ] `python scripts/check_docs.py`
- [ ] Packaging checked when changing distribution paths / 配布パス変更時は梱包を確認

## Hardware and data impact / 機器・データへの影響

State changes to commands, coordinates, timing, limits, SDK versions and saved formats. Distinguish simulation and physical checks. / 命令・座標・時刻・制限・SDK・保存形式の変更と、模擬／実機確認の区別を記載してください。
