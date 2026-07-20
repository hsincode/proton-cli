# proton-cli development helpers
# CI と同じく --locked を既定にする（Cargo.lock を正とする）

CARGO ?= cargo
BIN    := proton-cli

.PHONY: help check test build release run install install-local \
        fmt fmt-check clippy package clean ci doctor

help: ## このヘルプを表示
	@grep -E '^[a-zA-Z_-]+:.*?## ' $(MAKEFILE_LIST) | \
		awk 'BEGIN {FS = ":.*?## "}; {printf "  \033[36m%-14s\033[0m %s\n", $$1, $$2}'

check: ## 型チェックのみ（高速）
	$(CARGO) check --locked

test: ## テスト（CI 相当）
	$(CARGO) test --locked

build: ## デバッグビルド
	$(CARGO) build --locked

release: ## リリースビルド（CI 相当）
	$(CARGO) build --release --locked

# 例: make run                 → 最新ソースで TUI
#     make run ARGS='doctor'
#     make run ARGS='launch my-game'
# PATH 上の古い ~/.cargo/bin/proton-cli は使わない（必ず target/debug を起動）。
run: build ## ソースをビルドしてから実行（最新版）。ARGS 省略で TUI
	./target/debug/$(BIN) $(ARGS)

install-local: ## ソースから ~/.cargo/bin へインストール
	$(CARGO) install --path . --locked --force

install: install-local ## install-local の別名

fmt: ## rustfmt で整形
	$(CARGO) fmt

fmt-check: ## rustfmt チェックのみ（書き込みなし）
	$(CARGO) fmt -- --check

clippy: ## clippy 警告をエラー扱い
	$(CARGO) clippy --locked -- -D warnings

package: ## crates.io 用パッケージの dry-run
	$(CARGO) package --locked --allow-dirty

clean: ## target/ を削除
	$(CARGO) clean

ci: test release ## ローカルで CI 相当（test + release）

doctor: build ## ビルドしたバイナリで doctor を実行
	./target/debug/$(BIN) doctor
