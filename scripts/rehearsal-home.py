#!/usr/bin/env python3
"""Build a disposable fake home for exercising delete/move/run/undo end to end.

Usage: rehearsal-home.py DIR [ARCHIVE]
Prints the env to launch against it, e.g.
  MEM_INSPECTOR_SCAN_HOME=DIR/home MEM_INSPECTOR_ARCHIVE=DIR/archive mem-inspector
Put DIR on the SSD and ARCHIVE on the HDD to see real SSD -> HDD moves.
Never point the app at real data while rehearsing.
"""
import os
import subprocess
import sys
import time

root = os.path.abspath(sys.argv[1])
home = os.path.join(root, "home")
archive = os.path.abspath(sys.argv[2]) if len(sys.argv) > 2 else os.path.join(root, "archive")
now = time.time()
old = now - 120 * 86400


def blob(rel, mib, age=None):
    path = os.path.join(home, rel)
    os.makedirs(os.path.dirname(path), exist_ok=True)
    with open(path, "wb") as f:
        chunk = os.urandom(1 << 20)
        for _ in range(mib):
            f.write(chunk)
    if age:
        os.utime(path, (age, age))
    return path


def age_tree(rel, when):
    top = os.path.join(home, rel)
    for dirpath, dirnames, filenames in os.walk(top, topdown=False):
        for name in filenames + dirnames:
            os.utime(os.path.join(dirpath, name), (when, when), follow_symlinks=False)
    os.utime(top, (when, when))


# Rebuildable caches and dependencies (cleanup lane).
blob(".cache/pip/wheels.bin", 160)
blob(".cache/ms-playwright/chromium.bin", 90)
blob(".npm/_cacache/index.bin", 230)
blob(".gradle/caches/modules.bin", 260)
blob("code/old-site/node_modules/react/index.bin", 140)
blob("code/old-site/package.json", 0, old)
blob("code/old-api/.venv/lib/torch.bin", 180)
blob("code/old-api/pyproject.toml", 0, old)
# A uv cache: the app offers `uv cache clean --cache-dir <this home>/.cache/uv`.
blob(".cache/uv/archive-v0/torch-wheel.bin", 210)
blob(".cache/uv/CACHEDIR.TAG", 0)
# A Go module cache with many small files, so `go clean -modcache` (scoped to
# this home with GOMODCACHE) runs long enough to watch in the terminal.
mod = os.path.join(home, "go/pkg/mod")
for i in range(1500):
    pkg = os.path.join(mod, f"example.com/pkg{i}@v1.0.0")
    os.makedirs(pkg, exist_ok=True)
    for j in range(60):
        with open(os.path.join(pkg, f"f{j}.go"), "w") as f:
            f.write("package p\n" * 40)
blob("go/pkg/mod/cache/download/example.com/big/@v/v1.0.0.zip", 210)
for dirpath, dirnames, filenames in os.walk(mod):
    for name in filenames:
        os.chmod(os.path.join(dirpath, name), 0o444)

# Placement lane: data that belongs on the HDD.
blob("Videos/2024-trip.mkv", 150, old)
blob("Videos/demo-recording.mp4", 120, old)
blob("isos/archlinux-2025.iso", 160, old)
blob("models/llama-3-8b.Q4.gguf", 520, old)
blob("Downloads/installer-bundle/setup.run", 60)
blob("Downloads/installer-bundle/readme.txt", 0)
age_tree("Downloads/installer-bundle", now - 60 * 86400)
project = os.path.join(home, "code/old-game")
blob("code/old-game/assets/textures.pak", 230)
blob("code/old-game/src/main.rs", 0)
subprocess.run(["git", "init", "-q", project], check=True)
age_tree("code/old-game", now - 100 * 86400)

# Never offered: config and secrets.
blob(".ssh/id_ed25519", 0)
blob(".config/app/huge-clip.mp4", 110, old)
os.makedirs(archive, exist_ok=True)
print(f"MEM_INSPECTOR_SCAN_HOME={home} MEM_INSPECTOR_ARCHIVE={archive}")
