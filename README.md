# proton-cli

Linux 上で Windows のゲーム・アプリを起動するための **CLI + ミニマル TUI** です。  
実行エンジンは **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)** と **Proton** で、既定の Proton ビルドは **[dwproton](https://dawn.wine/dawn-winery/dwproton)**（Dawn Winery）です。

本ツールは **薄いオーケストレータ** です。Wine / Proton 本体は同梱しません。アプリ登録・wineprefix 管理・Proton 取得・アプリメニュー用ショートカット作成を行い、起動は `umu-run` に任せます（現行の Lutris / Heroic が Proton を使うやり方と同系統）。

---

## 必要なもの（インストール必須・任意）

`proton-cli` 単体ではゲームは起動しません。次を揃えてください。

### 必須（どちらかの経路）

**経路 A — Steam だけ（推奨しやすい）**

| もの | 役割 |
|------|------|
| **Steam** | クライアント + ライブラリ |
| **Steam Linux Runtime**（sniper 等） | コンテナ起動（Steam が Proton 用に入れる） |
| **Proton**（Steam 同梱 / GE / dwproton） | Windows 互換 |
| **GPU ドライバ**（Vulkan） | DXVK / 描画 |

`umu-run` は **不要** です。Steam は `umu-run` を同梱しませんが、proton-cli は  
`SteamLinuxRuntime + proton` で Steam と同じ系統の起動ができます。

**経路 B — umu-launcher（Steam なしでも可）**

| もの | 役割 |
|------|------|
| **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)**（`umu-run`） | Steam 外向けの統合ランチャ |
| **Proton** | `proton-cli proton install` または既存ビルド |
| **GPU ドライバ**（Vulkan） | 同上 |

```bash
# 経路 B の例
# Arch:   pacman -S umu-launcher
# Fedora: dnf install umu-launcher
proton-cli proton install
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
  proton-cli 0.1.1+ は Steam 経路にフォールバックするので、`umu-run not found` でも起動できます。
- Steam の Proton / dwproton / GE-Proton は自動検出します。
- Steam ストアのゲーム起動や anti-cheat は本ツールの対象外です。

---

## インストール（proton-cli 本体）

crates.io の **パッケージ名は `protoncli`** です（`proton-cli` は既存クレートに取られているため）。  
インストール後の **コマンド名は `proton-cli`** のままです。

```bash
# crates.io
cargo install protoncli

# Git（Codeberg）
cargo install --git https://codeberg.org/xuanling/proton-cli.git

# ソースから
cargo install --path .
```

`proton-cli` が **PATH 上**にあること（例: `~/.cargo/bin`）。  
アプリメニューのショートカットは `Exec=proton-cli launch <id>` のため、PATH が通っていないとメニューから起動できません。

導入後の健全性確認:

```bash
proton-cli doctor
```

`umu-run` や Proton が無い場合はここで不足が分かります。

---

## クイックスタート

```bash
# 1. 依存確認（umu / Proton / PATH）
proton-cli doctor

# 2. umu が入っていること（ディストロで先に導入）
# 3. dwproton を ~/.protoncli/runtimes に入れる（Steam 不要）
proton-cli proton install

# 4. アプリ登録（既定 wineprefix = ~/.protoncli/default）
proton-cli add --name "My Game" --exe /path/to/Game.exe

# 専用 prefix を指定する場合
proton-cli add --name "Other" --exe /path/to/Other.exe --prefix other-game

# 5. 起動
proton-cli launch my-game

# 対話セットアップ TUI（引数なし）
proton-cli
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
proton-cli add --name "A" --exe ./A.exe

# 専用 bottle
proton-cli add --name "B" --exe ./B.exe --prefix my-b-prefix
# → ~/.protoncli/prefixes/my-b-prefix

proton-cli prefix list
proton-cli prefix create indie-game
proton-cli prefix path default
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
  proton-cli-<id>.desktop  # アプリメニュー用（任意・登録時に作成）
```

Proton 解決時は、managed（`~/.protoncli/runtimes`）に加え、Steam が入っていれば  
`compatibilitytools.d` もスキャンします（**Steam のインストールは任意**）。

---

## TUI（引数なし）

```bash
proton-cli
# または
proton-cli tui
```

ゲーム前に触りそうな項目だけのミニマル UI です。

| 画面 | 内容 |
|------|------|
| Apps | 一覧・起動・削除（prefix は保持） |
| Register app | name / exe / id / prefix / proton |
| Wine prefixes | 一覧・`pfx-N` 作成・設定の default 変更 |
| Proton settings | dwproton インストール、prefer managed / steam |
| Doctor | umu・PROTONPATH・パス確認 |

各画面のタイトル／status にキー操作のヒントがあります（`↑↓` `Enter` `Esc` など）。

---

## コマンド一覧

| コマンド | 説明 |
|----------|------|
| （なし）/ `tui` | セットアップ TUI |
| `add --name --exe [--prefix]` | 登録 + 既定で `.desktop` |
| `launch <id>` | 登録アプリを起動 |
| `run <exe>` | 未登録のワンショット起動 |
| `list` / `remove` | アプリ管理 |
| `prefix list\|create\|path` | `~/.protoncli` の prefix 管理 |
| `desktop install\|uninstall` | アプリメニュー用ショートカット |
| `proton install\|list` | dwproton 取得 / 検出一覧 |
| `doctor` / `path <id>` | 診断・パス表示 |

---

## 設定ファイル

初回実行で `~/.protoncli/config.toml` が作られます。

```toml
default_prefix = "default"

[proton]
default = "dwproton"
prefer = "managed"   # managed | steam
# prefer = "steam" にする場合は Steam を入れ、compatibilitytools.d に Proton があること

[umu]
# command = "/usr/bin/umu-run"   # PATH に無いときだけ指定

[desktop]
create_on_add = true
categories = ["Game"]
```

- `prefer = "managed"` … `~/.protoncli/runtimes` と `proton-cli proton install` を主に使う（**Steam 不要**）
- `prefer = "steam"` … Steam 配下の Proton を優先（**Steam の導入と、そこに Proton があることが前提**）

---

## 起動の流れ

```
proton-cli launch <id>
  → WINEPREFIX=~/.protoncli/…  PROTONPATH=…  GAMEID=…
  → umu-run /path/to/game.exe
```

互換性の本体は **umu + Proton** 側です。本バイナリは設定と起動の橋渡しだけをします。

---

## 依存関係まとめ（チェックリスト）

導入順の目安（Steam 利用者）:

1. **GPU ドライバ（Vulkan）**
2. **Steam** を入れ、Proton と Steam Linux Runtime を用意（通常は Steam が自動）
3. **proton-cli** … `cargo install protoncli`
4. `proton-cli doctor` で **backend=steam** と **PROTONPATH** を確認
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
| バイナリ | **`proton-cli`** |
| リポジトリ | https://codeberg.org/xuanling/proton-cli |

---

## crates.io について

- 配布物は **小さな Rust バイナリ**のみです。Proton（数百 MB）は同梱しません。
- 実行する Windows ソフトウェアのライセンス・合法性は利用者の責任です。
- anti-cheat やストア DRM によっては Steam / 公式ランチャが別途必要な場合があります。

---

## ライセンス

MIT OR Apache-2.0（`LICENSE-MIT` / `LICENSE-APACHE` を参照）。
