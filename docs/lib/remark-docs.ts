import type { Root } from 'mdast';
import { visit } from 'unist-util-visit';

/** Keep repository Markdown links usable on the exported website. */
export function remarkDocs() {
  return (tree: Root) => {
    visit(tree, 'blockquote', (node) => {
      const first = node.children[0];
      if (first?.type !== 'paragraph' || first.children[0]?.type !== 'text') return;
      const text = first.children[0];
      const match = /^\[!(NOTE|TIP|IMPORTANT|WARNING|CAUTION)\](?:\n|$)/.exec(text.value);
      if (!match) return;
      text.value = text.value.slice(match[0].length);
      if (!text.value) first.children.shift();
      if (!first.children.length) node.children.shift();
      node.data = { ...node.data, hProperties: { 'data-callout': match[1] } };
    });
    visit(tree, 'link', (node) => {
      if (!/^(?:[a-z]+:|\/\/)/i.test(node.url)) {
        node.url = node.url.replace(/\.md(?=#|$)/, '');
      }
    });
  };
}
