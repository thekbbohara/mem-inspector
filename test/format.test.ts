import { test } from "node:test";
import assert from "node:assert/strict";
import {
  ago,
  clock,
  easeOut,
  hddPath,
  middle,
  selectedBytes,
  size,
  tilde,
  usedRatio,
} from "../src/format";
import { requestNative } from "../src/bridge";
import { present, segments } from "../src/kinds";

test("sizes round like the Rust side", () => {
  assert.equal(size(0), "0 B");
  assert.equal(size(1536), "1.5 KB");
  assert.equal(size(28225 * 1024 * 1024), "28 GB");
  assert.equal(size(9.4 * 1024 ** 3), "9.4 GB");
});
test("home paths shorten only at a path boundary", () => {
  assert.equal(tilde("/home/kb/.cache", "/home/kb"), "~/.cache");
  assert.equal(tilde("/home/kbx/file", "/home/kb"), "/home/kbx/file");
  assert.equal(tilde("/home/kb", "/home/kb"), "~");
});
test("disk ratio and selection totals", () => {
  assert.equal(usedRatio(100, 6), 0.94);
  assert.equal(usedRatio(0, 0), 0);
  const rows = [
    { path: "/a", bytes: 5 },
    { path: "/b", bytes: 7 },
  ];
  assert.equal(selectedBytes(rows, new Set(["/b"])), 7);
  assert.equal(clock(71), "1:11");
});
test("browser preview never fakes the native bridge", async () => {
  await assert.rejects(requestNative({ action: "suggest" }), /desktop app/);
});
test("long paths keep both ends", () => {
  const p =
    "~/work/apps/example-mobile-app/packages/very/deep/folder/node_modules";
  const m = middle(p, 40);
  assert.equal(m.length, 40);
  assert.ok(m.startsWith("~/work") && m.endsWith("node_modules"));
  assert.equal(middle("~/.cache"), "~/.cache");
});
test("scan age reads in plain words", () => {
  const t = 1_000_000;
  assert.equal(ago(t, t + 10_000), "just now");
  assert.equal(ago(t, t + 3 * 60_000), "3 min ago");
  assert.equal(ago(t, t + 2 * 3_600_000), "2 h ago");
  assert.equal(easeOut(0), 0);
  assert.equal(easeOut(1), 1);
  assert.ok(easeOut(0.5) > 0.5, "fast start");
});
test("disk segments keep free space and fixed type order", () => {
  const segs = segments(
    { deps: 50, video: 20, system: 30, caches: 0.01 },
    100,
    100,
  );
  assert.deepEqual(
    segs.map((s) => s.key),
    ["video", "deps", "system"],
  );
  assert.ok(Math.abs(segs.reduce((a, s) => a + s.share, 0) - 0.5) < 0.001);
  // Never more than the used part, even if type totals overshoot.
  const over = segments({ deps: 300 }, 100, 100);
  assert.equal(over[0].share, 0.5);
  assert.deepEqual(present([segs, over]), ["video", "deps", "system"]);
  assert.deepEqual(segments(null, 1, 1), []);
});
test("HDD targets keep the archive root and the end", () => {
  const a = "/mnt/data2/archive";
  assert.equal(
    hddPath("/mnt/data2/archive/home/alex/work/clips", a),
    "/mnt/data2/archive/…/work/clips",
  );
  assert.equal(
    hddPath("/mnt/data2/archive/home/al", a),
    "/mnt/data2/archive/home/al",
  );
  assert.equal(hddPath("/elsewhere/x", a), "/elsewhere/x");
});
