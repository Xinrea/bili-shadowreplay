import { defineTranslations } from 'fumadocs-core/i18n';
import { uiTranslations } from 'fumadocs-ui/i18n';

const labels: Record<string, string> = {
  Search: '搜索文档',
  'Open Search': '打开搜索',
  'No results found': '没有找到相关文档',
  'On this page': '本页内容',
  'No Headings': '暂无标题',
  'Table of Contents': '目录',
  'Last updated on': '最后更新',
  'Next Page': '下一页',
  'Previous Page': '上一页',
  'Close Search': '关闭搜索',
  'Open Sidebar': '打开侧栏',
  'Close Sidebar': '关闭侧栏',
  'Collapse Sidebar': '收起侧栏',
  'Show Sidebar': '显示侧栏',
  'Hide Sidebar': '隐藏侧栏',
  'Skip to Content': '跳转到正文',
  'Toggle Menu': '切换菜单',
  'Toggle Theme': '切换主题',
  Light: '浅色',
  Dark: '深色',
  System: '跟随系统',
  'Copy Text': '复制文本',
  'Copied Text': '已复制',
  'Copy Anchor Link': '复制标题链接',
  'Copied Anchor Link': '已复制标题链接',
  'Copy Link': '复制链接',
  'Copied Link': '已复制链接',
};

// Fumadocs qualifies labels with component context (e.g. Search(search trigger)).
export const translations = defineTranslations().extend(uiTranslations()).add(
  Object.fromEntries(uiTranslations().keys.filter((key) => key.split('(')[0] in labels).map((key) => [key, labels[key.split('(')[0]]])),
);
