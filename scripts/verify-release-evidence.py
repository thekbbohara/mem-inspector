#!/usr/bin/python3
"""Independent post-run filesystem verification, without launching the app."""

import hashlib
import json
import sys
from pathlib import Path

evidence = Path(sys.argv[1])
record = json.loads((evidence / "delivery-release-checks.json").read_text())
cleanup = record["cleanup"]
sha = lambda path: hashlib.sha256(Path(path).read_bytes()).hexdigest()
assert (
    sha(cleanup["restored_path"])
    == cleanup["original_sha256"]
    == cleanup["trashed_sha256"]
)
assert sha(cleanup["collision_saved_path"]) == cleanup["collision_sha256"]
assert not Path(cleanup["trashed_path"]).exists()
assert (
    cleanup["no_overwrite"]
    and "File exists (os error 17)" in cleanup["collision_error"]
)
assert record["large_scan"]["entries"] >= 10000
assert record["advice"]["fake_only"] and record["advice"]["consent_consumed"]
assert record["advice"]["unsafe_startup_rejected"]
assert len(record["advice"]["records"]) == 2
for state in ["idle", "after_scan"]:
    sample = record[state]
    assert sample["process_count"] >= 3
    assert sample["rss_kib"] == sum(p["rss_kib"] for p in sample["processes"])
    assert sample["pss_kib"] == sum(p["pss_kib"] for p in sample["processes"])
    assert all(p["pss_kib"] > 0 for p in sample["processes"])
result = {
    "verification": "Independent filesystem reads after the UI harness exited",
    "restored_sha256": sha(cleanup["restored_path"]),
    "original_sha256": cleanup["original_sha256"],
    "collision_occupant_sha256": sha(cleanup["collision_saved_path"]),
    "byte_equality": True,
    "collision_preserved": True,
    "trash_file_absent_after_restore": True,
    "memory_totals_consistent": True,
}
(evidence / "delivery-independent-bytes.json").write_text(json.dumps(result, indent=2))
print(json.dumps(result, indent=2))
