# Changelog

## 0.1.1

- Steam のみで起動可能: `umu-run` が無くても Steam Linux Runtime + Proton にフォールバック。
- umu 検出を拡張（Lutris / Heroic / `~/.protoncli/bin` など）。
- prefix は Steam compatdata 形式（`STEAM_COMPAT_DATA_PATH`、wine は `pfx/` 配下）。
- `doctor` が backend / Steam / umu を分けて表示。

## 0.1.0

- App registry, dwproton install, umu launch, Freedesktop shortcuts, doctor.
- Fixed wineprefix root: `~/.protoncli` (`default` → `~/.protoncli/default`, others → `~/.protoncli/prefixes/<id>`).
- `add`/`run` default prefix is `default` (configurable via `default_prefix`).
- No-args / `tui` command: minimal setup TUI (apps, register, prefixes, proton, doctor).
- `prefix list|create|path` subcommands.
- crates.io package name: **`protoncli`** (binary: `proton-cli`).
