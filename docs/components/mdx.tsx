import defaultComponents from 'fumadocs-ui/mdx';
import type { MDXComponents } from 'mdx/types';
import { Callout } from 'fumadocs-ui/components/callout';
import { Mermaid } from './mermaid';

export function getMDXComponents(): MDXComponents {
  return {
    ...defaultComponents,
    blockquote: ({ children, ...props }) => {
      const kind = (props as Record<string, unknown>)['data-callout'];
      if (typeof kind !== 'string') return <blockquote {...props}>{children}</blockquote>;
      const titles: Record<string, string> = { NOTE: '说明', TIP: '提示', IMPORTANT: '重要', WARNING: '注意', CAUTION: '警告' };
      return <Callout title={titles[kind]} type={kind === 'CAUTION' ? 'error' : kind === 'WARNING' ? 'warn' : 'info'}>{children}</Callout>;
    },
    pre: (props) => {
      if ('data-language' in props && props['data-language'] === 'mermaid') {
        return <Mermaid chart={String((props as Record<string, unknown>)['data-code'] ?? '')} />;
      }
      const Pre = defaultComponents.pre!;
      return <Pre {...props} />;
    },
  };
}
