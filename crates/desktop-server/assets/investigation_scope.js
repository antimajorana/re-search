/**
 * Pure formatting and DOM controller for the question-scope chip in Investigation mode.
 * All text from paper headings, labels, and questions is inserted strictly via textContent.
 */

function formatRanges(ranges) {
  if (!Array.isArray(ranges) || !ranges.length) return '';
  return ranges
    .filter(r => Array.isArray(r) && r.length === 2 && Number.isFinite(r[0]) && Number.isFinite(r[1]))
    .map(([a, b]) => (a === b ? 'line ' + a : 'lines ' + a + '–' + b))
    .join(', ');
}

function formatPages(targets) {
  const starts = [];
  const ends = [];
  for (const t of targets || []) {
    if (Number.isFinite(t?.page_start)) starts.push(t.page_start);
    if (Number.isFinite(t?.page_end)) ends.push(t.page_end);
  }
  if (!starts.length || !ends.length) return '';
  const a = Math.min(...starts);
  const b = Math.max(...ends);
  return a === b ? 'p. ' + a : 'pp. ' + a + '–' + b;
}

export function scopeChipData(scope) {
  if (!scope || scope.status !== 'resolved') return null;
  const targets = Array.isArray(scope.targets) ? scope.targets : [];
  const targetNames = targets.map(t => String(t?.label || '').trim()).filter(Boolean);
  const primary = targetNames.length
    ? targetNames.join(', ')
    : String(scope.label || '').trim();
  if (!primary) return null;
  const lineText = formatRanges(scope.ranges);
  const pageText = formatPages(targets);
  const parts = [lineText, pageText].filter(Boolean);
  const detail = parts.join(' · ');
  const titles = targets
    .map(t => (t?.title ? String(t.label || '') + ': ' + String(t.title) : ''))
    .filter(Boolean);
  return {
    text: 'Scoped to ' + primary,
    detail,
    tooltip: titles.length ? titles.join(' | ') : 'Reading scoped to ' + primary,
  };
}

export function createScopeChip(anchorEl) {
  const el = document.createElement('div');
  el.id = 'investigation-scope-chip';
  el.className = 'investigation-scope-chip';
  el.setAttribute('role', 'status');
  el.hidden = true;
  if (typeof anchorEl?.after === 'function') anchorEl.after(el);

  let signature = '';

  function reset() {
    signature = '';
    el.hidden = true;
    el.replaceChildren();
    el.removeAttribute?.('title');
  }

  function update(scope) {
    const data = scopeChipData(scope);
    const next = data ? JSON.stringify(data) : '';
    if (next === signature) return;
    signature = next;
    el.replaceChildren();
    if (!data) {
      el.hidden = true;
      el.removeAttribute?.('title');
      return;
    }
    const label = document.createElement('strong');
    label.className = 'investigation-scope-chip-label';
    label.textContent = data.text;
    el.append(label);
    if (data.detail) {
      const detail = document.createElement('span');
      detail.className = 'investigation-scope-chip-detail';
      detail.textContent = data.detail;
      el.append(document.createTextNode(' · '), detail);
    }
    el.setAttribute('title', data.tooltip);
    el.hidden = false;
  }

  return { reset, update, element: el };
}
