# Report: `eval-q-cases` — Solve fixed evaluation cases (F8, former S0)

- **Brief:** `docs/design/modes-2026-11/agents/eval-q-cases.md`
- **Base commit:** `87da8daa558d8b41867afca63f4e18c488f0450d` (`origin/gemini/work/fixed`)
- **Work branch:** `work/eval-q-cases` -> pushed to `gemini/modes/eval-q-cases`
- **Toolchain (Linux VM):** `rustc 1.100.0 (0ef1bc386 2026-11-09)`, `cargo 1.100.0 (3acc79c0c 2026-11-05)`, `Python 3.12.16` (`sympy 1.14.0`, `mpmath 1.3.0`, `numpy 2.5.5`)

---

## 1. What was implemented (F8 fixed Q slot, former S0)

No production code, tool definitions, `Cargo.lock`, or golden files were changed. Both existing harness self-test cases (`Q-selftest-gaussian`, `Q-selftest-planted`) and `tests/fixtures/eval/scripts/Q-selftest-gaussian.json` remain intact.

| File | Lines | Commit | Role |
|---|---:|---|---|
| `tests/fixtures/eval/manifest-q.json` | 1217 | `9e6d361` | Preserves 2 self-test cases intact and adds 15 original fixed-tier Solve evaluation cases (10 `calculate`, 3 `check_derivation`, 2 `prove`) |
| `tests/fixtures/eval/scripts/Q-calc-01-effective-potential.json` | 70 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-calc-02-quartic-perturbation-0d.json` | 40 | `e58c72c` | Scripted transport fixture (3 turns, `sympy_eval` + `ledger_note`) |
| `tests/fixtures/eval/scripts/Q-calc-03-harmonic-oscillator-thermal.json` | 40 | `e58c72c` | Scripted transport fixture (3 turns, `sympy_eval` + `ledger_note`) |
| `tests/fixtures/eval/scripts/Q-calc-04-bose-einstein-3d-integral.json` | 48 | `e58c72c` | Scripted transport fixture (3 turns, `sympy_eval` + `ledger_note`) |
| `tests/fixtures/eval/scripts/Q-calc-05-feynman-log-integral.json` | 48 | `e58c72c` | Scripted transport fixture (3 turns, `sympy_eval` + `ledger_note`) |
| `tests/fixtures/eval/scripts/Q-calc-06-pauli-su2-exponential.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-calc-07-damped-oscillator-greens-function.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-calc-08-contour-rational-integral.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-calc-09-hydrogen-radial-expectation.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-calc-10-bessel-laplace-integral.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval`) |
| `tests/fixtures/eval/scripts/Q-prove-01-hermitian-eigenvalues.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` only) |
| `tests/fixtures/eval/scripts/Q-prove-02-primes-3mod4.json` | 54 | `e58c72c` | Scripted transport fixture (4 turns, `ledger_note` + `sympy_eval` for `s2` lemma) |
| `crates/agent/tests/eval_harness.rs` | 577 | `5b92e57` | Updated fixed Q case count (2 -> 17) while preserving all self-test checks; added `fixed_q_evaluation_cases_validate_keys_badges_and_scripts` validating S2 badges, monotone closure, planted-error invariants, and end-to-end execution of all 12 new scripts |
| `docs/design/modes-2026-11/reports/eval-q-cases.md` | this file | final | Brief report |

All touched source files pass `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` (`crates/agent/tests/eval_harness.rs` is 577 lines <= 600).

---

## 2. Inventory of the 15 fixed-tier Q cases

| Case ID | Task | Domain / Key Assumptions | Final Answer | Steps | Expected Final Badge | Planted Step / Kind | Transport Script |
|---|---|---|---|---:|---|---|---|
| `Q-calc-01-effective-potential` | `calculate` | Euclidean $\lambda\phi^4$, $\hbar=c=1$, $\lambda>0$, $\phi\ne 0$, $m^2=\lambda\phi^2/2>0$, $\mu>0$, $d=4-2\varepsilon$, $\overline{\text{MS}}$ with $\mu^{2\varepsilon}$ | `m^4/(64*pi^2)*(ln(m^2/mu^2) - 3/2)` | 9 | `Unchecked` (own `Verified`; rests on `s2`/`s7` `Assumed by model`, `s3` `Unchecked`; auxiliary `s4` `Checked numerically`) | `null` | Yes (4 turns) |
| `Q-calc-02-quartic-perturbation-0d` | `calculate` | $x\in\mathbb{R}$, $g>0$ as $g\to 0^+$ (Poincare asymptotic series) | `Z(g) = sqrt(2*pi)*(1 - 3*g + (105/2)*g^2 + O(g^3)), F(g) = 3*g - 48*g^2 + O(g^3)` | 6 | `Model agreed` (own `Verified`; rests on `s4` Poincare remainder bound `Model agreed`) | `null` | Yes (3 turns) |
| `Q-calc-03-harmonic-oscillator-thermal` | `calculate` | $\beta,\hbar,\omega,k_B>0$, $x=\beta\hbar\omega>0$, $E_n=\hbar\omega(n+1/2)$ | `(x/(2*sinh(x/2)))^2 = 1 - x^2/12 + x^4/240 + O(x^6)` | 6 | `Verified` | `null` | Yes (3 turns) |
| `Q-calc-04-bose-einstein-3d-integral` | `calculate` | $x\in(0,\infty)$, given $\zeta(4)=\pi^4/90$ | `pi^4/15` | 7 | `Model agreed` (own `Verified`; rests on `s3` Tonelli interchange `Model agreed`; auxiliary `s6` `Checked numerically`) | `null` | Yes (3 turns) |
| `Q-calc-05-feynman-log-integral` | `calculate` | $x\in(0,\infty)$, $a>0$ (general real $a$ gives $\pi\ln(1+|a|)$) | `pi*ln(1 + a)` | 6 | `Model agreed` (own `Verified`; rests on `s2` Leibniz/DCT `Model agreed`; auxiliary `s5` `Checked numerically`) | `null` | Yes (3 turns) |
| `Q-calc-06-pauli-su2-exponential` | `calculate` | $\theta\in\mathbb{R}$, $\hat{n}\in\mathbb{R}^3$, $\|\hat{n}\|=1$ | `cos(theta)*I - i*sin(theta)*(n_x*sigma_x + n_y*sigma_y + n_z*sigma_z)` | 5 | `Verified` | `null` | Yes (4 turns) |
| `Q-calc-07-damped-oscillator-greens-function` | `calculate` | $t\in\mathbb{R}$, $0<\gamma<\omega_0$, retarded causality & jump conditions | `Theta(t)*exp(-gamma*t)*sin(sqrt(omega_0^2 - gamma^2)*t)/sqrt(omega_0^2 - gamma^2)` | 5 | `Verified` | `null` | Yes (4 turns) |
| `Q-calc-08-contour-rational-integral` | `calculate` | $a>0, b>0$ (both $a\ne b$ and $b\to a^+$) | `pi/(a*b*(a + b))` | 5 | `Verified` | `null` | Yes (4 turns) |
| `Q-calc-09-hydrogen-radial-expectation` | `calculate` | $r\in(0,\infty)$, $a_0>0$, $\hbar>0$, $m>0$ | `<r> = (3/2)*a_0, <r^2> = 3*a_0^2, <1/r> = 1/a_0` | 5 | `Verified` | `null` | Yes (4 turns) |
| `Q-calc-10-bessel-laplace-integral` | `calculate` | $s>0$, $a\in\mathbb{R}$ (covering $|a|<s$, $|a|=s$, $|a|>s$) | `1/sqrt(s^2 + a^2)` | 5 | `Checked numerically` (`s2`/`s3` `Verified` on $|a|<s$; `s4`/`s5` `Checked numerically` on full real domain) | `null` | Yes (4 turns) |
| `Q-check-01-sampled-domain-sqrt` | `check_derivation` | $x\in[0,4]$, principal real $\sqrt{\cdot}\ge 0$ | `12` | 3 | `Failed` | `s2` (`sampled_domain_trap`: true on $[2,4]$, false on $[0,2)$) | None (awaits S9/S11) |
| `Q-check-02-branch-cut-log` | `check_derivation` | $z=r e^{i\theta}$, $r>0$, $\theta\in(\pi/2,\pi)$, principal `Arg` $\in(-\pi,\pi]$ | `Log(z^2) = 2*ln(r) + i*(2*theta - 2*pi), Arg(z^2) = 2*theta - 2*pi` | 3 | `Failed` | `s2` (`branch_cut_simplification`: off by $-2\pi i$ across $(-\infty,0]$) | None (awaits S9/S11) |
| `Q-check-03-gaussian-moment-ibp` | `check_derivation` | $x\in(-\infty,\infty)$, $\alpha>0$ | `3*sqrt(pi)/(4*alpha^(5/2))` | 4 | `Failed` | `s3` (`algebraic_factor_error`: dropped $1/2$ in IBP) | None (awaits S9/S11) |
| `Q-prove-01-hermitian-eigenvalues` | `prove` | Finite-dim complex inner-product space (antilinear 1st, linear 2nd), $H=H^\dagger$ | `For any Hermitian operator H = H^dagger on a finite-dimensional complex inner-product space (V, <.,.>), every eigenvalue lambda is real (lambda = conj(lambda)) and eigenvectors for distinct eigenvalues lambda != mu are orthogonal (<u, v> = 0).` | 3 | `Model agreed` (pure prose; `s1` `Given`, `s2`/`s3` `Model agreed`) | `null` | Yes (4 turns) |
| `Q-prove-02-primes-3mod4` | `prove` | Integers $\mathbb{Z}$ + Fundamental Theorem of Arithmetic | `There are infinitely many prime numbers p congruent to 3 (mod 4).` | 4 | `Model agreed` (`s2` algebraic lemma `Verified`, prose steps `s3`/`s4` `Model agreed` cap final badge) | `null` | Yes (4 turns) |

---

## 3. Expected badge in an answer key vs measured badge in a run

- **Expected badge in `answer_key` (`expected_own_tier`, `expected_tier`, `expected_final_badge`):** A normative specification written and independently reviewed before Solve runs. It records what S2's badge function (`research_tools::solve_status::badge_for_step`) *must* produce for a faithful derivation under the problem's stated assumptions:
  - Exact symbolic or exact series/vanishing checks on handles earn `Verified`.
  - Numeric agreement (12+ points at 50+ digits, or two-run numeric value agreement) earns `Checked numerically` (never `Verified`).
  - Prose proof arguments and real-analysis justifications (Tonelli, DCT/Leibniz, Poincare remainder inequality) cannot earn computational badges; after independent verifier review they reach at most `Model agreed`.
  - Unacknowledged solver assumptions (`Assumed by model`) and unchecked definitions (`Unchecked`) stay rank 0 and pull downstream effective tiers down (`Q-calc-01-effective-potential`).
  - A planted error must end `Failed`, and downstream dependent steps end `Failed` (`Broken: rests on failed <id>`) even when their local algebra (`expected_own_tier`) is `Verified`.
- **Measured badge in a run:** Computed exclusively by host code (`SolveState::apply` + `badge_for_step`) from actual ledger events (`proof.entry`, `solve.expr`, `solve.check`, `proof.verification`, `solve.ack`) produced during execution. Model text never sets a badge.
- **What is measured today:** **Zero Solve badges are measured on these 15 cases.** No live or Solve-runtime scores are claimed. The 12 current-format scripted fixtures run through `run_agent_loop` with `ScriptedModel` and `make_router` (`ledger_note` and mocked `sympy_eval`) solely to verify fixed-tier transport plumbing, tool-name hygiene, exact model-call accounting, and verbatim `final_answer` delivery.

---

## 4. Scripted transport coverage and cases requiring S9/S11 integration

Without inventing unmerged tool definitions (`step_record`, `check_run`, `expr_show`, `solve_finish`, `review_steps`), the existing scripted transport format (`eval_support::scripted`) supports only merged tools (`ledger_note`, `sympy_eval`).

| Cases | Current-format script? | What works today in `eval_harness` | What still needs S7/S8/S9/S11/S14 before scripted end-to-end Solve evaluation |
|---|---|---|---|
| `Q-calc-03`, `Q-calc-06`, `Q-calc-07`, `Q-calc-08`, `Q-calc-09` (5 exact symbolic calculations) | Yes (`scripts/Q-calc-*.json`, merged tools only) | Transport, `run_agent_loop`, model-call count, verbatim `final_answer`, standalone Python execution of all `sympy_eval` snippets | Needs `solve/1` profile & tools (S7/S8: `step_record`, `expr_show`), host check process & registry (S9: `identity_symbolic`, `series_match`, numeric guard), and gates/finish (S11: `solve_finish`) to measure `Verified` badges end-to-end |
| `Q-calc-10` (1 numeric-extended calculation) | Yes (`scripts/Q-calc-10-bessel-laplace-integral.json`) | Same transport & standalone 12-point 50-digit `mpmath` snippet execution | Needs S7/S8 + S9 (`identity_symbolic`, `series_match`, `identity_numeric` 12-pt/50-digit check) + S11 (`solve_finish`) to measure `Verified` on `s2`/`s3` and `Checked numerically` on `s4`/`s5` |
| `Q-calc-01` (effective-potential worked example) | Yes (`scripts/Q-calc-01-effective-potential.json`) | Same transport & standalone Python/mpmath snippet execution | Needs S7/S8 + S9 (`identity_numeric` two-run value check, `series_match`, `vanishes_to_order`) + S10 (`dimensions`, `symmetry` optional sublabels) + S11 (`solve_finish` returning `finished_open_items` with effective badge `Unchecked`) |
| `Q-calc-02`, `Q-calc-04`, `Q-calc-05` (3 calculations with real-analysis prose steps) | Yes (`scripts/Q-calc-0{2,4,5}-*.json`) | Same transport & standalone Python/mpmath snippet execution | Needs S7/S8 + S9 (`identity_symbolic`, `series_match`, `identity_numeric`) + S11 (`solve_finish`) + **S14** (`review_steps` / verifier loop) to measure `Model agreed` on prose analysis steps (`s4` in Q2, `s3` in Q4, `s2` in Q5) and verify lowering of downstream `Verified` own checks |
| `Q-prove-01`, `Q-prove-02` (2 prose-heavy proofs) | Yes (`scripts/Q-prove-0{1,2}-*.json`) | Same transport & standalone Python snippet execution (`Q-prove-02` `s2`) | Needs S7/S8 + S9 (`identity_symbolic` for `Q-prove-02` `s2`) + S11 (`solve_finish`) + **S14** (`review_steps` / verifier loop) to measure `Model agreed` and confirm prose never earns a computational badge |
| `Q-check-01`, `Q-check-02`, `Q-check-03` (3 planted-error derivations) | **No script yet** (deliberately omitted so no fake model prose masquerades as a host `Failed` badge) | Manifest loading, schema validation, single planted-step and S2 closure validation | **Blocked on S7/S8 + S9 + S11:** requires `step_record` + host auto-checks (`identity_symbolic` with numeric guard points across domain/branch cuts in S9) to emit `solve.check` with `outcome: fail` -> `Failed`, plus S11 finish refusal when a `Failed` step is in closure |

In short: **all 15 cases** require S7/S8/S9/S11 (and **5 cases** — `Q-calc-02`, `Q-calc-04`, `Q-calc-05`, `Q-prove-01`, `Q-prove-02` — also require S14 verifier review) before scripted end-to-end Solve evaluation can measure S2 badges and fill an evaluation sheet.

---

## 5. Subagent authoring and independent blind mathematical review record

All subagents ran as Gemini 4 Neon (`Model: "inherit"`).

### Disjoint authoring subagents
1. **Group A Case Author**: Authored `Q-calc-01` .. `Q-calc-05` and their 5 transport scripts into scratch files, verifying every formula and quadrature in SymPy 1.14 / mpmath 1.3 (65 dps).
2. **Group B Case Author**: Authored `Q-calc-06` .. `Q-calc-10` and their 5 transport scripts into scratch files, verifying every matrix/ODE/residue/expectation/Laplace identity and 12-point 50-digit Bessel quadrature in SymPy / mpmath / NumPy.
3. **Group C Case Author**: Authored `Q-check-01` .. `Q-check-03` and `Q-prove-01` .. `Q-prove-02` plus the 2 proof transport scripts into scratch files, verifying all 3 planted errors, counterexample residuals, true closed forms, and proof lemmas in SymPy / mpmath / NumPy.

### Blind independent mathematical reviewers & corrections applied
Before launching reviewers, all `answer_key` and `notes` fields were stripped into inputs-only JSON files (`review_inputs_{a,b,c}.json`). Each reviewer solved/checked all 5 inputs in its group strictly blind from stated assumptions using fresh Python computations, wrote its independent solution file (`review_independent_{a,b,c}.json`), and **only then** opened the authored group and scripts to audit discrepancies.

| Reviewer | Case | Finding from Blind Review | Correction Applied Before Commit |
|---|---|---|---|
| **Reviewer A** | `Q-calc-01-effective-potential` | **Direct internal contradiction:** `input.statement` supplied `with beta(lambda) = 3*lambda^2/(16*pi^2)`, which made `s7` a `Given`, contradicting `s7` being `Assumed by model` (unlike `solve.md` §2 `e-8b`). Also noted implicit $\mu^{2\varepsilon}$ scale placement, partial-derivative / $\gamma_\phi = O(\lambda^2)$ convention, $\mu=1$ implicit in `s4`, `phi` declared `positive=True` in `tu_qcalc01_4`, and `tu_qcalc01_3` duplicating `num_d3`. | Removed `with beta(lambda) = 3*lambda^2/(16*pi^2)` from `input.statement` so `s7` genuinely matches `solve.md` §2 `Assumed by model`; added explicit $\mu^{2\varepsilon}$, partial-derivative, and $\gamma_\phi = O(\lambda^2)$ conventions to `input.statement` and `assumptions`; clarified `(omitting mu^(2*eps), i.e., mu = 1)` in `s4`; updated `tu_qcalc01_4` to `phi = Symbol('phi', real=True, nonzero=True)` and separated `tu_qcalc01_3` cleanly from `num_d3`. |
| **Reviewer A** | `Q-calc-02-quartic-perturbation-0d` | **Completeness gap & S2 badge honesty:** `input.statement` asked for both $Z(g)$ and $F(g)$, but `final_answer` and `end_turn` gave only $F(g)$. Also, `s3` bundled the Poincare remainder inequality $|R_3(u)| \le u^3/6$ under `Verified` (`series_match`), whereas `solve.md` §4.4 states inequalities and prose proofs cannot earn `Verified` (and SymPy 1.14's `series` on $K_{1/4}(1/(32g))$ has a known sign bug). | Expanded `final_answer` and `end_turn` to include both `Z(g)` and `F(g)`; split out the Lagrange/Poincare remainder bound into its own prose step `s4` (`review_prose`, `Model agreed`), lowering `s5`/`s6` effective tier and `expected_final_badge` from `Verified` to `Model agreed` while keeping `s2`/`s3`/`s5`/`s6` own checks `Verified`. |
| **Reviewer A** | `Q-calc-03-harmonic-oscillator-thermal` | Clean mathematically (`Verified` 100% earned via `sp.summation` + differentiation + series). Two minor nits: `s6` used `check_kind: "identity_symbolic"` despite carrying `O(x^6)`, and mixed `beta*hbar*omega` vs `x`. | Changed `s6` `check_kind` to `"series_match"` and expressed `s6` uniformly in `x = beta*hbar*omega`. |
| **Reviewer A** | `Q-calc-04-bose-einstein-3d-integral` | **S2 badge honesty & script gap:** `s2` bundled Tonelli's theorem interchange under `Verified` (confirmed empirically: SymPy 1.14 leaves `integrate(x^3/(exp(x)-1), (x,0,oo))` unevaluated); `tu_qcalc04_1` omitted checking the geometric series identity `s2`. | Split Tonelli's theorem interchange into explicit prose step `s3` (`review_prose`, `Model agreed`), making `expected_final_badge` honestly `Model agreed` (own checks `Verified`, auxiliary 50-digit quadrature `s6` `Checked numerically`); added symbolic geometric-series assertion to `tu_qcalc04_1`. |
| **Reviewer A** | `Q-calc-05-feynman-log-integral` | **Misclassified assumption & S2 badge honesty:** `I(0) = 0` + DCT continuity was listed under `assumptions` despite being a derived property; `s2`/`s4` bundled Leibniz/DCT under `Verified` (confirmed empirically: SymPy 1.14 leaves direct integration at `a = 1` as unsimplified `meijerg`); `tu_qcalc05_1` did not assert `integrand.subs(a, 0) == 0`. | Removed derived `I(0) = 0` lemma from `assumptions`; split Leibniz/DCT & `0+` continuity into explicit prose step `s2` (`review_prose`, `Model agreed`), making `expected_final_badge` honestly `Model agreed` (own checks `Verified`, auxiliary 12-pt 50-digit quadrature `s5` `Checked numerically`); added `assert integrand.subs(a, 0) == 0` and noted $\int_0^\infty dx/(1+a^2 x^2) = \pi/(2|a|)$. |
| **Reviewer B** | `Q-calc-06-pauli-su2-exponential` | **No issues found.** Exact closed form, assumptions, S2 badges, and script verified. | None needed. |
| **Reviewer B** | `Q-calc-07-damped-oscillator-greens-function` | **No errors.** Optional polish: explain `0 < gamma` exponential decay symmetrically alongside `gamma < omega_0`. | Added symmetric `0 < gamma` exponential-decay note in `assumptions[2]`. |
| **Reviewer B** | `Q-calc-08-contour-rational-integral` | **No issues found.** Residues, degenerate `b -> a+` double pole, positivity assumptions, S2 badges, and script verified. | None needed. |
| **Reviewer B** | `Q-calc-09-hydrogen-radial-expectation` | **Missing assumption in `input.statement`:** `hbar > 0, m > 0` appeared in `answer_key` and `s1` (`Given`) but were not explicitly declared in `input.statement`. | Added `and real positive physical constants hbar > 0, m > 0` explicitly to `input.statement`. |
| **Reviewer B** | `Q-calc-10-bessel-laplace-integral` | **Mathematical inaccuracy at boundary `|a| = s`:** The alternating binomial series converges *conditionally* at `|a| = s` by Leibniz's test (not divergent!); what fails at `|a| = s` is *absolute* convergence (and hence Fubini-Tonelli term-by-term integration), while strict divergence occurs for `|a| > s`. Also minor polish on `s5` closure phrasing and adding symbolic `k`-th Laplace term integration to `tu_calc10_2`. | Fixed all 4 locations (`input.statement`, `assumptions[2]`, `notes`, `scripts/Q-calc-10` `end_turn`) to state conditional convergence / Fubini-Tonelli failure at `|a| = s` vs divergence for `|a| > s`; updated `s5` description; added `sp.integrate(t**(2*k)*sp.exp(-s*t), (t, 0, sp.oo))` check to `tu_calc10_2`. |
| **Reviewer C** | `Q-check-01-sampled-domain-sqrt` | **No issues found.** Single planted error (`s2`), `g(x)` wording in `s3`, counterexample `x = 0`, true answer `12`, and badges verified. | None needed. |
| **Reviewer C** | `Q-check-02-branch-cut-log` | **Actionable literal-wording defect in `input.steps[2]`:** Saying `"to read off Arg(z^2) = 2*theta"` asserts a *second* literally false equation (since `Arg` is defined as the principal argument in `(-pi, pi]`), which would cause `s3`'s own check to fail instead of being broken strictly by closure over failed `s2`. | Rewrote `input.steps[2]` to extract the imaginary coefficient of the step-2 expression `L_2 = 2*ln(r) + 2*i*theta`: `(L_2 - 2*ln(r))/i = 2*theta`, keeping `s3` locally `Verified` and `Failed` purely via S2 closure over `s2`. |
| **Reviewer C** | `Q-check-03-gaussian-moment-ibp` | **Actionable literal-wording defect in `input.steps[3]`:** Saying `"to obtain I_4(alpha) = 3*sqrt(pi)/(2*alpha^(5/2))"` asserts a *second* literally false equation against the true integral `I_4(alpha)` defined in `input.statement`. | Rewrote `input.steps[3]` to substitute `I_2(alpha)` into the step-3 expression `J_4(alpha) = (3/alpha)*I_2(alpha)`, keeping `s4` locally `Verified` and `Failed` purely via S2 closure over `s3`. |
| **Reviewer C** | `Q-prove-01-hermitian-eigenvalues` | **No issues found.** Hypotheses, proof steps, `Model agreed` badge cap, and `ledger_note`-only script verified. | None needed. |
| **Reviewer C** | `Q-prove-02-primes-3mod4` | **No blocking issues.** Minor polish: make `k >= 1` explicit in `s3` and mention `"by induction"` in `tu_q_prove_02_3`. Also made `tu_q_prove_02_2` standalone with `import sympy as sp` + `assert`. | Applied all three polishes in `manifest-q.json` and `scripts/Q-prove-02-primes-3mod4.json`. |

---

## 6. Deviations from the design and why

1. **Both harness self-test cases (`Q-selftest-gaussian`, `Q-selftest-planted`) still carry legacy `"expected_tier": "Computed"`.**
   - *Why:* The brief explicitly instructs: *"Keep both harness self-test cases and their existing scripted transport fixture intact."* All 15 new fixed-tier Q cases strictly use the post-2026-11-25 S2 badge vocabulary (`Verified`, `Checked numerically`, `Model agreed`, `Unchecked`, `Failed`, `Given`, `Assumed by model`) with zero uses of `"Computed"` or `"Lean-checked"`.
2. **Real-analysis justifications in `Q-calc-02`, `Q-calc-04`, and `Q-calc-05` are separated into explicit `Model agreed` (`review_prose`) steps rather than bundled into `Verified` steps.**
   - *Why:* Empirical testing against SymPy 1.14 confirmed that SymPy cannot directly verify the Poincare asymptotic remainder bound (`Q-calc-02`), Tonelli's sum-integral interchange (`Q-calc-04`), or Leibniz/DCT differentiation at `a = 1` (`Q-calc-05`). Separating those steps keeps `answer_key` strictly faithful to `solve.md` §4.4 ("A proof in prose can reach at most Model agreed or Checked by you; only claims between handles are computed") and makes Set Q honestly measure SymPy's reach (`00-overview.md` §7.3).

---

## 7. Decisions taken alone and assumptions

1. **Uniform extended `answer_key` schema for the 15 new Q cases:** Retained all 3 manifest-required top-level keys (`final_answer`, `steps`, `planted_step`) and added uniform structured metadata (`domain`, `assumptions`, `expected_final_badge`, per-step `description`, `depends_on`, `check_kind`, `expected_own_tier`, `expected_tier`, plus `planted_error_kind` and `counterexample` on the 3 `check_derivation` cases). This stays 100% compatible with `eval_support::manifest` (`answer_key` inner shape is explicitly left to key writers).
2. **Planted-error selection directly satisfies `00-overview.md` §7.3 line 807:** Chose the 3 `check_derivation` cases to include the two specific failure modes named in `00-overview.md` §7.3 — one identity true only on a sampled subdomain (`Q-check-01-sampled-domain-sqrt`) and one wrong simplification across a branch cut (`Q-check-02-branch-cut-log`) — plus one classic physics algebraic factor error (`Q-check-03-gaussian-moment-ibp`).
3. **No fake scripts for planted-error cases:** Decided not to add `ledger_note`/`sympy_eval`-only scripts for `Q-check-01`..`03` because a model writing prose about a bad step is not a host-computed `Failed` badge.
4. **Focused validation inside `crates/agent/tests/eval_harness.rs`:** Updated `fixed_tier_manifests_load_and_validate` (which previously hard-coded `set.cases_of(Set::Q).len() == 2` and required every case to be a harness self-test) to expect 17 Q entries (2 self-tests + 15 fixed evaluation cases) and added `fixed_q_evaluation_cases_validate_keys_badges_and_scripts` importing S2's `research_tools::solve_status::Tier` directly so `cargo test --locked -p agent --test eval_harness` validates all 15 new keys, S2 closure, and all 12 scripted runs.
5. **Assumption on `check_kind` strings:** Used `given`, `assumed_by_model`, `unchecked_definition`, `identity_symbolic`, `identity_numeric`, `series_match`, `vanishes_to_order`, and `review_prose` as descriptive key labels aligned with `solve.md` §5.1; once S9/S14 land, any live/scripted Solve runner can map or tighten these strings against the implemented check registry.

---

## 8. Merge notes

| File / Interface | Change in this branch | Coordination needed with other lanes |
|---|---|---|
| `tests/fixtures/eval/manifest-q.json` (commit `9e6d361`) | Populated 15 new fixed-tier Q cases alongside the 2 untouched self-tests | Sole owner per brief `eval-q-cases.md`. |
| `tests/fixtures/eval/scripts/Q-{calc,prove}-*.json` (commit `e58c72c`) | 12 new scripted fixtures (`Q-selftest-gaussian.json` untouched) | When S7/S8/S9/S11/S14 land, add or upgrade Solve-profile (`solve/1`) scripted fixtures (including `Q-check-01`..`03`) using `step_record`, `check_run`, `solve_finish`, and `review_steps`. |
| `crates/agent/tests/eval_harness.rs` (commit `5b92e57`, 577 lines) | Small local edit to `fixed_tier_manifests_load_and_validate` (lines 34–46) + added `fixed_q_evaluation_cases_validate_keys_badges_and_scripts` (lines 424–547) | When Set P (Investigate) or Set R (Resume) lanes add fixed-tier cases, they will update lines 34–36 (`sets`, `Set::P` count) cleanly alongside this test. Note `eval_harness.rs` is at 577 lines; future additions >23 lines should split into a sibling integration test file to stay under the 600-line limit. |
| `crates/agent/tests/eval_support/measures.rs` (untouched) | Not edited (outside brief scope) | Still names `planted_error_ends_computed` and `steps_computed_share` from the pre-2026-11-25 revision; needs a follow-up F8 update to split `"Verified"` and `"Checked numerically"` per `00-overview.md` §7.3 (and `EVAL_BLESS=1` on `sheet-template.md`). |

---

## 9. Verification, test commands, and exact counts

| Command | Exit | Exact Result |
|---|---:|---|
| `python3 scratch/assemble_eval_q.py` (all 15 cases S2 closure + all `sympy_eval` snippets in all 12 scripts) | 0 | 15/15 cases valid, 12/12 scripts verbatim `final_answer`, all Python/SymPy/mpmath snippets exited 0 |
| `python3 scripts/check_file_length.py --base origin/gemini/work/fixed` | 0 | `No touched source file is over 600 lines.` (`eval_harness.rs`: 577 lines) |
| `cargo test --locked -p agent --test eval_harness` | 0 | **10 passed, 0 failed, 2 ignored** (2 pre-existing live-tier `#[ignore]` tests: `live_manifests_validate`, `file_sheet_from_json`) |
| `cargo test --locked -p research-tools --test tool_def_golden` | 0 | **3 passed, 0 failed, 0 ignored** (`crates/research-tools/tests/golden/tool_defs.txt` unchanged) |
| `cargo test --locked --workspace --no-fail-fast` | 0 | **779 passed, 0 failed, 2 ignored** across 105 test/doc-test targets |
| `RE_SEARCH_PYTHON="$(command -v python3)" cargo test --locked -p export --no-fail-fast` | 0 | **8 passed, 0 failed, 0 ignored** across 4 targets (Python replay verified) |
| `cargo clippy --locked -p agent --test eval_harness -- -D warnings` | 101 | **0 diagnostics in `eval_harness.rs`**; 2 pre-existing Rust 1.100.0 lints in untouched `eval_support/manifest.rs:289` and `eval_support/sheet.rs:102` |
| `cargo clippy --locked --workspace --all-targets --keep-going -- -D warnings` | 101 | **0 diagnostics in any touched file**; 6 pre-existing diagnostics in untouched files (proved identical to base commit `87da8daa` via empty `git diff`) |

### Pre-existing clippy diagnostics (untouched files, verified identical to base commit `87da8daa558d8b41867afca63f4e18c488f0450d`)
Verified via `git diff --stat origin/gemini/work/fixed -- <files>` returning empty output:
1. `crates/desktop-server/src/evidence.rs:744` — `clippy::manual_unwrap_or` (documented in `RULES.md`; file > 600 lines).
2. `crates/agent/tests/eval_support/manifest.rs:289` — `clippy::match_like_matches_macro` (Rust 1.100.0).
3. `crates/agent/tests/eval_support/sheet.rs:102` — `clippy::unnecessary_map_or` (Rust 1.100.0).
4. `crates/research-tools/tests/sandbox_and_fs.rs:6` — `unused_imports` (`TEX_AUDIT`, `is_symlink_or_junction`, Windows-only call sites on Linux).
5. `crates/agent/src/guardrails.rs:335` — `clippy::items_after_test_module`.
6. `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — `unused_imports` (`BufRead`, `BufReader`, Linux build).

No `crates/desktop-server/assets/` files were touched, so `node --check` / `node --test` were not applicable.

---

## 10. What could not be compiled or run on Linux

- All `#[cfg(windows)]` code paths and Windows-only integration tests (AppContainer sandbox, Job Objects, WebView2 desktop app, `worker/tests/appcontainer_sandbox.rs`, `worker/tests/python_worker.rs`, Windows-gated tests in `sandbox_and_fs.rs` and `chatgpt-auth`).
- Scripted end-to-end Solve badge execution (`step_record`, `check_run`, `solve_finish`, `review_steps`) because S7/S8/S9/S11/S14 are not yet merged.

---

## 11. Bugs / issues noticed but not fixed

1. `crates/agent/tests/eval_support/measures.rs:121-123` — Still defines `planted_error_ends_computed` and `steps_computed_share` using the retired `"Computed"` badge name instead of `"Verified"` and `"Checked numerically"` (2026-11-25 revision in `00-overview.md` §7.3). Left untouched as outside this brief's scope.
2. `crates/desktop-server/src/evidence.rs:744` — Pre-existing `clippy::manual_unwrap_or` (known in `RULES.md`).
3. `crates/agent/tests/eval_support/manifest.rs:289`, `crates/agent/tests/eval_support/sheet.rs:102`, `crates/research-tools/tests/sandbox_and_fs.rs:6`, `crates/agent/src/guardrails.rs:335`, `crates/chatgpt-auth/tests/vault_lock_multiprocess.rs:6` — Pre-existing Rust 1.100.0 / Linux clippy lints in untouched files.

---

## 12. Contradictions or limitations noticed in the design / toolchain

1. **`eval_support/measures.rs` vs `00-overview.md` §7.3 (2026-11-25 revision):** `00-overview.md` §7.3 splits Q's computed share into `Verified` and `Checked numerically` and adds the numeric-guard pass rule (line 807), whereas the earlier F8 skeleton in `measures.rs` and `sheet-template.md` still records the pre-revision `"Computed"` measures. Harmonizing `measures.rs` + `sheet-template.md` should happen before the first Solve evaluation sheet is filed.
2. **SymPy 1.14 symbolic reach measured empirically on Set Q:**
   - `sp.series` on the exact closed form $Z(g) = (8g)^{-1/2} e^{1/(32g)} K_{1/4}(1/(32g))$ in `Q-calc-02` currently exhibits a SymPy sign bug (`1 + 3*g - 63/2*g^2` instead of `1 - 3*g + 105/2*g^2`), confirming the value of the numeric guard on `Verified` equations (`solve.md` §4.4 guard (b)).
   - `sp.integrate(x**3 / (sp.exp(x) - 1), (x, 0, sp.oo))` (`Q-calc-04`) returns unevaluated, and `sp.integrate(sp.log(1 + x**2) / (1 + x**2), (x, 0, sp.oo))` (`Q-calc-05` at `a = 1`) returns an unsimplified `meijerg(...)`, confirming that Tonelli and Leibniz/DCT steps genuinely require prose review (`Model agreed`) unless backed by numeric quadrature (`Checked numerically`).
   - `sp.laplace_transform(sp.besselj(0, a*t), t, s, noconds=True)` (`Q-calc-10`) succeeds via table lookup even though `sp.Integral(...).doit()` leaves `a <= 0` unevaluated; `Q-calc-10`'s prompt therefore explicitly instructs the power-series + 12-point 50-digit quadrature path to exercise `Checked numerically`.
