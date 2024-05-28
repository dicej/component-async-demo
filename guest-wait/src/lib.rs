#![deny(warnings)]

#[allow(warnings)] // TODO: fix `wit-bindgen`-generated warnings so this isn't necessary
mod bindings {
    wit_bindgen::generate!({
        path: "../wit",
        world: "round-trip",
        async: {
            imports: [
                "local:local/baz#foo",
            ]
        }
    });

    use super::Component;
    export!(Component);
}

use bindings::{async_support, exports::local::local::baz::Guest as Baz, local::local::baz};

struct Component;

impl Baz for Component {
    fn foo(s: String) -> String {
        async_support::block_on(async move {
            format!(
                "{} - exited guest",
                baz::foo(&format!("{s} - entered guest")).await
            )
        })
    }
}
