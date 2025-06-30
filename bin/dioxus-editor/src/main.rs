/// Where we own all the resources of the browser.
mod surface;

use dioxus::prelude::*;
use web_sys::wasm_bindgen::JsCast as _;

fn main() {
    init_tracing();
    tracing::info!("starting app");
    dioxus_web::launch::launch_cfg(App, dioxus_web::Config::new().rootname("main"));
}

fn init_tracing() {
    static GLOBAL: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    use tracing_subscriber::prelude::*;
    use tracing_web::MakeWebConsoleWriter;

    GLOBAL.get_or_init(|| {
        let fmt_layer = tracing_subscriber::fmt::layer()
            .with_ansi(false)
            .without_time()
            .with_writer(
                MakeWebConsoleWriter::new()
                    .with_min_level(tracing::Level::ERROR)
                    .with_max_level(tracing::Level::INFO),
            );

        tracing_subscriber::registry().with(fmt_layer).init()
    });
}

#[expect(non_snake_case)]
fn App() -> Element {
    fn surface_from_document() -> surface::Surface {
        tracing::info!("Acquiring WebGPU canvas");

        let element = web_sys::window()
            .or_else(|| {
                tracing::error!("No window");
                None
            })
            .unwrap()
            .document()
            .or_else(|| {
                tracing::error!("No document");
                None
            })
            .unwrap()
            .get_element_by_id("main-canvas")
            .or_else(|| {
                tracing::error!("No such element");
                None
            })
            .unwrap();

        let canvas = element
            .dyn_into()
            .map_err(|e| {
                tracing::error!("Not a canvas {e:?}");
                e
            })
            .unwrap();

        tracing::info!("Surface booting");

        let surface = surface::Surface::new(canvas)
            .map_err(|e| {
                tracing::info!("Can not create surface {e:?}");
                e
            })
            .unwrap();

        tracing::info!("Surface booted");
        surface
    }

    use_effect(|| {
        let mut surface = surface_from_document();
        spawn(run_surface(surface));
    });

    rsx! {
        link { rel: "stylesheet", href: "main.css" }
        div {
            canvas { id: "main-canvas", width: "800", height: "600" },
        }
    }
}

async fn run_surface(mut _surface: surface::Surface) {
    core::future::pending().await
}
