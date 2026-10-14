# Compatibility with the blog, Python CLI, and SlopCodeBench

This implementation follows the metric ideas described in
[Measuring the sloppiness of code](https://earendil.com/posts/measuring-code-sloppiness/),
but it is not score-for-score or command-line compatible with the Python
[`scb-check`](https://github.com/gabeorlanski/scb-check). It also does not
replace the full [SlopCodeBench](https://github.com/SprocketLab/slop-code-bench)
evaluation harness.

The safe comparison boundary is a report produced by the same `metric_version`,
language set, thresholds, exclusions, and generated-source policy.

## Blog formulas

The blog defines:

```text
Verbosity = |AST-Grep flagged lines ∪ clone lines| / LOC

mass(f) = CC(f) * sqrt(SLOC(f))

Erosion = sum(mass(f) where CC(f) > 10) / sum(mass(f))
```

The Rust report maps those ideas as follows.

| Blog concept | Rust field | Compatibility |
| --- | --- | --- |
| Physical lines | `summary.loc` | Same denominator concept: all physical lines, including blanks and comment-only lines |
| Source lines | `summary.sloc` and function `sloc` | Same intent, with grammar-specific line classification |
| Flagged or clone physical lines divided by LOC | `summary.blog_verbosity` | Same union and denominator form; local detectors replace the blog's AST-Grep rule set |
| Function mass | Function `mass`, `summary.total_mass`, `summary.high_cc_mass` | Same `CC * sqrt(SLOC)` formula |
| Strict high-complexity cutoff | `settings.complexity_threshold` | Defaults to 10 and uses `CC > threshold`, matching the blog's strict `> 10` boundary |
| Erosion | `summary.erosion` | Same mass-share formula; concrete function, SLOC, and CC extraction can differ |
| Change in code size | Baseline `delta` | Reports physical LOC and SLOC changes for two supplied path sets |

`blog_verbosity` is formula compatible, not detector compatible. Its numerator
is the union of physical spans from this executable's local verbosity rules and
clone instances. It does not run the blog's AST-Grep YAML rules.

The main `summary.verbosity` field intentionally follows the maintained Python
CLI convention instead:

```text
verbosity = |local rule SLOC ∪ clone SLOC| / SLOC
```

Both numerators use line-set unions. Overlapping rules, nested clone groups, and
rule/clone overlap count each covered line once.

## Python `scb-check`

The Python project was used as a behavioral reference, especially for SLOC
based verbosity, normalized syntax clones, mass-based erosion, and cognitive
erosion. The implementations differ in several scoring-sensitive areas.

| Area | Python CLI | Rust CLI |
| --- | --- | --- |
| Primary verbosity denominator | SLOC | SLOC in `verbosity`; also exposes the blog's LOC form as `blog_verbosity` |
| Rule engine | Bundled and optional AST-Grep YAML plus Python structural rules | Built-in Rust regex and structural checks; no `sg` subprocess |
| Rule language coverage | AST-Grep, structural rules, and directives are Python-only; other supported languages contribute clone lines | Local rules run for every language where a rule is defined; generic Boolean comparison and trivial-wrapper checks span languages |
| Clone implementation | Normalized parser-native tree hashes with language-specific candidates | Normalized Tree-sitter hashes with language-specific candidates and identifier-reuse preservation |
| Complexity implementation | Language adapters produce IR and complexity facts | Direct language-specific Tree-sitter node classification |
| Source directives | Python `scbc ignore[...]` and `scbc boundary` | Not implemented |
| Extra rules | `SCB_CHECK_EXTRA_SLOP_RULES` YAML files | Not implemented |
| Output | Python CLI field schema and human flags | Structured `metric_version/settings/summary/languages/...` schema and specialized human views |
| Default status on findings | 1 | 0; opt into status 1 with `--fail-on-findings` |

Consequently, identical source can receive different clone coverage,
complexities, and verbosity. Neither implementation's score should be treated
as a conversion of the other.

### Language coverage

The maintained Python CLI documents Python, Rust, JavaScript, TypeScript, Zig,
Haskell, and C++. This Rust CLI supports a different set:

| Rust CLI language | Extensions | Relative to the Python CLI |
| --- | --- | --- |
| TypeScript/TSX | `.ts`, `.tsx`, `.mts`, `.cts` | Shared family; Rust CLI also recognizes TSX and module/common variants |
| JavaScript/JSX | `.js`, `.jsx`, `.mjs`, `.cjs` | Shared family; Rust CLI also recognizes JSX |
| Python | `.py`, `.pyw`, `.pyi` | Shared family; Rust CLI also recognizes type stubs |
| Rust | `.rs` | Shared |
| Go | `.go` | Rust CLI addition |
| Java | `.java` | Rust CLI addition |
| Kotlin | `.kt`, `.kts` | Rust CLI addition |

Zig, Haskell, and C++ are outside this implementation's supported set.

Command-line language filters use the same natural names as config and JSON:
`typescript`, `javascript`, `python`, `go`, `rust`, `java`, and `kotlin`. The
older `type-script` and `java-script` spellings remain accepted, as do the short
`ts` and `js` aliases and the `golang` alias.

### Local rule coverage

The Rust CLI uses a deliberately small, inspectable rule set:

- all languages: redundant comparison with a Boolean literal and trivial
  forwarding wrappers;
- Python: `range(len(...))`, redundant `dict.get(..., None)`, explicit length
  checks, and `list(...)` materialized only for iteration;
- Rust: collection `.len() > 0` or `.len() != 0` checks;
- Java and Kotlin: `.size`/`.length` emptiness checks and redundant Boolean
  ternaries;
- JavaScript and TypeScript: `new Array()` and redundant Boolean ternaries.

These are local heuristics, not a port of the Python AST-Grep catalog. Rule
findings include `counts_toward_verbosity` in JSON so consumers can distinguish
scoring findings from diagnostics.

### Clone semantics

The Rust implementation admits language-specific functions and control-flow
blocks with at least three SLOC and two immediate executable body statements by
default. It then:

1. ignores parser-recognized comments and standalone Python docstrings;
2. normalizes identifiers while retaining their within-candidate reuse pattern;
3. buckets literals as string, Boolean, null-like, or numeric;
4. retains syntax node kinds, operators, and identifier relationships;
5. groups equal fingerprints within one language;
6. unions covered lines before calculating ratios.

The thresholds are configurable. Candidate node sets and grammar trees differ
from the Python project, so conceptual similarity does not imply equal clone
groups.

`cloned_sloc_lines` includes all covered instances, matching the coverage form
used in verbosity. `excess_duplicate_sloc_lines` is a Rust-only diagnostic that
retains the first path/line-sorted instance of each group and counts coverage in
the remaining instances.

### Complexity semantics

The Rust CLI starts each function at cyclomatic complexity 1 and increments for
language-specific decisions and Boolean operators. Cognitive complexity adds
nesting costs and jump statements. Nested functions are measured separately;
their SLOC and decisions do not inflate the enclosing function's values.

The mass and erosion arithmetic matches the blog, while the Tree-sitter node
sets are this project's definition of CC and SLOC. This is the largest reason
erosion may differ across implementations even when both use the same formula.
The `metric_version` field exists so future scoring changes are visible.

## Intentional cross-language behavior

Tree-sitter supplies a concrete syntax tree for each language; it does not make
different languages structurally identical. This implementation uses a common
report model with per-language adapters for:

- function-like nodes, including methods, lambdas, closures, and constructors;
- branch, loop, exception, match/switch, and conditional-expression nodes;
- Boolean operator spelling;
- parameter containers and names;
- clone-candidate control-flow nodes;
- comments, identifiers, and literal syntax.

The goal is equivalent meaning, not identical raw counts between languages.
For example, a Rust `match`, Python comprehension, Java `catch`, and Kotlin
`when` have distinct grammar structures and complexity rules. Use language-level
summaries when a repository's language mix changes.

SLOC removes parser-recognized comments and punctuation-only delimiter lines in
every language. It additionally removes standalone non-byte, non-f-string
Python string statements as docstrings. That Python-specific rule intentionally
has no JVM, Rust, Go, or JavaScript analogue.

Clone groups are separated by language even when normalized structures look
similar. Cross-language clones would make previews and refactoring advice
misleading and would couple scores to grammar coincidences.

## Additional diagnostics

The blog's two composite scores are not a complete quality model. The Rust
report also exposes:

- physical LOC and SLOC baseline deltas;
- cognitive complexity and cognitive erosion;
- complexity mean, maximum, population standard deviation, and Gini
  concentration;
- high and extreme complexity counts;
- excess duplicate coverage;
- long functions, deep nesting, and large parameter lists;
- comment density, debt markers, and commented-out-code hints;
- syntax-node and parser-error counts;
- skipped files and analysis time.

Only local verbosity rules and clone coverage contribute to verbosity.
Complexity, comment debt, parse recovery, and structural warnings remain visible
without silently changing that numerator.

## SlopCodeBench integration boundary

SlopCodeBench is an iterative coding-agent evaluation harness. It runs tasks,
tests checkpoints, and aggregates many metrics. Its optional LLM judge is a
separate evaluation feature. The verbosity and erosion metrics discussed here
are static-analysis outputs and require no model.

SlopCodeBench currently consumes the Python `scb-check` report schema. This Rust
CLI uses different commands, JSON field names, language coverage, detectors, and
default finding exit behavior. It is therefore not a drop-in replacement for
the benchmark's pinned checker without an adapter and an explicit decision to
change metric semantics.

For standalone before/after checks, Rust `--baseline` analyzes supplied paths
with the same settings as the current paths. `--strict` treats skipped files and
parser errors in either side as incomplete analysis and exits 2 after emitting
the current report. This operational baseline comparison is outside the blog's
two composite formulas and the SlopCodeBench checkpoint protocol.

## Comparison guidance

For a useful before/after comparison:

1. use the same `scbr` binary and `metric_version`;
2. hold config, thresholds, languages, and exclusions constant;
3. apply the same generated/test/vendor policy;
4. inspect absolute LOC/SLOC and the location-level findings alongside ratios;
5. treat parser errors and skipped files as incomplete evidence;
6. avoid comparing the Rust score directly with published blog or benchmark
   distributions unless the detector and corpus setup are reproduced.

The scores are review signals. A high value can identify removable slop, or it
can reflect intentionally repetitive generated code, protocol models, tests, or
compatibility layers.
