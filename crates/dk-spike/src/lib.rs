//! Backend crate spike — evaluates candidate dependencies for integration.
//!
//! Each probe is a zero-op that forces the crate into the compile graph so we
//! can assert it builds in-workspace without touching production code.
#![allow(dead_code)]

fn _probe_octocrab() {
    let _ = octocrab::instance;
}

fn _probe_enum_dispatch() {
    #[enum_dispatch::enum_dispatch]
    trait Speak {
        fn hello(&self) -> &'static str;
    }
    struct Dog;
    impl Speak for Dog {
        fn hello(&self) -> &'static str {
            "woof"
        }
    }
    struct Cat;
    impl Speak for Cat {
        fn hello(&self) -> &'static str {
            "meow"
        }
    }
    #[enum_dispatch::enum_dispatch(Speak)]
    enum Animal {
        Dog,
        Cat,
    }
    let _ = Animal::Dog(Dog).hello();
}

fn _probe_typetag() {
    #[typetag::serde(tag = "kind")]
    trait Event: std::fmt::Debug {
        fn name(&self) -> &'static str;
    }
    #[derive(Debug, serde::Serialize, serde::Deserialize)]
    struct Ping;
    #[typetag::serde]
    impl Event for Ping {
        fn name(&self) -> &'static str {
            "ping"
        }
    }
}

fn _probe_tracing_core() {
    let _ = tracing_core::Level::INFO;
}

fn _probe_gh_workflow() {
    let _w = gh_workflow::Workflow::new("ci");
}

fn _probe_cargo_issue_lib() {
    // cargo-issue-lib exposes a proc macro to flag open GitHub issues at
    // compile time. We just ensure the crate is linkable here.
}
