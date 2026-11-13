import { fileURLToPath } from 'node:url';
import { createMDX } from 'fumadocs-mdx/next';

const withMDX = createMDX();

/** @type {import('next').NextConfig} */
export default withMDX({
  output: 'export',
  outputFileTracingRoot: fileURLToPath(new URL('.', import.meta.url)),
  turbopack: { root: fileURLToPath(new URL('.', import.meta.url)) },
  // Keep the existing /path.html URLs working on GitHub Pages.
  trailingSlash: false,
  images: { unoptimized: true },
});
