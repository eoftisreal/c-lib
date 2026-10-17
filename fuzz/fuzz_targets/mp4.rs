#![no_main]
use libfuzzer_sys::fuzz_target;
fuzz_target!(|data: &[u8]| {
    let mut demux = container_mp4::Mp4Demuxer::new(data);
    if demux.parse_moov().is_ok() {
        for index in 0..demux.tracks().len() {
            let count = demux.tracks()[index].stsz.len();
            for sample in [0, count / 2, count.saturating_sub(1), count] {
                let _ = demux.get_sample(index, sample);
            }
        }
    }
});
