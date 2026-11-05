#!/usr/bin/env python3
"""Test cleanup confirmation and namespace isolation without SMB or hardware."""
import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

exe = str(Path(sys.argv[1]).resolve())
session = "0123456789abcdef0123456789abcdef"
with tempfile.TemporaryDirectory(prefix="ponte-storage-lifecycle-") as tmp:
    root = Path(tmp)
    env = dict(os.environ, PONTE_DESK_HOME=tmp)
    file = root / "storage-host.json"
    def stop(token):
        result = subprocess.run([exe, "storage", "stop", "--session", token], env=env,
                                text=True, capture_output=True, timeout=5)
        return result.returncode, json.loads(result.stdout)
    cancelled = dict(session=session, active=False, phase="error", cleanup_confirmed=True,
                     error="authorization cancelled", pid=0)
    file.write_text(json.dumps(cancelled))
    code, result = stop(session)
    assert code == 0 and result["phase"] == "stopped"
    assert json.loads(file.read_text())["phase"] == "stopped"
    assert not list(root.glob("storage-stop-*"))
    assert stop(session)[0] == 0  # repeated stop is safe
    unknown = dict(cancelled, cleanup_confirmed=False)
    file.write_text(json.dumps(unknown))
    before = file.read_bytes()
    assert stop(session)[0] != 0
    assert stop("f" * 32)[0] != 0
    assert stop("../../unrelated")[0] != 0
    assert file.read_bytes() == before
    assert not list(root.glob("storage-stop-*"))
print(json.dumps(dict(passed=True, cancelled_session_can_close=True,
                     unknown_cleanup_not_claimed=True, unrelated_state_preserved=True,
                     smb_used=False, physical_media_used=False)))
