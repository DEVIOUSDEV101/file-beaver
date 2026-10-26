//! Removes the white halo from a logo exported against a white background: edge pixels
//! are colour blended with white (c = a*fg + (1-a)*255), so solve for the true colour.
//! usage: defringe <in.png> <out.png>

fn main() {
    let args: Vec<String> = std::env::args().collect();
    let mut img = image::open(&args[1]).expect("open").to_rgba8();
    let (mut fixed, mut total) = (0u32, 0u32);
    for p in img.pixels_mut() {
        let a = p[3] as f32 / 255.0;
        if p[3] == 0 {
            continue;
        }
        total += 1;
        if p[3] < 255 {
            for c in 0..3 {
                let v = (p[c] as f32 - 255.0 * (1.0 - a)) / a;
                p[c] = v.round().clamp(0.0, 255.0) as u8;
            }
            fixed += 1;
        }
    }
    img.save(&args[2]).expect("save");
    println!("un-blended {fixed} of {total} visible pixels");
}
