# component-async-demo

This is a minimal demonstration of experimental async lift/lower support for the
WebAssembly Component Model, based on temporary forks of `wit-bindgen`,
`wasm-tools`, and `wasmtime`.

## Building and running

### Prerequisites

- A clone of this repo
- Rust
- curl

```
cargo install --locked --git https://github.com/dicej/wasm-tools --branch async wasm-tools
curl -LO https://github.com/bytecodealliance/wasmtime/releases/download/v19.0.1/wasi_snapshot_preview1.reactor.wasm
cargo build --release --target wasm32-wasi --manifest-path guest/Cargo.toml
wasm-tools component new --adapt wasi_snapshot_preview1.reactor.wasm \
    guest/target/wasm32-wasi/release/round_trip.wasm -o round-trip.wasm
cargo run --manifest-path host/Cargo.toml -- round-trip.wasm
```

That last command should print "success!".  If it doesn't, that's bad.
