import { defineConfig, defineDocs } from 'fumadocs-mdx/config';
import rehypeRaw from 'rehype-raw';
import { remarkDocs } from './lib/remark-docs';

export const docs = defineDocs({
  dir: '.',
  docs: {
    files: ['getting-started/**/*.md', 'usage/**/*.md', 'development/**/*.md', 'TESTING.md'],
  },
  meta: {
    files: ['meta.json', 'getting-started/**/meta.json', 'usage/**/meta.json', 'development/**/meta.json'],
  },
});

export default defineConfig({
  mdxOptions: {
    rehypeCodeOptions: {
      themes: { light: 'github-light', dark: 'github-dark' },
      transformers: [{
        pre(node) {
          if (this.options.lang === 'mermaid') {
            node.properties['data-language'] = 'mermaid';
            node.properties['data-code'] = this.source;
          }
        },
      }],
    },
    remarkPlugins: [remarkDocs],
    rehypePlugins: [[rehypeRaw, { passThrough: ['mdxjsEsm', 'mdxJsxFlowElement', 'mdxJsxTextElement', 'mdxFlowExpression', 'mdxTextExpression'] }]],
  },
});
