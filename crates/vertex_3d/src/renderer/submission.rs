//! GPU residency and queued scene submission infrastructure.
//!
//! This layer keeps frequently used resources resident on the GPU and batches
//! per-frame scene data into rotating upload buffers so the CPU can keep
//! building future work without stalling after every draw setup step.

use std::{borrow::Cow, collections::BTreeMap, mem};

use bytemuck::{Pod, Zeroable};
use wgpu::util::DeviceExt;

use crate::{
    ShaderHandle,
    asset::{MeshHandle, RenderAssetLibrary, TextureHandle},
    material::MaterialHandle,
    scene::{DrawPacket, Scene},
};

/// Resident GPU mesh buffers ready for repeated draws.
#[derive(Debug)]
pub struct GpuMesh {
    pub vertex_buffer: wgpu::Buffer,
    pub index_buffer: Option<wgpu::Buffer>,
    pub vertex_count: u32,
    pub index_count: u32,
}

impl GpuMesh {
    pub fn from_mesh(device: &wgpu::Device, mesh: &crate::Mesh, label: &str) -> Self {
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some(&format!("{label}-vertex")),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = if mesh.is_indexed && !mesh.indices.is_empty() {
            Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some(&format!("{label}-index")),
                    contents: bytemuck::cast_slice(&mesh.indices),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            )
        } else {
            None
        };

        Self {
            vertex_buffer,
            index_buffer,
            vertex_count: mesh.vertex_count() as u32,
            index_count: mesh.index_count() as u32,
        }
    }
}

/// Resident GPU texture resources.
#[derive(Debug)]
pub struct GpuTexture {
    pub texture: wgpu::Texture,
    pub view: wgpu::TextureView,
    pub sampler: wgpu::Sampler,
    pub size: [u32; 2],
    pub format: wgpu::TextureFormat,
}

impl GpuTexture {
    pub fn new(
        texture: wgpu::Texture,
        view: wgpu::TextureView,
        sampler: wgpu::Sampler,
        size: [u32; 2],
        format: wgpu::TextureFormat,
    ) -> Self {
        Self {
            texture,
            view,
            sampler,
            size,
            format,
        }
    }
}

/// Registry of resident GPU resources keyed by public asset handles.
#[derive(Debug, Default)]
pub struct GpuResourceRegistry {
    meshes: BTreeMap<MeshHandle, GpuMesh>,
    textures: BTreeMap<TextureHandle, GpuTexture>,
}

impl GpuResourceRegistry {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn ensure_mesh(
        &mut self,
        device: &wgpu::Device,
        assets: &RenderAssetLibrary,
        handle: MeshHandle,
    ) -> Result<&GpuMesh, SubmissionError> {
        if !self.meshes.contains_key(&handle) {
            let mesh = assets.mesh(handle).ok_or(SubmissionError::MissingMesh {
                handle_id: handle.id(),
            })?;
            let gpu_mesh = GpuMesh::from_mesh(device, &mesh.mesh, &mesh.label);
            self.meshes.insert(handle, gpu_mesh);
        }
        Ok(self.meshes.get(&handle).expect("mesh inserted"))
    }

    pub fn insert_texture(&mut self, handle: TextureHandle, texture: GpuTexture) {
        self.textures.insert(handle, texture);
    }

    pub fn mesh(&self, handle: MeshHandle) -> Option<&GpuMesh> {
        self.meshes.get(&handle)
    }

    pub fn texture(&self, handle: TextureHandle) -> Option<&GpuTexture> {
        self.textures.get(&handle)
    }
}

/// One rotating upload buffer used for a single frame-in-flight slot.
#[derive(Debug)]
struct FrameUploadSlot {
    buffer: wgpu::Buffer,
    capacity: u64,
    cursor: u64,
}

impl FrameUploadSlot {
    fn new(device: &wgpu::Device, label: &str, capacity: u64, usage: wgpu::BufferUsages) -> Self {
        let buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: capacity.max(256),
            usage,
            mapped_at_creation: false,
        });
        Self {
            buffer,
            capacity: capacity.max(256),
            cursor: 0,
        }
    }

    fn reset(&mut self) {
        self.cursor = 0;
    }

    fn ensure_capacity(
        &mut self,
        device: &wgpu::Device,
        label: &str,
        usage: wgpu::BufferUsages,
        required: u64,
    ) {
        if required <= self.capacity {
            return;
        }
        let next_capacity = required.next_power_of_two().max(self.capacity * 2);
        self.buffer = device.create_buffer(&wgpu::BufferDescriptor {
            label: Some(label),
            size: next_capacity,
            usage,
            mapped_at_creation: false,
        });
        self.capacity = next_capacity;
        self.cursor = 0;
    }
}

/// Host-managed upload ring for frame-local scene buffers.
#[derive(Debug)]
pub struct FrameUploadArena {
    label: Cow<'static, str>,
    usage: wgpu::BufferUsages,
    alignment: u64,
    slots: Vec<FrameUploadSlot>,
    frame_index: usize,
}

impl FrameUploadArena {
    pub fn new(
        device: &wgpu::Device,
        label: impl Into<Cow<'static, str>>,
        frames_in_flight: usize,
        initial_capacity: u64,
        usage: wgpu::BufferUsages,
        alignment: u64,
    ) -> Self {
        let label = label.into();
        let frames_in_flight = frames_in_flight.max(1);
        let mut slots = Vec::with_capacity(frames_in_flight);
        for slot_index in 0..frames_in_flight {
            slots.push(FrameUploadSlot::new(
                device,
                &format!("{label}-{slot_index}"),
                initial_capacity,
                usage,
            ));
        }
        Self {
            label,
            usage,
            alignment: alignment.max(1),
            slots,
            frame_index: 0,
        }
    }

    pub fn begin_frame(&mut self) {
        self.frame_index = (self.frame_index + 1) % self.slots.len();
        self.slots[self.frame_index].reset();
    }

    pub fn upload<T: Pod>(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        data: &[T],
    ) -> UploadAllocation {
        let size = mem::size_of_val(data) as u64;
        let aligned = align_up(size, self.alignment);
        let label = format!("{}-{}", self.label, self.frame_index);
        let slot = &mut self.slots[self.frame_index];
        let offset = align_up(slot.cursor, self.alignment);
        let required = offset + aligned;
        slot.ensure_capacity(device, &label, self.usage, required);
        queue.write_buffer(&slot.buffer, offset, bytemuck::cast_slice(data));
        slot.cursor = required;

        UploadAllocation {
            offset,
            size,
            buffer: slot.buffer.clone(),
        }
    }
}

fn align_up(value: u64, alignment: u64) -> u64 {
    if alignment <= 1 {
        value
    } else {
        value.div_ceil(alignment) * alignment
    }
}

/// One uploaded slice inside the current frame-local upload buffer.
#[derive(Debug, Clone)]
pub struct UploadAllocation {
    pub buffer: wgpu::Buffer,
    pub offset: u64,
    pub size: u64,
}

/// Instance data consumed by queued scene submissions.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod, Zeroable)]
pub struct GpuInstanceData {
    pub world_from_local: [[f32; 4]; 4],
    pub material_id: u32,
    pub _padding: [u32; 3],
}

impl GpuInstanceData {
    pub fn from_draw_packet(packet: &DrawPacket) -> Self {
        Self {
            world_from_local: packet.transform.to_cols_array_2d(),
            material_id: packet.material.id() as u32,
            _padding: [0; 3],
        }
    }

    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        const ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![
            8 => Float32x4,
            9 => Float32x4,
            10 => Float32x4,
            11 => Float32x4,
            12 => Uint32
        ];
        wgpu::VertexBufferLayout {
            array_stride: mem::size_of::<GpuInstanceData>() as u64,
            step_mode: wgpu::VertexStepMode::Instance,
            attributes: &ATTRIBUTES,
        }
    }
}

/// A batch of instances sharing the same shader, material, and mesh.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DrawBatch {
    pub shader: ShaderHandle,
    pub material: MaterialHandle,
    pub mesh: MeshHandle,
    pub instance_range: std::ops::Range<u32>,
}

/// Uploaded scene submission for one frame.
#[derive(Debug, Clone)]
pub struct QueuedSceneSubmission {
    pub instance_upload: UploadAllocation,
    pub instance_count: u32,
    pub batches: Vec<DrawBatch>,
}

impl QueuedSceneSubmission {
    pub fn encode<'pass>(
        &'pass self,
        pass: &mut wgpu::RenderPass<'pass>,
        resources: &'pass GpuResourceRegistry,
    ) -> Result<(), SubmissionError> {
        pass.set_vertex_buffer(
            1,
            self.instance_upload.buffer.slice(
                self.instance_upload.offset
                    ..self.instance_upload.offset + self.instance_upload.size,
            ),
        );
        for batch in &self.batches {
            let mesh = resources
                .mesh(batch.mesh)
                .ok_or(SubmissionError::MissingResidentMesh {
                    handle_id: batch.mesh.id(),
                })?;
            pass.set_vertex_buffer(0, mesh.vertex_buffer.slice(..));
            if let Some(index_buffer) = &mesh.index_buffer {
                pass.set_index_buffer(index_buffer.slice(..), wgpu::IndexFormat::Uint32);
                pass.draw_indexed(0..mesh.index_count, 0, batch.instance_range.clone());
            } else {
                pass.draw(0..mesh.vertex_count, batch.instance_range.clone());
            }
        }
        Ok(())
    }
}

/// High-level queued submission path for instanced scenes.
#[derive(Debug)]
pub struct SceneSubmissionQueue {
    instance_uploads: FrameUploadArena,
}

impl SceneSubmissionQueue {
    pub fn new(
        device: &wgpu::Device,
        frames_in_flight: usize,
        initial_instance_capacity: u64,
    ) -> Self {
        Self {
            instance_uploads: FrameUploadArena::new(
                device,
                "vertex3d-instance-upload",
                frames_in_flight,
                initial_instance_capacity,
                wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                wgpu::COPY_BUFFER_ALIGNMENT,
            ),
        }
    }

    pub fn begin_frame(&mut self) {
        self.instance_uploads.begin_frame();
    }

    pub fn queue_scene(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        assets: &RenderAssetLibrary,
        resources: &mut GpuResourceRegistry,
        scene: &Scene,
    ) -> Result<QueuedSceneSubmission, SubmissionError> {
        let draw_packets = scene.build_draw_list(assets);
        let mut instances = Vec::with_capacity(draw_packets.len());
        let mut batches = Vec::new();

        let mut current_key: Option<(ShaderHandle, MaterialHandle, MeshHandle)> = None;
        let mut instance_start = 0u32;

        for packet in &draw_packets {
            resources.ensure_mesh(device, assets, packet.mesh)?;
            instances.push(GpuInstanceData::from_draw_packet(packet));

            let key = (packet.shader, packet.material, packet.mesh);
            match current_key {
                Some(existing) if existing == key => {}
                Some((shader, material, mesh)) => {
                    batches.push(DrawBatch {
                        shader,
                        material,
                        mesh,
                        instance_range: instance_start..instances.len() as u32 - 1,
                    });
                    current_key = Some(key);
                    instance_start = instances.len() as u32 - 1;
                }
                None => {
                    current_key = Some(key);
                    instance_start = instances.len() as u32 - 1;
                }
            }
        }

        if let Some((shader, material, mesh)) = current_key {
            batches.push(DrawBatch {
                shader,
                material,
                mesh,
                instance_range: instance_start..instances.len() as u32,
            });
        }

        let instance_upload = self.instance_uploads.upload(device, queue, &instances);
        Ok(QueuedSceneSubmission {
            instance_upload,
            instance_count: instances.len() as u32,
            batches,
        })
    }
}

/// Errors raised while turning a scene into queued GPU work.
#[derive(Debug, thiserror::Error)]
pub enum SubmissionError {
    #[error("scene references mesh handle {handle_id}, but no such mesh asset exists")]
    MissingMesh { handle_id: u64 },

    #[error("queued scene references mesh handle {handle_id}, but no resident mesh exists")]
    MissingResidentMesh { handle_id: u64 },
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        Mesh,
        asset::{RenderAssetLibrary, ShaderAsset},
        material::Material,
        renderer::FrameGraph,
        shader::{
            CompiledShaderProgram, ReflectedStage, ReflectionSnapshot, ShaderBackendTarget,
            ShaderKind, ShaderProgram, ShaderSourceLanguage,
        },
    };

    #[test]
    fn batches_group_by_shader_material_and_mesh() {
        let mut assets = RenderAssetLibrary::new();
        let shader = assets.insert_shader(ShaderAsset {
            label: "shader".to_string(),
            source_language: ShaderSourceLanguage::Slang,
            compiled: CompiledShaderProgram {
                program: ShaderProgram::with_name("shader"),
                reflection: ReflectionSnapshot {
                    stages: vec![ReflectedStage::new(ShaderKind::Vertex, "vs_main")],
                    resources: Vec::new(),
                    render_targets: Vec::new(),
                },
                target: ShaderBackendTarget::Wgsl,
                stages: BTreeMap::new(),
            },
            frame_graph: FrameGraph::new(),
            pass_templates: Vec::new(),
        });
        let material = assets.insert_material(Material::unlit("mat", shader));
        let mesh = assets.insert_mesh("mesh", Mesh::new());

        let scene = Scene::new()
            .add(crate::RenderObject::new(mesh, material))
            .add(crate::RenderObject::new(mesh, material));
        let packets = scene.build_draw_list(&assets);

        assert_eq!(packets.len(), 2);
        let mut batches = Vec::new();
        let mut current_key: Option<(ShaderHandle, MaterialHandle, MeshHandle)> = None;
        let mut instance_start = 0u32;
        for (index, packet) in packets.iter().enumerate() {
            let key = (packet.shader, packet.material, packet.mesh);
            match current_key {
                Some(existing) if existing == key => {}
                Some((shader, material, mesh)) => {
                    batches.push(DrawBatch {
                        shader,
                        material,
                        mesh,
                        instance_range: instance_start..index as u32,
                    });
                    current_key = Some(key);
                    instance_start = index as u32;
                }
                None => {
                    current_key = Some(key);
                    instance_start = index as u32;
                }
            }
        }
        if let Some((shader, material, mesh)) = current_key {
            batches.push(DrawBatch {
                shader,
                material,
                mesh,
                instance_range: instance_start..packets.len() as u32,
            });
        }

        assert_eq!(batches.len(), 1);
        assert_eq!(batches[0].instance_range, 0..2);
    }

    #[test]
    fn instance_layout_leaves_slot_zero_for_mesh_vertices() {
        let layout = GpuInstanceData::vertex_layout();
        assert_eq!(layout.step_mode, wgpu::VertexStepMode::Instance);
        assert_eq!(layout.attributes[0].shader_location, 8);
    }
}
