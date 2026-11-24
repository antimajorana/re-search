# <img src="docs/assets/re-search-icon.png" width="56" height="56" align="middle" alt=""> reSearch

reSearch is an AI research workbench for mathematics and theoretical physics. Ask an agent to derive an equation, check a numerical result, or trace a claim to a paper. Each session keeps the argument alongside its assumptions, sources, and recorded calculations so you can inspect how the agent reached its answer.

**[Download reSearch for Windows 11 (x64)](https://github.com/antimajorana/re-search/releases/latest/download/reSearch-Setup-Windows-x64.exe)** · [Release notes and SHA-256 checksum](https://github.com/antimajorana/re-search/releases/latest)

## Get started

1. Run the installer and open **reSearch** from the Start menu. Setup includes Python, SymPy, NumPy, and mpmath. The desktop window uses Microsoft Edge WebView2. The installer is unsigned.
2. Choose **Connect model** to run an AI session. For example, connect Gemini with an API key or select a local model served by Ollama. You can also open **Projects** and organize papers and notes before connecting a model.

## Your first calculation

How quickly does the sum of the first $N$ inverse squares approach $\pi^2/6$? The exact limit gives you an independent way to check the agent’s answer.

1. Choose **Test numerical convergence** on the welcome screen. It puts an editable example in the composer. Ask the agent to use cutoffs of 32, 64, and 128 and report the absolute error at each cutoff. Press **Send**.
2. Open **Canvas → Proof DAG** and select the numerical check. Check the reported errors against 0.03077, 0.01550, and 0.00778. Each doubling roughly halves the error. You can see why by comparing the omitted tail with the integral of $1/x^2$ from $N$ to infinity, which gives the upper bound $1/N$.
3. Open **Canvas → LaTeX → Reproducible ZIP** to share the calculation. The archive includes the report, bibliography, recorded computation, and replay instructions.

The **Verified** review mode asks a verifier to examine the reasoning in a fresh context. Choose **Trace a claim** beneath an answer to inspect the evidence for one statement or request an individual review. Select an assumption in the proof graph to see its downstream conclusions. **Check record integrity** checks the saved events and files. With Lean 4 installed, ask the Lean worker to type-check a precise statement. For a proof using Physlib, attach a Lean workspace with the library already built.

## Bring your own research

| When you want to… | Where to start |
| --- | --- |
| Understand a paper | Choose **New investigation**, add the paper and optionally a reading question. Read the main claims, expand their dependencies, and open the cited passage or PDF page. |
| Derive a field equation | Choose **Check a derivation**. Set the metric signature and units in **Research options → Conventions** so the agent uses your notation. |
| Check a citation against a paper | Use **Attach** to add the paper to your message, then **Send**. Open **Canvas → Sources** to search the ingested text and read the cited lines in context. |
| Carry a result into the next session | Open **Projects** to keep findings, open questions, and a bibliography together. Use **Experiments** to compare approaches or **Validation** to record a reference value and tolerances before running a calculation. |

See the in-app **Settings → User Guide** for more workflows. For command-line use, model providers, approvals, and development commands, see the [reference](docs/reference.md). The [threat model](THREAT-MODEL.md) describes how reSearch handles local files and isolates Python calculations on Windows.

## License

reSearch is licensed under the GNU General Public License, version 3 only (`GPL-3.0-only`). You may use it commercially. If you distribute a modified version, you must license it under GPLv3 and provide its recipients access to the corresponding source. [Full license](LICENSE)

Earlier versions released under MIT or Apache-2.0 retain those grants. Their license texts are preserved in [the MIT notice](licenses/legacy/LICENSE-MIT) and [the Apache-2.0 notice](licenses/legacy/LICENSE-APACHE). Bundled third-party components retain their own licenses. The Windows installer includes those notices and the source link for its build. [Windows packaging](packaging/windows/README.md)
