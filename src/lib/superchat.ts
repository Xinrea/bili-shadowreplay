// SC styling follows https://github.com/FangDingli/blive-sc-gen.
// Bilibili reports the SC price in CNY while the SC card displays it in
// battery (电池), 1 CNY = 10 battery, and both the color tiers and the badge
// thresholds operate on the battery value.
const SC_BATTERY_PER_CNY = 10;

export interface ScColors {
  /** Light background color (card header). */
  info: string;
  /** Accent color (card body, border, price text). */
  content: string;
}

const SC_COLOR_TIERS: Array<{ minBattery: number; colors: ScColors }> = [
  { minBattery: 0, colors: { info: "#EDF5FF", content: "#2A60B2" } },
  { minBattery: 500, colors: { info: "#dbfffd", content: "#427d9e" } },
  { minBattery: 1000, colors: { info: "#fff1c5", content: "#e2b52b" } },
  { minBattery: 5000, colors: { info: "#ffead2", content: "#e09443" } },
  { minBattery: 10000, colors: { info: "#ffe7e4", content: "#e54d4d" } },
  { minBattery: 20000, colors: { info: "#ffd8d8", content: "#ab1a32" } },
];

export function scBattery(price: number): number {
  return price * SC_BATTERY_PER_CNY;
}

export function getScColors(price: number): ScColors {
  const battery = scBattery(price);
  let colors = SC_COLOR_TIERS[0].colors;
  for (const tier of SC_COLOR_TIERS) {
    if (battery >= tier.minBattery) {
      colors = tier.colors;
    }
  }
  return colors;
}

export function formatScPrice(price: number): string {
  return `${scBattery(price)} 电池`;
}

export const SUPER_CHAT_EVENT_TYPE = "super_chat";

export function isSuperChat(entry: { type: string }): boolean {
  return entry.type === SUPER_CHAT_EVENT_TYPE;
}
