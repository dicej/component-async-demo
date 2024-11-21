#![deny(warnings)]

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

use {
    bindings::{exports::local::local::baz::Guest as Baz, local::local::baz},
    wit_bindgen_rt::async_support,
};

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
