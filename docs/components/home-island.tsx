'use client';

import { useEffect, useRef } from 'react';

export function HomeIsland({ html }: { html: string }) {
  const container = useRef<HTMLDivElement>(null);
  useEffect(() => {
    let disposed = false;
    let unmount: (() => void) | undefined;
    // This module is built separately by Vite and hydrates only its own subtree.
    const entry = '/home/home.js';
    import(/* webpackIgnore: true */ entry).then(({ mount }) => {
      if (!disposed && container.current) unmount = mount(container.current);
    });
    return () => { disposed = true; unmount?.(); };
  }, []);
  return <div ref={container} dangerouslySetInnerHTML={{ __html: html }} />;
}
