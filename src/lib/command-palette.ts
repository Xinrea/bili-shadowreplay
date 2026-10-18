import type { Icon } from "lucide-svelte";

export type CommandGroup = "page" | "action" | "setting";

export interface PaletteCommand {
  /** Stable across sessions; recently used commands are remembered by id. */
  id: string;
  group: CommandGroup;
  title: string;
  /** Where the command acts, e.g. its page or settings section. */
  detail?: string;
  /** Extra search terms, such as English names. */
  keywords?: string[];
  icon?: typeof Icon;
  /** Main window page to show before running. */
  page?: string;
  /** Leaves the command out while it cannot run. */
  available?: () => boolean;
  run?: () => unknown;
}

export interface CommandSection {
  /** Absent for search results, which are ranked rather than grouped. */
  label?: string;
  commands: PaletteCommand[];
}

const GROUP_LABELS: Record<CommandGroup, string> = {
  page: "页面",
  action: "功能",
  setting: "设置项",
};

export const RECENT_LIMIT = 5;
const MAX_GAPS = 10;

/** Ctrl/⌘ + Shift + P, by physical key so other keyboard layouts work too. */
export function isPaletteShortcut(event: KeyboardEvent) {
  return (
    (event.ctrlKey || event.metaKey) &&
    event.shiftKey &&
    !event.altKey &&
    (event.code === "KeyP" || event.key.toLowerCase() === "p")
  );
}

const apple = typeof navigator !== "undefined" && /Mac|iPhone|iPad/.test(navigator.userAgent);

/** The shortcut as this platform writes it. */
export const PALETTE_SHORTCUT = apple ? "⇧⌘P" : "Ctrl+Shift+P";

/** How well `term` (lowercase) matches `text`; 0 when it does not. */
function matchScore(text: string, term: string) {
  const haystack = text.toLowerCase();
  if (haystack === term) return 100;
  if (haystack.startsWith(term)) return 80;
  const index = haystack.indexOf(term);
  if (index >= 0) return 60 - Math.min(index, 20);
  // Characters in order, so "切路" finds "切片保存路径"; tighter spans rank
  // higher and widely scattered ones do not count.
  let start = -1;
  let position = -1;
  for (const char of term) {
    position = haystack.indexOf(char, position + 1);
    if (position < 0) return 0;
    if (start < 0) start = position;
  }
  const gaps = position - start + 1 - term.length;
  return gaps > MAX_GAPS ? 0 : 30 - gaps;
}

function commandScore(command: PaletteCommand, terms: string[]) {
  let total = 0;
  for (const term of terms) {
    const best = Math.max(
      matchScore(command.title, term),
      ...(command.keywords ?? []).map((keyword) => matchScore(keyword, term) * 0.8),
      command.detail ? matchScore(command.detail, term) * 0.6 : 0,
    );
    // Every term has to match something.
    if (best === 0) return 0;
    total += best;
  }
  return total;
}

/**
 * The palette's list. A query ranks every command; an empty query lists recent
 * commands first, then the rest by group in registration order.
 */
export function paletteSections(
  commands: PaletteCommand[],
  query: string,
  recent: string[] = [],
): CommandSection[] {
  const available = commands.filter((command) => command.available?.() ?? true);
  const terms = query.trim().toLowerCase().split(/\s+/).filter(Boolean);

  if (terms.length > 0) {
    const ranked = available
      .map((command, index) => ({ command, index, score: commandScore(command, terms) }))
      .filter((entry) => entry.score > 0)
      .sort((a, b) => b.score - a.score || a.index - b.index)
      .map((entry) => entry.command);
    return ranked.length > 0 ? [{ commands: ranked }] : [];
  }

  const byId = new Map(available.map((command) => [command.id, command]));
  const recentCommands = recent.flatMap((id) => byId.get(id) ?? []);
  const shown = new Set(recentCommands);
  const sections: CommandSection[] = [];
  if (recentCommands.length > 0) sections.push({ label: "最近使用", commands: recentCommands });
  for (const group of Object.keys(GROUP_LABELS) as CommandGroup[]) {
    const grouped = available.filter((command) => command.group === group && !shown.has(command));
    if (grouped.length > 0) sections.push({ label: GROUP_LABELS[group], commands: grouped });
  }
  return sections;
}

export function rememberRecent(recent: string[], id: string, limit = RECENT_LIMIT) {
  return [id, ...recent.filter((entry) => entry !== id)].slice(0, limit);
}
