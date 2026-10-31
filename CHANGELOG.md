# Changelog

## 0.1.7

- **TUI を作り直し**（ライブラリ型）:
  - 起動直後がアプリ一覧。`Enter` で起動、右に実際に使われる設定と由来（global / override）を表示。設定は `e` → `←→` でその場で変更・即保存（18 画面 → 5 タブ + ポップアップ）。
  - タブ: Library / Proton / Prefixes / Settings / Doctor（`1`–`5`・`Tab`）。上部バーに問題の件数。
  - `/` でアプリ絞り込み、`:` でコマンドパレット（アプリ名で起動・全操作を検索）、`?` でキー一覧。
  - 削除（アプリ・prefix・env リセット）は確認ダイアログを表示。
  - dwproton のインストールを TUI 内で実行し進捗を表示（TUI を抜けない）。`proton::install_dwproton_with` に進捗コールバックを追加。
  - 通知は種別（成功・警告・エラー）付きで数秒後に消える。文字列マッチによる色分けを廃止。
  - Add app フォーム: exe はパス入力にも対応、名前と id は自動入力（プレースホルダ表示）、`Ctrl+S` で保存。
  - `tui.rs`（3,935 行）を `src/tui/` 配下のモジュールに分割。描画ごとのディスク読み込みを廃止。
- **アプリ個別 Proton**: `apps.toml` の `proton`（未設定は global `config.proton.default` を継承）。TUI Library → **e** → Proton、`add --proton NAME`、Add app フォームの Proton 行。
  - 名前付きインストール（GE や特定の `dwproton-*`）は global `explicit_path` より優先（タイトルごとにビルドを固定できる）。
- **バイナリ縮小**: release プロファイルに LTO / strip / `opt-level = "s"` / `panic = "abort"` を設定し、HTTP クライアントを `reqwest`（tokio/hyper）から `ureq` に置き換え。9.6 MB → 2.9 MB。
  - `panic = "abort"` でも TUI がパニック時に端末（raw mode / 代替スクリーン）を元に戻すよう panic hook を追加。
- **リポジトリを GitHub に移行**: https://github.com/hsincode/proton-cli 。CI / 公開ワークフローを Forgejo Actions から GitHub Actions に移植（リリースは GitHub Release を作成）。

## 0.1.6

- TUI **ライトテーマ向け可読性**: 本文を `Color::White` 固定から端末デフォルト前景色へ。警告/アクセントは Bold、入力フォーカス値は黄 on 暗灰でコントラスト確保。
- TUI **Exe ブラウザ**: 長い一覧を `ListState` でスクロール表示（選択行が常に可視）。位置表示 `n/total`。
- **`doctor`**: バージョン表示、PATH 上の `protoncli` 実パス、旧名 `proton-cli` 残存の警告。
- **`make install`**: 旧 `proton-cli` バイナリ掃除、PATH 解決結果の明示、古い `.desktop` の警告。

## 0.1.5

- **`[launch.gamescope]`**（既定 **off**）: 起動を `gamescope [flags…] -- <umu|steam+proton>` でラップ。既定フラグは `-f` + `--force-windows-fullscreen`（ホスト FS とネスト内排他 FS を両立）。
  - CLI: `launch` / `run` の `--gamescope` / `--no-gamescope`、`add --gamescope` / `--no-gamescope`
  - アプリ個別: `apps.toml` の `gamescope = true|false`（TUI Apps → o → Gamescope）
  - TUI Launch settings でグローバル on/off、`doctor` に状態表示
- **`protoncli prefix remove <id>`**（CLI）: managed wineprefix を削除。`default` は常に拒否。他アプリが参照中は拒否、**`--force`** で参照中もディレクトリのみ削除（`apps.toml` は触らない）。
- TUI **Wine prefixes**: **`d`** で prefix 削除（`--force` 相当なし。参照中は status に理由表示）。
- TUI **Apps**: 解決先 prefix ディレクトリが無い行を **赤** で表示し **`[NO PREFIX]`** を明示。
- CLI **`list`**: prefix 欠落時は PREFIX 列を `id!` 表記。
- TUI **Register app** フォーム改善:
  - **Exe**: 内蔵ファイルピッカー（ホスト FS・dirs + `.exe` のみ）。
  - **Proton**: 検出済みインストールのリストから選択（↑↓/jk/←→、Space で次）。
  - **Prefix**: `default` / `auto`（app id と同 slug）/ `custom`（文字入力、初期空）。Enter/Space で切替。
  - 文字入力欄は背景ハイライト + **点滅カーソル**。

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
