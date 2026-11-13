import { i18nProvider } from 'fumadocs-ui/i18n';
import { RootProvider } from 'fumadocs-ui/provider/next';
import type { Metadata } from 'next';
import type { ReactNode } from 'react';
import { siteUrl } from '@/lib/layout.shared';
import { translations } from '@/lib/translations';
import './global.css';

export const metadata: Metadata = {
  metadataBase: new URL(siteUrl),
  title: { default: 'BiliBili ShadowReplay', template: '%s | BiliBili ShadowReplay' },
  description: '直播录制、实时回放、剪辑与投稿工具。BiliBili ShadowReplay 安装指南、功能说明和开发文档。',
  icons: { icon: '/images/icon.svg' },
};

export default function Layout({ children }: { children: ReactNode }) {
  return (
    <html lang="zh-CN" suppressHydrationWarning>
      <body className="flex min-h-screen flex-col">
        <RootProvider
          i18n={i18nProvider(translations)}
          search={{ options: { type: 'static', api: '/api/search' } }}
        >
          {children}
        </RootProvider>
      </body>
    </html>
  );
}
