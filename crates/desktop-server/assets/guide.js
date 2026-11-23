// In-app user guide rendered inside Settings -> Guide.

const GUIDE_SECTIONS = [
  {
    title: '1. Quickstart Workflow',
    items: [
      '<strong>Connect a Provider:</strong> Open <em>Settings -> Providers</em> to paste an API key (Anthropic, OpenAI, Gemini), sign in with ChatGPT, use Claude Code / Antigravity CLI, or start Ollama locally.',
      '<strong>Pick Model &amp; Reasoning Effort:</strong> Select your model in the composer dropdown; the secondary <em>Reasoning Effort</em> window opens automatically (or click <code>Effort: ...</code> anytime) to choose <code>Default</code>, <code>Low</code>, <code>Medium</code>, <code>High</code>, <code>X-High</code>, or <code>Max</code>.',
      '<strong>Attach a Paper Workspace:</strong> Click <code>Workspace</code> to grant read/write access to your LaTeX project folder (nothing outside it can be touched).',
      '<strong>Choose Rigor Mode:</strong> <code>Direct</code> (single-pass Orchestrator), <code>Verified</code> (Orchestrator + SymPy guard + Independent Verifier up to 3 rounds), <code>Deep</code> (adds Research Strategist diagnosis on rejections), or <code>Interactive</code> (Plan-First approval + <em>Pause on Explore</em> branch selection).',
    ],
  },
  {
    title: '2. Built-in Research Agents, Models & Custom Subagents',
    items: [
      '<strong>Orchestrator -&gt; Worker Delegation:</strong> The primary <em>Orchestrator</em> plans derivations and keeps its context clean by delegating focused work to isolated specialists: <em>Compute Worker</em> (<code>delegate_compute</code> for iterative SymPy/Python checks), <em>Literature Worker</em> (<code>delegate_literature</code> for arXiv/literature lookups &amp; bulky <code>.tex</code> reads), <em>Lean Prover Worker</em> (<code>delegate_lean</code> + <code>lean_check</code> formal verification with anti-cheat blocking <code>sorry</code>, <code>admit</code>, and unproven axioms), <em>Parallel Explorer / Ultramath</em> (<code>delegate_explore</code> running 2–4 candidate approaches concurrently), and <em>Custom Subagent</em> delegation (<code>delegate_subagent</code>).',
      '<strong>Verifiers, Exposition &amp; Per-Agent Models:</strong> Stateless <em>Step Verifier</em> &amp; <em>Full Verifier</em> audit Proof-DAG nodes without Orchestrator chain-of-thought, alongside <em>Research Strategist</em> (Deep mode recovery), <em>Proof Narrator</em>, <em>Explain</em> (isolated Q&amp;A), and <em>Subagent Builder</em>. Workers default to <code>Default (Auto Smaller Model)</code> for fast, low-cost execution, or you can assign any dedicated model per agent in <em>Settings -&gt; Agents &amp; Models</em>.',
      '<strong>Create Custom Subagents Inside reSearch:</strong> Open <em>Settings -&gt; Agents &amp; Models</em>, describe the specialist you need (e.g. "Lattice QFT loop checker" or "Strict arXiv referee"), click <em>Generate with AI Agent</em>, optionally choose an assigned model, tweak if needed, and click <em>Save Subagent</em>.',
      '<strong>Strict Sandbox Invariant:</strong> Custom subagents and delegated workers steer domain methodology, assigned model, default rigor, and reasoning effort, but can never weaken policy clamps, AppContainer isolation, or approval boundaries (nor recursively delegate or record <code>final_answer</code>).',
    ],
  },
  {
    title: '3. Interactive Steering, Checkpoints & Branch Picker',
    items: [
      '<strong>Mid-Turn Live Steering:</strong> While a turn is running, the composer submit button switches to <code>Steer</code>. Type guidance and submit anytime to queue a live steer (shown as <code>Steer queued -&gt; ...</code>) injected at the Orchestrator\'s next step without aborting the turn.',
      '<strong>Interactive Plan-First Mode &amp; <code>ask_user</code> Checkpoints:</strong> Choosing the <code>Interactive</code> rigor preset instructs the Orchestrator to present a numbered <em>Research Plan Checkpoint</em> before heavy worker delegation. During any turn, <code>ask_user</code> can also surface an inline <em>Orchestrator Question</em> card for convention or mathematical decisions.',
      '<strong>Ultramath Branch Picker (<code>Pause on Explore</code>):</strong> Toggle <code>Pause on Explore</code> in the <em>Conventions</em> popover (enabled automatically by <code>Interactive</code> mode) to pause after parallel <code>delegate_explore</code> runs, compare candidate branches side by side, and adopt, combine, or steer the chosen approach.',
      '<strong>Promote from Explain:</strong> Answers in Canvas -&gt; <code>Explain</code> include <code>Pin to Conventions</code> (adds the insight to pinned conventions) and <code>Send to Orchestrator</code> (queues a live steer if a turn is running, or populates the composer when idle).',
    ],
  },
  {
    title: '4. Computation Sandbox & Tool Approvals',
    items: [
      '<strong>Default-On Desktop Auto-Approval:</strong> In desktop mode, <code>auto_approve_tools</code> in <em>Settings -&gt; Advanced</em> is enabled by default so sandboxed SymPy runs, workspace writes, and literature lookups run uninterrupted while still fail-closed recording <code>approval.answered</code> in <code>ledger.db</code>.',
      '<strong>Manual Approvals &amp; Policy Clamps:</strong> Uncheck <em>Auto-approve computations, workspace writes &amp; literature lookups</em> in <em>Settings -&gt; Advanced</em> anytime to require manual approval cards on every call or once per chat (<code>Approve for Session</code>, revocable via the topbar pill). The CLI route and <code>crates/policy</code> always clamp <code>execute-code</code>, <code>write-workspace</code>, and <code>network-read</code> to <em>Ask</em> at the policy layer.',
      '<strong>AppContainer Sandbox:</strong> Every <code>sympy_eval</code> script executes inside a zero-capability Windows 11 AppContainer (no network, no access to your files or credentials) and is guarded by <em>Calculation Guard</em>.',
    ],
  },
  {
    title: '5. Sources, Proof DAG, Explain & LaTeX Canvas',
    items: [
      '<strong>Direct Source Links &amp; Reader:</strong> Literature hits and ingested arXiv sources include direct clickable links (<code>Open Paper ↗</code>, <code>PDF ↗</code>, <code>DOI ↗</code>) plus a <code>Read Source</code> modal to inspect downloaded TeX/Markdown/BibTeX files in-app.',
      '<strong>Campaign Phases &amp; Shared SHA-256 Ledger:</strong> Research turns progress through structured campaign phases (<code>INGEST -&gt; EXPLORE -&gt; COMPUTE -&gt; SYNTHESIZE -&gt; VERIFY -&gt; REVISE -&gt; COMPLETE</code>). The append-only SHA-256 <strong>Ledger</strong> acts as shared Proof-DAG memory across isolated workers, an anti-overclaiming gate, and a tamper-evident token/routing provenance log.',
      '<strong>Honesty Linter (<code>honesty.lint</code>):</strong> Enforces <em>evidence ≠ proof</em> across final responses and Proof-DAG claims—flagging unverified derivations, missing Lean/SymPy backing, or overclaimed theorems before acceptance.',
      '<strong>Proof DAG, Explain &amp; Counterfactual Forking:</strong> Switch Canvas to <code>Proof DAG</code> to inspect dependency nodes (<code>e-1</code>, <code>e-2</code>...), verify the SHA-256 hash chain, generate a <em>Narrate Proof</em> sheet, or <em>Fork Branch</em> under a different assumption. Use Canvas -&gt; <code>Explain</code> to ask questions without mutating the Proof DAG and export notes as <code>.md</code> or <code>.html</code>.',
      '<strong>Manuscript Split View &amp; Export:</strong> Edit <code>.tex</code>/<code>.bib</code> files with live offline KaTeX preview, run <code>TeX Audit</code>, and export reproducible <code>.tex</code>, <code>.html</code>, or <code>.zip</code> bundles.',
    ],
  },
  {
    title: '6. Managing Chats, Live Model Updates & Shortcuts',
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
