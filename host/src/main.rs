#![deny(warnings)]

use {
    anyhow::{anyhow, Result},
    clap::Parser,
    std::{path::PathBuf, time::Duration},
    tokio::fs,
    wasmtime::{
        component::{Component, Linker, ResourceTable},
        Config, Engine, Store, StoreContextMut,
    },
    wasmtime_wasi::{command, WasiCtx, WasiCtxBuilder, WasiView},
};

// TODO: use `wasmtime-wit-bindgen` once it's updated to (optionally) use `LinkerInstance::func_wrap_concurrent`
// and possibly `TypedFunc::call_concurrent`.

// mod bindings {
//     wasmtime::component::bindgen!({
//         path: "../wit",
//         world: "round-trip",
//         async: true
//     });
// }

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    component: PathBuf,
    input: String,
    expected_output: String,
}

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
}

impl WasiView for Ctx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

// impl bindings::local::local::baz::Host for Ctx {
//     fn foo(
//         &mut self,
//         s: String,
//     ) -> wasmtime::Result<
//         impl Future<Output = impl FnOnce(&mut Self) -> wasmtime::Result<String> + 'static>
//             + Send
//             + 'static,
//     > {
//         Ok(async move {
//             tokio::time::sleep(Duration::from_millis(10)).await;
//             move |_: &mut Self| Ok(format!("{s} - entered host - exited host"))
//         })
//     }
// }

async fn test(component: &[u8], input: &str, expected_output: &str) -> Result<()> {
    let mut config = Config::new();
    config.cranelift_debug_verifier(true);
    config.wasm_component_model(true);
    config.async_support(true);

    let engine = Engine::new(&config)?;

    let component = Component::new(&engine, component)?;

    let mut linker = Linker::new(&engine);

    command::add_to_linker(&mut linker)?;
    // bindings::RoundTrip::add_to_linker(&mut linker, |ctx| ctx)?;

    fn for_any<F, R, T>(fun: F) -> F
    where
        F: FnOnce(StoreContextMut<T>) -> R + 'static,
        R: 'static,
    {
        fun
    }

    linker.instance("local:local/baz")?.func_wrap_concurrent(
        "foo",
        |_, (s,): (String,)| async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            for_any(move |_| Ok((format!("{s} - entered host - exited host"),)))
        },
    )?;

    let mut store = Store::new(
        &engine,
        Ctx {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::default(),
        },
    );

    // let (round_trip, instance) =
    //     bindings::RoundTrip::instantiate_async(&mut store, &component, &linker).await?;

    // let value = round_trip
    //     .local_local_baz()
    //     .call_foo(&mut store, "hello, world!")
    //     .await?;

    let instance = linker.instantiate_async(&mut store, &component).await?;

    let export = instance
        .exports(&mut store)
        .instance("local:local/baz")
        .ok_or_else(|| anyhow!("`local:local/baz` not found"))?
        .typed_func::<_, (String,)>("foo")?;

    let (value,) = export.call_async(&mut store, (input.to_owned(),)).await?;

    assert_eq!(expected_output, &value);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    test(
        &fs::read(&cli.component).await?,
        &cli.input,
        &cli.expected_output,
    )
    .await?;

    println!("success!");

    Ok(())
}

#[cfg(test)]
mod test {
    use {
        super::test,
        anyhow::Result,
        std::sync::Once,
        tokio::{fs, process::Command, sync::OnceCell},
        wasm_compose::{composer::ComponentComposer, config::Config},
        wit_component::ComponentEncoder,
    };

    fn init_logger() {
        static ONCE: Once = Once::new();
        ONCE.call_once(pretty_env_logger::init);
    }

    async fn build_rust_component(name: &str) -> Result<Vec<u8>> {
        init_logger();

        static BUILD: OnceCell<()> = OnceCell::const_new();

        BUILD
            .get_or_init(|| async {
                assert!(
                    Command::new("cargo")
                        .args([
                            "build",
                            "--release",
                            "--workspace",
                            "--exclude",
                            "host",
                            "--target",
                            "wasm32-wasi"
                        ])
                        .status()
                        .await
                        .unwrap()
                        .success(),
                    "cargo build failed"
                );
            })
            .await;

        const ADAPTER_PATH: &str = "../target/wasi_snapshot_preview1.reactor.wasm";

        static ADAPTER: OnceCell<()> = OnceCell::const_new();

        ADAPTER
            .get_or_init(|| async {
                let adapter_url = "https://github.com/bytecodealliance/wasmtime/releases\
                                   /download/v19.0.2/wasi_snapshot_preview1.reactor.wasm";

                if !fs::try_exists(ADAPTER_PATH).await.unwrap() {
                    fs::write(
                        ADAPTER_PATH,
                        reqwest::get(adapter_url)
                            .await
                            .unwrap()
                            .bytes()
                            .await
                            .unwrap(),
                    )
                    .await
                    .unwrap();
                }
            })
            .await;

        ComponentEncoder::default()
            .validate(true)
            .module(&fs::read(format!("../target/wasm32-wasi/release/{name}.wasm")).await?)?
            .adapter("wasi_snapshot_preview1", &fs::read(ADAPTER_PATH).await?)?
            .encode()
    }

    async fn compose(a: &[u8], b: &[u8]) -> Result<Vec<u8>> {
        let dir = tempfile::tempdir()?;

        let a_file = dir.path().join("a.wasm");
        fs::write(&a_file, a).await?;

        let b_file = dir.path().join("b.wasm");
        fs::write(&b_file, b).await?;

        ComponentComposer::new(
            &a_file,
            &Config {
                dir: dir.path().to_owned(),
                definitions: vec![b_file.to_owned()],
                ..Default::default()
            },
        )
        .compose()
    }

    #[tokio::test]
    async fn guest_async() -> Result<()> {
        test(
            &build_rust_component("guest_async").await?,
            "hello, world!",
            "hello, world! - entered guest - entered host - exited host - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_sync() -> Result<()> {
        test(
            &build_rust_component("guest_sync").await?,
            "hello, world!",
            "hello, world! - entered guest - entered host - exited host - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_async_async() -> Result<()> {
        let guest_async = &build_rust_component("guest_async").await?;
        test(
            &compose(guest_async, guest_async).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_sync_async() -> Result<()> {
        let guest_sync = &build_rust_component("guest_sync").await?;
        let guest_async = &build_rust_component("guest_async").await?;
        test(
            &compose(guest_sync, guest_async).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_async_sync() -> Result<()> {
        let guest_async = &build_rust_component("guest_async").await?;
        let guest_sync = &build_rust_component("guest_sync").await?;
        test(
            &compose(guest_async, guest_sync).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_sync_sync() -> Result<()> {
        let guest_sync = &build_rust_component("guest_sync").await?;
        test(
            &compose(guest_sync, guest_sync).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }
}
