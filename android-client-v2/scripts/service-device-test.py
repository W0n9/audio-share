"""使用显式 adb UI 操作驱动真实 release APK 的前台会话回归。"""

import argparse
import json
import re
import subprocess
import threading
from pathlib import Path


def wake_screen(serial: str, *, strict: bool) -> None:
    """正常唤醒严格检查；失败清理尽力完成两个动作，不掩盖原始失败。"""
    for action in (("input", "keyevent", "224"), ("wm", "dismiss-keyguard")):
        try:
            subprocess.run(["adb", "-s", serial, "shell", *action], check=strict, timeout=10)
        except (OSError, subprocess.TimeoutExpired, subprocess.CalledProcessError):
            if strict:
                raise


parser = argparse.ArgumentParser(description=__doc__)
parser.add_argument("--serial", required=True, help="adb 设备序列号")
parser.add_argument("--config", required=True, help="只含 host/port/pinnedSpki/serverKey 的 JSON")
parser.add_argument("--output", required=True, help="脱敏测试结果文件")
parser.add_argument("--server-log", help="本次隔离 Server 的日志，用于验证连接复用")
parser.add_argument("--test", choices=("ready", "rebind", "recreate", "stop", "lock"), default="ready")
args = parser.parse_args()
if args.test in ("recreate", "lock") and not args.server_log:
    parser.error("recreate/lock 需要 --server-log，才能验证没有重新建立连接")
config = json.loads(Path(args.config).read_text())
command = ["adb", "-s", args.serial, "shell", "am", "instrument", "-w", "-r", "-e", "case", args.test,
           "dev.audioshare.v2.test/dev.audioshare.v2.ControlSessionInstrumentation"]
process = subprocess.Popen(command, stdout=subprocess.PIPE, stderr=subprocess.STDOUT, text=True)
timer = threading.Timer(45, process.kill)
timer.start()
lines = []
launched = False
screen_asleep = False
connections_at_ready = None
assert process.stdout is not None
try:
    for line in process.stdout:
        lines.append(line)
        if "AWAITING_ACTIVITY" in line and not launched:
            launched = True
            launch = ["adb", "-s", args.serial, "shell", "am", "start", "-W", "-f", "0x10008000",
                      "-n", "dev.audioshare.v2/.MainActivity"]
            for name in ("host", "pinnedSpki", "serverKey"):
                launch.extend(["-e", name, str(config[name])])
            launch.extend(["--ei", "port", str(config["port"]), "--ez", "bindHint", "false"])
            result = subprocess.run(launch, stdout=subprocess.PIPE, stderr=subprocess.STDOUT,
                                    text=True, timeout=20)
            if result.returncode or "Error:" in result.stdout:
                lines.append("FAIL: adb 未能显式启动 Activity\n")
        elif "NATIVE_READY" in line and args.server_log:
            connections_at_ready = Path(args.server_log).read_text().count("[info] v2 connection accepted")
        elif "AWAIT_SCREEN_SLEEP" in line:
            subprocess.run(["adb", "-s", args.serial, "shell", "input", "keyevent", "223"], check=True, timeout=10)
            screen_asleep = True
        elif "AWAIT_SCREEN_WAKE" in line:
            wake_screen(args.serial, strict=True)
            screen_asleep = False
    process.wait(timeout=5)
except (subprocess.TimeoutExpired, subprocess.CalledProcessError):
    lines.append("FAIL: adb 操作失败或超时\n")
finally:
    timer.cancel()
    if process.poll() is None:
        process.kill()
    process.wait(timeout=5)
    if screen_asleep:
        wake_screen(args.serial, strict=False)
output = "".join(lines)
if args.server_log and connections_at_ready is not None:
    final_connections = Path(args.server_log).read_text().count("[info] v2 connection accepted")
    output += f"\nServer connection count at READY/final: {connections_at_ready}/{final_connections}\n"
    if connections_at_ready == 0 or (args.test in ("lock", "recreate") and final_connections != connections_at_ready):
        output += "FAIL: 连接复用证据缺失或生命周期改变导致新建连接\n"
for name in ("pinnedSpki", "serverKey"):
    output = output.replace(str(config[name]), "<REDACTED>")
Path(args.output).write_text(output)
print(output)
raise SystemExit(0 if process.returncode == 0 and "FAIL:" not in output and re.search(r"OK \(\d+ tests?\)", output) else 1)
