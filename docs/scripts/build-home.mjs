import { build } from 'vite';
import vue from '@vitejs/plugin-vue';
import { mkdir, writeFile } from 'node:fs/promises';
import { fileURLToPath } from 'node:url';

// Keep the existing Vue product demo as a prerendered island. Fumadocs owns
// navigation and documentation; Vue only hydrates this dedicated DOM subtree.
const root = fileURLToPath(new URL('..', import.meta.url));
const shared = {
  root,
  configFile: false,
  define: { 'process.env.NODE_ENV': JSON.stringify('production') },
  plugins: [vue()],
  publicDir: false,
  css: { postcss: { plugins: [] } },
};
await build({
  ...shared,
  build: {
    outDir: 'public/home',
    emptyOutDir: true,
    lib: { entry: `${root}/home/client.ts`, formats: ['es'], fileName: () => 'home.js', cssFileName: 'home' },
  },
});
await build({
  ...shared,
  build: { ssr: `${root}/home/server.ts`, outDir: '.home', emptyOutDir: true },
});
const { render } = await import('../.home/server.js');
await mkdir(`${root}/.home`, { recursive: true });
await writeFile(`${root}/.home/index.html`, await render());
