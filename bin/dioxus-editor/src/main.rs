/// Where we own all the resources of the browser.
mod compute;
mod surface;

use dioxus::prelude::*;
use tokio::sync;
use web_sys::wasm_bindgen::JsCast as _;

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

#[expect(non_snake_case)]
fn App() -> Element {
    fn surface_from_document() -> surface::Surface {
        tracing::info!("Acquiring WebGPU canvas");

        // FIXME: errors here should fail the boot mechanism, not panic.
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
        let compute = compute::Compute::new(&mut surface);
        spawn(run_surface(surface, compute));
    });

    rsx! {
        link { rel: "stylesheet", href: "main.css" }
        div {
            canvas { id: "main-canvas", width: "800", height: "600" },
        }
    }
}

async fn run_surface(mut surface: surface::Surface, mut compute: compute::Compute) {
    let mut chain = surface.configure_swap_chain(1 << 2);
    tracing::info!("Running surface");
    const BACKGROUND: Asset = asset!("/assets/background.png");

    let img_if = async move {
        let background = asset_to_url(&BACKGROUND)?;
        let response = reqwest::get(background).await?;

        let bytes = response.bytes().await?;
        let img = image::ImageReader::new(std::io::Cursor::new(bytes)).with_guessed_format()?;
        let img = img.decode()?;

        Ok::<_, BoxedError>(img)
    };

    match img_if.await {
        Ok(img) => surface.set_image(&img),
        Err(e) => tracing::error!("No default background asset {e:?}"),
    }

    // Using notify means we get skipping behavior if we miss reaping an interval callback
    // execution. Biggest problem is we can not react to that interval being canceled so we may
    // need to figure out tear down to avoid the risk of refactoring introducing an endless notify
    // await that will never come. (If we need proper tear down at all).
    let notify = std::sync::Arc::new(tokio::sync::Notify::new());

    let sender = notify.clone();
    let _timer = gloo_timers::callback::Interval::new(16, move || {
        sender.notify_one();
    });

    for frame_idx in 0.. {
        /* Running means: we make a certain frame target on the canvas itself. We also run the actual
         * compute program at its own pace.
         */
        tracing::info!("Tick {frame_idx}");
        notify.notified().await;

        let mut texture = match surface.get_current_texture() {
            Err(e) => {
                tracing::error!("{e:?}");
                continue;
            }
            Ok(tx) => tx,
        };

        tracing::info!("Rendering presentation frame");
        surface.present_to_texture(&mut texture);

        tracing::info!("Presenting frame");
        texture.present();
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

    // dioxus assets are stored as absolute paths, but we need a relative one to the assets
    // directory that we load the bundled resource from.
    let base: url::Url = base.join("assets/")?;
    let path = asset.bundled().bundled_path().trim_start_matches('/');
    tracing::trace!("Using asset {} {}", base, path);

    Ok(base.join(path)?)
}
