#![deny(warnings)]

#[allow(warnings)] // TODO: fix `wit-bindgen`-generated warnings so this isn't necessary
mod bindings {
    wit_bindgen::generate!({
        path: "../wit",
        world: "yield-callee",
    });

    use super::Component;
    export!(Component);
}

use bindings::{async_support, exports::local::local::run::Guest, local::local::continue_};

struct Component;

impl Guest for Component {
    fn run() {
        while continue_::get_continue() {
            async_support::task_yield();
        }
    }
}
