import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";

const source = await readFile(new URL("../../src/lib/room-check-timeline.ts", import.meta.url), "utf8");
const { outputText } = ts.transpileModule(source, {
  compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
});
const module = new vm.SourceTextModule(outputText, { context: vm.createContext({}) });
await module.link(() => { throw new Error("timeline layout must not require runtime dependencies"); });
await module.evaluate();
const { buildRoomCheckTimeline, roomCheckBucketLabel } = module.namespace;
const now = 1_000_000;
const sample = (checked_at, duration_ms = 178, error = null) => ({ checked_at, duration_ms, error });

test("only genuine recent checks are plotted, independent of when the modal opens", () => {
  const history = [sample(now - 5_000), sample(now - 300_001), sample(now - 300_000), sample(now - 50_000, 940, "timeout")];
  const snapshot = structuredClone(history);
  const layout = buildRoomCheckTimeline(history, now, 900);
  assert.equal(layout.count, 3);
  assert.equal(layout.failures, 1);
  assert.deepEqual(history, snapshot);
  assert.equal(buildRoomCheckTimeline(history, now + 300_000, 900).count, 0);
});

test("dense checks keep all failures and peak latency when grouped for a narrow plot", () => {
  const history = Array.from({ length: 300 }, (_, index) => sample(now - 299_000 + index * 1_000, 100 + index));
  history[50].error = "request failed";
  history[51].error = "retry failed";
  history[51].duration_ms = 12_000;
  const layout = buildRoomCheckTimeline(history, now, 250);
  assert.equal(layout.count, 300);
  assert.equal(layout.failures, 2);
  assert.equal(layout.bars.reduce((count, bar) => count + bar.count, 0), 300);
  assert.equal(layout.bars.reduce((count, bar) => count + bar.failures, 0), 2);
  assert.equal(Math.max(...layout.bars.map(bar => bar.duration_ms)), 12_000);
  const failed = layout.bars.find(bar => bar.failures);
  assert.match(roomCheckBucketLabel(failed), /2 次失败/);
  assert.match(roomCheckBucketLabel(failed), /最高耗时 12000 ms/);
  assert.match(roomCheckBucketLabel(failed), /retry failed/);
  assert.equal(layout.unit, "s");
});

test("bars and hit areas remain within the plot, including time-window boundaries and zero latency", () => {
  for (const width of [210, 280, 700, 950]) {
    const layout = buildRoomCheckTimeline([sample(now - 300_000, 0), sample(now, 1)], now, width);
    for (const bar of layout.bars) {
      assert.ok(Number.isFinite(bar.height) && bar.height > 0);
      assert.ok(bar.x - bar.width / 2 >= layout.left - 0.001);
      assert.ok(bar.x + bar.width / 2 <= layout.right + 0.001);
      assert.ok(bar.hitLeft >= layout.left && bar.hitLeft + bar.hitWidth <= layout.right + 0.001);
      assert.ok(bar.y >= layout.top && bar.y + bar.height === layout.bottom);
    }
  }
});

test("invalid readings cannot corrupt the scale; server clock differences keep the latest check visible", () => {
  const layout = buildRoomCheckTimeline([sample(now, NaN), sample(Infinity), sample(now, -1), sample(now + 10_000, 250, "")], now, 700);
  assert.equal(layout.count, 1);
  assert.equal(layout.failures, 1);
  assert.equal(layout.end, now + 10_000);
  assert.match(roomCheckBucketLabel(layout.bars[0]), /检查失败/);
  const empty = buildRoomCheckTimeline([], now, 700);
  assert.equal(empty.count, 0);
  assert.ok(Number.isFinite(empty.ceiling));
});
