"""Exercise the actual guard CLI without native input, LAN peers or app windows."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile

executable = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="ponte-agent-closed-") as temporary:
    root = Path(temporary)
    environment = dict(os.environ, PONTE_DESK_HOME=str(root))
    if os.name == "nt":
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.bind(("127.0.0.1", 0))
        environment["LAN_MOUSE_IPC_PORT"] = str(listener.getsockname()[1])
        listener.close()
    else:
        environment["LAN_MOUSE_SOCKET_PATH"] = str(root / "ipc.sock")

    def check(expected_success):
        result = subprocess.run(
            [executable, "agent", "begin", "--owner", "regression", "--ttl", "30"],
            env=environment, capture_output=True, text=True, timeout=10,
        )
        token = result.stdout.strip()
        assert token and (root / "agent-leases" / (token + ".json")).exists()
        try:
            assert (result.returncode == 0) == expected_success, result.stderr
        finally:
            subprocess.run([executable, "agent", "end", token], env=environment, check=True)

    check(True)
    # An open IPC endpoint cannot take the stopped-engine shortcut.
    if os.name == "nt":
        listener = socket.socket(socket.AF_INET, socket.SOCK_STREAM)
        listener.bind(("127.0.0.1", int(environment["LAN_MOUSE_IPC_PORT"])))
    else:
        listener = socket.socket(socket.AF_UNIX, socket.SOCK_STREAM)
        listener.bind(environment["LAN_MOUSE_SOCKET_PATH"])
    listener.listen(128)
    try:
        check(False)
    finally:
        listener.close()
    # A recorded live process remains a blocker even without an IPC endpoint.
    (root / "engine-status.json").write_text(json.dumps({"pid": os.getpid()}))
    check(False)
    print("PASS: stopped bridge allowed with lease; active IPC and live process blocked")
