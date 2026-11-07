/**
 * Modal dialogs keep keyboard focus within themselves. Dialogs open over one
 * another, such as the cover editor or the task center over the upload
 * dialog, so only the topmost one holds Tab, and closing it gives focus back
 * to where it was before it opened.
 */

interface FocusTrapOptions {
  /** Whether the dialog is showing, for one kept mounted while hidden. */
  active?: boolean;
  /** What takes focus when it opens; else its first focusable element. */
  initial?: () => HTMLElement | null | undefined;
}

/** Open dialogs, topmost last. */
const traps: HTMLElement[] = [];

const FOCUSABLE = "a[href], button, input, select, textarea, [tabindex]";

function focusable(root: HTMLElement): HTMLElement[] {
  return [...root.querySelectorAll<HTMLElement>(FOCUSABLE)].filter(
    (element) =>
      element.tabIndex >= 0 &&
      !element.matches(":disabled") &&
      !element.closest("[inert]") &&
      element.getClientRects().length > 0,
  );
}

// In the capture phase on the window, so that a dialog stopping the key's
// propagation there, as the task center does, still has Tab kept inside. A
// Tab already handled is left alone: a layer above all dialogs, such as the
// command palette, keeps it for itself. So is one an input method is
// composing with.
function keepTabInside(event: KeyboardEvent) {
  const top = [...traps].reverse().find((node) => !node.closest("[inert]"));
  if (event.key !== "Tab" || event.defaultPrevented || event.isComposing || !top) return;
  const elements = focusable(top);
  if (elements.length === 0) {
    event.preventDefault();
    return;
  }
  const first = elements[0];
  const last = elements[elements.length - 1];
  const active = document.activeElement;
  const inside = active instanceof Node && top.contains(active);
  if (!inside || (event.shiftKey ? active === first : active === last)) {
    event.preventDefault();
    (event.shiftKey ? last : first).focus();
  }
}

/** Svelte action: while `node` is an open dialog, focus stays within it. */
export function trapFocus(node: HTMLElement, options: FocusTrapOptions = {}) {
  let open = false;
  let returnTo: HTMLElement | null = null;

  function activate() {
    open = true;
    returnTo = document.activeElement instanceof HTMLElement ? document.activeElement : null;
    if (traps.length === 0) window.addEventListener("keydown", keepTabInside, true);
    traps.push(node);
    // After the update that shows it, such as dropping `inert`.
    queueMicrotask(() => {
      if (!open || node.closest("[inert]") || node.contains(document.activeElement)) return;
      (options.initial?.() ?? focusable(node)[0])?.focus();
    });
  }

  function deactivate() {
    open = false;
    traps.splice(traps.indexOf(node), 1);
    if (traps.length === 0) window.removeEventListener("keydown", keepTabInside, true);
    // Unless focus has already moved on elsewhere.
    const active = document.activeElement;
    if (returnTo?.isConnected && !returnTo.closest("[inert]") && (!active || active === document.body || node.contains(active))) {
      returnTo.focus();
    }
    returnTo = null;
  }

  if (options.active ?? true) activate();
  return {
    update(next: FocusTrapOptions = {}) {
      options = next;
      const active = next.active ?? true;
      if (active && !open) activate();
      else if (!active && open) deactivate();
    },
    destroy() {
      if (open) deactivate();
    },
  };
}
