import test from 'node:test';
import assert from 'node:assert/strict';
import { readFile } from 'node:fs/promises';

function node(tag = 'div') {
  const events = new Map();
  return {
    tag, children: [], dataset: {}, textContent: '', value: '', hidden: false, disabled: false,
    append(...children) { this.children.push(...children); },
    replaceChildren(...children) { this.children = children; },
    addEventListener(name, callback) { events.set(name, callback); },
    trigger(name) { return events.get(name)?.({ preventDefault() {} }); },
    focus() {},
  };
}

function flatten(el) {
  return [el, ...el.children.flatMap(flatten)];
}

function moduleUrl(code) {
  return JSON.stringify(`data:text/javascript;base64,${Buffer.from(code).toString('base64')}`);
}

test('Validation freezes private controls, evaluates an exact recorded result, and opens its step', async (t) => {
  const view = new Map();
  for (const name of ['plan', 'new', 'status', 'freeze', 'detail']) view.set(`[data-${name}]`, node());
  const panel = node();
  panel.hidden = true;
  panel.querySelector = (name) => view.get(name);
  const freeze = view.get('[data-freeze]');
  freeze.elements = { namedItem: () => view.get('session') };
  freeze.querySelector = (name) => view.get(name);
  freeze.reset = () => {};
  view.set('session', node('select'));
  view.set('[data-cancel]', node('button'));
  view.set('[type="submit"]', node('button'));
  const prior = { document: globalThis.document, FormData: globalThis.FormData };
  globalThis.document = {
    getElementById: (id) => id === 'project-validation-panel' ? panel : null,
    createElement: node,
  };
  const input = {
    session_id: 's-lab', claim: 'The smallest eigenvalue converges to one.',
    candidate_method: 'Finite differences', reference_method: 'Exact spectrum', axis: 'resolution',
    atol: '1e-12', rtol: '0', reference: '1.00000000000000',
    known_pass: '1.00000000000001', planted_fail: '0.99999999900000',
  };
  globalThis.FormData = class {
    [Symbol.iterator]() { return Object.entries(input)[Symbol.iterator](); }
  };
  const { reference, known_pass, planted_fail, ...publicInputs } = input;
  const plan = {
    id: 'plan-1', ...publicInputs, status: 'open', commit_seq: 7, commit_hash: 'commit',
    private_commitment: 'private-digest', evaluation: null,
  };
  let frozen;
  let releaseFreeze;
  let recordedCandidates = [];
  let evaluated;
  let opened;
  globalThis.validationTest = {
    listValidationPlans: async () => ({ plans: [] }),
    freezeValidationPlan: async (id, requested) => {
      frozen = [id, requested];
      await new Promise((resolve) => { releaseFreeze = resolve; });
      return plan;
    },
    listValidationCandidates: async () => ({ candidates: recordedCandidates }),
    evaluateValidationPlan: async (id, planId, candidate) => {
      evaluated = [id, planId, candidate];
      return {
        ...plan,
        status: 'accepted',
        evaluation: {
          entry_id: 'e-3', result_seq: 20, value: '1.0', known_pass_passed: true,
          planted_fail_rejected: true, evaluation_seq: 24, evaluation_hash: 'evaluation', result_hash: 'result',
        },
      };
    },
  };
  t.after(() => {
    Object.assign(globalThis, prior);
    delete globalThis.validationTest;
  });
  const api = Object.keys(globalThis.validationTest).map((name) =>
    `export const ${name} = globalThis.validationTest.${name};`).join('\n');
  const path = new URL('../assets/project_validation.js', import.meta.url);
  const code = (await readFile(path, 'utf8'))
    .replace("'/assets/api.js'", moduleUrl(api))
    .replace("'/assets/math.js'", moduleUrl('export const renderRichInto = (el, text) => { el.textContent = text; };'));
  const page = await import(JSON.parse(moduleUrl(code)));
  page.initProjectValidation({
    getSessions: () => [{ id: 's-lab', title: 'Spectral check' }],
    onOpenSession: (...args) => { opened = args; },
  });

  page.setValidationProject({ id: 'p-lab', session_ids: [] });
  panel.hidden = false;
  await page.loadProjectValidation();
  assert.equal(view.get('[data-plan]').disabled, true);
  assert.equal(view.get('[data-new]').disabled, true);
  assert.match(view.get('[data-status]').textContent, /Link or create a session/);
  page.setValidationProject({ id: 'p-lab', session_ids: ['s-lab'] });
  assert.equal(view.get('[data-new]').disabled, false);
  await view.get('[data-new]').trigger('click');
  assert.equal(freeze.hidden, false);
  assert.equal(view.get('session').children[1].textContent, 'Spectral check');
  assert.equal(view.get('session').value, 's-lab');
  const submitted = freeze.trigger('submit');
  assert.equal(view.get('[data-cancel]').disabled, true);
  await page.loadProjectValidation();
  releaseFreeze();
  await submitted;
  assert.deepEqual(frozen, ['p-lab', input]);
  assert.equal(freeze.hidden, true);
  assert.equal(view.get('[data-cancel]').disabled, false);
  const detail = view.get('[data-detail]');
  assert.ok(flatten(detail).some(({ textContent }) => textContent.includes('run check_convergence')));
  recordedCandidates = [{ entry_id: 'e-3', entry_summary: 'Eigenvalue sequence', result_seq: 20 }];
  await flatten(detail).find(({ textContent }) => textContent === 'Refresh candidates').trigger('click');
  const candidate = flatten(detail).find(({ id }) => id === 'validation-candidate');
  assert.match(candidate.children[1].textContent, /e-3 · result 20/);
  candidate.value = '1';
  await candidate.trigger('change');
  await flatten(detail).find(({ tag }) => tag === 'form').trigger('submit');
  assert.deepEqual(evaluated, ['p-lab', 'plan-1', { entry_id: 'e-3', result_seq: 20 }]);
  const contents = flatten(detail).map(({ textContent }) => textContent);
  assert.ok(contents.includes('Within frozen criteria'));
  assert.ok(contents.includes('Recorded outcome'));
  assert.ok(!contents.some((line) => [reference, known_pass, planted_fail].includes(line)));
  await flatten(detail).find(({ textContent }) => textContent === 'Open proof step e-3').trigger('click');
  assert.deepEqual(opened, ['s-lab', 'e-3']);
});
