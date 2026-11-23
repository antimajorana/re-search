import test from 'node:test';
import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

test('Stage 16 Sources title & deduplicated Origin links + Proof DAG controls & CSS safety', async () => {
  const mathUrl = JSON.stringify(pathToFileURL(path.join(assetsDir, 'math.js')).href);
  const srcLinksCode = fs
    .readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
    .replace("'/assets/math.js'", mathUrl);
  const srcLinksUrl = `data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`;
  const sl = await import(srcLinksUrl);

  const src = {
    source_id: 'arxiv_2301_12345v1-abcd1234',
    title: 'Two-Dimensional Lieb-Schultz-Mattis Obstructions',
    origin: 'https://export.arxiv.org/e-print/2301.12345v1',
    primary_path: 'arxiv_2301_12345v1-abcd1234/main.tex',
  };
  assert.equal(sl.resolveSourceTitle(src), 'Two-Dimensional Lieb-Schultz-Mattis Obstructions');
  const originInfo = sl.resolveOriginInfo(src);
  assert.equal(originInfo.url, 'https://arxiv.org/abs/2301.12345v1');
  const extra = sl.linksForIngestedSource(src).filter((lk) => lk.url !== originInfo.url);
  assert.equal(extra.some((lk) => lk.url === originInfo.url), false, 'Origin link must not repeat in chips');
  assert.equal(extra.some((lk) => lk.label === 'PDF'), true, 'Secondary PDF link preserved');

  // Legacy/untitled source must fall back to clean filename, never raw URL.
  const untitled = {
    source_id: 'paper-1234abcd',
    origin: 'https://export.arxiv.org/e-print/2402.00001v1',
    primary_path: 'paper-1234abcd/article.tex',
  };
  assert.equal(sl.resolveSourceTitle(untitled), 'article.tex');

  const indexHtml = fs.readFileSync(path.join(assetsDir, 'index.html'), 'utf8');
  const assetsRs = fs.readFileSync(path.join(crateRoot, 'src', 'assets.rs'), 'utf8');
  const dagCss = fs.readFileSync(path.join(assetsDir, 'dag.css'), 'utf8');
  for (const id of ['dag-viewport', 'dag-zoom-out', 'dag-zoom-reset', 'dag-zoom-in', 'dag-zoom-fit']) {
    assert.match(indexHtml, new RegExp(`id="${id}"`), `Missing #${id}`);
  }
  for (const asset of ['/assets/dag.css', '/assets/canvas_dag_detail.js']) {
    assert.ok(assetsRs.includes(asset), `Missing ${asset} in assets.rs`);
  }
  assert.equal(/\.(?:dag-eq-box|dag-explanation-box|verdict-recovery-body|rich-prose)\s+(?:span|div|\*)\b/.test(dagCss), false);
});
