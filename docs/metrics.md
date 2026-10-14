# Metric specification

## Line counts

- `loc`: physical lines, including blank and comment-only lines.
- `sloc`: non-blank source lines after parser-recognized comments and
  punctuation-only delimiter lines are removed.
- `comment_lines`: physical lines touched by a parser-recognized comment.
- `comment_only_lines`: comment lines with no executable source outside the
  comment span.

All line-set composites use unions, so overlapping findings count a line once.

## Verbosity

The blog defines:

```text
blog_verbosity = |rule-flagged lines ∪ clone lines| / LOC
```

The maintained Python CLI uses SLOC as its denominator. This implementation
reports both forms:

```text
verbosity      = |flagged SLOC| / SLOC
blog_verbosity = |flagged physical lines| / LOC
```

Rule findings are deterministic local heuristics. Clone lines come from
normalized Tree-sitter subtrees. Identifiers and literals are normalized while
syntax and operators remain significant.

`cloned_sloc_lines` is the union of every SLOC line participating in a clone.
`excess_duplicate_sloc_lines` retains the first deterministic instance of each
clone group and unions the remaining instances. The latter estimates removable
duplication without double counting overlapping clone groups.

## Erosion

For every function or method `f`:

```text
mass(f) = cyclomatic_complexity(f) * sqrt(sloc(f))
erosion = sum(mass(f) where cyclomatic_complexity(f) > threshold)
          / sum(mass(f))
```

The default threshold is 10, matching the blog and SlopCodeBench. Cognitive
erosion applies the same formula to cognitive complexity.

## Additional signals

The complete report includes function count, mean/max complexity, high and
extreme complexity counts, complexity concentration (Gini coefficient), long
functions, deep nesting, large parameter lists, comment density, TODO/FIXME/
HACK/XXX markers, parser-error counts, clone groups, and duplicate instances.

These signals are evidence for review, not a proof that code is bad. Generated
sources, tests, schema models, and deliberately repetitive code can reasonably
score high.
