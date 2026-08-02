# protoncli development helpers
# CI と同じく --locked を既定にする（Cargo.lock を正とする）

CARGO      ?= cargo
BIN        := protoncli
# cargo install の既定配置。CARGO_HOME / CARGO_INSTALL_ROOT を尊重する。
CARGO_HOME ?= $(HOME)/.cargo
INSTALL_ROOT := $(if $(CARGO_INSTALL_ROOT),$(CARGO_INSTALL_ROOT),$(CARGO_HOME))
BIN_DIR    := $(INSTALL_ROOT)/bin

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
# PATH 上の古い ~/.cargo/bin は使わない（必ず target/debug を起動）。
run: build ## ソースをビルドしてから実行（最新版）。ARGS 省略で TUI
	./target/debug/$(BIN) $(ARGS)

# cargo install --force は「現在の [[bin]] 名」だけを差し替える。
# 0.1.3 で proton-cli → protoncli に改名したため、古いバイナリと
# cargo の install メタデータ（protoncli@0.1.x 複数行）が残り、
# ユーザーが proton-cli を叩くと 0.1.1 のまま、という症状になる。
install-local: ## ソースから ~/.cargo/bin へ最新をインストール（旧 proton-cli も掃除）
	$(CARGO) install --path . --locked --force
	@# 改名前バイナリが残っていれば削除（cargo は別 bin 名を上書きしない）
	@if [ -e "$(BIN_DIR)/proton-cli" ]; then \
		echo "==> Removing obsolete $(BIN_DIR)/proton-cli (renamed to $(BIN) in 0.1.3)"; \
		rm -f "$(BIN_DIR)/proton-cli"; \
	fi
	@# cargo install --list に残る旧バージョン行を best-effort で掃除
	@# （仕様が protoncli@x.y.z と曖昧だと uninstall が失敗するため、行単位で消す）
	@$(CARGO) install --list 2>/dev/null | \
		awk '/^protoncli v/{ver=$$2; bin=""} /^    /{bin=$$1} \
		     ver && bin=="proton-cli"{print ver; ver=""}' | \
		while read -r ver; do \
			spec="protoncli@$${ver#v}"; \
			echo "==> cargo uninstall $${spec} (stale metadata)"; \
			$(CARGO) uninstall "$${spec}" 2>/dev/null || true; \
		done
	@# 何が PATH に載ったか明示（「最新が効かない」切り分けの第一情報）
	@inst="$(BIN_DIR)/$(BIN)"; \
	if [ ! -x "$$inst" ]; then \
		echo "error: expected binary missing: $$inst" >&2; exit 1; \
	fi; \
	echo "==> Installed $$("$$inst" --version) -> $$inst"; \
	resolved=$$(command -v $(BIN) 2>/dev/null || true); \
	if [ -z "$$resolved" ]; then \
		echo "    warning: '$(BIN)' is not on PATH (add $(BIN_DIR) to PATH)"; \
	elif [ "$$resolved" != "$$inst" ]; then \
		echo "    warning: PATH resolves to $$resolved (not $$inst)"; \
		echo "             another install may be shadowing make install"; \
	else \
		echo "    PATH OK: $$resolved"; \
	fi; \
	if command -v proton-cli >/dev/null 2>&1; then \
		echo "    warning: 'proton-cli' still on PATH at $$(command -v proton-cli) (use $(BIN))"; \
	fi; \
	stale=$$(grep -l '^Exec=proton-cli ' "$(HOME)/.local/share/applications/"*.desktop 2>/dev/null || true); \
	if [ -n "$$stale" ]; then \
		echo "    warning: .desktop files still call proton-cli (recreate shortcuts via TUI or '$(BIN) desktop install'):"; \
		echo "$$stale" | sed 's/^/      /'; \
	fi

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
