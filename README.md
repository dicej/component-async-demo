# component-async-demo

This is a minimal demonstration of experimental async lift/lower support for the
WebAssembly Component Model, based on temporary forks of `wit-bindgen`,
`wasm-tools`, and `wasmtime`.

## Building and running

### Prerequisites

- A clone of this repo
- Rust
- curl

Once you have the above, you can install the temporary fork of `wasm-tools` and download the WASI 0.1 adapter:

```
cargo install --locked --git https://github.com/dicej/wasm-tools --branch async wasm-tools
curl -LO https://github.com/bytecodealliance/wasmtime/releases/download/v19.0.1/wasi_snapshot_preview1.reactor.wasm
```

### Simple case

The guest component targets a simple world which imports and exports an
interface with a function that accepts and returns a string, and the
implementation appends to that string before and after passing it on to the
host.  The host implementation appends its own items to the string prior to
returning it, as well as sleeping asynchronously for a short time.  That sleep
causes control to pass back to the guest and on to the original host call.  The
host then idles until the sleep finishes, at which point the import task
finishes, allowing the export task to also finish, yielding a final result back
to the host.

This will build the guest component using the `wit-bindgen` and `wasm-tools`
forks and then run it using the `wasmtime` fork:

```
cargo build --release --target wasm32-wasi --manifest-path guest/Cargo.toml
wasm-tools component new --adapt wasi_snapshot_preview1.reactor.wasm \
    guest/target/wasm32-wasi/release/round_trip.wasm -o round-trip.wasm
cargo run --manifest-path host/Cargo.toml -- round-trip.wasm \
    'hello, world!' \
    'hello, world! - entered guest - entered host - exited host - exited guest'
```

That last command should print "success!".  If it doesn't, that's bad.

### Composition

Now we'll compose the guest component with itself and run the result:

```
wasm-tools compose round-trip.wasm -d round-trip.wasm -o composed.wasm
cargo run --manifest-path host/Cargo.toml -- composed.wasm \
    'hello, world!' \
    'hello, world! - entered guest - entered guest - entered host - exited host - exited guest - exited guest'
```

Okay, so that doesn't work yet -- stay tuned!
