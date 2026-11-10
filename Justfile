default: check

# Type check every crate with its tests, and the browser crates for wasm.
check:
    @cargo check -q --workspace --all-targets
    @cargo check -q -p currents-web --target wasm32-unknown-unknown

# Clippy over every target, native and wasm. Any warning fails the recipe.
lint:
    cargo clippy -q --workspace --all-targets -- -D warnings
    cargo clippy -q -p currents-web --target wasm32-unknown-unknown --no-default-features --features webgpu -- -D warnings
    cargo clippy -q -p currents-web --target wasm32-unknown-unknown --no-default-features --features webgl -- -D warnings

test:
    cargo nextest run --workspace

fmt:
    cargo fmt --all

fmt-check:
    cargo fmt --all --check

# Open the viewer, or pass flags through, e.g. `just run -p 4=0.5`.
run *args:
    cargo run -q --release -p currents -- {{args}}

# Render to PNG without a window, e.g. `just render --view a --ref`.
render *args:
    cargo run -q --release -p currents -- render {{args}}

# Time the GPU passes per view and size, e.g. `just bench-gpu --stages` or `--baseline DIR`.
bench-gpu *args:
    cargo run -q --release -p currents -- bench {{args}}

# Benchmark the CPU side of an export with divan; arguments filter by name.
bench-cpu *args:
    cargo bench -q -p currents --bench export -- {{args}}

# Profile a render's CPU time with samply, e.g. `just profile --frames 60`; needs kernel.perf_event_paranoid at 1 or lower.
profile *args:
    cargo build -q --profile profiling -p currents
    samply record target/profiling/currents render {{args}}

wasm_features := "--enable-simd --enable-bulk-memory --enable-nontrapping-float-to-int --enable-sign-ext --enable-mutable-globals --enable-reference-types"

# Build the WebGPU and WebGL2 bundles and assemble dist/; mise supplies wasm-bindgen and wasm-opt.
web:
    mise install --quiet
    just _web-bundle webgpu
    just _web-bundle webgl
    python3 -I web/build.py

# One wasm bundle, built with only `backend`'s wgpu feature and written to target/web/<backend>.
_web-bundle backend:
    cargo build -q --profile web --target wasm32-unknown-unknown -p currents-web --no-default-features --features {{backend}}
    mise exec -- wasm-bindgen --target web --no-typescript --remove-name-section --remove-producers-section --out-dir target/web/{{backend}} --out-name currents target/wasm32-unknown-unknown/web/currents_web.wasm
    mise exec -- wasm-opt -Oz {{wasm_features}} target/web/{{backend}}/currents_bg.wasm -o target/web/{{backend}}/currents_bg.wasm

# Serve dist/ locally with the production headers (default port 8080).
web-serve port="8080":
    python3 -I web/serve.py dist {{port}}
