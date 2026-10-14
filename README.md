# scbr

`scbr` is a local static-analysis CLI for finding code growth, complexity,
duplication, verbosity, and comment debt. It accepts one file, a repository, or
any mixture of files and directories. Overlapping inputs are canonicalized and
counted once.

The two primary composite metrics come from
[Measuring the sloppiness of code](https://earendil.com/posts/measuring-code-sloppiness/):

```text
blog_verbosity = |locally flagged physical lines ∪ clone lines| / LOC
mass(f)        = cyclomatic_complexity(f) * sqrt(function SLOC(f))
erosion        = sum(mass(f) where cyclomatic_complexity(f) > 10) / sum(mass(f))
```

The report also includes a SLOC based `verbosity` score, cognitive complexity,
clone groups, excess duplicate lines, per-function metrics, comment density,
debt markers, nesting, parameter counts, and parser errors.

Scoring is deterministic and local. The CLI does not call a model, upload
source, contact a service, or invoke an external rule engine. Tree-sitter
grammars are linked into the executable.

## Install

Building from source requires Rust 1.90 or newer.

```bash
# From this repository checkout:
cd scbr
cargo install --path .
scbr --version
```

For development, build without installing:

```bash
cargo build --release
./target/release/scbr check .
```

Run the test suite with `cargo test --all-targets`.

With `just` installed, use the repository recipes to build and run the CLI:

```bash
just                         # Build the release binary
just check                   # Analyze this repository
just complexity src          # Run a specific report
just check --format json --pretty src/main.rs
just languages
just rules
just run --help               # Forward arbitrary CLI arguments
just --list                   # List all recipes
```

CLI recipes rebuild the release binary as needed. Analysis recipes accept the
same paths and options as the CLI and default to `.` when no arguments are given.

## Quick start

```bash
# One file
scbr check src/main.rs

# A repository
scbr check .

# Several roots; overlapping files are counted once
scbr check crates/core crates/cli shared/types.ts

# A complete machine-readable report
scbr check --format json --pretty .

# Compare the current tree with a separate baseline checkout
scbr check --baseline ../project-before .

# Restrict the scan to selected languages
scbr check --language rust,python .

# Return a nonzero status when findings are present
scbr check --fail-on-findings .
```

Options may appear before the input paths. Run `scbr --help` or
`scbr check --help` for the generated command reference.

## Commands

Every analysis command runs the complete analyzer. The command selects the
human-readable view; JSON output always contains the complete report.

| Command | Human-readable view |
| --- | --- |
| `scbr check PATH...` | Composite scores, headline counts, complex functions, duplicate groups, and findings |
| `scbr analyze PATH...` | Alias for `check` |
| `scbr loc PATH...` | Physical LOC, SLOC, blanks, comments, languages, and files |
| `scbr complexity PATH...` | Per-function cyclomatic and cognitive complexity plus erosion |
| `scbr duplicates PATH...` | Normalized syntax clone groups and duplicated SLOC |
| `scbr verbosity PATH...` | Rule lines, clone lines, and both verbosity scores |
| `scbr comments PATH...` | Comment density, debt markers, and commented-out-code hints |
| `scbr languages` | Supported languages and extensions |
| `scbr rules` | Built-in finding classes and composite formulas |

## Supported languages

| Language | Extensions | `--language` value |
| --- | --- | --- |
| TypeScript and TSX | `.ts`, `.tsx`, `.mts`, `.cts` | `typescript` (aliases: `type-script`, `ts`) |
| JavaScript and JSX | `.js`, `.jsx`, `.mjs`, `.cjs` | `javascript` (aliases: `java-script`, `js`) |
| Python | `.py`, `.pyw`, `.pyi` | `python` |
| Go | `.go` | `go` (alias: `golang`) |
| Rust | `.rs` | `rust` |
| Java | `.java` | `java` |
| Kotlin | `.kt`, `.kts` | `kotlin` |

Repeat `--language` or use a comma-separated list:

```bash
scbr check --language typescript --language javascript web/
scbr check --language rust,python .
```

## Scan options

These options are shared by `check`, `loc`, `complexity`, `duplicates`,
`verbosity`, and `comments`.

| Option | Effect |
| --- | --- |
| `--format human\|json` | Select output encoding; defaults to `human` |
| `--pretty` | Indent JSON; requires `--format json` |
| `--language LANGUAGE` | Include only selected languages; repeat or comma-separate values |
| `--exclude GLOB` | Add an exclusion; may be repeated |
| `--no-default-excludes` | Disable the built-in directory exclusions |
| `--include-ignored`, `--include-all` | Traverse files ignored by Git and `.ignore` rules |
| `--hidden` | Traverse hidden files and directories |
| `--config FILE` | Load one explicit TOML or `pyproject.toml` configuration |
| `--no-config` | Disable automatic configuration discovery |
| `--threads N` | Analyze files with exactly `N` worker threads; `N` must be positive |
| `--max-file-bytes BYTES` | Skip larger files and record them in `skipped_files` |
| `--baseline PATH` | Analyze an additional baseline root; may be repeated |
| `--complexity-threshold N` | Set the strict cyclomatic cutoff used by erosion and findings |
| `--extreme-complexity-threshold N` | Set the extreme cyclomatic cutoff; it must be at least the normal cutoff |
| `--cognitive-threshold N` | Set the strict cognitive-complexity cutoff |
| `--min-clone-lines N` | Require this many SLOC in a clone candidate |
| `--min-clone-statements N` | Require this many immediate executable body statements |
| `--long-function-sloc N` | Set the strict long-function cutoff |
| `--max-parameters N` | Set the strict parameter-count cutoff |
| `--max-nesting N` | Set the strict nesting-depth cutoff |
| `--fail-on-findings` | Exit 1 when the report contains slop or high-complexity findings |
| `--strict` | Exit 2 when a current or baseline file is skipped or has parser errors |

### Discovery and exclusions

Directory scans respect `.gitignore`, `.git/info/exclude`, global Git excludes,
and `.ignore` files. Hidden paths are skipped independently. Use both
`--include-ignored` and `--hidden` when both kinds of path should be traversed.
An explicitly named supported file is considered even when it is hidden or
ignored, but configured and command-line exclusion globs still apply.

Built-in exclusions are:

```text
.git  .hg  .svn  .venv  venv  __pycache__  node_modules  .gradle  .idea
```

Exclusion globs are matched against the full path, the path relative to each
directory input, and the base name. A matching directory is pruned. Configured
and command-line exclusions remain active with `--include-ignored` and
`--no-default-excludes`.

Unsupported files found inside a directory are ignored. An explicitly supplied
unsupported file or nonexistent path is an input error. Files reached through
overlapping roots and symlink aliases are deduplicated by canonical path;
nested directory symlinks are not followed during traversal.

## Metrics

### Lines

- `loc` is the number of physical lines, including blank and comment-only lines.
- `sloc` removes blank lines, parser-recognized comments, punctuation-only
  delimiter lines, and standalone non-byte, non-f-string Python string
  statements.
- `comment_lines` counts physical lines touched by a parser-recognized comment.
- `comment_only_lines` counts comment lines with no source outside the comment.

### Verbosity and duplication

The CLI reports two related scores. Line sets are unioned, so a line covered by
several findings or clone groups is counted once.

```text
verbosity      = |rule-flagged SLOC ∪ cloned SLOC| / SLOC
blog_verbosity = |rule-flagged physical lines ∪ clone spans| / LOC
```

Clone candidates are functions and language-specific control-flow blocks. A
candidate must meet both clone thresholds. Tree-sitter subtrees are hashed with
comments ignored, identifiers normalized while preserving identifier reuse
patterns, literals reduced to type buckets, and operators retained. Clone
groups never cross language boundaries.

`cloned_sloc_lines` covers every instance of a clone group.
`excess_duplicate_sloc_lines` retains one canonical instance per group and
counts the remaining covered SLOC, which is useful when estimating removable
duplication.

Local verbosity rules flag redundant Boolean comparisons, selected Python
collection idioms, Rust and JVM length checks, JavaScript/TypeScript empty array
construction, redundant Boolean ternaries, and trivial forwarding wrappers.
`scbr rules` prints the built-in rule IDs.

### Complexity and erosion

Each function, method, closure, or language equivalent receives SLOC,
parameter-count, maximum-nesting, cyclomatic-complexity, and
cognitive-complexity values.

```text
mass(f) = cyclomatic_complexity(f) * sqrt(sloc(f))

erosion = sum(mass(f), CC(f) > complexity_threshold) / sum(mass(f))
```

The comparison is strictly greater than the threshold. Cognitive erosion uses
the same calculation with cognitive complexity. The report also includes mean,
maximum, standard deviation, high and extreme counts, and the Gini coefficient
of function cyclomatic complexity as `cc_concentration`.

Default thresholds are:

| Setting | Default |
| --- | ---: |
| Cyclomatic complexity | 10 |
| Extreme cyclomatic complexity | 30 |
| Cognitive complexity | 10 |
| Minimum clone SLOC | 3 |
| Minimum clone statements | 2 |
| Long function SLOC | 50 |
| Maximum parameters | 5 |
| Maximum nesting | 4 |

### Comments and diagnostics

The comment view reports comment density, `TODO`, `FIXME`, `HACK`, and `XXX`
markers, comments that resemble disabled source, and unusually high comment
density. Parse recovery is reported as a finding because metrics for malformed
files may be partial. Comment, parser, and complexity diagnostics do not add to
the verbosity numerator unless their JSON `counts_toward_verbosity` field is
`true`.

See [Metric specification](docs/metrics.md) for the compact formulas and
[Compatibility](docs/compatibility.md) before comparing scores produced by a
different implementation.

## Configuration

The CLI searches upward from the first input for `scbr.toml`, `.scbr.toml`,
or a `pyproject.toml` containing `[tool.scbr]`. Search stops after the Git root.
Use `--config FILE` to pick one file explicitly or `--no-config` to disable discovery.

Example `scbr.toml`:

```toml
exclude = ["vendor/**", "generated", "**/*.generated.ts"]
include-hidden = false
include-ignored = false
languages = ["typescript", "javascript", "python", "go", "rust", "java", "kotlin"]

[thresholds]
complexity = 10
extreme-complexity = 30
cognitive-complexity = 10
min-clone-lines = 3
min-clone-statements = 2
long-function-sloc = 50
max-parameters = 5
max-nesting = 4
```

The same values can live in `pyproject.toml`:

```toml
[tool.scbr]
exclude = ["vendor/**"]
languages = ["rust", "python"]

[tool.scbr.thresholds]
complexity = 12
min-clone-lines = 5
```

Unknown fields are errors. Command-line thresholds override config thresholds;
command-line exclusions are added to config exclusions; a command-line language
selection replaces the configured language list.

## Baselines

`--baseline` accepts a file or directory and may be repeated. The current and
baseline trees are analyzed independently with the same languages, exclusions,
and thresholds.

```bash
scbr check --baseline ../before .
scbr loc --format json --pretty --baseline ../before-a --baseline ../before-b .
```

Human output reports LOC, SLOC, verbosity, and erosion changes where relevant.
JSON adds `delta` with baseline/current LOC and SLOC, absolute and percentage
line deltas, verbosity and erosion changes, and baseline scanned/skipped/parse
counts. A percentage is `null` when its baseline line count is zero. This
compares paths on disk; it does not read Git history. With `--strict`, a skipped
file or parser error in either the current or baseline inputs produces status 2
after the current report is written.

## Output

Human output is stable plain text without terminal control codes. Informational
messages about the selected configuration and skipped files go to stderr.

Use JSON for automation:

```bash
scbr check --format json . > scbr-report.json
scbr check --format json --pretty .
```

The top-level JSON object contains:

| Field | Contents |
| --- | --- |
| `metric_version` | Scoring-semantics version; currently `scbr-v1` |
| `settings` | Effective thresholds used by the run |
| `summary` | Repository totals and composite scores |
| `languages` | Totals and rule coverage keyed by language |
| `files` | Per-file LOC, SLOC, clone, rule, comment, and parser counts |
| `functions` | Function spans, complexity, SLOC, mass, nesting, and parameters |
| `clone_groups` | Fingerprint, language, total SLOC, and clone instances |
| `findings` | Rule ID, category, severity, span, message, and verbosity effect |
| `skipped_files` | Paths that could not be analyzed and their reasons |
| `delta` | Optional baseline comparison |

`summary` includes line and comment counts; syntax nodes and parse errors;
function and complexity statistics; cyclomatic and cognitive mass/erosion;
clone groups, instances, total and excess duplicate coverage; rule coverage;
both verbosity forms; debt markers; and `analysis_time_ms`. Paths are canonical
absolute paths. Non-UTF-8 operating-system paths use the same lossy display form
as human output. Ordering is deterministic; `analysis_time_ms` naturally varies.

## Exit codes

| Code | Meaning |
| ---: | --- |
| `0` | Analysis completed. Findings are allowed unless `--fail-on-findings` is set. |
| `1` | `--fail-on-findings` was set and the report contains a clone group, a finding, a high-complexity function, or nonzero cognitive erosion. |
| `2` | Invalid arguments, paths, config, thresholds, or discovery; or `--strict` found a skipped current or baseline file or parser error. |

The report is written before status 1 or a strict status 2. When both status
conditions apply, `--strict` takes precedence.

## Scope and interpretation

These metrics identify review candidates. Generated code, tests, schema models,
compatibility layers, and intentionally repetitive implementations can score
high for sound reasons. Compare like with like: keep the tool version,
thresholds, language mix, exclusions, and generated-source policy stable.

This repository is a clean Rust implementation. It does not vendor code from
the [Python `scb-check`](https://github.com/gabeorlanski/scb-check) or
[SlopCodeBench](https://github.com/SprocketLab/slop-code-bench). See
[Compatibility](docs/compatibility.md) for exact similarities and differences.

## Results

```
The Rust version was 15.9× faster on OpenClaw. Methodology was one warm-up and three measured runs each, sequentially.

                                Run 1       Run 2       Run 3      Median
  ━━━━━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━━  ━━━━━━━━━━  ━━━━━━━━━━  ━━━━━━━━━━
   Rust release binary        41.97 s     41.52 s     42.33 s     41.97 s
  ────────────────────────  ──────────  ──────────  ──────────  ──────────
   Python scb-check 0.2.0    670.91 s    665.46 s    667.41 s    667.41 s

  Timing includes startup, analysis, and writing JSON.

  ### Output comparison

   Metric                   Rust     Python
  ━━━━━━━━━━━━━━━━━━━━  ━━━━━━━━━  ━━━━━━━━━
   Files analyzed         36,574     33,816
  ────────────────────  ─────────  ─────────
   Functions             775,477    297,129
  ────────────────────  ─────────  ─────────
   Verbosity               5.71%      5.91%
  ────────────────────  ─────────  ─────────
   Cyclomatic erosion     40.24%     66.30%
  ────────────────────  ─────────  ─────────
   Cognitive erosion      67.60%     81.04%
```

---

e Rust version changes what gets measured—it isn’t a behavior-preserving port of Python. I confirmed several differences using tiny, identical TypeScript inputs.

  ### 1. Python misses some anonymous callbacks

  For code like:

  items.forEach(item => {
    if (item.active) work(item);
  });

  Rust counts 1 function; Python counts 0. Python skips creating a function record when it can’t resolve a name. This helps explain the large function-count gap.

  ### 2. Nested functions change complexity differently

  For an outer function containing an inner function with one if:

   Function    Rust complexity    Python complexity
  ━━━━━━━━━━  ━━━━━━━━━━━━━━━━━  ━━━━━━━━━━━━━━━━━━━
   Outer                     1                    2
  ──────────  ─────────────────  ───────────────────
   Inner                     2                    2

  Python includes the inner function’s branches and lines in the outer function’s metrics. Rust excludes them from the outer function.

  That directly affects erosion, which weights functions by complexity × √source lines.

  ### 3. Duplicate detection uses different thresholds

  Rust requires at least two executable statements in a clone candidate. Python 0.2.0 doesn’t enforce that requirement.

  In a reproduced example containing two similar multiline return calls:

  - Rust: 0 duplicated source lines.
  - Python: 8 duplicated source lines.

  Their additional verbosity rules also differ, so similar final verbosity percentages can come from different findings.

  ### 4. They analyze different files

  - Python rejected 1,804 files that Rust analyzed with parser recovery.
  - Rust analyzed another 1,016 files Python didn’t discover, mostly additional language/extensions.
  - Python analyzed 62 files Rust omitted, including hidden .agents paths.

  These explain why the scores differ, but I haven’t isolated how much each contributes to the full-repo gap. If you want Rust to reproduce Python’s scores, matching these
  semantics is necessary—not just matching the final formulas.

  Reproductions and source pointers (outputs/scb-comparison-20260915/explanation/README.md)
