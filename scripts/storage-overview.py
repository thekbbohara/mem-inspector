#!/usr/bin/env python3
"""Read-only GNU du / lsblk / statvfs storage evidence. Never follows symlinks.

One `du -a --time` traversal yields everything: folder totals (depth <= 4),
used space by data type, big files, git repositories with their newest
modification time, and the entries of ~/Downloads.

`storage-overview.py ROOT` scans a home. `storage-overview.py --kinds MOUNT`
only measures data types on another disk (used for the HDD bars).
"""
import json
import os
import subprocess
import sys
import tempfile
import time

KINDS = ('video', 'images', 'code', 'deps', 'caches', 'models', 'archives', 'other')
VIDEO_EXT = {'.mp4', '.mkv', '.mov', '.avi', '.webm', '.m4v', '.wmv', '.flv', '.mts', '.m2ts', '.mpg',
             '.mpeg', '.ts', '.prores', '.mxf', '.braw', '.r3d', '.mp3', '.flac', '.wav', '.m4a', '.ogg',
             '.opus', '.aac', '.aiff', '.blend', '.kdenlive', '.prproj', '.drp'}
IMAGE_EXT = {'.jpg', '.jpeg', '.png', '.gif', '.webp', '.heic', '.heif', '.avif', '.tif', '.tiff', '.bmp',
             '.svg', '.psd', '.xcf', '.kra', '.raw', '.cr2', '.cr3', '.nef', '.arw', '.dng', '.orf',
             '.rw2', '.exr', '.ico'}
ARCHIVE_EXT = {'.zip', '.tar', '.gz', '.tgz', '.xz', '.txz', '.zst', '.bz2', '.7z', '.rar', '.iso', '.img',
               '.dmg', '.qcow2', '.vdi', '.vmdk', '.vhd', '.vhdx', '.deb', '.rpm', '.apk', '.aab', '.ova',
               '.sql', '.dump', '.bak'}
MODEL_EXT = {'.gguf', '.safetensors', '.ckpt', '.pt', '.pth', '.onnx', '.bin', '.h5', '.tflite', '.mlmodel',
             '.ggml', '.npz', '.engine'}
CODE_EXT = {'.py', '.rs', '.ts', '.tsx', '.js', '.jsx', '.go', '.c', '.h', '.cpp', '.hpp', '.java', '.kt',
            '.swift', '.rb', '.php', '.cs', '.lua', '.sh', '.toml', '.lock', '.vue', '.svelte', '.dart',
            '.ipynb', '.md', '.json', '.yaml', '.yml', '.css', '.scss', '.html'}
DEP_DIRS = {'node_modules', '.venv', 'venv', 'site-packages', '__pycache__', '.tox', 'target', '.rustup',
            '.cargo', '.nvm', '.bun', '.deno', 'go', '.gradle', '.m2', '.pub-cache', '.npm-global', 'Sdk',
            '.local/lib', 'dist-packages', '.dart_tool', 'Pods', 'vendor', '.pnpm-store', 'build', 'dist',
            '.docker', 'flatpak'}
MODEL_DIRS = {'.ollama', 'huggingface', '.lmstudio', 'lm-studio', 'ComfyUI', 'stable-diffusion-webui',
              'whisper', 'torch'}
CACHE_DIRS = {'.cache', 'cache', 'Cache', 'caches', '.npm', '.yarn', 'Trash', 'CacheStorage', 'Code Cache',
              'GPUCache', '.next', '.turbo', '.parcel-cache', 'Service Worker', 'ShaderCache', '.ccache'}
BIG_FILE = 100 << 20


def run(args, timeout=120):
    return subprocess.run(args, stdout=subprocess.PIPE, stderr=subprocess.PIPE, timeout=timeout)


def classify(path, root):
    rel = os.path.relpath(path, root)
    parts = rel.split(os.sep)
    if parts[-1] in ('node_modules', 'target', '.venv', 'venv'):
        return ('Rebuildable dependencies', 'Review', 'Generated dependencies or build output. Confirm the project, lockfiles and offline requirements before regenerating; target may contain executables you use.')
    if rel == '.cache' or rel.startswith('.cache/'):
        return ('Application cache', 'Review', 'Cache location observed. Close the owning application and use its cache controls; some apps keep downloads or offline state here.')
    if rel == '.local/share/Trash':
        return ('Trash', 'Review', 'Already trashed data. Review original locations and recovery needs before emptying Trash in your file manager.')
    if parts[0] in ('Downloads', 'Videos', 'Pictures', 'Documents'):
        return ('Personal media / documents', 'Keep', 'Personal data. Review large archives and media for backup or placement, never assume disposable.')
    if any(x in parts for x in ('worktrees',)):
        return ('Agent workspaces', 'Keep', 'May contain uncommitted work, logs and deliverables. Check git status and active tasks before archiving any workspace.')
    if any(x in parts for x in ('.codex', '.claude', '.agents', '.local')):
        return ('Application / agent state', 'Keep', 'May contain credentials, history, installed tools and models. Use application-specific retention controls.')
    return ('Folder contents', 'Keep', 'Directory size is measured; contents and ownership are not classified. Inspect before making a retention decision.')


def context_kind(rel_dir):
    """Data type implied by the folders a file sits in, or None."""
    parts = rel_dir.split('/') if rel_dir else []
    joined = '/' + rel_dir + '/'
    if any(p in MODEL_DIRS for p in parts):
        return 'models'
    if any(p in DEP_DIRS for p in parts) or '/.local/lib/' in joined:
        return 'deps'
    if any(p in CACHE_DIRS for p in parts):
        return 'caches'
    if '.git' in parts:
        return 'code'
    return None


def ext_kind(name, size):
    ext = os.path.splitext(name)[1].lower()
    if ext in VIDEO_EXT:
        return 'video'
    if ext in IMAGE_EXT:
        return 'images'
    if ext in MODEL_EXT and (ext != '.bin' or size >= 256 << 20):
        return 'models'
    if ext in ARCHIVE_EXT:
        return 'archives'
    if ext in CODE_EXT:
        return 'code'
    return None


class Kinds:
    """Per-file data type accounting during one du -a traversal."""

    def __init__(self, root):
        self.root = root
        self.prefix = len(root) + 1 if root != '/' else 1
        self.totals = dict.fromkeys(KINDS, 0)
        self.context = {}
        self.unknown = {}
        self.repos = {}
        self.big = []
        # Running bytes per top-level folder, so a live scan can show progress
        # before du reports a folder's total.
        self.live = {}
        self.seen = 0

    def file(self, path, size, mtime):
        self.seen += size
        top = path[self.prefix:].split('/', 1)[0]
        self.live[top] = self.live.get(top, 0) + size
        parent, _, name = path.rpartition('/')
        rel_dir = parent[self.prefix:] if len(parent) >= self.prefix else ''
        ctx = self.context.get(parent, 0)
        if ctx == 0:
            ctx = self.context[parent] = context_kind(rel_dir)
        if size >= BIG_FILE:
            self.big.append(dict(path=path, allocated=size, mtime=mtime))
        kind = ctx if ctx and not (ctx == 'code' and size >= BIG_FILE) else None
        if name == '.git':
            self.repos.setdefault(parent, None)
        if kind is None:
            kind = ext_kind(name, size)
        if kind is None:
            self.unknown[parent] = self.unknown.get(parent, 0) + size
        else:
            self.totals[kind] += size

    def directory(self, path, size, mtime):
        if path.endswith('/.git'):
            self.repos.setdefault(path[:-5], None)
        if path in self.repos:
            self.repos[path] = (size, mtime)

    def finish(self):
        in_repo = {}

        def repo(d):
            chain = []
            found = False
            while len(d) >= len(self.root):
                if d in in_repo:
                    found = in_repo[d]
                    break
                chain.append(d)
                if d in self.repos:
                    found = True
                    break
                d = d.rpartition('/')[0]
            for c in chain:
                in_repo[c] = found
            return found

        for d, size in self.unknown.items():
            self.totals['code' if repo(d) else 'other'] += size
        self.big.sort(key=lambda b: -b['allocated'])
        return {k: str(v) for k, v in self.totals.items()}


def traverse(root, kinds, on_dir=None, tick=None):
    """Stream du -a; returns (total, exit code, errors)."""
    total = '0'
    with tempfile.TemporaryFile() as error_file:
        process = subprocess.Popen(['du', '-x', '-B1', '-a', '--time', '--time-style=+%s', '--null', '--', root],
                                   stdout=subprocess.PIPE, stderr=error_file)
        pending = b''
        previous = ''
        try:
            while chunk := process.stdout.read1(1 << 20):
                pending += chunk
                records = pending.split(b'\0')
                pending = records.pop()
                for record in records:
                    size, mtime, path = record.split(b'\t', 2)
                    path = os.fsdecode(path)
                    size = int(size)
                    mtime = int(mtime) if mtime.isdigit() else 0
                    # du -a is post-order: a folder follows its last descendant.
                    if previous.startswith(path + '/') or path == root:
                        if path == root:
                            total = str(size)
                        kinds.directory(path, size, mtime)
                        if on_dir:
                            on_dir(path, size, mtime)
                    else:
                        kinds.file(path, size, mtime)
                    previous = path
                if tick:
                    tick()
            code = process.wait()
            process.stdout.close()
        finally:
            if process.poll() is None:
                process.kill()
                process.wait()
        error_file.seek(0)
        errors = error_file.read().decode(errors='replace').splitlines()
    return total, code, errors


def disks_info():
    disks = []
    block = run(['lsblk', '--json', '-b', '-o', 'NAME,TYPE,SIZE,ROTA,FSTYPE,MOUNTPOINTS'], 10)

    def visit(items, parent=None):
        for item in items:
            hardware = item if item['type'] == 'disk' else parent
            for mount in item.get('mountpoints') or []:
                if not mount or not mount.startswith('/') or item.get('fstype') not in ('ext4', 'btrfs', 'xfs', 'ntfs', 'vfat', 'exfat'):
                    continue
                try:
                    v = os.statvfs(mount)
                    disks.append(dict(mount=mount, device='/dev/' + item['name'], filesystem=item['fstype'], medium='HDD (rotational)' if hardware and hardware.get('rota') else ('NVMe SSD / non-rotational' if hardware and hardware['name'].startswith('nvme') else 'SSD / non-rotational'), total=str(v.f_blocks*v.f_frsize), used=str((v.f_blocks-v.f_bfree)*v.f_frsize), free=str(v.f_bavail*v.f_frsize)))
                except OSError:
                    pass
            visit(item.get('children', []), hardware)
    if block.returncode == 0:
        visit(json.loads(block.stdout)['blockdevices'])
    disks.sort(key=lambda d: (d['mount'] != '/', d['mount'] == '/boot', d['mount']))
    return disks


def overview(root):
    started = time.monotonic()
    root = os.path.abspath(root)
    if not os.path.isdir(root) or os.path.islink(root):
        raise ValueError('Scan root must be an existing real directory')
    disks = disks_info()
    rows = []
    downloads_dir = os.path.join(root, 'Downloads')
    downloads = []
    kinds = Kinds(root)
    last = [started]

    def snapshot(running, complete=False, errors=None):
        ordered = sorted(rows, key=lambda r: (-int(r['allocated']), r['path'].count(os.sep)))
        top = [r for r in ordered if os.path.dirname(r['path']) == root]
        candidates = []
        for row in ordered:
            if row['disposition'] == 'Review' and int(row['allocated']) >= 1048576 and not any(row['path'].startswith(c['path'] + os.sep) for c in candidates):
                candidates.append(row)
        if running:
            # Partial, growing totals per top-level folder while du walks.
            done = {r['path'] for r in top}
            live = [dict(path=os.path.join(root, name), allocated=str(size), partial=True)
                    for name, size in kinds.live.items()
                    if os.path.join(root, name) not in done and os.path.isdir(os.path.join(root, name))
                    and not os.path.islink(os.path.join(root, name))]
            top = sorted(top + live, key=lambda r: -int(r['allocated']))
        state = dict(root=root, allocated=total[0] if not running else str(kinds.seen), seconds=round(time.monotonic()-started, 2), complete=complete, running=running, errors=(errors or [])[:20], errorCount=len(errors or []), folders=top[:30], candidates=candidates[:30], disks=disks, candidateBytes=str(sum(int(r['allocated']) for r in candidates)))
        if os.environ.get('MEM_OVERVIEW_STREAM'):
            print(json.dumps(state), flush=True)
        return state

    def on_dir(path, size, mtime):
        if path == root:
            return
        if path[len(root) + 1:].count('/') < 4:
            category, disposition, reason = classify(path, root)
            rows.append(dict(path=path, allocated=str(size), category=category, disposition=disposition, reason=reason))
        if os.path.dirname(path) == downloads_dir:
            downloads.append(dict(path=path, allocated=size, newest=mtime, dir=True))

    def tick():
        if time.monotonic() - last[0] > 1:
            snapshot(True)
            last[0] = time.monotonic()

    total = ['0']
    snapshot(True)
    total[0], code, errors = traverse(root, kinds, on_dir, tick)
    state = snapshot(False, code == 0, errors)
    # Files directly in Downloads (folders were collected above). A symlinked
    # Downloads already lives on another disk.
    try:
        if os.path.islink(downloads_dir):
            raise OSError('symlinked')
        with os.scandir(downloads_dir) as entries:
            for entry in entries:
                if entry.is_file(follow_symlinks=False):
                    st = entry.stat(follow_symlinks=False)
                    downloads.append(dict(path=entry.path, allocated=st.st_blocks * 512, newest=int(st.st_mtime), dir=False))
    except OSError:
        pass
    state['kinds'] = kinds.finish()
    big = []
    for b in kinds.big[:400]:
        try:
            st = os.lstat(b['path'])
        except OSError:
            continue
        big.append(dict(path=b['path'], allocated=str(b['allocated']), mtime=int(st.st_mtime), atime=int(st.st_atime), uid=st.st_uid, links=st.st_nlink))
    state['bigFiles'] = big
    state['repos'] = [dict(path=p, allocated=str(v[0]), newest=v[1]) for p, v in kinds.repos.items() if v and v[0] >= 64 << 20]
    state['downloads'] = [dict(d, allocated=str(d['allocated'])) for d in downloads]
    state['scannedAt'] = int(time.time())
    # Independent scope measurement for the displayed candidates; never sum
    # these scopes as reclaimable because hardlinks may connect siblings.
    from concurrent.futures import ThreadPoolExecutor, as_completed

    def measure(row):
        measured = run(['du', '-sx', '-B1', '--null', '--', row['path']], 180)
        if measured.returncode == 0:
            row['scopeAllocated'] = measured.stdout.split(b'\t', 1)[0].decode()
        else:
            row['scopeError'] = 'Independent scope measurement incomplete'
    with ThreadPoolExecutor(max_workers=2) as pool:
        futures = [pool.submit(measure, row) for row in state['candidates']]
        for future in as_completed(futures):
            try:
                future.result()
            except Exception:
                pass
    state['seconds'] = round(time.monotonic()-started, 2)
    if os.environ.get('MEM_OVERVIEW_STREAM'):
        print(json.dumps(state), flush=True)
    return state


def disk_kinds(mount):
    """Used space by data type on one mounted disk, for the disk bar."""
    started = time.monotonic()
    mount = os.path.abspath(mount)
    if not os.path.isdir(mount) or os.path.islink(mount):
        raise ValueError('Mount must be an existing real directory')
    kinds = Kinds(mount)
    total, code, errors = traverse(mount, kinds)
    return dict(mount=mount, allocated=total, kinds=kinds.finish(), complete=code == 0, errorCount=len(errors),
                seconds=round(time.monotonic() - started, 2), scannedAt=int(time.time()))


if __name__ == '__main__':
    try:
        if len(sys.argv) > 2 and sys.argv[1] == '--kinds':
            print(json.dumps(disk_kinds(sys.argv[2])))
        else:
            result = overview(sys.argv[1] if len(sys.argv) > 1 else os.path.expanduser('~'))
            if not os.environ.get('MEM_OVERVIEW_STREAM'):
                print(json.dumps(result))
    except Exception as error:
        print(str(error), file=sys.stderr)
        sys.exit(1)
