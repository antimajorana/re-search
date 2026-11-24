import { getCsrfToken } from '/assets/api.js';

let current = null;
const vendor = '/vendor/pdfjs/';

export async function openOriginalPdf({ sessionId, sourceId, path, startLine, label }) {
  current?.close();
  const launchingElement = document.activeElement;
  const dialog = document.createElement('dialog');
  dialog.className = 'original-pdf';
  dialog.setAttribute('aria-label', 'Original PDF');
  dialog.innerHTML = `
    <header><h2></h2><button type="button" class="pill-btn" data-action="close" autofocus>Close</button></header>
    <div class="pdf-toolbar">
      <button type="button" class="pill-btn" data-action="back" disabled>Previous page</button>
      <label>Page <input type="number" min="1" step="1" value="1" disabled></label>
      <span class="pdf-count"></span>
      <button type="button" class="pill-btn" data-action="next" disabled>Next page</button>
      <label>Zoom <select disabled><option value="1">Fit width</option><option value="1.5">150%</option><option value="2">200%</option></select></label>
    </div>
    <p role="status"></p><div class="pdf-sheet" tabindex="0" aria-label="Original page image"></div>`;
  const title = dialog.querySelector('h2');
  title.textContent = label ? `Original PDF · ${label}` : 'Original PDF';
  const status = dialog.querySelector('[role="status"]');
  const sheet = dialog.querySelector('.pdf-sheet');
  const input = dialog.querySelector('input');
  const zoom = dialog.querySelector('select');
  const back = dialog.querySelector('[data-action="back"]');
  const next = dialog.querySelector('[data-action="next"]');
  const cancel = new AbortController();
  let loader, pdf, drawing, requested = 0;
  let active = 1;
  let note = '';
  let lastWidth = 0;

  async function show(number) {
    if (!pdf || !dialog.open) return;
    const generation = ++requested;
    active = Math.max(1, Math.min(pdf.numPages, Math.trunc(number) || 1));
    back.disabled = active === 1;
    next.disabled = active === pdf.numPages;
    input.value = String(active);
    drawing?.cancel();
    sheet.replaceChildren();
    status.textContent = `Rendering page ${active}…`;
    const pageNumber = active;
    try {
      const page = await pdf.getPage(pageNumber);
      if (generation !== requested || !dialog.open) return;
      const base = page.getViewport({ scale: 1 });
      const width = Math.max(100, sheet.clientWidth - 24);
      lastWidth = sheet.clientWidth;
      const fitted = page.getViewport({ scale: (width / base.width) * Number(zoom.value) });
      const density = Math.min(window.devicePixelRatio || 1, 2, 4096 / Math.max(fitted.width, fitted.height));
      const canvas = document.createElement('canvas');
      canvas.width = Math.max(1, Math.ceil(fitted.width * density));
      canvas.height = Math.max(1, Math.ceil(fitted.height * density));
      canvas.style.width = `${fitted.width}px`;
      canvas.style.height = `${fitted.height}px`;
      canvas.setAttribute('role', 'img');
      canvas.setAttribute('aria-label', `Original PDF page ${pageNumber} of ${pdf.numPages}`);
      drawing = page.render({ canvas, viewport: fitted, transform: [density, 0, 0, density, 0, 0], annotationMode: 0 });
      await drawing.promise;
      if (generation === requested && dialog.open) {
        sheet.replaceChildren(canvas);
        sheet.scrollTo(0, 0);
        status.textContent = `Page ${pageNumber} of ${pdf.numPages}.${note ? ' ' + note : ''}`;
      }
    } catch (err) {
      if (generation === requested && dialog.open) status.textContent = `Could not render this page. ${err.message}`;
    }
  }

  const resize = new ResizeObserver(() => {
    if (pdf && sheet.clientWidth !== lastWidth) { lastWidth = sheet.clientWidth; void show(active); }
  });
  dialog.addEventListener('close', () => {
    ++requested;
    cancel.abort();
    drawing?.cancel();
    resize.disconnect();
    loader?.destroy().catch(() => {});
    dialog.remove();
    if (current === dialog) {
      current = null;
      if (launchingElement?.isConnected) launchingElement.focus({ preventScroll: true });
    }
  }, { once: true });
  dialog.querySelector('[data-action="close"]').onclick = () => dialog.close();
  back.onclick = () => show(active - 1);
  next.onclick = () => show(active + 1);
  input.onchange = () => show(input.valueAsNumber);
  input.onkeydown = (ev) => { if (ev.key === 'Enter') { ev.preventDefault(); void show(input.valueAsNumber); } };
  zoom.onchange = () => show(active);
  current = dialog;
  document.body.appendChild(dialog);
  dialog.showModal();
  resize.observe(sheet);
  status.textContent = 'Opening the saved PDF…';
  try {
    const query = new URLSearchParams();
    if (path && Number.isInteger(startLine) && startLine > 0) {
      query.set('path', path);
      query.set('line', String(startLine));
    }
    const url = `/api/sessions/${encodeURIComponent(sessionId)}/sources/${encodeURIComponent(sourceId)}/original-pdf?${query}`;
    const response = await fetch(url, { headers: { 'X-ReSearch-Token': getCsrfToken() }, credentials: 'same-origin', signal: cancel.signal });
    if (!response.ok) throw new Error((await response.json()).error || 'PDF unavailable.');
    const anchored = Number(response.headers.get('X-ReSearch-Pdf-Page'));
    const [data, renderer] = await Promise.all([response.arrayBuffer(), import('/vendor/pdfjs/build/pdf.min.mjs')]);
    if (!dialog.open) return;
    renderer.GlobalWorkerOptions.workerSrc = vendor + 'build/pdf.worker.min.mjs';
    loader = renderer.getDocument({
      data, cMapUrl: vendor + 'cmaps/', cMapPacked: true, standardFontDataUrl: vendor + 'standard_fonts/',
      wasmUrl: vendor + 'wasm/', iccUrl: vendor + 'iccs/', enableXfa: false, useSystemFonts: true,
      canvasMaxAreaInBytes: 32 * 1024 * 1024, maxImageSize: 32 * 1024 * 1024,
    });
    pdf = await loader.promise;
    if (!dialog.open) return;
    input.max = String(pdf.numPages);
    input.disabled = zoom.disabled = false;
    dialog.querySelector('.pdf-count').textContent = `of ${pdf.numPages}`;
    const located = anchored >= 1 && anchored <= pdf.numPages;
    if (startLine && !located) note = 'The exact page for this passage is unavailable. Browse the original pages.';
    await show(located ? anchored : 1);
  } catch (err) {
    if (dialog.open) status.textContent = `Could not open the original PDF. ${err.message}`;
  }
}
