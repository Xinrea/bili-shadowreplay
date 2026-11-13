import type { Heading, Root } from 'mdast';

const pageSlugs = new WeakMap<Root, Set<string>>();

/** Preserve VitePress 1.6 heading URLs, including Unicode punctuation and duplicate suffixes. */
export function headingSlug(root: Root, _heading: Heading, text: string): string {
  // Matches the slugify bundled with VitePress 1.6.3 (@mdit-vue/shared).
  const base = text
    .normalize('NFKD')
    .replace(/[\u0300-\u036f]/g, '')
    .replace(/[\u0000-\u001f]/g, '')
    .replace(/[\s~`!@#$%^&*()\-_+=[\]{}|\\;:"'“”‘’<>,.?/]+/g, '-')
    .replace(/-{2,}/g, '-')
    .replace(/^-+|-+$/g, '')
    .replace(/^(\d)/, '_$1')
    .toLowerCase();
  let used = pageSlugs.get(root);
  if (!used) {
    used = new Set();
    pageSlugs.set(root, used);
  }
  let slug = base;
  for (let suffix = 1; used.has(slug); suffix++) slug = `${base}-${suffix}`;
  used.add(slug);
  return slug;
}
