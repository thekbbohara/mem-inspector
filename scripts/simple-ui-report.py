#!/usr/bin/python3
"""Generate the approved-direction finalization report from verified evidence."""
import hashlib
import html
import json
import shutil
import subprocess
import sys
from pathlib import Path

evidence = Path(sys.argv[1]).resolve()
delivery = evidence.parents[1]
assert delivery.name == "T137" and evidence.name == "finalization"
checks = json.loads((evidence / "delivery-release-checks.json").read_text())
launch = json.loads((evidence / "delivery-normal-launch.json").read_text())
metadata = json.loads((evidence / "worker-metadata.json").read_text())
source = json.loads((evidence / "runtime-source.json").read_text())
independent = json.loads((evidence / "independent-verification.json").read_text())
assert independent["frontend_coherent"] and independent["runtime_integrity"]
assert checks["cleanup"]["no_overwrite"] and checks["cleanup"]["multifile_review_exact_locations"]
assert metadata["browser_direction_approved_by_captain"]
assert source["binary_sha256"] == checks["binary_sha256"] == launch["binary_sha256"]
sha = subprocess.check_output(["git", "rev-parse", "HEAD"], text=True).strip()
assert not subprocess.check_output(["git", "status", "--porcelain"], text=True)
report = delivery / "report.html"
previous = delivery / "report-before-approved-dark-finalization.html"
if report.exists() and not previous.exists():
    shutil.copy2(report, previous)
esc = html.escape
images = [
    ("Final native populated results, optimized app and real disposable files", "delivery-release-scan.png"),
    ("Final native two-file cleanup review, exact locations and total", "delivery-release-review.png"),
    ("Native collision refusal, both occupants preserved", "delivery-release-collision.png"),
    ("Native long-filename restore review", "delivery-release-long-restore-review.png"),
]
figures = ""
for caption, name in images:
    path = evidence / name
    assert path.is_file()
    relative = "simple-ui/finalization/" + name
    figures += f'<figure><figcaption>{esc(caption)}</figcaption><a href="{relative}"><img src="{relative}" alt="{esc(caption)}"></a><code>{esc(str(path))}</code></figure>'
commands = json.loads((evidence / "commands.json").read_text())
command_text = "\n".join(commands)
limits = "Only regular unshared files can be selected for cleanup; all Rust protection and approval checks remain authoritative. Distribution covers the current folder and includes directory overhead in Other; capped/cancelled/error scans explicitly show partial totals. UI pages contain at most 100 rows and reviews at most 500 selections; filtering and aggregation are in memory. History/reviews are session-local; OS Trash data remains recoverable after restart. RSS sums shared pages and PSS apportions them; Xvfb results are not a cold-disk or interactive-GPU benchmark. The runtime requires compatible host libraries and bubblewrap."
report.write_text(f'''<!doctype html><html lang="en"><meta charset="utf-8"><meta name="viewport" content="width=device-width,initial-scale=1"><title>T145 native finalization review</title><style>
body{{margin:0;background:#101213;color:#f2f4f1;font:15px system-ui;line-height:1.65}}main{{max-width:1080px;margin:auto;padding:32px}}h1{{font-size:34px;letter-spacing:-1px}}h2{{font-size:21px;margin-top:30px}}p{{color:#a0a8a5}}a,.status{{color:#85dbb5}}code,pre{{font:12px monospace;overflow-wrap:anywhere}}pre{{white-space:pre-wrap;background:#191c1e;padding:18px;border-radius:10px}}figure{{margin:28px 0}}figcaption{{margin-bottom:10px;font-weight:650}}figure code{{display:block;color:#a0a8a5;margin-top:8px}}img{{display:block;width:100%;border-radius:10px}}@media(max-width:600px){{main{{padding:16px}}}}</style><main>
<h1>T145: native finalization review</h1><p class="status">Captain personally approved the pass-2 browser visual direction. Final native acceptance is pending; T145 remains running and is not claimed accepted. AI expansion remains paused.</p>
<p>The approved desktop composition is retained. Targeted changes improve quiet-text contrast, mint graphics contrast, long-name wrapping and deterministic keyboard restore selection. Normal enabled text audits pass 4.5:1; meaningful icons and graphics pass 3:1. Disabled controls use the native disabled/opacity exception. The actual browser contrast audit recorded minimum text 4.565:1 and graphics 3.562:1, including the review.</p>
<p><a href="simple-ui/visual-checkpoint.html">Captain-approved browser checkpoint</a> · <a href="report-before-approved-dark-finalization.html">Preserved prior release report</a>. Browser images use an explicitly injected fixture bridge and do not prove filesystem outcomes. The native screenshots below use the optimized installed binary, production Rust bridge and actual isolated fixture files; no inspector or mock bridge was injected.</p>
<h2>Verified commit and launcher</h2><pre>Local commit: {sha}
Model argument: {esc(metadata['model'])}
CODEX_HOME: {esc(metadata['codex_home'])}
Installed optimized binary SHA256: {source['binary_sha256']}
Exact NEW launch command:
{esc(checks['runtime'])}
Existing current selection preserved:
{esc(metadata['original_current'])}</pre>
<p>The new versioned snapshot copies the previous persistent current/runtime assets. Existing snapshots and current are unchanged. Normal owner-HOME launch ran without scanning or inference and with all /tmp hidden except the isolated X11 socket. Fixture file operations use a separate HOME/root/Trash. Runtime asset hashes, embedded frontend assets and deb code/assets were independently verified; the deb differs only in Tauri's standard bundle marker.</p>
<h2>Checks and commands</h2><pre>{esc(command_text)}</pre>
<p>Five frontend model/bridge tests passed, including zero/partial/u64 distribution and exact four-largest-plus-Other totals. Frontend build, formatting, installer preservation and Python syntax checks passed. Real browser tests cover long names/paths, empty/loading/error/partial/cancelled states, 250 rows over three pages, exact multi-file locations/total, default Cancel focus, Tab/reverse-Tab, Escape and busy dismissal protection, fresh consent and inert text. No live provider calls were made.</p>
<p>Real optimized XTest input selected and reviewed two files before mutation, confirmed both OS Trash moves, reviewed restore, rejected a collision without overwriting either file, preserved the collision occupant, then explicitly restored both originals. Independent reads after the harness exited verify both original hashes, the collision hash, empty Trash/files and two retained recovery metadata files. The same optimized app scanned {checks['large_scan']['entries']:,} entries. Fake-CLI consent and unsafe-startup regression checks run through production mem-ai without network inference.</p>
<pre>{esc(json.dumps(independent['verified_files'],indent=2))}
Idle app tree: RSS {checks['idle']['rss_kib']} KiB / PSS {checks['idle']['pss_kib']} KiB
After scan: RSS {checks['after_scan']['rss_kib']} KiB / PSS {checks['after_scan']['pss_kib']} KiB
Observed scan UI latency: {checks['large_scan']['elapsed_ui_ms']:.0f} ms (includes XTest/clipboard polling)</pre>
<p><a href="simple-ui/finalization/delivery-release-checks.json">Native checks</a> · <a href="simple-ui/finalization/independent-verification.json">Independent bytes/package/runtime verification</a> · <a href="simple-ui/finalization/delivery-normal-launch.json">Normal-HOME launch</a> · <a href="simple-ui/finalization/browser/browser-checks.log">Browser checks</a> · <a href="simple-ui/finalization/browser/contrast-audit.log">Contrast audit</a> · <a href="simple-ui/finalization/tests.log">Tests</a> · <a href="simple-ui/finalization/package.log">Optimized package log</a></p>
<h2>Final native evidence, 1280×800</h2>{figures}
<h2>Limits</h2><p>{esc(limits)}</p></main></html>''')
print(report)
