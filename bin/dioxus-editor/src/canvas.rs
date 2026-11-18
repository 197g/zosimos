use dioxus::prelude::*;
use web_sys::wasm_bindgen::JsCast as _;

use crate::{BoxedError, compute, linker, surface};

pub fn Editor() -> Element {
    let next_frame = use_hook(|| std::sync::Arc::<surface::NextFrameInformation>::default());

    async fn surface_from_document(
        comms: std::sync::Arc<surface::NextFrameInformation>,
    ) -> surface::Surface {
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

        let linker = linker::from_assets().await.unwrap();
        let surface = surface::Surface::new(comms, canvas, linker).unwrap();

        tracing::info!("Surface booted");
        surface
    }

    let render_count = use_signal(|| 1);

    let write_render = render_count.clone();
    let next_frame_surface = next_frame.clone();

    use_effect(move || {
        let next_frame_surface = next_frame_surface.clone();

        spawn(async move {
            let mut write_render = write_render;
            let mut surface = surface_from_document(next_frame_surface).await;
            let compute = compute::Compute::new(&mut surface);

            // Feedback so we can debug what happened in rendering.
            let on_render = Box::new(move || write_render += 1);
            run_surface(surface, compute, on_render).await
        });
    });

    rsx! {
        div {
            canvas {
                id: "main-canvas",
                onresize: move |cx| {
                    tracing::error!("Resized {cx:?}");
                    if let Ok(cbox) = cx.data().get_content_box_size() {
                        let height = cbox.to_u32().height;
                        let width = cbox.to_u32().width;

                        next_frame.recreate.store(true, std::sync::atomic::Ordering::Relaxed);

                        document::eval(&format!(r#"
                            let el = document.getElementById("main-canvas");
                            el.getContext("webgpu")?.unconfigure();
                            el.width = {width};
                            el.height = {height};
                        "#));
                    }
                }
            },
            div {
                p {
                    color: "#fff",
                    "Frame: ",
                    b { "{render_count}" }
                }
            }
        }
    }
}

#[used]
pub static BACKGROUND: Asset = asset!("/assets/background.png");

async fn run_surface(
    mut surface: surface::Surface,
    compute: compute::Compute,
    mut on_render_cb: Box<impl FnMut()>,
) {
    let chain = surface.configure_swap_chain(1 << 2);
    tracing::trace!("Running surface");

    let img_if = async move {
        tracing::info!("Getting background asset");
        let background = crate::asset_to_url(&BACKGROUND)?;
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

        let texture = match surface.get_current_texture() {
            Err(e) => {
                tracing::error!("{e:?}");
                continue;
            }
            Ok(tx) => tx,
        };

        tracing::trace!("Rendering presentation frame");
        surface.present_to_texture(texture).await;

        on_render_cb();
    }
}
