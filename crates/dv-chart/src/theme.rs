use egui::Color32;

/// Colors and sizes shared by the on-screen and export renderers.
#[derive(Clone, Debug)]
pub struct ChartTheme {
    pub bg: Color32,
    pub surface: Color32,
    pub grid: Color32,
    pub axis_line: Color32,
    pub axis_text: Color32,
    pub text: Color32,
    pub text_muted: Color32,
    pub crosshair: Color32,
    pub selection_fill: Color32,
    pub selection_edge: Color32,
    pub legend_bg: Color32,
    pub font_size: f32,
    pub series_palette: [Color32; 10],
}

impl ChartTheme {
    pub fn dark() -> Self {
        Self {
            bg: rgb(0x111317),
            surface: rgb(0x1A1E24),
            grid: Color32::from_white_alpha(14),
            axis_line: rgb(0x343B45),
            axis_text: rgb(0x8C95A1),
            text: rgb(0xE7EAEE),
            text_muted: rgb(0x8994A2),
            crosshair: Color32::from_white_alpha(102),
            selection_fill: rgba(0x5B8CFF, 36),
            selection_edge: rgba(0x5B8CFF, 179),
            legend_bg: rgba(0x1A1E24, 226),
            font_size: 11.5,
            series_palette: [
                rgb(0x5B8CFF),
                rgb(0xFF9F43),
                rgb(0x2EC4B6),
                rgb(0xFF5C7A),
                rgb(0xA68BFF),
                rgb(0x7BD389),
                rgb(0xFFD166),
                rgb(0x5BC0EB),
                rgb(0xF78FB3),
                rgb(0xB8C1CC),
            ],
        }
    }
    pub fn light() -> Self {
        Self {
            bg: rgb(0xF5F6F8),
            surface: rgb(0xFFFFFF),
            grid: Color32::from_black_alpha(15),
            axis_line: rgb(0xCDD3DB),
            axis_text: rgb(0x6A7380),
            text: rgb(0x1A1E23),
            text_muted: rgb(0x687380),
            crosshair: Color32::from_black_alpha(89),
            selection_fill: rgba(0x2F6BFF, 26),
            selection_edge: rgba(0x2F6BFF, 179),
            legend_bg: rgba(0xFFFFFF, 230),
            font_size: 11.5,
            series_palette: [
                rgb(0x2F6BFF),
                rgb(0xE8800F),
                rgb(0x0FA394),
                rgb(0xE23B57),
                rgb(0x7552F2),
                rgb(0x2F9E52),
                rgb(0xC99700),
                rgb(0x1789C9),
                rgb(0xCF4F8C),
                rgb(0x6F7A87),
            ],
        }
    }
}
impl Default for ChartTheme {
    fn default() -> Self {
        Self::dark()
    }
}
fn rgb(value: u32) -> Color32 {
    Color32::from_rgb((value >> 16) as u8, (value >> 8) as u8, value as u8)
}
fn rgba(value: u32, alpha: u8) -> Color32 {
    Color32::from_rgba_unmultiplied((value >> 16) as u8, (value >> 8) as u8, value as u8, alpha)
}
