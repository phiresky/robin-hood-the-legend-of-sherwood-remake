//! Export a picture from a `.res` file to PNG for visual inspection.
//!
//!   cargo run --example res_pic_to_png -- <file.res> <id> <sub_id> <out.png>
#![deny(clippy::print_stdout, clippy::print_stderr)]
fn main() {
    tracing_subscriber::fmt::init();
    let args: Vec<String> = std::env::args().collect();
    let [_, path, id, sub_id, out] = &args[..] else {
        tracing::error!("usage: res_pic_to_png <file.res> <id> <sub_id> <out.png>");
        std::process::exit(2);
    };
    let mut mgr = robin_assets::resource_manager::ResourceManager::new();
    mgr.attach_resource_file(path).expect("attach res file");
    let pic = mgr
        .get_picture(id.parse().unwrap(), sub_id.parse().unwrap())
        .expect("get picture");
    let (w, h) = (pic.width as u32, pic.height as u32);
    let mut rgba = Vec::with_capacity((w * h * 4) as usize);
    for px in pic.data.chunks_exact(2) {
        let v = u16::from_le_bytes([px[0], px[1]]);
        let r = ((v >> 11) & 0x1f) as u8;
        let g = ((v >> 5) & 0x3f) as u8;
        let b = (v & 0x1f) as u8;
        rgba.extend_from_slice(&[r << 3, g << 2, b << 3, 255]);
    }
    let file = std::fs::File::create(out).expect("create png");
    let mut enc = png::Encoder::new(std::io::BufWriter::new(file), w, h);
    enc.set_color(png::ColorType::Rgba);
    enc.set_depth(png::BitDepth::Eight);
    enc.write_header().unwrap().write_image_data(&rgba).unwrap();
}
