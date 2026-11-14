<!-- SPDX-License-Identifier: MPL-2.0 -->

# 扩展程序协议（v1）

> 草案：协议未定版，字段仍可能调整。
>
> 本协议文档采用 MPL-2.0 许可（[全文](../LICENSES/MPL-2.0.txt)）：按协议另写实现不受任何约束，只有修改本文档本身时，改动部分要公开。

清风输入法把输入过程中的事件（按键、上屏、模式）**单向、异步**投递给扩展程序。扩展只能
旁观，不能改变上屏结果；扩展崩溃、卡死、处理慢，都不影响打字。

v1 只在 Windows 上可用。

## 1. 进程关系

```text
输入法核心 ──匿名管道──▶ 扩展宿主 wind_input_ext ──子进程 stdio──▶ 扩展
```

- 用户在设置里打开「扩展程序」总开关并启用某个扩展后，核心拉起扩展宿主，宿主再拉起扩展。
  扩展**不能自行启动后接入**。
- 扩展的工作目录是它自己的扩展目录，环境变量 `WIND_EXTENSION_ID` 是扩展 id。
- **stdin** 收宿主发来的帧，**stdout** 回帧，**stderr** 由宿主追加写入
  `<日志目录>/extension-<id>.log`（超过 1MB 时轮换为 `extension-<id>.old.log`）。
- 宿主随核心退出，扩展随宿主退出（Windows 上都在 Job 对象里）。扩展读到 stdin EOF 时应立即退出。

## 2. 安装

扩展装在用户目录的 `extensions/<id>/` 下（Windows：`%APPDATA%\WindInput\extensions\<id>\`）。
**目录名就是扩展 id**，只许小写字母、数字、`-`、`_`，最长 64 个字符。删除目录即卸载。

目录里必须有清单 `extension.toml`：

```toml
name = "按键音"                    # 设置里显示的名字，省略时用 id
exe = "keysound.exe"               # 目录内的相对路径；也可以按平台分写：
# exe = { windows = "keysound.exe", linux = "keysound", macos = "keysound" }
args = []                          # 启动参数
want = ["key.detail"]              # 申请的授权档位，见 §4
protocol = [1]                     # 支持的协议版本

[[option]]                         # 可选：扩展自己的选项，见 §6
key = "volume"
label = "音量"
type = "number"
default = 0.6
min = 0.0
max = 1.0
step = 0.05
```

可执行文件必须在扩展目录内：绝对路径、`..`、指向目录外的符号链接都会被拒绝。清单写错时，
设置里会显示原因，扩展不会被拉起。

## 3. 帧

每一帧是 **4 字节大端无符号长度 + UTF-8 JSON 对象**，单帧上限 1MB。

```jsonc
{"t":"key","v":1,"seq":12,"dropped":3, ...消息字段}
```

| 字段 | 说明 |
|---|---|
| `t` | 消息类型 |
| `v` | 协议版本 |
| `seq` | 这条管道上单调递增的序号，可以据缺口判断丢包 |
| `dropped` | 上一条成功投递以来丢掉的帧数，为 0 时省略 |

**前向兼容**：不认识的 `t` 直接丢弃，不认识的字段直接忽略。新版本只会加类型、加字段。

## 4. 授权档位

| 档位 | 能收到 |
|---|---|
| `key.class` | `key` 事件，只有分类（`class`），没有具体键名和修饰键 |
| `key.detail` | `key` 事件的全部字段；隐含 `key.class` |
| `mode` | `mode` 事件 |
| `commit` | `commit` 事件 |
| `composition` | 预留，v1 不产出 |
| `candidates` | 预留，v1 不产出 |
| `candidate.provide` | 预留给将来的候选注入，v1 一律不授予 |

清单里的 `want` 只是申请，实际授予以用户在设置里勾选的为准（首次启用时默认授予申请的档位）。
握手时扩展收到的 `granted`，是申请与用户授予两者的交集。

**隐私降级**：焦点在密码框时，`key.detail` 降为 `key.class`，`commit` 不发。

## 5. 消息

### 5.1 握手

扩展启动后先发 `hello`。**握手前宿主不会发任何事件。**

```jsonc
// 扩展 → 宿主
{"t":"hello","protocol":[1],"want":["key.detail"]}
// 宿主 → 扩展
{"t":"welcome","protocol":1,"granted":["key.class","key.detail"],"core":"0.127.0"}
```

`granted` 是 `hello` 里的 `want` 与用户授予的交集：`want` 可以比清单里申请的少，但**不能省略**，
省略就什么事件都收不到。`welcome` 之后，宿主会立即：

- 补发一次当前的 `mode`（如果授予了 `mode`）；
- 发一次 `options`（见 §6）。

### 5.2 其他扩展 → 宿主消息

| 消息 | 说明 |
|---|---|
| `{"t":"subscribe","events":["key"]}` | 进一步收窄要收的事件类型，取值 `key`/`mode`/`commit`。不发就是收全部已授予的 |
| `{"t":"ping"}` | 宿主回 `{"t":"pong"}`，用于自检 |

### 5.3 事件

所有事件都带 `ts`：核心的单调时钟毫秒，不是墙上时间。只能用来比较先后、计算间隔，比如
按键音扩展可以丢弃已经滞后几十毫秒的按键。

**key**：按下一个键。没有抬起事件，长按连发时每次重复都会发。

```json
{"t":"key","ts":123,"sid":7,"app":"notepad.exe","key":"a","mods":["shift"],"class":"letter","handled":true,"observed":false}
```

| 字段 | 说明 |
|---|---|
| `sid` | 组合会话 id：一次编码输入从第一个键到上屏或取消共用一个值；没有组合时为 0 |
| `app` | 当前应用的进程名，可能为空 |
| `key` | 物理键名，只有 `key.detail` 档才有；不在下表里的键省略 |
| `mods` | 按下的修饰键：`shift`、`ctrl`、`alt`、`win`；只有 `key.detail` 档才有，为空时省略 |
| `class` | 分类，见下表 |
| `handled` | 输入法是否处理了这个键（编码、选词等）；`false` 表示原样交给了应用 |
| `observed` | `true` 表示输入法本身不处理这个键（如英文模式下打字），前端只是额外上报给扩展 |

键名与产出的字符无关：`a` 就是 A 键，无论打出的是 `a`、`A` 还是编码。

| `class` | `key` 可能的取值 |
|---|---|
| `letter` | `a`~`z` |
| `digit` | `0`~`9` |
| `numpad` | `numpad0`~`numpad9`、`numpad_add`、`numpad_subtract`、`numpad_multiply`、`numpad_divide`、`numpad_decimal` |
| `function` | `f1`~`f24` |
| `punct` | `backtick`、`minus`、`equal`、`lbracket`、`rbracket`、`backslash`、`semicolon`、`quote`、`comma`、`period`、`slash` |
| `space` | `space` |
| `enter` | `enter` |
| `backspace` | `backspace` |
| `edit` | `tab`、`escape`、`delete`、`insert` |
| `navigation` | `home`、`end`、`pageup`、`pagedown`、`left`、`right`、`up`、`down` |
| `modifier` | `capslock`、`lshift`、`rshift`、`lctrl`、`rctrl` |
| `other` | 无（`key` 省略） |

**mode**：中英、全半角、方案变化时发送，相同状态不重复发。

```json
{"t":"mode","ts":124,"chinese":true,"full_width":false,"schema":"wubi86"}
```

**commit**：文字上屏。

```json
{"t":"commit","ts":130,"sid":7,"app":"notepad.exe","text":"你们","source":"candidate"}
```

| 字段 | 说明 |
|---|---|
| `source` | `candidate`（选词上屏）、`punct`（标点或其他 ASCII 文本）、`command`（命令、鼠标点选等）、`softkeyboard`（鼠标点软键盘键帽） |
| `rect` | 预留：组合矩形 `[x, y, w, h]`（屏幕物理像素），v1 不提供 |

## 6. 选项

清单里用 `[[option]]` 声明选项后，设置里会按声明生成表单，用户设的值存进输入法配置，
宿主校验后发给扩展：

```json
{"t":"options","values":{"theme":"piano","volume":0.6}}
```

- `values` 里每个声明过的选项都有值：用户没设、或设的值不合法时用默认值，数值会夹到范围内。
- 握手后发一次，之后用户每次修改都会再发一次完整取值。
- 修改清单后要重启输入法（或关掉再打开扩展）才会读到新声明。

| `type` | 字段 |
|---|---|
| `choice` | `default`（字符串）、`choices = [{ value, label }]` |
| `bool` | `default` |
| `number` | `default`、`min`、`max`、可选 `step` |

每个选项都可以带 `label`（显示名）和 `description`（说明）。`key` 只许小写字母、数字、`-`、`_`，
且不能重复；默认值不合法时整份清单无效。

## 7. 投递与监护

- **背压**：宿主给每个扩展一个 256 帧的写队列，满了就丢**最旧**的事件，并在下一帧的
  `dropped` 里告知丢了多少。控制消息（`welcome`、`options`）不会被丢。
- **卡死**：扩展不读 stdin，管道写满后宿主的一次写入阻塞超过 10 秒，宿主就结束它，按崩溃处理。扩展应该在独立线程里读 stdin，
  不要让耗时操作（播放、网络）挡住读取。
- **崩溃**：退避重启（1s、2s、4s……）；60 秒内退出 3 次就熔断，不再拉起，设置里显示原因。
  用户关掉再打开该扩展可以重置。
- 事件内容不会写进输入法和宿主的任何日志。扩展自己写的 stderr 由扩展负责，请不要把用户
  输入的内容写进去。
- 输入法不审查扩展，用户第一次打开总开关时会看到隐私提示，须勾选同意。请只申请用得到的档位。
- 建议给分发的扩展做 Authenticode 签名：未签名的程序接收按键事件，容易被杀毒软件误报。

## 8. 最小示例（Python）

```python
import json, struct, sys

def read():
    head = sys.stdin.buffer.read(4)
    if len(head) < 4:
        return None                      # EOF：宿主要我退出
    (n,) = struct.unpack(">I", head)
    return json.loads(sys.stdin.buffer.read(n))

def send(msg):
    data = json.dumps(msg).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(data)) + data)
    sys.stdout.buffer.flush()

send({"t": "hello", "protocol": [1], "want": ["key.class"]})
while (msg := read()) is not None:
    if msg["t"] == "key":
        print("按下一个", msg["class"], file=sys.stderr, flush=True)
```

对应的清单：

```toml
name = "按键日志"
exe = "python.exe"                 # 需要把解释器放进扩展目录，或改用打包好的 exe
args = ["main.py"]
want = ["key.class"]
protocol = [1]
```

可直接复制本仓 [`template/python`](../template/python) 起步（MIT，闭源也可以）。Rust 写的扩展可以依赖
WindInput 仓里的 `wind-hook` crate（消息定义与帧读写，MPL-2.0），参考本仓的 `keysound`。
