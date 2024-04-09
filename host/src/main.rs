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

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    let mut config = Config::new();
    config.wasm_component_model(true);
    config.async_support(true);

    let engine = Engine::new(&config)?;

    let component = Component::new(&engine, fs::read(&cli.component).await?)?;

    let mut linker = Linker::new(&engine);

    command::add_to_linker(&mut linker)?;
    // bindings::RoundTrip::add_to_linker(&mut linker, |ctx| ctx)?;

    fn for_any<F, R, T>(fun: F) -> F
    where
        F: FnOnce(StoreContextMut<T>) -> Result<R> + 'static,
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

    let (value,) = export.call_async(&mut store, ("hello, world!",)).await?;

    assert_eq!(
        "hello, world! - entered guest - entered host - exited host - exited guest",
        &value
    );

    println!("success!");

    Ok(())
}
