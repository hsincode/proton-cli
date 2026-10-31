# AGENTS.md — proton-cli

Linux 上で Windows ゲーム/アプリを **umu + Proton** 経由で起動する薄い CLI オーケストレータ（Rust）。
Wine / Proton 本体は同梱しない。ユーザー向け説明は [README.md](README.md)。

| 項目 | 値 |
|------|-----|
| リポジトリ | https://github.com/hsincode/proton-cli |
| crates.io パッケージ名 | `protoncli`（`proton-cli` は占有済み） |
| バイナリ / コマンド名 | `protoncli` |
| MSRV | 1.80 |
| 既定 Proton | dwproton（Dawn Winery） |
| ランタイムデータ | `~/.protoncli/`（config / apps / prefixes / runtimes） |

## 開発コマンド

```bash
make help          # ターゲット一覧
make test          # cargo test --locked
make release       # cargo build --release --locked
make ci            # test + release（CI 相当）
make run           # 最新ソースを build → target/debug で TUI（PATH の古い bin は使わない）
make run ARGS='doctor'
```

素の cargo でも可:

```bash
cargo test --locked
cargo build --release --locked
cargo run --locked --bin protoncli -- <subcommand>
```

CI（`.github/workflows/ci.yml`、GitHub Actions）は `cargo test --locked` と `cargo build --release --locked`。

## ソース構成（`src/`）

| モジュール | 役割 |
|------------|------|
| `main.rs` / `cli.rs` | エントリ・clap 定義 |
| `paths.rs` | `~/.protoncli` レイアウト固定 |
| `config.rs` / `apps.rs` | `config.toml` / `apps.toml` |
| `locale.rs` | 起動ロケールカタログ（`LANG`/`LC_*`、TUI 初回選択） |
| `browse.rs` | TUI exe ピッカー用（dirs + `.exe` のみ列挙） |
| `launch.rs` | umu-run 優先、なければ Steam 経路。Vulkan ICD sanitize / graphics |
| `proton.rs` / `steam.rs` | Proton 取得・検出 |
| `desktop.rs` | `.desktop` ショートカット |
| `doctor.rs` | 診断（CLI `doctor`、TUI Doctor タブの元データ） |
| `tui/` | TUI。`app`（状態）· `actions`（保存→再読込）· `keys` · `view` / `view_overlay`（描画）· `overlay`（ピッカー・フォーム・ブラウザ・パレット）· `checks` · `text` · `theme` |
| `error.rs` | エラー型 |

## 守ること

- **薄いオーケストレータを維持する**。互換性本体（DXVK / protonfixes 等）をここに再実装しない。起動は `umu-run` または Steam Linux Runtime + Proton に委譲。
- **パッケージ名とバイナリ名**: crates.io もコマンドも **`protoncli`**（リポジトリ名は `proton-cli`）。
- **パスの正**: 既定 wineprefix は `~/.protoncli/default`（id `default`）。名前付きは `~/.protoncli/prefixes/<id>`。
- **ユーザーの `~/.protoncli/` をリポジトリに持ち込まない**（gitignore 対象外でもコミットしない）。
- 変更後は少なくとも `cargo test --locked` を通す。

## コメント方針

非自明なロジックやヒューリスティックには **"why"** を書く。自明な getter の言い換えや古いコメントの放置は避ける。
