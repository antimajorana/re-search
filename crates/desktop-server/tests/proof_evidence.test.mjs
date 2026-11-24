import assert from 'node:assert/strict';
import fs from 'node:fs/promises';
import test from 'node:test';

const assets = new URL('../assets/', import.meta.url);
const inspectorSource = (await fs.readFile(new URL('canvas_dag_detail.js', assets), 'utf8'))
  .replace("'/assets/math.js'", JSON.stringify(new URL('math.js', assets).href));
const { renderProofEvidence } = await import(`data:text/javascript;base64,${Buffer.from(inspectorSource).toString('base64')}`);

function element(tag) {
  return {
    tag, children: [], textContent: '',
    appendChild(child) { this.children.push(child); },
    replaceChildren() { this.children = []; },
    addEventListener(event, handler) { this[event] = handler; },
    setAttribute(name, value) { this[name] = value; },
    set innerHTML(_) { throw new Error('Recorded evidence must render as text'); },
  };
}

test('proof evidence gives model, machine, and researcher records independent labels', () => {
  const previous = globalThis.document;
  globalThis.document = { createElement: element };
  try {
    const mount = element('div');
    let picked;
    let opened;
    const reader = { source_id: 'counting', path: 'counting/count.tex', file_sha256: 'a'.repeat(64), start_line: 2, end_line: 3 };
    renderProofEvidence(mount, { entry_id: 'e-3' }, {
      model_reviews: [{ scope: 'full', verdict: 'accepted', seq: 7, summary: 'Model verdict' }],
      support: [
        { kind: 'lean', status: 'verified', via_entry_id: 'e-2', code: 'theorem demo : True := trivial', axioms: ['A'] },
        { kind: 'citation', status: 'researcher_recorded', quote: '<img src=x onerror=alert(1)>' },
        { kind: 'paper_passages', status: 'matches', passages: [{ title: 'Counting', reader, path: reader.path, start_line: 2, end_line: 3 }] },
      ],
    }, (id) => { picked = id; }, (anchor) => { opened = anchor; });
    const [review, lean, citation, paper] = mount.children;
    assert.equal(review.children[0].textContent, 'Full-solution model review · accepted');
    assert.equal(lean.children[0].textContent, 'Lean check · type-checked');
    lean.children.find((child) => child.tag === 'button').click();
    assert.equal(picked, 'e-2');
    assert.ok(lean.children.some((child) => child.textContent === 'Declared axioms'));
    assert.equal(citation.children[0].textContent, 'Researcher citation · researcher-entered');
    assert.equal(citation.children.at(-1).textContent, '<img src=x onerror=alert(1)>');
    const open = paper.children.find((child) => child.tag === 'button');
    assert.equal(open.textContent, 'Open cited lines');
    open.click();
    assert.deepEqual(opened, reader);
    renderProofEvidence(mount, { entry_id: 'e-9' }, { support: [], model_reviews: [] });
    assert.ok(mount.children[0].textContent.includes('No recorded evidence'));
  } finally {
    globalThis.document = previous;
  }
});

test('structured computation shows scoped identity and convergence evidence', () => {
  const previous = globalThis.document;
  globalThis.document = { createElement: element };
  try {
    const mount = element('div');
    renderProofEvidence(mount, { entry_id: 'e-3' }, {
      support: [
        {
          kind: 'computation', status: 'completed', tool_call_ref: 'identity',
          value: 'Unstructured representation',
          validation: {
            schema_version: 1, kind: 'symbolic_identity', status: 'symbolic_agreement',
            lhs: 'sqrt(x**2)', rhs: 'x', residual: '0', assumptions: { x: { positive: true } },
            domain: { requested: 'Reals', effective: '(0, ∞)', common: '(0, ∞)', status: 'complete' },
            samples: [{ point: '<img src=x onerror=alert(1)>', lhs: '2', rhs: '2', status: 'agreement' }],
            limits: [{ point: '0', direction: '+', lhs: '0', rhs: '0', status: 'agreement' }],
            dimensions: { status: 'match', lhs: 'L', rhs: 'L' },
          },
        },
        {
          kind: 'computation', status: 'completed', tool_call_ref: 'refinement',
          validation: {
            schema_version: 1, kind: 'numerical_convergence', axis: 'precision',
            status: 'evaluation_failed', within_tolerance: null, stable: true,
            reference: '1', tail: 2, analysis_dps: 70, atol: '1e-20', rtol: '0',
            samples: [
              { level: 30, work_dps: 30, value: '1.0', reference_error: '0', reference_passes: true },
              { level: 50, work_dps: 50, error: { type: 'ValueError', message: 'domain error' } },
            ],
            rows_omitted: true,
          },
        },
      ],
    });
    const [identity, refinement] = mount.children;
    const descendants = (node) => [node, ...node.children.flatMap(descendants)];
    const texts = (node) => descendants(node).map((child) => child.textContent);
    assert.equal(identity.children[0].textContent, 'Symbolic identity · zero symbolic residual');
    assert.ok(texts(identity).includes('Computational evidence'));
    assert.ok(texts(identity).includes('Assumptions for x'));
    assert.ok(texts(identity).includes('Common domain'));
    assert.ok(texts(identity).includes('Limits'));
    assert.ok(texts(identity).includes('<img src=x onerror=alert(1)>'));
    assert.ok(!texts(identity).includes('Unstructured representation'));
    assert.equal(refinement.children[0].textContent, 'Numerical convergence · evaluation failed');
    for (const inline of ['Precision (dps)', 'Undetermined', '1e-20',
      'Reference error', 'ValueError · domain error', 'Showing selected rows from the recorded report.']) {
      assert.ok(texts(refinement).includes(inline), inline);
    }
  } finally {
    globalThis.document = previous;
  }
});
