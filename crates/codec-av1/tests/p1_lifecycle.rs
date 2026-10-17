use codec_av1::{
    frame::{
        grain::{FilmGrainParameters, GrainContext},
        inter::{ExtendedFrameHeader, ExtendedFrameType, InterRefFrame},
        lifecycle::DecodedPictureBuffer,
        FrameHeader, FrameType,
    },
    DecoderState,
};
use codec_bitstream::BitReader;
use codec_core::{ChromaSubsampling, CodecError, PlanarFrame, Plane, Timestamp};
use std::sync::Arc;
fn fixture() -> (codec_av1::SequenceHeader, FrameHeader) {
    let mut state = DecoderState::default();
    codec_av1::obu::validate_sample_with_state(
        include_bytes!("../../../tests/intra_lossless.obu"),
        &mut state,
    )
    .unwrap();
    (
        state.sequence().unwrap().clone(),
        state.last_header().unwrap().clone(),
    )
}
fn pixels(s: &codec_av1::SequenceHeader, h: &FrameHeader, value: u16) -> PlanarFrame {
    let w = h.dimensions.width as usize;
    let ht = h.dimensions.height as usize;
    let sub = match (s.color_config.subsampling_x, s.color_config.subsampling_y) {
        (true, true) => ChromaSubsampling::Cs420,
        (true, false) => ChromaSubsampling::Cs422,
        _ => ChromaSubsampling::Cs444,
    };
    let cw = w.div_ceil(1 << usize::from(s.color_config.subsampling_x));
    let ch = ht.div_ceil(1 << usize::from(s.color_config.subsampling_y));
    let depth = if s.color_config.twelve_bit {
        12
    } else if s.color_config.high_bitdepth {
        10
    } else {
        8
    };
    PlanarFrame::new(
        depth,
        sub,
        Plane::new(w, ht, w, vec![value; w * ht]).unwrap(),
        Plane::new(cw, ch, cw, vec![value; cw * ch]).unwrap(),
        Plane::new(cw, ch, cw, vec![value; cw * ch]).unwrap(),
    )
    .unwrap()
}
fn dummy_grain() -> FilmGrainParameters {
    FilmGrainParameters {
        seed: 0,
        y_points: Vec::new(),
        cb_points: Vec::new(),
        cr_points: Vec::new(),
        chroma_scaling_from_luma: false,
        scaling_shift: 0,
        ar_coeff_lag: 0,
        ar_coeffs_y: Vec::new(),
        ar_coeffs_cb: Vec::new(),
        ar_coeffs_cr: Vec::new(),
        ar_coeff_shift: 0,
        grain_scale_shift: 0,
        cb_scaling: None,
        cr_scaling: None,
        overlap: false,
        clip_to_restricted_range: false,
    }
}
#[test]
fn eight_slots_share_real_pixels_and_preserve_display_metadata() {
    let (s, h) = fixture();
    let mut dpb = DecodedPictureBuffer::default();
    let pts = Timestamp::new(19, 1, 24).unwrap();
    let out = dpb
        .commit_intra(&s, &h, pixels(&s, &h, 73), Some(pts), 7)
        .unwrap()
        .unwrap();
    assert_eq!(out.pts, Some(pts));
    assert_eq!(out.temporal_unit, 7);
    assert_eq!(out.render_width, h.dimensions.render_width);
    for i in 0..8 {
        let r = dpb.reference(i).unwrap().unwrap();
        assert!(Arc::ptr_eq(&r.pixels, &out.pixels));
        assert_eq!(r.pixels.planes()[0].row(0).unwrap()[0], 73);
    }
    for reference in [
        InterRefFrame::Last,
        InterRefFrame::Last2,
        InterRefFrame::Last3,
        InterRefFrame::Golden,
        InterRefFrame::BwdRef,
        InterRefFrame::AltRef2,
        InterRefFrame::AltRef,
    ] {
        assert!(Arc::ptr_eq(
            &dpb.reference_for(reference, [0, 1, 2, 3, 4, 5, 6])
                .unwrap()
                .pixels,
            &out.pixels
        ));
    }
    assert!(dpb.reference(8).is_err());
    dpb.reset();
    assert!(dpb.reference(0).unwrap().is_none());
    assert_eq!(out.pixels.planes()[0].row(0).unwrap()[0], 73);
}
#[test]
fn hidden_key_show_existing_once_refreshes_all_slots() {
    let (s, mut h) = fixture();
    h.show_frame = false;
    h.showable_frame = true;
    h.refresh_frame_flags = 1;
    h.frame_id = 12;
    let mut dpb = DecodedPictureBuffer::default();
    assert!(dpb
        .commit_intra(&s, &h, pixels(&s, &h, 91), None, 0)
        .unwrap()
        .is_none());
    let saved = dpb.clone();
    assert!(dpb.show_existing(0, Some(11), None, 1).is_err());
    assert_eq!(dpb, saved);
    let out = dpb
        .show_existing(0, Some(12), Some(Timestamp::new(80, 1, 1000).unwrap()), 1)
        .unwrap();
    for i in 0..8 {
        assert!(Arc::ptr_eq(
            &dpb.reference(i).unwrap().unwrap().pixels,
            &out.pixels
        ));
    }
    assert!(dpb.show_existing(0, Some(12), None, 2).is_err());
}
#[test]
fn showable_intra_repeats_without_refreshing_other_slots() {
    let (s, mut h) = fixture();
    h.frame_type = FrameType::IntraOnly;
    h.show_frame = false;
    h.showable_frame = true;
    h.refresh_frame_flags = 2;
    let mut dpb = DecodedPictureBuffer::default();
    dpb.commit_intra(&s, &h, pixels(&s, &h, 39), None, 0)
        .unwrap();
    let saved = dpb.clone();
    for unit in 1..4 {
        let out = dpb
            .show_existing(
                1,
                None,
                Some(Timestamp::new(unit, 1, 30).unwrap()),
                unit as u64,
            )
            .unwrap();
        assert_eq!(out.temporal_unit, unit as u64);
        assert_eq!(dpb, saved);
    }
    assert!(dpb.reference(0).unwrap().is_none());
}
#[test]
fn required_filters_grain_and_budget_fail_before_refresh() {
    let (s, h) = fixture();
    let mut dpb = DecodedPictureBuffer::default();
    dpb.commit_intra(&s, &h, pixels(&s, &h, 8), None, 0)
        .unwrap();
    let saved = dpb.clone();
    let mut dpb_filtered = dpb.clone();
    for stage in 0..3 {
        let mut filtered = h.clone();
        match stage {
            0 => filtered.loop_filter.levels[0] = 1,
            1 => filtered.cdef.strengths[0][0] = 1,
            _ => filtered.restoration.types[0] = 2,
        };
        assert!(dpb_filtered
            .commit_intra(&s, &filtered, pixels(&s, &filtered, 99), None, 1)
            .is_ok());
    }
    let mut bad_grain = h.clone();
    bad_grain.film_grain = Some(dummy_grain());
    assert_eq!(
        dpb.commit_intra(&s, &bad_grain, pixels(&s, &bad_grain, 99), None, 1),
        Err(CodecError::Unsupported)
    );
    assert_eq!(dpb, saved);
    assert!(dpb
        .commit_intra(
            &s,
            &h,
            pixels(&s, &h, 99),
            Some(Timestamp {
                pts: 0,
                time_base_num: 1,
                time_base_den: 0
            }),
            1
        )
        .is_err());
    assert_eq!(dpb, saved);
    let mut small = DecodedPictureBuffer::with_budget(1);
    assert_eq!(
        small.commit_intra(&s, &h, pixels(&s, &h, 0), None, 0),
        Err(CodecError::OutOfMemory)
    );
    assert!(small.reference(0).unwrap().is_none());
}
#[test]
fn superres_commits_upscaled_pixels_and_coded_render_geometry() {
    let (s, mut h) = fixture();
    h.dimensions.width = 64;
    h.dimensions.height = 5;
    h.dimensions.upscaled_width = 128;
    h.dimensions.superres_denom = 16;
    h.dimensions.render_width = 160;
    h.dimensions.render_height = 9;
    let mut dpb = DecodedPictureBuffer::default();
    let out = dpb
        .commit_intra(&s, &h, pixels(&s, &h, 110), None, 0)
        .unwrap()
        .unwrap();
    assert_eq!(
        (
            out.pixels.width(),
            out.pixels.height(),
            out.render_width,
            out.render_height
        ),
        (128, 5, 160, 9)
    );
    assert!(out.pixels.planes()[0]
        .row(2)
        .unwrap()
        .iter()
        .all(|&v| v == 110));
}
#[test]
fn real_film_grain_header_matches_ffmpeg_trace_every_field() {
    let mut state = DecoderState::default();
    codec_av1::obu::validate_sample_with_state(
        include_bytes!("../../../tests/intra_film_grain.obu"),
        &mut state,
    )
    .unwrap();
    let h = state.last_header().unwrap();
    assert_eq!(h.header_bits, 963);
    let g = h.film_grain.as_ref().unwrap();
    let mut fields = std::collections::BTreeMap::new();
    for (name, v) in [
        ("apply_grain", 1),
        ("grain_seed", i32::from(g.seed)),
        ("num_y_points", g.y_points.len() as i32),
        ("num_cb_points", g.cb_points.len() as i32),
        ("num_cr_points", g.cr_points.len() as i32),
        (
            "chroma_scaling_from_luma",
            i32::from(g.chroma_scaling_from_luma),
        ),
        ("grain_scaling_minus_8", i32::from(g.scaling_shift - 8)),
        ("ar_coeff_lag", i32::from(g.ar_coeff_lag)),
        ("ar_coeff_shift_minus_6", i32::from(g.ar_coeff_shift - 6)),
        ("grain_scale_shift", i32::from(g.grain_scale_shift)),
        ("overlap_flag", i32::from(g.overlap)),
        (
            "clip_to_restricted_range",
            i32::from(g.clip_to_restricted_range),
        ),
    ] {
        fields.insert(name.to_string(), v);
    }
    for (name, points) in [
        ("y", &g.y_points),
        ("cb", &g.cb_points),
        ("cr", &g.cr_points),
    ] {
        for (i, p) in points.iter().enumerate() {
            fields.insert(format!("point_{name}_value[{i}]"), i32::from(p[0]));
            fields.insert(format!("point_{name}_scaling[{i}]"), i32::from(p[1]));
        }
    }
    for (name, coeffs) in [
        ("y", &g.ar_coeffs_y),
        ("cb", &g.ar_coeffs_cb),
        ("cr", &g.ar_coeffs_cr),
    ] {
        for (i, &c) in coeffs.iter().enumerate() {
            fields.insert(
                format!("ar_coeffs_{name}_plus_128[{i}]"),
                i32::from(c) + 128,
            );
        }
    }
    for (name, scaling) in [("cb", g.cb_scaling), ("cr", g.cr_scaling)] {
        if let Some((m, l, o)) = scaling {
            fields.insert(format!("{name}_mult"), i32::from(m));
            fields.insert(format!("{name}_luma_mult"), i32::from(l));
            fields.insert(format!("{name}_offset"), i32::from(o));
        }
    }
    let expected: std::collections::BTreeMap<_, _> =
        include_str!("../../../tests/oracles/grain_header_values.txt")
            .lines()
            .map(|line| {
                let (k, v) = line.split_once(' ').unwrap();
                (k.to_string(), v.parse::<i32>().unwrap())
            })
            .collect();
    assert_eq!(fields, expected);
    assert!(state.pictures().reference(0).unwrap().is_none());
    let s = state.sequence().unwrap().clone();
    let mut dpb = DecodedPictureBuffer::default();
    assert_eq!(
        dpb.commit_intra(&s, h, pixels(&s, h, 100), None, 0),
        Err(CodecError::Unsupported)
    );
    assert!(dpb.reference(0).unwrap().is_none());
}
struct Bits {
    data: Vec<u8>,
    n: usize,
}
impl Bits {
    fn new() -> Self {
        Self { data: vec![], n: 0 }
    }
    fn put(&mut self, v: u32, n: usize) {
        for i in (0..n).rev() {
            if self.n.is_multiple_of(8) {
                self.data.push(0);
            }
            let last = self.data.len() - 1;
            self.data[last] |= (((v >> i) & 1) as u8) << (7 - self.n % 8);
            self.n += 1;
        }
    }
}
fn context() -> GrainContext {
    GrainContext {
        present: true,
        show_frame: true,
        showable_frame: false,
        inter: false,
        monochrome: false,
        subsampling_x: true,
        subsampling_y: true,
    }
}
#[test]
fn grain_inheritance_validation_and_atomic_reader() {
    let mut state = DecoderState::default();
    codec_av1::obu::validate_sample_with_state(
        include_bytes!("../../../tests/intra_film_grain.obu"),
        &mut state,
    )
    .unwrap();
    let base = state.last_header().unwrap().film_grain.clone().unwrap();
    let mut refs = std::array::from_fn(|_| None);
    refs[3] = Some(base.clone());
    let mut bits = Bits::new();
    bits.put(1, 1);
    bits.put(0, 16);
    bits.put(0, 1);
    bits.put(3, 3);
    let mut c = context();
    c.inter = true;
    let mut r = BitReader::new(&bits.data);
    let inherited = FilmGrainParameters::parse(&mut r, c, &refs, &[0, 1, 2, 3, 4, 5, 6])
        .unwrap()
        .unwrap();
    let mut expected = base;
    expected.seed = 0;
    assert_eq!(inherited, expected);
    let mut r = BitReader::new(&bits.data);
    let before = r.bits_left();
    assert!(FilmGrainParameters::parse(&mut r, c, &refs, &[0, 1, 2, 4, 5, 6, 7]).is_err());
    assert_eq!(r.bits_left(), before);
    let mut bad = Bits::new();
    bad.put(1, 1);
    bad.put(9, 16);
    bad.put(15, 4);
    let mut r = BitReader::new(&bad.data);
    let before = r.bits_left();
    assert!(FilmGrainParameters::parse(&mut r, context(), &refs, &[]).is_err());
    assert_eq!(r.bits_left(), before);
}
#[test]
fn show_existing_prefix_does_not_invent_inter_type_or_ignore_timing() {
    let (mut s, _) = fixture();
    s.reduced_still_picture_header = false;
    let data = [0x80];
    let mut r = BitReader::new(&data);
    let h = ExtendedFrameHeader::parse_prefix(&mut r, &s).unwrap();
    assert_eq!(h.frame_type, ExtendedFrameType::UnknownReference);
    s.decoder_model_info_present_flag = true;
    let mut r = BitReader::new(&data);
    let before = r.bits_left();
    assert_eq!(
        ExtendedFrameHeader::parse_prefix(&mut r, &s),
        Err(CodecError::Unsupported)
    );
    assert_eq!(r.bits_left(), before);
}

#[test]
fn grain_all_ar_lags_monochrome_luma_scaling_and_truncation() {
    let refs = std::array::from_fn(|_| None);
    for lag in 0..=3 {
        for mono in [false, true] {
            let mut c = context();
            c.monochrome = mono;
            let mut b = Bits::new();
            b.put(1, 1);
            b.put(617, 16);
            b.put(1, 4);
            b.put(100, 8);
            b.put(230, 8);
            if !mono {
                b.put(1, 1);
            } // Chroma scaling from luma, so no chroma points.
            b.put(2, 2);
            b.put(lag, 2);
            let count = (2 * lag * (lag + 1)) as usize;
            for i in 0..count {
                b.put((i % 256) as u32, 8);
            }
            if !mono {
                for i in 0..count + 1 {
                    b.put((255 - i) as u32, 8);
                }
                for i in 0..count + 1 {
                    b.put((128 + i) as u32, 8);
                }
            }
            b.put(3, 2);
            b.put(2, 2);
            b.put(1, 1);
            b.put(0, 1);
            let mut r = BitReader::new(&b.data);
            let p = FilmGrainParameters::parse(&mut r, c, &refs, &[])
                .unwrap()
                .unwrap();
            assert_eq!(p.ar_coeff_lag, lag as u8);
            assert_eq!(p.ar_coeffs_y.len(), count);
            assert_eq!(p.ar_coeff_shift, 9);
            assert_eq!(p.scaling_shift, 10);
            assert!(p.overlap);
            assert_eq!(p.ar_coeffs_cb.len(), if mono { 0 } else { count + 1 });
            assert_eq!(p.ar_coeffs_cr.len(), if mono { 0 } else { count + 1 });
            assert_eq!(r.bits_left(), b.data.len() * 8 - b.n);
            for cut in 0..b.data.len() {
                let mut r = BitReader::new(&b.data[..cut]);
                let before = r.bits_left();
                assert!(FilmGrainParameters::parse(&mut r, c, &refs, &[]).is_err());
                assert_eq!(r.bits_left(), before);
            }
        }
    }
    let mut b = Bits::new();
    b.put(1, 1);
    b.put(1, 16);
    b.put(2, 4);
    b.put(90, 8);
    b.put(1, 8);
    b.put(90, 8);
    b.put(2, 8);
    assert!(
        FilmGrainParameters::parse(&mut BitReader::new(&b.data), context(), &refs, &[]).is_err()
    );
    let mut c = context();
    c.present = false;
    let mut r = BitReader::new(&[]);
    assert_eq!(
        FilmGrainParameters::parse(&mut r, c, &refs, &[]).unwrap(),
        None
    );
}

#[test]
fn state_finalization_flush_reset_and_corrupt_sample_are_transactional() {
    let data = include_bytes!("../../../tests/intra_lossless.obu");
    let mut state = DecoderState::default();
    assert!(state.flush().is_ok());
    codec_av1::obu::validate_sample_with_state(data, &mut state).unwrap();
    assert_eq!(state.flush(), Err(CodecError::NeedMoreData));
    let s = state.sequence().unwrap().clone();
    let h = state.last_header().unwrap().clone();
    let out = state
        .commit_intra_picture(pixels(&s, &h, 60), None)
        .unwrap()
        .unwrap();
    assert!(state.flush().is_ok());
    assert!(state
        .commit_intra_picture(pixels(&s, &h, 61), None)
        .is_err());
    let saved = state.clone();
    assert!(
        codec_av1::obu::validate_sample_with_state(&data[..data.len() / 2], &mut state).is_err()
    );
    assert_eq!(state, saved);
    state.reset();
    assert!(state.pictures().reference(0).unwrap().is_none());
    assert!(state.sequence().is_none());
    assert_eq!(out.pixels.planes()[0].row(0).unwrap()[0], 60);
}

#[test]
fn filter_validation_and_unsupported_paths_do_not_modify_buffers() {
    use codec_av1::frame::filters::*;
    let mut edge = [100u16; 14];
    let saved = edge;
    assert_eq!(
        deblock_edge_13tap(&mut edge, 20, 0, 14),
        Err(CodecError::InvalidData)
    );
    assert_eq!(edge, saved);
    assert_eq!(deblock_edge_13tap(&mut edge, 20, 0, 8), Ok(()));
    assert_eq!(edge, saved);
    assert_eq!(cdef_constrain(7, 8, 5), 7);
    assert_eq!(cdef_constrain(-7, 8, 5), -7);
    assert_eq!(cdef_constrain(i32::MIN, 255, 0), 0);
    assert!(cdef_find_direction_checked(&[], 0, 8).is_err());
    assert!(superres_upscale_row(&[1; 16], 32, 8, 8).is_err());
    assert!(codec_av1::frame::superres::upscale_row(&[256], 2, 8).is_err());
    assert!(codec_av1::frame::superres::upscale_row(&[1], usize::MAX, 8).is_err());
}

#[test]
fn frame_id_window_wraparound_and_failed_refresh_preserve_references() {
    let (mut s, mut h) = fixture();
    s.frame_id_length = 5;
    s.delta_frame_id_length = 2;
    h.show_frame = false;
    h.showable_frame = true;
    h.refresh_frame_flags = 1;
    h.frame_id = 31;
    let mut dpb = DecodedPictureBuffer::default();
    dpb.commit_intra(&s, &h, pixels(&s, &h, 31), None, 0)
        .unwrap();
    h.frame_id = 1;
    h.refresh_frame_flags = 2;
    dpb.commit_intra(&s, &h, pixels(&s, &h, 1), None, 1)
        .unwrap();
    assert_eq!(dpb.reference(0).unwrap().unwrap().header.frame_id, 31);
    h.frame_id = 5;
    h.refresh_frame_flags = 4;
    let saved = dpb.clone();
    h.film_grain = Some(dummy_grain());
    assert!(dpb
        .commit_intra(&s, &h, pixels(&s, &h, 5), None, 2)
        .is_err());
    assert_eq!(dpb, saved);
    h.film_grain = None;
    dpb.commit_intra(&s, &h, pixels(&s, &h, 5), None, 2)
        .unwrap();
    assert!(dpb.reference(0).unwrap().is_none());
    assert_eq!(dpb.reference(1).unwrap().unwrap().header.frame_id, 1);
    h.frame_id = 32;
    assert!(dpb
        .commit_intra(&s, &h, pixels(&s, &h, 5), None, 3)
        .is_err());
}

#[test]
fn restoration_stripe_boundaries_reuse_preserved_pre_cdef_samples() {
    use codec_av1::frame::restoration::{RestorationSource, SgrprojParameters, WienerCoefficients};
    let pre = Plane::new(
        4,
        10,
        4,
        (0..10).flat_map(|y| vec![y * 10 + 5; 4]).collect(),
    )
    .unwrap();
    let post = Plane::new(4, 10, 4, (0..10).flat_map(|y| vec![100 + y; 4]).collect()).unwrap();
    let source = RestorationSource::new(&pre, &post, (4, 6), 8).unwrap();
    assert_eq!(source.sample(0, 0).unwrap(), 25);
    assert_eq!(source.sample(0, 2).unwrap(), 25);
    assert_eq!(source.sample(0, 4).unwrap(), 104);
    assert_eq!(source.sample(0, 5).unwrap(), 105);
    assert_eq!(source.sample(0, 6).unwrap(), 65);
    assert_eq!(source.sample(0, 9).unwrap(), 75);
    let out = WienerCoefficients {
        horizontal: [0; 3],
        vertical: [0; 3],
    }
    .apply(&source, (0, 4), (4, 2))
    .unwrap();
    assert_eq!(out, [104, 104, 104, 104, 105, 105, 105, 105]);
    assert!(WienerCoefficients {
        horizontal: [11, 0, 0],
        vertical: [0; 3]
    }
    .apply(&source, (0, 4), (4, 2))
    .is_err());
    assert!(SgrprojParameters {
        set: 16,
        xqd: [0, 0]
    }
    .apply(&source, (0, 4), (4, 2))
    .is_err());
    assert!(SgrprojParameters {
        set: 0,
        xqd: [-97, 0]
    }
    .apply(&source, (0, 4), (4, 2))
    .is_err());
    assert!(SgrprojParameters {
        set: 0,
        xqd: [0, 0]
    }
    .apply(&source, (0, 3), (4, 2))
    .is_err());
}

#[test]
fn high_bit_depth_cdef_adjustment_retains_fractional_strength() {
    let k = codec_av1::frame::cdef::CdefKernel::for_luma(4, 15, 4, 3, 12, 128).unwrap();
    assert_eq!(k.primary, 75);
    assert_eq!(k.secondary, 64);
    let k = codec_av1::frame::cdef::CdefKernel::for_luma(4, 0, 2, 3, 10, 128).unwrap();
    assert_eq!(k.direction, 0);
    assert_eq!(k.primary, 0);
}

#[test]
fn chroma_cdef_remaps_direction_and_damping() {
    let k = codec_av1::frame::cdef::CdefKernel::for_chroma(0, true, false, 7, 2, 3, 10).unwrap();
    assert_eq!(k.direction, 7);
    assert_eq!(k.primary, 28);
    assert_eq!(k.secondary, 8);
    assert_eq!(k.damping, 2);
    let k = codec_av1::frame::cdef::CdefKernel::for_chroma(5, true, true, 0, 1, 4, 8).unwrap();
    assert_eq!(k.direction, 0);
}
