use codec_av1::frame::{
    partition::{BlockSize, PartitionType},
    partition_decode::{BlockGeometry, PartitionRequest, PartitionSyntax, PartitionTraversal},
};
use codec_core::{CodecError, CodecResult};

struct Script<F> {
    choose: F,
    requests: Vec<PartitionRequest>,
    leaves: Vec<BlockGeometry>,
}
impl<F: FnMut(PartitionRequest) -> PartitionType> PartitionSyntax for Script<F> {
    fn read_partition(&mut self, r: PartitionRequest) -> CodecResult<PartitionType> {
        self.requests.push(r);
        Ok((self.choose)(r))
    }
    fn decode_leaf(&mut self, b: BlockGeometry) -> CodecResult<()> {
        self.leaves.push(b);
        Ok(())
    }
}
fn script<F: FnMut(PartitionRequest) -> PartitionType>(choose: F) -> Script<F> {
    Script {
        choose,
        requests: vec![],
        leaves: vec![],
    }
}

#[test]
fn all_ten_shapes_have_specified_order_and_geometry() {
    use PartitionType::*;
    type Geometry = (u32, u32, u32, u32);
    let cases: &[(PartitionType, &[Geometry])] = &[
        (None, &[(0, 0, 64, 64)]),
        (Horizontal, &[(0, 0, 64, 32), (8, 0, 64, 32)]),
        (Vertical, &[(0, 0, 32, 64), (0, 8, 32, 64)]),
        (
            Split,
            &[
                (0, 0, 32, 32),
                (0, 8, 32, 32),
                (8, 0, 32, 32),
                (8, 8, 32, 32),
            ],
        ),
        (HorzA, &[(0, 0, 32, 32), (0, 8, 32, 32), (8, 0, 64, 32)]),
        (HorzB, &[(0, 0, 64, 32), (8, 0, 32, 32), (8, 8, 32, 32)]),
        (VertA, &[(0, 0, 32, 32), (8, 0, 32, 32), (0, 8, 32, 64)]),
        (VertB, &[(0, 0, 32, 64), (0, 8, 32, 32), (8, 8, 32, 32)]),
        (
            Horz4,
            &[
                (0, 0, 64, 16),
                (4, 0, 64, 16),
                (8, 0, 64, 16),
                (12, 0, 64, 16),
            ],
        ),
        (
            Vert4,
            &[
                (0, 0, 16, 64),
                (0, 4, 16, 64),
                (0, 8, 16, 64),
                (0, 12, 16, 64),
            ],
        ),
    ];
    for &(partition, expected) in cases {
        let mut s = script(|r| {
            if r.block.bsize == BlockSize::Block64x64 {
                partition
            } else {
                None
            }
        });
        let mut tile = PartitionTraversal::new(16, 16, (0, 16), (0, 16), false).unwrap();
        assert!(tile.decode_next(&mut s).unwrap());
        assert!(!tile.decode_next(&mut s).unwrap());
        let actual: Vec<_> = s
            .leaves
            .iter()
            .map(|b| {
                let (w, h) = b.bsize.dimensions_px();
                (b.mi_row, b.mi_col, w, h)
            })
            .collect();
        assert_eq!(actual, expected, "{partition:?}");
    }
}

#[test]
fn recursive_64_and_128_cover_every_edge_unit_exactly_once() {
    for large in [false, true] {
        for rows in (2..=34).step_by(2) {
            for cols in (2..=34).step_by(2) {
                for force_split in [false, true] {
                    let mut tile =
                        PartitionTraversal::new(rows, cols, (0, rows), (0, cols), large).unwrap();
                    let mut s = script(|r| {
                        if force_split {
                            PartitionType::Split
                        } else if !r.has_rows {
                            PartitionType::Horizontal
                        } else if !r.has_cols {
                            PartitionType::Vertical
                        } else {
                            PartitionType::None
                        }
                    });
                    while tile.decode_next(&mut s).unwrap() {}
                    let mut coverage = vec![0u8; (rows * cols) as usize];
                    for b in s.leaves {
                        let (w, h) = b.bsize.dimensions_px();
                        assert!(b.mi_row < rows && b.mi_col < cols);
                        for y in b.mi_row..(b.mi_row + h / 4).min(rows) {
                            for x in b.mi_col..(b.mi_col + w / 4).min(cols) {
                                coverage[(y * cols + x) as usize] += 1;
                            }
                        }
                        if force_split {
                            assert_eq!(b.bsize, BlockSize::Block4x4);
                        }
                    }
                    assert!(
                        coverage.iter().all(|&n| n == 1),
                        "{rows}x{cols} large={large}"
                    );
                }
            }
        }
    }
}

#[test]
fn contexts_follow_prior_decoded_geometry_and_reset_at_tile_start() {
    // Four superblocks: the first splits to 4x4, the others remain whole.
    let mut tile = PartitionTraversal::new(32, 32, (0, 32), (0, 32), false).unwrap();
    let mut s = script(|r| {
        if r.block.mi_row < 16 && r.block.mi_col < 16 {
            PartitionType::Split
        } else {
            PartitionType::None
        }
    });
    while tile.decode_next(&mut s).unwrap() {}
    let roots: Vec<_> = s
        .requests
        .iter()
        .filter(|r| r.block.bsize == BlockSize::Block64x64)
        .map(|r| r.context)
        .collect();
    assert_eq!(roots, [0, 2, 1, 0]);
    let mut tile = PartitionTraversal::new(32, 32, (16, 32), (16, 32), false).unwrap();
    let mut s = script(|_| PartitionType::None);
    tile.decode_next(&mut s).unwrap();
    assert_eq!(s.requests[0].context, 0);
}

#[test]
fn illegal_shapes_and_callback_errors_poison_the_tile() {
    let mut tile = PartitionTraversal::new(32, 32, (0, 32), (0, 32), true).unwrap();
    let mut s = script(|_| PartitionType::Horz4); // Not valid for 128x128.
    assert_eq!(tile.decode_next(&mut s), Err(CodecError::InvalidData));
    assert!(s.leaves.is_empty());
    assert!(tile.decode_next(&mut s).is_err());
    assert_eq!(s.requests.len(), 1);
    struct Failure;
    impl PartitionSyntax for Failure {
        fn read_partition(&mut self, _: PartitionRequest) -> CodecResult<PartitionType> {
            Ok(PartitionType::None)
        }
        fn decode_leaf(&mut self, _: BlockGeometry) -> CodecResult<()> {
            Err(CodecError::Unsupported)
        }
    }
    let mut tile = PartitionTraversal::new(16, 16, (0, 16), (0, 16), false).unwrap();
    assert_eq!(tile.decode_next(&mut Failure), Err(CodecError::Unsupported));
    assert_eq!(tile.decode_next(&mut Failure), Err(CodecError::InvalidData));
    for args in [
        (0, 16, (0, 16), (0, 16)),
        (3, 16, (0, 3), (0, 16)),
        (32, 32, (1, 32), (0, 32)),
        (32, 32, (0, 8), (0, 32)),
        (32, 32, (0, 32), (0, 34)),
    ] {
        assert!(PartitionTraversal::new(args.0, args.1, args.2, args.3, false).is_err());
    }
    for (width, count) in [(8, 4), (16, 10), (32, 10), (64, 10), (128, 8)] {
        let r = PartitionRequest {
            block: BlockGeometry {
                mi_row: 0,
                mi_col: 0,
                bsize: BlockSize::from_dimensions_px(width, width).unwrap(),
            },
            context: 0,
            has_rows: true,
            has_cols: true,
        };
        for symbol in 0..10 {
            assert_eq!(
                r.allows(PartitionType::try_from(symbol).unwrap()),
                symbol < count
            );
        }
    }
    assert!(PartitionType::try_from(10).is_err());
    assert!(BlockSize::from_dimensions_px(128, 32).is_err());
}
