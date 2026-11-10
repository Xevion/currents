mod debug;
mod key;
mod noise;
mod pass;
mod resolve;
mod timing;

pub use debug::{FieldView, UnknownView};
use key::{Key, Keys};
pub(crate) use pass::draw_fullscreen;
use pass::{Inputs, Pass, SHARED_BINDINGS};
use resolve::{History, Resolve};
pub use timing::PassTimes;
use timing::Timing;

use crate::gpu::linear_sampler;
use crate::params::ParamBlock;
use crate::shaders::Sources;

#[repr(C)]
#[derive(Clone, Copy, PartialEq, bytemuck::Pod, bytemuck::Zeroable)]
pub struct Uniforms {
    /// Shade pixels along each axis.
    resolution: [f32; 2],
    time: f32,
    /// Shade pixels per field texel along each axis.
    field_scale: f32,
    view: [f32; 4],
    /// Shade pixels per output pixel along each axis.
    supersampling: f32,
    /// The key the field pass reads, filled in by the renderer: its time, the texels across it
    /// (zero when none is held) and the part of the cover it spans.
    key_time: f32,
    key_size: [f32; 2],
    key_view: [f32; 4],
    /// What blending with the frame before needs, filled in by the renderer: that frame's
    /// view, this frame's shift within the pixel, the time between them, and how many frames
    /// have been blended so far (0 when there is none to build on).
    prev_view: [f32; 4],
    jitter: [f32; 2],
    elapsed: f32,
    history: f32,
    params: ParamBlock,
}

impl Uniforms {
    /// Uniforms for a picture of `output` pixels showing `view` at `time`.
    ///
    /// The shade pass runs `supersampling` times finer than the output along each axis and the
    /// field pass `field_step` times coarser; both are at least 1. A `field_step` of 1 keeps
    /// thin filaments of the stream exact, and each step up costs the field pass about a
    /// quarter of the one below.
    pub fn new(
        output: [u32; 2],
        supersampling: u32,
        field_step: u32,
        time: f32,
        view: [f32; 4],
        params: ParamBlock,
    ) -> Self {
        let supersampling = supersampling.max(1);
        Self {
            resolution: output.map(|v| (v * supersampling) as f32),
            time,
            field_scale: (supersampling * field_step.max(1)) as f32,
            view,
            supersampling: supersampling as f32,
            key_time: 0.0,
            key_size: [0.0; 2],
            key_view: [0.0; 4],
            prev_view: [0.0; 4],
            jitter: [0.0; 2],
            elapsed: 0.0,
            history: 0.0,
            params,
        }
    }

    /// Whether `before` is a frame this one can be blended with: the same picture size and
    /// tuning, a little earlier. The view may have moved.
    fn follows(&self, before: &Self) -> bool {
        let shape = |u: &Self| Self { time: 0.0, view: [0.0; 4], ..*u };
        shape(self) == shape(before) && (0.0..MAX_BLEND_GAP).contains(&(self.time - before.time))
    }

    /// Whether this frame shows the flow `before` showed, no earlier in time. The view and the
    /// picture's size may have changed: keys span the cover, not the screen.
    fn continues(&self, before: &Self) -> bool {
        self.params == before.params && self.time >= before.time
    }

    fn with_key(mut self, key: &Key) -> Self {
        self.key_time = key.time;
        self.key_size = key.size.map(|v| v as f32);
        self.key_view = key.view;
        self
    }

    /// Uniforms for drawing `key` itself: its span of the cover as the view, a pixel a texel,
    /// at its time, reading `from` when it is carried forward from that key.
    fn for_key(&self, key: &Key, from: Option<&Key>) -> Self {
        let own = Self {
            resolution: key.size.map(|v| v as f32),
            time: key.time,
            field_scale: 1.0,
            view: key.view,
            supersampling: 1.0,
            key_time: 0.0,
            key_size: [0.0; 2],
            key_view: [0.0; 4],
            prev_view: [0.0; 4],
            jitter: [0.0; 2],
            elapsed: 0.0,
            history: 0.0,
            params: self.params,
        };
        from.map_or(own, |from| own.with_key(from))
    }
}

pub(crate) const FIELD_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba32Float;
/// Field texels between the points where the way back to the key is worked out; the field pass
/// blends between them, which the hop map's smoothness over a few texels allows.
const HOP_STEP: u32 = 4;
const HOP_CONSTANTS: [(&str, f64); 1] = [("hop_step", HOP_STEP as f64)];
const COLOR_FORMAT: wgpu::TextureFormat = wgpu::TextureFormat::Rgba8Unorm;
/// Longest stretch of flow between two frames that are still blended; past it the liquid has
/// moved too far for the earlier frame to say anything about this one.
const MAX_BLEND_GAP: f32 = 0.25;
/// Past this many frames a blend has settled, and the count stops mattering.
const MAX_BLENDED: u32 = 64;

/// Shade resolution in pixels and the supersampling factor; the field texture is derived from both.
#[derive(Clone, Copy, PartialEq, Eq)]
struct TargetSize {
    width: u32,
    height: u32,
    field_scale: u32,
}

struct Targets {
    size: TargetSize,
    /// The hop map: the field's size over `HOP_STEP`, and a border for the blend's footprint.
    hop_view: wgpu::TextureView,
    field_view: wgpu::TextureView,
    color: wgpu::Texture,
    color_view: wgpu::TextureView,
    shade_bind: wgpu::BindGroup,
    history: History,
}

/// A texture keys are rendered into, at least as large as the key it holds.
struct KeyTarget {
    size: [u32; 2],
    view: wgpu::TextureView,
}

struct KeyBinds {
    key: wgpu::BindGroup,
    field: wgpu::BindGroup,
}

/// Two fullscreen passes: `field` writes the stripe coordinate, `shade` turns it into colour.
/// Both read their gradient noise from a tile baked when the renderer is made.
///
/// Tracing the liquid's history costs far more than a frame can spend, so while frames show
/// the same view moving forward in time the renderer keeps a key: that history at a fixed
/// time, read by every frame of a stretch of flow. Only the first key is traced; each one
/// after is the key before it carried forward, a few stripes a frame while that one serves.
/// Keys span part of the cover, so they go on serving while the view moves.
pub struct Renderer {
    ubuf: wgpu::Buffer,
    noise: wgpu::TextureView,
    noise_sampler: wgpu::Sampler,
    key_pass: Pass,
    /// The key pass's own uniforms: it draws a key as a picture of its own.
    key_ubuf: wgpu::Buffer,
    key_targets: [KeyTarget; 2],
    /// The hop pass, run before the field pass of every frame that reads a key.
    hop: Pass,
    hop_bind: wgpu::BindGroup,
    /// Bindings that read each key texture: the key pass's, to carry that key forward into the
    /// other texture, and the field pass's, with the hop map. Unset while a texture they name
    /// has been replaced.
    key_binds: Option<[KeyBinds; 2]>,
    keys: Keys,
    /// The last frame rendered, to tell whether this one continues it.
    last: Option<Uniforms>,
    field: Pass,
    shade: Pass,
    /// One pipeline per `FieldView`, in its order, bound like the shade pass.
    debug: Vec<wgpu::RenderPipeline>,
    targets: Option<Targets>,
    resolve: Resolve,
    /// Whether frames are blended with the ones before them.
    temporal: bool,
    /// Frames blended so far without a break, the last of which the next can build on.
    blended: u32,
    /// Frames drawn while blending, which picks each one's shading points.
    frame: u32,
    sampler: wgpu::Sampler,
    timing: Option<Timing>,
}

/// The shade pass with the `sample_field` the device can run appended. The two are separate
/// sources because even a constant branch between them changes the driver's codegen.
fn shade_source(sources: &Sources, filterable: bool) -> String {
    let sampler =
        if filterable { include_str!("sample_filtered.wgsl") } else { include_str!("sample_unfiltered.wgsl") };
    sources.assemble(&format!("{}\n{sampler}", sources.shade))
}

/// The bindings every pass starts with: the uniforms, the noise tile and its sampler.
fn shared_entries<'a>(
    ubuf: &'a wgpu::Buffer,
    noise: &'a wgpu::TextureView,
    noise_sampler: &'a wgpu::Sampler,
) -> [wgpu::BindGroupEntry<'a>; 3] {
    [
        wgpu::BindGroupEntry { binding: 0, resource: ubuf.as_entire_binding() },
        wgpu::BindGroupEntry { binding: 1, resource: wgpu::BindingResource::TextureView(noise) },
        wgpu::BindGroupEntry { binding: 2, resource: wgpu::BindingResource::Sampler(noise_sampler) },
    ]
}

/// A key texture of `size` texels.
fn key_target(device: &wgpu::Device, size: [u32; 2]) -> KeyTarget {
    let view = make_tex(device, size[0], size[1], key::FORMAT).create_view(&Default::default());
    KeyTarget { size, view }
}

fn make_tex(device: &wgpu::Device, w: u32, h: u32, format: wgpu::TextureFormat) -> wgpu::Texture {
    device.create_texture(&wgpu::TextureDescriptor {
        label: None,
        size: wgpu::Extent3d { width: w, height: h, depth_or_array_layers: 1 },
        mip_level_count: 1,
        sample_count: 1,
        dimension: wgpu::TextureDimension::D2,
        format,
        usage: wgpu::TextureUsages::RENDER_ATTACHMENT
            | wgpu::TextureUsages::COPY_SRC
            | wgpu::TextureUsages::TEXTURE_BINDING,
        view_formats: &[],
    })
}

impl Renderer {
    /// Without `FLOAT32_FILTERABLE` (common on WebGL and phones) the shade pass filters the field itself.
    pub fn new(device: &wgpu::Device, queue: &wgpu::Queue, sources: &Sources) -> Self {
        let filterable = device.features().contains(wgpu::Features::FLOAT32_FILTERABLE);
        let ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let field_source = sources.assemble(&sources.field);
        let key_pass = Pass::new(device, "key", field_source.clone(), "fs_key", Inputs::loaded(1), &[], key::FORMAT);
        let hop = Pass::new(device, "hop", field_source.clone(), "fs_hop", Inputs::NONE, &HOP_CONSTANTS, FIELD_FORMAT);
        let field =
            Pass::new(device, "field", field_source, "fs_main", Inputs::loaded(2), &HOP_CONSTANTS, FIELD_FORMAT);
        let shade = Pass::new(
            device,
            "shade",
            shade_source(sources, filterable),
            "fs_main",
            Inputs { count: 1, filterable, sampler: true },
            &[],
            COLOR_FORMAT,
        );
        let debug_source = sources.assemble(include_str!("debug.wgsl"));
        let debug = FieldView::ALL
            .map(|view| {
                let constants = [("mode", f64::from(view.mode()))];
                pass::pipeline(
                    device,
                    view.name(),
                    debug_source.clone(),
                    "fs_main",
                    &shade.layout,
                    &constants,
                    COLOR_FORMAT,
                )
            })
            .to_vec();
        let sampler = if filterable {
            linear_sampler(device)
        } else {
            device.create_sampler(&wgpu::SamplerDescriptor::default())
        };
        let noise = noise::bake(device, queue, sources);
        let noise_sampler = noise::sampler(device);
        let key_ubuf = device.create_buffer(&wgpu::BufferDescriptor {
            label: None,
            size: size_of::<Uniforms>() as u64,
            usage: wgpu::BufferUsages::UNIFORM | wgpu::BufferUsages::COPY_DST,
            mapped_at_creation: false,
        });
        let hop_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &hop.layout,
            entries: &shared_entries(&ubuf, &noise, &noise_sampler),
        });
        let key_targets = [(); 2].map(|()| key_target(device, [1, 1]));
        Self {
            ubuf,
            noise,
            noise_sampler,
            key_pass,
            key_ubuf,
            key_targets,
            hop,
            hop_bind,
            key_binds: None,
            keys: Keys::default(),
            last: None,
            field,
            shade,
            debug,
            targets: None,
            resolve: Resolve::new(device, sources),
            temporal: true,
            blended: 0,
            frame: 0,
            sampler,
            timing: Timing::new(device, queue),
        }
    }

    /// Call after submitting the encoder passed to `render`: starts reading back its timestamps.
    pub fn after_submit(&mut self) {
        if let Some(t) = self.timing.as_mut() {
            t.start_readback();
        }
    }

    /// Milliseconds in the field and shade passes of the most recent frame read back. With
    /// `wait`, blocks until the latest submitted readback lands; otherwise never stalls.
    pub fn pass_ms(&mut self, device: &wgpu::Device, wait: bool) -> Option<PassTimes> {
        self.timing.as_mut()?.latest(device, wait)
    }

    /// Decides which key, if any, this frame reads, and returns the stripes of a key to render
    /// before it. A frame gets a key only when it continues the last one: a still would pay
    /// for a key and use it once.
    fn update_keys(
        &mut self,
        device: &wgpu::Device,
        uniforms: &Uniforms,
        field_texels: [u32; 2],
    ) -> Option<key::Slice> {
        let before = self.last.filter(|before| uniforms.continues(before));
        self.last = Some(*uniforms);
        let Some(before) = before else {
            self.keys.clear();
            return None;
        };
        let dt = uniforms.time - before.time;
        let drift = std::array::from_fn(|i| uniforms.view[i] - before.view[i]);
        let slice = self.keys.advance(uniforms.time, dt, uniforms.view, drift, field_texels)?;
        if slice.is_heavy() {
            // a large share of a key in one frame is a visible hitch
            tracing::debug!(
                time = uniforms.time,
                dt,
                key_time = slice.key.time,
                span = slice.key.span,
                stripes = ?slice.stripes,
                "rendering much of a key at once"
            );
        }
        if slice.stripes.start == 0 {
            let [width, height] = slice.key.size;
            let held = self.key_targets[slice.slot].size;
            let moving = drift.iter().any(|&edge| edge != 0.0);
            // a moving view asks for a slightly different size each time, so its keys get
            // room to spare and keep their texture; one at rest gets exactly its own
            let fits = held[0] >= width && held[1] >= height;
            let loose = u64::from(held[0]) * u64::from(held[1]) > u64::from(width) * u64::from(height) * 5 / 4;
            if !fits || (loose && !moving) {
                let spare = |texels: u32| if moving { texels + texels / 8 } else { texels };
                self.key_targets[slice.slot] = key_target(device, [spare(width), spare(height)]);
                self.key_binds = None;
            }
        }
        Some(slice)
    }

    /// Returns whether the targets were recreated, which invalidates views taken from the old ones.
    fn ensure_targets(&mut self, device: &wgpu::Device, size: TargetSize) -> bool {
        if self.targets.as_ref().is_some_and(|t| t.size == size) {
            return false;
        }
        let TargetSize { width, height, field_scale } = size;
        let field = [width.div_ceil(field_scale), height.div_ceil(field_scale)];
        let field_view = make_tex(device, field[0], field[1], FIELD_FORMAT).create_view(&Default::default());
        // a texel either side of the field for the blend, and one more where the division rounds
        let hop = field.map(|texels| texels.div_ceil(HOP_STEP) + 4);
        let hop_view = make_tex(device, hop[0], hop[1], FIELD_FORMAT).create_view(&Default::default());
        self.key_binds = None;
        let color = make_tex(device, width, height, COLOR_FORMAT);
        let color_view = color.create_view(&Default::default());
        let [uniforms, noise, noise_sampler] = shared_entries(&self.ubuf, &self.noise, &self.noise_sampler);
        let shade_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
            label: None,
            layout: &self.shade.layout,
            entries: &[
                uniforms,
                noise,
                noise_sampler,
                wgpu::BindGroupEntry {
                    binding: SHARED_BINDINGS,
                    resource: wgpu::BindingResource::TextureView(&field_view),
                },
                wgpu::BindGroupEntry {
                    binding: SHARED_BINDINGS + 1,
                    resource: wgpu::BindingResource::Sampler(&self.sampler),
                },
            ],
        });
        let history = self.resolve.history(device, &self.ubuf, &field_view, width, height);
        self.blended = 0;
        self.targets = Some(Targets { size, hop_view, field_view, color, color_view, shade_bind, history });
        true
    }

    /// Turns blending each frame with the ones before it on or off. It stands in for
    /// supersampling at the cost of one cheap pass, and is on to begin with.
    pub fn set_temporal(&mut self, temporal: bool) {
        self.temporal = temporal;
        self.blended = 0;
    }

    pub const fn temporal(&self) -> bool {
        self.temporal
    }

    /// Bindings that read each key texture, with the hop map the current targets hold.
    fn key_binds(&self, device: &wgpu::Device, hop_view: &wgpu::TextureView) -> [KeyBinds; 2] {
        std::array::from_fn(|slot| {
            let key = wgpu::BindGroupEntry {
                binding: SHARED_BINDINGS,
                resource: wgpu::BindingResource::TextureView(&self.key_targets[slot].view),
            };
            let [uniforms, noise, noise_sampler] = shared_entries(&self.key_ubuf, &self.noise, &self.noise_sampler);
            let key_bind = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.key_pass.layout,
                entries: &[uniforms, noise, noise_sampler, key.clone()],
            });
            let [uniforms, noise, noise_sampler] = shared_entries(&self.ubuf, &self.noise, &self.noise_sampler);
            let field = device.create_bind_group(&wgpu::BindGroupDescriptor {
                label: None,
                layout: &self.field.layout,
                entries: &[
                    uniforms,
                    noise,
                    noise_sampler,
                    key,
                    wgpu::BindGroupEntry {
                        binding: SHARED_BINDINGS + 1,
                        resource: wgpu::BindingResource::TextureView(hop_view),
                    },
                ],
            });
            KeyBinds { key: key_bind, field }
        })
    }

    /// Renders one frame into the internal colour texture, sized by `uniforms`; `view` replaces the shaded picture with a view of the field. Returns whether that
    /// texture was recreated, so views of `color` taken earlier are stale.
    pub fn render(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        enc: &mut wgpu::CommandEncoder,
        uniforms: &Uniforms,
        view: Option<FieldView>,
    ) -> bool {
        let size = TargetSize {
            width: uniforms.resolution[0] as u32,
            height: uniforms.resolution[1] as u32,
            field_scale: uniforms.field_scale.max(1.0) as u32,
        };
        let rebuilt = self.ensure_targets(device, size);
        let before = self.last;
        let field_texels = [size.width.div_ceil(size.field_scale), size.height.div_ceil(size.field_scale)];
        let slice = self.update_keys(device, uniforms, field_texels);
        let held = self.keys.held();
        let mut frame = held.map_or(*uniforms, |(_, key)| uniforms.with_key(&key));
        let blend = self.temporal && view.is_none();
        let built_on = before.filter(|before| blend && self.blended > 0 && uniforms.follows(before));
        if let Some(before) = built_on {
            frame.prev_view = before.view;
            frame.elapsed = uniforms.time - before.time;
            frame.history = self.blended.min(MAX_BLENDED) as f32;
            frame.jitter = resolve::jitter(self.frame);
            self.frame = self.frame.wrapping_add(1);
        }
        self.blended = if !blend {
            0
        } else if built_on.is_some() {
            self.blended.saturating_add(1)
        } else {
            1
        };
        queue.write_buffer(&self.ubuf, 0, bytemuck::bytes_of(&frame));
        if self.key_binds.is_none() {
            let hop_view = &self.targets.as_ref().expect("ensure_targets left the targets unset").hop_view;
            self.key_binds = Some(self.key_binds(device, hop_view));
        }
        let t = self.targets.as_mut().expect("ensure_targets left the targets unset");
        let key_binds = self.key_binds.as_ref().expect("key bindings were made just above");
        let stamps = |pass, first, last| self.timing.as_ref().map(|tm| tm.pass_writes(pass, first, last));
        let keyed = slice.is_some();
        if let Some(key::Slice { slot, key, from, stripes }) = slice {
            queue.write_buffer(&self.key_ubuf, 0, bytemuck::bytes_of(&uniforms.for_key(&key, from.as_ref())));
            let target = &self.key_targets[slot].view;
            let stamps = stamps(0, true, false);
            let columns = stripes.map(|stripe| key.stripe_columns(stripe));
            // the other texture is the one read, or bound unread for a key traced afresh
            let bind = &key_binds[slot ^ 1].key;
            pass::draw_columns(enc, target, &self.key_pass.pipeline, bind, key.size[1], columns, stamps);
        }
        if held.is_some() {
            let stamps = if keyed { None } else { stamps(0, true, false) };
            draw_fullscreen(enc, &t.hop_view, &self.hop.pipeline, &self.hop_bind, stamps);
        }
        let field_bind = &key_binds[held.map_or(0, |(slot, _)| slot)].field;
        let field = &self.field.pipeline;
        draw_fullscreen(enc, &t.field_view, field, field_bind, stamps(0, !keyed && held.is_none(), true));
        let colour = view.map_or(&self.shade.pipeline, |v| &self.debug[v as usize]);
        if blend {
            draw_fullscreen(enc, &t.history.shaded, colour, &t.shade_bind, stamps(1, true, false));
            self.resolve.draw(enc, &mut t.history, &t.color_view, stamps(1, false, true));
        } else {
            draw_fullscreen(enc, &t.color_view, colour, &t.shade_bind, stamps(1, true, true));
        }
        if let Some(tm) = self.timing.as_mut() {
            tm.resolve(enc);
        }
        rebuilt
    }

    /// Whether the last frame is as good as its picture gets; `false` while a key is being
    /// fitted to a view that moved, which rendering the same frame again carries on with.
    pub const fn settled(&self) -> bool {
        !self.keys.busy()
    }

    pub fn color(&self) -> &wgpu::Texture {
        &self.targets.as_ref().expect("colour target requested before the first render").color
    }

    pub fn color_view(&self) -> &wgpu::TextureView {
        &self.targets.as_ref().expect("colour target requested before the first render").color_view
    }
}

#[cfg(test)]
mod tests {
    use rstest::rstest;

    use super::*;
    use crate::shader_check::{to_webgl, validate};

    #[test]
    fn field_shader_validates_and_translates_for_webgl() {
        let sources = Sources::embedded();
        let source = sources.assemble(&sources.field);
        validate("field", &source);
        to_webgl("field", &source, &[]);
    }

    #[rstest]
    fn shade_shader_validates_and_translates_for_webgl(#[values(true, false)] filterable: bool) {
        let source = shade_source(&Sources::embedded(), filterable);
        validate("shade", &source);
        to_webgl("shade", &source, &[]);
    }

    #[test]
    fn resolve_shader_validates_and_translates_for_webgl() {
        let source = resolve::source(&Sources::embedded());
        validate("resolve", &source);
        to_webgl("resolve", &source, &[]);
    }

    #[test]
    fn noise_shader_validates_and_translates_for_webgl() {
        let source = noise::source(&Sources::embedded());
        validate("noise", &source);
        to_webgl("noise", &source, &[("texels", 512.0), ("taps", 2.0)]);
    }

    #[test]
    fn debug_views_validate_and_translate_for_webgl() {
        let source = Sources::embedded().assemble(include_str!("debug.wgsl"));
        validate("debug", &source);
        for view in FieldView::ALL {
            to_webgl(view.name(), &source, &[("mode", f64::from(view.mode()))]);
        }
    }
}
