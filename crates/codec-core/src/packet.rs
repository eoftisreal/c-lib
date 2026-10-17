use crate::time::Timestamp;

#[derive(Debug, Clone)]
pub struct Packet {
    pub data: Vec<u8>,
    pub pts: Option<Timestamp>,
    pub dts: Option<Timestamp>,
    pub is_keyframe: bool,
    pub duration: Option<u64>,
    pub stream_id: u32,
}
