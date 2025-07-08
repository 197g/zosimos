/// Where we own all the resources of the browser.
mod compute;
mod surface;

use dioxus::prelude::*;
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
            .expect("Loaded in a JS window")
            .document()
            .expect("Loaded in a document page environment")
            .get_element_by_id("main-canvas")
            .or_else(|| {
                tracing::error!("Did not find the element, searched for `#main-canvas`");
                None
            })
            .unwrap();

        let canvas = element.dyn_into().unwrap();

        tracing::info!("Surface booting");

        let surface = surface::Surface::new(canvas).unwrap();

        tracing::info!("Surface booted");
        surface
    }

    let render_count = use_signal(|| 0);

    let write_render = render_count.clone();
    use_effect(move || {
        let mut write_render = write_render;
        let mut surface = surface_from_document();
        let compute = compute::Compute::new(&mut surface);

        // Feedback so we can debug what happened in rendering.
        let on_render = Box::new(move || write_render += 1);
        spawn(run_surface(surface, compute, on_render));
    });

    const STYLE: Asset = asset!("/assets/main.css");

    rsx! {
        document::Stylesheet { href: STYLE }
        div {
            canvas {
                id: "main-canvas",
                onresize: move |cx| {
                    tracing::error!("Resized {cx:?}");
                    if let Ok(cbox) = cx.data().get_content_box_size() {
                        let height = cbox.to_u32().height;
                        let width = cbox.to_u32().width;

                        document::eval(&format!(r#"
                            let el = document.getElementById("main-canvas");
                            el.width = {width};
                            el.height = {height};
                        "#));
                    }
                }
            }
        }
    }
}

async fn run_surface(
    mut surface: surface::Surface,
    mut compute: compute::Compute,
    mut on_render_cb: Box<impl FnMut()>,
) {
    let mut chain = surface.configure_swap_chain(1 << 2);
    tracing::trace!("Running surface");
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
        tracing::trace!("Tick {frame_idx}");
        notify.notified().await;

        let mut texture = match surface.get_current_texture() {
            Err(e) => {
                tracing::error!("{e:?}");
                continue;
            }
            Ok(tx) => tx,
        };

        tracing::trace!("Rendering presentation frame");
        surface.present_to_texture(&mut texture);

        tracing::trace!("Presenting frame");
        texture.present();
        on_render_cb();
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
