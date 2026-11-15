# SPDX-License-Identifier: MIT
"""模板的冒烟测试：握手帧正确，收到按键事件后写 stderr，EOF 时正常退出。"""

import json
import pathlib
import struct
import subprocess
import sys

MAIN = pathlib.Path(__file__).with_name("main.py")


def frame(msg):
    data = json.dumps(msg).encode()
    return struct.pack(">I", len(data)) + data


def test_handshake_and_key_event():
    p = subprocess.run(
        [sys.executable, str(MAIN)],
        input=frame({"t": "key", "class": "letter"}),
        capture_output=True,
        timeout=10,
    )
    assert p.returncode == 0
    (n,) = struct.unpack(">I", p.stdout[:4])
    hello = json.loads(p.stdout[4 : 4 + n])
    assert hello == {"t": "hello", "protocol": [1], "want": ["key.class"]}
    assert "letter" in p.stderr.decode()


if __name__ == "__main__":
    test_handshake_and_key_event()
    print("ok")
