import assert from 'node:assert/strict';
import test from 'node:test';

const { formatElapsed, paperViewState, setupButtonLabel } = await import(
  new URL('../assets/investigation_state.js', import.meta.url).href
);

test('question-only session hides setup form, summary, and conventions header while showing the answer', () => {
  const inv = {
    source_id: 'src-1',
    mapping_requested: false,
    map: null,
    map_seq: null,
    summary: null,
    conventions: null,
    conventions_requested: false,
    answers: [{ seq: 5, question: 'What is Eq. (4)?', status: 'answered', text: 'It defines the coupling.' }],
  };
  const state = paperViewState(inv, { running: false, events: [] }, 10_000);
  assert.equal(state.showSetup, false);
  assert.equal(state.refillQuestion, false);
  assert.equal(state.showSummary, false);
  assert.equal(state.showConventions, false);
  assert.equal(state.showMapUnavailable, false);
  assert.equal(state.pollSummary, false);
  assert.equal(state.showAnswer, true);
  assert.equal(state.showComposer, true);
  assert.equal(state.composerPlaceholder, 'Ask a follow-up about this paper…');
  assert.equal(state.statusText, '');
  assert.equal(state.timerActive, false);
});

test('mapping in progress shows the summary placeholder and polls summary', () => {
  const inv = {
    source_id: 'src-1',
    mapping_requested: true,
    map: null,
    summary: null,
    answers: [],
  };
  const events = [{ seq: 4, ts_ms: 1_000, kind: 'investigation.mapping_requested', payload: {} }];
  const state = paperViewState(inv, { running: true, events }, 43_000);
  assert.equal(state.showSetup, false);
  assert.equal(state.showSummary, true);
  assert.equal(state.pollSummary, true);
  assert.equal(state.statusText, 'Reading the paper · 0:42');
  assert.equal(state.timerActive, true);

  const withSummary = paperViewState(
    { ...inv, summary: { what: { text: 'Overview', anchors: [] } }, summary_is_current: true },
    { running: true, events },
    43_000,
  );
  assert.equal(withSummary.statusText, 'Overview saved. Finding the main results · 0:42');
});

test('timer formats 0:42 and 12:05 and handles upload and question running states', () => {
  assert.equal(formatElapsed(42_000), '0:42');
  assert.equal(formatElapsed((12 * 60 + 5) * 1000), '12:05');
  assert.equal(formatElapsed(-500), '0:00');

  const pdf = paperViewState({}, { uploading: 'pdf', startedAtMs: 1_000 }, 43_000);
  assert.equal(pdf.statusText, 'Extracting text from the PDF · 0:42. Large PDFs can take a few minutes.');
  assert.equal(pdf.timerActive, true);

  const other = paperViewState({}, { uploading: 'other', startedAtMs: 1_000 }, 726_000);
  assert.equal(other.statusText, 'Adding paper · 12:05');
  assert.equal(other.timerActive, true);

  const qRunning = paperViewState(
    { source_id: 'src-1', mapping_requested: false, answers: [{ seq: 3, question: 'Why?', status: 'running' }] },
    { running: true, events: [{ seq: 3, ts_ms: 10_000, kind: 'investigation.question_requested', payload: {} }] },
    52_000,
  );
  assert.equal(qRunning.statusText, 'Reading the paper for your question · 0:42');
  assert.equal(qRunning.showSummary, false);
  assert.equal(qRunning.pollSummary, false);
});

test('stopped mapping copy uses session.abort with stopped:true after the request', () => {
  const inv = {
    source_id: 'src-1',
    mapping_requested: true,
    mapping_status: 'failed',
    mapping_error: 'Stopped by user',
    map: null,
    answers: [],
  };
  const events = [
    { seq: 5, ts_ms: 1000, kind: 'investigation.mapping_requested', payload: {} },
    { seq: 8, ts_ms: 5000, kind: 'session.abort', payload: { stopped: true, reason: 'Stopped by user' } },
    { seq: 9, ts_ms: 5010, kind: 'investigation.mapping_finished', payload: { request_seq: 5, status: 'failed' } },
  ];
  const state = paperViewState(inv, { running: false, events }, 10_000);
  assert.equal(state.statusText, 'Stopped. Anything already saved is kept below.');
  assert.equal(state.showApprovalButton, false);
  assert.equal(state.timerActive, false);
});

test('approval copy overrides running wait and enables the Conversation review button', () => {
  const inv = { source_id: 'src-1', mapping_requested: true, answers: [] };
  const fromLoad = paperViewState(inv, { running: true, pending_approvals: [{ call_id: 'c1' }] }, 5_000);
  assert.equal(fromLoad.statusText, 'The model is waiting for your approval.');
  assert.equal(fromLoad.showApprovalButton, true);
  assert.equal(fromLoad.timerActive, false);

  const fromEvent = paperViewState(inv, { running: true, approvalWaiting: true }, 5_000);
  assert.equal(fromEvent.statusText, 'The model is waiting for your approval.');
  assert.equal(fromEvent.showApprovalButton, true);
});

test('setup button label follows whether the question is empty or populated', () => {
  assert.equal(setupButtonLabel(''), 'Find main results');
  assert.equal(setupButtonLabel('   \n  '), 'Find main results');
  assert.equal(setupButtonLabel('Help me understand section V.D.4'), 'Answer my question');
  assert.equal(paperViewState({}, { draftQuestion: '' }, 0).buttonLabel, 'Find main results');
  assert.equal(paperViewState({}, { draftQuestion: 'Why does it diverge?' }, 0).buttonLabel, 'Answer my question');
});

test('absent P1b fields fall back to no answers and legacy mapping gates', () => {
  const legacy = paperViewState({ source_id: 'src-1', map: null }, { running: false }, 0);
  assert.deepEqual(legacy.answers, []);
  assert.equal(legacy.showSummary, true);
  assert.equal(legacy.showMapUnavailable, true);
  assert.equal(legacy.pollSummary, true);
  assert.equal(legacy.showAnswer, false);
});
