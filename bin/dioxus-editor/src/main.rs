#![allow(non_snake_case)]
use dioxus::prelude::*;
use dioxus_logger::tracing;

fn main() {
    // Init logger
    dioxus_logger::init(tracing::Level::INFO).expect("failed to init logger");
    tracing::info!("starting app");
    launch(App);
}

fn App() -> Element {
    let task = spawn(async move {
    });

    rsx! {
        link { rel: "stylesheet", href: "main.css" }
        div {
            canvas { id: "main-canvas", width: "800", height: "600" }
        }
    }
}
