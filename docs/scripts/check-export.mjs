import assert from 'node:assert/strict';
import { existsSync, readFileSync, readdirSync } from 'node:fs';
import path from 'node:path';
import { parse } from 'parse5';
import { staticClient } from 'fumadocs-core/search/client/orama-static';

const root = path.resolve('out');
const origin = 'https://bsr.powerlive.io';
const files = readdirSync(root, { recursive: true }).filter((file) => file.endsWith('.html'));
const failures = [];
let links = 0;
for (const file of files) {
  const html = readFileSync(path.join(root, file), 'utf8');
  const url = new URL(file === 'index.html' ? '/' : `/${file.replace(/\.html$/, '')}`, origin);
  function walk(node) {
    for (const { name, value } of node.attrs ?? []) {
      if (!['href', 'src'].includes(name) || !value || /^(data:|mailto:|tel:|#)/.test(value)) continue;
      const target = new URL(value, url);
      if (target.origin !== origin) continue;
      const pathname = decodeURIComponent(target.pathname);
      const base = path.join(root, pathname);
      if (![base, `${base}.html`, path.join(base, 'index.html')].some(existsSync)) failures.push(`${file}: ${value}`);
      links++;
    }
    for (const child of node.childNodes ?? []) walk(child);
  }
  walk(parse(html));
  assert(!html.includes('bsr.xinrea.cn'), `Old domain in ${file}`);
}
assert.deepEqual(failures, [], 'Broken internal links or missing assets');
const content = ['getting-started', 'usage', 'development'].flatMap((dir) => readdirSync(dir, { recursive: true }).filter((f) => f.endsWith('.md')).map((f) => `${dir}/${f}`));
for (const file of [...content, 'TESTING.md']) assert(existsSync(path.join(root, file.replace(/\.md$/, '.html'))), `Missing page: ${file}`);
assert.equal(readFileSync(path.join(root, 'CNAME'), 'utf8').trim(), 'bsr.powerlive.io');
assert(existsSync(path.join(root, '.nojekyll')));
assert.match(readFileSync(path.join(root, 'index.html'), 'utf8'), /切片编辑/, 'Homepage is prerendered');
assert.match(readFileSync(path.join(root, 'usage/features/room.html'), 'utf8'), /<video[^>]+src="\/videos\/deeplinking.mp4"/, 'Inline videos survive Markdown compilation');
assert.match(readFileSync(path.join(root, 'usage/features/workflow.html'), 'utf8'), /图表加载中/, 'Mermaid blocks reach the renderer');
const index = readFileSync(path.join(root, 'api/search'), 'utf8');
const client = staticClient({ from: `data:application/json;base64,${Buffer.from(index).toString('base64')}` });
for (const [query, expected] of [['字幕', '/usage/features/subtitle'], ['Docker', '/getting-started/installation/docker'], ['录制', '/development/backend/recorder-manager']]) {
  const results = await client.search(query);
  assert(results.some((result) => result.url?.startsWith(expected)), `Search failed: ${query}`);
}
console.log(`Verified ${content.length + 1} documentation pages, ${links} internal links/assets, homepage, videos, Mermaid and Chinese/English static search.`);
