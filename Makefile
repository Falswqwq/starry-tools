# 常用命令。打包安装只覆盖 Linux；macOS / Windows 见 docs/development.md#打包。
PREFIX ?= $(HOME)/.local
BIN     = target/release/starrytools

.PHONY: run build test lint install uninstall

run:
	cargo run --release -p starrytools-app

build:
	cargo build --release -p starrytools-app

test:
	cargo test

lint:
	cargo clippy --all-targets

# 装到 PREFIX（默认 ~/.local）：二进制、桌面入口、hicolor 图标。
install: build
	install -Dm755 $(BIN) $(DESTDIR)$(PREFIX)/bin/starrytools
	install -Dm644 app/assets/starrytools.desktop $(DESTDIR)$(PREFIX)/share/applications/starrytools.desktop
	install -Dm644 app/assets/32x32.png   $(DESTDIR)$(PREFIX)/share/icons/hicolor/32x32/apps/starrytools.png
	install -Dm644 app/assets/128x128.png $(DESTDIR)$(PREFIX)/share/icons/hicolor/128x128/apps/starrytools.png
	install -Dm644 app/assets/icon.png    $(DESTDIR)$(PREFIX)/share/icons/hicolor/512x512/apps/starrytools.png
	@echo "装好了：$(DESTDIR)$(PREFIX)/bin/starrytools。应用菜单可能需要重新登录才会刷新。"

uninstall:
	rm -f $(DESTDIR)$(PREFIX)/bin/starrytools
	rm -f $(DESTDIR)$(PREFIX)/share/applications/starrytools.desktop
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/32x32/apps/starrytools.png
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/128x128/apps/starrytools.png
	rm -f $(DESTDIR)$(PREFIX)/share/icons/hicolor/512x512/apps/starrytools.png
