//! Mesh module providing mesh geometry types and utilities
//!
//! This module contains:
//! - `Vertex` - A single vertex with position, UV, normal, color
//! - `Mesh` - A collection of vertices forming a 3D object

use bytemuck::{Pod, Zeroable};
use std::collections::HashMap;
use wgpu::util::DeviceExt;

/// A single vertex in 3D space.
///
/// This struct is designed to be compatible with GPU memory layout.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod, Zeroable)]
pub struct Vertex {
    /// Position in 3D space (x, y, z)
    pub position: [f32; 3],
    /// UV texture coordinates (u, v)
    pub uv: [f32; 2],
    /// Normal vector for lighting calculations (x, y, z)
    pub normal: [f32; 3],
    /// Vertex color (r, g, b, a) - values from 0.0 to 1.0
    pub color: [f32; 4],
}

impl Vertex {
    /// Create a new vertex with all attributes.
    pub fn new(position: [f32; 3], uv: [f32; 2], normal: [f32; 3], color: [f32; 4]) -> Self {
        Self {
            position,
            uv,
            normal,
            color,
        }
    }

    /// Create a vertex with default UV (0,0), normal (0,1,0), and white color.
    pub fn from_position(position: [f32; 3]) -> Self {
        Self::new(position, [0.0, 0.0], [0.0, 1.0, 0.0], [1.0, 1.0, 1.0, 1.0])
    }

    /// Get the vertex attribute layout for wgpu.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        wgpu::VertexBufferLayout {
            array_stride: std::mem::size_of::<Vertex>() as u64,
            step_mode: wgpu::VertexStepMode::Vertex,
            attributes: &[
                // Position
                wgpu::VertexAttribute {
                    offset: 0,
                    shader_location: 0,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // UV
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, uv) as u64,
                    shader_location: 1,
                    format: wgpu::VertexFormat::Float32x2,
                },
                // Normal
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, normal) as u64,
                    shader_location: 2,
                    format: wgpu::VertexFormat::Float32x3,
                },
                // Color
                wgpu::VertexAttribute {
                    offset: std::mem::offset_of!(Vertex, color) as u64,
                    shader_location: 3,
                    format: wgpu::VertexFormat::Float32x4,
                },
            ],
        }
    }

    /// Calculate the normal for a triangle given three vertices.
    pub fn calculate_normal(v1: [f32; 3], v2: [f32; 3], v3: [f32; 3]) -> [f32; 3] {
        let edge1 = [v2[0] - v1[0], v2[1] - v1[1], v2[2] - v1[2]];
        let edge2 = [v3[0] - v1[0], v3[1] - v1[1], v3[2] - v1[2]];

        // Cross product for normal
        let nx = edge1[1] * edge2[2] - edge1[2] * edge2[1];
        let ny = edge1[2] * edge2[0] - edge1[0] * edge2[2];
        let nz = edge1[0] * edge2[1] - edge1[1] * edge2[0];

        // Normalize
        let len = (nx * nx + ny * ny + nz * nz).sqrt();
        if len > 0.0 {
            [nx / len, ny / len, nz / len]
        } else {
            [0.0, 1.0, 0.0]
        }
    }
}

/// A mesh representing a 3D object's geometry.
#[derive(Clone)]
pub struct Mesh {
    /// Vertices that make up the mesh
    pub vertices: Vec<Vertex>,
    /// Indices for indexed rendering (empty means non-indexed)
    pub indices: Vec<u32>,
    /// Whether to use indexed rendering
    pub is_indexed: bool,
}

impl Mesh {
    /// Create a new empty mesh.
    pub fn new() -> Self {
        Self {
            vertices: Vec::new(),
            indices: Vec::new(),
            is_indexed: false,
        }
    }

    /// Create a new mesh with pre-allocated capacity.
    pub fn with_capacity(vertex_count: usize) -> Self {
        Self {
            vertices: Vec::with_capacity(vertex_count),
            indices: Vec::new(),
            is_indexed: false,
        }
    }

    /// Add a vertex to the mesh and return its index.
    pub fn add_vertex(&mut self, vertex: Vertex) -> u32 {
        let index = self.vertices.len() as u32;
        self.vertices.push(vertex);
        index
    }

    /// Add multiple vertices at once.
    pub fn extend_vertices(&mut self, vertices: impl IntoIterator<Item = Vertex>) {
        self.vertices.extend(vertices);
    }

    /// Add an indexed triangle (three vertex indices).
    pub fn add_triangle(&mut self, a: u32, b: u32, c: u32) {
        self.indices.push(a);
        self.indices.push(b);
        self.indices.push(c);
        self.is_indexed = true;
    }

    /// Add multiple triangles.
    pub fn extend_triangles(&mut self, triangles: impl IntoIterator<Item = (u32, u32, u32)>) {
        for (a, b, c) in triangles {
            self.add_triangle(a, b, c);
        }
    }

    /// Build a simple quad (two triangles).
    pub fn build_quad(
        &mut self,
        pos1: [f32; 3],
        pos2: [f32; 3],
        pos3: [f32; 3],
        pos4: [f32; 3],
        uv1: [f32; 2],
        uv2: [f32; 2],
        uv3: [f32; 2],
        uv4: [f32; 2],
    ) {
        let v0 = self.add_vertex(Vertex::new(pos1, uv1, [0.0, 1.0, 0.0], [1.0; 4]));
        let v1 = self.add_vertex(Vertex::new(pos2, uv2, [0.0, 1.0, 0.0], [1.0; 4]));
        let v2 = self.add_vertex(Vertex::new(pos3, uv3, [0.0, 1.0, 0.0], [1.0; 4]));
        let v3 = self.add_vertex(Vertex::new(pos4, uv4, [0.0, 1.0, 0.0], [1.0; 4]));

        self.add_triangle(v0, v1, v2);
        self.add_triangle(v0, v2, v3);
    }

    /// Build a cuboid (box) with the given dimensions and pivot at center.
    pub fn build_cuboid(&mut self, width: f32, height: f32, depth: f32) {
        let w = width * 0.5;
        let h = height * 0.5;
        let d = depth * 0.5;

        // Define the 8 corners of the cuboid
        let vertices = [
            [-w, -h, -d], // 0: bottom-left-back
            [w, -h, -d],  // 1: bottom-right-back
            [w, h, -d],   // 2: top-right-back
            [-w, h, -d],  // 3: top-left-back
            [-w, -h, d],  // 4: bottom-left-front
            [w, -h, d],   // 5: bottom-right-front
            [w, h, d],    // 6: top-right-front
            [-w, h, d],   // 7: top-left-front
        ];

        // UV coordinates for each face (simplified)
        let uvs = [
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // back
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // front
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // left
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // right
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // top
            [[0.0, 1.0], [1.0, 1.0], [1.0, 0.0], [0.0, 0.0]], // bottom
        ];

        // Add all vertices
        for face in 0..6 {
            let corners = match face {
                0 => &[0, 1, 2, 3], // back
                1 => &[5, 4, 7, 6], // front
                2 => &[0, 4, 7, 3], // left
                3 => &[1, 5, 6, 2], // right
                4 => &[3, 2, 6, 7], // top
                _ => &[4, 5, 1, 0], // bottom
            };

            for (i, &corner) in corners.iter().enumerate() {
                let pos = vertices[corner];
                let uv = uvs[face][i];
                self.add_vertex(Vertex::new(pos, uv, [0.0, 1.0, 0.0], [1.0; 4]));
            }
        }

        // Add triangles for each face (two per quad)
        let faces = [
            &[0, 1, 2, 3], // back
            &[5, 4, 7, 6], // front
            &[0, 4, 7, 3], // left
            &[1, 5, 6, 2], // right
            &[3, 2, 6, 7], // top
            &[4, 5, 1, 0], // bottom
        ];

        for face_indices in faces {
            self.add_triangle(face_indices[0], face_indices[1], face_indices[2]);
            self.add_triangle(face_indices[0], face_indices[2], face_indices[3]);
        }
    }

    /// Clear all vertices and indices.
    pub fn clear(&mut self) {
        self.vertices.clear();
        self.indices.clear();
        self.is_indexed = false;
    }

    /// Get the number of vertices.
    pub fn vertex_count(&self) -> usize {
        self.vertices.len()
    }

    /// Get the number of indices (triangles * 3).
    pub fn index_count(&self) -> usize {
        self.indices.len()
    }

    /// Get the number of triangles.
    pub fn triangle_count(&self) -> usize {
        if self.is_indexed {
            self.indices.len() / 3
        } else {
            self.vertices.len() / 3
        }
    }

    /// Create a vertex buffer from this mesh.
    pub fn create_vertex_buffer(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> wgpu::Buffer {
        let data = bytemuck::cast_slice(&self.vertices);
        device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("Vertex Buffer"),
            contents: data.as_ref(),
            usage: wgpu::BufferUsages::VERTEX,
        })
    }

    /// Create an index buffer from this mesh (if indexed).
    pub fn create_index_buffer(
        &self,
        device: &wgpu::Device,
        _queue: &wgpu::Queue,
    ) -> Option<wgpu::Buffer> {
        if self.is_indexed && !self.indices.is_empty() {
            let data = bytemuck::cast_slice(&self.indices);
            Some(
                device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
                    label: Some("Index Buffer"),
                    contents: data.as_ref(),
                    usage: wgpu::BufferUsages::INDEX,
                }),
            )
        } else {
            None
        }
    }

    /// Get the vertex buffer layout.
    pub fn vertex_layout() -> wgpu::VertexBufferLayout<'static> {
        Vertex::vertex_layout()
    }
}

impl Default for Mesh {
    fn default() -> Self {
        Self::new()
    }
}

/// A collection of meshes organized by name or type.
#[derive(Default)]
pub struct MeshCollection {
    meshes: HashMap<String, Mesh>,
}

impl MeshCollection {
    /// Create a new empty mesh collection.
    pub fn new() -> Self {
        Self {
            meshes: HashMap::new(),
        }
    }

    /// Add or update a mesh in the collection.
    pub fn insert(&mut self, name: impl Into<String>, mesh: Mesh) {
        self.meshes.insert(name.into(), mesh);
    }

    /// Get a reference to a mesh by name.
    pub fn get(&self, name: &str) -> Option<&Mesh> {
        self.meshes.get(name)
    }

    /// Get a mutable reference to a mesh by name.
    pub fn get_mut(&mut self, name: &str) -> Option<&mut Mesh> {
        self.meshes.get_mut(name)
    }

    /// Remove and return a mesh from the collection.
    pub fn remove(&mut self, name: &str) -> Option<Mesh> {
        self.meshes.remove(name)
    }

    /// Check if the collection contains a mesh with the given name.
    pub fn contains(&self, name: &str) -> bool {
        self.meshes.contains_key(name)
    }

    /// Get all mesh names.
    pub fn keys(&self) -> impl Iterator<Item = &str> {
        self.meshes.keys().map(|s| s.as_str())
    }

    /// Clear all meshes from the collection.
    pub fn clear(&mut self) {
        self.meshes.clear();
    }

    /// Get the number of meshes in the collection.
    pub fn len(&self) -> usize {
        self.meshes.len()
    }

    /// Check if the collection is empty.
    pub fn is_empty(&self) -> bool {
        self.meshes.is_empty()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_vertex_creation() {
        let vertex = Vertex::new([1.0, 2.0, 3.0], [0.5, 0.5], [0.0, 1.0, 0.0], [1.0; 4]);
        assert_eq!(vertex.position, [1.0, 2.0, 3.0]);
        assert_eq!(vertex.uv, [0.5, 0.5]);
    }

    #[test]
    fn test_mesh_quad() {
        let mut mesh = Mesh::new();
        mesh.build_quad(
            [-1.0, -1.0, 0.0],
            [1.0, -1.0, 0.0],
            [1.0, 1.0, 0.0],
            [-1.0, 1.0, 0.0],
            [0.0, 0.0],
            [1.0, 0.0],
            [1.0, 1.0],
            [0.0, 1.0],
        );

        assert_eq!(mesh.vertex_count(), 4);
        assert_eq!(mesh.index_count(), 6);
        assert_eq!(mesh.triangle_count(), 2);
    }

    #[test]
    fn test_mesh_cuboid() {
        let mut mesh = Mesh::new();
        mesh.build_cuboid(1.0, 1.0, 1.0);

        // A cuboid has 6 faces * 4 vertices each = 24 vertices
        assert_eq!(mesh.vertex_count(), 24);
        // 6 faces * 2 triangles * 3 indices = 36 indices
        assert_eq!(mesh.index_count(), 36);
        // 12 triangles total
        assert_eq!(mesh.triangle_count(), 12);
    }

    #[test]
    fn test_mesh_collection() {
        let mut collection = MeshCollection::new();
        let mesh = Mesh::new();
        collection.insert("test", mesh);

        assert!(collection.contains("test"));
        assert_eq!(collection.len(), 1);

        let retrieved = collection.get("test");
        assert!(retrieved.is_some());
    }
}
