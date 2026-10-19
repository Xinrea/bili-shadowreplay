// Vendors the cover editor's stickers: Microsoft's Fluent Emoji (MIT) in the
// flat style of Windows 11's Segoe UI Emoji, without the gradients of the
// newer "Color" designs.
//
//   node scripts/fetch-fluent-emoji.mjs [commit]
//
// Writes public/stickers/*.svg with the license beside them, and the manifest
// the editor reads, src/lib/components/cover/stickers.json. Edit GROUPS to
// change the selection; the default skin tone is used for people.
import { mkdirSync, rmSync, writeFileSync } from 'node:fs';
import { dirname, join } from 'node:path';
import { fileURLToPath } from 'node:url';

const REPO = 'microsoft/fluentui-emoji';
const STYLE = 'Flat';

const GROUPS = [
  {
    name: '表情',
    emoji: `😀 😃 😄 😁 😆 😅 🤣 😂 🙂 🙃 🫠 😉 😊 😇 🥰 😍 🤩 😘 😋 😛 😜 🤪 😝 🤑
      🤗 🤭 🫢 🫣 🤫 🤔 🫡 🤐 🤨 😐 😑 😶 😏 😒 🙄 😬 😮‍💨 😌 😔 😪 🤤 😴 😷 🤢
      🤮 🥵 🥶 🥴 😵‍💫 🤯 🥳 😎 🤓 🧐 😕 🙁 😮 😲 😳 🥺 🥹 😨 😰 😢 😭 😱 😖 😩
      😫 🥱 😤 😡 🤬 😈 💀 💩 🤡 👻 👽 🤖 😹 😻 🙀`,
  },
  {
    name: '手势',
    emoji: `👍 👎 👏 🙌 🙏 🤝 ✌️ 🤞 🫰 🤟 🤘 👌 🤌 🤏 👈 👉 👆 👇 ☝️ 🫵 ✋ 👋
      🤙 💪 🫶 ✍️ 👀 🧠`,
  },
  {
    name: '符号',
    emoji: `❤️ 🧡 💛 💚 💙 💜 🖤 🤍 💔 ❤️‍🔥 💕 💞 💗 💖 💘 💝 💯 💢 💥 💫 💦 💨
      💬 💭 💤 ❗ ❓ ‼️ ⁉️ ⚠️ 🚫 ❌ ⭕ ✅ ✔️ 🆗 🆕 🆒 🆙 🔝 🔥 ✨ ⭐ 🌟 💡 ⚡ 🎵
      🎶 ➡️ ⬅️ ⬆️ ⬇️`,
  },
  {
    name: '物品',
    emoji: `🎉 🎊 🎁 🎈 🎂 🏆 🥇 🥈 🥉 🏅 👑 💎 💰 💸 🧧 🎮 🕹️ 🎤 🎧 📺 📷 🎬
      📢 📣 🔔 ⏰ ⌛ 💣 🧨 🎯 🎲 🀄 🎨 📌 🔍 💊 🚀 🛸 📈 📉`,
  },
  {
    name: '动物',
    emoji: `🐶 🐱 🐭 🐹 🐰 🦊 🐻 🐼 🐨 🐯 🦁 🐮 🐷 🐸 🐵 🙈 🙉 🙊 🐔 🐧 🐤 🦆
      🦉 🐺 🦄 🐝 🦋 🐌 🐢 🐍 🐙 🦀 🐟 🐬 🐳 🦈 🐘 🌸 🌹 🌻 🍀 🌈 ☀️ 🌙 ❄️ ⛄`,
  },
  {
    name: '食物',
    emoji: `🍎 🍉 🍓 🍑 🍒 🍋 🍌 🍇 🍅 🍆 🌶️ 🍄 🍞 🍔 🍟 🍕 🌭 🍣 🍜 🍙 🍚 🥟
      🍰 🧁 🍩 🍪 🍫 🍬 🍭 🍦 🍿 🧋 ☕ 🍵 🍺 🍻 🥂`,
  },
];

const root = fileURLToPath(new URL('../', import.meta.url));
const output = join(root, 'public/stickers');
const manifest = join(root, 'src/lib/components/cover/stickers.json');

async function get(url) {
  const response = await fetch(url);
  if (!response.ok) throw new Error(`${response.status} ${url}`);
  return response;
}

async function each(items, limit, task) {
  let next = 0;
  const worker = async () => {
    while (next < items.length) await task(items[next++]);
  };
  await Promise.all(Array.from({ length: limit }, worker));
}

// Variation selectors differ between sources; match without them.
const bare = (glyph) => glyph.replace(/\uFE0F/g, '');

// Two decimals on a 32px grid stay sharp at any cover size. A number cut
// before another number that starts with "." keeps its zeros, so the two
// are not read as one.
function optimize(svg) {
  return svg
    .replace(/>\s+</g, '><')
    .replace(/\s+/g, ' ')
    .replace(/-?\d*\.\d{3,}/g, (number, offset, text) => {
      const fixed = Number(number).toFixed(2);
      return text[offset + number.length] === '.' ? fixed : fixed.replace(/\.?0+$/, '') || '0';
    })
    // Rasterized at its own size by some engines; draw it from a large one.
    .replace(/^<svg width="32" height="32"/, '<svg width="512" height="512"')
    .trim();
}

const commit =
  process.argv[2] ?? (await (await get(`https://api.github.com/repos/${REPO}/commits/main`)).json()).sha;
const tree = await (await get(`https://api.github.com/repos/${REPO}/git/trees/${commit}?recursive=1`)).json();
if (tree.truncated) throw new Error('The repository tree is truncated');
const raw = (path) =>
  `https://raw.githubusercontent.com/${REPO}/${commit}/${path.split('/').map(encodeURIComponent).join('/')}`;
const paths = tree.tree.map((entry) => entry.path);

// Folders are named by CLDR names, so find each glyph through its metadata.
const assets = new Map();
await each(
  paths.filter((path) => /^assets\/[^/]+\/metadata\.json$/.test(path)),
  24,
  async (path) => {
    const metadata = await (await get(raw(path))).json();
    assets.set(bare(metadata.glyph), { folder: dirname(path), metadata });
  },
);

const wanted = GROUPS.flatMap((group) =>
  group.emoji.trim().split(/\s+/).map((glyph) => ({ group: group.name, glyph })),
);
const missing = wanted.filter(({ glyph }) => !assets.has(bare(glyph)));
if (missing.length) throw new Error(`Not in ${REPO}: ${missing.map((m) => m.glyph).join(' ')}`);

rmSync(output, { recursive: true, force: true });
mkdirSync(output, { recursive: true });
const groups = GROUPS.map((group) => ({ name: group.name, stickers: [] }));
await each(wanted, 16, async ({ group, glyph }) => {
  const { folder, metadata } = assets.get(bare(glyph));
  const svg =
    paths.find((path) => path.startsWith(`${folder}/${STYLE}/`) && path.endsWith('.svg')) ??
    paths.find((path) => path.startsWith(`${folder}/Default/${STYLE}/`) && path.endsWith('.svg'));
  if (!svg) throw new Error(`No ${STYLE} design for ${glyph}`);
  const id = metadata.unicode.replace(/ /g, '-');
  writeFileSync(join(output, `${id}.svg`), optimize(await (await get(raw(svg))).text()));
  groups.find((g) => g.name === group).stickers.push({ id, glyph, name: metadata.cldr });
});
writeFileSync(join(output, 'LICENSE'), await (await get(raw('LICENSE'))).text());

// Keep the order of GROUPS rather than download order.
for (const group of groups) {
  const order = GROUPS.find((g) => g.name === group.name).emoji.trim().split(/\s+/);
  group.stickers.sort((a, b) => order.indexOf(a.glyph) - order.indexOf(b.glyph));
}
// One sticker per line.
const lines = groups.map(
  (group) =>
    `    { "name": ${JSON.stringify(group.name)}, "stickers": [\n` +
    group.stickers.map((sticker) => `      ${JSON.stringify(sticker)}`).join(',\n') +
    '\n    ] }',
);
mkdirSync(dirname(manifest), { recursive: true });
writeFileSync(
  manifest,
  `{\n  "source": "https://github.com/${REPO}/tree/${commit}",\n  "groups": [\n${lines.join(',\n')}\n  ]\n}\n`,
);
console.log(`Saved ${wanted.length} stickers from ${REPO}@${commit.slice(0, 7)}`);
