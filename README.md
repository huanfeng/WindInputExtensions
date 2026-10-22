# 清风输入法扩展程序

清风输入法（WindInput）的官方扩展程序：示例与基础扩展。扩展程序是独立进程，由输入法拉起，
只能旁观输入过程中的事件（按键、上屏、模式），不能改变上屏结果。协议见 [docs/protocol.md](docs/protocol.md)。

> 开发中：协议未定版，v1 只在 Windows 上可用。

| 目录 | 扩展 | 说明 |
|---|---|---|
| [keysound](keysound/) | 按键音 | 按键时播放音效：键盘、打字机录音音效包，钢琴、木琴合成音色，也可以放自己的音效 |

## 构建

需要与 [WindInput](https://github.com/huanfeng/WindInput) 仓平级放置（扩展依赖其中的
`wind-hook` crate）：

```text
windinput/
  WindInput/
  WindInputExtensions/
```

```sh
cargo test
# Windows 版（在 Linux 上交叉编译用 cargo-xwin）
cargo xwin build --release --target x86_64-pc-windows-msvc
```

## 安装

把扩展目录放到用户目录的 `extensions/<id>/` 下（Windows：`%APPDATA%\WindInput\extensions\`），
例如按键音：

```text
extensions/keysound/
  extension.toml
  keysound.exe
  packs/
```

然后在输入法设置的「高级 → 扩展程序」里打开总开关、启用该扩展。第一次打开总开关会弹出隐私提示，
勾选同意后才能启用。

> ⚠️ 扩展程序能收到你授权的按键、上屏文字等输入内容。输入法只负责转发，不审查扩展程序的来源和
> 行为。只安装、启用你信任的扩展程序。

## 许可

GPL-3.0-or-later，见 [LICENSE](LICENSE)。各扩展附带素材的来源与许可见其目录下的说明
（如 [keysound/packs/README.md](keysound/packs/README.md)）。
