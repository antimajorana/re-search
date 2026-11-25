import { renderMathTextInto } from '/assets/math.js';

const categories = {
  notation: 'Notation', terminology: 'Terminology', units: 'Units', normalization: 'Normalization',
  sign: 'Signs', coordinates: 'Coordinates', indexing: 'Indices', ordering: 'Ordering',
  orientation: 'Orientation', algebra: 'Algebra', geometry: 'Geometry', reporting: 'Measurement reporting', other: 'Other conventions',
};

function textElement(tag, text, className) {
  const el = document.createElement(tag);
  if (className) el.className = className;
  el.textContent = text;
  return el;
}

function mathElement(tag, text) {
  const el = document.createElement(tag);
  renderMathTextInto(el, text);
  return el;
}

export function filterConventions(items, query) {
  const term = query.trim().toLocaleLowerCase();
  if (!term) return items;
  return items.filter(item => [item.label, item.meaning, item.scope, item.rationale, categories[item.category]]
    .some(value => String(value || '').toLocaleLowerCase().includes(term)));
}

export function pendingCitationsLabel(count) {
  return count + (count === 1 ? ' citation pending' : ' citations pending');
}

export function conventionStatus(catalog, requested, running, selectedPages, current = true) {
  const count = catalog?.items?.length || 0;
  const pending = Array.isArray(catalog?.pending_items) ? catalog.pending_items.length : 0;
  const scope = selectedPages ? 'selected pages' : 'paper';
  if (!catalog && requested) return running ? 'Reading the paper…' : 'Review stopped before the first batch';
  if (!catalog) return 'No catalog recorded';
  if (!current) return (count ? count + (count === 1 ? ' entry' : ' entries') + ' from an earlier reading' : 'Earlier reading') +
    (requested && running ? ' · reading again…' : '');
  const suffix = pending ? ' · ' + pendingCitationsLabel(pending) : '';
  if (!catalog.done) return (running ? (count ? count + ' found so far' : 'Reading the paper…') : 'Incomplete catalog') + suffix;
  if (!count) return (catalog.coverage === 'partial' ? 'None located in the reviewed passages' : 'None reported in the ' + scope) + suffix;
  return count + (count === 1 ? ' entry' : ' entries') + (catalog.coverage === 'partial' ? ' · partial coverage' : '') + suffix;
}

export function createConventionCatalog(host, renderAnchors, find) {
  const title = host.querySelector('summary');
  const notes = document.createElement('div');
  const search = textElement('input', '', 'text-input');
  search.type = 'search'; search.placeholder = 'Find notation, meaning or scope';
  search.setAttribute('aria-label', 'Find a convention');
  const sections = document.createElement('div');
  const resume = textElement('button', '', 'pill-btn sm');
  resume.type = 'button';
  const feedback = textElement('p', '', 'investigation-meta'); feedback.setAttribute('role','status'); feedback.hidden = true;
  host.append(notes, search, sections, resume, feedback);
  let signature = '', itemSignature = '', renderedItems = '', catalog = null, mapSeq = null, epoch = 0, pending = false;
  const openItems = new Set();
  function reset() {
    epoch++; signature = ''; itemSignature = ''; renderedItems = ''; catalog = null; mapSeq = null; pending = false; openItems.clear();
    resume.hidden = true; feedback.hidden = true;
    host.open = false; host.hidden = true; search.value = ''; search.hidden = true;
    notes.replaceChildren(); sections.replaceChildren();
  }
  function renderItems() {
    renderedItems = itemSignature;
    const items = filterConventions(catalog?.items || [], search.value);
    const openAnchors = new Set([...sections.querySelectorAll('[data-anchor][open]')].map(a =>
      JSON.stringify([a.closest('[data-entry]').dataset.entry, a.dataset.anchor])));
    sections.replaceChildren();
    if (!items.length && search.value) sections.append(textElement('p', 'No matching conventions.'));
    const groups = new Map();
    for (const item of items) {
      const key = Object.hasOwn(categories, item.category) ? item.category : 'other';
      if (!groups.has(key)) groups.set(key, []);
      groups.get(key).push(item);
    }
    for (const key of Object.keys(categories)) {
      if (!groups.has(key)) continue;
      const group = document.createElement('section');
      group.append(textElement('h3', categories[key]));
      for (const item of groups.get(key)) {
        const row = document.createElement('details'); row.className = 'investigation-convention';
        row.dataset.entry = item.id; row.open = openItems.has(item.id);
        row.addEventListener('toggle', () => {
          if (!row.isConnected) return;
          if (row.open) openItems.add(item.id); else openItems.delete(item.id);
        });
        const head = document.createElement('summary');
        const term = String(item.label || '');
        const bareTex = item.category === 'notation' && /^\\[A-Za-z]+/.test(term) && !term.includes('$') &&
          !term.startsWith('\\(') && !term.startsWith('\\[');
        head.append(mathElement('strong', bareTex ? '$' + term + '$' : term), document.createTextNode(' — '), mathElement('span', item.meaning));
        const scope = mathElement('small', 'Scope · ' + item.scope);
        head.append(scope);
        row.append(head, textElement('strong', item.basis === 'inferred' ? 'Inferred from use' : 'Stated in the paper', 'investigation-basis'));
        if (item.rationale) row.append(mathElement('p', item.rationale));
        renderAnchors(row, item.anchors);
        for (const a of row.querySelectorAll('[data-anchor]')) a.open = openAnchors.has(JSON.stringify([item.id,a.dataset.anchor]));
        group.append(row);
      }
      sections.append(group);
    }
  }
  host.addEventListener('toggle', () => {
    if (host.open && renderedItems !== itemSignature) renderItems();
  });
  search.addEventListener('input', renderItems);
  resume.addEventListener('click', async () => {
    if (pending || resume.disabled || mapSeq == null) return;
    const start = epoch;
    if (document.activeElement === resume) title.focus({preventScroll:true});
    pending = true; resume.disabled = true; feedback.hidden = true;
    try { await find(mapSeq); }
    catch (error) {
      if (start === epoch) { feedback.textContent = error.message; feedback.hidden = false; }
    } finally {
      if (start === epoch) { pending = false; resume.disabled = false; }
    }
  });
  function update(inv, running) {
    const next = JSON.stringify([inv.conventions, inv.conventions_requested, inv.conventions_is_current, inv.conventions_error, inv.page_start, inv.source_id, inv.map_seq, running]);
    if (next === signature) return;
    signature = next;
    catalog = inv.conventions || null;
    mapSeq = inv.map_seq;
    resume.hidden = running || mapSeq == null || (catalog?.done && inv.conventions_is_current !== false);
    resume.disabled = pending;
    resume.textContent = catalog || inv.conventions_requested ? 'Continue conventions review' : 'Find paper conventions';
    host.hidden = !(catalog || inv.conventions_requested || mapSeq != null);
    if (host.hidden) return;
    const mapEl = document.getElementById('investigation-map'), panel = document.getElementById('investigation-panel');
    const before = mapEl.getBoundingClientRect().top;
    const preserve = !panel.hidden && panel.scrollTop > 0 && before <= panel.getBoundingClientRect().top;
    title.textContent = 'Conventions · ' + conventionStatus(catalog, inv.conventions_requested, running, inv.page_start, inv.conventions_is_current !== false);
    notes.replaceChildren();
    if (catalog && inv.conventions_is_current === false) notes.append(textElement('p', 'Saved catalog from an earlier reading of this paper.'));
    if (inv.conventions_error && !running && (!catalog?.done || inv.conventions_is_current === false)) {
      notes.append(textElement('p', 'Conventions review stopped. ' + inv.conventions_error));
    }
    const pendingCount = inv.conventions_is_current !== false && Array.isArray(catalog?.pending_items) ? catalog.pending_items.length : 0;
    if (pendingCount) notes.append(textElement('p', pendingCitationsLabel(pendingCount), 'investigation-meta'));
    for (const limitation of catalog?.limitations || []) notes.append(mathElement('p', limitation));
    search.hidden = (catalog?.items?.length || 0) < 7;
    if (search.hidden) search.value = '';
    const active = document.activeElement;
    const focusedEntry = active?.closest('.investigation-convention')?.dataset.entry;
    const focusedAnchor = active?.closest('[data-anchor]')?.dataset.anchor;
    itemSignature = JSON.stringify(catalog?.items || []);
    const changed = host.open && itemSignature !== renderedItems;
    if (changed) renderItems();
    if (changed && focusedEntry && host.open) {
      const row = [...sections.querySelectorAll('.investigation-convention')].find(r => r.dataset.entry === focusedEntry);
      const receipt = row && [...row.querySelectorAll('[data-anchor]')].find(a => a.dataset.anchor === focusedAnchor);
      const control = receipt && [...receipt.querySelectorAll('summary,button,a')].find(el =>
        el.tagName === active.tagName && el.textContent === active.textContent);
      (control || row?.querySelector('summary') || title).focus({preventScroll:true});
    }
    if (preserve) panel.scrollTop += mapEl.getBoundingClientRect().top - before;
  }
  return { reset, update };
}
