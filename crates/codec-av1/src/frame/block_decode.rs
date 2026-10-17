//! Intra leaf-block syntax decoding and spatial reconstruction.
use super::block_syntax_tables::*;
use super::{
    cfl::{predict_cfl_block, CflBlock},
    coeff_decode::{decode_tx_block_4x4_with_type, update_entropy_context, TileCoeffCdfs},
    intra::{mode_to_nominal_angle, predict_basic_block, predict_directional, IntraMode},
    palette::{self, Colors, PaletteCdfs},
    partition::{BlockSize, PartitionCdfs, PartitionType},
    partition_decode::{BlockGeometry, PartitionRequest, PartitionSyntax},
    picture::{PictureReconstruction, TransformBlock},
    quantization::QuantizerSteps,
    transform::{TxSize, TxType2D},
};
use crate::entropy::SymbolDecoder;
use codec_core::{ChromaSubsampling, CodecError, CodecResult, PlaneId};

/// AV1 8x8-and-larger intra tools require both coded dimensions >= 8.
#[inline]
fn supports_8x8_intra_tools(block: BlockSize) -> bool {
    let (width, height) = block.dimensions_px();
    width >= 8 && height >= 8
}

#[inline]
pub fn intra_mode_ctx(mode: IntraMode) -> usize {
    match mode {
        IntraMode::DcPred | IntraMode::SmoothPred | IntraMode::PaethPred => 0,
        IntraMode::VPred | IntraMode::SmoothVPred => 1,
        IntraMode::HPred | IntraMode::SmoothHPred => 2,
        IntraMode::D45Pred | IntraMode::D67Pred => 3,
        IntraMode::D135Pred | IntraMode::D113Pred | IntraMode::D157Pred | IntraMode::D203Pred => 4,
    }
}

pub fn intra_mode_from_sym(sym: usize) -> CodecResult<IntraMode> {
    match sym {
        0 => Ok(IntraMode::DcPred),
        1 => Ok(IntraMode::VPred),
        2 => Ok(IntraMode::HPred),
        3 => Ok(IntraMode::D45Pred),
        4 => Ok(IntraMode::D135Pred),
        5 => Ok(IntraMode::D113Pred),
        6 => Ok(IntraMode::D157Pred),
        7 => Ok(IntraMode::D203Pred),
        8 => Ok(IntraMode::D67Pred),
        9 => Ok(IntraMode::SmoothPred),
        10 => Ok(IntraMode::SmoothVPred),
        11 => Ok(IntraMode::SmoothHPred),
        12 => Ok(IntraMode::PaethPred),
        _ => Err(CodecError::InvalidData),
    }
}

/// AV1 Mode_To_Txfm for inferred 4x4 intra chroma transforms.
fn chroma_tx_type(mode: IntraMode) -> TxType2D {
    match mode {
        IntraMode::DcPred | IntraMode::D45Pred => TxType2D::DctDct,
        IntraMode::VPred | IntraMode::D113Pred | IntraMode::D67Pred | IntraMode::SmoothVPred => {
            TxType2D::AdstDct
        }
        IntraMode::HPred | IntraMode::D157Pred | IntraMode::D203Pred | IntraMode::SmoothHPred => {
            TxType2D::DctAdst
        }
        IntraMode::D135Pred | IntraMode::SmoothPred | IntraMode::PaethPred => TxType2D::AdstAdst,
    }
}

fn read_intra_tx_type_4x4(
    dec: &mut SymbolDecoder<'_>,
    reduced: bool,
    set1: &mut [u16; 8],
    set2: &mut [u16; 6],
) -> CodecResult<TxType2D> {
    const SET1: [TxType2D; 7] = [
        TxType2D::Idtx,
        TxType2D::DctDct,
        TxType2D::VIdtx,
        TxType2D::HIdtx,
        TxType2D::AdstAdst,
        TxType2D::AdstDct,
        TxType2D::DctAdst,
    ];
    const SET2: [TxType2D; 5] = [
        TxType2D::Idtx,
        TxType2D::DctDct,
        TxType2D::AdstAdst,
        TxType2D::AdstDct,
        TxType2D::DctAdst,
    ];
    Ok(if reduced {
        SET2[dec.read_symbol(set2)?]
    } else {
        SET1[dec.read_symbol(set1)?]
    })
}

pub struct TileCdfs<'a> {
    pub intra_tx_set1: &'a mut [[u16; 8]; 13],
    pub intra_tx_set2: &'a mut [[u16; 6]; 13],
    pub partition_cdfs: &'a mut PartitionCdfs,
    pub skip_cdfs: &'a mut [[u16; 3]; 3],
    pub kf_y_cdf: &'a mut [[[u16; 14]; 5]; 5],
    pub uv_mode_cdf: &'a mut [[u16; 14]; 13],
    pub uv_mode_cfl_cdf: &'a mut [[u16; 15]; 13],
    pub angle_delta_cdf: &'a mut [[u16; 8]; 8],
    pub cfl_sign_cdf: &'a mut [u16; 9],
    pub cfl_alpha_cdf: &'a mut [[u16; 17]; 6],
    pub txb_skip_cdfs: &'a mut [[[u16; 3]; 13]; 5],
    pub eob_multi16_cdfs: &'a mut [[[u16; 6]; 2]; 2],
    pub eob_extra_cdfs: &'a mut [[[[u16; 3]; 9]; 2]; 5],
    pub base_eob_cdfs: &'a mut [[[[u16; 4]; 4]; 2]; 5],
    pub base_cdfs: &'a mut [[[[u16; 5]; 42]; 2]; 5],
    pub br_cdfs: &'a mut [[[[u16; 5]; 21]; 2]; 5],
    pub palette_y: &'a mut [[[u16; 3]; 3]; 7],
    pub palette_uv: &'a mut [[u16; 3]; 2],
    pub filter_intra: &'a mut [[u16; 3]; 22],
    pub dc_sign_cdfs: &'a mut [[[u16; 3]; 3]; 2],
}

pub struct IntraBlockDecoder<'a, 'data> {
    pub dec: &'a mut SymbolDecoder<'data>,
    pub cdfs: TileCdfs<'a>,
    pub recon: &'a mut PictureReconstruction,
    pub width: usize,
    pub height: usize,
    pub mi_cols: usize,
    pub mi_rows: usize,
    pub bit_depth: u8,
    pub enable_intra_edge_filter: bool,
    /// Tile origin in luma samples. Prediction must never cross these edges.
    tile_origin: (usize, usize),
    lossless: bool,
    signal_tx_type: bool,
    reduced_tx_set: bool,
    quantizer_steps: [QuantizerSteps; 3],
    subsampling: ChromaSubsampling,
    chroma_shifts: (usize, usize),
    allow_screen_content_tools: bool,
    enable_filter_intra: bool,
    above_mode: Vec<IntraMode>,
    left_mode: Vec<IntraMode>,
    above_skip: Vec<bool>,
    left_skip: Vec<bool>,
    above_ec: [Vec<u8>; 3],
    left_ec: [Vec<u8>; 3],
    palette_cdfs: PaletteCdfs,
    above_palette: Vec<[Colors; 2]>,
    left_palette: Vec<[Colors; 2]>,
    filter_intra_mode_cdf: [u16; 6],
}

impl<'a, 'data> IntraBlockDecoder<'a, 'data> {
    #[allow(clippy::too_many_arguments)]
    pub fn new(
        dec: &'a mut SymbolDecoder<'data>,
        cdfs: TileCdfs<'a>,
        recon: &'a mut PictureReconstruction,
        width: usize,
        height: usize,
        mi_cols: usize,
        mi_rows: usize,
        bit_depth: u8,
        enable_intra_edge_filter: bool,
        allow_screen_content_tools: bool,
        enable_filter_intra: bool,
        tile_origin: (usize, usize),
        header: &super::FrameHeader,
        quantizer_steps: [QuantizerSteps; 3],
        subsampling: ChromaSubsampling,
    ) -> CodecResult<Self> {
        let chroma_shifts = match subsampling {
            ChromaSubsampling::Cs420 => (1, 1),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs444 => (0, 0),
            ChromaSubsampling::Monochrome => return Err(CodecError::Unsupported),
        };
        Ok(Self {
            dec,
            cdfs,
            recon,
            width,
            height,
            mi_cols,
            mi_rows,
            bit_depth,
            enable_intra_edge_filter,
            tile_origin,
            lossless: header.coded_lossless,
            signal_tx_type: !header.coded_lossless && header.quantization.base_q_idx > 0,
            reduced_tx_set: header.reduced_tx_set,
            quantizer_steps,
            subsampling,
            chroma_shifts,
            allow_screen_content_tools,
            enable_filter_intra,
            palette_cdfs: PaletteCdfs::default(),
            filter_intra_mode_cdf: super::filter_intra_tables::FILTER_INTRA_MODE,
            above_palette: vec![[Colors::default(); 2]; mi_cols],
            left_palette: vec![[Colors::default(); 2]; mi_rows],
            above_mode: vec![IntraMode::DcPred; mi_cols],
            left_mode: vec![IntraMode::DcPred; mi_rows],
            above_skip: vec![false; mi_cols],
            left_skip: vec![false; mi_rows],
            above_ec: [
                vec![0; mi_cols],
                vec![0; mi_cols.div_ceil(1 << chroma_shifts.0)],
                vec![0; mi_cols.div_ceil(1 << chroma_shifts.0)],
            ],
            left_ec: [
                vec![0; mi_rows],
                vec![0; mi_rows.div_ceil(1 << chroma_shifts.1)],
                vec![0; mi_rows.div_ceil(1 << chroma_shifts.1)],
            ],
        })
    }
}

#[allow(clippy::too_many_arguments)]
fn predict_sub_block(
    recon: &PictureReconstruction,
    bit_depth: u8,
    enable_intra_edge_filter: bool,
    plane_id: PlaneId,
    px: usize,
    py: usize,
    tile_origin: (usize, usize),
    subsampling: ChromaSubsampling,
    mode: IntraMode,
    angle_delta: i32,
    is_cfl: bool,
    alpha: i8,
    filter_mode: Option<usize>,
    pred: &mut [u16; 16],
) -> CodecResult<()> {
    let have_above = py > tile_origin.1;
    let have_left = px > tile_origin.0;

    let corner = if have_above && have_left {
        recon.sample(plane_id, px - 1, py - 1)
    } else {
        None
    };

    let mut above_full = [0u16; 16];
    let mut left_full = [0u16; 16];

    let corner_val = match (have_above, have_left) {
        (true, true) => corner.ok_or(CodecError::InvalidData)?,
        (true, false) => recon
            .sample(plane_id, px, py - 1)
            .ok_or(CodecError::InvalidData)?,
        (false, true) => recon
            .sample(plane_id, px - 1, py)
            .ok_or(CodecError::InvalidData)?,
        (false, false) => 1u16 << (bit_depth - 1),
    };
    above_full[0] = corner_val;
    left_full[0] = corner_val;

    if have_above {
        let mut last = corner_val;
        for i in 0..8 {
            if let Some(s) = recon.sample(plane_id, px + i, py - 1) {
                last = s;
            }
            above_full[1 + i] = last;
        }
    } else if have_left {
        let fill = recon
            .sample(plane_id, px - 1, py)
            .ok_or(CodecError::InvalidData)?;
        above_full[1..9].fill(fill);
    } else {
        above_full[1..9].fill((1u16 << (bit_depth - 1)) - 1);
    }

    if have_left {
        let mut last = corner_val;
        for i in 0..8 {
            if let Some(s) = recon.sample(plane_id, px - 1, py + i) {
                last = s;
            }
            left_full[1 + i] = last;
        }
    } else if have_above {
        let fill = recon
            .sample(plane_id, px, py - 1)
            .ok_or(CodecError::InvalidData)?;
        left_full[1..9].fill(fill);
    } else {
        left_full[1..9].fill((1u16 << (bit_depth - 1)) + 1);
    }

    if let Some(mode) = filter_mode {
        for y in (0..4).step_by(2) {
            let mut p = [0u16; 7];
            if y == 0 {
                p[..5].copy_from_slice(&above_full[..5]);
            } else {
                p[0] = left_full[y];
                p[1..5].copy_from_slice(&pred[(y - 1) * 4..y * 4]);
            }
            p[5] = left_full[y + 1];
            p[6] = left_full[y + 2];
            for i in 0..8 {
                let sum: i32 = super::filter_intra_tables::INTRA_FILTER_TAPS[mode][i]
                    .iter()
                    .zip(p)
                    .map(|(&tap, sample)| i32::from(tap) * i32::from(sample))
                    .sum();
                let rounded = if sum < 0 {
                    -((-sum + 8) >> 4)
                } else {
                    (sum + 8) >> 4
                };
                pred[y * 4 + i] = rounded.clamp(0, (1 << bit_depth) - 1) as u16;
            }
        }
        Ok(())
    } else if is_cfl {
        let mut dc_pred = [0u16; 16];
        predict_basic_block(
            IntraMode::DcPred,
            &mut dc_pred,
            4,
            4,
            4,
            bit_depth,
            if have_above {
                Some(&above_full[1..5])
            } else {
                None
            },
            if have_left {
                Some(&left_full[1..5])
            } else {
                None
            },
            corner,
        )?;
        let dc_val = dc_pred[0];

        let (sx, sy) = match subsampling {
            ChromaSubsampling::Cs420 => (1, 1),
            ChromaSubsampling::Cs422 => (1, 0),
            ChromaSubsampling::Cs444 => (0, 0),
            ChromaSubsampling::Monochrome => return Err(CodecError::Unsupported),
        };
        let mut luma_buf = [0u16; 64];
        let luma_stride = 4 << sx;
        for ly in 0..(4 << sy) {
            for lx in 0..luma_stride {
                luma_buf[ly * luma_stride + lx] = recon
                    .sample(PlaneId::Y, (px << sx) + lx, (py << sy) + ly)
                    .ok_or(CodecError::InvalidData)?;
            }
        }

        predict_cfl_block(
            &luma_buf,
            pred,
            CflBlock {
                width: 4,
                height: 4,
                luma_stride,
                chroma_stride: 4,
                subsampling,
                bit_depth,
                alpha,
                dc: dc_val,
            },
        )
    } else if let Some(nominal) = mode_to_nominal_angle(mode) {
        let p_angle = (nominal as i32 + angle_delta * 3) as usize;
        predict_directional(
            p_angle,
            pred,
            4,
            4,
            4,
            bit_depth,
            &above_full[..9],
            &left_full[..9],
            enable_intra_edge_filter,
            0,
        )
    } else {
        predict_basic_block(
            mode,
            pred,
            4,
            4,
            4,
            bit_depth,
            if have_above {
                Some(&above_full[1..5])
            } else {
                None
            },
            if have_left {
                Some(&left_full[1..5])
            } else {
                None
            },
            corner,
        )
    }
}

impl PartitionSyntax for IntraBlockDecoder<'_, '_> {
    fn read_partition(&mut self, request: PartitionRequest) -> CodecResult<PartitionType> {
        self.cdfs.partition_cdfs.read_partition(self.dec, request)
    }

    fn decode_leaf(&mut self, block: BlockGeometry) -> CodecResult<()> {
        let r = block.mi_row as usize;
        let c = block.mi_col as usize;
        let (bw_px, bh_px) = block.bsize.dimensions_px();
        if !(4..=64).contains(&bw_px) || !(4..=64).contains(&bh_px) {
            return Err(CodecError::Unsupported);
        }
        let bw4 = (bw_px / 4) as usize;
        let bh4 = (bh_px / 4) as usize;

        // 1. Skip flag
        let has_above = r * 4 > self.tile_origin.1;
        let has_left = c * 4 > self.tile_origin.0;
        let sctx = (if has_above {
            self.above_skip[c] as usize
        } else {
            0
        }) + (if has_left {
            self.left_skip[r] as usize
        } else {
            0
        });
        let skip = self.dec.read_symbol(&mut self.cdfs.skip_cdfs[sctx])? != 0;

        // 2. Luma Mode
        let above_mode = if has_above {
            self.above_mode[c]
        } else {
            IntraMode::DcPred
        };
        let left_mode = if has_left {
            self.left_mode[r]
        } else {
            IntraMode::DcPred
        };
        let above_ctx = intra_mode_ctx(above_mode);
        let left_ctx = intra_mode_ctx(left_mode);
        let y_mode_sym = self
            .dec
            .read_symbol(&mut self.cdfs.kf_y_cdf[above_ctx][left_ctx])?;
        let y_mode = intra_mode_from_sym(y_mode_sym)?;

        let mut angle_delta_y = 0;
        if (1..=8).contains(&y_mode_sym) && supports_8x8_intra_tools(block.bsize) {
            let delta_sym = self
                .dec
                .read_symbol(&mut self.cdfs.angle_delta_cdf[y_mode_sym - 1])?;
            angle_delta_y = (delta_sym as i32) - 3;
        }

        // 3. Chroma Mode
        let (sx, sy) = self.chroma_shifts;
        let (xmask, ymask) = ((1 << sx) - 1, (1 << sy) - 1);
        let has_chroma =
            ((r & ymask) != 0 || (bh4 & ymask) == 0) && ((c & xmask) != 0 || (bw4 & xmask) == 0);
        let mut uv_mode = IntraMode::DcPred;
        let mut angle_delta_uv = 0;
        let mut is_cfl = false;
        let mut cfl_alpha_u = 0i8;
        let mut cfl_alpha_v = 0i8;

        if has_chroma {
            let cbw4 = (bw4 >> sx).max(1);
            let cbh4 = (bh4 >> sy).max(1);
            let cfl_allowed = cbw4 == 1 && cbh4 == 1;
            let uv_mode_sym = if cfl_allowed {
                self.dec
                    .read_symbol(&mut self.cdfs.uv_mode_cfl_cdf[y_mode_sym])?
            } else {
                self.dec
                    .read_symbol(&mut self.cdfs.uv_mode_cdf[y_mode_sym])?
            };

            if uv_mode_sym == 13 {
                is_cfl = true;
                let cfl_signs = self.dec.read_symbol(&mut *self.cdfs.cfl_sign_cdf)?;
                let sign_u = (cfl_signs + 1) / 3;
                let sign_v = (cfl_signs + 1) % 3;
                let cfl_ctx = cfl_signs.saturating_sub(2);
                if sign_u != 0 {
                    let u = self
                        .dec
                        .read_symbol(&mut self.cdfs.cfl_alpha_cdf[cfl_ctx])?
                        as i8
                        + 1;
                    cfl_alpha_u = if sign_u == 1 { -u } else { u };
                }
                if sign_v != 0 {
                    let vctx = (sign_v - 1) * 3 + sign_u;
                    let v = self.dec.read_symbol(&mut self.cdfs.cfl_alpha_cdf[vctx])? as i8 + 1;
                    cfl_alpha_v = if sign_v == 1 { -v } else { v };
                }
            } else {
                uv_mode = intra_mode_from_sym(uv_mode_sym)?;
                if (1..=8).contains(&uv_mode_sym) && supports_8x8_intra_tools(block.bsize) {
                    let delta_sym = self
                        .dec
                        .read_symbol(&mut self.cdfs.angle_delta_cdf[uv_mode_sym - 1])?;
                    angle_delta_uv = (delta_sym as i32) - 3;
                }
            }
        }

        let mut palettes = [Colors::default(); 3];
        if self.allow_screen_content_tools
            && supports_8x8_intra_tools(block.bsize)
            && bw_px.max(bh_px) <= 64
        {
            let ctx = (bw_px.ilog2() + bh_px.ilog2() - 6) as usize;
            let yctx = usize::from(has_above && self.above_palette[c][0].len > 0)
                + usize::from(has_left && self.left_palette[r][0].len > 0);
            if y_mode == IntraMode::DcPred
                && self.dec.read_symbol(&mut self.cdfs.palette_y[ctx][yctx])? != 0
            {
                let size = self.dec.read_symbol(&mut self.palette_cdfs.sizes[0][ctx])? + 2;
                palettes[0] = palette::read_colors(
                    self.dec,
                    size,
                    self.bit_depth,
                    if !r.is_multiple_of(16) {
                        self.above_palette[c][0]
                    } else {
                        Colors::default()
                    },
                    if has_left {
                        self.left_palette[r][0]
                    } else {
                        Colors::default()
                    },
                    true,
                )?;
            }
            if has_chroma
                && uv_mode == IntraMode::DcPred
                && !is_cfl
                && self
                    .dec
                    .read_symbol(&mut self.cdfs.palette_uv[usize::from(palettes[0].len > 0)])?
                    != 0
            {
                let size = self.dec.read_symbol(&mut self.palette_cdfs.sizes[1][ctx])? + 2;
                palettes[1] = palette::read_colors(
                    self.dec,
                    size,
                    self.bit_depth,
                    if !r.is_multiple_of(16) {
                        self.above_palette[c][1]
                    } else {
                        Colors::default()
                    },
                    if has_left {
                        self.left_palette[r][1]
                    } else {
                        Colors::default()
                    },
                    false,
                )?;
                palettes[2] = palette::read_v_colors(self.dec, size, self.bit_depth)?;
            }
        }
        let mut filter_mode = None;
        if self.enable_filter_intra
            && y_mode == IntraMode::DcPred
            && palettes[0].len == 0
            && bw_px.max(bh_px) <= 32
            && self
                .dec
                .read_symbol(&mut self.cdfs.filter_intra[block.bsize as usize])?
                != 0
        {
            filter_mode = Some(self.dec.read_symbol(&mut self.filter_intra_mode_cdf)?);
        }
        let mut maps = [Vec::new(), Vec::new()];
        // Oblique directional prediction still needs normative edge availability
        // and independent pixel oracles before enabling its compressed path.
        if [y_mode, uv_mode].iter().any(|&mode| {
            matches!(
                mode,
                IntraMode::D45Pred
                    | IntraMode::D135Pred
                    | IntraMode::D113Pred
                    | IntraMode::D157Pred
                    | IntraMode::D203Pred
                    | IntraMode::D67Pred
            )
        }) || angle_delta_y != 0
            || angle_delta_uv != 0
        {
            return Err(CodecError::Unsupported);
        }
        for p in 0..2 {
            if palettes[p].len > 0 {
                let (shift_x, shift_y) = if p == 0 { (0, 0) } else { (sx, sy) };
                maps[p] = self.palette_cdfs.read_map(
                    self.dec,
                    p,
                    palettes[p].len,
                    (
                        (bw_px as usize >> shift_x).max(4),
                        (bh_px as usize >> shift_y).max(4),
                    ),
                    (
                        ((bw_px as usize).min(self.width - c * 4) >> shift_x).max(4),
                        ((bh_px as usize).min(self.height - r * 4) >> shift_y).max(4),
                    ),
                )?;
            }
        }
        // 4. Decode & Reconstruct Y plane sub-blocks (4x4)

        let mut coeff_cdfs = TileCoeffCdfs {
            txb_skip_cdfs: self.cdfs.txb_skip_cdfs,
            eob_multi16_cdfs: self.cdfs.eob_multi16_cdfs,
            eob_extra_cdfs: self.cdfs.eob_extra_cdfs,
            base_eob_cdfs: self.cdfs.base_eob_cdfs,
            base_cdfs: self.cdfs.base_cdfs,
            br_cdfs: self.cdfs.br_cdfs,
            dc_sign_cdfs: self.cdfs.dc_sign_cdfs,
        };

        let max_sub_w = self.width.saturating_sub(c * 4).div_ceil(4).min(bw4);
        let max_sub_h = self.height.saturating_sub(r * 4).div_ceil(4).min(bh4);

        for by in 0..max_sub_h {
            for bx in 0..max_sub_w {
                let px = (c + bx) * 4;
                let py = (r + by) * 4;

                let mut pred = [0u16; 16];
                if palettes[0].len > 0 {
                    pred = palette::predict(palettes[0], &maps[0], bw_px as usize, bx * 4, by * 4);
                } else {
                    predict_sub_block(
                        self.recon,
                        self.bit_depth,
                        self.enable_intra_edge_filter,
                        PlaneId::Y,
                        px,
                        py,
                        self.tile_origin,
                        self.subsampling,
                        y_mode,
                        angle_delta_y,
                        false,
                        0,
                        filter_mode,
                        &mut pred,
                    )?;
                }

                let above_ec_val = self.above_ec[0][c + bx];
                let left_ec_val = self.left_ec[0][r + by];

                let (coeffs, all_zero, kind) = decode_tx_block_4x4_with_type(
                    self.dec,
                    &mut coeff_cdfs,
                    0,
                    block.bsize,
                    above_ec_val,
                    left_ec_val,
                    skip,
                    |dec| {
                        if !self.signal_tx_type {
                            return Ok(TxType2D::DctDct);
                        }
                        let direction =
                            filter_mode.map_or(y_mode_sym, |mode| [0, 1, 2, 6, 0][mode]);
                        read_intra_tx_type_4x4(
                            dec,
                            self.reduced_tx_set,
                            &mut self.cdfs.intra_tx_set1[direction],
                            &mut self.cdfs.intra_tx_set2[direction],
                        )
                    },
                )?;

                let dequantized = super::residual::dequantize_block(
                    &coeffs,
                    TxSize::Tx4x4,
                    self.quantizer_steps[0],
                    self.bit_depth,
                    None,
                )?;
                self.recon.write_transform(
                    PlaneId::Y,
                    (px, py),
                    TransformBlock {
                        prediction: &pred,
                        coefficients: &dequantized,
                        size: TxSize::Tx4x4,
                        kind,
                        lossless: self.lossless,
                    },
                )?;

                let new_ec = if all_zero {
                    0
                } else {
                    update_entropy_context(&coeffs)
                };
                self.above_ec[0][c + bx] = new_ec;
                self.left_ec[0][r + by] = new_ec;
            }
        }

        // 5. Decode & Reconstruct Chroma sub-blocks (U and V)
        if has_chroma {
            let cbw4 = (bw4 >> sx).max(1);
            let cbh4 = (bh4 >> sy).max(1);
            let c_col_start = (c & !xmask) >> sx;
            let c_row_start = (r & !ymask) >> sy;
            let plane_bsize = BlockSize::from_dimensions_px((cbw4 * 4) as u32, (cbh4 * 4) as u32)?;

            let chroma_w = self.width.div_ceil(1 << sx);
            let chroma_h = self.height.div_ceil(1 << sy);
            let max_c_w = chroma_w
                .saturating_sub(c_col_start * 4)
                .div_ceil(4)
                .min(cbw4);
            let max_c_h = chroma_h
                .saturating_sub(c_row_start * 4)
                .div_ceil(4)
                .min(cbh4);

            for (plane_id, plane_idx, alpha) in
                [(PlaneId::U, 1, cfl_alpha_u), (PlaneId::V, 2, cfl_alpha_v)]
            {
                for by in 0..max_c_h {
                    for bx in 0..max_c_w {
                        let c_col = c_col_start + bx;
                        let c_row = c_row_start + by;
                        let px = c_col * 4;
                        let py = c_row * 4;

                        let mut pred = [0u16; 16];
                        if palettes[plane_idx].len > 0 {
                            pred = palette::predict(
                                palettes[plane_idx],
                                &maps[1],
                                cbw4 * 4,
                                bx * 4,
                                by * 4,
                            );
                        } else {
                            predict_sub_block(
                                self.recon,
                                self.bit_depth,
                                self.enable_intra_edge_filter,
                                plane_id,
                                px,
                                py,
                                (self.tile_origin.0 >> sx, self.tile_origin.1 >> sy),
                                self.subsampling,
                                uv_mode,
                                angle_delta_uv,
                                is_cfl,
                                alpha,
                                None,
                                &mut pred,
                            )?;
                        }

                        let above_ec_val = self.above_ec[plane_idx][c_col];
                        let left_ec_val = self.left_ec[plane_idx][c_row];

                        let (coeffs, all_zero, kind) = decode_tx_block_4x4_with_type(
                            self.dec,
                            &mut coeff_cdfs,
                            plane_idx,
                            plane_bsize,
                            above_ec_val,
                            left_ec_val,
                            skip,
                            |_| {
                                Ok(if self.lossless || is_cfl {
                                    TxType2D::DctDct
                                } else {
                                    chroma_tx_type(uv_mode)
                                })
                            },
                        )?;

                        let dequantized = super::residual::dequantize_block(
                            &coeffs,
                            TxSize::Tx4x4,
                            self.quantizer_steps[plane_idx],
                            self.bit_depth,
                            None,
                        )?;
                        self.recon.write_transform(
                            plane_id,
                            (px, py),
                            TransformBlock {
                                prediction: &pred,
                                coefficients: &dequantized,
                                size: TxSize::Tx4x4,
                                kind,
                                lossless: self.lossless,
                            },
                        )?;

                        let new_ec = if all_zero {
                            0
                        } else {
                            update_entropy_context(&coeffs)
                        };
                        self.above_ec[plane_idx][c_col] = new_ec;
                        self.left_ec[plane_idx][c_row] = new_ec;
                    }
                }
            }
        }

        // 6. Update context state across the block extent
        for x in 0..bw4.min(self.mi_cols - c) {
            self.above_skip[c + x] = skip;
            self.above_mode[c + x] = y_mode;
            self.above_palette[c + x] = [palettes[0], palettes[1]];
        }
        for y in 0..bh4.min(self.mi_rows - r) {
            self.left_skip[r + y] = skip;
            self.left_mode[r + y] = y_mode;
            self.left_palette[r + y] = [palettes[0], palettes[1]];
        }
        Ok(())
    }
}

/// Decode one complete supported key picture to owned native YUV pixels.
///
/// The original 8/10-bit 4:2:0 lossless path supports rectangular 4..64 coding
/// Supports rectangular 4..64 coding blocks across 8, 10 and 12-bit depths and
/// 4:2:0, 4:2:2, and 4:4:4 subsampling formats for both lossless and lossy modes.
/// All use 64-pixel superblocks and independently initialized tiles, including
/// multiple tile groups. Real transform type, quantizer and coefficient syntax
/// drive lossy reconstruction. Only pictures with no active in-loop filtering
/// currently pass the compressed-picture acceptance gate; active filtering is
/// rejected until complete filter syntax and pixel parity are verified.
/// DC, zero-delta V/H, smooth, Paeth, CfL, palette and filter-intra are connected.
/// Segmentation, oblique prediction, IntraBC and film grain remain unsupported.
/// Other syntax returns `Unsupported`; failed decoding leaves `state` unchanged.
/// This does not commit reconstructed pictures to reference slots.
/// Input is bounded to 16 MiB and pictures to 4096x4096 samples.
pub fn decode_intra_frame(
    data: &[u8],
    state: &mut super::state::DecoderState,
) -> CodecResult<codec_core::PlanarFrame> {
    use super::default_cdfs::*;
    use crate::obu::validate_sample_with_state;

    if data.len() > 16 * 1024 * 1024 {
        return Err(CodecError::InvalidData);
    }
    let mut next = state.clone();
    let validation = validate_sample_with_state(data, &mut next)?;
    if validation.frame_headers.len() != 1 {
        return Err(CodecError::Unsupported);
    }
    if validation.tile_ranges.is_empty() {
        return Err(CodecError::InvalidData);
    }
    // Framing validation also accepts partial tile groups for streaming callers.
    // This whole-picture API must not use a prior last_header while a new frame
    // remains incomplete, or commit a partially parsed picture's state.
    if !matches!(next.parser(), super::state::ParserState::AwaitingHeader) {
        return Err(CodecError::NeedMoreData);
    }

    let header = next.last_header().ok_or(CodecError::InvalidData)?;
    let width = header.dimensions.width as usize;
    let height = header.dimensions.height as usize;
    let seq = next.sequence().ok_or(CodecError::InvalidData)?;
    let bit_depth = if seq.color_config.high_bitdepth {
        if seq.color_config.twelve_bit {
            12
        } else {
            10
        }
    } else {
        8
    };
    let subsampling = match (
        seq.color_config.subsampling_x,
        seq.color_config.subsampling_y,
    ) {
        (true, true) => ChromaSubsampling::Cs420,
        (true, false) => ChromaSubsampling::Cs422,
        (false, false) => ChromaSubsampling::Cs444,
        _ => return Err(CodecError::Unsupported),
    };

    if header.frame_type != super::FrameType::Key
        || header.segmentation.enabled
        || header.delta.q_res.is_some()
        || header.delta.lf_res.is_some()
        || header.quantization.qm.is_some()
        || header.allow_intrabc
        || header.film_grain.is_some()
        || width != header.dimensions.upscaled_width as usize
        || seq.use_128x128_superblock
        || seq.color_config.mono_chrome
        || !matches!(bit_depth, 8 | 10 | 12)
        || !width.is_multiple_of(8)
        || !height.is_multiple_of(8)
        || width > 4096
        || height > 4096
    {
        return Err(CodecError::Unsupported);
    }
    if header.cdef.bits > 3
        || header.cdef.strengths.len()
            != 1usize
                .checked_shl(u32::from(header.cdef.bits))
                .ok_or(CodecError::InvalidData)?
    {
        return Err(CodecError::InvalidData);
    }
    // Filter kernels alone cannot decode per-block deblocking decisions, CDEF
    // indices, or restoration-unit coefficients. Reject these modes before
    // allocating reconstruction buffers or publishing decoder state.
    super::filters::require_verified_filter_configuration(
        &header.loop_filter,
        &header.cdef,
        &header.restoration,
    )?;
    let mut recon = PictureReconstruction::new(width, height, bit_depth, subsampling)?;

    let mi_cols = width.div_ceil(4);
    let mi_rows = height.div_ceil(4);
    let mut quantizer_steps = [QuantizerSteps { dc: 1, ac: 1 }; 3];
    for (plane, steps) in quantizer_steps.iter_mut().enumerate() {
        *steps =
            header
                .quantization
                .steps(&header.segmentation, 0, plane as u8, bit_depth, None)?;
    }
    let qctx = match header.quantization.base_q_idx {
        0..=20 => 0,
        21..=60 => 1,
        61..=120 => 2,
        _ => 3,
    };

    // AV1 tile traversal
    if validation.tile_ranges.len() != header.tiles.count() as usize {
        return Err(CodecError::InvalidData);
    }
    let tile_cols = header.tiles.col_starts.len() - 1;
    for (tile_id, tile_range) in validation.tile_ranges.into_iter().enumerate() {
        let tile_row = tile_id / tile_cols;
        let tile_col = tile_id % tile_cols;
        let row_start = header.tiles.row_starts[tile_row];
        let row_end = header.tiles.row_starts[tile_row + 1];
        let col_start = header.tiles.col_starts[tile_col];
        let col_end = header.tiles.col_starts[tile_col + 1];
        let tile_bytes = &data[tile_range];
        let mut dec = SymbolDecoder::new(tile_bytes, header.disable_cdf_update)?;

        let mut partition_cdfs = PartitionCdfs::default();
        let mut skip_cdfs = DEFAULT_SKIP_CDFS;
        let mut kf_y_cdf = DEFAULT_KF_Y_MODE_CDF;
        let mut uv_mode_cdf = DEFAULT_UV_MODE_CDF;
        let mut uv_mode_cfl_cdf = DEFAULT_UV_MODE_CFL_CDF;
        let mut angle_delta_cdf = DEFAULT_ANGLE_DELTA_CDF;
        let mut cfl_sign_cdf = DEFAULT_CFL_SIGN_CDF;
        let mut cfl_alpha_cdf = DEFAULT_CFL_ALPHA_CDF;
        let mut txb_skip_cdfs = super::lossy_tables::TXB_SKIP_BY_Q[qctx];
        let mut eob_multi16_cdfs = super::lossy_tables::EOB_MULTI16_BY_Q[qctx];
        let mut eob_extra_cdfs = super::lossy_tables::EOB_EXTRA_BY_Q[qctx];
        let mut base_eob_cdfs = super::lossy_tables::BASE_EOB_BY_Q[qctx];
        let mut base_cdfs = super::lossy_tables::BASE_BY_Q[qctx];
        let mut br_cdfs = super::lossy_tables::BR_BY_Q[qctx];
        let mut dc_sign_cdfs = super::lossy_tables::DC_SIGN_BY_Q[qctx];
        let mut intra_tx_set1 = super::lossy_tables::INTRA_TX_SET1[0];
        let mut intra_tx_set2 = super::lossy_tables::INTRA_TX_SET2[0];

        let mut palette_y = PALETTE_Y;
        let mut palette_uv = PALETTE_UV;
        let mut filter_intra = FILTER_INTRA;
        let tile_cdfs = TileCdfs {
            intra_tx_set1: &mut intra_tx_set1,
            intra_tx_set2: &mut intra_tx_set2,
            palette_y: &mut palette_y,
            palette_uv: &mut palette_uv,
            filter_intra: &mut filter_intra,
            partition_cdfs: &mut partition_cdfs,
            skip_cdfs: &mut skip_cdfs,
            kf_y_cdf: &mut kf_y_cdf,
            uv_mode_cdf: &mut uv_mode_cdf,
            uv_mode_cfl_cdf: &mut uv_mode_cfl_cdf,
            angle_delta_cdf: &mut angle_delta_cdf,
            cfl_sign_cdf: &mut cfl_sign_cdf,
            cfl_alpha_cdf: &mut cfl_alpha_cdf,
            txb_skip_cdfs: &mut txb_skip_cdfs,
            eob_multi16_cdfs: &mut eob_multi16_cdfs,
            eob_extra_cdfs: &mut eob_extra_cdfs,
            base_eob_cdfs: &mut base_eob_cdfs,
            base_cdfs: &mut base_cdfs,
            br_cdfs: &mut br_cdfs,
            dc_sign_cdfs: &mut dc_sign_cdfs,
        };

        let mut block_decoder = IntraBlockDecoder::new(
            &mut dec,
            tile_cdfs,
            &mut recon,
            width,
            height,
            mi_cols,
            mi_rows,
            bit_depth,
            seq.enable_intra_edge_filter,
            header.allow_screen_content_tools,
            seq.enable_filter_intra,
            (col_start as usize * 4, row_start as usize * 4),
            header,
            quantizer_steps,
            subsampling,
        )?;

        let mut trav = super::partition_decode::PartitionTraversal::new(
            mi_rows as u32,
            mi_cols as u32,
            (row_start, row_end),
            (col_start, col_end),
            seq.use_128x128_superblock,
        )?;

        while trav.decode_next(&mut block_decoder)? {}
        drop(block_decoder);
        dec.finish()?;
    }

    let mut frame = recon.finish()?;
    super::filters::apply_in_loop_filters(
        &mut frame,
        &header.loop_filter,
        &header.cdef,
        &header.restoration,
    )?;
    *state = next;
    Ok(frame)
}

#[cfg(test)]
mod geometry_tests {
    use super::supports_8x8_intra_tools;
    use super::BlockSize;

    #[test]
    fn intra_8x8_tools_reject_one_narrow_dimension() {
        for block in [
            BlockSize::Block4x4,
            BlockSize::Block4x8,
            BlockSize::Block8x4,
            BlockSize::Block4x16,
            BlockSize::Block16x4,
        ] {
            assert!(!supports_8x8_intra_tools(block), "{block:?}");
        }
        for block in [
            BlockSize::Block8x8,
            BlockSize::Block8x16,
            BlockSize::Block16x8,
            BlockSize::Block8x32,
            BlockSize::Block32x8,
            BlockSize::Block64x64,
        ] {
            assert!(supports_8x8_intra_tools(block), "{block:?}");
        }
    }
}
