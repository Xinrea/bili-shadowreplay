import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import test from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { compile } from "svelte/compiler";
import { render } from "svelte/server";
import { recordingStorage } from "./recording-storage-fixture.mjs";

const { archiveFilesAvailable, archiveStorageReason } = recordingStorage;
const read = path => readFile(new URL(`../../src/${path}`, import.meta.url), "utf8");
const row = (live_id, storage_available = true) => ({
  live_id, platform: "bilibili", room_id: "room", parent_id: "parent",
  title: `Recording ${live_id}`, length: 60, size: 1024, created_at: "2026-01-01",
  storage_backend_id: `disk-${live_id}`, storage_available,
});
const plain = value => JSON.parse(JSON.stringify(value));
async function handlers(path, state) {
  state = {
    storageLoading: false, storageError: "",
    getStorageInitializationStatus: async () => ({ loading: false, error: null }),
    ...state,
  };
  const source = await read(path);
  const script = source.match(/<script lang="ts">([\s\S]*?)<\/script>/)[1];
  const parsed = ts.createSourceFile("component.ts", script, ts.ScriptTarget.Latest, true);
  const functions = parsed.statements.filter(ts.isFunctionDeclaration).map(node => node.getText(parsed)).join("\n");
  vm.runInContext(ts.transpileModule(functions, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  }).outputText, vm.createContext(Object.assign(state, recordingStorage)));
  return state;
}

test("availability is pinned to each archive and only explicit false blocks legacy records", () => {
  assert.equal(archiveFilesAvailable(row("old", false)), false);
  assert.equal(archiveFilesAvailable(row("new")), true);
  assert.equal(archiveFilesAvailable({ storage_backend_id: "removed-but-legacy-response" }), true);
  assert.equal(archiveFilesAvailable({ storage_available: null }), true);
  assert.equal(archiveFilesAvailable(null), false);
  assert.match(archiveStorageReason(row("old", false)), /disk-old/);
  assert.equal(archiveStorageReason({ storage_unavailable_reason: "Unsupported S3 backend" }), "Unsupported S3 backend");
});

test("storage badges show only the identifier unless storage is unavailable", async () => {
  const source = await read("lib/components/ArchiveStorageBadge.svelte");
  const module = new vm.SourceTextModule(compile(source, { generate: "server" }).js.code);
  await module.link(async name => {
    const exports = name === "../recording-storage" ? recordingStorage : await import(name);
    return new vm.SyntheticModule(Object.keys(exports), function () {
      for (const [key, value] of Object.entries(exports)) this.setExport(key, value);
    });
  });
  await module.evaluate();
  const show = (archive, props = {}) => render(module.namespace.default, { props: { archive, ...props } }).body;
  assert.match(show(row("old", false)), /存储不可用 · 仅记录/);
  assert.match(show(row("old", false)), /disk-old/);
  assert.match(show(row("new")), /存储：disk-new/);
  assert.doesNotMatch(show(row("new")), /已配置|未检测文件/);
  assert.doesNotMatch(show(row("new")), /存储不可用/);
  assert.match(show(row("new"), { showLabel: false }), /disk-new/);
  assert.doesNotMatch(show(row("new"), { showLabel: false }), /存储：/);
  assert.match(show(row("old", false), { showLabel: false }), /存储不可用/);
  assert.match(show({}, { showLabel: false }), /—/);
});

test("recent cards place the identifier before size and archive tables give storage its own column", async () => {
  const summary = await read("page/Summary.svelte");
  const metadata = summary.match(/<p class="mt-1 text-\[11px\] text-fg-muted">([\s\S]*?)<\/p>/)[1];
  assert.match(metadata, /format_date[\s\S]*ArchiveStorageBadge archive=\{record\} showLabel=\{false\}[\s\S]*formatSize\(record.size\)/);
  assert.equal((summary.match(/<ArchiveStorageBadge/g) || []).length, 1);
  const list = await read("lib/components/ArchiveList.svelte");
  assert.match(list, /<th[^>]*>存储<\/th>\s*<th[^>]*>大小<\/th>/);
  const cell = list.match(/<td[^>]*>\s*<ArchiveStorageBadge \{archive\} showLabel=\{false\} \/>\s*<\/td>/);
  assert.ok(cell, "the identifier/status belongs to its own table cell, including compact mode");
});

test("single preview/delete and mixed batch delete never invoke file operations for missing storage", async () => {
  const calls = [];
  const records = [row("available"), row("missing", false)];
  const state = await handlers("lib/components/ArchiveList.svelte", {
    archives: records, selectedArchives: new Set(records.map(item => item.live_id)),
    actionError: "", invoke: async (...args) => calls.push(args),
  });
  await state.playArchive(records[1]);
  assert.match(state.actionError, /disk-missing/);
  await assert.rejects(state.deleteArchive(records[1]), /disk-missing/);
  await assert.rejects(state.deleteSelectedArchives(), /disk-missing/);
  assert.equal(calls.length, 0, "preflight the entire batch before deleting even its available first item");
  assert.equal(state.selectedArchives.size, 2);
  await state.playArchive(records[0]);
  assert.equal(calls[0][0], "open_live");
});

test("whole-clip groups retain missing rows but select and submit only available archives", async () => {
  const rows = [row("one"), row("two", false), row("three", undefined)];
  // Explicitly simulate an older server response without the flag.
  delete rows[2].storage_available;
  const requests = [];
  const state = await handlers("lib/components/GenerateWholeClipModal.svelte", {
    archive: rows[0], wholeClipArchives: [], selectedLiveIds: [],
    loadSeq: 0, isLoading: false, isSubmitting: false, generationError: "", outputName: "",
    encodeDanmu: false, reencode: false, showModal: true, onGenerated() {},
    get_static_url: async () => "/cover", console,
    invoke: async (command, args) => {
      if (command === "get_archives_by_parent_id") return rows.map(row => ({ ...row }));
      requests.push({ command, args: plain(args) });
    },
  });
  await state.loadWholeClipArchives("room", "parent");
  assert.equal(state.wholeClipArchives.length, 3);
  assert.deepEqual(plain(state.selectedLiveIds), ["one", "three"]);
  state.selectAll();
  assert.deepEqual(plain(state.selectedLiveIds), ["one", "three"]);
  state.toggleArchiveSelection("two", true);
  assert.deepEqual(plain(state.selectedLiveIds), ["one", "three"]);
  state.selectedLiveIds = ["one", "two"];
  await state.generateWholeClip();
  assert.equal(requests.length, 0);
  assert.match(state.generationError, /存储不可用/);
  state.selectAll();
  await state.generateWholeClip();
  assert.deepEqual(requests[0].args.selectedLiveIds, ["one", "three"]);
  state.selectedLiveIds = ["three", "one"];
  state.outputName = "custom.mp4";
  rows[2].storage_available = false;
  await state.loadWholeClipArchives("room", "parent", true);
  assert.deepEqual(plain(state.selectedLiveIds), ["one"]);
  assert.equal(state.outputName, "custom.mp4");
  assert.match(state.generationError, /部分已选片段/);
});

test("saved summaries/subtitles remain readable while generation and highlight clips are blocked", async () => {
  const requests = [];
  const archive = row("missing", false);
  const state = await handlers("lib/components/ArchiveSummaryModal.svelte", {
    archive, actionLoading: false, loading: false, error: "", summary: null, pollTimer: null,
    invoke: async (command) => {
      requests.push(command);
      return { status: "success", summary_markdown: "Saved summary", subtitle_srt: "Saved subtitle" };
    },
    onUpdated() {},
  });
  await state.loadSummary();
  assert.equal(state.summary.summary_markdown, "Saved summary");
  assert.equal(state.summary.subtitle_srt, "Saved subtitle");
  await state.generate(false);
  await state.generate(true);
  assert.deepEqual(requests, ["get_archive_summary"]);
  const clip = await handlers("lib/components/ArchiveClipButton.svelte", {
    archive, disabled: false, activeRanges: [{ start: 0, end: 10 }], showConfirm: false,
  });
  clip.openConfirm();
  await clip.generateClip();
  assert.equal(clip.showConfirm, false);
});

test("direct preview waits for archive metadata and hides every file surface on explicit unavailability", async () => {
  const source = await read("AppLive.svelte");
  const unavailableBranch = source.indexOf("{:else if !archiveFilesAvailable(archive)}");
  const workspace = source.indexOf('<div class="preview-workspace">');
  assert.ok(unavailableBranch > 0 && workspace > unavailableBranch);
  assert.match(source.slice(unavailableBranch, workspace), /\{:else\}/);
  for (const component of ["<Player", "<ArchiveTimeline", "<ArchiveInspector"]) {
    assert.ok(source.indexOf(component) > workspace);
  }
  const state = await handlers("AppLive.svelte", {
    archive: row("initial"), archiveError: "", disposed: false, room_id: "room", live_id: "missing",
    invoke: async () => row("missing", false), set_title() {},
  });
  await state.refreshArchive();
  assert.equal(archiveFilesAvailable(state.archive), false);
  state.invoke = async () => { throw new Error("offline"); };
  await state.refreshArchive();
  assert.equal(state.archive, null, "do not keep a stale player mounted after a failed metadata reload");
  assert.match(state.archiveError, /offline/);
});

test("dashboard refresh updates old rows after backend removal even when newest live ID is unchanged", async () => {
  const state = await handlers("page/Summary.svelte", {
    summary: {}, total: 0, online: 0, disk_usage: 0, account_count: 0,
    total_length: 0, today_record_count: 0, offset: 2, hasMore: false,
    recent_records: [row("latest"), row("old")],
    get_static_url: async () => "/cover",
    invoke: async command => {
      assert.notEqual(command, "get_disk_info", "host telemetry must not block the summary");
      if (command === "get_recorder_list") return { count: 0, recorders: [] };
      if (command === "get_recent_record") return [row("latest"), row("old", false)];
      return 0;
    },
  });
  await state.update_summary();
  assert.equal(state.recent_records[1].storage_available, false);
  assert.equal(state.recent_records[1].cover, "");
  assert.equal(state.recent_records[0].live_id, "latest");
});

test("dashboard refresh reopens exhausted pagination when new rows displace old ones", async () => {
  const rows = [row("new"), row("latest"), row("old")];
  const state = await handlers("page/Summary.svelte", {
    summary: {}, total: 0, online: 0, disk_usage: 0, account_count: 0,
    total_length: 0, today_record_count: 0, offset: 2, hasMore: false,
    hasNewRecords: false, loading: false, RECORDS_PER_PAGE: 5,
    recent_records: [row("latest"), row("old")],
    get_static_url: async () => "/cover",
    invoke: async (command, args) => {
      if (command === "get_recorder_list") return { count: 0, recorders: [] };
      if (command === "get_recent_record") return rows.slice(args.offset, args.offset + args.limit);
      return 0;
    },
  });
  await state.update_summary();
  assert.equal(state.hasMore, true);
  assert.equal(state.offset, 2);
  assert.deepEqual(plain(state.recent_records.map(item => item.live_id)), ["new", "latest"]);
  await state.loadMoreRecords();
  assert.deepEqual(plain(state.recent_records.map(item => item.live_id)), ["new", "latest", "old"]);
  assert.equal(state.hasMore, false);

  rows.length = 0;
  await state.update_summary();
  assert.equal(state.recent_records.length, 0);
  rows.push(row("after-empty"));
  await state.update_summary();
  assert.equal(state.recent_records[0].live_id, "after-empty");
});

test("dashboard leaves recording sections loading while local room/account data is available", async () => {
  const requests = [];
  let initializing = true;
  const state = await handlers("page/Summary.svelte", {
    summary: {}, total: 0, online: 0, disk_usage: 0, account_count: 0,
    total_length: 0, today_record_count: 0, offset: 0, hasMore: true,
    hasNewRecords: false, loading: false, RECORDS_PER_PAGE: 5, recent_records: [],
    getStorageInitializationStatus: async () => ({ loading: initializing, error: null }),
    get_static_url: async () => "/cover",
    invoke: async command => {
      requests.push(command);
      if (command === "get_recorder_list") return { count: 2, recorders: [{ room_info: { status: true } }] };
      if (command === "get_account_count") return 3;
      if (command === "get_recent_record") return [row("ready")];
      return 60;
    },
  });
  await state.update_summary();
  await state.loadMoreRecords();
  assert.equal(state.storageLoading, true);
  assert.equal(state.total, 2);
  assert.equal(state.account_count, 3);
  assert.deepEqual(requests, ["get_recorder_list", "get_account_count"]);

  initializing = false;
  await state.update_summary();
  assert.equal(state.storageLoading, false);
  assert.equal(state.recent_records[0].live_id, "ready");
  assert.equal(state.disk_usage, 60);
});

test("a failed dashboard record fetch releases loading and permits the next refresh", async () => {
  const state = await handlers("page/Summary.svelte", {
    loading: false, hasMore: true, hasNewRecords: false, offset: 0,
    RECORDS_PER_PAGE: 5, recent_records: [],
    invoke: async () => { throw new Error("offline"); },
    get_static_url: async () => "/cover",
  });
  await assert.rejects(state.loadMoreRecords(), /offline/);
  assert.equal(state.loading, false);
  state.invoke = async () => [row("retry")];
  await state.loadMoreRecords();
  assert.equal(state.loading, false);
  assert.equal(state.recent_records[0].live_id, "retry");
});

for (const [path, refresh] of [
  ["page/Archive.svelte", "fetchArchives"],
  ["lib/components/RoomArchiveModal.svelte", "refreshArchives"],
]) {
  test(`${path} waits for initialization, refreshes when ready, and settles errors`, async () => {
    let status = { loading: true, error: null };
    let archiveReads = 0;
    let release;
    const state = await handlers(path, {
      loading: true, loadError: "", disposed: false, archives: [], rooms: [],
      room: { room_info: { platform: "bilibili", room_id: "room" } },
      getStorageInitializationStatus: async () => status,
      invoke: async () => ({ recorders: [{ room_info: {} }] }),
      loadRoomArchives: async () => {
        archiveReads++;
        return await new Promise(resolve => { release = resolve; });
      },
    });
    await state[refresh]();
    assert.equal(state.loading, true, "loading must outlive the fast local status request");
    assert.equal(archiveReads, 0);
    assert.equal(state.archives.length, 0);

    status = { loading: false, error: null };
    const pending = state[refresh]();
    for (let i = 0; i < 10; i++) await Promise.resolve();
    assert.equal(state.loading, true, "keep loading through the actual archive snapshot");
    release([row("loaded")]);
    await pending;
    assert.equal(state.loading, false);
    assert.equal(state.archives[0].live_id, "loaded");

    status = { loading: false, error: "catalog unavailable" };
    await state[refresh]();
    assert.equal(state.loading, false);
    assert.match(state.loadError, /catalog unavailable/);
    assert.equal(archiveReads, 1);

    state.getStorageInitializationStatus = async () => { throw new Error("connection lost"); };
    await state[refresh]();
    assert.equal(state.loading, false);
    assert.match(state.loadError, /connection lost/);
  });
}
