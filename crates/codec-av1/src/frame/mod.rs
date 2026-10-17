//! AV1 intra/inter-frame syntax, spatial/temporal reconstruction, transforms,
//! normative in-loop filtering, and transactional parser state.
pub mod block_decode;
mod block_syntax_tables;
pub mod cdef;
pub mod cfl;
pub mod coeff_decode;
pub mod default_cdfs;
pub mod dimensions;
mod filter_intra_tables;
pub mod filters;
pub mod grain;
pub mod header;
pub mod inter;
pub mod intra;
pub mod inverse;
pub mod lifecycle;
mod lossy_tables;
mod palette;
mod palette_tables;
pub mod partition;
pub mod partition_decode;
mod partition_tables;
pub mod picture;
mod quant_tables;
pub mod quantization;
pub mod reconstruction;
pub mod residual;
pub mod restoration;
pub mod scan;
mod scan_tables;
mod smooth_tables;
pub mod state;
pub mod superres;
mod superres_tables;
pub mod tile;
pub mod transform;

pub use block_decode::{decode_intra_frame, IntraBlockDecoder, TileCdfs};
pub use cfl::{predict_cfl_block, CflBlock};
pub use header::{parse_uncompressed_header, FrameHeader, FrameType, TxMode};
pub use inter::{
    ExtendedFrameHeader, ExtendedFrameType, GlobalMotionModel, InterRefFrame, InterpFilter,
    MotionVector,
};
pub use intra::{FilterIntraMode, IntraMode};
pub use partition::{BlockSize, CodingLeafBlock, PartitionType};
pub use state::{DecoderState, ParserState, ReferenceMetadata};
pub use transform::{TxSize, TxType1D, TxType2D};

#[cfg(test)]
mod tests {
    use super::*;
    use crate::{
        obu::{parse_obu_header, validate_sample_with_state},
        ObuType,
    };
    use codec_bitstream::BitReader;
    fn bits(fields: &[(u32, u8)]) -> (Vec<u8>, usize) {
        let mut bytes = Vec::new();
        let mut pos = 0;
        for &(value, n) in fields {
            for shift in (0..n).rev() {
                if pos % 8 == 0 {
                    bytes.push(0);
                }
                let last = bytes.len() - 1;
                bytes[last] |= (((value >> shift) & 1) as u8) << (7 - pos % 8);
                pos += 1;
            }
        }
        (bytes, pos)
    }
    fn sequence() -> crate::SequenceHeader {
        let mut r = BitReader::new(include_bytes!("../../../../tests/intra_tiles.obu"));
        loop {
            let h = parse_obu_header(&mut r).unwrap();
            let n = r.read_leb128().unwrap() as usize;
            let payload = r.read_bytes(n).unwrap();
            if h.obu_type == ObuType::SequenceHeader {
                return crate::parse_sequence_header(&mut BitReader::new(payload)).unwrap();
            }
        }
    }
    #[test]
    fn intra_only_quantization_segmentation_deltas_and_filters() {
        let mut s = sequence();
        s.seq_force_screen_content_tools = 0;
        s.order_hint_bits = 0;
        s.enable_superres = false;
        s.enable_cdef = false;
        s.enable_restoration = false;
        s.color_config.mono_chrome = true;
        s.film_grain_params_present = false;
        let mut f = vec![
            (0, 1),
            (2, 2),
            (1, 1),
            (0, 1),
            (1, 1), // existing,type,show,resilient,disable_cdf
            (0, 1),
            (1, 8),
            (0, 1), // size override, refresh, render
            (1, 1),
            (0, 1),
            (0, 1), // uniform tiles, cols, rows
            (10, 8),
            (1, 1),
            (125, 7),
            (0, 1),
            (1, 1),
        ]; // q, Ydc=-3, no matrix, segmentation
        for segment in 0..8 {
            for feature in 0..8 {
                let present = segment == 0 && feature == 0;
                f.push((u32::from(present), 1));
                if present {
                    f.push((502, 9));
                } // signed -10
            }
        }
        f.extend([
            (1, 1),
            (2, 2),
            (1, 1),
            (1, 2),
            (1, 1), // delta q and lf
            (3, 6),
            (4, 6),
            (2, 3),
            (1, 1),
            (1, 1),
            (1, 1),
            (126, 7),
        ]); // levels/sharpness/update/ref0=-2
        f.extend([(0, 1); 7]);
        f.extend([(0, 1), (1, 1), (63, 7), (0, 1), (1, 1)]); // mode1=63, tx largest, reduced
        let (bytes, n) = bits(&f);
        let mut r = BitReader::new(&bytes);
        let h = parse_uncompressed_header(&mut r, &s).unwrap();
        assert_eq!(h.header_bits, n);
        assert_eq!(r.bits_left(), bytes.len() * 8 - n);
        assert_eq!(h.frame_type, FrameType::IntraOnly);
        assert_eq!(h.refresh_frame_flags, 1);
        assert_eq!(h.quantization.deltas, [-3, 0, 0, 0, 0]);
        assert_eq!(h.segmentation.features[0][0], Some(-10));
        assert_eq!(h.delta.q_res, Some(2));
        assert_eq!(h.delta.lf_res, Some(1));
        assert!(h.delta.lf_multi);
        assert_eq!(h.loop_filter.levels, [3, 4, 0, 0]);
        assert_eq!(h.loop_filter.ref_deltas[0], -2);
        assert_eq!(h.loop_filter.mode_deltas, [0, 63]);
        assert_eq!(h.tx_mode, TxMode::Largest);
        for cut in 0..bytes.len() {
            assert!(parse_uncompressed_header(&mut BitReader::new(&bytes[..cut]), &s).is_err());
        }
    }
    #[test]
    fn nonuniform_tile_layout_and_superresolution_render_size() {
        let mut s = sequence();
        s.enable_superres = true;
        let (bytes, _) = bits(&[(1, 1), (7, 3), (1, 1), (639, 16), (359, 16)]);
        let d = dimensions::Dimensions::parse(&mut BitReader::new(&bytes), &s, false).unwrap();
        assert_eq!((d.upscaled_width, d.width, d.height), (256, 128, 128));
        assert_eq!((d.render_width, d.render_height), (640, 360));
        // Two 1-SB columns and a single 2-SB row, explicit spacing, context tile 1.
        let (bytes, _) = bits(&[(0, 1), (0, 1), (1, 1), (1, 1), (0, 2)]);
        let tiles = tile::TileInfo::parse(&mut BitReader::new(&bytes), &d, false).unwrap();
        assert_eq!(tiles.col_starts, vec![0, 16, 32]);
        assert_eq!(tiles.row_starts, vec![0, 32]);
        assert_eq!(tiles.context_update_tile_id, 1);
        assert_eq!(tiles.tile_size_bytes, 1);
        assert!(tiles.group(&[0, 255], 0, false).is_err()); // size exceeds group
    }
    #[test]
    fn cdef_secondary_remapping_and_chroma_restoration_units() {
        let (bytes, _) = bits(&[(0, 2), (0, 2), (15, 4), (3, 2), (1, 4), (3, 2)]);
        let c = filters::Cdef::parse(&mut BitReader::new(&bytes), false, false).unwrap();
        assert_eq!(c.strengths, vec![[15, 4, 1, 4]]);
        let mut s = sequence();
        s.use_128x128_superblock = false;
        let (bytes, _) = bits(&[(2, 2), (3, 2), (0, 2), (1, 1), (1, 1), (1, 1)]);
        let r = filters::Restoration::parse(&mut BitReader::new(&bytes), &s, false).unwrap();
        assert_eq!(r.types, [2, 3, 0]);
        assert_eq!(r.unit_sizes, Some([256, 128, 128]));
    }
    #[test]
    fn reference_id_wraparound_and_order_hint_invalidation() {
        let mut state = DecoderState::default();
        validate_sample_with_state(
            include_bytes!("../../../../tests/intra_tiles.obu"),
            &mut state,
        )
        .unwrap();
        state.sequence.as_mut().unwrap().frame_id_length = 5;
        state.sequence.as_mut().unwrap().delta_frame_id_length = 2;
        let mut h = state.last_header.clone().unwrap();
        h.frame_type = FrameType::IntraOnly;
        h.refresh_frame_flags = 1;
        h.frame_id = 1;
        for (slot, id) in state
            .references
            .iter_mut()
            .zip([31, 30, 29, 1, 2, 0, 16, 28])
        {
            slot.as_mut().unwrap().frame_id = id;
        }
        state.begin(&h);
        assert_eq!(
            state
                .references
                .iter()
                .map(Option::is_some)
                .collect::<Vec<_>>(),
            vec![true, true, true, true, false, true, false, false]
        );
        state.complete(h.clone());
        assert_eq!(state.references[0].as_ref().unwrap().frame_id, 1);
        h.ref_order_hints = Some([1; 8]);
        h.refresh_frame_flags = 0;
        state.begin(&h);
        assert!(state.references.iter().all(Option::is_none));
    }
}
