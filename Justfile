default: check

# Type check the crate with its tests.
check:
    @cargo check -q --all-targets

# Clippy over every target. Any warning fails the recipe.
lint:
    cargo clippy -q --all-targets -- -D warnings

test:
    cargo nextest run

fmt:
    cargo fmt

fmt-check:
    cargo fmt --check

# Open the viewer, or pass flags through, e.g. `just run -p 4=0.5`.
run *args:
    cargo run -q --release -- {{args}}

# Render to PNG without a window, e.g. `just render --view a --ref`.
render *args:
    cargo run -q --release -- render {{args}}
