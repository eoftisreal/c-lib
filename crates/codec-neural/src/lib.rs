//! Real-time neural frame interpolation orchestration (`RIFE IFNet` and `GMFSS`),
//! zero-cost bitstream scene-cut gating (`BitstreamSceneCutDetector`), and 1/8-pel
//! motion-vector warmstart prior construction (`build_mv_warmstart_prior`).

use codec_av1::frame::{ExtendedFrameHeader, ExtendedFrameType, MotionVector};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum NeuralModelKind {
    RifeV4TensorRt,
    GmfssAnime,
}

#[derive(Debug, Clone, Default)]
pub struct BitstreamSceneCutDetector {
    pub intra_ratio_threshold: f32,
}

impl BitstreamSceneCutDetector {
    pub fn new(intra_ratio_threshold: f32) -> Self {
        Self {
            intra_ratio_threshold: intra_ratio_threshold.clamp(0.1, 1.0),
        }
    }

    pub fn is_scene_cut(&self, header: &ExtendedFrameHeader, intra_block_ratio: f32) -> bool {
        if matches!(
            header.frame_type,
            ExtendedFrameType::Key | ExtendedFrameType::IntraOnly | ExtendedFrameType::Switch
        ) {
            return true;
        }
        intra_block_ratio >= self.intra_ratio_threshold
    }
}

/// Converts 1/8-pel AV1 motion vectors into normalized floating-point optical-flow
/// priors `(flow_x_px, flow_y_px)` for RIFE/GMFSS IFNet warmstart.
pub fn build_mv_warmstart_prior(mvs: &[MotionVector]) -> Vec<(f32, f32)> {
    mvs.iter()
        .map(|mv| (f32::from(mv.col) * 0.125, f32::from(mv.row) * 0.125))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn scene_cut_detection_and_mv_prior() {
        let det = BitstreamSceneCutDetector::new(0.75);
        let key_hdr = ExtendedFrameHeader {
            show_existing_frame: false,
            frame_to_show_map_idx: None,
            frame_type: ExtendedFrameType::Key,
            show_frame: true,
            showable_frame: false,
            error_resilient_mode: true,
            disable_cdf_update: false,
            frame_id: 0,
            order_hint: 0,
            refresh_frame_flags: 0xff,
        };
        assert!(det.is_scene_cut(&key_hdr, 0.0));
        let flow = build_mv_warmstart_prior(&[MotionVector { row: -8, col: 16 }]);
        assert_eq!(flow, vec![(2.0, -1.0)]);
    }
}
