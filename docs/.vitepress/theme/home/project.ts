/** The clip project the editor, subtitle and upload scenes work on, in seconds. */
import type { Clip, Span } from "./EditorFrame.vue";

export const MATERIALS: Clip[] = [
  { start: 0, length: 52, label: "深渊满星挑战 选区 1", scene: "dusk" },
  { start: 52, length: 47, label: "深渊满星挑战 选区 2", scene: "forest" },
  { start: 99, length: 41, label: "深渊满星挑战 选区 3", scene: "sea" },
];

export const BGM: Span = { start: 0, length: 140, label: "轻快 BGM.mp3" };

export const TITLE: Span = { start: 48, length: 8, label: "满星达成！" };

/** What AI 字幕 recognizes. */
export const SUBTITLES: Span[] = [
  { start: 0, length: 4, label: "好，今天我们来打深境螺旋" },
  { start: 5, length: 3, label: "目标是十二层满星" },
  { start: 14, length: 3, label: "先上胡桃这一队" },
  { start: 31, length: 4, label: "这一下伤害直接拉满" },
  { start: 57, length: 5, label: "雷电将军开大，好起来了" },
  { start: 75, length: 4, label: "稳住，还有最后一波" },
  { start: 104, length: 4, label: "满星！终于拿下了" },
  { start: 128, length: 4, label: "谢谢大家的弹幕陪伴" },
];

/** Danmaku saved with the materials, burned when 压制弹幕 is on. */
export const DANMU_COUNT = 1286;
