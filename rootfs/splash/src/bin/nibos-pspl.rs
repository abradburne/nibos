//! nibos-pspl: a picture for nibos-splash, made at build time from a PNG.
//!
//!   nibos-pspl OUT.pspl TOP.png [WIDTH HEIGHT]
//!
//! The PNG goes in the middle of the top screen, on black; the bottom screen is black, as
//! DSlot's own picture is (`dslot --write-splash`). Screens 1024 by 768 unless given. The
//! format is nibos-splash's (src/main.rs): "PSPL", version 1, a count, then per screen, top
//! first, width, height, a byte count and an LZ4 block of XRGB8888 rows.

use std::{fs::File, process::exit};

fn fail(msg: impl std::fmt::Display) -> ! {
    eprintln!("nibos-pspl: {msg}");
    exit(1);
}

fn main() {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let (out, top) = match &args[..] {
        [out, top] | [out, top, _, _] => (out, top),
        _ => fail("usage: nibos-pspl OUT.pspl TOP.png [WIDTH HEIGHT]"),
    };
    let (w, h) = match &args[..] {
        [_, _, w, h] => (
            w.parse().unwrap_or_else(|_| fail("WIDTH is not a number")),
            h.parse().unwrap_or_else(|_| fail("HEIGHT is not a number")),
        ),
        _ => (1024usize, 768usize),
    };

    let mut decoder =
        png::Decoder::new(File::open(top).unwrap_or_else(|e| fail(format!("{top}: {e}"))));
    decoder.set_transformations(png::Transformations::EXPAND | png::Transformations::STRIP_16);
    let mut reader = decoder
        .read_info()
        .unwrap_or_else(|e| fail(format!("{top}: {e}")));
    let mut buf = vec![0; reader.output_buffer_size()];
    let info = reader
        .next_frame(&mut buf)
        .unwrap_or_else(|e| fail(format!("{top}: {e}")));
    let (pw, ph) = (info.width as usize, info.height as usize);
    if pw > w || ph > h {
        fail(format!(
            "{top} is {pw}x{ph}, larger than the {w}x{h} screen"
        ));
    }
    let channels = info.color_type.samples();
    let rgb = |x: usize, y: usize| -> [u8; 3] {
        let p = &buf[(y * pw + x) * channels..][..channels];
        match channels {
            1 | 2 => [p[0], p[0], p[0]],
            _ => [p[0], p[1], p[2]],
        }
    };

    // XRGB8888 is B, G, R, X in memory.
    let mut screen = vec![0u8; w * h * 4];
    let (x0, y0) = ((w - pw) / 2, (h - ph) / 2);
    for y in 0..ph {
        for x in 0..pw {
            let [r, g, b] = rgb(x, y);
            screen[((y0 + y) * w + x0 + x) * 4..][..4].copy_from_slice(&[b, g, r, 255]);
        }
    }
    let black = vec![0u8; w * h * 4];

    let mut file = Vec::new();
    file.extend_from_slice(b"PSPL");
    for v in [1u32, 2] {
        file.extend_from_slice(&v.to_le_bytes());
    }
    for px in [&screen, &black] {
        let block = lz4_flex::block::compress(px);
        for v in [w as u32, h as u32, block.len() as u32] {
            file.extend_from_slice(&v.to_le_bytes());
        }
        file.extend_from_slice(&block);
    }
    std::fs::write(out, &file).unwrap_or_else(|e| fail(format!("{out}: {e}")));
    println!("{out}: {top} ({pw}x{ph}) on {w}x{h}, {} bytes", file.len());
}
