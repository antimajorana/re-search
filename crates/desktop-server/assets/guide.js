// In-app user guide rendered inside Settings -> Guide.

const GUIDE_SECTIONS = [
  {
    title: '1. Quickstart Workflow',
    items: [
      '<strong>Connect a Provider:</strong> Open <em>Settings -> Providers</em> to paste an API key (Anthropic, OpenAI, Gemini), sign in with ChatGPT, use Claude Code / Antigravity CLI, or start Ollama locally.',
      '<strong>Pick Model &amp; Reasoning Effort:</strong> Select your model in the composer dropdown; the secondary <em>Reasoning Effort</em> window opens automatically (or click <code>Effort: ...</code> anytime) to choose <code>Default</code>, <code>Low</code>, <code>Medium</code>, <code>High</code>, <code>X-High</code>, or <code>Max</code>.',
      '<strong>Attach a Paper Workspace:</strong> Click <code>Workspace</code> to grant read/write access to your LaTeX project folder (nothing outside it can be touched).',
      '<strong>Choose Rigor Mode:</strong> <code>Direct</code> (single-pass Solver), <code>Verified</code> (Solver + SymPy guard + Independent Verifier up to 3 rounds), or <code>Deep</code> (adds Research Strategist diagnosis on rejections).',
    ],
  },
  {
    title: '2. Built-in Research Agents & Custom Subagents',
    items: [
      '<strong>Built-in Agents:</strong> <em>Solver</em> (derivation + tools), <em>Step Verifier</em> &amp; <em>Full Verifier</em> (stateless mathematical &amp; TeX audit), <em>Research Strategist</em> (recovery plan in Deep mode), <em>Proof Narrator</em> (walkthrough sheet), <em>Side-Talk</em> (isolated Q&amp;A), and <em>Subagent Builder</em>.',
      '<strong>Create Custom Subagents Inside reSearch:</strong> Open <em>Settings -> Subagents</em>, describe the specialist you need (e.g. "Lattice QFT loop checker" or "Strict arXiv referee"), click <em>Generate with AI Agent</em>, tweak if needed, and click <em>Save Subagent</em>.',
      '<strong>Strict Sandbox Invariant:</strong> Custom subagents steer domain methodology, default rigor, and reasoning effort, but can never weaken policy clamps, AppContainer isolation, or approval gates.',
    ],
  },
  {
    title: '3. Why Computation & Tools Ask for Approval',
    items: [
      '<strong>Fail-Closed Safety:</strong> Running Python/SymPy code (<code>sympy_eval</code>), writing files, or fetching papers has real side effects, so policy clamps those actions to <em>Ask</em> on first use.',
      '<strong>AppContainer Sandbox:</strong> Every <code>sympy_eval</code> script executes inside a zero-capability Windows 11 AppContainer (no network, no access to your files or credentials) and is guarded by <em>Calculation Guard</em> + human review.',
      '<strong>One-Click Session Approval:</strong> On the first SymPy, literature, or workspace prompt in a chat, click <code>Approve for Session</code> so all remaining sandboxed calls in that conversation run smoothly without interrupting you. Click the topbar <code>Session allowed: ... ×</code> pill anytime to revoke.',
    ],
  },
  {
    title: '4. Sources, Proof DAG & LaTeX Canvas',
    items: [
      '<strong>Direct Source Links &amp; Reader:</strong> Literature hits and ingested arXiv sources include direct clickable links (<code>Open Paper ↗</code>, <code>PDF ↗</code>, <code>DOI ↗</code>) plus a <code>Read Source</code> modal to inspect downloaded TeX/Markdown/BibTeX files in-app.',
      '<strong>Proof DAG &amp; Counterfactual Forking:</strong> Switch Canvas to <code>Proof DAG</code> to inspect dependency nodes (<code>e-1</code>, <code>e-2</code>...), verify the SHA-256 hash chain, generate a <em>Narrate Proof</em> sheet, or <em>Fork Branch</em> under a different assumption.',
      '<strong>Manuscript Split View &amp; Export:</strong> Edit <code>.tex</code>/<code>.bib</code> files with live offline KaTeX preview, run <code>TeX Audit</code>, and export reproducible <code>.tex</code>, <code>.html</code>, or <code>.zip</code> bundles.',
    ],
  },
  {
    title: '5. Managing Chats, Live Model Updates & Shortcuts',
    items: [
      '<strong>Stop, Archive &amp; Delete:</strong> Press the red <code>Stop</code> button (or <kbd class="guide-kbd">Esc</kbd>) to halt an active turn cleanly. Hover any thread or use topbar buttons to <em>Archive</em> or <em>Delete</em> (local workspace files are never modified; the append-only audit chain stays intact).',
      '<strong>Automatic New Models:</strong> reSearch checks connected providers automatically when stale and highlights newly released models with <code>✦ New</code>.',
      '<strong>Keyboard Shortcuts:</strong> <kbd class="guide-kbd">Ctrl+K</kbd> focus chat filter · <kbd class="guide-kbd">Ctrl+B</kbd> toggle left rail · <kbd class="guide-kbd">Ctrl+\\</kbd> toggle Canvas · <kbd class="guide-kbd">Esc</kbd> stop active turn or close modal.',
    ],
  },
];

export function renderGuide() {
  const host = document.getElementById('settings-guide-content');
  if (!host || host.dataset.rendered === 'true') return;
  host.innerHTML = `<div class="guide-grid">${GUIDE_SECTIONS.map(
    (sec) => `
    <section class="guide-card">
      <h3>${sec.title}</h3>
      <ul>${sec.items.map((item) => `<li>${item}</li>`).join('')}</ul>
    </section>`
  ).join('')}</div>`;
  host.dataset.rendered = 'true';
}
