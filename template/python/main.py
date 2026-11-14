# SPDX-License-Identifier: MIT
"""扩展程序最小模板（Python，只用标准库）。

收到按键事件就把「按下一个什么类别的键」写到 stderr（输入法把它收进
extension-<id>.log）。复制整个目录、改名、改 extension.toml 即可起步；协议见
docs/protocol.md。

⚠️ 不要把用户输入的内容（具体按键、上屏文字）写进日志。
"""

import json
import struct
import sys


def read():
    """读一帧：4 字节大端长度 + UTF-8 JSON。EOF 返回 None（宿主要求退出）。"""
    head = sys.stdin.buffer.read(4)
    if len(head) < 4:
        return None
    (n,) = struct.unpack(">I", head)
    return json.loads(sys.stdin.buffer.read(n))


def send(msg):
    data = json.dumps(msg).encode()
    sys.stdout.buffer.write(struct.pack(">I", len(data)) + data)
    sys.stdout.buffer.flush()


def main():
    # 启动后第一件事：声明支持的协议版本和要申请的授权档位。
    send({"t": "hello", "protocol": [1], "want": ["key.class"]})
    while (msg := read()) is not None:
        if msg["t"] == "key":
            print("按下一个", msg["class"], file=sys.stderr, flush=True)
        # 不认识的消息类型直接忽略：协议只会加类型、加字段。


if __name__ == "__main__":
    main()
