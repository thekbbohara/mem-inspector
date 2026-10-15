#!/usr/bin/python3
"""Launch installed runtime with real HOME/PATH, without scans or inference.
DISPLAY selects a disposable X server because the worker has no desktop display.
No transient library/toolchain environment is supplied to the app.
"""

import json
import hashlib
import os
import runpy
import subprocess
import sys
import time
from pathlib import Path

HERE = Path(__file__).resolve().parent
helpers = runpy.run_path(str(HERE / "release-ui-check.py"))
UI, tree = helpers["UI"], helpers["tree"]
evidence = Path(sys.argv[1])
runtime = (Path(sys.argv[2]) if len(sys.argv) > 2 else Path.home() / ".local/share/mem-inspector/current").resolve()
assert "/.local/share/mem-inspector/" in str(runtime)
env = os.environ.copy()
for key in [
    "LD_LIBRARY_PATH",
    "PKG_CONFIG_PATH",
    "CARGO_HOME",
    "RUSTUP_HOME",
    "WEBKIT_EXEC_PATH",
    "WEBKIT_INJECTED_BUNDLE_PATH",
    "WEBKIT_INSPECTOR_RESOURCES_PATH",
    "WEBKIT_INSPECTOR_HTTP_SERVER",
    "MEM_INSPECTOR_FIXTURE_ROOT",
    "MEM_INSPECTOR_FIXTURE_TRASH",
    "MEM_INSPECTOR_BINARY",
    "MEM_INSPECTOR_RUNTIME",
    "MEM_INSPECTOR_HOME",
]:
    env.pop(key, None)
env["DISPLAY"] = ":144"
xenv = dict(env, LD_LIBRARY_PATH=str(runtime / "runtime/usr/lib"))
xvfb = subprocess.Popen(
    [
        str(runtime / "runtime/usr/bin/Xvfb"),
        ":144",
        "-screen",
        "0",
        "1280x800x24",
        "-nolisten",
        "tcp",
    ],
    env=xenv,
    stdout=(evidence / "delivery-normal-xvfb.log").open("w"),
    stderr=subprocess.STDOUT,
)
app = None
try:
    helpers["wait_xvfb"](xvfb, ":144")
    # Normal launcher command. Only the outer verification namespace hides old
    # assets; this option is not part of the shipped launcher.
    app = subprocess.Popen(
        [
            "bwrap",
            "--bind",
            "/",
            "/",
            "--dev-bind",
            "/dev",
            "/dev",
            "--tmpfs",
            "/tmp",
            "--ro-bind",
            "/tmp/.X11-unix",
            "/tmp/.X11-unix",
            str(runtime / "mem-inspector"),
        ],
        env=env,
        start_new_session=True,
        stdout=(evidence / "delivery-normal-launch.log").open("w"),
        stderr=subprocess.STDOUT,
    )
    u = UI(evidence, ":144")
    text = u.wait(
        lambda: (
            t
            if "local storage" in (t := u.text()).lower() and "Local commands" in t
            else None
        ),
        "normal launch",
    )
    # Discovery can take several bounded CLI probes. Do not click any operation.
    time.sleep(3)
    text = u.text()
    assert "A little space to breathe" in text and "History" in text
    u.capture("delivery-normal-home-launch.png")
    (evidence / "delivery-normal-launch.json").write_text(
        json.dumps(
            {
                "command": str(runtime / "mem-inspector"),
                "home": env["HOME"],
                "runtime": str(runtime),
                "binary_sha256": hashlib.sha256((runtime / "bin/mem-inspector-desktop").read_bytes()).hexdigest(),
                "display_only_test_override": ":144",
                "old_toolchain_hidden": True,
                "all_tmp_hidden_except_x11_socket": True,
                "no_transient_loader_or_toolchain_env": True,
                "no_scan_no_inference": True,
                "processes": tree(app.pid),
                "rendered_ui": text,
            },
            indent=2,
        )
    )
    print(text)
finally:
    if app:
        import signal

        for p in reversed(tree(app.pid)):
            try:
                os.kill(p["pid"], signal.SIGTERM)
            except ProcessLookupError:
                pass
        app.wait(timeout=5)
    xvfb.terminate()
    xvfb.wait(timeout=5)
