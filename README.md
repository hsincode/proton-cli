# proton-cli

Linux 上で Windows のゲーム・アプリを起動するための **CLI + ミニマル TUI** です。  
実行エンジンは **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)** と **Proton** で、既定の Proton ビルドは **[dwproton](https://dawn.wine/dawn-winery/dwproton)**（Dawn Winery）です。

本ツールは **薄いオーケストレータ** です。Wine / Proton 本体は同梱しません。アプリ登録・wineprefix 管理・Proton 取得・アプリメニュー用ショートカット作成を行い、起動は `umu-run` に任せます（現行の Lutris / Heroic が Proton を使うやり方と同系統）。

---

## 必要なもの（インストール必須・任意）

`protoncli` 単体ではゲームは起動しません。次を揃えてください。

### 必須（どちらかの経路）

**経路 A — Steam だけ（推奨しやすい）**

| もの | 役割 |
|------|------|
| **Steam** | クライアント + ライブラリ |
| **Steam Linux Runtime**（sniper 等） | コンテナ起動（Steam が Proton 用に入れる） |
| **Proton**（Steam 同梱 / GE / dwproton） | Windows 互換 |
| **GPU ドライバ**（Vulkan） | DXVK / 描画 |

`umu-run` は **不要** です。Steam は `umu-run` を同梱しませんが、protoncli は  
`SteamLinuxRuntime + proton` で Steam と同じ系統の起動ができます。

**経路 B — umu-launcher（Steam なしでも可）**

| もの | 役割 |
|------|------|
| **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)**（`umu-run`） | Steam 外向けの統合ランチャ |
| **Proton** | `protoncli proton install` または既存ビルド |
| **GPU ドライバ**（Vulkan） | 同上 |

```bash
# 経路 B の例
# Arch:   pacman -S umu-launcher
# Fedora: dnf install umu-launcher
protoncli proton install
```

起動時の優先順位: **umu-run があれば umu** → なければ **Steam 経路**。

### ビルド / インストール時に必要（開発・cargo 利用者）

| もの | 役割 |
|------|------|
| **Rust / cargo**（MSRV 1.80 目安） | `cargo install` やソースビルド |
| ネットワーク | dwproton のダウンロード、crates 取得 |

### 任意

| もの | いつ必要か |
|------|------------|
| **umu-launcher** | Steam を使わず経路 B で起動したい / Lutris と同じ umu 経路を優先したい |
| **デスクトップ環境** | アプリメニューに `.desktop` を出したい |
| **winetricks / protontricks** | 依存 DLL 整備（手動） |
| **Gamescope** | フルスクリーン等 |

### よくある誤解

- **Steam が入っていても `umu-run` は出てきません。** Steam ≠ umu。  
  protoncli 0.1.1+ は Steam 経路にフォールバックするので、`umu-run not found` でも起動できます。
- Steam の Proton / dwproton / GE-Proton は自動検出します。
- Steam ストアのゲーム起動や anti-cheat は本ツールの対象外です。

---

## インストール（protoncli 本体）

crates.io の **パッケージ名もコマンド名も `protoncli`** です（クレート名 `proton-cli` は既存スタブに占有されているため使えません）。

```bash
# crates.io
cargo install protoncli

# Git（Codeberg）
cargo install --git https://codeberg.org/xuanling/proton-cli.git

# ソースから
cargo install --path .
```

`protoncli` が **PATH 上**にあること（例: `~/.cargo/bin`）。  
アプリメニューのショートカットは `Exec=protoncli launch <id>` のため、PATH が通っていないとメニューから起動できません。

導入後の健全性確認:

```bash
protoncli doctor
```

`umu-run` や Proton が無い場合はここで不足が分かります。

---

## クイックスタート

```bash
# 1. 依存確認（umu / Proton / PATH）
protoncli doctor

# 2. umu が入っていること（ディストロで先に導入）
# 3. dwproton を ~/.protoncli/runtimes に入れる（Steam 不要）
protoncli proton install

# 4. アプリ登録（既定 wineprefix = ~/.protoncli/default）
# --name 省略時は exe のファイル名（stem）が名前になる → あとから TUI で変更可
protoncli add --exe /path/to/Game.exe
# または明示名
protoncli add --name "My Game" --exe /path/to/Game.exe

# 専用 prefix を指定する場合
protoncli add --name "Other" --exe /path/to/Other.exe --prefix other-game

# 5. 起動
protoncli launch my-game

# 対話セットアップ TUI（引数なし）
protoncli
```

---

## 既定の wineprefix

| 項目 | 値 |
|------|-----|
| **既定の prefix id** | `default` |
| **既定パス** | **`~/.protoncli/default`** |
| 名前付き prefix | `~/.protoncli/prefixes/<id>` |
| 絶対パスの `--prefix` | 指定どおり |

`add` / `run` で `--prefix` を省略すると、設定の `default_prefix`（初期値 `default`）が使われます。

```bash
# 共有 default
protoncli add --name "A" --exe ./A.exe

# 専用 bottle
protoncli add --name "B" --exe ./B.exe --prefix my-b-prefix
# → ~/.protoncli/prefixes/my-b-prefix

protoncli prefix list
protoncli prefix create indie-game
protoncli prefix path default
# 未使用 bottle を消す（default は削除不可。参照中は --force）
protoncli prefix remove orphan-game
```

---

## データ配置

```
~/.protoncli/
  config.toml
  apps.toml
  default/                 # id "default" の WINEPREFIX
  prefixes/<name>/         # その他の WINEPREFIX
  runtimes/                # managed の Proton（dwproton など）
  icons/

~/.local/share/applications/
  protoncli-<id>.desktop  # アプリメニュー用（任意・登録時に作成）
```

Proton 解決時は、managed（`~/.protoncli/runtimes`）に加え、Steam が入っていれば  
`compatibilitytools.d` もスキャンします（**Steam のインストールは任意**）。

---

## TUI（引数なし）

```bash
protoncli
# または
protoncli tui
```

ゲーム前に触りそうな項目だけのミニマル UI です。

| 画面 | 内容 |
|------|------|
| **Locale**（初回） | 起動ロケール選択（日本語 / English）。`config.locale` 未設定時に必ず表示 |
| Apps | 一覧・起動・削除（prefix は保持）。**`o`** でアプリ個別 **exe** / locale / graphics。**prefix ディレクトリが無い行は赤 + `[NO PREFIX]`** |
| Register app | **Name/Id** 文字入力（点滅カーソル）· **Exe** 内蔵ピッカー（dirs+.exe）· **Prefix** default/auto/custom（Enter 切替）· **Proton** 検出一覧から選択 |
| Wine prefixes | 一覧・`pfx-N` 作成・**`d` 削除**（`default` 不可・参照中は拒否）・設定の default 変更 |
| Proton settings | dwproton インストール、prefer managed / steam |
| Launch settings | グローバル **locale** / **graphics** (dxvk\|wined3d) / **sanitize_vulkan** / **log_level** / **gamescope** |
| Doctor | umu・PROTONPATH・パス確認 |

各画面のタイトル／status にキー操作のヒントがあります（`↑↓` `Enter` `Esc` など）。

### 登録後の exe 変更（TUI）

インストーラを `run` して prefix に入れたあと、本編バイナリへ差し替える流れです。

1. **Apps** → 対象を選んで **`o`**（App settings）
2. **Exe** を選んで Enter
3. 探索元を選ぶ  
   - **Wineprefix** … そのアプリの bottle 内だけ（`drive_c` から開始、外へは出ない）  
   - **Filesystem** … ホスト上の任意パス（現在の exe の親、または `$HOME` から。`h` / `~` でホームへ）
4. 一覧は **ディレクトリと `.exe` だけ**（他のファイルは非表示）
5. ディレクトリは Enter で入る / `.exe` は Enter で確定（`cwd` も exe の親に更新、`.desktop` も再生成）

CLI から本編を最初から登録するなら、従来どおり `add --exe /path/to/Game.exe --prefix <id>` でも可。

---

## コマンド一覧

| コマンド | 説明 |
|----------|------|
| （なし）/ `tui` | セットアップ TUI |
| `add --exe [--name] [--prefix]` | 登録 + 既定で `.desktop`（`--name` 省略時は exe stem） |
| `launch <id>` | 登録アプリを起動 |
| `run <exe>` | 未登録のワンショット起動 |
| `list` / `remove` | アプリ管理（`list` の PREFIX 列は欠落時に `id!`） |
| `prefix list\|create\|remove\|path` | `~/.protoncli` の prefix 管理（`remove` は `default` 不可、参照中は `--force`） |
| `desktop install\|uninstall` | アプリメニュー用ショートカット |
| `proton install\|list` | dwproton 取得 / 検出一覧 |
| `doctor` / `path <id>` | 診断・パス表示 |

---

## 設定ファイル

初回実行で `~/.protoncli/config.toml` が作られます。

```toml
default_prefix = "default"
# locale = "ja"   # または "en"。未設定なら TUI 初回で選択
# 起動時に LANG / LC_ALL / LC_CTYPE を設定。ホストに ja_JP が無くても
# umu/Steam ランタイムがコンテナ内で生成する（Lutris と同じ）。

[proton]
default = "dwproton"
prefer = "managed"   # managed | steam
# prefer = "steam" にする場合は Steam を入れ、compatibilitytools.d に Proton があること

[launch]
# Steam/Lutris 同様、ホストの VK_ICD_FILENAMES 固定を外す（32bit DXVK 対策）。既定 true
sanitize_vulkan = true
# 既定の D3D 変換: dxvk（Vulkan）| wined3d（OpenGL フォールバック）
graphics = "dxvk"
# 起動時の端末ログ量: quiet | normal | verbose（既定 normal）
# quiet は umu / Proton / Wine の stdout・stderr を捨てる（日々のプレイ向け）
# log_level = "quiet"

# gamescope ラップ（既定オフ）。排他フルスクリーンやマルチモニタで崩れやすいタイトル向け。
# 有効時: gamescope -f --force-windows-fullscreen -- <umu|steam+proton> …
# [launch.gamescope]
# enabled = false
# fullscreen = true                 # -f（ホスト側フルスクリーン）
# force_windows_fullscreen = true   # ネスト内の排他 FS を出力いっぱいに広げる
# # width = 2560                    # 任意: -W / -H / -w / -h
# # height = 1440
# # extra_args = ["-S", "integer"]

# 全アプリ共通の起動環境変数（未記載時は下記ビルトインが使われる）
# dwproton は未設定だと DXVK_HUD=compiler を入れ「Compiling shaders...」を出すため 0 で消す
[launch.env]
DXVK_HUD = "0"
STEAMOS = "1"
UMU_USE_STEAM = "1"

[umu]
# command = "/usr/bin/umu-run"   # PATH に無いときだけ指定

[desktop]
create_on_add = true
categories = ["Game"]
```

- `prefer = "managed"` … `~/.protoncli/runtimes` と `protoncli proton install` を主に使う（**Steam 不要**）
- `prefer = "steam"` … Steam 配下の Proton を優先（**Steam の導入と、そこに Proton があることが前提**）
- `locale` … グローバル起動ロケール（`ja` / `en`）。アプリに `locale` が無ければこれを使う
- `[launch].sanitize_vulkan` … ホストの `VK_ICD_FILENAMES` / `VK_DRIVER_FILES` を起動時に外す（Steam/Lutris 相当。x86_64 ICD だけ指定されていると 32bit が落ちる）
- `[launch].graphics` … グローバル D3D バックエンド（`dxvk` / `wined3d`）
- `[launch].log_level` … 起動時の端末ログ（`quiet` / `normal` / `verbose`）。TUI の Launch settings、または `run --log-level quiet`
  - **quiet** … 子プロセス（umu/Proton/Wine）の stdout/stderr を捨て、protoncli のバナーも出さない
  - **normal** … 従来どおり子の出力を端末に出す（既定）
  - **verbose** … normal に加え protoncli 側の起動診断を追加
- `[launch.gamescope]` … **既定オフ**。有効にすると `gamescope [flags…] -- <backend>` でラップ（フルスクリーン可）
  - `enabled` … マスタースイッチ（既定 `false`）
  - `fullscreen` … `-f`（既定 `true`）
  - `force_windows_fullscreen` … ネスト内の排他 FS を埋める（既定 `true`）
  - `width` / `height` / `nested_width` / `nested_height` / `extra_args` / `command` … 任意
  - CLI: `launch <id> --gamescope` / `--no-gamescope`、`run … --gamescope`
  - アプリ個別: `apps.toml` の `gamescope = true|false`、または TUI Apps → **o** → Gamescope、`add --gamescope`
- `[launch.env]` … 全起動に付与する環境変数。未記載時は `DXVK_HUD=0` / `STEAMOS=1` / `UMU_USE_STEAM=1`。TUI Home → **Environment variables** で編集・ビルトインへリセット可
- アプリ個別: `apps.toml` の `locale` / `graphics` / `gamescope` / `use_custom_env` + `env`、TUI の Apps → **`o` または `e`**、または `add --locale ja --graphics dxvk` / `run --locale en --graphics wined3d`
  - **env モード default**（既定）… グローバル `[launch.env]` のみ
  - **env モード custom** … グローバル既定の**上に**アプリ `env` を追加（同じキーは上書き）
- 優先順位（env キーごと）: **アプリ custom env > グローバル `[launch.env]` > アプリ locale/graphics 由来 > （それ以外）**

---

## 起動の流れ

```
protoncli launch <id>
  → (optional) gamescope -f --force-windows-fullscreen --
  → WINEPREFIX=~/.protoncli/…  PROTONPATH=…  GAMEID=…
  → umu-run /path/to/game.exe
```

互換性の本体は **umu + Proton** 側です。本バイナリは設定と起動の橋渡しだけをします。

---

## 依存関係まとめ（チェックリスト）

導入順の目安（Steam 利用者）:

1. **GPU ドライバ（Vulkan）**
2. **Steam** を入れ、Proton と Steam Linux Runtime を用意（通常は Steam が自動）
3. **protoncli** … `cargo install protoncli`
4. `protoncli doctor` で **backend=steam** と **PROTONPATH** を確認
5. `add` → `launch`

| チェック | 必須? |
|----------|--------|
| Steam **または** umu-launcher | **どちらか必須** |
| Proton | **必須** |
| SteamLinuxRuntime | Steam 経路では **強く推奨**（無いと bare proton） |
| umu-run | 任意（あれば優先） |

### crates.io パッケージ名

| 項目 | 値 |
|------|-----|
| package | **`protoncli`**（`cargo install protoncli`） |
| バイナリ / コマンド | **`protoncli`** |
| リポジトリ | https://codeberg.org/xuanling/proton-cli |

---

## crates.io について

- 配布物は **小さな Rust バイナリ**のみです。Proton（数百 MB）は同梱しません。
- 実行する Windows ソフトウェアのライセンス・合法性は利用者の責任です。
- anti-cheat やストア DRM によっては Steam / 公式ランチャが別途必要な場合があります。

---

## ライセンス

MIT OR Apache-2.0（`LICENSE-MIT` / `LICENSE-APACHE` を参照）。
