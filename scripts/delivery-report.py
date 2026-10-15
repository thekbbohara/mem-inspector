#!/usr/bin/python3
"""Build the final owner report from actual captured delivery evidence.
Preserves the previous report once; escapes every evidence-derived value.
"""

import argparse
import hashlib
import html
import json
import shutil
import subprocess
from datetime import UTC, datetime
from pathlib import Path


def escaped(value):
    return html.escape(str(value), quote=True)


def read_json(directory, name):
    return json.loads((directory / name).read_text())


def render(evidence, commit):
    checks = read_json(evidence, "delivery-release-checks.json")
    independent = read_json(evidence, "delivery-independent-bytes.json")
    normal = read_json(evidence, "delivery-normal-launch.json")
    worker = read_json(evidence, "delivery-worker-metadata.json")
    runtime = read_json(evidence, "delivery-runtime-integrity.json")
    assert independent["byte_equality"] and independent["collision_preserved"]
    assert checks["old_toolchain_hidden"] and normal["no_scan_no_inference"]
    assert checks["advice"]["unsafe_startup_rejected"]
    e = escaped
    pictures = [
        (
            "delivery-release-review.png",
            "Optimized native UI: exact cleanup review before confirmation",
        ),
        (
            "delivery-release-trashed.png",
            "Optimized native UI: confirmed Trash move and session restore action",
        ),
        (
            "delivery-release-restore-review.png",
            "Optimized native UI: explicit restore review",
        ),
        (
            "delivery-release-collision.png",
            "Optimized native UI: restore collision rejected, both files preserved",
        ),
        (
            "delivery-release-restored.png",
            "Optimized native UI: restored original, independently byte verified",
        ),
        (
            "delivery-release-10k.png",
            "Optimized native UI: 10,003 real filesystem entries, 0 errors",
        ),
        (
            "delivery-ai-consent.png",
            "Integrated fake CLI: fresh consent before request",
        ),
        (
            "delivery-ai-inert-consumed.png",
            "Integrated fake CLI: literal markup and consumed consent",
        ),
        (
            "delivery-ai-unsafe-rejected.png",
            "Integrated fake CLI: unsafe startup rejected by production mem-ai",
        ),
        (
            "delivery-normal-home-launch.png",
            "Normal HOME: installed Claude discovered usable; no scan or inference",
        ),
    ]
    for filename, _ in pictures:
        assert (evidence / filename).is_file(), filename
    rows = []
    for name in ["idle", "after_scan"]:
        sample = checks[name]
        rows.append(
            f"<tr><td>{e(name.replace('_', ' '))}</td><td>{sample['process_count']}</td>"
            f"<td>{sample['rss_kib'] / 1024:.2f} MiB</td><td>{sample['pss_kib'] / 1024:.2f} MiB</td></tr>"
        )
    processes = []
    for name in ["idle", "after_scan"]:
        for p in checks[name]["processes"]:
            processes.append(
                f"<tr><td>{e(name)}</td><td>{p['pid']}</td><td>{p['ppid']}</td><td>{e(p['name'])}</td>"
                f"<td>{p['rss_kib']}</td><td>{p['pss_kib']}</td></tr>"
            )
    package_rows = "".join(
        f"<tr><td>{e(p['pkgname'])}</td><td>{e(p['pkgver'])}</td><td><code>{e(p['archive_sha256'])}</code></td></tr>"
        for p in runtime["packages"]
    )
    figures = "".join(
        f'<figure><figcaption>{e(caption)}</figcaption><a href="tauri-ui/{e(filename)}"><img loading="lazy" src="tauri-ui/{e(filename)}" alt="{e(caption)}"></a></figure>'
        for filename, caption in pictures
    )
    links = [
        "delivery-worker-metadata.json",
        "delivery-release-checks.json",
        "delivery-independent-bytes.json",
        "delivery-normal-launch.json",
        "delivery-runtime-integrity.json",
        "delivery-ui-check.log",
        "delivery-package.log",
        "delivery-host-tests.log",
        "delivery-clippy.log",
        "delivery-frontend-tests.log",
        "delivery-format.log",
        "delivery-python-lint.log",
        "delivery-installer-tests.log",
        "delivery-release-launch.log",
        "delivery-ai-launch.log",
        "delivery-normal-launch.log",
        "delivery-installed-claude-help.txt",
        "report-before-delivery.html",
    ]
    link_html = " · ".join(
        f'<a href="tauri-ui/{e(name)}">{e(name)}</a>' for name in links
    )
    request = checks["advice"]["records"][0]
    build_sha = checks["binary_sha256"]
    cleanup = checks["cleanup"]
    return f"""<!doctype html><html lang="en"><head><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>T137 / T141 delivery evidence</title>
<style>body{{margin:0;background:#f5f5f2;color:#202522;font:15px system-ui;line-height:1.65}}main{{max-width:1160px;margin:auto;padding:32px}}header{{border-bottom:1px solid #dce0da;padding-bottom:24px}}h1{{font-size:36px;line-height:1.2;letter-spacing:-.03em}}h2{{margin-top:32px;font-size:22px}}small,.muted{{color:#606963}}a{{color:#23694c}}table{{width:100%;border-collapse:collapse;background:white}}th,td{{padding:10px;text-align:left;border:1px solid #dce0da;overflow-wrap:anywhere}}th{{font-size:12px;color:#606963}}code,pre{{font:12px monospace;overflow-wrap:anywhere}}pre{{white-space:pre-wrap;background:white;border:1px solid #dce0da;padding:16px}}img{{display:block;width:100%;border:1px solid #dce0da;margin-top:12px}}figure{{margin:24px 0}}.ok{{color:#23694c}}details{{border-block:1px solid #dce0da;padding:12px 0}}@media(max-width:760px){{main{{padding:16px}}h1{{font-size:28px}}table{{font-size:12px}}td,th{{padding:6px}}}}</style></head><body><main>
<header><small>T137 / T139 / T140 / T141 · LOCAL DELIVERY</small><h1>Mem Inspector: four delivery gaps verified</h1><p class="ok">Optimized native release review, confirmed Trash, explicit restore, byte equality and no-overwrite collision passed. Persistent home-local launch passed with the old toolchain hidden. Whole-app RSS/PSS and a native 10,003-entry scan measured. Integrated fake CLI advice and production safety rejection passed.</p><p>Final local coding commit <code>{e(commit)}</code>. Changes are root scripts and operating documentation. Rust sibling crates and the production UI/bridge safety controls remain unmodified. Existing owner evidence is retained, including the <a href="tauri-ui/report-before-delivery.html">previous full report</a>.</p></header>
<h2>Optimized cleanup and independent filesystem verification</h2><p>Current frontend assets were rebuilt into the optimized Tauri release and deb bundle. The copied standalone binary has SHA256 <code>{e(build_sha)}</code>. Real XTest mouse/keyboard events drove the existing UI, which invoked the production bridge and mem-core. No mock bridge, inspector, fixture-only privileged automation, or production safety bypass was added.</p><p>A fresh marked disposable root, private same-filesystem Trash and isolated subprocess HOME kept all mutation confined to the fixture. Review left the original untouched. Confirmed Trash retained its exact bytes. Restore was explicitly reviewed and confirmed with an occupied original path, producing <code>Filesystem: File exists (os error 17)</code>. The occupant and trashed original both remained unchanged, with recovery metadata retained. The fixture-created occupant was moved aside without deletion; a second explicit UI confirmation restored the original.</p><pre>Original / trashed / restored SHA256: {e(cleanup["original_sha256"])}
Original length: {cleanup["original_bytes"]} bytes (includes NUL and non-UTF8 bytes)
Collision occupant SHA256: {e(cleanup["collision_sha256"])}
Independent verifier: {e(independent["verification"])}
Fixture: {e(cleanup["fixture"])}</pre><p>The independent verifier ran in a separate Python process after the UI harness exited, read the restored file and retained collision occupant, and checked their SHA256 against the pre-mutation evidence. <code>Trash/files</code> is empty after restore. Current core retains {cleanup["retained_trashinfo_count"]} .trashinfo record; this worker did not change the sibling crate.</p>
<h2>Persistent home-local launch on Arch</h2><pre># Exact installed executable, no source checkout or toolchain required:
{e(checks["runtime"])}
# Selected runtime from this checkout:
sh scripts/mem-inspector.sh
# Bootstrap from a trusted extracted Arch runtime prefix:
sh scripts/install-home-runtime.sh /absolute/path/to/extracted-arch-prefix
# Optional new destination and release binary are arguments 2 and 3.</pre><p>Assets and executable live under <code>{e(runtime["runtime"])}</code>, approximately {runtime["runtime_bytes"] / 1048576:.1f} MiB. All manifest file hashes were independently verified, and the installed launcher matches its committed source. Installations refuse existing destinations and external runtime symlinks; existing <code>current</code> selections are preserved. The installer copies user-space package extraction only and performs no downloads or system package installation.</p><p>The launcher configures paths internally and overlays only <code>/usr/lib</code> read-only through unprivileged bubblewrap. Host paths retain ordinary user permissions for native scans and OS Trash. WebKit's own sandbox stays enabled. Host GTK3/libsoup/graphics ABI compatibility, bubblewrap overlay/user-namespace support and a graphical session remain prerequisites. This is an Arch x86_64 runtime overlay, not a universal Linux installer or an OS security sandbox.</p><p>Both isolated fixture tests and normal-HOME launch hid <code>/tmp/t141-toolchain</code> entirely with an outer verification namespace. Normal launch used actual HOME <code>{e(normal["home"])}</code> and ordinary PATH/config, with no transient loader/toolchain/fixture environment supplied. It rendered the native workstation with zero entries and no cleanup operations. This worker has no desktop DISPLAY: the sole test display override selected Xvfb <code>{e(normal["display_only_test_override"])}</code>; a normal desktop terminal supplies its own graphical session. Disposable outputs under /tmp are evidence fixtures, not runtime prerequisites.</p><details><summary>Copied package versions and archive integrity</summary><table><tr><th>Package</th><th>Version</th><th>SHA256 of existing extracted-source archive</th></tr>{package_rows}</table><p>These are hashes of the preexisting local archives used in prior dependency recovery, not an independent package-signature audit.</p></details>
<h2>Optimized whole-app memory and native scan</h2><table><tr><th>State</th><th>Processes</th><th>Summed RSS</th><th>Summed PSS</th></tr>{"".join(rows)}</table><p>The optimized native app, WebKit network process and WebKit web process were sampled from /proc smaps_rollup. Every app descendant was included. The X server, test harness and launcher/session DBus are outside the app tree and excluded. Other exploratory app instances were closed before this final measurement. RSS double-counts shared mappings; PSS apportions them. These are one-run headless Xvfb observations at 1600×1200, not an interactive-GPU memory promise or a comparison to other apps.</p><p>The same optimized app scanned 10,000 freshly generated regular files plus the original fixture file, marker and root: <strong>10,003 entries, 0 errors</strong>. Fixture creation was excluded. Real XTest input to rendered text observation took <strong>{checks["large_scan"]["elapsed_ui_ms"]:.2f} ms</strong>, including input, clipboard and polling overhead. Memory after scan was sampled after a one-second settle; small cleanup/restore session history was already present. Files were warm and on tmpfs, not cold persistent storage.</p><pre>Filesystem: {e(checks["filesystem"])}
Host memory: {e(checks["mem_total"])}
Cgroup memory.max: max at every measured ancestor (no imposed finite cgroup cap)
Process resident-set / address-space limits: unlimited
Cgroup: {e(checks["cgroup"].strip())}</pre><details><summary>Exact process samples and limits</summary><table><tr><th>State</th><th>PID</th><th>PPID</th><th>Process</th><th>RSS KiB</th><th>PSS KiB</th></tr>{"".join(processes)}</table><pre>{e(checks["app_limits"])}
{e(json.dumps(checks["cgroup_memory_limits"], indent=2))}</pre></details>
<h2>Integrated AI: availability and fake-provider proof</h2><p>The installed CLI is <code>2.1.296 (Claude Code)</code>; local version/help were verified without credential-file reads. The production adapter requires this exact version, reviewed safety-help semantics, and an existing first-party claude.ai subscription login. Normal-HOME app discovery showed Claude usable, while installed Codex/agy/opencode/pi remain unsupported adapters and Ollama was missing. Discovery runs bounded version/help/auth probes, not inference. A login/version match cannot guarantee network or quota. Use the same trusted CLI PATH and HOME/config as the subscription setup; an unreviewed CLI upgrade fails closed.</p><p>This worker performed <strong>no live or paid inference</strong>. The <a href="rust-ai/report.html">AI sibling report</a> separately documents its actual tool-free subscription inference. This delivery's integrated response evidence is a fake local executable in an isolated fixture HOME, explicitly returning the exact discovery/runtime markers required by the unchanged production mem-ai adapter. It is evidence of integration and safety, not a claim that desktop live inference succeeded.</p><p>Real UI consent triggered Rust aggregate calculation, mem-ai invocation and advice rendering. The fake executable asserted the production empty-tools/MCP settings, safe/restricted modes, hook/plugin overrides, cleared API-key environment and fresh empty working directory. Its captured stdin contains only fixed categories and numeric totals. The UI showed <code>&lt;b&gt;FIXTURE ADVICE&lt;/b&gt;</code> literally, reset consent, and a second click without new consent invoked nothing. A second separately consented request deliberately reported an unsafe startup plugin; production mem-ai rejected it and the UI displayed no advice. Fixture file bytes remained unchanged.</p><pre>{e(json.dumps(request["stdin"], indent=2))}</pre><details><summary>Actual integrated invocation and environment keys</summary><pre>{e(json.dumps(request, indent=2))}</pre></details>
<h2>Verification and source scope</h2><p>Current verification passed: full optimized Tauri release/deb rebuild with TypeScript/Vite frontend build; 6 root bridge tests (one explicit benchmark ignored); root Rust formatting and Clippy with warnings denied; 3 frontend tests; home-runtime installer preservation/boundary tests; Python formatting/lint and shell syntax; real optimized fixture cleanup/collision/restore/10k scan/integrated fake advice; normal-HOME launch and independent byte/runtime-manifest checks. Prior sibling verification remains 49 combined Rust tests, debug native cleanup, and release/deb launch evidence in the preserved report. Standalone sibling suites were not needlessly rerun because neither crate changed.</p><pre>Worker model from actual argv: {e(worker["model_verified_from_argv"])} (PID {worker["pid"]})
CODEX_HOME: {e(worker["CODEX_HOME"])}
Worktree: {e(worker["cwd"])}
Base: a0e249aa92d2aa7ca576712540a2c95637f55a30
Final local commit: {e(commit)}
No subagent spawned; coding and report generator completed by this worker.
Model-server routing cannot be independently observed from process metadata.</pre><p>Known limits remain: Linux-only filesystem implementation; same-UID namespace-race bounds; sequential recoverable cleanup batches; history is in-memory across the app session; retained Trash data remains recoverable after restart; AI synchronous worker calls enforce mem-ai timeout and are not live streams. No production privilege broadening, sibling-crate edits, owner data cleanup, system changes, push, deployment or public action occurred.</p>
<h2>Captured screenshots</h2>{figures}
<h2>Evidence files</h2><p>{link_html}</p><p>Earlier evidence: <a href="tauri-ui/native-ui-checks.json">debug native flow</a> · <a href="tauri-ui/recovery-release-checks.json">prior release/deb scan</a> · <a href="rust-core/api.html">core contract</a> · <a href="rust-ai/report.html">AI sibling report</a>.</p><footer><p class="muted">Original project DESIGN.md retained. Updated {e(datetime.now(UTC).isoformat())}. Final report generated from local evidence, with previous report preserved once.</p></footer></main></body></html>"""


def main():
    parser = argparse.ArgumentParser()
    parser.add_argument("evidence", type=Path)
    parser.add_argument("report", type=Path)
    args = parser.parse_args()
    commit = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
    assert not subprocess.check_output(["git", "status", "--porcelain"], text=True), (
        "Commit source changes first"
    )
    backup = args.evidence / "report-before-delivery.html"
    if not backup.exists():
        shutil.copy2(args.report, backup)
    content = render(args.evidence, commit)
    temporary = args.report.with_suffix(".delivery-new")
    temporary.write_text(content)
    temporary.replace(args.report)
    print(
        json.dumps(
            {
                "report": str(args.report),
                "sha256": hashlib.sha256(args.report.read_bytes()).hexdigest(),
                "backup_sha256": hashlib.sha256(backup.read_bytes()).hexdigest(),
                "commit": commit,
            },
            indent=2,
        )
    )


if __name__ == "__main__":
    main()
