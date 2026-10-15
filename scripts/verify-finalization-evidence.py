#!/usr/bin/python3
"""Independent read-only verification after the native UI harness has exited."""
import hashlib
import io
import json
import subprocess
import sys
import tarfile
from pathlib import Path


def digest(path):
    return hashlib.sha256(path.read_bytes()).hexdigest()


evidence = Path(sys.argv[1]).resolve()
checks = json.loads((evidence / "delivery-release-checks.json").read_text())
source = json.loads((evidence / "runtime-source.json").read_text())
launch = json.loads((evidence / "delivery-normal-launch.json").read_text())
metadata = json.loads((evidence / "worker-metadata.json").read_text())
cleanup = checks["cleanup"]
fixture = Path(cleanup["fixture"]).resolve()
assert str(fixture).startswith("/tmp/mem-inspector-release-check-")
assert (fixture / "mem-core-fixture-release/.mem-inspector-fixture").is_file()
verified = []
for key, expected in [
    ("restored_path", cleanup["original_sha256"]),
    ("companion_original_path", cleanup["companion_sha256"]),
    ("collision_saved_path", cleanup["collision_sha256"]),
]:
    path = Path(cleanup[key]).resolve()
    assert path.is_relative_to(fixture)
    assert digest(path) == expected
    verified.append({"path": str(path), "bytes": path.stat().st_size, "sha256": expected})
assert not list((fixture / "Trash/files").iterdir())
assert len(list((fixture / "Trash/info").iterdir())) == 2
runtime = Path(checks["runtime"]).parent
binary = runtime / "bin/mem-inspector-desktop"
assert digest(binary) == source["binary_sha256"] == checks["binary_sha256"] == launch["binary_sha256"]
with (evidence / "runtime-integrity.log").open("w") as log:
    subprocess.run(["sha256sum", "-c", "assets.sha256"], cwd=runtime, stdout=log, stderr=subprocess.STDOUT, check=True)
assert str((Path.home() / ".local/share/mem-inspector/current").resolve()) == metadata["original_current"]
assert launch["all_tmp_hidden_except_x11_socket"] and launch["no_scan_no_inference"]
assert checks["advice"]["fake_only"] and checks["advice"]["consent_consumed"] and checks["advice"]["unsafe_startup_rejected"]
assert source["embedded_assets_verified"]
for name, expected in source["asset_sha256"].items():
    assert digest(Path("dist/ui/assets") / name) == expected
# The deb bundle uses Tauri's standard three-byte bundle-type marker.
deb = Path("target/release/bundle/deb/Mem Inspector_0.2.0_amd64.deb").resolve()
member = next(n for n in subprocess.check_output(["ar", "t", str(deb)], text=True).splitlines() if n.startswith("data.tar"))
with tarfile.open(fileobj=io.BytesIO(subprocess.check_output(["ar", "p", str(deb), member]))) as archive:
    packed = archive.extractfile("usr/bin/mem-inspector-desktop").read()
assert packed.replace(b"__TAURI_BUNDLE_TYPE_VAR_DEB", b"__TAURI_BUNDLE_TYPE_VAR_UNK") == binary.read_bytes()
result = {"independent_post_harness_reads": True, "verified_files": verified, "trash_files_empty": True, "retained_trashinfo": 2, "runtime_integrity": True, "current_unchanged": True, "frontend_coherent": True, "deb_matches_except_bundle_marker": True, "deb_binary_sha256": hashlib.sha256(packed).hexdigest(), "native_captain_acceptance": "pending"}
(evidence / "independent-verification.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
