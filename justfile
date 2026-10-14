set positional-arguments

# Compile the optimized CLI (also the default recipe).
[group('development')]
build:
    cargo build --release

# Run any CLI command, rebuilding as needed.
[group('development')]
run *args='--help':
    cargo run --release -- "$@"

# Run the complete analysis; defaults to this repository.
[group('analysis')]
check *args='.':
    cargo run --release -- check "$@"

alias analyze := check

# Report physical and source line counts.
[group('analysis')]
loc *args='.':
    cargo run --release -- loc "$@"

# Report per-function complexity and erosion.
[group('analysis')]
complexity *args='.':
    cargo run --release -- complexity "$@"

# Report syntax clone groups and duplicated lines.
[group('analysis')]
duplicates *args='.':
    cargo run --release -- duplicates "$@"

# Report style rules, clone coverage, and verbosity.
[group('analysis')]
verbosity *args='.':
    cargo run --release -- verbosity "$@"

# Report comment density and debt markers.
[group('analysis')]
comments *args='.':
    cargo run --release -- comments "$@"

# List supported languages and extensions.
[group('reference')]
languages:
    cargo run --release -- languages

# List built-in findings and metric formulas.
[group('reference')]
rules:
    cargo run --release -- rules
