#![deny(warnings)]

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

use {
    bindings::{exports::local::local::run::Guest, local::local::ready},
    wit_bindgen_rt::async_support,
};

struct Component;

impl Guest for Component {
    fn run() {
        ready::set_ready(false);

        assert!(async_support::poll_future(ready::when_ready()).is_none());

        ready::set_ready(true);

        assert!(async_support::poll_future(ready::when_ready()).is_some());
    }
}
