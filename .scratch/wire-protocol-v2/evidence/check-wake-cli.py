"""Host-only public CLI checks; PATH contains only a fake adb, never a device."""

import json
import os
from pathlib import Path
import subprocess
import sys
import tempfile

root = Path(sys.argv[1]).resolve()
results = []
with tempfile.TemporaryDirectory(prefix="wake-cli-check-", dir=root / "target") as temporary:
    directory = Path(temporary)
    adb = directory / "adb"
    adb.write_text(f"#!{sys.executable} -I\n" + '''import json, os, sys
arguments = sys.argv[1:]
with open(os.environ["FAKE_ADB_LOG"], "a") as log:
    log.write(json.dumps(arguments) + "\\n")
if "instrument" in arguments:
    print("INSTRUMENTATION_STATUS: stream=AWAITING_ACTIVITY", flush=True)
    print("INSTRUMENTATION_STATUS: stream=AWAIT_SCREEN_SLEEP", flush=True)
    print("INSTRUMENTATION_STATUS: stream=AWAIT_SCREEN_WAKE", flush=True)
    print("OK (1 test)", flush=True)
    raise SystemExit(0)
with open(os.environ["FAKE_ADB_LOG"]) as log:
    slept = any(json.loads(line)[-3:] == ["input", "keyevent", "223"] for line in log)
if os.environ.get("FAIL_WAKE") and slept and arguments[-3:] == ["input", "keyevent", "224"]:
    raise SystemExit(7)
if os.environ.get("FAIL_WAKE") and slept and arguments[-2:] == ["wm", "dismiss-keyguard"]:
    raise SystemExit(7)
''')
    adb.chmod(0o700)
    config = directory / "public-config.json"
    config.write_text(json.dumps({
        "host": "NO-SERVER", "port": 0,
        "pinnedSpki": "HOST-ONLY-PIN", "serverKey": "HOST-ONLY-KEY",
    }))
    for fail in (False, True):
        log = directory / f"calls-{fail}.jsonl"
        output = directory / f"result-{fail}.log"
        environment = dict(os.environ, PATH=str(directory), FAKE_ADB_LOG=str(log))
        if fail:
            environment["FAIL_WAKE"] = "1"
        else:
            environment.pop("FAIL_WAKE", None)
        run = subprocess.run([
            sys.executable, "-I", str(root / "android-client-v2/scripts/service-device-test.py"),
            "--serial", "HOST-STUB-ONLY", "--config", str(config), "--output", str(output),
        ], env=environment, capture_output=True, text=True, timeout=20, cwd=root)
        calls = [json.loads(line) for line in log.read_text().splitlines()]
        wakes = sum(call[-3:] == ["input", "keyevent", "224"] for call in calls)
        dismisses = sum(call[-2:] == ["wm", "dismiss-keyguard"] for call in calls)
        assert run.returncode == (1 if fail else 0), (run.returncode, run.stdout, run.stderr)
        assert wakes == (3 if fail else 2), calls
        assert dismisses == 2, calls
        launch = next(index for index, call in enumerate(calls) if "start" in call)
        assert calls[launch - 2][-3:] == ["input", "keyevent", "224"], "启动 Activity 前必须唤醒屏幕"
        assert calls[launch - 1][-2:] == ["wm", "dismiss-keyguard"], "启动 Activity 前必须请求普通 keyguard 解除"
        if fail:
            assert "FAIL: adb 操作失败或超时" in output.read_text()
        else:
            assert "OK (1 test)" in output.read_text()
        results.append({
            "scenario": "strict failure plus best-effort cleanup" if fail else "normal strict wake",
            "exit": run.returncode, "wake_attempts": wakes,
            "dismiss_attempts": dismisses, "status": "PASS",
        })
summary = json.dumps({
    "host_stub_only": True, "real_adb_or_device_used": False, "results": results,
}, ensure_ascii=False, indent=2)
(root / ".scratch/wire-protocol-v2/evidence/green-harness-wake.log").write_text(summary + "\n")
print(summary)
