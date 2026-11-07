import { writable } from "svelte/store";
import type { PaletteCommand } from "../command-palette";

const registry = writable<PaletteCommand[]>([]);

/** Every command contributed to the main window's palette, in registration order. */
export const commands = { subscribe: registry.subscribe };

export const commandPaletteOpen = writable(false);

/**
 * Contribute commands next to the state they act on. Returns a function that
 * removes them again, so it can be returned from `onMount`.
 */
export function registerCommands(list: PaletteCommand[]) {
  registry.update((all) => [...all, ...list]);
  return () => registry.update((all) => all.filter((command) => !list.includes(command)));
}

export function openCommandPalette() {
  commandPaletteOpen.set(true);
}

export function closeCommandPalette() {
  commandPaletteOpen.set(false);
}
