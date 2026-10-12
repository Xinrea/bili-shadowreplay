import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

class Element {
  constructor(tagName) {
    this.tagName = tagName;
    this.style = {};
    this.children = [];
    this.listeners = new Map();
  }
  set textContent(value) {
    this.text = value;
    this.children = [];
  }
  get textContent() {
    return this.text;
  }
  append(...nodes) {
    this.children.push(...nodes);
    for (const node of nodes) node.parentElement = this;
  }
  appendChild(node) {
    this.append(node);
    return node;
  }
  remove() {
    if (!this.parentElement) return;
    this.parentElement.children = this.parentElement.children.filter((node) => node !== this);
    this.parentElement = null;
  }
  removeAttribute(name) {
    if (name === "src") this.listenersAtSourceRemoval = this.listeners.size;
    delete this[name];
  }
  replaceChildren(...nodes) {
    this.children = nodes;
    this.text = "";
  }
  addEventListener(type, listener) {
    this.listeners.set(type, listener);
  }
  removeEventListener(type, listener) {
    if (this.listeners.get(type) === listener) this.listeners.delete(type);
  }
  dispatch(type, event = {}) {
    this.listeners.get(type)?.({ target: this, propertyName: "transform", ...event });
  }
}

async function loadModule(name = "danmu-content", globals = {}) {
  const source = await readFile(
    new URL(`../../src/lib/${name}.ts`, import.meta.url), "utf8",
  );
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: {
      target: ts.ScriptTarget.ES2022,
      module: ts.ModuleKind.ESNext,
    },
  });
  const context = vm.createContext({
    URL,
    document: { createElement: (tagName) => new Element(tagName) },
    ...globals,
  });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {
    throw new Error(`${name}.ts must have no runtime dependencies`);
  });
  await module.evaluate();
  return module.namespace;
}

const emoji = (metadata = {}) => ({
  ts: 0,
  type: "danmu",
  content: "[表情]",
  emoji_content: { url: "https://i0.hdslb.com/bfs/test.webp", ...metadata },
});

test("only image danmaku with trusted CDN URLs get an image presentation", async () => {
  const api = await loadModule();
  for (const url of [
    "", " ", "not a URL", "/local.png", "file:///private/image.png",
    "javascript:alert(1)", "data:image/svg+xml,test", "tauri://localhost/image",
    "https://user:password@example.com/image.png",
    "http://localhost/image.png", "https://127.0.0.1/image.png",
    "http://192.168.0.1/image.png", "http://169.254.169.254/image.png",
    "https://[::1]/image.png", "https://example.com/image.png",
    "https://hdslb.com.evil.example/image.png", "https://fakehdslb.com/image.png",
    "https://i0.hdslb.com:8443/image.png", "https://user@i0.hdslb.com/image.png",
  ]) {
    assert.equal(api.emojiPresentation(emoji({ url })), null, url);
  }
  assert.equal(api.emojiPresentation({ type: "danmu", content: "plain" }), null);
  assert.equal(api.emojiPresentation({ ...emoji(), type: "super_chat" }), null);
  assert.equal(api.emojiPresentation(emoji({
    url: "//i0.hdslb.com/bfs/image.png",
  })).url, "https://i0.hdslb.com/bfs/image.png");
  assert.equal(api.emojiPresentation(emoji({
    url: "http://i0.hdslb.com/bfs/image.png",
  })).url, "https://i0.hdslb.com/bfs/image.png");
  assert.equal(api.emojiPresentation(emoji({
    url: "https://i0.biliimg.com/bfs/image.png",
  })).url, "https://i0.biliimg.com/bfs/image.png");
});

test("image footprint fits row height and bounds extreme or missing dimensions", async () => {
  const api = await loadModule();
  assert.equal(api.emojiPresentation(emoji()).widthEm, 1.5);
  assert.equal(api.emojiPresentation(emoji({ width: 200, height: 100 })).widthEm, 3);
  assert.equal(api.emojiPresentation(emoji({ width: 100000, height: 1 })).widthEm, 4.5);
  assert.equal(api.emojiPresentation(emoji({ width: 1, height: 100000 })).widthEm, 0.375);
  for (const width of [0, -1, NaN, Infinity]) {
    assert.equal(api.emojiPresentation(emoji({ width, height: 100 })).widthEm, 1.5);
  }
  assert.equal(api.emojiPresentation(emoji()).heightEm, 1.5);
});

test("text is rendered literally, not as HTML", async () => {
  const api = await loadModule();
  const node = new Element("p");
  const content = '<img src="x" onerror="alert(1)">';
  api.renderDanmuContent(node, { type: "danmu", content });
  assert.equal(node.textContent, content);
  assert.equal(node.children.length, 0);
});

test("image load preserves its footprint and failure restores readable text", async () => {
  const api = await loadModule();
  const node = new Element("p");
  const dispose = api.renderDanmuContent(node, emoji({ width: 200, height: 100 }));
  const [box] = node.children;
  const [fallback, image] = box.children;
  assert.equal(box.style.width, "3em");
  assert.equal(box.style.height, "1.5em");
  assert.equal(box.title, "[表情]");
  assert.equal(fallback.textContent, "[表情]");
  assert.equal(image.alt, "[表情]");
  assert.equal(image.referrerPolicy, "no-referrer");
  assert.equal(image.draggable, false);
  assert.equal(image.style.objectFit, "contain");
  assert.equal(image.style.visibility, "hidden");
  assert.equal(image.src, "https://i0.hdslb.com/bfs/test.webp");
  image.dispatch("load");
  assert.equal(image.style.visibility, "visible");
  assert.equal(fallback.style.visibility, "hidden");
  assert.equal(box.style.width, "3em");
  assert.equal(box.style.height, "1.5em");
  image.dispatch("error");
  assert.equal(node.textContent, "[表情]");
  assert.equal(node.children.length, 0);
  assert.equal(image.listeners.size, 0);
  assert.equal(image.src, undefined);
  assert.equal(image.listenersAtSourceRemoval, 0);
  dispose();
  assert.equal(image.listeners.size, 0);
  assert.equal(node.textContent, "[表情]");
});

test("virtual row updates and destruction revoke pending image sources", async () => {
  const api = await loadModule();
  const node = new Element("span");
  const action = api.danmuContent(node, emoji());
  const oldImage = node.children[0].children[1];
  action.update({ type: "danmu", content: "普通弹幕" });
  assert.equal(oldImage.listeners.size, 0);
  assert.equal(oldImage.src, undefined);
  assert.equal(oldImage.listenersAtSourceRemoval, 0);
  assert.equal(node.textContent, "普通弹幕");
  assert.equal(node.children.length, 0);
  action.update(emoji({ url: "https://i0.hdslb.com/new.gif" }));
  const newImage = node.children[0].children[1];
  assert.equal(newImage.src, "https://i0.hdslb.com/new.gif");
  action.destroy();
  assert.equal(newImage.listeners.size, 0);
  assert.equal(newImage.src, undefined);
  assert.equal(newImage.listenersAtSourceRemoval, 0);
  action.destroy();
  assert.equal(newImage.src, undefined);
});

async function floatingHarness() {
  const frames = new Map();
  let frameId = 0;
  const api = await loadModule("floating-overlay", {
    requestAnimationFrame(callback) {
      frames.set(++frameId, callback);
      return frameId;
    },
    cancelAnimationFrame(id) {
      frames.delete(id);
    },
  });
  const content = await loadModule();
  const parent = new Element("div");
  const overlay = new Element("div");
  overlay.clientWidth = 800;
  parent.append(overlay);
  return {
    owner: api.createFloatingOverlay(overlay),
    content,
    overlay,
    parent,
    frames,
    flushFrames() {
      for (const [id, callback] of [...frames]) {
        frames.delete(id);
        callback();
      }
    },
    addEmoji() {
      const node = new Element("p");
      node.clientWidth = 30;
      const dispose = content.renderDanmuContent(node, emoji());
      const image = node.children[0].children[1];
      this.owner.add(node, dispose);
      return { node, image };
    },
  };
}

test("floating completion releases content and ignores unrelated transitions", async () => {
  const h = await floatingHarness();
  const { node, image } = h.addEmoji();
  h.flushFrames();
  assert.equal(node.style.transform, "translateX(-830px)");
  node.dispatch("transitionend", { target: image });
  node.dispatch("transitionend", { propertyName: "opacity" });
  assert.equal(h.overlay.children.length, 1);
  node.dispatch("transitionend");
  assert.equal(image.src, undefined);
  assert.equal(image.listeners.size, 0);
  assert.equal(node.listeners.size, 0);
  assert.equal(h.overlay.children.length, 0);
  h.owner.destroy();
});

test("clearing disabled danmaku revokes sources and cancels pending frames", async () => {
  const h = await floatingHarness();
  const first = h.addEmoji();
  const second = h.addEmoji();
  const card = new Element("div");
  h.owner.add(card);
  assert.equal(h.frames.size, 3);
  h.owner.clear();
  assert.equal(h.frames.size, 0);
  assert.equal(h.overlay.children.length, 0);
  assert.equal(h.parent.children.length, 1);
  for (const { node, image } of [first, second]) {
    assert.equal(image.src, undefined);
    assert.equal(image.listeners.size, 0);
    assert.equal(node.listeners.size, 0);
  }
  h.flushFrames();
  assert.equal(first.node.style.transform, undefined);
  // The owner is reusable when danmaku is enabled again.
  const next = h.addEmoji();
  assert.equal(h.overlay.children.length, 1);
  assert.ok(next.image.src);
  h.owner.destroy();
  assert.equal(next.image.src, undefined);
});

test("player destruction releases floating images without transitionend", async () => {
  const h = await floatingHarness();
  const { node, image } = h.addEmoji();
  const queuedFrame = [...h.frames.values()][0];
  h.owner.destroy();
  assert.equal(h.frames.size, 0);
  assert.equal(h.parent.children.length, 0);
  assert.equal(h.overlay.children.length, 0);
  assert.equal(image.src, undefined);
  assert.equal(image.listeners.size, 0);
  assert.equal(node.listeners.size, 0);
  // Even an already-dispatched callback cannot animate a disposed node.
  queuedFrame();
  assert.equal(node.style.transform, undefined);
  h.owner.destroy();
  const late = h.addEmoji();
  assert.equal(late.image.src, undefined);
  assert.equal(h.overlay.children.length, 0);
  assert.equal(h.frames.size, 0);
});

test("cancelled transitions dispose each floating node only once", async () => {
  const h = await floatingHarness();
  const node = new Element("p");
  let disposed = 0;
  const release = h.owner.add(node, () => disposed++);
  node.dispatch("transitioncancel");
  assert.equal(disposed, 1);
  assert.equal(h.frames.size, 0);
  assert.equal(node.listeners.size, 0);
  release();
  h.owner.clear();
  h.owner.destroy();
  assert.equal(disposed, 1);
});
