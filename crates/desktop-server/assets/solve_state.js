// Pure DOM-free client state model for Solve setup and first screen (package S5b).
// Holds all owner-accepted display strings as single named constants so wording stays
// aligned with `00-overview.md` 2.10, 5.1, 8.1-8.3 and `solve.md` D1, D2, D4, D15, D20, D21.

// Task kinds (`solve.md` 4.1; `00-overview.md` 3.1 row 9).
export const TASK_KINDS = Object.freeze([
  Object.freeze({ id: 'calculate', label: 'Calculate' }),
  Object.freeze({ id: 'prove', label: 'Prove' }),
  Object.freeze({ id: 'check_derivation', label: 'Check a derivation' }),
]);

// Owner decision `00-overview.md` 8.3 question 9 & `solve.md` D21.
export const PROVE_PROSE_NOTICE =
  'a proof in prose is reviewed by a model; only calculations are computed';

// `solve.md` section 2 & D2.
export const SAME_MODEL_WARNING = 'Same model: reviews will be labelled as weaker';

// `00-overview.md` 5.1: works with zero model connected.
export const NO_MODEL_BANNER =
  'No model connected. Setup, prefill, and re-running checks of earlier sessions work without a model.';

// Owner clarification 2026-11-26 (`00-overview.md` 2.10, S2 `solve_badge.rs`).
export const TIER_VERIFIED_WORD = 'Verified';
export const TIER_NUMERIC_WORD = 'Numerically checked';
export const TIER_MODEL_AGREED_WORD = 'Model agreed';
export const TIER_GIVEN_WORD = 'Given';
export const ASK_BEFORE_RUNNING_LABEL = 'Ask before running';

export const DEFAULT_SOLVE_BUDGETS = Object.freeze({
  max_model_calls: 60,
  max_wall_minutes: 30,
  max_model_calls_per_leg: 48,
  max_review_loops: 6,
  max_gate_bounces: 3,
});

export const SYMBOL_ASSUMPTION_PRESETS = Object.freeze([
  'real',
  'real, positive',
  'positive',
  'integer',
  'complex',
  'nonzero',
]);

const CLI_ROLE_PROVIDERS = new Set(['claude-cli', 'antigravity']);

function canonicalProviderFamily(raw) {
  const norm = String(raw || '').trim().toLowerCase();
  if (['anthropic', 'anthropic-messages', 'claude-cli', 'claude'].includes(norm)) return 'anthropic';
  if (['openai', 'openai-chat', 'chatgpt', 'chatgpt-responses'].includes(norm)) return 'openai';
  if (['gemini', 'gemini-generate-content', 'google', 'antigravity'].includes(norm)) return 'gemini';
  if (['ollama', 'ollama-chat'].includes(norm)) return 'ollama';
  return norm;
}

function isInCoreRole(role) {
  const p = String(role?.provider || '').trim().toLowerCase();
  return Boolean(p && !CLI_ROLE_PROVIDERS.has(p));
}

export function isSameModelSelection(solver, verifier, connectedModels = null) {
  let sp = String(solver?.provider || '').trim().toLowerCase();
  let sm = String(solver?.model || '').trim().toLowerCase();
  const vp = String(verifier?.provider || '').trim().toLowerCase();
  const vm = String(verifier?.model || '').trim().toLowerCase();
  if ((!sp || !sm) && Array.isArray(connectedModels) && connectedModels.length === 1 && vp && vm) {
    sp = String(connectedModels[0]?.provider || '').trim().toLowerCase();
    sm = String(connectedModels[0]?.model || '').trim().toLowerCase();
  }
  if (!sp || !sm) return false;
  if (!vp || !vm) return true;
  return canonicalProviderFamily(sp) === canonicalProviderFamily(vp) && sm === vm;
}

export function defaultTargetForKind(taskKind) {
  if (taskKind === 'prove') {
    return { description: 'Proof of the statement', form: 'proof' };
  }
  if (taskKind === 'check_derivation') {
    return { description: 'Per-step verdict for the supplied derivation', form: 'per_step_verdict' };
  }
  return { description: 'Final calculated expression', form: 'closed_form' };
}

export function defaultCriteriaForKind(taskKind, allowNumericTarget = false) {
  const tier = allowNumericTarget ? 'checked_numerically' : 'verified';
  if (taskKind === 'prove') {
    return [
      { id: 'final_different_model_review', kind: 'final_tier_at_least', params: { tier: 'model_agreed', require_different_model: true }, enabled: true },
      { id: 'closure_model_agreed', kind: 'closure_tier_at_least', params: { tier: 'model_agreed', require_different_model: true }, enabled: true },
      { id: 'no_open_flags', kind: 'no_open_flags', params: {}, enabled: true },
    ];
  }
  if (taskKind === 'check_derivation') {
    return [
      { id: 'closure_checked', kind: 'closure_tier_at_least', params: { tier }, enabled: true },
      { id: 'no_open_flags', kind: 'no_open_flags', params: {}, enabled: true },
      { id: 'confirm_transcriptions', kind: 'user_confirms', params: { text: 'Confirm side-by-side TeX transcriptions' }, enabled: true },
    ];
  }
  return [
    { id: 'final_result_tier', kind: 'final_tier_at_least', params: { tier }, enabled: true },
    { id: 'closure_model_agreed', kind: 'closure_tier_at_least', params: { tier: 'model_agreed' }, enabled: true },
    { id: 'no_open_flags', kind: 'no_open_flags', params: {}, enabled: true },
  ];
}

export function formatTierName(rawTier) {
  switch (String(rawTier || '').trim()) {
    case 'verified': return TIER_VERIFIED_WORD;
    case 'checked_numerically': return TIER_NUMERIC_WORD;
    case 'model_agreed': return TIER_MODEL_AGREED_WORD;
    case 'checked_by_you': return 'Checked by you';
    case 'model_agreed_same_model': return 'Model agreed (same model)';
    default: return String(rawTier || 'Unchecked');
  }
}

export function formatCriterionLabel(criterion) {
  if (!criterion || typeof criterion !== 'object') return '';
  const p = criterion.params || {};
  switch (criterion.kind) {
    case 'final_tier_at_least':
      if (p.require_different_model) return `Final result is ${formatTierName(p.tier)} by a different model`;
      if (p.tier === 'checked_numerically') return `Final result is ${TIER_VERIFIED_WORD} or ${TIER_NUMERIC_WORD}`;
      return `Final result is ${formatTierName(p.tier)}`;
    case 'closure_tier_at_least':
      if (p.require_different_model) return `Everything it rests on is at least ${formatTierName(p.tier)} (different model)`;
      if (p.tier === 'checked_numerically') return `Every step it rests on is ${TIER_VERIFIED_WORD} or ${TIER_NUMERIC_WORD}`;
      return `Everything it rests on is at least ${formatTierName(p.tier)}`;
    case 'named_check_passed': {
      const check = String(p.check || 'check');
      const extra = p.expression ? ` (${p.expression}${p.order_variable ? `, order ${p.order_variable}^${p.order_power ?? 1}` : ''})` : '';
      return `Named check \`${check}\` passes${extra}`;
    }
    case 'no_open_flags':
      return 'No unacknowledged model assumptions or open flags';
    case 'user_confirms':
      return String(p.text || 'User confirmation required');
    default:
      return String(criterion.id || criterion.kind || 'Criterion');
  }
}

const cloneTarget = (t) => (t && typeof t === 'object' ? { ...t } : null);
const cloneSymbols = (list) => (Array.isArray(list) ? list : []).map((s) => ({
  name: String(s?.name || ''),
  assumptions: Array.isArray(s?.assumptions) ? [...s.assumptions] : [],
}));
const cloneDimensions = (list) => (Array.isArray(list) ? list : []).map((d) => ({
  symbol: String(d?.symbol || ''), base: String(d?.base || ''), power: String(d?.power ?? ''),
}));
const sameJson = (a, b) => JSON.stringify(a ?? null) === JSON.stringify(b ?? null);
const sameStringList = (a, b) => Array.isArray(a) && Array.isArray(b) && a.length === b.length && a.every((v, i) => v === b[i]);

const makeUserOwned = (v) => ({
  taskKind: v, target: v, symbols: v, conventions: v, dimensions: v, done: v, budgets: v, review: v,
});

function snapshotPrefillBase(state) {
  return {
    taskKind: state.taskKind, target: cloneTarget(state.target), symbols: cloneSymbols(state.symbols),
    conventions: state.conventions, dimensions: cloneDimensions(state.dimensions),
    budgets: { ...state.budgets }, review: state.review,
  };
}

export function markFieldOwned(state, field) {
  if (!state) return;
  if (!state.userOwned) state.userOwned = makeUserOwned(false);
  state.userOwned[field] = true;
}

export function markSymbolRemoved(state, name) {
  const norm = String(name || '').trim();
  if (!state || !norm) return;
  if (!Array.isArray(state.removedSymbols)) state.removedSymbols = [];
  if (!state.removedSymbols.includes(norm)) state.removedSymbols.push(norm);
  if (Array.isArray(state.editedSymbols)) state.editedSymbols = state.editedSymbols.filter((n) => n !== norm);
}

export function markSymbolEdited(state, name, prevName = '') {
  if (!state) return;
  const prev = String(prevName || '').trim();
  const next = String(name || '').trim();
  if (prev && prev !== next && (state.prefillBase?.symbols || []).some((s) => String(s?.name || '').trim() === prev)) {
    markSymbolRemoved(state, prev);
  }
  if (!next) return;
  if (!Array.isArray(state.editedSymbols)) state.editedSymbols = [];
  if (!state.editedSymbols.includes(next)) state.editedSymbols.push(next);
  if (Array.isArray(state.removedSymbols)) state.removedSymbols = state.removedSymbols.filter((n) => n !== next);
}

export function markDoneToggled(state, criterionId, enabled) {
  if (!state || !criterionId) return;
  if (!state.doneEnabledById || typeof state.doneEnabledById !== 'object') state.doneEnabledById = {};
  state.doneEnabledById[criterionId] = Boolean(enabled);
}

export function switchTaskKind(state, kindId) {
  if (!state || !kindId) return false;
  markFieldOwned(state, 'taskKind');
  if (state.taskKind === kindId) return false;
  state.taskKind = kindId;
  state.target = defaultTargetForKind(kindId);
  state.userOwned.target = false;
  if (state.prefillBase) state.prefillBase.target = cloneTarget(state.target);
  state.proveNotice = kindId === 'prove' ? PROVE_PROSE_NOTICE : null;
  state.done = defaultCriteriaForKind(kindId, state.allowNumericTarget);
  state.userOwned.done = false;
  state.doneEnabledById = {};
  return true;
}

export function prefillTaskKindOverride(state) {
  if (!state) return null;
  return (state.userOwned?.taskKind || (state.prefillBase && state.taskKind !== state.prefillBase.taskKind))
    ? (state.taskKind || null) : null;
}

export function setupCardSig(state) {
  if (!state) return '';
  return JSON.stringify([
    state.phase, Boolean(state.setup), Boolean(state.modelConnected), state.toolbox?.line || '',
    state.taskKind || 'calculate', state.proveNotice || null, state.statement || '',
    state.symbols || [], state.conventions || '', state.dimensions || [],
    Boolean(state.allowNumericTarget), Boolean(state.everyStepVerified), state.done || [],
    state.connectedModels || [], state.solver || null, state.verifier || null,
    Boolean(state.sameModel), state.sameModelWarning || null, Boolean(state.acceptSameModel),
    state.approvalGrants?.pre_turn_approver_supported !== false, state.approvalGrants?.grants || [],
    Boolean(state.askBeforeRunning), state.error || '',
  ]);
}

export function createInitialSolveState(initial = {}) {
  const hasSetup = Boolean(initial.setup);
  const taskKind = initial.taskKind || 'calculate';
  const target = initial.target ? { ...initial.target } : defaultTargetForKind(taskKind);
  const symbols = cloneSymbols(initial.symbols);
  const conventions = String(initial.conventions || '');
  const dimensions = dimensionsObjectToRows(initial.dimensions);
  const done = Array.isArray(initial.done)
    ? initial.done.map((c) => ({ ...c, enabled: c.enabled !== false }))
    : defaultCriteriaForKind(taskKind, Boolean(initial.allowNumericTarget));
  const budgets = { ...DEFAULT_SOLVE_BUDGETS, ...(initial.budgets || {}) };
  const review = initial.review || 'prose_only';
  const solver = initial.solver || { provider: '', model: '' };
  const verifier = initial.verifier || { provider: '', model: '' };
  const connectedModels = Array.isArray(initial.connectedModels) ? initial.connectedModels : [];
  const sameModel = isSameModelSelection(solver, verifier, connectedModels);
  const doneEnabledById = {};
  for (const c of done) if (c?.id && c.enabled === false) doneEnabledById[c.id] = false;
  const userOwned = {
    taskKind: hasSetup || initial.taskKind !== undefined,
    target: hasSetup || initial.target !== undefined,
    symbols: hasSetup || symbols.length > 0,
    conventions: hasSetup || conventions.length > 0,
    dimensions: hasSetup || dimensions.length > 0,
    done: hasSetup || Array.isArray(initial.done),
    budgets: hasSetup || Boolean(initial.budgets),
    review: hasSetup || initial.review !== undefined,
  };
  const state = {
    phase: hasSetup ? 'collapsed' : 'setup',
    statement: String(initial.statement || ''),
    taskKind, target, symbols, conventions, dimensions,
    assumptions: Array.isArray(initial.assumptions)
      ? initial.assumptions.map((a) => ({ id: String(a.id || ''), text: String(a.text || '') })) : [],
    done,
    allowNumericTarget: Boolean(initial.allowNumericTarget),
    acceptSameModel: Boolean(initial.acceptSameModel),
    everyStepVerified: Boolean(initial.everyStepVerified),
    solver: { provider: String(solver.provider || ''), model: String(solver.model || '') },
    verifier: { provider: String(verifier.provider || ''), model: String(verifier.model || '') },
    solverTouched: Boolean(initial.solverTouched || (hasSetup && (solver.provider || solver.model))),
    verifierTouched: Boolean(initial.verifierTouched || (hasSetup && (verifier.provider || verifier.model))),
    sameModel,
    sameModelWarning: sameModel ? SAME_MODEL_WARNING : null,
    proveNotice: taskKind === 'prove' ? PROVE_PROSE_NOTICE : null,
    budgets, review,
    askBeforeRunning: Boolean(initial.askBeforeRunning),
    toolbox: initial.toolbox || { available: false, line: 'Toolbox: checking…' },
    approvalGrants: initial.approvalGrants || { pre_turn_approver_supported: false, grants: [] },
    modelConnected: initial.modelConnected !== undefined ? Boolean(initial.modelConnected) : false,
    connectedModels,
    setup: initial.setup || null,
    steps: Array.isArray(initial.steps) ? initial.steps : [],
    graph: initial.graph || { nodes: [], edges: [], legend: [] },
    handles: Array.isArray(initial.handles) ? initial.handles : [],
    checks: Array.isArray(initial.checks) ? initial.checks : [],
    receipts: Array.isArray(initial.receipts) ? initial.receipts : [],
    artifacts: Array.isArray(initial.artifacts) ? initial.artifacts : [],
    doneResult: initial.doneResult || null,
    plan: initial.plan || null,
    runState: initial.runState || { state: 'ready', label: 'Ready', model_calls_used: 0, model_calls_max: 60 },
    userOwned, removedSymbols: [], editedSymbols: [], doneEnabledById, error: '', recheckNotice: '',
  };
  state.prefillBase = snapshotPrefillBase(state);
  return state;
}

export function dimensionsObjectToRows(dimsObj) {
  if (!dimsObj || typeof dimsObj !== 'object' || Array.isArray(dimsObj)) return [];
  const rows = [];
  for (const [symbol, bases] of Object.entries(dimsObj)) {
    if (!bases || typeof bases !== 'object') continue;
    for (const [base, power] of Object.entries(bases)) {
      rows.push({ symbol: String(symbol), base: String(base), power: String(power) });
    }
  }
  return rows;
}

export function dimensionRowsToObject(rows) {
  const out = {};
  for (const row of Array.isArray(rows) ? rows : []) {
    const sym = String(row?.symbol || '').trim();
    const base = String(row?.base || '').trim();
    const rawPow = String(row?.power ?? '').trim();
    if (!sym || !base || !rawPow) continue;
    if (!out[sym]) out[sym] = {};
    out[sym][base] = /^-?\d+$/.test(rawPow) ? Number(rawPow) : rawPow;
  }
  return out;
}

function mergePrefillSymbols(state, incoming, baseSymbols) {
  const removed = new Set((state.removedSymbols || []).map((n) => String(n || '').trim()).filter(Boolean));
  const edited = new Set((state.editedSymbols || []).map((n) => String(n || '').trim()).filter(Boolean));
  if (state.userOwned?.symbols) {
    return { symbols: cloneSymbols(state.symbols), removedSymbols: [...removed], editedSymbols: [...edited] };
  }
  const baseByName = new Map();
  for (const b of baseSymbols) {
    const name = String(b?.name || '').trim();
    if (name) baseByName.set(name, Array.isArray(b.assumptions) ? b.assumptions : []);
  }
  const curNames = new Set((state.symbols || []).map((s) => String(s?.name || '').trim()).filter(Boolean));
  for (const bName of baseByName.keys()) if (!curNames.has(bName)) removed.add(bName);
  const incomingByName = new Map();
  for (const inc of incoming) {
    const name = String(inc?.name || '').trim();
    if (name && !incomingByName.has(name)) incomingByName.set(name, inc);
  }
  const merged = [];
  const mergedNames = new Set();
  for (const cur of Array.isArray(state.symbols) ? state.symbols : []) {
    const name = String(cur?.name || '').trim();
    const curAssumps = Array.isArray(cur?.assumptions) ? [...cur.assumptions] : [];
    if (!name) { merged.push({ name: cur?.name || '', assumptions: curAssumps }); continue; }
    const baseAssumps = baseByName.get(name);
    if (!baseAssumps || edited.has(name) || !sameStringList(curAssumps, baseAssumps)) {
      edited.add(name);
      merged.push({ name, assumptions: curAssumps });
      mergedNames.add(name);
    } else if (incomingByName.has(name)) {
      merged.push({ name, assumptions: [...incomingByName.get(name).assumptions] });
      mergedNames.add(name);
    }
  }
  for (const inc of incoming) {
    const name = String(inc?.name || '').trim();
    if (!name || mergedNames.has(name) || removed.has(name)) continue;
    merged.push({ name, assumptions: [...inc.assumptions] });
    mergedNames.add(name);
  }
  return { symbols: merged, removedSymbols: [...removed], editedSymbols: [...edited] };
}

export function applyPrefillToState(state, resp) {
  if (!resp || typeof resp !== 'object') return state;
  const p = resp.prefill || resp;
  const owned = { ...makeUserOwned(false), ...(state.userOwned || {}) };
  const base = state.prefillBase ? { ...state.prefillBase } : snapshotPrefillBase(state);
  const pickField = (key, cur, hasIncoming, incomingVal, cloneFn = (v) => v) => {
    const isOwned = owned[key] || !sameJson(cur, base[key]);
    if (isOwned) owned[key] = true;
    if (!isOwned && hasIncoming) {
      base[key] = cloneFn(incomingVal);
      return incomingVal;
    }
    return cur;
  };

  const taskKind = pickField('taskKind', state.taskKind, Boolean(p.task_kind), p.task_kind);
  const kindMatches = !p.task_kind || p.task_kind === taskKind;
  const target = pickField('target', state.target, Boolean(kindMatches && p.target), p.target ? { ...p.target } : null, cloneTarget);
  const conventions = pickField('conventions', state.conventions, typeof p.conventions === 'string', p.conventions);
  const incomingDims = p.dimensions ? dimensionsObjectToRows(p.dimensions) : null;
  const dimensions = pickField('dimensions', state.dimensions, Boolean(incomingDims), incomingDims, cloneDimensions);
  const budgets = pickField('budgets', state.budgets, Boolean(p.budgets), p.budgets ? { ...DEFAULT_SOLVE_BUDGETS, ...p.budgets } : null, (b) => ({ ...b }));
  const review = pickField('review', state.review, Boolean(p.review), p.review);

  let symbols = state.symbols;
  let removedSymbols = [...(state.removedSymbols || [])];
  let editedSymbols = [...(state.editedSymbols || [])];
  if (Array.isArray(p.symbols)) {
    const incoming = cloneSymbols(p.symbols);
    const merged = mergePrefillSymbols(state, incoming, base.symbols || []);
    symbols = merged.symbols;
    removedSymbols = merged.removedSymbols;
    editedSymbols = merged.editedSymbols;
    base.symbols = incoming;
  }

  const doneEnabledById = { ...(state.doneEnabledById || {}) };
  for (const c of Array.isArray(state.done) ? state.done : []) {
    if (c?.id && c.enabled === false) doneEnabledById[c.id] = false;
  }
  const done = (!owned.done && kindMatches && Array.isArray(p.done))
    ? p.done.map((c) => ({ ...c, params: c.params ? { ...c.params } : {}, enabled: !(c?.id && doneEnabledById[c.id] === false) }))
    : state.done;

  const next = {
    ...state,
    taskKind, target, symbols, conventions, dimensions, done, budgets, review,
    userOwned: owned, prefillBase: base, removedSymbols, editedSymbols, doneEnabledById,
    proveNotice: taskKind === 'prove' ? (p.prove_notice || PROVE_PROSE_NOTICE) : null,
    toolbox: resp.toolbox || state.toolbox,
    approvalGrants: resp.approval_grants || state.approvalGrants,
    modelConnected: (typeof resp.model_connected === 'boolean' ? resp.model_connected : state.modelConnected)
      || (Array.isArray(state.connectedModels) && state.connectedModels.length > 0),
    error: '',
  };
  if (!state.solverTouched && isInCoreRole(resp.roles?.solver)) next.solver = { ...resp.roles.solver };
  if (!state.verifierTouched && isInCoreRole(resp.roles?.verifier)) next.verifier = { ...resp.roles.verifier };
  next.sameModel = isSameModelSelection(next.solver, next.verifier, next.connectedModels);
  next.sameModelWarning = next.sameModel ? (p.same_model_warning || SAME_MODEL_WARNING) : null;
  return next;
}

export function applySolveViewToState(state, view) {
  if (!view || typeof view !== 'object') return state;
  const setup = view.setup !== undefined ? (view.setup || null) : state.setup;
  const connectedModels = Array.isArray(view.connected_models) ? view.connected_models : state.connectedModels;
  const next = {
    ...state, setup,
    steps: Array.isArray(view.steps) ? view.steps : state.steps,
    graph: view.graph || state.graph,
    handles: Array.isArray(view.handles) ? view.handles : state.handles,
    checks: Array.isArray(view.checks) ? view.checks : state.checks,
    receipts: Array.isArray(view.receipts) ? view.receipts : state.receipts,
    artifacts: Array.isArray(view.artifacts) ? view.artifacts : state.artifacts,
    doneResult: view.done !== undefined ? view.done : state.doneResult,
    plan: view.plan !== undefined ? view.plan : state.plan,
    runState: view.run_state || state.runState,
    toolbox: view.toolbox || state.toolbox,
    approvalGrants: view.approval_grants || state.approvalGrants,
    modelConnected: (typeof view.model_connected === 'boolean' ? view.model_connected : state.modelConnected)
      || (Array.isArray(connectedModels) && connectedModels.length > 0),
    connectedModels,
  };
  if (view.setup !== undefined && setup && state.phase !== 'revising') {
    next.phase = 'collapsed';
    next.statement = String(setup.statement || '');
    next.taskKind = setup.task_kind || 'calculate';
    next.target = setup.target ? { ...setup.target } : defaultTargetForKind(next.taskKind);
    next.conventions = String(setup.conventions || '');
    next.symbols = cloneSymbols(setup.symbols);
    next.dimensions = dimensionsObjectToRows(setup.dimensions);
    next.assumptions = Array.isArray(setup.assumptions)
      ? setup.assumptions.map((a) => ({ id: String(a.id || ''), text: String(a.text || '') })) : [];
    next.done = Array.isArray(setup.done)
      ? setup.done.map((c) => ({ ...c, params: c.params ? { ...c.params } : {}, enabled: true })) : [];
    if (setup.roles?.solver) {
      next.solver = { provider: String(setup.roles.solver.provider || ''), model: String(setup.roles.solver.model || '') };
      if (next.solver.provider || next.solver.model) next.solverTouched = true;
    }
    if (setup.roles?.verifier) {
      next.verifier = { provider: String(setup.roles.verifier.provider || ''), model: String(setup.roles.verifier.model || '') };
      if (next.verifier.provider || next.verifier.model) next.verifierTouched = true;
    }
    next.sameModel = isSameModelSelection(next.solver, next.verifier, connectedModels);
    next.sameModelWarning = next.sameModel ? SAME_MODEL_WARNING : null;
    next.budgets = setup.budgets ? { ...DEFAULT_SOLVE_BUDGETS, ...setup.budgets } : next.budgets;
    next.review = setup.review || next.review;
    next.askBeforeRunning = Boolean(setup.ask_before_running);
    next.allowNumericTarget = next.done.some(
      (c) => (c.id === 'final_result_tier' || c.id === 'closure_checked') && c.params?.tier === 'checked_numerically',
    );
    next.everyStepVerified = next.done.some((c) => c.id === 'every_step_verified');
    next.acceptSameModel = next.sameModel && next.done.some(
      (c) => (c.kind === 'final_tier_at_least' || c.kind === 'closure_tier_at_least')
        && c.params?.tier === 'model_agreed_same_model',
    );
    next.proveNotice = next.taskKind === 'prove' ? PROVE_PROSE_NOTICE : null;
    next.userOwned = makeUserOwned(true);
    next.prefillBase = snapshotPrefillBase(next);
    next.removedSymbols = [];
    next.editedSymbols = [];
    next.doneEnabledById = {};
  }
  return next;
}

export function buildSetupSubmission(state) {
  let enabledDone = (Array.isArray(state.done) ? state.done : [])
    .filter((c) => c && c.enabled !== false)
    .map(({ id, kind, params }) => ({
      id: String(id || ''), kind: String(kind || ''),
      params: params && typeof params === 'object' ? { ...params } : {},
    }));
  if (state.everyStepVerified) {
    if (!enabledDone.some((c) => c.id === 'every_step_verified')) {
      enabledDone.push({ id: 'every_step_verified', kind: 'closure_tier_at_least', params: { tier: 'verified' } });
    }
  } else {
    enabledDone = enabledDone.filter((c) => c.id !== 'every_step_verified');
  }
  if (state.sameModel && state.acceptSameModel) {
    for (const c of enabledDone) {
      if (c.params?.require_different_model) c.params.require_different_model = false;
      if ((c.kind === 'final_tier_at_least' || c.kind === 'closure_tier_at_least') && c.params?.tier === 'model_agreed') {
        c.params.tier = 'model_agreed_same_model';
      }
    }
  }
  const fallbackTarget = defaultTargetForKind(state.taskKind);
  const target = {
    description: String(state.target?.description || fallbackTarget.description),
    form: state.target?.form || fallbackTarget.form,
  };
  if (state.target?.order != null && String(state.target.order).trim() !== '') target.order = String(state.target.order);
  if (state.target?.digits != null) target.digits = Number(state.target.digits);
  return {
    statement: String(state.statement || '').trim(),
    task_kind: state.taskKind || 'calculate',
    target,
    done: enabledDone,
    conventions: String(state.conventions || '').trim(),
    symbols: (Array.isArray(state.symbols) ? state.symbols : [])
      .filter((s) => String(s?.name || '').trim())
      .map((s) => ({
        name: String(s.name).trim(),
        assumptions: (Array.isArray(s.assumptions) ? s.assumptions : []).map((a) => String(a).trim()).filter(Boolean),
      })),
    dimensions: dimensionRowsToObject(state.dimensions),
    assumptions: (Array.isArray(state.assumptions) ? state.assumptions : [])
      .filter((a) => String(a?.text || '').trim())
      .map((a, i) => ({ id: String(a.id || `a-${i + 1}`).trim(), text: String(a.text).trim() })),
    roles: {
      solver: { provider: String(state.solver?.provider || ''), model: String(state.solver?.model || '') },
      verifier: { provider: String(state.verifier?.provider || ''), model: String(state.verifier?.model || '') },
    },
    budgets: { ...DEFAULT_SOLVE_BUDGETS, ...(state.budgets || {}) },
    review: state.review || 'prose_only',
    tool_profile: 'solve/1',
    ask_before_running: Boolean(state.askBeforeRunning),
  };
}

export function summarizeCollapsedSetup(setup) {
  if (!setup) return 'No setup recorded yet.';
  const kindMeta = TASK_KINDS.find((k) => k.id === setup.task_kind);
  const kindLabel = kindMeta ? kindMeta.label : String(setup.task_kind || 'Solve');
  const rev = setup.revision ? `rev ${setup.revision}` : 'draft';
  const stmt = String(setup.statement || '').replace(/\s+/g, ' ').trim();
  const clipped = stmt.length > 96 ? `${stmt.slice(0, 95)}…` : stmt;
  const symCount = Array.isArray(setup.symbols) ? setup.symbols.length : 0;
  return `${kindLabel} (${rev}, ${symCount} symbol${symCount === 1 ? '' : 's'}): ${clipped}`;
}

export function buildLocalGivenPreviewSteps(payload) {
  const out = [];
  let n = 0;
  const pushGiven = (summary, details) => {
    n += 1;
    out.push({
      entry_id: `e-${n}`, entry_type: 'assumption', summary, details, origin: 'setup',
      badge: { label: TIER_GIVEN_WORD, provenance_tag: 'you', grade: 'user_statement', border: 'plain', flags: [] },
    });
  };
  if (payload.conventions) pushGiven('Setup conventions', payload.conventions);
  if (payload.symbols?.length) {
    pushGiven('Symbol assumptions', payload.symbols.map((s) => `${s.name}: ${s.assumptions.join(', ')}`).join('; '));
  }
  for (const a of payload.assumptions || []) pushGiven(`Assumption (${a.id})`, a.text);
  return out;
}
