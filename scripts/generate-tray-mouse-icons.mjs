// SPDX-License-Identifier: AGPL-3.0-or-later
import { execFileSync } from 'node:child_process';
import { mkdirSync, unlinkSync, writeFileSync } from 'node:fs';
import { dirname, resolve } from 'node:path';
import { fileURLToPath } from 'node:url';

const root = resolve(dirname(fileURLToPath(import.meta.url)), '..');
const outputDir = resolve(root, 'src-tauri/icons/tray-mouse-levels');
mkdirSync(outputDir, { recursive: true });

// Exact MouseSimple geometry from Phosphor Icons, regular weight (MIT).
const bodyPath = 'M144,16H112A64.07,64.07,0,0,0,48,80v96a64.07,64.07,0,0,0,64,64h32a64.07,64.07,0,0,0,64-64V80A64.07,64.07,0,0,0,144,16Z';
const outlinePath = 'M144,16H112A64.07,64.07,0,0,0,48,80v96a64.07,64.07,0,0,0,64,64h32a64.07,64.07,0,0,0,64-64V80A64.07,64.07,0,0,0,144,16Zm48,160a48.05,48.05,0,0,1-48,48H112a48.05,48.05,0,0,1-48-48V80a48.05,48.05,0,0,1,48-48h32a48.05,48.05,0,0,1,48,48ZM136,64v48a8,8,0,0,1-16,0V64a8,8,0,0,1,16,0Z';

function batteryColor(level) {
  if (level <= 20) return '#d84a50';
  if (level <= 50) return '#e58a38';
  return '#49a968';
}

for (let level = 0; level <= 100; level += 10) {
  const fillHeight = 224 * level / 100;
  const fillY = 240 - fillHeight;
  const svg = `<!-- Phosphor MouseSimple, MIT License, Copyright (c) 2020 Phosphor Icons. -->
<svg xmlns="http://www.w3.org/2000/svg" viewBox="0 0 256 256">
  <defs><clipPath id="mouse-body"><path d="${bodyPath}"/></clipPath></defs>
  <path d="${bodyPath}" fill="#8b8b8b" opacity=".26"/>
  <rect x="48" y="${fillY}" width="160" height="${fillHeight}" fill="${batteryColor(level)}" clip-path="url(#mouse-body)"/>
  <path d="${outlinePath}" fill="#202124"/>
</svg>`;
  const svgPath = resolve(outputDir, `mouse-${level}.svg`);
  const pngPath = resolve(outputDir, `mouse-${level}.png`);
  writeFileSync(svgPath, svg);
  execFileSync('sips', ['-s', 'format', 'png', '-z', '32', '32', svgPath, '--out', pngPath], { stdio: 'ignore' });
  unlinkSync(svgPath);
}
