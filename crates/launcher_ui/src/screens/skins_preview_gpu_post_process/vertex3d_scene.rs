use vertex_3d::{FrameGraph, FrameGraphPass, FrameGraphUsage, RenderTargetType};

#[derive(Debug, Clone)]
pub(super) enum Vertex3dSceneOp {
    SceneDraw {
        batch_index: usize,
        clear_depth: bool,
    },
    DepthResolve,
    Accumulate {
        batch_index: usize,
        clear_target: bool,
    },
}

#[derive(Debug, Clone)]
pub(super) struct Vertex3dScenePlan {
    pub(super) frame_graph: vertex_3d::FrameGraphPlan,
    pub(super) operations: Vec<Vertex3dSceneOp>,
}

impl Vertex3dScenePlan {
    pub(super) fn build(batch_count: usize, scene_msaa_samples: u32) -> Self {
        let mut graph = FrameGraph::new();
        let mut operations = Vec::new();

        for batch_index in 0..batch_count {
            // Scene pass writes to both multisampled and resolved depth if MSAA enabled
            let mut scene_pass = FrameGraphPass::new(format!("scene_{batch_index}"))
                .writes(FrameGraphUsage::new(
                    "scene_color",
                    RenderTargetType::Lighting,
                ))
                .writes(FrameGraphUsage::new(
                    "scene_depth_linear",
                    RenderTargetType::Lighting,
                ))
                .writes(FrameGraphUsage::new("scene_depth", RenderTargetType::Depth));

            if scene_msaa_samples > 1 {
                scene_pass = scene_pass
                    .writes(FrameGraphUsage::new(
                        "scene_depth_resolve",
                        RenderTargetType::Depth,
                    ))
                    .writes(FrameGraphUsage::new(
                        "scene_depth_linear_msaa",
                        RenderTargetType::Lighting,
                    ));
            }

            graph = graph.with_pass(scene_pass);
            operations.push(Vertex3dSceneOp::SceneDraw {
                batch_index,
                clear_depth: true,
            });

            // Depth resolve pass when MSAA is enabled
            if scene_msaa_samples > 1 {
                graph = graph.with_pass(
                    FrameGraphPass::new(format!("depth_resolve_{batch_index}"))
                        .reads("scene_depth")
                        .writes(FrameGraphUsage::new(
                            "scene_depth_resolve",
                            RenderTargetType::Depth,
                        )),
                );
                operations.push(Vertex3dSceneOp::DepthResolve);
            }

            graph = graph.with_pass(
                FrameGraphPass::new(format!("accumulate_{batch_index}"))
                    .reads("scene_color")
                    .writes(FrameGraphUsage::new(
                        "accumulation",
                        RenderTargetType::Lighting,
                    )),
            );
            operations.push(Vertex3dSceneOp::Accumulate {
                batch_index,
                clear_target: batch_index == 0,
            });
        }

        Self {
            frame_graph: graph.plan(),
            operations,
        }
    }
}
