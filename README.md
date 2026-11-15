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

**下载现成的包**：到 [Releases](https://github.com/huanfeng/WindInputExtensions/releases) 下载
`keysound-<版本>-windows-x64.zip`（旁边有 `.sha256` 校验文件），整个 `keysound` 文件夹解压到
`%APPDATA%\WindInput\extensions\` 下。包暂未签名，杀毒软件可能误报。

**或自己构建后**，把扩展目录放到用户目录的 `extensions/<id>/` 下（Windows：`%APPDATA%\WindInput\extensions\`），
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

## 发布

推送形如 `keysound-v0.1.0` 的 tag 触发 [release 工作流](.github/workflows/release.yml)：校验 tag 与
`Cargo.toml` 版本一致，构建 Windows 包，把 zip 与 `.sha256` 挂到同名 Release（带 `-` 的版本号标为预发布）。
日常提交与 PR 跑 [CI](.github/workflows/ci.yml)：格式、clippy、测试（Windows 与 Linux），以及 Python 模板的冒烟测试。

## 许可

按目录分别声明，全文在 [LICENSES/](LICENSES/)，每个文件头有 SPDX 标识：

| 内容 | 许可 | 对你意味着 |
| --- | --- | --- |
| [`template/`](template/) 扩展模板 | MIT | 随便拷、闭源商用都行 |
| [`docs/protocol.md`](docs/protocol.md) 协议文档、[`keysound/`](keysound/) 官方按键音 | MPL-2.0 | 自己写扩展不受约束；只有修改这些文件本身时，改动那部分要公开 |
| 输入法仓里的 `wind-hook` 库 | MPL-2.0 | 同上 |

各扩展附带素材的来源与许可见其目录下的说明（如 [keysound/packs/README.md](keysound/packs/README.md)）。
