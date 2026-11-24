// In-app user guide rendered inside Settings -> Guide.

const GUIDE_SECTIONS = [
  {
    title: 'Starting a research session',
    items: [
      '<strong>Providers &amp; Effort:</strong> Open <em>Settings -&gt; Providers</em> to add an API key, sign in with ChatGPT, or use Claude Code, Antigravity CLI, or Ollama. Choose a model beside Send. Adjust its reasoning effort under <em>Research options</em>.',
      '<strong>Paper Workspace:</strong> Choose <em>Research options → Workspace</em> to attach your manuscript folder. Use <em>Attach</em> to ingest individual papers.',
      'Open <em>Projects</em> to keep a question, notation, papers, and findings across sessions. Its <em>Validation</em> and <em>Experiments</em> tabs record criteria and compare approaches.',
      '<strong>Review mode under Research options.</strong> <code>Direct</code> (single-pass), <code>Verified</code> (Orchestrator, calculation guard, and Independent Verifier), <code>Deep</code> (adds Research Strategist diagnosis &amp; recovery), or <code>Interactive</code> (plan approval &amp; branch selection).',
    ],
  },
  {
    title: 'Research agents and recovery',
    items: [
      '<strong>Specialist Workers:</strong> Orchestrator delegates heavy work to isolated <em>Compute</em> (SymPy), <em>Literature</em> (arXiv/Crossref/INSPIRE/S2), <em>Lean Prover</em> (anti-cheat checked), and <em>Parallel Explorer</em> workers, configurable per role in <em>Agents &amp; Models</em>.',
      'The Windows installer includes the isolated Python worker and its SymPy and NumPy libraries. Select a computation in the proof graph to inspect its recorded inputs and output.',
      'To create a specialist, describe its task in <em>Agents &amp; Models</em>, choose <em>Draft instructions</em>, then review and save.',
      '<strong>Constructive Rejection Recovery:</strong> If verification cannot accept a draft, the Verifier verdict card summarizes what is already solid, shows recovery options, and offers one-click <code>Try Another Approach</code>, <code>Fix Flagged Issues</code>, or <code>Discuss in Explain</code>.',
    ],
  },
  {
    title: 'Steering and follow-up questions',
    items: [
      '<strong>Live Steering:</strong> During an in-core turn, Submit becomes <code>Steer</code> so you can redirect the agent at its next step without aborting.',
      '<strong>Plan &amp; Explore Checkpoints:</strong> <code>Interactive</code> mode and <code>Pause on Explore</code> pause for branch comparison or convention decisions.',
      '<strong>Explain Tab:</strong> Ask clarifying questions or click <code>Explain in Detail</code> on any Proof-DAG node without altering the ledger, then promote notes via <code>Pin to Conventions</code> or <code>Send to Orchestrator</code>.',
    ],
  },
  {
    title: 'Proof graph and sources',
    items: [
      'Select a step in <em>Proof DAG</em> to see its assumptions, dependencies, recorded checks, and any individual verdict. The full-solution verdict applies to the answer as a whole.',
      'Use <em>Verify Chain</em> to check the integrity of the saved record. Choose <em>Narrate Proof</em> for a walkthrough of the reasoning.',
      'Open <em>Sources Used</em> in the Sources tab to see citations and proof links. Choose a proof step to see where a paper enters the argument, or <em>Read cited lines</em> to inspect a recorded passage. Other search results are in <em>Search History</em>.',
    ],
  },
  {
    title: 'Shortcuts and session management',
    items: [
      '<strong>Shortcuts:</strong> <kbd class="guide-kbd">Ctrl+K</kbd> filter chats · <kbd class="guide-kbd">Ctrl+B</kbd> toggle rail · <kbd class="guide-kbd">Ctrl+\\</kbd> toggle Canvas · <kbd class="guide-kbd">Esc</kbd> stop active turn.',
      '<code>Stop</code> requests cancellation of the active turn. <em>Archive</em> moves a session out of the active list. You can restore it later.',
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
