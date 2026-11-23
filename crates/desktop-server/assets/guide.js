// In-app user guide rendered inside Settings -> Guide.

const GUIDE_SECTIONS = [
  {
    title: '1. Quickstart Workflow',
    items: [
      '<strong>Providers &amp; Effort:</strong> Open <em>Settings -&gt; Providers</em> to add an API key, sign in with ChatGPT, or use Claude Code, Antigravity CLI, or Ollama. Select your model and reasoning effort next to Send.',
      '<strong>Paper Workspace:</strong> Click <code>Workspace</code> to attach your LaTeX folder (nothing outside it can be accessed).',
      '<strong>Rigor Modes:</strong> <code>Direct</code> (single-pass), <code>Verified</code> (Orchestrator + SymPy guard + Independent Verifier), <code>Deep</code> (adds Research Strategist diagnosis &amp; recovery), or <code>Interactive</code> (plan approval &amp; branch selection).',
    ],
  },
  {
    title: '2. Research Agents & Recovery',
    items: [
      '<strong>Specialist Workers:</strong> Orchestrator delegates heavy work to isolated <em>Compute</em> (SymPy), <em>Literature</em> (arXiv/Crossref/INSPIRE/S2), <em>Lean Prover</em> (anti-cheat checked), and <em>Parallel Explorer</em> workers, configurable per role in <em>Agents &amp; Models</em>.',
      '<strong>Custom Subagents:</strong> Describe a specialist in <em>Agents &amp; Models</em> and click <em>Generate with AI Agent</em> to create one.',
      '<strong>Constructive Rejection Recovery:</strong> If verification cannot accept a draft, the Verifier verdict card summarizes what is already solid, shows recovery options, and offers one-click <code>Try Another Approach</code>, <code>Fix Flagged Issues</code>, or <code>Discuss in Explain</code>.',
    ],
  },
  {
    title: '3. Interactive Steering & Explain',
    items: [
      '<strong>Live Steering:</strong> During an in-core turn, Submit becomes <code>Steer</code> so you can redirect the agent at its next step without aborting.',
      '<strong>Plan &amp; Explore Checkpoints:</strong> <code>Interactive</code> mode and <code>Pause on Explore</code> pause for branch comparison or convention decisions.',
      '<strong>Explain Tab:</strong> Ask clarifying questions or click <code>Explain in Detail</code> on any Proof-DAG node without altering the ledger, then promote notes via <code>Pin to Conventions</code> or <code>Send to Orchestrator</code>.',
    ],
  },
  {
    title: '4. Proof DAG, Narrator & Sources',
    items: [
      '<strong>Zoomable Proof DAG:</strong> Use zoom controls, scroll wheel, or drag-to-pan on the graph; click any node to read formatted summaries, KaTeX equations, step explanations, and clickable dependency links.',
      '<strong>Pedagogical Proof Narrator:</strong> Click <code>Narrate Proof</code> for a textbook-style walkthrough explaining intuition and explicitly filling in intermediate algebra and assumptions.',
      '<strong>Sources &amp; Reader:</strong> Ingested papers display their title cleanly with a single Origin link, <code>PDF ↗</code> / <code>DOI ↗</code> links, and an in-app <code>Read Source</code> TeX/Markdown viewer.',
    ],
  },
  {
    title: '5. Shortcuts & Session Management',
    items: [
      '<strong>Shortcuts:</strong> <kbd class="guide-kbd">Ctrl+K</kbd> filter chats · <kbd class="guide-kbd">Ctrl+B</kbd> toggle rail · <kbd class="guide-kbd">Ctrl+\\</kbd> toggle Canvas · <kbd class="guide-kbd">Esc</kbd> stop active turn.',
      '<strong>Safe Management:</strong> <code>Stop</code> halts turns immediately; <em>Archive</em> or <em>Delete</em> never modifies your local workspace files.',
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
