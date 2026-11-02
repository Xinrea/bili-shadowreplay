import { writable } from "svelte/store";

export type ArchiveViewMode = "list" | "calendar";
const STORAGE_KEY = "archive-view-mode";
export const archiveViewMode = writable<ArchiveViewMode>("list");
let restored = false;

/** The archive page and room dialogs share the same preference in this window. */
export function restoreArchiveViewPreference() {
  if (restored) return;
  restored = true;
  try {
    const saved = localStorage.getItem(STORAGE_KEY);
    if (saved === "list" || saved === "calendar") archiveViewMode.set(saved);
    else if (saved === "timeline") archiveViewMode.set("calendar");
  } catch { /* The view switch still works when browser storage is unavailable. */ }
}

export function setArchiveViewMode(mode: ArchiveViewMode) {
  archiveViewMode.set(mode);
  try { localStorage.setItem(STORAGE_KEY, mode); } catch { /* Keep the in-memory preference. */ }
}
