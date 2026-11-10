"""Regression: blocked lease I/O must not stall IPC or shutdown (Unix, dummy only)."""
import json
import os
from pathlib import Path
import signal
import socket
import subprocess
import sys
import tempfile
import time

exe = str(Path(sys.argv[1]).resolve())
with tempfile.TemporaryDirectory(prefix="ponte-coordinator-stall-") as folder:
    root = Path(folder)
    def unused_port():
        with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
            sock.bind(("127.0.0.1", 0))
            return sock.getsockname()[1]
    (root / "preferences.json").write_text(json.dumps({"shared_monitor": False, "manage_displays": False, "input_mode": "to_mac"}))
    (root / "observer.json").write_text(json.dumps({"side": "mac", "key": "4b" * 32, "peers": [], "port": unused_port(), "bind_address": "127.0.0.1"}))
    (root / "config.toml").write_text(f"port = {unused_port()}\n")
    env = dict(os.environ, PONTE_DESK_HOME=str(root), LAN_MOUSE_SOCKET_PATH=str(root / "ipc.sock"), LAN_MOUSE_IPC_PORT=str(unused_port()))
    env.pop("LAN_MOUSE_INTEGRATION_CONTROL", None)
    with (root / "test.log").open("wb") as log:
        proc = subprocess.Popen([exe, "--capture-backend", "dummy", "--emulation-backend", "dummy", "daemon"], env=env, stdout=log, stderr=log)
        try:
            deadline = time.monotonic() + 8
            while not (root / "engine-status.json").exists():
                assert proc.poll() is None, (root / "test.log").read_text()
                assert time.monotonic() < deadline, "startup failed"
                time.sleep(.05)
            # No writer opens this FIFO: the next read stays blocked in the OS.
            os.mkfifo(root / "agent-leases" / "blocked.json")
            time.sleep(.8)
            started = time.monotonic()
            subprocess.run([exe, "cli", "list"], env=env, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=1.5, check=True)
            ipc_ms = (time.monotonic() - started) * 1000
            started = time.monotonic()
            proc.send_signal(signal.SIGINT)
            proc.wait(2)
            assert proc.returncode == 0, (root / "test.log").read_text()
            print(json.dumps({"passed": True, "ipc_ms": ipc_ms, "shutdown_ms": (time.monotonic() - started) * 1000, "blocked_lease_io": True, "dummy_only": True}))
        finally:
            if proc.poll() is None:
                proc.kill()
                proc.wait(3)
