import { readFile } from 'node:fs/promises';
import path from 'node:path';
import { HomeLayout } from 'fumadocs-ui/layouts/home';
import { baseOptions } from '@/lib/layout.shared';
import { HomeIsland } from '@/components/home-island';
import type { Metadata } from 'next';

export const metadata: Metadata = { alternates: { canonical: '/' } };

export default async function HomePage() {
  const html = await readFile(path.join(process.cwd(), '.home/index.html'), 'utf8');
  return (
    <HomeLayout {...baseOptions()}>
      <link rel="stylesheet" href="/home/home.css" />
      <HomeIsland html={html} />
    </HomeLayout>
  );
}
