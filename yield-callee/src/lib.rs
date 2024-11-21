#![deny(warnings)]

mod bindings {
    wit_bindgen::generate!({
        path: "../wit",
        world: "yield-callee",
    });

    use super::Component;
    export!(Component);
}

use {
    bindings::{exports::local::local::run::Guest, local::local::continue_},
    wit_bindgen_rt::async_support,
};

struct Component;

impl Guest for Component {
    fn run() {
        while continue_::get_continue() {
            async_support::task_yield();
        }
    }
}
