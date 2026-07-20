# Changelog

## Unreleased

## 0.1.4

- 既定起動 env: `DXVK_HUD=0` / `STEAMOS=1` / `UMU_USE_STEAM=1`（`[launch.env]`、未記載時はビルトイン）。
- TUI **Environment variables**（Home）: グローバル env の一覧・追加・編集・削除・ビルトインへリセット。
- アプリ別 env: Default / Custom 切替。Custom はグローバル既定に **追加**（同キーは上書き）。Apps → `o`/`e` → Env。
- Apps 一覧で設定キー **`e`** を `o` と同じく受付。
- `protoncli add --exe path` のみで登録可（`--name` 省略時は exe の stem）。表示名は Apps → settings → Name で後から変更可。
- TUI ナビを全画面で統一: **j/k**（一覧）、**Ctrl+n / Ctrl+p**（次/前、フォームでも可）、**Ctrl+j / Ctrl+k**、↑↓、Tab（フォーム）。

## 0.1.3

- 実行ファイル / コマンド名を **`protoncli`** に統一（旧 `proton-cli` バイナリ名は廃止）。`.desktop` も `Exec=protoncli launch …` / `protoncli-<id>.desktop`。
- TUI **Proton settings**:
  - 現在使っている Proton を **★ + 緑** で表示（カーソル上なら緑背景）。
  - 一覧から **Enter / Space** で default を切替（`prefer` も source に合わせる）。
  - 上部に Using / PROTONPATH を表示。prefer の有効側（[m]/[t]）も色分け。
- `doctor` でも default Proton に ★ を付与。

## 0.1.2

- TUI 初回起動で起動ロケール（日本語 / English）を選択。`config.locale` に保存し、`launch` / `run` で `LANG` / `LC_*` を付与（ホストへの locale-gen 不要）。カタログは拡張前提（UI は当面 2 択）。
- **グローバル locale** を Home → Locale で後から変更可能。
- **アプリ個別 locale**（`apps.toml` の `locale`、TUI Apps の `o`、CLI `add --locale` / `run --locale`）。未設定は global を継承。
- TUI に色分け（タイトル / 選択行 / status / 画面種別など）。
- **Launch settings**（Steam/Lutris 相当）:
  - 既定でホストの `VK_ICD_FILENAMES` / `VK_DRIVER_FILES` を起動時に除去（`[launch].sanitize_vulkan`、32bit DXVK 対策）。
  - グローバル / アプリ別 **graphics**（`dxvk` | `wined3d`）。TUI Home → Launch settings、Apps → `o`、CLI `--graphics`。
  - `run --no-sanitize-vulkan` でホスト ICD 固定を残すことも可。
  - **log_level**（`quiet` | `normal` | `verbose`）。TUI Launch settings で切替、`[launch].log_level`、CLI `run --log-level`。`quiet` は umu/Proton/Wine の端末出力を捨てる。
- **TUI から登録後の exe 変更**（Apps → `o` → Exe）:
  - 探索元: **Wineprefix 内**（bottle 外へ出ない / `drive_c` 起点）または **ホスト filesystem**。
  - 一覧は **ディレクトリと `.exe` のみ**。確定時に `cwd` を exe 親へ更新し、`.desktop` も再生成。
- 開発向け: `Makefile` / `AGENTS.md` を追加。
- Publish CI: 既に crates.io に載っている版は成功扱い（再実行で `forgejo-release` がスキップされないようにする）。

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
- crates.io package name: **`protoncli`** (binary was historically `proton-cli`).
