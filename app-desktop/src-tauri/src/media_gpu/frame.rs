#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct AdapterId {
    pub high: i32,
    pub low: u32,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Matrix {
    Bt601,
    Bt709,
    Bt2020,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Range {
    Limited,
    Full,
}
#[derive(Clone, Copy, Debug)]
pub struct FrameMetadata {
    pub width: u32,
    pub height: u32,
    pub pts_100ns: i64,
    pub duration_100ns: i64,
    pub format: SurfaceFormat,
    pub adapter: AdapterId,
    pub matrix: Matrix,
    pub range: Range,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum SurfaceFormat {
    Nv12,
}
