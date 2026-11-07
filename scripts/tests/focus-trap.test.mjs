import assert from "node:assert/strict";
import { readFile } from "node:fs/promises";
import { test } from "node:test";
import vm from "node:vm";
import ts from "typescript";

/** Just enough of the DOM for focus-trap.ts: a tree of focusable elements. */
function fakeDom() {
  const listeners = new Set();
  const document = { activeElement: null, body: null };
  class Element {
    constructor(children = [], { disabled = false } = {}) {
      this.children = children;
      this.disabled = disabled;
      this.tabIndex = 0;
      this.isConnected = true;
      for (const child of children) child.parent = this;
    }
    focus() {
      document.activeElement = this;
    }
    matches(selector) {
      return selector === ":disabled" && this.disabled;
    }
    closest(selector) {
      for (let node = this; node; node = node.parent) {
        if (selector === "[inert]" && node.inert) return node;
      }
      return null;
    }
    getClientRects() {
      return [{}];
    }
    contains(other) {
      for (let node = other; node; node = node.parent) if (node === this) return true;
      return false;
    }
    querySelectorAll() {
      return this.children.flatMap((child) => [child, ...child.querySelectorAll()]);
    }
  }
  document.body = new Element();
  document.activeElement = document.body;
  const window = {
    addEventListener: (type, listener, capture) => type === "keydown" && capture && listeners.add(listener),
    removeEventListener: (type, listener) => listeners.delete(listener),
  };
  /**
   * Press Tab, and move focus on as the browser would when not prevented.
   * `handledAbove` is a layer listening earlier that keeps Tab for itself;
   * `isComposing`, an input method composing with it.
   */
  function tab(shiftKey = false, { handledAbove = false, isComposing = false } = {}) {
    let prevented = handledAbove;
    const event = {
      key: "Tab",
      shiftKey,
      isComposing,
      get defaultPrevented() {
        return prevented;
      },
      preventDefault: () => (prevented = true),
    };
    for (const listener of listeners) listener(event);
    return prevented;
  }
  return { Element, document, window, listeners, tab };
}

async function loadTrap() {
  const dom = fakeDom();
  const source = await readFile(new URL("../../src/lib/focus-trap.ts", import.meta.url), "utf8");
  const { outputText } = ts.transpileModule(source, {
    compilerOptions: { target: ts.ScriptTarget.ES2022, module: ts.ModuleKind.ESNext },
  });
  const context = vm.createContext({
    window: dom.window,
    document: dom.document,
    HTMLElement: dom.Element,
    Node: dom.Element,
    queueMicrotask,
  });
  const module = new vm.SourceTextModule(outputText, { context });
  await module.link(() => {});
  await module.evaluate();
  return { ...dom, trapFocus: module.namespace.trapFocus };
}

const settle = () => new Promise((resolve) => setTimeout(resolve));

test("an open dialog takes focus and keeps Tab within it", async () => {
  const { Element, document, tab, trapFocus } = await loadTrap();
  const [first, skipped, middle, last] = [new Element(), new Element([], { disabled: true }), new Element(), new Element()];
  const dialog = new Element([first, skipped, middle, last]);
  const page = new Element();
  page.focus();

  trapFocus(dialog, { initial: () => middle });
  await settle();
  assert.equal(document.activeElement, middle);

  // Within it, Tab moves on by itself.
  assert.equal(tab(), false);
  last.focus();
  assert.equal(tab(), true);
  assert.equal(document.activeElement, first);
  assert.equal(tab(true), true);
  assert.equal(document.activeElement, last, "skipping the disabled element");
  // Focus left on the page under it is brought back in.
  page.focus();
  assert.equal(tab(), true);
  assert.equal(document.activeElement, first);

  // A layer over every dialog, such as the command palette, keeps its Tab,
  // as does an input method composing with it there.
  const palette = new Element();
  palette.focus();
  tab(true, { handledAbove: true });
  assert.equal(document.activeElement, palette);
  assert.equal(tab(true, { isComposing: true }), false);
  assert.equal(document.activeElement, palette);
});

test("only the topmost dialog holds Tab, and closing one gives focus back", async () => {
  const { Element, document, listeners, tab, trapFocus } = await loadTrap();
  const opener = new Element();
  const [upperClose, upperButton] = [new Element(), new Element()];
  const lower = new Element([opener]);
  const upper = new Element([upperClose, upperButton]);

  const lowerTrap = trapFocus(lower);
  await settle();
  assert.equal(document.activeElement, opener);

  // Kept mounted while hidden, as the cover editor is.
  const upperTrap = trapFocus(upper, { active: false });
  await settle();
  assert.equal(document.activeElement, opener);
  upperTrap.update({ active: true });
  await settle();
  assert.equal(document.activeElement, upperClose);
  upperButton.focus();
  assert.equal(tab(), true);
  assert.equal(document.activeElement, upperClose, "not into the dialog under it");

  upperTrap.update({ active: false });
  assert.equal(document.activeElement, opener);
  lowerTrap.destroy();
  assert.equal(listeners.size, 0);
  assert.equal(tab(), false);
});

test("dialogs on inactive pages neither take initial focus nor intercept navigation on another page", async () => {
  const { Element, document, tab, trapFocus } = await loadTrap();
  const visibleButton = new Element();
  const visibleDialog = new Element([visibleButton]);
  const hiddenButton = new Element();
  const hiddenDialog = new Element([hiddenButton]);
  const inactivePage = new Element([hiddenDialog]);
  inactivePage.inert = true;
  const visibleTrap = trapFocus(visibleDialog);
  await settle();
  const hiddenTrap = trapFocus(hiddenDialog);
  await settle();
  assert.equal(document.activeElement, visibleButton);
  assert.equal(tab(), true);
  assert.equal(document.activeElement, visibleButton);
  visibleTrap.destroy();
  assert.equal(tab(), false, "the hidden dialog must not block another page's Tab");
  hiddenTrap.destroy();
});
