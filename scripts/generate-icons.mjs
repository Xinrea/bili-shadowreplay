// public/icon.svg is the editable source for every project icon.
import { execFileSync } from 'node:child_process';
import { copyFileSync, mkdtempSync, readdirSync, rmSync } from 'node:fs';
import { tmpdir } from 'node:os';
import { join } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = fileURLToPath(new URL('../', import.meta.url));
const source = join(root, 'public/icon.svg');
const output = mkdtempSync(join(tmpdir(), 'shadowreplay-icons-'));

try {
  execFileSync(process.execPath, [
    join(root, 'node_modules/@tauri-apps/cli/tauri.js'),
    'icon', source, '--output', output,
  ], { cwd: root, stdio: 'inherit' });

  // Keep the existing desktop icon set; mobile assets are not used here.
  const desktop = join(root, 'src-tauri/icons');
  for (const name of readdirSync(desktop)) {
    if (/\.(png|ico|icns)$/.test(name)) {
      copyFileSync(join(output, name), join(desktop, name));
    }
  }
  copyFileSync(source, join(root, 'docs/public/images/icon.svg'));
  copyFileSync(join(output, 'icon.png'), join(root, 'docs/public/images/icon.png'));
} finally {
  rmSync(output, { recursive: true, force: true });
}
