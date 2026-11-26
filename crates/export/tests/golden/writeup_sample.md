# Synthetic Lattice Sum Identity

**Status:** Done with open items: 1 step checked numerically

## Assumptions and Sources

User-supplied lattice conventions for the finite chain.

**e-1** [Given]: Let N >= 2 be an integer and let x > 0.

> "The discrete boundary sum equals 7/12 for every even chain length."
>
> -- [@voss2024lattice, p. 2, line 4] (OCR)

**e-2** [Cited]: Boundary sum lemma from the cited lattice note.

## Derivation

We expand the polynomial identity and test the tail bound. *(model)*

<!-- handle: poly_id@1 -->
$$
(x + 1)^2 - (x^2 + 2x + 1) = 0
$$

**e-3** [Verified]: Symbolic expansion of the square vanishes identically.

<!-- handle: tail_err@1 -->
$$
\left|\sum_{k=1}^{64} k^{-2} - \frac{\pi^2}{6}\right| < 0.016
$$

**e-4** [Checked numerically]: Partial sum matches the analytic limit at 12 points.

## How each statement was checked

| Step / Claim | How checked | Rests on |
|---|---|---|
| e-1 | Given | - |
| e-2 | Cited | - (OCR source) |
| e-3 | Verified | e-1 |
| e-4 | Checked numerically | e-1, e-3 |

## References

- [@voss2024lattice] Voss, Elena (2024). Synthetic Boundary Sums on Finite Chains. *Journal of Synthetic Tests*. `arxiv:2402.01234`
- [@okafor2023_23907b] Okafor, K. (2023). Discrete Tail Estimates. `doi:10.9999/synth.tail.2023`
