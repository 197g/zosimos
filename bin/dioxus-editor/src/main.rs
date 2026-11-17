/// Where we own all the resources of the browser.
mod canvas;
mod compute;
mod linker;
mod surface;

use dioxus::prelude::*;

use crate::canvas::Editor;

fn main() {
    init_tracing();
    tracing::info!("starting app");
    dioxus_web::launch::launch_cfg(App, dioxus_web::Config::new().rootname("main"));
}

type BoxedError = Box<dyn std::error::Error + 'static>;

fn init_tracing() {
    static GLOBAL: std::sync::OnceLock<()> = std::sync::OnceLock::new();
    use tracing_subscriber::prelude::*;
    use tracing_web::MakeWebConsoleWriter;

    GLOBAL.get_or_init(|| {
        std::panic::set_hook(Box::new(console_error_panic_hook::hook));

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

#[derive(Routable, Clone, PartialEq)]
enum Route {
    #[route("/")]
    Editor,
}

#[expect(non_snake_case)]
fn App() -> Element {
    rsx! {
        document::Stylesheet { href: asset!("/assets/main.css") }
        Router::<Route> {}
    }
}

fn asset_to_url(asset: &Asset) -> Result<url::Url, BoxedError> {
    static BASE: std::sync::OnceLock<url::Url> = std::sync::OnceLock::new();

    let base = BASE.get_or_init(|| {
        let url_str = web_sys::window()
            .expect("Loaded in a JS window")
            .document()
            .expect("Loaded in a document page environment")
            .url()
            .unwrap();
        url_str
            .parse::<url::Url>()
            .expect("Document has a valid url")
    });

    // In 0.7.0 it did not replace the asset paths as we want.. They show up as `this is a
    // placeholder path which will be replaced by the linker` instead so let us warn as long as
    // this issue is present.
    //
    // That is a problem with a mismatched `dx` but we can degrade gracefully.
    let mut base: url::Url = base.clone();

    let mut path = asset.resolve();

    if asset.bundled().bundled_path().contains("placeholder") {
        tracing::error!(
            "If you see this in development, your `dx` version may not match your dioxus version"
        );

        tracing::warn!(
            "Bad linked asset {} {} (bundled: {}, absolute: {})",
            base,
            path.display(),
            asset.bundled().bundled_path(),
            asset.bundled().absolute_source_path(),
        );

        let bundle = asset.bundled();
        const DIOXUS_MARK: &'static str = "bin/dioxus-editor/";
        let relpath = bundle
            .absolute_source_path()
            .split_once(DIOXUS_MARK)
            .unwrap()
            .1;
        path = relpath.into();
    };

    base.path_segments_mut()
        .unwrap()
        .extend(path.components().filter_map(|seg| match seg {
            std::path::Component::Prefix(_)
            | std::path::Component::RootDir
            | std::path::Component::CurDir
            | std::path::Component::ParentDir => None,
            std::path::Component::Normal(os_str) => os_str.to_str(),
        }));

    Ok(base)
}
