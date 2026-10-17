//! Declarative Media Pipeline Modeling Language (`codec-dsl`).
//! Compiles `.hook`-style stage rules and telemetry predicates into an execution DAG.

use codec_core::{CodecError, CodecResult};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum HookStage {
    Native,
    Luma,
    Chroma,
    Rgb,
    Main,
    Scaled,
    Output,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum DslOperator {
    FilmGrain { gpu: bool },
    Tonemap { curve: String },
    Deband { iterations: u32 },
    Scale { kernel: String },
    Interpolate { model: String },
    FrameMixer { kernel: String },
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DslRule {
    pub stage: HookStage,
    pub operator: DslOperator,
}

#[derive(Debug, Clone)]
pub struct FrameEvalContext {
    pub is_hdr: bool,
    pub frame_time_ms: f64,
    pub thermal_throttling: bool,
}

impl Default for FrameEvalContext {
    fn default() -> Self {
        Self {
            is_hdr: true,
            frame_time_ms: 4.0,
            thermal_throttling: false,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct PipelineExecutionPlan {
    pub ordered_stages: Vec<DslOperator>,
    pub suppress_cpu_grain: bool,
    pub neural_lookahead_frames: u32,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CompiledPipelineDag {
    pub rules: Vec<DslRule>,
}

impl CompiledPipelineDag {
    pub fn compile_str(source: &str) -> CodecResult<Self> {
        let trimmed = source.trim();
        if trimmed.is_empty() {
            return Err(CodecError::InvalidData);
        }
        let mut rules = Vec::new();
        for stmt in trimmed.split(';').map(str::trim).filter(|s| !s.is_empty()) {
            let (name, args_raw) = stmt
                .split_once('(')
                .and_then(|(n, rest)| rest.strip_suffix(')').map(|a| (n.trim(), a.trim())))
                .ok_or(CodecError::InvalidData)?;

            let parse_kv = |key: &str| -> Option<String> {
                args_raw.split(',').find_map(|pair| {
                    let (k, v) = pair.split_once(':')?;
                    if k.trim() == key {
                        Some(v.trim().to_string())
                    } else {
                        None
                    }
                })
            };

            let rule = match name {
                "film_grain" => {
                    let gpu = match parse_kv("gpu").as_deref() {
                        Some("true") | None => true,
                        Some("false") => false,
                        _ => return Err(CodecError::InvalidData),
                    };
                    DslRule {
                        stage: HookStage::Luma,
                        operator: DslOperator::FilmGrain { gpu },
                    }
                }
                "tonemap" => {
                    let curve = parse_kv("curve").unwrap_or_else(|| "bt2390".to_string());
                    if curve.is_empty() {
                        return Err(CodecError::InvalidData);
                    }
                    DslRule {
                        stage: HookStage::Rgb,
                        operator: DslOperator::Tonemap { curve },
                    }
                }
                "deband" => {
                    let iterations = parse_kv("iterations")
                        .and_then(|v| v.parse::<u32>().ok())
                        .unwrap_or(1);
                    DslRule {
                        stage: HookStage::Main,
                        operator: DslOperator::Deband { iterations },
                    }
                }
                "scale" => {
                    let kernel = parse_kv("kernel").unwrap_or_else(|| "ewa_lanczos".to_string());
                    DslRule {
                        stage: HookStage::Scaled,
                        operator: DslOperator::Scale { kernel },
                    }
                }
                "interpolate" => {
                    let model = parse_kv("model").unwrap_or_else(|| "rife_v4_tensorrt".to_string());
                    if model.is_empty() {
                        return Err(CodecError::InvalidData);
                    }
                    DslRule {
                        stage: HookStage::Output,
                        operator: DslOperator::Interpolate { model },
                    }
                }
                "frame_mixer" => {
                    let kernel = parse_kv("kernel").unwrap_or_else(|| "oversample".to_string());
                    DslRule {
                        stage: HookStage::Output,
                        operator: DslOperator::FrameMixer { kernel },
                    }
                }
                _ => return Err(CodecError::InvalidData),
            };
            rules.push(rule);
        }
        if rules.is_empty() {
            return Err(CodecError::InvalidData);
        }
        Ok(Self { rules })
    }

    pub fn evaluate(&self, ctx: &FrameEvalContext) -> PipelineExecutionPlan {
        let mut ordered_stages = Vec::new();
        let mut suppress_cpu_grain = false;
        let mut neural_lookahead_frames = 0u32;

        for rule in &self.rules {
            match &rule.operator {
                DslOperator::FilmGrain { gpu } => {
                    if *gpu {
                        suppress_cpu_grain = true;
                    }
                    ordered_stages.push(rule.operator.clone());
                }
                DslOperator::Interpolate { .. } => {
                    if !ctx.thermal_throttling && ctx.frame_time_ms <= 16.6 {
                        neural_lookahead_frames = 1;
                        ordered_stages.push(rule.operator.clone());
                    }
                }
                _ => ordered_stages.push(rule.operator.clone()),
            }
        }

        PipelineExecutionPlan {
            ordered_stages,
            suppress_cpu_grain,
            neural_lookahead_frames,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsl_compilation_and_gpu_grain_suppression() {
        let dag = CompiledPipelineDag::compile_str(
            "film_grain(gpu: true); tonemap(curve: mobius); interpolate(model: rife_v4_tensorrt)",
        )
        .unwrap();
        let plan = dag.evaluate(&FrameEvalContext::default());
        assert_eq!(plan.ordered_stages.len(), 3);
        assert!(plan.suppress_cpu_grain);
        assert_eq!(plan.neural_lookahead_frames, 1);
        assert!(CompiledPipelineDag::compile_str("").is_err());
        assert!(CompiledPipelineDag::compile_str("unknown_op(foo: bar)").is_err());
    }
}
