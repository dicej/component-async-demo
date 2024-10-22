#![deny(warnings)]

use {
    anyhow::Result,
    clap::{Parser, Subcommand},
    std::{
        future::Future,
        path::PathBuf,
        sync::{Arc, Mutex},
        task::Waker,
        time::Duration,
    },
    tokio::fs,
    wasmtime::{
        component::{self, Component, Linker, ResourceTable},
        Config, Engine, Store, StoreContextMut,
    },
    wasmtime_wasi::{WasiCtx, WasiCtxBuilder, WasiView},
};

#[allow(warnings)]
mod round_trip {
    wasmtime::component::bindgen!({
        trappable_imports: true,
        path: "../wit",
        world: "round-trip",
        async: concurrent
    });
}

#[derive(Parser)]
#[command(version, about)]
struct Cli {
    #[command(subcommand)]
    command: Command,
}

#[derive(Subcommand)]
enum Command {
    /// Host a component targeting the `round-trip` world
    RoundTrip {
        component: PathBuf,
        input: String,
        expected_output: String,
    },

    /// Host a component targeting the `wasi:http/handler@0.3.0-draft` world
    Serve { component: PathBuf },
}

struct Ctx {
    wasi: WasiCtx,
    table: ResourceTable,
    #[allow(unused)]
    drop_count: usize,
    #[allow(unused)]
    wakers: Arc<Mutex<Option<Vec<Waker>>>>,
    #[allow(unused)]
    continue_: bool,
}

impl WasiView for Ctx {
    fn table(&mut self) -> &mut ResourceTable {
        &mut self.table
    }
    fn ctx(&mut self) -> &mut WasiCtx {
        &mut self.wasi
    }
}

impl round_trip::local::local::baz::Host for Ctx {
    type Data = Ctx;

    #[allow(clippy::manual_async_fn)]
    fn foo(
        _: StoreContextMut<'_, Self>,
        s: String,
    ) -> impl Future<
        Output = impl FnOnce(StoreContextMut<'_, Self>) -> wasmtime::Result<String> + 'static,
    > + Send
           + 'static {
        async move {
            tokio::time::sleep(Duration::from_millis(10)).await;
            component::for_any(move |_: StoreContextMut<'_, Self>| {
                Ok(format!("{s} - entered host - exited host"))
            })
        }
    }
}

async fn test_round_trip(component: &[u8], input: &str, expected_output: &str) -> Result<()> {
    let mut config = Config::new();
    config.debug_info(true);
    config.cranelift_debug_verifier(true);
    config.wasm_component_model(true);
    config.async_support(true);

    let engine = Engine::new(&config)?;

    let component = Component::new(&engine, component)?;

    let mut linker = Linker::new(&engine);

    wasmtime_wasi::add_to_linker_async(&mut linker)?;
    round_trip::RoundTrip::add_to_linker(&mut linker, |ctx| ctx)?;

    let mut store = Store::new(
        &engine,
        Ctx {
            wasi: WasiCtxBuilder::new().inherit_stdio().build(),
            table: ResourceTable::default(),
            drop_count: 0,
            continue_: false,
            wakers: Arc::new(Mutex::new(None)),
        },
    );

    let round_trip =
        round_trip::RoundTrip::instantiate_async(&mut store, &component, &linker).await?;

    let value = round_trip
        .local_local_baz()
        .call_foo(&mut store, input.to_owned())
        .await?;

    assert_eq!(expected_output, &value);

    Ok(())
}

#[tokio::main]
async fn main() -> Result<()> {
    let cli = Cli::parse();

    match cli.command {
        Command::RoundTrip {
            component,
            input,
            expected_output,
        } => test_round_trip(&fs::read(&component).await?, &input, &expected_output).await?,

        Command::Serve { .. } => {
            todo!()
        }
    }

    println!("success!");

    Ok(())
}

#[cfg(test)]
mod test {
    use {
        super::{test_round_trip, Ctx},
        anyhow::{anyhow, Result},
        futures::future,
        std::{
            future::Future,
            ops::DerefMut,
            sync::Once,
            sync::{Arc, Mutex},
            task::Poll,
            time::Duration,
        },
        tokio::{fs, process::Command, sync::OnceCell},
        wasi_http_draft::{
            wasi::http::types::{Body, ErrorCode, Method, Request, Response, Scheme},
            Fields, WasiHttpView,
        },
        wasm_compose::composer::ComponentComposer,
        wasmtime::{
            component::{self, Component, Linker, Resource, ResourceTable},
            AsContextMut, Config, Engine, Store, StoreContextMut,
        },
        wasmtime_wasi::{WasiCtxBuilder, WasiView},
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
                            "--workspace",
                            "--exclude",
                            "host",
                            "--exclude",
                            "wasi-http-draft",
                            "--target",
                            "wasm32-wasip1"
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
            .module(&fs::read(format!("../target/wasm32-wasip1/debug/{name}.wasm")).await?)?
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
            &wasm_compose::config::Config {
                dir: dir.path().to_owned(),
                definitions: vec![b_file.to_owned()],
                ..Default::default()
            },
        )
        .compose()
    }

    #[tokio::test]
    async fn guest_async() -> Result<()> {
        test_round_trip(
            &build_rust_component("guest_async").await?,
            "hello, world!",
            "hello, world! - entered guest - entered host - exited host - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_sync() -> Result<()> {
        test_round_trip(
            &build_rust_component("guest_sync").await?,
            "hello, world!",
            "hello, world! - entered guest - entered host - exited host - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_async_async() -> Result<()> {
        let guest_async = &build_rust_component("guest_async").await?;
        test_round_trip(
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
        test_round_trip(
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
        test_round_trip(
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
        test_round_trip(
            &compose(guest_sync, guest_sync).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_wait() -> Result<()> {
        test_round_trip(
            &build_rust_component("guest_wait").await?,
            "hello, world!",
            "hello, world! - entered guest - entered host - exited host - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_wait_wait() -> Result<()> {
        let guest_wait = &build_rust_component("guest_wait").await?;
        test_round_trip(
            &compose(guest_wait, guest_wait).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_sync_wait() -> Result<()> {
        let guest_sync = &build_rust_component("guest_sync").await?;
        let guest_wait = &build_rust_component("guest_wait").await?;
        test_round_trip(
            &compose(guest_sync, guest_wait).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_wait_sync() -> Result<()> {
        let guest_wait = &build_rust_component("guest_wait").await?;
        let guest_sync = &build_rust_component("guest_sync").await?;
        test_round_trip(
            &compose(guest_wait, guest_sync).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_async_wait() -> Result<()> {
        let guest_async = &build_rust_component("guest_async").await?;
        let guest_wait = &build_rust_component("guest_wait").await?;
        test_round_trip(
            &compose(guest_async, guest_wait).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    #[tokio::test]
    async fn guest_wait_async() -> Result<()> {
        let guest_wait = &build_rust_component("guest_wait").await?;
        let guest_async = &build_rust_component("guest_async").await?;
        test_round_trip(
            &compose(guest_wait, guest_async).await?,
            "hello, world!",
            "hello, world! - entered guest - entered guest - entered host \
             - exited host - exited guest - exited guest",
        )
        .await
    }

    mod yield_host {
        wasmtime::component::bindgen!({
            path: "../wit",
            world: "yield-host",
            async: concurrent {
                only_imports: [
                    "local:local/ready#when-ready",
                ]
            },
        });
    }

    impl yield_host::local::local::continue_::Host for Ctx {
        fn set_continue(&mut self, v: bool) {
            self.continue_ = v;
        }

        fn get_continue(&mut self) -> bool {
            self.continue_
        }
    }

    impl yield_host::local::local::ready::Host for Ctx {
        type Data = Ctx;

        fn set_ready(&mut self, ready: bool) {
            let mut wakers = self.wakers.lock().unwrap();
            if ready {
                if let Some(wakers) = wakers.take() {
                    for waker in wakers {
                        waker.wake();
                    }
                }
            } else if wakers.is_none() {
                *wakers = Some(Vec::new());
            }
        }

        fn when_ready(
            store: StoreContextMut<Self::Data>,
        ) -> impl Future<Output = impl FnOnce(StoreContextMut<Self::Data>) + 'static>
               + Send
               + Sync
               + 'static {
            let wakers = store.data().wakers.clone();
            future::poll_fn(move |cx| {
                let mut wakers = wakers.lock().unwrap();
                if let Some(wakers) = wakers.deref_mut() {
                    wakers.push(cx.waker().clone());
                    Poll::Pending
                } else {
                    Poll::Ready(component::for_any(|_| ()))
                }
            })
        }
    }

    async fn test_yield(component: &[u8]) -> Result<()> {
        let mut config = Config::new();
        config.debug_info(true);
        config.cranelift_debug_verifier(true);
        config.wasm_component_model(true);
        config.async_support(true);
        config.epoch_interruption(true);

        let engine = Engine::new(&config)?;

        let component = Component::new(&engine, component)?;

        let mut linker = Linker::new(&engine);

        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        yield_host::YieldHost::add_to_linker(&mut linker, |ctx| ctx)?;

        let mut store = Store::new(
            &engine,
            Ctx {
                wasi: WasiCtxBuilder::new().inherit_stdio().build(),
                table: ResourceTable::default(),
                drop_count: 0,
                continue_: false,
                wakers: Arc::new(Mutex::new(None)),
            },
        );
        store.set_epoch_deadline(1);

        std::thread::spawn(move || {
            std::thread::sleep(Duration::from_secs(10));
            engine.increment_epoch();
        });

        let yield_host =
            yield_host::YieldHost::instantiate_async(&mut store, &component, &linker).await?;

        yield_host.local_local_run().call_run(&mut store).await?;

        Ok(())
    }

    #[tokio::test]
    async fn yield_() -> Result<()> {
        let yield_caller = &build_rust_component("yield_caller").await?;
        let yield_callee = &build_rust_component("yield_callee").await?;
        test_yield(&compose(yield_caller, yield_callee).await?).await
    }

    mod proxy {
        wasmtime::component::bindgen!({
            path: "../wit",
            world: "wasi:http/proxy",
            async: concurrent {
                only_imports: [
                    "wasi:http/types@0.3.0-draft#[static]body.finish",
                    "wasi:http/handler@0.3.0-draft#handle",
                ]
            },
            with: {
                "wasi:http/types": wasi_http_draft::wasi::http::types,
            }
        });
    }

    impl WasiHttpView for Ctx {
        type Data = Ctx;

        fn table(&mut self) -> &mut ResourceTable {
            &mut self.table
        }

        #[allow(clippy::manual_async_fn)]
        fn send_request(
            _store: StoreContextMut<'_, Self::Data>,
            _request: Resource<Request>,
        ) -> impl Future<
            Output = impl FnOnce(
                StoreContextMut<'_, Self::Data>,
            )
                -> wasmtime::Result<Result<Resource<Response>, ErrorCode>>
                         + 'static,
        > + Send
               + 'static {
            async move {
                move |_: StoreContextMut<'_, Self>| {
                    Err(anyhow!("no outbound request handler available"))
                }
            }
        }
    }

    async fn test_http_echo(component: &[u8], use_compression: bool) -> Result<()> {
        use {
            flate2::{
                write::{DeflateDecoder, DeflateEncoder},
                Compression,
            },
            std::io::Write,
        };

        let mut config = Config::new();
        config.cranelift_debug_verifier(true);
        config.wasm_component_model(true);
        config.async_support(true);

        let engine = Engine::new(&config)?;

        let component = Component::new(&engine, component)?;

        let mut linker = Linker::new(&engine);

        wasmtime_wasi::add_to_linker_async(&mut linker)?;
        wasi_http_draft::add_to_linker(&mut linker)?;

        let mut store = Store::new(
            &engine,
            Ctx {
                wasi: WasiCtxBuilder::new().inherit_stdio().build(),
                table: ResourceTable::default(),
                drop_count: 0,
                continue_: false,
                wakers: Arc::new(Mutex::new(None)),
            },
        );

        let proxy = proxy::Proxy::instantiate_async(&mut store, &component, &linker).await?;

        let headers = [("foo".into(), b"bar".into())];

        let body = b"And the mome raths outgrabe";

        let request_body_rx = {
            let (mut request_body_tx, request_body_rx) = component::stream(&mut store)?;

            request_body_tx.send(
                &mut store,
                if use_compression {
                    let mut encoder = DeflateEncoder::new(Vec::new(), Compression::fast());
                    encoder.write_all(body)?;
                    encoder.finish()?
                } else {
                    body.to_vec()
                },
            )?;
            request_body_tx.close(&mut store)?;

            request_body_rx
        };

        let trailers = vec![("fizz".into(), b"buzz".into())];

        let request_trailers_rx = {
            let (request_trailers_tx, request_trailers_rx) = component::future(&mut store)?;

            let trailers = WasiView::table(store.data_mut()).push(Fields(trailers.clone()))?;

            request_trailers_tx.send(&mut store, trailers)?;

            request_trailers_rx
        };

        let request = WasiView::table(store.data_mut()).push(Request {
            method: Method::Post,
            scheme: Some(Scheme::Http),
            path_with_query: Some("/".into()),
            authority: Some("localhost".into()),
            headers: Fields(
                headers
                    .iter()
                    .cloned()
                    .chain(if use_compression {
                        vec![
                            ("content-encoding".into(), b"deflate".into()),
                            ("accept-encoding".into(), b"deflate".into()),
                        ]
                    } else {
                        Vec::new()
                    })
                    .collect(),
            ),
            body: Body {
                stream: Some(request_body_rx),
                trailers: Some(request_trailers_rx),
            },
            options: None,
        })?;

        let response = proxy
            .wasi_http_handler()
            .call_handle(&mut store, request)
            .await?;

        let mut response = WasiView::table(store.data_mut()).delete(response?)?;

        assert!(response.status_code == 200);

        assert!(headers.iter().all(|(k0, v0)| response
            .headers
            .0
            .iter()
            .any(|(k1, v1)| k0 == k1 && v0 == v1)));

        let mut response_body_rx = response.body.stream.take().unwrap();

        let mut response_body = Vec::new();
        loop {
            let rx = response_body_rx.receive(&mut store)?;
            if let Ok(chunk) = store.as_context_mut().wait_until(rx).await? {
                response_body.extend(chunk.unwrap());
            } else {
                break;
            }
        }

        let response_body = if use_compression {
            assert!(response.headers.0.iter().any(|(k, v)| matches!(
                (k.as_str(), v.as_slice()),
                ("content-encoding", b"deflate")
            )));

            let mut decoder = DeflateDecoder::new(Vec::new());
            decoder.write_all(&response_body)?;
            decoder.finish()?
        } else {
            response_body
        };

        assert_eq!(body as &[_], &response_body);

        let response_trailers = response.body.trailers.take().unwrap().receive(&mut store)?;

        let response_trailers = store
            .as_context_mut()
            .wait_until(response_trailers)
            .await??
            .unwrap();

        let response_trailers = WasiView::table(store.data_mut()).delete(response_trailers)?;

        assert!(trailers.iter().all(|(k0, v0)| response_trailers
            .0
            .iter()
            .any(|(k1, v1)| k0 == k1 && v0 == v1)));

        Ok(())
    }

    #[tokio::test]
    async fn http_echo() -> Result<()> {
        test_http_echo(&build_rust_component("http_echo").await?, false).await
    }

    #[tokio::test]
    async fn middleware() -> Result<()> {
        let http_echo = &build_rust_component("http_echo").await?;
        let middleware = &build_rust_component("middleware").await?;
        test_http_echo(&compose(middleware, http_echo).await?, true).await
    }
}
