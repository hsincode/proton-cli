# proton-cli

Linux 上で Windows のゲーム・アプリを起動するための **CLI + ミニマル TUI** です。  
実行エンジンは **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)** と **Proton** で、既定の Proton ビルドは **[dwproton](https://dawn.wine/dawn-winery/dwproton)**（Dawn Winery）です。

本ツールは **薄いオーケストレータ** です。Wine / Proton 本体は同梱しません。アプリ登録・wineprefix 管理・Proton 取得・アプリメニュー用ショートカット作成を行い、起動は `umu-run` に任せます（現行の Lutris / Heroic が Proton を使うやり方と同系統）。

---

## 必要なもの（インストール必須・任意）

`proton-cli` 単体ではゲームは起動しません。次を揃えてください。

### 必須

| もの | 役割 | 導入方法の例 |
|------|------|----------------|
| **Linux（x86_64）** | 実行環境 | ディストロ本体 |
| **GPU ドライバ**（Vulkan） | Proton / DXVK が前提 | NVIDIA / AMD / Intel の公式またはディストロの Vulkan 対応ドライバ |
| **[umu-launcher](https://github.com/Open-Wine-Components/umu-launcher)**（`umu-run`） | Steam 同等のランタイム上で Proton を起動 | 下記「umu の入れ方」 |
| **Proton 本体**（dwproton など） | Windows 互換レイヤ | `proton-cli proton install`（推奨）または Steam 経由で入れた Proton を検出 |

```bash
# umu-launcher の例
# Arch
pacman -S umu-launcher

# Fedora
dnf install umu-launcher

# パッケージが無い場合
# https://github.com/Open-Wine-Components/umu-launcher のリリース / ビルド手順を参照
```

```bash
# Proton（dwproton）— proton-cli から取得（Steam 不要）
proton-cli proton install
```

### ビルド / インストール時に必要（開発・cargo 利用者）

| もの | 役割 |
|------|------|
| **Rust / cargo**（MSRV 1.80 目安） | `cargo install` やソースビルド |
| ネットワーク | dwproton のダウンロード、crates 取得 |

### 任意だがよく使うもの

| もの | いつ必要か | 備考 |
|------|------------|------|
| **[Steam](https://store.steampowered.com/about/)** | `prefer = "steam"` で Steam 配下の Proton を使う / 既に Steam に入れた GE-Proton・dwproton を流用する | **起動そのものには必須ではない**。`proton-cli proton install` で managed に入れれば Steam なしでも可 |
| **Steam Linux Runtime** | umu が内部で取得・利用 | 通常は umu が面倒を見る。手動で Steam を入れる必要はないことが多い |
| **デスクトップ環境** | アプリメニューに `.desktop` を出したいとき | `~/.local/share/applications/` に作成 |
| **winetricks / protontricks** | 依存 DLL・VC++ などを prefix に入れたいとき | 現状 CLI ラッパーは未実装（手動利用可） |
| **Gamescope** | フルスクリーン・スケール調整など | 任意。env で渡す運用 |

### 明示: Steam は必須ではない

- **ゲーム起動に Steam クライアントは不要**（umu + Proton があれば足りる）。
- Steam を入れると、`~/.steam/steam/compatibilitytools.d` などに入れた **GE-Proton / dwproton を自動検出**できる。
- Steam ゲーム本体の起動・Steamworks・一部の anti-cheat は **Steam 側**の話であり、本ツールのスコープ外。

---

## インストール（proton-cli 本体）

```bash
# crates.io（公開後）
cargo install proton-cli

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

導入順の目安:

1. **GPU ドライバ（Vulkan）** … ディストロ / ベンダー手順
2. **umu-launcher** … パッケージまたは upstream
3. **proton-cli** … `cargo install` など
4. **Proton（dwproton）** … `proton-cli proton install`  
   - または **Steam をインストール**し、Steam の互換ツールとして Proton / dwproton を入れる
5. `proton-cli doctor` で **umu-run OK** と **PROTONPATH** を確認
6. アプリを `add` して `launch`

| チェック | 必須? |
|----------|--------|
| umu-launcher | **必須**（起動時） |
| Proton（dwproton 等） | **必須**（起動時） |
| Steam | **任意**（Proton 流用・`prefer=steam` 時） |
| アプリメニュー用 DE | 任意 |
| winetricks 等 | 任意 |

---

## crates.io について

- 配布物は **小さな Rust バイナリ**のみです。Proton（数百 MB）は同梱しません。
- 実行する Windows ソフトウェアのライセンス・合法性は利用者の責任です。
- anti-cheat やストア DRM によっては Steam / 公式ランチャが別途必要な場合があります。

---

## ライセンス

MIT OR Apache-2.0（`LICENSE-MIT` / `LICENSE-APACHE` を参照）。
