# Brief: latex (wave 1)

Make re:Search render Markdown and LaTeX the way Kandinsky's reading view does.

Design: `03-latex-kandinsky-parity.md`, all of it. Implement **WP1 to WP5** in order (section 3),
one commit or more per package, including every test case in "Test cases" that belongs to them.
WP6 and the optional extras are out of scope.

Kandinsky source: clone it next to the repo and use it as the reference.

```bash
git clone https://github.com/antimajorana/kandinsky.git ~/kandinsky
cd ~/kandinsky && git checkout 833a95f
```

The user allows porting Kandinsky code (README.md, LaTeX decision 1). Read Kandinsky's
`ui/src/lib/math.ts`, `math-align.ts`, `markdown.ts`, `equations.ts`, `preview.ts` and
`ui/src/styles/markdown.css` first. Port what fits re:Search's plain ES modules (no bundler, no
TypeScript, no new npm dependency, KaTeX 0.19.1 is vendored under `crates/desktop-server/vendor/katex`),
with the attribution header named in README.md. Where the design records a deliberate deviation from
Kandinsky, follow the design. Do not read or copy anything under `~/kandinsky/plugins/obsidian/`.

Files you own: everything the design lists for WP1–WP5 under `crates/desktop-server/`
(`assets/math*.js`, new `assets/markdown.js`, `assets/tex_preview.js`, `assets/code_highlight.js`,
`assets/math_scan.js`, `assets/math_katex.js`, `assets/components.css` or a new `assets/math.css`,
`assets/tokens.css`, the 2-line hook in `assets/app.js` `selectSession`, `src/assets.rs`,
`tests/math_render.test.mjs` (only additions), new `tests/math_parity.test.mjs`, `tests/ui_math.rs`).
Another agent is changing `assets/investigation*.js`, `composer.js` and the Rust Investigation code at
the same time: do not touch those.

Done means: `node --test crates/desktop-server/tests/*.test.mjs` passes with the new parity tests;
`node --check` passes on every asset; the existing `math_render.test.mjs` assertions are unchanged
and green; `cargo test -p desktop-server --test ui_math` passes if the crate builds on Linux (say if
it does not); every file at or under 600 lines except `app.js`; the report lists which rows of the
parity table are done, and the visual checks from section 4 that only Windows can do.
