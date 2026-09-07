import assert from "node:assert/strict";
import { Blob, File } from "node:buffer";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";
import { IDBFactory, IDBObjectStore } from "fake-indexeddb";

async function loadModule(name, globals = {}, fileSystem = {}) {
  const context = vm.createContext({ Blob, File, structuredClone, ...globals });
  const modules = new Map();
  const fs = new vm.SyntheticModule(["stat", "readFile"], function () {
    this.setExport("stat", fileSystem.stat);
    this.setExport("readFile", fileSystem.readFile);
  }, { context });
  await fs.link(() => {});
  await fs.evaluate();

  async function create(name) {
    if (modules.has(name)) return modules.get(name);
    const source = await readFile(new URL(`../../src/lib/agent/${name}.ts`, import.meta.url), "utf8");
    const { outputText } = ts.transpileModule(source, {
      compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
    });
    const module = new vm.SourceTextModule(outputText, {
      context,
      importModuleDynamically: async () => fs,
    });
    modules.set(name, module);
    await module.link((specifier) => create(specifier.replace("./", "")));
    return module;
  }

  const module = await create(name);
  await module.evaluate();
  return module.namespace;
}

class FileReader {
  readAsDataURL(blob) {
    blob.arrayBuffer().then((data) => {
      this.result = `data:${blob.type};base64,${Buffer.from(data).toString("base64")}`;
      this.onload();
    });
  }
}

test("unsupported images are rejected by picker/paste before reading bytes", async () => {
  const api = await loadModule("attachments");
  const files = ["svg", "heic", "heif", "bmp", "avif"].flatMap((ext) => [
    { name: `image.${ext}`, type: "", size: 100 },
    { name: "renamed.png", type: `image/${ext}`, size: 100 },
  ]);
  const result = await api.loadAttachmentsFromFiles(files);
  assert.equal(result.attachments.length, 0);
  assert.equal(result.errors.length, files.length);
  assert.ok(!api.FILE_ACCEPT.includes("image/*"));
});

test("supported images preserve bytes and MIME, including JPG alias and unknown file MIME", async () => {
  const api = await loadModule("attachments", { FileReader });
  for (const [ext, inputType, expected] of [
    ["png", "image/png", "image/png"],
    ["jpg", "image/jpg", "image/jpeg"],
    ["jpeg", "", "image/jpeg"],
    ["png", "application/octet-stream", "image/png"],
    ["gif", "image/gif", "image/gif"],
    ["webp", "image/webp", "image/webp"],
  ]) {
    const result = await api.loadAttachmentsFromFiles([new File(["bytes"], `image.${ext}`, { type: inputType })]);
    assert.equal(result.errors.length, 0);
    assert.equal(result.attachments[0].mimeType, expected);
    assert.equal(result.attachments[0].data, Buffer.from("bytes").toString("base64"));
  }
});

test("desktop drops reject unsupported paths without metadata or file reads", async () => {
  const api = await loadModule("attachments", {}, {
    stat: () => assert.fail("must reject before stat"),
    readFile: () => assert.fail("must reject before readFile"),
  });
  const result = await api.loadAttachmentsFromPaths(["/huge.mp4", "/image.svg", "/image.bmp"]);
  assert.equal(result.errors.length, 3);
});

test("compression uses the actual MIME when the browser falls back to PNG", async () => {
  const revoked = [];
  const api = await loadModule("attachments", {
    FileReader,
    URL: { createObjectURL: () => "blob:test", revokeObjectURL: (url) => revoked.push(url) },
    Image: class {
      width = 2400;
      height = 1200;
      set src(_) { this.onload(); }
    },
    document: { createElement: () => ({
      getContext: () => ({ drawImage() {} }),
      toDataURL: (mime) => {
        assert.equal(mime, "image/webp");
        return "data:image/png;base64,cG5n";
      },
    }) },
  });
  const result = await api.loadAttachmentsFromFiles([
    new File([new Uint8Array(600 * 1024)], "image.webp", { type: "image/webp" }),
  ]);
  assert.equal(result.errors.length, 0);
  assert.equal(result.attachments[0].mimeType, "image/png");
  assert.equal(result.attachments[0].data, "cG5n");
  assert.deepEqual(revoked, ["blob:test"]);
});

test("desktop drops reject oversized files and directories before reading", async () => {
  const api = await loadModule("attachments", {}, {
    stat: async (path) => ({ isFile: !path.includes("directory"), size: 9 * 1024 * 1024 }),
    readFile: () => assert.fail("must reject before readFile"),
  });
  const result = await api.loadAttachmentsFromPaths(["/large.png", "/large.txt", "/directory.txt"]);
  assert.equal(result.errors.length, 3);
  assert.match(result.errors[0].message, /8MB/);
  assert.match(result.errors[1].message, /256KB/);
});

test("desktop drops read valid text after stat and recheck size if the file grew", async () => {
  const calls = [];
  const api = await loadModule("attachments", {}, {
    stat: async (path) => { calls.push(`stat:${path}`); return { isFile: true, size: 5 }; },
    readFile: async (path) => {
      calls.push(`read:${path}`);
      return path.includes("grew") ? new Uint8Array(256 * 1024 + 1) : new TextEncoder().encode("hello");
    },
  });
  const result = await api.loadAttachmentsFromPaths(["/valid.txt", "/grew.txt"]);
  assert.equal(result.attachments[0].text, "hello");
  assert.equal(result.errors.length, 1);
  for (const path of ["/valid.txt", "/grew.txt"]) {
    assert.ok(calls.indexOf(`stat:${path}`) < calls.indexOf(`read:${path}`));
  }
});

function human(attachments = []) {
  return { kind: "human", content: "hello", timestamp: "2026-09-07T00:00:00.000Z", attachments };
}

async function storage(legacy) {
  const values = new Map(legacy === undefined ? [] : [["messages", JSON.stringify(legacy)]]);
  const indexedDB = new IDBFactory();
  const localStorage = {
    getItem: (key) => values.get(key) ?? null,
    removeItem: (key) => values.delete(key),
    setItem: () => assert.fail("conversation payloads must never be written to localStorage"),
  };
  const api = await loadModule("conversation-store", { indexedDB, localStorage });
  return { api, values, indexedDB, localStorage };
}

test("large image and text attachments survive save and reload outside localStorage", async () => {
  const { api, indexedDB, localStorage } = await storage();
  const messages = [human([
    { kind: "image", name: "large.gif", mimeType: "image/gif", data: "A".repeat(Math.ceil(8 * 1024 * 1024 / 3) * 4) },
    { kind: "text", name: "notes.txt", mimeType: "text/plain", text: "notes" },
  ])];
  await api.saveConversation(messages);
  const reloaded = await loadModule("conversation-store", { indexedDB, localStorage });
  assert.equal(JSON.stringify(await reloaded.loadConversation()), JSON.stringify(messages));
});

test("legacy conversation migrates once and only disappears after commit", async () => {
  const legacy = [human()];
  const { api, values } = await storage(legacy);
  assert.equal(JSON.stringify(await api.loadConversation()), JSON.stringify(legacy));
  assert.equal(values.has("messages"), false);
  await api.saveConversation([]);
  assert.equal((await api.loadConversation()).length, 0);
});

test("aborted migration keeps legacy history and permits retry", async () => {
  const { api, values } = await storage([human()]);
  const original = IDBObjectStore.prototype.put;
  IDBObjectStore.prototype.put = function (...args) {
    const request = original.apply(this, args);
    this.transaction.abort();
    return request;
  };
  try {
    await assert.rejects(api.loadConversation());
    assert.ok(values.has("messages"));
  } finally {
    IDBObjectStore.prototype.put = original;
  }
  assert.equal((await api.loadConversation()).length, 1);
  assert.equal(values.has("messages"), false);
});

test("queued save, clear and reload preserve order and capture a stable snapshot", async () => {
  const { api } = await storage();
  const messages = [human()];
  const save = api.saveConversation(messages);
  messages[0].content = "mutated after save";
  const loaded = api.loadConversation();
  const clear = api.saveConversation([]);
  const cleared = api.loadConversation();
  await Promise.all([save, clear]);
  assert.equal((await loaded)[0].content, "hello");
  assert.equal((await cleared).length, 0);
});

test("failed save preserves committed history and the queue recovers", async () => {
  const { api } = await storage();
  await api.saveConversation([human()]);
  const original = IDBObjectStore.prototype.put;
  IDBObjectStore.prototype.put = function (...args) {
    const request = original.apply(this, args);
    this.transaction.abort();
    return request;
  };
  try {
    await assert.rejects(api.saveConversation([]));
  } finally {
    IDBObjectStore.prototype.put = original;
  }
  assert.equal((await api.loadConversation()).length, 1);
  await api.saveConversation([]);
  assert.equal((await api.loadConversation()).length, 0);
});
