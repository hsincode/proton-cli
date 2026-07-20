# Changelog

## 0.1.0

- App registry, dwproton install, umu launch, Freedesktop shortcuts, doctor.
- Fixed wineprefix root: `~/.protoncli` (`default` → `~/.protoncli/default`, others → `~/.protoncli/prefixes/<id>`).
- `add`/`run` default prefix is `default` (configurable via `default_prefix`).
- No-args / `tui` command: minimal setup TUI (apps, register, prefixes, proton, doctor).
- `prefix list|create|path` subcommands.
