//! wgpu renderer: one instance buffer per frame, assembled from per-chunk caches,
//! drawn into an offscreen scene and then outlined onto the screen.
//!
//! Each visible chunk's instance lists are cached on the CPU and rebuilt only when
//! that chunk's revision changes, so the neighbour scanning that autotiling needs
//! is paid on edit rather than per frame. The lists are then copied into a single
//! vertex buffer — all bases first, then all overlays, so alpha blending lands in
//! the right order, then trees, the player and mobs.
//!
//! The scene pass writes colour and, alongside it, which object covers each pixel.
//! The outline pass then darkens pixels that have something behind them nearby.

use std::sync::Arc;
use std::time::{Duration, Instant};

use winit::window::Window;

use crate::atlas;
use crate::autotile::{self, Instance};
use crate::backdrop::{self, BackdropUniform};
use crate::camera::Camera;
use crate::entity::{Body, Entities, Sprite};
use crate::light::LightMap;
use crate::sky;
use crate::player::{self, Player};
use crate::trees;
use crate::world::{CHUNK_SIZE, World};

const INSTANCE_ATTRS: [wgpu::VertexAttribute; 7] = wgpu::vertex_attr_array![
    0 => Float32x2, 1 => Float32x2, 2 => Float32x2, 3 => Uint16x2, 4 => Unorm8x4, 5 => Uint32, 6 => Uint32
];

/// Per-pixel object ids, compared by the outline pass.
const ID_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R32Uint;
/// Outline thickness in art pixels of a 16-pixel tile, as Terraria's art has it.
const OUTLINE_ART_PX: f32 = 2.0;
// ponytail: one unlit near-black; per-material dark shades are the art-direction upgrade.
const OUTLINE_COLOUR: [f32; 4] = [0.02, 0.02, 0.03, 1.0];
/// How strongly air in direct light glows. Light scattering off dust and haze is
/// what makes a beam visible; shadowed air beside it has none, so beams show
/// wherever leaves or ground break the light up.
const RAY_STRENGTH: f32 = 0.14;
/// Brightness of direct light at noon, where rays are at full strength.
const DAY_DIRECT: f32 = 0.6;

#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct OutlineUniform {
    colour: [f32; 4],
    /// Glow added to open air in direct light: the god rays.
    ray: [f32; 4],
    /// World position of the top-left pixel corner.
    origin: [f32; 2],
    /// The sun or moon on screen, in world tiles: where the beams fan out from.
    source: [f32; 2],
    ppt: f32,
    width: i32,
    _pad: [f32; 2],
}

/// The offscreen scene the outline pass reads, sized to the target.
struct Scene {
    colour: wgpu::TextureView,
    ids: wgpu::TextureView,
    bind_group: wgpu::BindGroup,
    size: (u32, u32),
}

/// `ndc = tile * scale + offset`, plus the atlas geometry the shader needs to turn
/// a cell index into texture coordinates.
#[repr(C)]
#[derive(Clone, Copy, bytemuck::Pod, bytemuck::Zeroable)]
struct ViewUniform {
    scale: [f32; 2],
    offset: [f32; 2],
    atlas_size: [f32; 2],
    cell: f32,
    mask_cols: f32,
    mask_band_y: f32,
    block: f32,
    block_tiles: f32,
    /// WGSL aligns `vec2<f32>` to 8 bytes, so the light fields start at 48 whether
    /// this is written down or not. Spelling it out keeps both sides identical.
    _pad: f32,
    light_origin: [f32; 2],
    light_size: [f32; 2],
    /// Direct sun or moonlight, added where the direct-light texture says it lands.
    direct: [f32; 4],
}

/// The light map and the direct-light texture over the same region, replaced
/// together when the region changes size.
struct LightTextures {
    light: wgpu::Texture,
    sun: wgpu::Texture,
    size: [i32; 2],
}

#[derive(Default)]
struct ChunkCache {
    /// Revision this cache was built from; `None` means never built.
    revision: Option<u32>,
    layers: autotile::Layers,
}

/// Offscreen captures render in sRGB so the written PNG matches what a window shows.
const CAPTURE_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8UnormSrgb;

/// The backdrop covers every pixel of the scene; black is what the outline pass
/// clears its target to before overwriting all of it.
const CLEAR: wgpu::Color = wgpu::Color { r: 0.0, g: 0.0, b: 0.0, a: 1.0 };
const INITIAL_INSTANCES: usize = 64 * 1024;

/// Where frames go. A window is the normal case; the offscreen target exists so
/// rendering can be verified and captured without depending on a visible window,
/// which macOS does not guarantee for a process launched from a terminal.
enum Target {
    Window {
        surface: wgpu::Surface<'static>,
        config: wgpu::SurfaceConfiguration,
    },
    Offscreen {
        texture: wgpu::Texture,
        size: (u32, u32),
    },
}

pub struct Renderer {
    target: Target,
    device: wgpu::Device,
    queue: wgpu::Queue,
    pipeline: wgpu::RenderPipeline,
    soft_pipeline: wgpu::RenderPipeline,
    outline_pipeline: wgpu::RenderPipeline,
    backdrop_pipeline: wgpu::RenderPipeline,
    backdrop_buffer: wgpu::Buffer,
    backdrop_bind_group: wgpu::BindGroup,
    outline_layout: wgpu::BindGroupLayout,
    outline_buffer: wgpu::Buffer,
    format: wgpu::TextureFormat,
    scene: Option<Scene>,
    /// Where the veils end, and where the overlays start and end, in `instances`:
    /// veils, then bases, then overlays, then objects.
    veils_end: usize,
    overlays: std::ops::Range<usize>,
    view_buffer: wgpu::Buffer,
    bind_group: wgpu::BindGroup,
    bind_layout: wgpu::BindGroupLayout,
    atlas_view: wgpu::TextureView,
    atlas_sampler: wgpu::Sampler,
    light_sampler: wgpu::Sampler,
    light_textures: LightTextures,
    instance_buffer: wgpu::Buffer,
    instance_capacity: usize,
    instances: Vec<Instance>,
    chunk_caches: Vec<ChunkCache>,
    presented: u64,
    /// Cost of rebuilding caches and copying them into `instances`, last frame.
    assembly_time: Duration,
    rebuilt_chunks: usize,
    /// Worst assembly seen, which is whichever frame rebuilt the most chunks.
    peak_assembly: Duration,
    peak_rebuilt: usize,
}

impl Renderer {
    pub fn new(window: Arc<Window>, world: &World) -> Renderer {
        let size = window.inner_size();
        let instance = wgpu::Instance::default();
        let surface = instance.create_surface(window).expect("create surface");
        let (adapter, device, queue) = request_gpu(&instance, Some(&surface));

        let config = surface
            .get_default_config(&adapter, size.width.max(1), size.height.max(1))
            .expect("surface is unsupported by this adapter");
        surface.configure(&device, &config);
        let format = config.format;

        Renderer::build(device, queue, format, Target::Window { surface, config }, world)
    }

    /// A renderer with no window, drawing into a texture that can be read back.
    pub fn offscreen(world: &World, width: u32, height: u32) -> Renderer {
        let instance = wgpu::Instance::default();
        let (_adapter, device, queue) = request_gpu(&instance, None);
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("offscreen target"),
            size: wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: CAPTURE_FORMAT,
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::COPY_SRC,
            view_formats: &[],
        });
        Renderer::build(
            device,
            queue,
            CAPTURE_FORMAT,
            Target::Offscreen { texture, size: (width, height) },
            world,
        )
    }

    fn build(
        device: wgpu::Device,
        queue: wgpu::Queue,
        format: wgpu::TextureFormat,
        target: Target,
        world: &World,
    ) -> Renderer {
        let shader = device.create_shader_module(wgpu::include_wgsl!("shader.wgsl"));

        let (pixels, atlas_width, atlas_height) = atlas::load_or_generate();
        let texture = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("tile atlas"),
            size: wgpu::Extent3d {
                width: atlas_width,
                height: atlas_height,
                depth_or_array_layers: 1,
            },
            mip_level_count: 1,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        queue.write_texture(
            texture.as_image_copy(),
            &pixels,
            wgpu::TexelCopyBufferLayout {
                offset: 0,
                bytes_per_row: Some(atlas_width * 4),
                rows_per_image: Some(atlas_height),
            },
            wgpu::Extent3d { width: atlas_width, height: atlas_height, depth_or_array_layers: 1 },
        );
        let texture_view = texture.create_view(&wgpu::TextureViewDescriptor::default());
        // ponytail: nearest filtering keeps art crisp at 1:1 but aliases when zoomed
        // far out; generate mipmaps if the zoomed-out view starts to matter.
        let sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("atlas sampler"),
            mag_filter: wgpu::FilterMode::Nearest,
            min_filter: wgpu::FilterMode::Nearest,
            ..Default::default()
        });

        let view_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("view uniform"),
            size: size_of::<ViewUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("frame layout"),
            entries: &[
                wgpu::BindGroupLayoutEntry {
                    binding: 0,
                    // The fragment stage reads the direct light colour from it.
                    visibility: wgpu::ShaderStages::VERTEX_FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 1,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: false },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::NonFiltering),
                    count: None,
                },
                // The light map is filtered, unlike the atlas: smooth light is the
                // whole point, and nearest sampling would give one flat shade per tile.
                wgpu::BindGroupLayoutEntry {
                    binding: 3,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
                wgpu::BindGroupLayoutEntry {
                    binding: 4,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
                    count: None,
                },
                // Direct light, at half-tile resolution, sampled like the light map.
                wgpu::BindGroupLayoutEntry {
                    binding: 5,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Texture {
                        sample_type: wgpu::TextureSampleType::Float { filterable: true },
                        view_dimension: wgpu::TextureViewDimension::D2,
                        multisampled: false,
                    },
                    count: None,
                },
            ],
        });
        let light_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("light sampler"),
            address_mode_u: wgpu::AddressMode::ClampToEdge,
            address_mode_v: wgpu::AddressMode::ClampToEdge,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        // Placeholders until the first frame sizes the real ones.
        let (light, light_view) = new_texture(&device, &queue, LIGHT_FORMAT, [1, 1], &[255; 4]);
        let (sun, sun_view) = new_texture(&device, &queue, SUN_FORMAT, [1, 1], &[0]);
        let bind_group = frame_bind_group(
            &device, &layout, &view_buffer, &texture_view, &sampler, &light_view, &light_sampler, &sun_view,
        );

        let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tile pipeline layout"),
            bind_group_layouts: &[Some(&layout)],
            immediate_size: 0,
        });
        let scene_pipeline = |entry: &str, ids: wgpu::ColorWrites| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&pipeline_layout),
                vertex: wgpu::VertexState {
                    module: &shader,
                    entry_point: Some("vs_main"),
                    compilation_options: Default::default(),
                    buffers: &[Some(wgpu::VertexBufferLayout {
                        array_stride: size_of::<Instance>() as wgpu::BufferAddress,
                        step_mode: wgpu::VertexStepMode::Instance,
                        attributes: &INSTANCE_ATTRS,
                    })],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &shader,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[
                        Some(wgpu::ColorTargetState {
                            format,
                            // Overlays are alpha-composited over the bases drawn before them.
                            blend: Some(wgpu::BlendState::ALPHA_BLENDING),
                            write_mask: wgpu::ColorWrites::ALL,
                        }),
                        Some(wgpu::ColorTargetState { format: ID_FORMAT, blend: None, write_mask: ids }),
                    ],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: wgpu::MultisampleState::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let pipeline = scene_pipeline("fs_main", wgpu::ColorWrites::ALL);
        let soft_pipeline = scene_pipeline("fs_soft", wgpu::ColorWrites::empty());

        let outline_shader = device.create_shader_module(wgpu::include_wgsl!("outline.wgsl"));
        let texture_entry = |binding, sample_type| wgpu::BindGroupLayoutEntry {
            binding,
            visibility: wgpu::ShaderStages::FRAGMENT,
            ty: wgpu::BindingType::Texture {
                sample_type,
                view_dimension: wgpu::TextureViewDimension::D2,
                multisampled: false,
            },
            count: None,
        };
        let outline_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("outline layout"),
            entries: &[
                texture_entry(0, wgpu::TextureSampleType::Float { filterable: false }),
                texture_entry(1, wgpu::TextureSampleType::Uint),
                wgpu::BindGroupLayoutEntry {
                    binding: 2,
                    visibility: wgpu::ShaderStages::FRAGMENT,
                    ty: wgpu::BindingType::Buffer {
                        ty: wgpu::BufferBindingType::Uniform,
                        has_dynamic_offset: false,
                        min_binding_size: None,
                    },
                    count: None,
                },
            ],
        });
        let outline_pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("outline pipeline layout"),
            bind_group_layouts: &[Some(&outline_layout)],
            immediate_size: 0,
        });
        let outline_pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("outline pipeline"),
            layout: Some(&outline_pipeline_layout),
            vertex: wgpu::VertexState {
                module: &outline_shader,
                entry_point: Some("vs_full"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &outline_shader,
                entry_point: Some("fs_outline"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: None,
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: wgpu::MultisampleState::default(),
            multiview_mask: None,
            cache: None,
        });
        let outline_buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("outline uniform"),
            size: size_of::<OutlineUniform>() as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        let (backdrop_pipeline, backdrop_buffer, backdrop_bind_group) =
            backdrop_resources(&device, &queue, format);

        let instance_buffer = new_instance_buffer(&device, INITIAL_INSTANCES);
        let chunk_count = (world.width_chunks() * world.height_chunks()) as usize;
        Renderer {
            target,
            device,
            queue,
            pipeline,
            soft_pipeline,
            outline_pipeline,
            backdrop_pipeline,
            backdrop_buffer,
            backdrop_bind_group,
            outline_layout,
            outline_buffer,
            format,
            scene: None,
            veils_end: 0,
            overlays: 0..0,
            view_buffer,
            bind_group,
            bind_layout: layout,
            atlas_view: texture_view,
            atlas_sampler: sampler,
            light_sampler,
            light_textures: LightTextures { light, sun, size: [1, 1] },
            instance_buffer,
            instance_capacity: INITIAL_INSTANCES,
            instances: Vec::with_capacity(INITIAL_INSTANCES),
            chunk_caches: (0..chunk_count).map(|_| ChunkCache::default()).collect(),
            presented: 0,
            assembly_time: Duration::ZERO,
            rebuilt_chunks: 0,
            peak_assembly: Duration::ZERO,
            peak_rebuilt: 0,
        }
    }

    pub fn resize(&mut self, width: u32, height: u32) {
        if let Target::Window { surface, config } = &mut self.target {
            config.width = width.max(1);
            config.height = height.max(1);
            surface.configure(&self.device, config);
        }
    }

    /// Replaces the light texture when the lit region changes shape, then uploads
    /// this frame's values.
    fn sync_light(&mut self, light: &LightMap) {
        let size = light.size();
        if self.light_textures.size != size {
            let (light_texture, light_view) =
                new_texture(&self.device, &self.queue, LIGHT_FORMAT, size, light.texels());
            let (sun, sun_view) =
                new_texture(&self.device, &self.queue, SUN_FORMAT, light.sun_size(), light.sun_texels());
            self.bind_group = frame_bind_group(
                &self.device,
                &self.bind_layout,
                &self.view_buffer,
                &self.atlas_view,
                &self.atlas_sampler,
                &light_view,
                &self.light_sampler,
                &sun_view,
            );
            self.light_textures = LightTextures { light: light_texture, sun, size };
        } else {
            upload(&self.queue, &self.light_textures.light, size, light.texels());
            upload(&self.queue, &self.light_textures.sun, light.sun_size(), light.sun_texels());
        }
    }

    pub fn render(
        &mut self,
        world: &World,
        camera: &Camera,
        player: &Player,
        entities: &Entities,
        light: &LightMap,
    ) {
        let frame = match &self.target {
            Target::Window { surface, .. } => match surface.get_current_texture() {
                wgpu::CurrentSurfaceTexture::Success(frame)
                | wgpu::CurrentSurfaceTexture::Suboptimal(frame) => Some(frame),
                // Occluded, outdated or lost surfaces resolve themselves on a later frame.
                _ => return,
            },
            Target::Offscreen { .. } => None,
        };

        self.sync_light(light);
        let lighting = world.sky.lighting();

        let started = Instant::now();
        self.assemble(world, camera);
        // Trees after the terrain, before anything that walks in front of them.
        let mut next_id = autotile::FIRST_OBJECT_ID;
        let view_tiles = camera.visible_tiles(world.width(), world.height());
        trees::emit(world, view_tiles, &mut next_id, &mut self.instances);
        let player_sprite =
            Sprite::standing_on(player::HALF, player::SPRITE_HALF, player::COLOR);
        let drawn = player.visual_pos();
        self.instances.push(autotile::tinted_box(
            player_sprite.corner(drawn),
            player_sprite.size(),
            player_sprite.tint,
            next_id,
        ));
        for (body, sprite) in entities.query::<(&Body, &Sprite)>().iter() {
            next_id += 1;
            self.instances.push(autotile::tinted_box(
                sprite.corner(body.pos),
                sprite.size(),
                sprite.tint,
                next_id,
            ));
        }
        self.assembly_time = started.elapsed();
        if self.assembly_time > self.peak_assembly {
            self.peak_assembly = self.assembly_time;
            self.peak_rebuilt = self.rebuilt_chunks;
        }

        if self.instances.len() > self.instance_capacity {
            self.instance_capacity = self.instances.len().next_power_of_two();
            self.instance_buffer = new_instance_buffer(&self.device, self.instance_capacity);
        }
        self.queue
            .write_buffer(&self.instance_buffer, 0, bytemuck::cast_slice(&self.instances));

        let view = camera.view_transform();
        self.queue.write_buffer(
            &self.view_buffer,
            0,
            bytemuck::bytes_of(&ViewUniform {
                scale: view.scale,
                offset: view.offset,
                atlas_size: [atlas::WIDTH as f32, atlas::HEIGHT as f32],
                cell: atlas::CELL as f32,
                mask_cols: atlas::MASK_COLS as f32,
                mask_band_y: atlas::MASK_BAND_Y as f32,
                block: atlas::BLOCK as f32,
                block_tiles: atlas::BLOCK_TILES as f32,
                _pad: 0.0,
                light_origin: [light.origin()[0] as f32, light.origin()[1] as f32],
                light_size: [light.size()[0] as f32, light.size()[1] as f32],
                direct: [lighting.direct[0], lighting.direct[1], lighting.direct[2], lighting.toward[0]],
            }),
        );

        self.queue.write_buffer(
            &self.backdrop_buffer,
            0,
            bytemuck::bytes_of(&backdrop::FOREST.uniform(camera, world.horizon(), world.forest.clock, &lighting)),
        );

        let size = match &self.target {
            Target::Window { config, .. } => (config.width, config.height),
            Target::Offscreen { size, .. } => *size,
        };
        if self.scene.as_ref().is_none_or(|scene| scene.size != size) {
            self.scene = Some(self.new_scene(size));
        }
        let width = (camera.pixels_per_tile() * OUTLINE_ART_PX / atlas::CELL as f32).round().max(1.0);
        self.queue.write_buffer(
            &self.outline_buffer,
            0,
            bytemuck::bytes_of(&OutlineUniform {
                colour: OUTLINE_COLOUR,
                ray: {
                    // Beams only show in strong light: weighting by brightness cubed
                    // keeps noon's and all but erases them by moonlight, which
                    // otherwise greys the whole night sky.
                    let strength = RAY_STRENGTH * (sky::luminance(lighting.direct) / DAY_DIRECT).powi(3).min(1.0);
                    let d = lighting.direct.map(|c| c * strength);
                    [d[0], d[1], d[2], 0.0]
                },
                origin: camera.px_to_tile([0.0, 0.0]),
                source: backdrop::body_centre(camera, lighting.toward[0].atan2(-lighting.toward[1])),
                ppt: camera.pixels_per_tile(),
                width: width as i32,
                _pad: [0.0; 2],
            }),
        );
        let scene = self.scene.as_ref().expect("created above");

        let target = match (&frame, &self.target) {
            (Some(frame), _) => frame.texture.create_view(&Default::default()),
            (None, Target::Offscreen { texture, .. }) => texture.create_view(&Default::default()),
            (None, Target::Window { .. }) => unreachable!("a window frame was acquired above"),
        };
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let attachment = |view, clear| {
                Some(wgpu::RenderPassColorAttachment {
                    view,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(clear), store: wgpu::StoreOp::Store },
                })
            };
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[
                    attachment(&scene.colour, CLEAR),
                    // Background id everywhere nothing is drawn.
                    attachment(&scene.ids, wgpu::Color::TRANSPARENT),
                ],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.backdrop_pipeline);
            pass.set_bind_group(0, &self.backdrop_bind_group, &[]);
            pass.draw(0..3, 0..1);
            pass.set_bind_group(0, &self.bind_group, &[]);
            pass.set_vertex_buffer(0, self.instance_buffer.slice(..));
            let (start, end) = (self.overlays.start as u32, self.overlays.end as u32);
            let veils = self.veils_end as u32;
            pass.set_pipeline(&self.soft_pipeline);
            pass.draw(0..6, 0..veils);
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..6, veils..start);
            pass.set_pipeline(&self.soft_pipeline);
            pass.draw(0..6, start..end);
            pass.set_pipeline(&self.pipeline);
            pass.draw(0..6, end..self.instances.len() as u32);
        }
        {
            let mut pass = encoder.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("outline"),
                color_attachments: &[Some(wgpu::RenderPassColorAttachment {
                    view: &target,
                    depth_slice: None,
                    resolve_target: None,
                    ops: wgpu::Operations { load: wgpu::LoadOp::Clear(CLEAR), store: wgpu::StoreOp::Store },
                })],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.outline_pipeline);
            pass.set_bind_group(0, &scene.bind_group, &[]);
            pass.draw(0..3, 0..1);
        }
        self.queue.submit([encoder.finish()]);
        if let Some(frame) = frame {
            self.queue.present(frame);
        }
        self.presented += 1;
    }

    /// Reads the offscreen target back and writes it as a PNG.
    pub fn save_capture(&self, path: &str) -> Result<(), Box<dyn std::error::Error>> {
        let Target::Offscreen { texture, size } = &self.target else {
            return Err("save_capture needs an offscreen renderer".into());
        };
        let (width, height) = *size;
        // Buffer rows must be 256-byte aligned, so the readback is padded and
        // trimmed row by row afterwards.
        let padded = (width * 4).next_multiple_of(wgpu::COPY_BYTES_PER_ROW_ALIGNMENT);
        let buffer = self.device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("capture readback"),
            size: (padded * height) as wgpu::BufferAddress,
            usage: wgpu::BufferUsages::COPY_DST | wgpu::BufferUsages::MAP_READ,
            mapped_at_creation: false,
        });
        let mut encoder = self
            .device
            .create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("capture") });
        encoder.copy_texture_to_buffer(
            texture.as_image_copy(),
            wgpu::TexelCopyBufferInfo {
                buffer: &buffer,
                layout: wgpu::TexelCopyBufferLayout {
                    offset: 0,
                    bytes_per_row: Some(padded),
                    rows_per_image: Some(height),
                },
            },
            wgpu::Extent3d { width, height, depth_or_array_layers: 1 },
        );
        self.queue.submit([encoder.finish()]);

        buffer.slice(..).map_async(wgpu::MapMode::Read, |r| r.expect("map readback"));
        self.device.poll(wgpu::PollType::Wait { submission_index: None, timeout: None })?;

        let mapped = buffer.slice(..).get_mapped_range()?;
        let mut pixels = Vec::with_capacity((width * height * 4) as usize);
        for row in 0..height {
            let start = (row * padded) as usize;
            pixels.extend_from_slice(&mapped[start..start + (width * 4) as usize]);
        }
        drop(mapped);
        buffer.unmap();

        image::RgbaImage::from_raw(width, height, pixels)
            .ok_or("capture has the wrong length")?
            .save(path)?;
        Ok(())
    }

    fn new_scene(&self, size: (u32, u32)) -> Scene {
        let texture = |label, format| {
            self.device
                .create_texture(&wgpu::TextureDescriptor {
                    label: Some(label),
                    size: wgpu::Extent3d { width: size.0, height: size.1, depth_or_array_layers: 1 },
                    mip_level_count: 1,
                    sample_count: 1,
                    dimension: wgpu::TextureDimension::D2,
                    format,
                    usage: wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING,
                    view_formats: &[],
                })
                .create_view(&Default::default())
        };
        let colour = texture("scene colour", self.format);
        let ids = texture("scene ids", ID_FORMAT);
        let bind_group = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("outline bind group"),
            layout: &self.outline_layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&colour) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&ids) },
                wgpu::BindGroupEntry { binding: 2, resource: self.outline_buffer.as_entire_binding() },
            ],
        });
        Scene { colour, ids, bind_group, size }
    }

    /// Rebuilds stale caches for visible chunks, then copies them into `instances`:
    /// every veil first, then every base, then every overlay.
    fn assemble(&mut self, world: &World, camera: &Camera) {
        let rect = camera.visible_tiles(world.width(), world.height());
        let (x0, y0) = (rect.x0 / CHUNK_SIZE, rect.y0 / CHUNK_SIZE);
        let x1 = ((rect.x1 - 1) / CHUNK_SIZE).min(world.width_chunks() - 1);
        let y1 = ((rect.y1 - 1) / CHUNK_SIZE).min(world.height_chunks() - 1);
        self.rebuilt_chunks = 0;
        let visible: Vec<usize> = (y0..=y1)
            .flat_map(|cy| (x0..=x1).map(move |cx| (cx, cy)))
            .map(|(cx, cy)| {
                let index = (cy * world.width_chunks() + cx) as usize;
                let revision = world.chunk_revision(cx, cy);
                let cache = &mut self.chunk_caches[index];
                if cache.revision != Some(revision) {
                    autotile::build_chunk(world, cx, cy, &mut cache.layers);
                    cache.revision = Some(revision);
                    self.rebuilt_chunks += 1;
                }
                index
            })
            .collect();

        self.instances.clear();
        for &i in &visible {
            self.instances.extend_from_slice(&self.chunk_caches[i].layers.veils);
        }
        self.veils_end = self.instances.len();
        for &i in &visible {
            self.instances.extend_from_slice(&self.chunk_caches[i].layers.bases);
        }
        let overlays_start = self.instances.len();
        for &i in &visible {
            self.instances.extend_from_slice(&self.chunk_caches[i].layers.overlays);
        }
        self.overlays = overlays_start..self.instances.len();
    }

    pub fn instance_count(&self) -> usize {
        self.instances.len()
    }

    /// Instances that are terrain transition overlays.
    pub fn overlay_count(&self) -> usize {
        self.overlays.len()
    }

    pub fn presented_frames(&self) -> u64 {
        self.presented
    }

    pub fn assembly_time(&self) -> Duration {
        self.assembly_time
    }

    pub fn rebuilt_chunks(&self) -> usize {
        self.rebuilt_chunks
    }

    pub fn peak_assembly(&self) -> (Duration, usize) {
        (self.peak_assembly, self.peak_rebuilt)
    }
}

/// The backdrop's pipeline, its per-frame uniform and the silhouette strips, which
/// are generated once and never change.
fn backdrop_resources(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
) -> (wgpu::RenderPipeline, wgpu::Buffer, wgpu::BindGroup) {
    let shader = device.create_shader_module(wgpu::include_wgsl!("backdrop.wgsl"));
    let size = wgpu::Extent3d {
        width: backdrop::STRIP as u32,
        height: backdrop::LAYERS as u32,
        depth_or_array_layers: 1,
    };
    let strips = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("backdrop strips"),
        size,
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::Rg32Float,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    queue.write_texture(
        strips.as_image_copy(),
        bytemuck::cast_slice(&backdrop::FOREST.strips()),
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(backdrop::STRIP as u32 * 8),
            rows_per_image: Some(backdrop::LAYERS as u32),
        },
        size,
    );
    let buffer = device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("backdrop uniform"),
        size: size_of::<BackdropUniform>() as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    });
    let layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
        label: Some("backdrop layout"),
        entries: &[
            wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Buffer {
                    ty: wgpu::BufferBindingType::Uniform,
                    has_dynamic_offset: false,
                    min_binding_size: None,
                },
                count: None,
            },
            wgpu::BindGroupLayoutEntry {
                binding: 1,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            },
        ],
    });
    let strips_view = strips.create_view(&Default::default());
    let bind_group = device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("backdrop bind group"),
        layout: &layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(&strips_view) },
        ],
    });
    let pipeline_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
        label: Some("backdrop pipeline layout"),
        bind_group_layouts: &[Some(&layout)],
        immediate_size: 0,
    });
    let pipeline = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
        label: Some("backdrop pipeline"),
        layout: Some(&pipeline_layout),
        vertex: wgpu::VertexState {
            module: &shader,
            entry_point: Some("vs_full"),
            compilation_options: Default::default(),
            buffers: &[],
        },
        fragment: Some(wgpu::FragmentState {
            module: &shader,
            entry_point: Some("fs_backdrop"),
            compilation_options: Default::default(),
            targets: &[
                Some(wgpu::ColorTargetState { format, blend: None, write_mask: wgpu::ColorWrites::ALL }),
                // The id target is already cleared to background.
                Some(wgpu::ColorTargetState { format: ID_FORMAT, blend: None, write_mask: wgpu::ColorWrites::empty() }),
            ],
        }),
        primitive: wgpu::PrimitiveState::default(),
        depth_stencil: None,
        multisample: wgpu::MultisampleState::default(),
        multiview_mask: None,
        cache: None,
    });
    (pipeline, buffer, bind_group)
}

/// Linear, not sRGB: these are multipliers, not colours to be gamma decoded.
const LIGHT_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Share of direct light reaching each half-tile cell.
const SUN_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::R8Unorm;

/// Rebuilt whenever the light textures are replaced, which happens only when the
/// lit region changes size — on resize or zoom, not per frame.
#[allow(clippy::too_many_arguments)]
fn frame_bind_group(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    view_buffer: &wgpu::Buffer,
    atlas_view: &wgpu::TextureView,
    atlas_sampler: &wgpu::Sampler,
    light_view: &wgpu::TextureView,
    light_sampler: &wgpu::Sampler,
    sun_view: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("frame bind group"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: view_buffer.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(atlas_view) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(atlas_sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(light_view) },
            wgpu::BindGroupEntry { binding: 4, resource: wgpu::BindingResource::Sampler(light_sampler) },
            wgpu::BindGroupEntry { binding: 5, resource: wgpu::BindingResource::TextureView(sun_view) },
        ],
    })
}

fn new_texture(
    device: &wgpu::Device,
    queue: &wgpu::Queue,
    format: wgpu::TextureFormat,
    size: [i32; 2],
    texels: &[u8],
) -> (wgpu::Texture, wgpu::TextureView) {
    let texture = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("light"),
        size: wgpu::Extent3d { width: size[0] as u32, height: size[1] as u32, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = texture.create_view(&wgpu::TextureViewDescriptor::default());
    upload(queue, &texture, size, texels);
    (texture, view)
}

fn upload(queue: &wgpu::Queue, texture: &wgpu::Texture, size: [i32; 2], texels: &[u8]) {
    let bytes = texture.format().block_copy_size(None).expect("an uncompressed format");
    queue.write_texture(
        texture.as_image_copy(),
        texels,
        wgpu::TexelCopyBufferLayout {
            offset: 0,
            bytes_per_row: Some(size[0] as u32 * bytes),
            rows_per_image: Some(size[1] as u32),
        },
        wgpu::Extent3d { width: size[0] as u32, height: size[1] as u32, depth_or_array_layers: 1 },
    );
}

fn request_gpu(
    instance: &wgpu::Instance,
    surface: Option<&wgpu::Surface<'static>>,
) -> (wgpu::Adapter, wgpu::Device, wgpu::Queue) {
    let adapter = pollster::block_on(instance.request_adapter(&wgpu::RequestAdapterOptions {
        power_preference: wgpu::PowerPreference::default(),
        force_fallback_adapter: false,
        compatible_surface: surface,
        ..Default::default()
    }))
    .expect("no suitable GPU adapter");
    let (device, queue) = pollster::block_on(adapter.request_device(&wgpu::DeviceDescriptor {
        label: Some("abyss device"),
        ..Default::default()
    }))
    .expect("request device");
    (adapter, device, queue)
}

fn new_instance_buffer(device: &wgpu::Device, capacity: usize) -> wgpu::Buffer {
    device.create_buffer(&wgpu::BufferDescriptor {
        label: Some("tile instances"),
        size: (capacity * size_of::<Instance>()) as wgpu::BufferAddress,
        usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
        mapped_at_creation: false,
    })
}
