import type { BaseLayoutProps } from 'fumadocs-ui/layouts/shared';

export const repository = 'https://github.com/powerlive-io/bili-shadowreplay';
export const siteUrl = 'https://bsr.powerlive.io';

export function baseOptions(): BaseLayoutProps {
  return {
    nav: {
      title: <><img src="/images/icon.svg" alt="" width={26} height={26} />BiliBili ShadowReplay</>,
      url: '/',
    },
    githubUrl: repository,
    links: [
      { text: '使用文档', url: '/getting-started/installation/desktop' },
      { text: '下载', url: `${repository}/releases`, external: true },
    ],
  };
}
