'use client';

import { useEffect, useId, useState } from 'react';
import { useTheme } from 'next-themes';

export function Mermaid({ chart }: { chart: string }) {
  const id = useId().replaceAll(':', '').replaceAll('«', '').replaceAll('»', '');
  const { resolvedTheme } = useTheme();
  const [svg, setSvg] = useState('');
  const [failed, setFailed] = useState(false);

  useEffect(() => {
    let cancelled = false;
    import('mermaid').then(async ({ default: mermaid }) => {
      mermaid.initialize({ startOnLoad: false, securityLevel: 'strict', theme: resolvedTheme === 'dark' ? 'dark' : 'default' });
      const result = await mermaid.render(`diagram-${id}`, chart);
      if (!cancelled) { setSvg(result.svg); setFailed(false); }
    }).catch(() => { if (!cancelled) setFailed(true); });
    return () => { cancelled = true; };
  }, [chart, id, resolvedTheme]);

  if (!svg || failed) return <pre aria-label={failed ? '图表源码（渲染失败）' : '图表加载中'}><code>{chart}</code></pre>;
  return <div className="my-6 overflow-x-auto rounded-lg border bg-fd-card p-4" role="img" aria-label="流程图" dangerouslySetInnerHTML={{ __html: svg }} />;
}
