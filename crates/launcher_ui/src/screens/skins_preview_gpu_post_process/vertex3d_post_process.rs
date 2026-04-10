use super::*;
use vertex_3d::{
    AttachmentLifecycle, FrameGraph, FrameGraphPass, FrameGraphUsage, RenderTargetType,
};

const ACCUMULATION: &str = "accumulation";
const POST_PROCESS: &str = "post_process";
const TAA_HISTORY: &str = "taa_history";
const PRESENT: &str = "present";

#[derive(Debug, Clone, Copy)]
pub(super) enum Vertex3dPostProcessPipeline {
    Ssao,
    Smaa,
    Fxaa,
    Taa,
}

#[derive(Debug, Clone)]
pub(super) enum Vertex3dPostProcessOp {
    Fullscreen {
        name: &'static str,
        pipeline: Vertex3dPostProcessPipeline,
        source: &'static str,
        history: Option<&'static str>,
        target: &'static str,
        taa_scalar: Option<f32>,
    },
    Copy {
        name: &'static str,
        source: &'static str,
        target: &'static str,
    },
    Present {
        source: &'static str,
    },
    SetTaaHistoryValid(bool),
}

#[derive(Debug, Clone)]
pub(super) struct Vertex3dPostProcessPlan {
    pub(super) frame_graph: vertex_3d::FrameGraphPlan,
    pub(super) operations: Vec<Vertex3dPostProcessOp>,
}

impl Vertex3dPostProcessPlan {
    pub(super) fn build(aa_mode: SkinPreviewAaMode, taa_history_valid: bool) -> Self {
        let mut graph = FrameGraph::new();
        let mut operations = Vec::new();

        add_fullscreen(
            &mut graph,
            &mut operations,
            "ssao",
            Vertex3dPostProcessPipeline::Ssao,
            ACCUMULATION,
            Some("scene_depth"),
            POST_PROCESS,
            None,
        );
        add_copy(
            &mut graph,
            &mut operations,
            "ssao_resolve",
            POST_PROCESS,
            ACCUMULATION,
        );

        match aa_mode {
            SkinPreviewAaMode::Smaa => {
                add_fullscreen(
                    &mut graph,
                    &mut operations,
                    "smaa",
                    Vertex3dPostProcessPipeline::Smaa,
                    ACCUMULATION,
                    None,
                    POST_PROCESS,
                    None,
                );
                add_present(&mut graph, &mut operations, POST_PROCESS);
                operations.push(Vertex3dPostProcessOp::SetTaaHistoryValid(false));
            }
            SkinPreviewAaMode::Fxaa => {
                add_fullscreen(
                    &mut graph,
                    &mut operations,
                    "fxaa",
                    Vertex3dPostProcessPipeline::Fxaa,
                    ACCUMULATION,
                    None,
                    POST_PROCESS,
                    None,
                );
                add_present(&mut graph, &mut operations, POST_PROCESS);
                operations.push(Vertex3dPostProcessOp::SetTaaHistoryValid(false));
            }
            SkinPreviewAaMode::Taa => {
                if taa_history_valid {
                    add_fullscreen(
                        &mut graph,
                        &mut operations,
                        "taa",
                        Vertex3dPostProcessPipeline::Taa,
                        ACCUMULATION,
                        Some(TAA_HISTORY),
                        POST_PROCESS,
                        Some(0.35),
                    );
                    add_copy(
                        &mut graph,
                        &mut operations,
                        "taa_history_copy",
                        POST_PROCESS,
                        TAA_HISTORY,
                    );
                    add_present(&mut graph, &mut operations, POST_PROCESS);
                } else {
                    add_copy(
                        &mut graph,
                        &mut operations,
                        "taa_history_prime",
                        ACCUMULATION,
                        TAA_HISTORY,
                    );
                    add_present(&mut graph, &mut operations, ACCUMULATION);
                }
                operations.push(Vertex3dPostProcessOp::SetTaaHistoryValid(true));
            }
            SkinPreviewAaMode::FxaaTaa => {
                if taa_history_valid {
                    add_fullscreen(
                        &mut graph,
                        &mut operations,
                        "taa",
                        Vertex3dPostProcessPipeline::Taa,
                        ACCUMULATION,
                        Some(TAA_HISTORY),
                        POST_PROCESS,
                        Some(0.22),
                    );
                    add_copy(
                        &mut graph,
                        &mut operations,
                        "taa_history_copy",
                        POST_PROCESS,
                        TAA_HISTORY,
                    );
                    add_fullscreen(
                        &mut graph,
                        &mut operations,
                        "fxaa_after_taa",
                        Vertex3dPostProcessPipeline::Fxaa,
                        POST_PROCESS,
                        None,
                        ACCUMULATION,
                        None,
                    );
                    add_present(&mut graph, &mut operations, ACCUMULATION);
                } else {
                    add_copy(
                        &mut graph,
                        &mut operations,
                        "taa_history_prime",
                        ACCUMULATION,
                        TAA_HISTORY,
                    );
                    add_fullscreen(
                        &mut graph,
                        &mut operations,
                        "fxaa",
                        Vertex3dPostProcessPipeline::Fxaa,
                        ACCUMULATION,
                        None,
                        POST_PROCESS,
                        None,
                    );
                    add_present(&mut graph, &mut operations, POST_PROCESS);
                }
                operations.push(Vertex3dPostProcessOp::SetTaaHistoryValid(true));
            }
            SkinPreviewAaMode::Msaa | SkinPreviewAaMode::Off => {
                add_present(&mut graph, &mut operations, ACCUMULATION);
                operations.push(Vertex3dPostProcessOp::SetTaaHistoryValid(false));
            }
        }

        Self {
            frame_graph: graph.plan(),
            operations,
        }
    }
}

fn add_fullscreen(
    graph: &mut FrameGraph,
    operations: &mut Vec<Vertex3dPostProcessOp>,
    name: &'static str,
    pipeline: Vertex3dPostProcessPipeline,
    source: &'static str,
    history: Option<&'static str>,
    target: &'static str,
    taa_scalar: Option<f32>,
) {
    let mut pass = FrameGraphPass::new(name)
        .reads(source)
        .writes(FrameGraphUsage::new(target, RenderTargetType::Lighting));
    if let Some(history) = history {
        pass = pass.reads(history);
    }
    *graph = std::mem::take(graph).with_pass(pass);
    operations.push(Vertex3dPostProcessOp::Fullscreen {
        name,
        pipeline,
        source,
        history,
        target,
        taa_scalar,
    });
}

fn add_copy(
    graph: &mut FrameGraph,
    operations: &mut Vec<Vertex3dPostProcessOp>,
    name: &'static str,
    source: &'static str,
    target: &'static str,
) {
    let pass = FrameGraphPass::new(name).reads(source).writes(
        FrameGraphUsage::new(target, RenderTargetType::Lighting)
            .with_lifecycle(AttachmentLifecycle::History),
    );
    *graph = std::mem::take(graph).with_pass(pass);
    operations.push(Vertex3dPostProcessOp::Copy {
        name,
        source,
        target,
    });
}

fn add_present(
    graph: &mut FrameGraph,
    operations: &mut Vec<Vertex3dPostProcessOp>,
    source: &'static str,
) {
    let pass = FrameGraphPass::new("present")
        .reads(source)
        .writes(FrameGraphUsage::new(PRESENT, RenderTargetType::Lighting));
    *graph = std::mem::take(graph).with_pass(pass);
    operations.push(Vertex3dPostProcessOp::Present { source });
}
