// Field-section renderers and DOM primitives for the Solve setup card (split from solve_setup.js).
// Zero modal dialogs; works with zero model connected; zero `innerHTML` writes.

import { katexOptions } from './math_katex.js';
import { extractMath } from './math_scan.js';
import {
  SAME_MODEL_WARNING,
  SYMBOL_ASSUMPTION_PRESETS,
  TIER_NUMERIC_WORD,
  TIER_VERIFIED_WORD,
  formatCriterionLabel,
  markDoneToggled,
  markFieldOwned,
  markSymbolEdited,
  markSymbolRemoved,
} from './solve_state.js';

export function el(tag, className, text) {
  const node = document.createElement(tag);
  if (className) node.className = className;
  if (text !== undefined && text !== null) node.textContent = String(text);
  return node;
}

export function textInput(cls, placeholder, value, onInput, ariaLabel = null) {
  const inp = el('input', cls);
  inp.type = 'text';
  inp.placeholder = placeholder;
  inp.value = value;
  if (ariaLabel) inp.setAttribute('aria-label', ariaLabel);
  inp.addEventListener('input', (ev) => onInput(ev.target.value));
  return inp;
}

export function checkboxRow(labelText, checked, testId, onChange) {
  const row = el('label', 'solve-check-row');
  const box = el('input');
  box.type = 'checkbox';
  box.checked = Boolean(checked);
  if (testId) box.setAttribute('data-testid', testId);
  box.addEventListener('change', (ev) => onChange(Boolean(ev.target.checked)));
  row.append(box, el('span', '', labelText));
  return row;
}

// Live statement math preview with zero innerHTML (uses katex.render + textContent only).
export function renderSafeMathPreview(container, rawText, katexImpl = globalThis.window?.katex) {
  if (!container) return;
  const text = String(rawText ?? '');
  container.replaceChildren();
  if (!text.trim()) {
    container.append(el('span', 'solve-preview-empty', 'Live math preview appears here as you type.'));
    return;
  }
  const tokens = [];
  const marked = extractMath(text.replace(/\u0000/g, ''), tokens, {
    protectInlineCode: true,
    allowSingleDollar: true,
  });
  let last = 0;
  for (const m of marked.matchAll(/\u0000M(\d+)\u0000/g)) {
    const plain = marked.slice(last, m.index);
    if (plain) container.append(el('span', 'solve-preview-text', plain));
    const tok = tokens[Number(m[1])];
    if (tok) {
      const mathEl = el('span', tok.display ? 'solve-preview-math is-display' : 'solve-preview-math');
      if (katexImpl && typeof katexImpl.render === 'function') {
        try {
          katexImpl.render(tok.tex, mathEl, katexOptions(tok.display));
        } catch {
          mathEl.textContent = tok.tex;
        }
      } else {
        mathEl.textContent = tok.tex;
      }
      container.append(mathEl);
    }
    last = m.index + m[0].length;
  }
  const tail = marked.slice(last);
  if (tail) container.append(el('span', 'solve-preview-text', tail));
}

export function buildModelOptions(selectEl, models, current) {
  selectEl.replaceChildren();
  const noneOpt = el('option', '', 'Not selected');
  noneOpt.value = '';
  selectEl.append(noneOpt);
  const currentKey = current?.provider && current?.model ? `${current.provider}::${current.model}` : '';
  let seenCurrent = false;
  for (const item of Array.isArray(models) ? models : []) {
    const key = `${item.provider}::${item.model}`;
    if (key === currentKey) seenCurrent = true;
    const opt = el('option', '', item.label || `${item.provider} / ${item.model}`);
    opt.value = key;
    if (key === currentKey) opt.selected = true;
    selectEl.append(opt);
  }
  if (currentKey && !seenCurrent) {
    const opt = el('option', '', `${current.provider} / ${current.model}`);
    opt.value = currentKey;
    opt.selected = true;
    selectEl.append(opt);
  }
  selectEl.value = currentKey;
}

export function parseModelKey(val) {
  const s = String(val || '');
  const idx = s.indexOf('::');
  if (idx < 0) return { provider: '', model: '' };
  return { provider: s.slice(0, idx), model: s.slice(idx + 2) };
}

export function renderSymbolsSection(card, state, render) {
  const sec = el('div', 'solve-field');
  const head = el('div', 'solve-field-head');
  head.append(el('label', 'solve-label', 'Symbols and assumptions'));
  const addBtn = el('button', 'pill-btn sm', '+ Add symbol');
  addBtn.type = 'button';
  addBtn.setAttribute('data-action', 'add-symbol');
  addBtn.addEventListener('click', () => {
    state.symbols.push({ name: '', assumptions: ['real'] });
    render();
  });
  head.append(addBtn);
  sec.append(head);

  const table = el('div', 'solve-table');
  table.setAttribute('data-testid', 'solve-symbols-table');
  if (state.symbols.length === 0) {
    table.append(el('div', 'solve-muted', 'Detected symbols from your statement appear here.'));
  }
  state.symbols.forEach((sym, idx) => {
    const row = el('div', 'solve-table-row');
    const nameIn = el('input', 'text-input solve-sym-name');
    nameIn.type = 'text';
    nameIn.value = sym.name;
    nameIn.placeholder = 'symbol';
    nameIn.setAttribute('aria-label', `Symbol ${idx + 1} name`);
    nameIn.addEventListener('input', (ev) => {
      const prev = sym.name;
      sym.name = ev.target.value;
      markSymbolEdited(state, sym.name, prev);
    });

    const assumpSel = el('select', 'compact-select solve-sym-assump');
    assumpSel.setAttribute('aria-label', `Symbol ${idx + 1} assumptions`);
    const curText = sym.assumptions.join(', ') || 'real';
    const presets = SYMBOL_ASSUMPTION_PRESETS.includes(curText)
      ? SYMBOL_ASSUMPTION_PRESETS
      : [curText, ...SYMBOL_ASSUMPTION_PRESETS];
    for (const preset of presets) {
      const opt = el('option', '', preset);
      opt.value = preset;
      if (preset === curText) opt.selected = true;
      assumpSel.append(opt);
    }
    assumpSel.value = curText;
    assumpSel.addEventListener('change', (ev) => {
      sym.assumptions = String(ev.target.value || '').split(',').map((x) => x.trim()).filter(Boolean);
      markSymbolEdited(state, sym.name);
    });

    const delBtn = el('button', 'pill-btn sm', 'Remove');
    delBtn.type = 'button';
    delBtn.setAttribute('aria-label', `Remove symbol ${idx + 1}`);
    delBtn.addEventListener('click', () => {
      markSymbolRemoved(state, sym.name);
      state.symbols.splice(idx, 1);
      render();
    });
    row.append(nameIn, assumpSel, delBtn);
    table.append(row);
  });
  sec.append(table);
  card.append(sec);
}

export function renderDimensionsSection(card, state, render) {
  const sec = el('div', 'solve-field');
  const head = el('div', 'solve-field-head');
  head.append(el('label', 'solve-label', 'Dimensions (optional)'));
  const addBtn = el('button', 'pill-btn sm', '+ Add dimension');
  addBtn.type = 'button';
  addBtn.setAttribute('data-action', 'add-dimension');
  addBtn.addEventListener('click', () => {
    state.dimensions.push({ symbol: '', base: 'mass', power: '1' });
    markFieldOwned(state, 'dimensions');
    render();
  });
  head.append(addBtn);
  sec.append(head);

  const table = el('div', 'solve-table');
  table.setAttribute('data-testid', 'solve-dimensions-table');
  state.dimensions.forEach((dim, idx) => {
    const row = el('div', 'solve-table-row');
    const symIn = textInput('text-input', 'symbol', dim.symbol, (v) => {
      dim.symbol = v;
      markFieldOwned(state, 'dimensions');
    }, `Dimension ${idx + 1} symbol`);
    const baseIn = textInput('text-input', 'base (e.g. mass)', dim.base, (v) => {
      dim.base = v;
      markFieldOwned(state, 'dimensions');
    }, `Dimension ${idx + 1} base`);
    const powIn = textInput('text-input', 'power (e.g. 1)', dim.power, (v) => {
      dim.power = v;
      markFieldOwned(state, 'dimensions');
    }, `Dimension ${idx + 1} power`);
    const delBtn = el('button', 'pill-btn sm', 'Remove');
    delBtn.type = 'button';
    delBtn.setAttribute('aria-label', `Remove dimension ${idx + 1}`);
    delBtn.addEventListener('click', () => {
      state.dimensions.splice(idx, 1);
      markFieldOwned(state, 'dimensions');
      render();
    });
    row.append(symIn, baseIn, powIn, delBtn);
    table.append(row);
  });
  sec.append(table);
  card.append(sec);
}

export function renderChecklistSection(card, state, render) {
  const sec = el('div', 'solve-field');
  sec.append(el('label', 'solve-label', 'Done checklist'));

  if (state.taskKind !== 'prove') {
    const tierRow = el('div', 'solve-tier-toggle');
    tierRow.append(
      checkboxRow(
        `Allow ${TIER_NUMERIC_WORD} (instead of requiring exact ${TIER_VERIFIED_WORD})`,
        state.allowNumericTarget,
        'solve-allow-numeric-toggle',
        (checked) => {
          state.allowNumericTarget = checked;
          const targetTier = checked ? 'checked_numerically' : 'verified';
          for (const item of state.done) {
            if (item.id === 'final_result_tier' || item.id === 'closure_checked') {
              item.params = { ...(item.params || {}), tier: targetTier };
            }
          }
          render();
        },
      ),
      checkboxRow(`Every step ${TIER_VERIFIED_WORD}`, state.everyStepVerified, 'solve-every-step-verified', (c) => {
        state.everyStepVerified = c;
      }),
    );
    sec.append(tierRow);
  }

  const list = el('div', 'solve-checklist');
  list.setAttribute('data-testid', 'solve-done-checklist');
  state.done.forEach((crit) => {
    const testId = crit?.id ? `solve-done-${crit.id}` : null;
    list.append(checkboxRow(formatCriterionLabel(crit), crit.enabled !== false, testId, (c) => {
      crit.enabled = c;
      markDoneToggled(state, crit?.id, c);
    }));
  });
  sec.append(list);
  card.append(sec);
}

export function renderModelsSection(card, state, updateSameModelFlags, render) {
  const modelSec = el('div', 'solve-models-grid');
  const modelField = (label, testId, current, assign) => {
    const wrap = el('div', 'solve-field');
    wrap.append(el('label', 'solve-label', label));
    const sel = el('select', 'compact-select');
    sel.setAttribute('data-testid', testId);
    buildModelOptions(sel, state.connectedModels, current);
    sel.addEventListener('change', (ev) => {
      assign(parseModelKey(ev.target.value));
      updateSameModelFlags();
      render();
    });
    wrap.append(sel);
    return wrap;
  };
  modelSec.append(
    modelField('Solver model', 'solve-solver-select', state.solver, (v) => { state.solver = v; state.solverTouched = true; }),
    modelField('Verifier model', 'solve-verifier-select', state.verifier, (v) => { state.verifier = v; state.verifierTouched = true; }),
  );
  card.append(modelSec);

  if (state.sameModel) {
    const warn = el('div', 'solve-warning-banner');
    warn.setAttribute('data-testid', 'solve-same-model-warning');
    warn.append(
      el('span', '', state.sameModelWarning || SAME_MODEL_WARNING),
      checkboxRow('Accept same-model review', state.acceptSameModel, 'solve-accept-same-model', (c) => {
        state.acceptSameModel = c;
      }),
    );
    card.append(warn);
  }
}
