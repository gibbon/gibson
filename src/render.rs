// GPU side: scene (floor + glass towers) into an HDR target, bloom, composite.

use crate::city::{City, Kind};
use crate::fs_scan::MAX_ENTRIES;
use crate::preview::{PV, PV_LAYERS, PV_MIPS};
use crate::textgen::{mip_size, MIP_LEVELS, TEX_H, TEX_W};
use bytemuck::{Pod, Zeroable};
use glam::{Mat4, Vec3};
use std::sync::Arc;
use wgpu::util::DeviceExt;
use winit::window::Window;

const HDR: wgpu::TextureFormat = wgpu::TextureFormat::Rgba16Float;
const DEPTH: wgpu::TextureFormat = wgpu::TextureFormat::Depth32Float;
const BLOOM_LEVELS: usize = 6;
const BLOOM_THRESHOLD: f32 = 1.2;

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
pub struct Globals {
    view_proj: [[f32; 4]; 4],
    cam: [f32; 4],
    city: [f32; 4],
    haze: [f32; 4],
    screen: [f32; 4],
    hl: [f32; 4],
    hl_rect: [f32; 4],
    th0: [f32; 4],
    th1: [f32; 4],
    th2: [f32; 4],
    th3: [f32; 4],
}

/// The media viewer, for this frame: which effect, the time, how far open (0..1),
/// and where the picture goes on screen (pixels).
#[derive(Clone, Copy)]
pub struct ViewerDraw {
    pub effect: u32,
    pub time: f32,
    pub open: f32,
    pub rect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct ViewerUniform {
    rect: [f32; 4],
    screen: [f32; 4],
    p: [f32; 4],
}

/// A name on a tower face to light up: the texture layer of its page, and its box
/// on that page in 0..1 page coordinates.
#[derive(Clone, Copy)]
pub struct Highlight {
    pub layer: u32,
    pub rect: [f32; 4],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct BoxVertex {
    pos: [f32; 3],
    nrm: [f32; 3],
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct Instance {
    base: [f32; 3],
    size: [f32; 3],
    text_col: [f32; 4],
    body_col: [f32; 4],
    misc: [f32; 4],
    pv: [f32; 4],      // preview: layer (−1 none), used width, used height (0..1), is video
    pv_rect: [f32; 4], // where it goes on the page: u0, v0, u1, v1
}

#[repr(C)]
#[derive(Clone, Copy, Pod, Zeroable)]
struct PostParams {
    texel: [f32; 4],
}

struct Target {
    view: wgpu::TextureView,
    w: u32,
    h: u32,
}

struct PostPass {
    bind: wgpu::BindGroup,
    params: wgpu::Buffer,
}

pub struct Renderer {
    surface: wgpu::Surface<'static>,
    device: wgpu::Device,
    queue: wgpu::Queue,
    config: wgpu::SurfaceConfiguration,

    globals_buf: wgpu::Buffer,
    globals_bind: wgpu::BindGroup,
    faces_tex: wgpu::Texture,
    faces_bind: wgpu::BindGroup,
    faces_layout: wgpu::BindGroupLayout,
    faces_sampler: wgpu::Sampler,
    preview_tex: wgpu::Texture,
    preview_view: wgpu::TextureView,
    face_capacity: u32,
    pub max_face_layers: u32,
    pub tower_fx: u32, // effect on tower previews (same list as the viewer)
    pub theme: usize,  // colour scheme (theme::THEMES)
    box_vb: wgpu::Buffer,
    inst_buf: wgpu::Buffer,

    floor_pipe: wgpu::RenderPipeline,
    tower_back: wgpu::RenderPipeline,
    tower_front: wgpu::RenderPipeline,
    bright_pipe: wgpu::RenderPipeline,
    down_pipe: wgpu::RenderPipeline,
    up_pipe: wgpu::RenderPipeline,
    composite_pipe: wgpu::RenderPipeline,
    post_layout: wgpu::BindGroupLayout,
    post_sampler: wgpu::Sampler,
    dummy: wgpu::TextureView,

    hdr: Target,
    depth: wgpu::TextureView,
    bloom: Vec<Target>,
    bright_pass: PostPass,
    down_passes: Vec<PostPass>,
    up_passes: Vec<PostPass>,
    composite_pass: PostPass,

    hud_layout: wgpu::BindGroupLayout,
    hud_pipe: wgpu::RenderPipeline,
    hud: Option<HudTex>,

    viewer_layout: wgpu::BindGroupLayout,
    viewer_pipe: wgpu::RenderPipeline,
    viewer_buf: wgpu::Buffer,
    viewer_sampler: wgpu::Sampler,
    viewer_img: (wgpu::Texture, wgpu::TextureView, u32, u32),
    glyph_view: wgpu::TextureView,
    viewer_bind: wgpu::BindGroup,
}

struct HudTex {
    tex: wgpu::Texture,
    bind: wgpu::BindGroup,
    w: u32,
    h: u32,
}

impl Renderer {
    pub async fn new(window: Arc<Window>) -> Renderer {
        let size = window.inner_size();
        let instance = wgpu::Instance::new(wgpu::InstanceDescriptor::new_without_display_handle_from_env());
        let surface = instance.create_surface(window).expect("create surface");
        let adapter = instance
            .request_adapter(&wgpu::RequestAdapterOptions {
                power_preference: wgpu::PowerPreference::HighPerformance,
                compatible_surface: Some(&surface),
                force_fallback_adapter: false,
                apply_limit_buckets: false,
            })
            .await
            .expect("no GPU adapter");
        log::info!("GPU: {:?}", adapter.get_info());
        let (device, queue) = adapter
            .request_device(&wgpu::DeviceDescriptor {
                label: Some("gibson"),
                required_limits: wgpu::Limits {
                    max_texture_array_layers: adapter.limits().max_texture_array_layers.min(crate::city::LAYER_BUDGET),
                    ..wgpu::Limits::default()
                }
                    .using_resolution(adapter.limits()),
                ..Default::default()
            })
            .await
            .expect("request device");

        let caps = surface.get_capabilities(&adapter);
        let format = caps.formats.iter().copied().find(|f| f.is_srgb()).unwrap_or(caps.formats[0]);
        let config = wgpu::SurfaceConfiguration {
            usage: wgpu::TextureUsages::RENDER_ATTACHMENT,
            format,
            width: size.width.max(1),
            height: size.height.max(1),
            present_mode: wgpu::PresentMode::AutoVsync,
            desired_maximum_frame_latency: 2,
            alpha_mode: caps.alpha_modes[0],
            view_formats: vec![],
            color_space: wgpu::SurfaceColorSpace::Auto,
        };
        surface.configure(&device, &config);
        log::info!("surface {}x{} {:?}", config.width, config.height, format);

        // ── Shared globals ───────────────────────────────────────────────────
        let globals_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("globals"),
            size: std::mem::size_of::<Globals>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let globals_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("globals"),
            entries: &[uniform_entry(0, wgpu::ShaderStages::VERTEX_FRAGMENT)],
        });
        let globals_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("globals"),
            layout: &globals_layout,
            entries: &[wgpu::BindGroupEntry { binding: 0, resource: globals_buf.as_entire_binding() }],
        });

        // ── Tower face textures: one array layer per face page (resized per city) ─
        let max_face_layers = device.limits().max_texture_array_layers;
        let faces_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("faces"),
            address_mode_u: wgpu::AddressMode::Repeat,
            address_mode_v: wgpu::AddressMode::Repeat,
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            mipmap_filter: wgpu::MipmapFilterMode::Linear,
            anisotropy_clamp: 8,
            ..Default::default()
        });
        let faces_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("faces"),
            entries: &[
                tex_entry(0, wgpu::TextureViewDimension::D2Array),
                sampler_entry(1),
                tex_entry(2, wgpu::TextureViewDimension::D2Array),
                tex_entry(3, wgpu::TextureViewDimension::D2),
            ],
        });
        let face_capacity = FACE_LAYERS_MIN.min(max_face_layers);
        // Colour previews (images, video frames): one RGBA layer each.
        let preview_tex = device.create_texture(&wgpu::TextureDescriptor {
            label: Some("previews"),
            size: wgpu::Extent3d { width: PV, height: PV, depth_or_array_layers: PV_LAYERS },
            mip_level_count: PV_MIPS,
            sample_count: 1,
            dimension: wgpu::TextureDimension::D2,
            format: wgpu::TextureFormat::Rgba8UnormSrgb,
            usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
            view_formats: &[],
        });
        let preview_view = preview_tex.create_view(&wgpu::TextureViewDescriptor {
            dimension: Some(wgpu::TextureViewDimension::D2Array),
            ..Default::default()
        });
        let glyph_view = make_viewer_tex(&device, 1, 1, wgpu::TextureFormat::R8Unorm).1;
        let faces_tex = make_faces(&device, face_capacity);
        let faces_bind = bind_faces(&device, &faces_layout, &faces_sampler, &faces_tex, &preview_view, &glyph_view);

        // ── Geometry ─────────────────────────────────────────────────────────
        let box_vb = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("box"),
            contents: bytemuck::cast_slice(&box_vertices()),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let inst_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("instances"),
            size: (std::mem::size_of::<Instance>() * MAX_ENTRIES) as u64,
            usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });

        // ── Pipelines ────────────────────────────────────────────────────────
        let floor_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("floor"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/floor.wgsl").into()),
        });
        let tower_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("tower"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/tower.wgsl").into()),
        });
        let post_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("post"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/post.wgsl").into()),
        });

        let floor_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("floor"),
            bind_group_layouts: &[Some(&globals_layout)],
            immediate_size: 0,
        });
        let floor_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("floor"),
            layout: Some(&floor_layout),
            vertex: wgpu::VertexState {
                module: &floor_mod,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &floor_mod,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState { format: HDR, blend: None, write_mask: wgpu::ColorWrites::ALL })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: Some(depth_state(true)),
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        let tower_layout = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("tower"),
            bind_group_layouts: &[Some(&globals_layout), Some(&faces_layout)],
            immediate_size: 0,
        });
        let box_attrs = wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3];
        let inst_attrs = wgpu::vertex_attr_array![2 => Float32x3, 3 => Float32x3, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4, 8 => Float32x4];
        let tower_pipe = |cull: wgpu::Face| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some("tower"),
                layout: Some(&tower_layout),
                vertex: wgpu::VertexState {
                    module: &tower_mod,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<BoxVertex>() as u64,
                            step_mode: wgpu::VertexStepMode::Vertex,
                            attributes: &box_attrs,
                        }),
                        Some(wgpu::VertexBufferLayout {
                            array_stride: std::mem::size_of::<Instance>() as u64,
                            step_mode: wgpu::VertexStepMode::Instance,
                            attributes: &inst_attrs,
                        }),
                    ],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &tower_mod,
                    entry_point: Some("fs"),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState {
                        format: HDR,
                        blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                        write_mask: wgpu::ColorWrites::ALL,
                    })],
                }),
                primitive: wgpu::PrimitiveState { cull_mode: Some(cull), ..Default::default() },
                depth_stencil: Some(depth_state(false)),
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let tower_back = tower_pipe(wgpu::Face::Front);
        let tower_front = tower_pipe(wgpu::Face::Back);

        let post_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("post"),
            entries: &[
                tex_entry(0, wgpu::TextureViewDimension::D2),
                sampler_entry(1),
                uniform_entry(2, wgpu::ShaderStages::FRAGMENT),
                tex_entry(3, wgpu::TextureViewDimension::D2),
            ],
        });
        let post_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("post"),
            bind_group_layouts: &[Some(&post_layout)],
            immediate_size: 0,
        });
        let post_pipe = |entry: &str, format: wgpu::TextureFormat, blend: Option<wgpu::BlendState>| {
            device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
                label: Some(entry),
                layout: Some(&post_pl),
                vertex: wgpu::VertexState {
                    module: &post_mod,
                    entry_point: Some("vs"),
                    compilation_options: Default::default(),
                    buffers: &[],
                },
                fragment: Some(wgpu::FragmentState {
                    module: &post_mod,
                    entry_point: Some(entry),
                    compilation_options: Default::default(),
                    targets: &[Some(wgpu::ColorTargetState { format, blend, write_mask: wgpu::ColorWrites::ALL })],
                }),
                primitive: wgpu::PrimitiveState::default(),
                depth_stencil: None,
                multisample: Default::default(),
                multiview_mask: None,
                cache: None,
            })
        };
        let additive = wgpu::BlendState {
            color: wgpu::BlendComponent {
                src_factor: wgpu::BlendFactor::One,
                dst_factor: wgpu::BlendFactor::One,
                operation: wgpu::BlendOperation::Add,
            },
            alpha: wgpu::BlendComponent::REPLACE,
        };
        let bright_pipe = post_pipe("fs_bright", HDR, None);
        let down_pipe = post_pipe("fs_down", HDR, None);
        let up_pipe = post_pipe("fs_up", HDR, Some(additive));
        let composite_pipe = post_pipe("fs_composite", format, None);

        let post_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("post"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let dummy = device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some("dummy"),
                size: wgpu::Extent3d { width: 1, height: 1, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: HDR,
                usage: wgpu::TextureUsages::TEXTURE_BINDING,
                view_formats: &[],
            })
            .create_view(&Default::default());

        // ── HUD overlay (premultiplied alpha over the final image) ───────────
        let hud_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("hud"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/hud.wgsl").into()),
        });
        let hud_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("hud"),
            entries: &[wgpu::BindGroupLayoutEntry {
                binding: 0,
                visibility: wgpu::ShaderStages::FRAGMENT,
                ty: wgpu::BindingType::Texture {
                    sample_type: wgpu::TextureSampleType::Float { filterable: false },
                    view_dimension: wgpu::TextureViewDimension::D2,
                    multisampled: false,
                },
                count: None,
            }],
        });
        let hud_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("hud"),
            bind_group_layouts: &[Some(&hud_layout)],
            immediate_size: 0,
        });
        let hud_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("hud"),
            layout: Some(&hud_pl),
            vertex: wgpu::VertexState {
                module: &hud_mod,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &hud_mod,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });

        // ── Media viewer ─────────────────────────────────────────────────────
        let viewer_mod = device.create_shader_module(wgpu::ShaderModuleDescriptor {
            label: Some("viewer"),
            source: wgpu::ShaderSource::Wgsl(include_str!("shaders/viewer.wgsl").into()),
        });
        let viewer_layout = device.create_bind_group_layout(&wgpu::BindGroupLayoutDescriptor {
            label: Some("viewer"),
            entries: &[
                uniform_entry(0, wgpu::ShaderStages::FRAGMENT),
                tex_entry(1, wgpu::TextureViewDimension::D2),
                sampler_entry(2),
                tex_entry(3, wgpu::TextureViewDimension::D2),
            ],
        });
        let viewer_pl = device.create_pipeline_layout(&wgpu::PipelineLayoutDescriptor {
            label: Some("viewer"),
            bind_group_layouts: &[Some(&viewer_layout)],
            immediate_size: 0,
        });
        let viewer_pipe = device.create_render_pipeline(&wgpu::RenderPipelineDescriptor {
            label: Some("viewer"),
            layout: Some(&viewer_pl),
            vertex: wgpu::VertexState {
                module: &viewer_mod,
                entry_point: Some("vs"),
                compilation_options: Default::default(),
                buffers: &[],
            },
            fragment: Some(wgpu::FragmentState {
                module: &viewer_mod,
                entry_point: Some("fs"),
                compilation_options: Default::default(),
                targets: &[Some(wgpu::ColorTargetState {
                    format,
                    blend: Some(wgpu::BlendState::PREMULTIPLIED_ALPHA_BLENDING),
                    write_mask: wgpu::ColorWrites::ALL,
                })],
            }),
            primitive: wgpu::PrimitiveState::default(),
            depth_stencil: None,
            multisample: Default::default(),
            multiview_mask: None,
            cache: None,
        });
        let viewer_buf = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some("viewer"),
            size: std::mem::size_of::<ViewerUniform>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let viewer_sampler = device.create_sampler(&wgpu::SamplerDescriptor {
            label: Some("viewer"),
            mag_filter: wgpu::FilterMode::Linear,
            min_filter: wgpu::FilterMode::Linear,
            ..Default::default()
        });
        let viewer_img = make_viewer_tex(&device, 1, 1, wgpu::TextureFormat::Rgba8UnormSrgb);
        let viewer_bind = make_viewer_bind(&device, &viewer_layout, &viewer_buf, &viewer_img.1, &viewer_sampler, &glyph_view);

        let (hdr, depth, bloom) = make_targets(&device, config.width, config.height);
        let ctx = PostCtx { device: &device, layout: &post_layout, sampler: &post_sampler, dummy: &dummy };
        let (bright_pass, down_passes, up_passes, composite_pass) = ctx.build(&hdr, &bloom);
        Renderer {
            surface,
            device,
            queue,
            config,
            globals_buf,
            globals_bind,
            faces_tex,
            faces_bind,
            faces_layout,
            faces_sampler,
            preview_tex,
            preview_view,
            face_capacity,
            max_face_layers,
            tower_fx: 1,
            theme: 0,
            box_vb,
            inst_buf,
            floor_pipe,
            tower_back,
            tower_front,
            bright_pipe,
            down_pipe,
            up_pipe,
            composite_pipe,
            post_layout,
            post_sampler,
            dummy,
            hdr,
            depth,
            bloom,
            bright_pass,
            down_passes,
            up_passes,
            composite_pass,
            hud_layout,
            hud_pipe,
            hud: None,
            viewer_layout,
            viewer_pipe,
            viewer_buf,
            viewer_sampler,
            viewer_img,
            glyph_view,
            viewer_bind,
        }
    }

    /// The character strip the data-mosaic effect draws with (R8, one row of glyphs).
    pub fn set_glyph_atlas(&mut self, w: u32, h: u32, px: &[u8]) {
        let (tex, view, _, _) = make_viewer_tex(&self.device, w, h, wgpu::TextureFormat::R8Unorm);
        self.queue.write_texture(
            tex.as_image_copy(),
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
        self.glyph_view = view;
        self.rebind_viewer();
        self.rebind_faces();
    }

    fn rebind_faces(&mut self) {
        self.faces_bind = bind_faces(
            &self.device,
            &self.faces_layout,
            &self.faces_sampler,
            &self.faces_tex,
            &self.preview_view,
            &self.glyph_view,
        );
    }

    /// The viewer's picture (RGBA): an image, or the current video frame.
    pub fn viewer_image(&mut self, w: u32, h: u32, px: &[u8]) {
        if (self.viewer_img.2, self.viewer_img.3) != (w, h) {
            self.viewer_img = make_viewer_tex(&self.device, w, h, wgpu::TextureFormat::Rgba8UnormSrgb);
            self.rebind_viewer();
        }
        self.queue.write_texture(
            self.viewer_img.0.as_image_copy(),
            px,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    fn rebind_viewer(&mut self) {
        self.viewer_bind = make_viewer_bind(
            &self.device,
            &self.viewer_layout,
            &self.viewer_buf,
            &self.viewer_img.1,
            &self.viewer_sampler,
            &self.glyph_view,
        );
    }

    /// Replaces the HUD overlay image (RGBA8, premultiplied, sRGB, window-sized).
    pub fn upload_hud(&mut self, w: u32, h: u32, rgba: &[u8]) {
        if w == 0 || h == 0 {
            return;
        }
        if self.hud.as_ref().map(|t| (t.w, t.h)) != Some((w, h)) {
            let tex = self.device.create_texture(&wgpu::TextureDescriptor {
                label: Some("hud"),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format: wgpu::TextureFormat::Rgba8UnormSrgb,
                usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
                view_formats: &[],
            });
            let view = tex.create_view(&Default::default());
            let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: Some("hud"),
                layout: &self.hud_layout,
                entries: &[wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) }],
            });
            self.hud = Some(HudTex { tex, bind, w, h });
        }
        let t = self.hud.as_ref().unwrap();
        self.queue.write_texture(
            wgpu::TexelCopyTextureInfo {
                texture: &t.tex,
                mip_level: 0,
                origin: wgpu::Origin3d::ZERO,
                aspect: wgpu::TextureAspect::All,
            },
            rgba,
            wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(h) },
            wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        );
    }

    pub fn size(&self) -> (u32, u32) {
        (self.config.width, self.config.height)
    }

    pub fn resize(&mut self, w: u32, h: u32) {
        if w == 0 || h == 0 {
            return;
        }
        self.config.width = w;
        self.config.height = h;
        self.surface.configure(&self.device, &self.config);
        let (hdr, depth, bloom) = make_targets(&self.device, w, h);
        self.hdr = hdr;
        self.depth = depth;
        self.bloom = bloom;
        self.rebuild_post();
    }

    fn rebuild_post(&mut self) {
        let ctx = PostCtx { device: &self.device, layout: &self.post_layout, sampler: &self.post_sampler, dummy: &self.dummy };
        let (b, d, u, c) = ctx.build(&self.hdr, &self.bloom);
        self.bright_pass = b;
        self.down_passes = d;
        self.up_passes = u;
        self.composite_pass = c;
    }

    /// Makes room for `layers` face pages. Grows only (and drops what was there), so
    /// call it before uploading a new city's pages.
    pub fn reserve_face_layers(&mut self, layers: u32) {
        let want = layers.min(self.max_face_layers);
        if want <= self.face_capacity {
            return;
        }
        let cap = want.next_multiple_of(64).min(self.max_face_layers);
        self.faces_tex = make_faces(&self.device, cap);
        self.rebind_faces();
        self.face_capacity = cap;
        log::info!("face pages: {cap} layers ({} MB)", cap as u64 * TEX_W as u64 * TEX_H as u64 * 4 / 3 / 1_048_576);
    }

    /// Uploads a baked face texture (full mip chain) into an array layer.
    /// Uploads a preview picture (RGBA, full mip chain) into its layer.
    pub fn upload_preview(&self, layer: u32, pv: &crate::preview::Preview) {
        for (level, (off, w, rows)) in pv.levels().enumerate() {
            let len = (w * rows * 4) as usize;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.preview_tex,
                    mip_level: level as u32,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                &pv.mips[off..off + len],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w * 4), rows_per_image: Some(rows) },
                wgpu::Extent3d { width: w, height: rows, depth_or_array_layers: 1 },
            );
        }
    }

    pub fn upload_face(&self, layer: u32, mips: &[u8]) {
        let mut off = 0usize;
        for level in 0..MIP_LEVELS {
            let (w, h) = mip_size(level);
            let len = (w * h) as usize;
            self.queue.write_texture(
                wgpu::TexelCopyTextureInfo {
                    texture: &self.faces_tex,
                    mip_level: level,
                    origin: wgpu::Origin3d { x: 0, y: 0, z: layer },
                    aspect: wgpu::TextureAspect::All,
                },
                &mips[off..off + len],
                wgpu::TexelCopyBufferLayout { offset: 0, bytes_per_row: Some(w), rows_per_image: Some(h) },
                wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
            );
            off += len;
        }
    }

    /// Draws a frame. Returns false if the surface needs to be skipped this frame.
    pub fn render(
        &mut self,
        view_proj: Mat4,
        cam: Vec3,
        time: f32,
        city: Option<&City>,
        hl: Option<Highlight>,
        viewer: Option<ViewerDraw>,
    ) {
        let frame = match self.surface.get_current_texture() {
            wgpu::CurrentSurfaceTexture::Success(f) | wgpu::CurrentSurfaceTexture::Suboptimal(f) => f,
            wgpu::CurrentSurfaceTexture::Outdated | wgpu::CurrentSurfaceTexture::Lost => {
                self.surface.configure(&self.device, &self.config);
                return;
            }
            _ => return,
        };
        let out_view = frame.texture.create_view(&Default::default());
        let th = &crate::theme::THEMES[self.theme % crate::theme::THEMES.len()];

        // Globals
        let (origin, radius) = city
            .map(|c| (c.origin, c.half_extent.max_element() + 30.0))
            .unwrap_or((glam::Vec2::ZERO, 100.0));
        let globals = Globals {
            view_proj: view_proj.to_cols_array_2d(),
            cam: [cam.x, cam.y, cam.z, time],
            city: [origin.x, origin.y, crate::city::CELL, radius],
            haze: [th.haze[0], th.haze[1], th.haze[2], 0.0],
            screen: [self.config.width as f32, self.config.height as f32, 0.0, 0.0],
            hl: [hl.map_or(0.0, |h| h.layer as f32), hl.map_or(0.0, |_| 1.0), self.tower_fx as f32, 0.0],
            hl_rect: hl.map_or([0.0; 4], |h| h.rect),
            th0: [th.glass_alpha, th.edge_gain, th.text_gain, th.kind_tint],
            th1: [th.floor_trace[0], th.floor_trace[1], th.floor_trace[2], th.floor_gain],
            th2: [th.floor_lane[0], th.floor_lane[1], th.floor_lane[2], 0.0],
            th3: [th.edge[0], th.edge[1], th.edge[2], th.edge_mix],
        };
        self.queue.write_buffer(&self.globals_buf, 0, bytemuck::bytes_of(&globals));
        self.queue.write_buffer(
            &self.composite_pass.params,
            0,
            bytemuck::bytes_of(&PostParams {
                texel: [1.0 / self.hdr.w as f32, 1.0 / self.hdr.h as f32, th.bloom, time],
            }),
        );

        // Towers back to front (glass needs it), only those with a texture.
        let mut instances: Vec<Instance> = Vec::new();
        if let Some(c) = city {
            let mut order: Vec<usize> = (0..c.towers.len()).filter(|&i| c.towers[i].layer.is_some() && c.towers[i].shown > 0.01).collect();
            let dist = |i: usize| (c.towers[i].base + glam::Vec3::Y * c.towers[i].size.y * 0.5).distance_squared(cam);
            order.sort_by(|&a, &b| dist(b).total_cmp(&dist(a)));
            instances = order.iter().map(|&i| instance_of(&c.towers[i], th)).collect();
            if !instances.is_empty() {
                self.queue.write_buffer(&self.inst_buf, 0, bytemuck::cast_slice(&instances));
            }
        }

        let mut enc = self.device.create_command_encoder(&wgpu::CommandEncoderDescriptor { label: Some("frame") });
        {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("scene"),
                color_attachments: &[Some(color_att(&self.hdr.view, Some(wgpu::Color::BLACK)))],
                depth_stencil_attachment: Some(wgpu::RenderPassDepthStencilAttachment {
                    view: &self.depth,
                    depth_ops: Some(wgpu::Operations { load: wgpu::LoadOp::Clear(1.0), store: wgpu::StoreOp::Store }),
                    stencil_ops: None,
                }),
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_bind_group(0, &self.globals_bind, &[]);
            pass.set_pipeline(&self.floor_pipe);
            pass.draw(0..6, 0..1);

            if !instances.is_empty() {
                pass.set_bind_group(1, &self.faces_bind, &[]);
                pass.set_vertex_buffer(0, self.box_vb.slice(..));
                pass.set_vertex_buffer(1, self.inst_buf.slice(..));
                for k in 0..instances.len() as u32 {
                    pass.set_pipeline(&self.tower_back);
                    pass.draw(0..36, k..k + 1);
                    pass.set_pipeline(&self.tower_front);
                    pass.draw(0..36, k..k + 1);
                }
            }
        }

        // Bloom: bright pass, down the chain, back up adding each level.
        self.post(&mut enc, &self.bright_pipe, &self.bright_pass, &self.bloom[0].view, true);
        for i in 1..self.bloom.len() {
            self.post(&mut enc, &self.down_pipe, &self.down_passes[i - 1], &self.bloom[i].view, true);
        }
        for i in (1..self.bloom.len()).rev() {
            self.post(&mut enc, &self.up_pipe, &self.up_passes[i - 1], &self.bloom[i - 1].view, false);
        }
        self.post(&mut enc, &self.composite_pipe, &self.composite_pass, &out_view, true);
        if let Some(vd) = viewer {
            let u = ViewerUniform {
                rect: vd.rect,
                screen: [
                    self.config.width as f32,
                    self.config.height as f32,
                    self.viewer_img.2 as f32,
                    self.viewer_img.3 as f32,
                ],
                p: [vd.effect as f32, vd.time, vd.open, 0.0],
            };
            self.queue.write_buffer(&self.viewer_buf, 0, bytemuck::bytes_of(&u));
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("viewer"),
                color_attachments: &[Some(color_att(&out_view, None))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.viewer_pipe);
            pass.set_bind_group(0, &self.viewer_bind, &[]);
            pass.draw(0..3, 0..1);
        }
        if let Some(hud) = &self.hud {
            let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
                label: Some("hud"),
                color_attachments: &[Some(color_att(&out_view, None))],
                depth_stencil_attachment: None,
                timestamp_writes: None,
                occlusion_query_set: None,
                multiview_mask: None,
            });
            pass.set_pipeline(&self.hud_pipe);
            pass.set_bind_group(0, &hud.bind, &[]);
            pass.draw(0..3, 0..1);
        }

        self.queue.submit(Some(enc.finish()));
        self.queue.present(frame);
    }

    fn post(&self, enc: &mut wgpu::CommandEncoder, pipe: &wgpu::RenderPipeline, pp: &PostPass, target: &wgpu::TextureView, clear: bool) {
        let mut pass = enc.begin_render_pass(&wgpu::RenderPassDescriptor {
            label: Some("post"),
            color_attachments: &[Some(color_att(target, if clear { Some(wgpu::Color::BLACK) } else { None }))],
            depth_stencil_attachment: None,
            timestamp_writes: None,
            occlusion_query_set: None,
            multiview_mask: None,
        });
        pass.set_pipeline(pipe);
        pass.set_bind_group(0, &pp.bind, &[]);
        pass.draw(0..3, 0..1);
    }
}

const FACE_LAYERS_MIN: u32 = 256;

fn make_viewer_tex(
    device: &wgpu::Device,
    w: u32,
    h: u32,
    format: wgpu::TextureFormat,
) -> (wgpu::Texture, wgpu::TextureView, u32, u32) {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("viewer"),
        size: wgpu::Extent3d { width: w.max(1), height: h.max(1), depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    let view = tex.create_view(&Default::default());
    (tex, view, w, h)
}

fn make_viewer_bind(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    buf: &wgpu::Buffer,
    img: &wgpu::TextureView,
    sampler: &wgpu::Sampler,
    glyphs: &wgpu::TextureView,
) -> wgpu::BindGroup {
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("viewer"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: buf.as_entire_binding() },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(img) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(glyphs) },
        ],
    })
}

fn make_faces(device: &wgpu::Device, layers: u32) -> wgpu::Texture {
    let tex = device.create_texture(&wgpu::TextureDescriptor {
        label: Some("faces"),
        size: wgpu::Extent3d { width: TEX_W, height: TEX_H, depth_or_array_layers: layers.max(2) },
        mip_level_count: MIP_LEVELS,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format: wgpu::TextureFormat::R8Unorm,
        usage: wgpu::TextureUsages::TEXTURE_BINDING | wgpu::TextureUsages::COPY_DST,
        view_formats: &[],
    });
    tex
}

fn bind_faces(
    device: &wgpu::Device,
    layout: &wgpu::BindGroupLayout,
    sampler: &wgpu::Sampler,
    faces: &wgpu::Texture,
    previews: &wgpu::TextureView,
    glyphs: &wgpu::TextureView,
) -> wgpu::BindGroup {
    let view = faces.create_view(&wgpu::TextureViewDescriptor {
        dimension: Some(wgpu::TextureViewDimension::D2Array),
        ..Default::default()
    });
    device.create_bind_group(&wgpu::BindGroupDescriptor {
        label: Some("faces"),
        layout,
        entries: &[
            wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&view) },
            wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(sampler) },
            wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::TextureView(previews) },
            wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(glyphs) },
        ],
    })
}

fn instance_of(t: &crate::city::Tower, th: &crate::theme::Theme) -> Instance {
    // Text colour by kind (HDR multiplier applied in the shader); bodies are deep tints.
    let (text, body) = match t.kind {
        Kind::Folder => ([0.28, 0.91, 1.0], [0.055, 0.165, 0.415]),
        Kind::Text => ([0.35, 1.0, 0.72], [0.03, 0.18, 0.20]),
        Kind::Media => ([1.0, 0.45, 0.95], [0.25, 0.05, 0.30]),
        Kind::Binary => ([1.0, 0.55, 0.22], [0.30, 0.10, 0.04]),
        Kind::Other => ([0.55, 0.62, 1.0], [0.08, 0.10, 0.35]),
    };
    // Theme: how much colour comes from the file type, and how lit this tower is.
    let towards = |a: [f32; 3], b: [f32; 3], k: f32| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
    let lit = crate::theme::lit(t.seed, th.lit_var);
    let text = towards(text, th.text, 1.0 - th.kind_tint).map(|c| c * lit);
    let body = towards(body, th.body, 1.0 - th.kind_tint);
    // Selected tower: text and glass shift to amber.
    let k = t.select.clamp(0.0, 1.0);
    let mix = |a: [f32; 3], b: [f32; 3], k: f32| [a[0] + (b[0] - a[0]) * k, a[1] + (b[1] - a[1]) * k, a[2] + (b[2] - a[2]) * k];
    let text = mix(text, [1.0, 0.68, 0.24], k * 0.85);
    let body = mix(body, [0.32, 0.17, 0.03], k * 0.8);
    // Search: matches burn white-green, everything else fades back.
    let text = mix(text, [0.8, 1.0, 0.62], t.found * 0.8);
    let lift = (1.0 - 0.8 * t.dim) * (1.0 + 0.6 * t.found);
    let text = text.map(|c| c * lift);
    let body = body.map(|c| c * (1.0 - 0.6 * t.dim));
    // The hidden garbage file: blood red, whatever the theme.
    let (text, body) = if t.secret { ([1.0, 0.12, 0.08].map(|c| c * 1.5), [0.16, 0.0, 0.02]) } else { (text, body) };
    let ease = t.appear * t.appear * (3.0 - 2.0 * t.appear);
    // Filtered out: sinks into the floor and fades.
    let shown = t.shown.clamp(0.0, 1.0);
    let ease = ease * (shown * shown * (3.0 - 2.0 * shown));
    Instance {
        base: t.base.to_array(),
        size: [t.size.x, t.size.y * (0.35 + 0.65 * ease) * shown.max(0.02), t.size.z], // rises as it appears
        text_col: [text[0], text[1], text[2], ease],
        body_col: [body[0], body[1], body[2], t.hover],
        misc: [t.layer.unwrap_or(0) as f32, t.seed, 1.0, t.pages.max(1) as f32],
        pv: match (t.pv_layer, t.pv_size) {
            (Some(l), Some((w, h))) => [l as f32, w as f32 / PV as f32, h as f32 / PV as f32, t.pv_video as u8 as f32 + t.pv_cut],
            _ => [-1.0, 0.0, 0.0, 0.0],
        },
        pv_rect: match t.pv_size {
            Some((w, h)) => {
                let (x0, y0, rw, rh) = crate::preview::page_rect(w, h);
                [x0 / TEX_W as f32, y0 / TEX_H as f32, (x0 + rw) / TEX_W as f32, (y0 + rh) / TEX_H as f32]
            }
            None => [0.0; 4],
        },
    }
}

fn make_targets(device: &wgpu::Device, w: u32, h: u32) -> (Target, wgpu::TextureView, Vec<Target>) {
    let tex = |label: &str, w: u32, h: u32, format: wgpu::TextureFormat, usage: wgpu::TextureUsages| {
        device
            .create_texture(&wgpu::TextureDescriptor {
                label: Some(label),
                size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
                mip_level_count: 1,
                sample_count: 1,
                dimension: wgpu::TextureDimension::D2,
                format,
                usage,
                view_formats: &[],
            })
            .create_view(&Default::default())
    };
    let rt = wgpu::TextureUsages::RENDER_ATTACHMENT | wgpu::TextureUsages::TEXTURE_BINDING;
    let hdr = Target { view: tex("hdr", w, h, HDR, rt), w, h };
    let depth = tex("depth", w, h, DEPTH, wgpu::TextureUsages::RENDER_ATTACHMENT);
    let mut bloom = Vec::new();
    let (mut bw, mut bh) = (w, h);
    for i in 0..BLOOM_LEVELS {
        bw = (bw / 2).max(1);
        bh = (bh / 2).max(1);
        bloom.push(Target { view: tex(&format!("bloom{i}"), bw, bh, HDR, rt), w: bw, h: bh });
    }
    (hdr, depth, bloom)
}

fn color_att(view: &wgpu::TextureView, clear: Option<wgpu::Color>) -> wgpu::RenderPassColorAttachment<'_> {
    wgpu::RenderPassColorAttachment {
        view,
        depth_slice: None,
        resolve_target: None,
        ops: wgpu::Operations {
            load: clear.map(wgpu::LoadOp::Clear).unwrap_or(wgpu::LoadOp::Load),
            store: wgpu::StoreOp::Store,
        },
    }
}

fn depth_state(write: bool) -> wgpu::DepthStencilState {
    wgpu::DepthStencilState {
        format: DEPTH,
        depth_write_enabled: Some(write),
        depth_compare: Some(wgpu::CompareFunction::LessEqual),
        stencil: Default::default(),
        bias: Default::default(),
    }
}

fn uniform_entry(binding: u32, vis: wgpu::ShaderStages) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: vis,
        ty: wgpu::BindingType::Buffer { ty: wgpu::BufferBindingType::Uniform, has_dynamic_offset: false, min_binding_size: None },
        count: None,
    }
}

fn tex_entry(binding: u32, dim: wgpu::TextureViewDimension) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Texture {
            sample_type: wgpu::TextureSampleType::Float { filterable: true },
            view_dimension: dim,
            multisampled: false,
        },
        count: None,
    }
}

fn sampler_entry(binding: u32) -> wgpu::BindGroupLayoutEntry {
    wgpu::BindGroupLayoutEntry {
        binding,
        visibility: wgpu::ShaderStages::FRAGMENT,
        ty: wgpu::BindingType::Sampler(wgpu::SamplerBindingType::Filtering),
        count: None,
    }
}

// Builds the bind groups + params for every post pass (on start and on resize).
struct PostCtx<'a> {
    device: &'a wgpu::Device,
    layout: &'a wgpu::BindGroupLayout,
    sampler: &'a wgpu::Sampler,
    dummy: &'a wgpu::TextureView,
}

impl PostCtx<'_> {
    fn pass(&self, src: &Target, z: f32, bloom: Option<&wgpu::TextureView>) -> PostPass {
        let params = self.device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("post params"),
            contents: bytemuck::bytes_of(&PostParams { texel: [1.0 / src.w as f32, 1.0 / src.h as f32, z, 0.0] }),
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
        });
        let bind = self.device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: Some("post"),
            layout: self.layout,
            entries: &[
                wgpu::BindGroupEntry { binding: 0, resource: wgpu::BindingResource::TextureView(&src.view) },
                wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::Sampler(self.sampler) },
                wgpu::BindGroupEntry { binding: 2, resource: params.as_entire_binding() },
                wgpu::BindGroupEntry { binding: 3, resource: wgpu::BindingResource::TextureView(bloom.unwrap_or(self.dummy)) },
            ],
        });
        PostPass { bind, params }
    }

    fn build(&self, hdr: &Target, bloom: &[Target]) -> (PostPass, Vec<PostPass>, Vec<PostPass>, PostPass) {
        let bright = self.pass(hdr, BLOOM_THRESHOLD, None);
        let down = (1..bloom.len()).map(|i| self.pass(&bloom[i - 1], 0.0, None)).collect();
        let up = (1..bloom.len()).map(|i| self.pass(&bloom[i], 0.0, None)).collect();
        let composite = self.pass(hdr, 0.45, Some(&bloom[0].view)); // strength: set per frame from the theme
        (bright, down, up, composite)
    }
}

/// Unit box: x,z in −0.5..0.5, y in 0..1, outward normals, CCW front faces.
fn box_vertices() -> Vec<BoxVertex> {
    let faces: [([f32; 3], [[f32; 3]; 4]); 6] = [
        ([0.0, 0.0, 1.0], [[-0.5, 0.0, 0.5], [0.5, 0.0, 0.5], [0.5, 1.0, 0.5], [-0.5, 1.0, 0.5]]),
        ([0.0, 0.0, -1.0], [[0.5, 0.0, -0.5], [-0.5, 0.0, -0.5], [-0.5, 1.0, -0.5], [0.5, 1.0, -0.5]]),
        ([1.0, 0.0, 0.0], [[0.5, 0.0, 0.5], [0.5, 0.0, -0.5], [0.5, 1.0, -0.5], [0.5, 1.0, 0.5]]),
        ([-1.0, 0.0, 0.0], [[-0.5, 0.0, -0.5], [-0.5, 0.0, 0.5], [-0.5, 1.0, 0.5], [-0.5, 1.0, -0.5]]),
        ([0.0, 1.0, 0.0], [[-0.5, 1.0, 0.5], [0.5, 1.0, 0.5], [0.5, 1.0, -0.5], [-0.5, 1.0, -0.5]]),
        ([0.0, -1.0, 0.0], [[-0.5, 0.0, -0.5], [0.5, 0.0, -0.5], [0.5, 0.0, 0.5], [-0.5, 0.0, 0.5]]),
    ];
    let mut v = Vec::with_capacity(36);
    for (n, q) in faces {
        for idx in [0, 1, 2, 0, 2, 3] {
            v.push(BoxVertex { pos: q[idx], nrm: n });
        }
    }
    v
}
