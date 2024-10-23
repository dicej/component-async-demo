#![deny(warnings)]

#[allow(warnings)] // TODO: fix `wit-bindgen`-generated warnings so this isn't necessary
mod bindings {
    wit_bindgen::generate!({
        path: "../wit",
        world: "poll",
        async: {
            imports: [
                "local:local/ready#when-ready",
            ]
        }
    });

    use super::Component;
    export!(Component);
}

use bindings::{async_support, exports::local::local::run::Guest, local::local::ready};

struct Component;

impl Guest for Component {
    fn run() {
        ready::set_ready(false);

        assert!(async_support::poll_future(ready::when_ready()).is_none());

        ready::set_ready(true);

        assert!(async_support::poll_future(ready::when_ready()).is_some());
    }
}
