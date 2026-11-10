"""Direct capture keeps local control during an automatic interruption."""
import json
import os
from pathlib import Path
import socket
import subprocess
import sys
import tempfile
import time

exe = str(Path(sys.argv[1]).resolve())


def unused_port():
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as sock:
        sock.bind(("127.0.0.1", 0))
        return sock.getsockname()[1]


with tempfile.TemporaryDirectory(prefix="ponte-auto-recovery-") as folder:
    root = Path(folder)
    policy_path = root / "policy.json"
    # Bound loopback endpoint with no DTLS reply; no packets leave this host.
    with socket.socket(socket.AF_INET, socket.SOCK_DGRAM) as unresponsive:
        unresponsive.bind(("127.0.0.1", 0))
        (root / "config.toml").write_text(
            f"port = {unused_port()}\n[[clients]]\n"
            f'ips = ["127.0.0.1"]\nport = {unresponsive.getsockname()[1]}\n'
            'position = "left"\nactivate_on_startup = true\n'
        )

        def policy(capture, emulation):
            temp = root / "policy.new"
            temp.write_text(json.dumps(dict(capture=capture, emulation=emulation,
                                            direct=capture, automatic=True, reason="fixture",
                                            expires_ms=int(time.time() * 1000) + 25000)))
            temp.replace(policy_path)

        policy(True, False)
        env = dict(os.environ, PONTE_DESK_HOME=str(root),
                   LAN_MOUSE_INTEGRATION_CONTROL=str(policy_path),
                   LAN_MOUSE_SOCKET_PATH=str(root / "ipc.sock"),
                   LAN_MOUSE_IPC_PORT=str(unused_port()))
        log_path = root / "test.log"
        with log_path.open("wb") as log:
            process = subprocess.Popen(
                [exe, "--capture-backend", "dummy", "--emulation-backend", "dummy", "daemon"],
                env=env, stdout=log, stderr=log)
            try:
                def wait(capture, direct, emulation, seconds=5):
                    deadline = time.monotonic() + seconds
                    while time.monotonic() < deadline:
                        assert process.poll() is None, log_path.read_text()
                        try:
                            state = json.loads((root / "engine-status.json").read_text())
                            assert not (root / "manual-pause").exists(), "automatic interruption wrote manual pause"
                            if (state["capture_installed"], state["emulation_enabled"]) == (capture, emulation) and (
                                    direct is None or state["direct_active"] == direct):
                                return
                        except (FileNotFoundError, json.JSONDecodeError):
                            pass
                        time.sleep(.05)
                    raise AssertionError(log_path.read_text())

                wait(True, None, False)
                wait(False, False, False)
                released = time.monotonic()
                wait(True, None, False, seconds=10)
                assert time.monotonic() - released >= 6, "native capture rearmed too early"
                wait(False, False, False)
                policy(False, True)
                wait(False, False, True)
                time.sleep(7.2)
                wait(False, False, True)
                print("PASS: automatic interruption releases locally, retries, and cancels retry on reversal")
            finally:
                process.terminate()
                process.wait(5)
