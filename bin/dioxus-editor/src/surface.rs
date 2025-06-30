use std::sync::Arc;

use zosimos::buffer::{Color, Descriptor, SampleParts, Texel, Transfer};
use zosimos::command;
use zosimos::pool::{GpuKey, Pool, PoolBridge, PoolImageMut, PoolKey, SwapChain};
use zosimos::program::{Capabilities, CompileError, LaunchError, Program};
use zosimos::run::{Executable, StepLimits};

use wgpu::{Adapter, Instance, SurfaceConfiguration};

pub struct Surface {
    /// The adapter for accessing devices.
    adapter: Arc<Adapter>,
    /// Mirrored configuration of the surface.
    config: SurfaceConfiguration,
    /// The driver instance used for drawing.
    instance: Instance,
    /// Graphics context on the presentation surface and its compositor reference.
    window: Window,
    /// Our private resource pool of the surface.
    pool: Pool,
    /// The pool entry.
    entry: PoolEntry,
    /// The runtime state from stealth paint.
    runtimes: Runtimes,
}

struct Window {
    canvas: web_sys::HtmlCanvasElement,
    /// The surface drawing into the window.
    surface: wgpu::Surface<'static>,
}

#[derive(Debug)]
pub enum PresentationError {
    GpuDeviceLost,
}

impl core::fmt::Display for PresentationError {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        match self {
            PresentationError::GpuDeviceLost => write!(f, "Gpu device was lost"),
        }
    }
}

impl core::error::Error for PresentationError {}

#[derive(Debug)]
struct NormalizingError {
    fail: String,
}

/// The pool entry of our surface declarator.
struct PoolEntry {
    gpu: Option<GpuKey>,
    key: Option<PoolKey>,
    presentable: PoolKey,
    descriptor: Descriptor,
}

#[derive(Default)]
struct Runtimes {
    /// An executable color normalizing the chosen output picture into the output texture, then
    /// writing it as an output.
    normalizing: Option<NormalizingExe>,
}

/// Another compiled program, which puts the image onto the screen.
struct NormalizingExe {
    exe: Arc<Executable>,
    in_descriptor: Descriptor,
    out_descriptor: Descriptor,
    in_reg: command::Register,
    out_reg: command::Register,
}

impl Surface {
    pub fn new(
        canvas: web_sys::HtmlCanvasElement,
    ) -> Result<Self, Box<dyn std::error::Error + Send + Sync + 'static>> {
        const ANY: wgpu::Backends = wgpu::Backends::all();

        let instance = wgpu::Instance::new(&wgpu::InstanceDescriptor {
            backends: wgpu::Backends::all(),
            ..Default::default()
        });

        tracing::info!("Found instance from descriptor");
        let surface = instance.create_surface(wgpu::SurfaceTarget::Canvas(canvas.clone()))?;

        tracing::info!("Created surface on canvas");
        let adapter = Program::request_compatible_adapter(
            &instance,
            &wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::default(),
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
            },
        )?;

        tracing::info!("Created adapter with {:?}", adapter.get_info());
        let (color, texel);
        let capabilities = surface.get_capabilities(&adapter);

        let preferred_format = match capabilities.formats.get(0) {
            None => {
                tracing::warn!("No supported surface formats …");
                color = Color::SRGB;
                texel = Texel::new_u8(SampleParts::RgbA);
                wgpu::TextureFormat::Rgba8Unorm
            }
            Some(wgpu::TextureFormat::Rgba8Unorm) => {
                tracing::warn!("Using format {:?}", wgpu::TextureFormat::Rgba8Unorm);
                color = match Color::SRGB {
                    Color::Rgb {
                        luminance,
                        transfer: _,
                        primary,
                        whitepoint,
                    } => Color::Rgb {
                        luminance,
                        primary,
                        whitepoint,
                        transfer: Transfer::Linear,
                    },
                    _ => unreachable!("That's not the right color"),
                };

                texel = Texel::new_u8(SampleParts::RgbA);
                wgpu::TextureFormat::Rgba8Unorm
            }
            Some(wgpu::TextureFormat::Rgba8UnormSrgb) => {
                tracing::warn!("Using format {:?}", wgpu::TextureFormat::Rgba8UnormSrgb);

                color = Color::SRGB;
                texel = Texel::new_u8(SampleParts::RgbA);
                wgpu::TextureFormat::Rgba8UnormSrgb
            }
            Some(wgpu::TextureFormat::Bgra8UnormSrgb) | _ => {
                tracing::warn!("Using format {:?}", wgpu::TextureFormat::Bgra8UnormSrgb);

                color = Color::SRGB;
                texel = Texel::new_u8(SampleParts::BgrA);
                wgpu::TextureFormat::Bgra8UnormSrgb
            }
        };

        let (width, height) = (canvas.width(), canvas.height());

        let config = SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format: preferred_format,
            width,
            height,
            present_mode: wgpu::PresentMode::AutoVsync,
            view_formats: [preferred_format].to_vec(),
            alpha_mode: Default::default(),
            desired_maximum_frame_latency: 0,
        };

        let descriptor = Descriptor {
            color,
            ..Descriptor::with_texel(texel, width, height).unwrap()
        };

        let empty = image::DynamicImage::new_rgba16(0, 0);

        let mut that = Surface {
            adapter: Arc::new(adapter),
            config,
            window: Window { canvas, surface },
            instance,
            pool: Pool::new(),
            entry: PoolEntry {
                gpu: None,
                key: None,
                presentable: PoolKey::null(),
                descriptor,
            },
            runtimes: Runtimes::default(),
        };

        let gpu = that.reconfigure_gpu();
        that.entry.gpu = Some(gpu);
        let surface = that.pool.declare(that.descriptor());
        that.entry.key = Some(surface.key());
        that.reconfigure_surface()?;
        // Create a nul image to ''present'' while booting.
        let presentable = that.pool.declare(Descriptor::with_srgb_image(&empty)).key();
        that.entry.presentable = presentable;

        Ok(that)
    }

    pub fn adapter(&self) -> Arc<Adapter> {
        self.adapter.clone()
    }

    /// Create a pool that shares the device with this surface.
    ///
    /// The pool can separately render textures which the surface's pool can then display.
    pub fn configure_pool(&mut self, pool: &mut Pool) -> (GpuKey, PoolBridge) {
        tracing::info!("Surface reconfiguring pool device");
        let internal_key = self.reconfigure_gpu();

        let mut bridge = PoolBridge::default();
        let shared_key = self
            .pool
            .share_device(internal_key, pool)
            .expect("maintained incorrect gpu key");
        bridge.add_translated_gpu(internal_key, shared_key);

        (shared_key, bridge)
    }

    /// Create a swap chain in our pool, for the presented texture.
    pub fn configure_swap_chain(&mut self, n: usize) -> SwapChain {
        self.pool.swap_chain(self.entry.presentable, n)
    }

    pub fn swap_into(&mut self, entry: PoolImageMut, img: PoolKey, bridge: &PoolBridge) {
        let present = self.pool.entry(img).unwrap();
        bridge.swap_image(present, entry);
    }

    /// Change the base device.
    pub(crate) fn reconfigure_gpu(&mut self) -> GpuKey {
        if let Some(gpu) = self.entry.gpu {
            gpu
        } else {
            tracing::info!("No gpu key, device lost or not initialized?");
            let mut descriptor = Program::minimal_device_descriptor();
            descriptor.required_limits.max_texture_dimension_1d = 4096;
            descriptor.required_limits.max_texture_dimension_2d = 4096;

            let gpu = self
                .pool
                .request_device(&self.adapter, descriptor)
                .expect("to get a device");

            gpu
        }
    }

    /// Get a new presentable image from the CPU host.
    pub fn set_image(&mut self, image: &image::DynamicImage) {
        tracing::info!("Uploading DynamicImage {:?} to GPU buffer", image.color());
        let gpu = self.reconfigure_gpu();
        let key = self.entry.presentable;
        let mut entry = self.pool.entry(key).unwrap();
        entry.set_srgb(&image);
        self.pool.upload(key, gpu).unwrap();
    }

    /// Get a new presentable image from the swap chain.
    pub fn set_from_swap_chain(&mut self, chain: &mut SwapChain) {
        chain.present(&mut self.pool)
    }

    pub fn get_current_texture(&mut self) -> Result<wgpu::SurfaceTexture, wgpu::SurfaceError> {
        self.window.surface.get_current_texture()
    }

    pub fn present_to_texture(&mut self, surface_tex: &mut wgpu::SurfaceTexture) {
        let gpu = match self.entry.gpu {
            Some(key) => key,
            None => {
                tracing::warn!("No gpu to paint with.");
                return;
            }
        };

        let surface = match self.entry.key {
            Some(key) => key,
            None => {
                tracing::warn!("No surface to paint to.");
                return;
            }
        };

        let present = self.entry.presentable;

        #[cfg(not(target_arch = "wasm32"))]
        let start = std::time::Instant::now();

        let present_desc = self.pool.entry(present).unwrap().descriptor();
        let surface_desc = self.pool.entry(surface).unwrap().descriptor();

        let device = self.pool.iter_devices().next().unwrap();
        let capabilities = Capabilities::from(device);

        let normalize = self
            .runtimes
            .get_or_insert_normalizing_exe(present_desc, surface_desc, capabilities)
            .expect("Should be able to build resize");

        let in_reg = normalize.in_reg;
        let out_reg = normalize.out_reg;

        self.pool
            .entry(surface)
            .unwrap()
            .replace_texture_unguarded(&mut surface_tex.texture, gpu);

        let mut run = normalize
            .exe
            .from_pool(&mut self.pool)
            .expect("Valid pool for our own executable");

        // Bind the input.
        run.bind(in_reg, present)
            .expect("Valid binding for our executable input");
        // Bind the output.
        run.bind_render(out_reg, surface)
            .expect("Valid binding for our executable output");
        tracing::warn!("Sub- optimality: {:?}", surface_tex.suboptimal);
        let recovered = run.recover_buffers();
        tracing::warn!("{:?}", recovered);

        let mut running = normalize
            .exe
            .launch(run)
            .expect("Valid binding to start our executable");

        // Ensure our cache does not grow infinitely.
        self.pool.clear_cache();

        // FIXME: No. Async. Luckily this is straightforward.
        while running.is_running() {
            let limits = StepLimits::new().with_steps(usize::MAX);
            let mut step = running
                .step_to(limits)
                .expect("Valid binding to start our executable");
            step.block_on()
                .expect("Valid binding to block on our execution");
        }

        tracing::warn!("{:?}", running.resources_used());
        let mut retire = running.retire_gracefully(&mut self.pool);
        retire
            .input(in_reg)
            .expect("Valid to retire input of our executable");
        retire
            .render(out_reg)
            .expect("Valid to retire outputof our executable");

        retire.prune();
        let retired = retire.retire_buffers();
        tracing::warn!("{:?}", retired);
        retire.finish();

        self.pool
            .entry(surface)
            .unwrap()
            .replace_texture_unguarded(&mut surface_tex.texture, gpu);
    }

    pub fn descriptor(&self) -> Descriptor {
        self.entry.descriptor.clone()
    }

    pub fn lost(&mut self) -> Result<(), PresentationError> {
        self.reconfigure_surface()
    }

    pub fn outdated(&mut self) -> Result<(), PresentationError> {
        self.reconfigure_surface()
    }

    fn reconfigure_surface(&mut self) -> Result<(), PresentationError> {
        let surface = match self.entry.key {
            Some(key) => key,
            None => {
                tracing::warn!("No surface to paint to.");
                return Err(PresentationError::GpuDeviceLost);
            }
        };

        let (width, height) = (self.window.canvas.width(), self.window.canvas.height());
        self.config.width = width;
        self.config.height = height;

        let mut surface = self.pool.entry(surface).unwrap();
        let old_descriptor = surface.descriptor().clone();
        surface.declare(Descriptor {
            color: old_descriptor.color,
            ..Descriptor::with_texel(old_descriptor.texel, width, height).unwrap()
        });

        // Could also be done in `get_or_insert_normalizing_exe` by storing the relevant input
        // parameters or by assigning version increments to each configuration.
        // FIXME: this would help reuse partial state.
        self.runtimes.normalizing = None;

        let dev = match self.pool.iter_devices().next() {
            Some(dev) => dev,
            None => {
                eprintln!("Lost device for screen rendering");
                return Err(PresentationError::GpuDeviceLost);
            }
        };

        tracing::info!("Reconfigured surface {:?}", &self.config);
        self.window.surface.configure(dev, &self.config);

        Ok(())
    }
}

impl Runtimes {
    pub(crate) fn get_or_insert_normalizing_exe(
        &mut self,
        // The descriptor of the to-render output.
        input: Descriptor,
        // The surface descriptor.
        surface: Descriptor,
        // Capabilities to use for conversion.
        caps: Capabilities,
    ) -> Result<&mut NormalizingExe, NormalizingError> {
        if let Some(normalize) = &self.normalizing {
            if input == normalize.in_descriptor && surface == normalize.out_descriptor {
                return Ok(self.normalizing.as_mut().unwrap());
            }
        }

        let mut cmd = command::CommandBuffer::default();
        let in_reg = cmd.input(input.clone())?;
        let resized = cmd.resize(in_reg, surface.size())?;
        let converted = cmd.color_convert(resized, surface.color.clone(), surface.texel.clone())?;
        let (out_reg, _desc) = cmd.render(converted)?;

        let program = cmd.compile()?;
        let exe = program.lower_to(caps)?;

        tracing::info!("{}", exe.dot());

        Ok(self.normalizing.get_or_insert(NormalizingExe {
            exe: Arc::new(exe),
            in_descriptor: input,
            out_descriptor: surface,
            in_reg,
            out_reg,
        }))
    }
}

impl From<CompileError> for NormalizingError {
    #[track_caller]
    fn from(err: CompileError) -> Self {
        let location = core::panic::Location::caller();
        NormalizingError {
            fail: format!("At {:?}: {:?}", location, err),
        }
    }
}

impl From<command::CommandError> for NormalizingError {
    #[track_caller]
    fn from(err: command::CommandError) -> Self {
        let location = core::panic::Location::caller();
        NormalizingError {
            fail: format!("At {:?}: {:?}", location, err),
        }
    }
}

impl From<LaunchError> for NormalizingError {
    #[track_caller]
    fn from(err: LaunchError) -> Self {
        let location = core::panic::Location::caller();
        NormalizingError {
            fail: format!("At {:?}: {:?}", location, err),
        }
    }
}
