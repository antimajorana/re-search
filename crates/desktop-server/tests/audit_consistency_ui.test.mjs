import assert from 'node:assert/strict';
import fs from 'node:fs';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath, pathToFileURL } from 'node:url';

const __dirname = path.dirname(fileURLToPath(import.meta.url));
const crateRoot = path.resolve(__dirname, '..');
const assetsDir = path.join(crateRoot, 'assets');

const mathUrl = pathToFileURL(path.join(assetsDir, 'math.js')).href;

const srcLinksCode = fs
  .readFileSync(path.join(assetsDir, 'source_links.js'), 'utf8')
  .replace("'/assets/math.js'", JSON.stringify(mathUrl));
const srcLinksMod = await import(
  `data:text/javascript;base64,${Buffer.from(srcLinksCode, 'utf8').toString('base64')}`
);

test('sessionSourcesUsed counts papers cited via [path:Lstart-end] in question-first investigation answers', () => {
  const session = {
    events: [
      { seq: 1, kind: 'session.start', payload: { prompt: 'What vanishes?' } },
      {
        seq: 2,
        kind: 'source.ingested',
        payload: {
          source_id: 'src-1',
          title: 'Notes',
          primary_path: 'src-1/notes.txt',
          files: ['src-1/notes.txt'],
        },
      },
      { seq: 3, kind: 'investigation.paper', payload: { source_id: 'src-1' } },
      { seq: 4, kind: 'investigation.question_requested', payload: { source_id: 'src-1', question: 'What vanishes?' } },
      { seq: 5, kind: 'model.response', payload: {} },
      { seq: 6, kind: 'model.text', payload: { text: 'The endpoint variation vanishes [L2].' } },
      {
        seq: 7,
        kind: 'investigation.answer',
        payload: {
          request_seq: 4,
          source_id: 'src-1',
          question: 'What vanishes?',
          answer: 'The endpoint variation vanishes [L2].',
          cites: [{ source_id: 'src-1', path: 'src-1/notes.txt', start: 2, end: 2, read: true }],
        },
      },
      { seq: 8, kind: 'session.end', payload: { outcome: 'completed' } },
    ],
    sources: [
      {
        source_id: 'src-1',
        title: 'Notes',
        primary_path: 'src-1/notes.txt',
        files: ['src-1/notes.txt'],
      },
    ],
    investigation: {
      source_id: 'src-1',
      answers: [
        {
          request_seq: 4,
          answer: 'The endpoint variation vanishes [L2].',
          cites: [{ source_id: 'src-1', path: 'src-1/notes.txt', start: 2, end: 2, read: true }],
        },
      ],
    },
  };

  const used = srcLinksMod.sessionSourcesUsed(session);
  assert.equal(
    used.length,
    1,
    'Question-first Investigation answer citations do not mark the primary paper as used in Sources & Literature'
  );
});
