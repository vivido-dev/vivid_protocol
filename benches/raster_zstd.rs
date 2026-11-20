//! Raster frame zstd encode and decode through the public media API.
//!
//! Runs natively and on `wasm32-wasip1` under wasmtime. Native runs export
//! their compressed frames to `target/bench-fixtures/raster_zstd/`. A build
//! that cannot compress rasters skips the encode benches and decodes those
//! exported frames instead.

use std::fs;
use std::hint::black_box;
use std::io;
use std::path::PathBuf;

use criterion::{BenchmarkId, Criterion, Throughput, criterion_group, criterion_main};
use vivid_protocol::media;

/// A small window or overlay, a 720p surface, and a 1080p surface.
const SIZES: &[(&str, u32, u32)] = &[
    ("320x240", 320, 240),
    ("1280x720", 1280, 720),
    ("1920x1080", 1920, 1080),
];

const FIXTURE_DIR: &str = "target/bench-fixtures/raster_zstd";

const GLYPH_W: usize = 8;
const GLYPH_H: usize = 14;
const LINE_H: usize = 16;
const TITLE_H: usize = 22;

/// SplitMix64 with a fixed seed, identical on every target.
struct Rng(u64);

impl Rng {
    fn next_u64(&mut self) -> u64 {
        self.0 = self.0.wrapping_add(0x9e37_79b9_7f4a_7c15);
        let mut z = self.0;
        z = (z ^ (z >> 30)).wrapping_mul(0xbf58_476d_1ce4_e5b9);
        z = (z ^ (z >> 27)).wrapping_mul(0x94d0_49bb_1331_11eb);
        z ^ (z >> 31)
    }

    fn below(&mut self, n: usize) -> usize {
        (self.next_u64() % n as u64) as usize
    }

    fn jitter(&mut self, amplitude: i32) -> i32 {
        self.below((2 * amplitude + 1) as usize) as i32 - amplitude
    }
}

#[derive(Clone, Copy)]
struct Rect {
    x0: usize,
    y0: usize,
    x1: usize,
    y1: usize,
}

impl Rect {
    /// A rectangle in percent of the frame size.
    fn percent(w: usize, h: usize, x0: usize, y0: usize, x1: usize, y1: usize) -> Self {
        Self {
            x0: w * x0 / 100,
            y0: h * y0 / 100,
            x1: w * x1 / 100,
            y1: h * y1 / 100,
        }
    }
}

struct Canvas {
    w: usize,
    px: Vec<u8>,
}

impl Canvas {
    fn put(&mut self, x: usize, y: usize, rgb: [i32; 3]) {
        let at = (y * self.w + x) * 4;
        for (dst, value) in self.px[at..at + 3].iter_mut().zip(rgb) {
            *dst = value.clamp(0, 255) as u8;
        }
        self.px[at + 3] = 255;
    }

    fn fill(&mut self, r: Rect, rgb: [i32; 3]) {
        for y in r.y0..r.y1 {
            for x in r.x0..r.x1 {
                self.put(x, y, rgb);
            }
        }
    }
}

/// Glyph coverage bitmaps with a blank right column and bottom rows, so
/// text is a mix of exact repeats (LZ matches) and anti-aliased edges.
fn font(rng: &mut Rng) -> Vec<[u8; GLYPH_W * GLYPH_H]> {
    (0..96)
        .map(|_| {
            let mut glyph = [0_u8; GLYPH_W * GLYPH_H];
            for y in 2..GLYPH_H - 2 {
                for x in 1..GLYPH_W - 1 {
                    glyph[y * GLYPH_W + x] = match rng.below(10) {
                        0..=1 => 255,
                        2 => 160,
                        3 => 80,
                        _ => 0,
                    };
                }
            }
            glyph
        })
        .collect()
}

/// Ragged lines of words, like a terminal or a document.
fn text(
    canvas: &mut Canvas,
    rng: &mut Rng,
    font: &[[u8; GLYPH_W * GLYPH_H]],
    area: Rect,
    bg: [i32; 3],
    palette: &[[i32; 3]],
) {
    let mut y = area.y0 + 4;
    while y + GLYPH_H <= area.y1 {
        if rng.below(6) == 0 {
            y += LINE_H;
            continue;
        }
        let mut x = area.x0 + 6 + rng.below(4) * 2 * GLYPH_W;
        let end = area.x0 + (area.x1 - area.x0) * (35 + rng.below(65)) / 100;
        while x + GLYPH_W <= end {
            let fg = palette[if rng.below(4) == 0 {
                rng.below(palette.len())
            } else {
                0
            }];
            for _ in 0..1 + rng.below(9) {
                if x + GLYPH_W > end {
                    break;
                }
                let glyph = &font[rng.below(font.len())];
                for gy in 0..GLYPH_H {
                    for gx in 0..GLYPH_W {
                        let c = i32::from(glyph[gy * GLYPH_W + gx]);
                        if c != 0 {
                            let rgb = [0, 1, 2].map(|i| (bg[i] * (255 - c) + fg[i] * c) / 255);
                            canvas.put(x + gx, y + gy, rgb);
                        }
                    }
                }
                x += GLYPH_W;
            }
            x += GLYPH_W;
        }
        y += LINE_H;
    }
}

/// Value noise in two octaves plus per-pixel grain, like a photo.
fn photo(canvas: &mut Canvas, rng: &mut Rng, area: Rect) {
    const CELL: usize = 32;
    let gw = (area.x1 - area.x0) / CELL + 2;
    let gh = (area.y1 - area.y0) / CELL + 2;
    let grid: Vec<[i32; 3]> = (0..gw * gh)
        .map(|_| [0, 1, 2].map(|_| 40 + rng.below(180) as i32))
        .collect();
    for y in area.y0..area.y1 {
        for x in area.x0..area.x1 {
            let (lx, ly) = (x - area.x0, y - area.y0);
            let (cx, cy) = (lx / CELL, ly / CELL);
            let (fx, fy) = ((lx % CELL) as i32, (ly % CELL) as i32);
            let at = |gx: usize, gy: usize| grid[gy * gw + gx];
            let (a, b, c, d) = (
                at(cx, cy),
                at(cx + 1, cy),
                at(cx, cy + 1),
                at(cx + 1, cy + 1),
            );
            let grain = rng.jitter(3);
            let detail = ((lx / 8 + ly / 8) % 3) as i32 * 4 - 4;
            let rgb = [0, 1, 2].map(|i| {
                let top = a[i] * (CELL as i32 - fx) + b[i] * fx;
                let bottom = c[i] * (CELL as i32 - fx) + d[i] * fx;
                (top * (CELL as i32 - fy) + bottom * fy) / (CELL * CELL) as i32 + detail + grain
            });
            canvas.put(x, y, rgb);
        }
    }
}

/// A desktop-like opaque RGBA8 frame: dithered gradient wallpaper, a
/// terminal, a document window with buttons, and a photo viewer.
fn synthetic_frame(width: u32, height: u32) -> Vec<u8> {
    let (w, h) = (width as usize, height as usize);
    let mut rng = Rng(0x7669_7669_6400_0000 ^ (u64::from(width) << 16) ^ u64::from(height));
    let font = font(&mut rng);
    let mut canvas = Canvas {
        w,
        px: vec![0; w * h * 4],
    };

    for y in 0..h {
        for x in 0..w {
            let t = ((x * 255 / w + y * 255 / h) / 2) as i32;
            let dither = match rng.below(8) {
                0 => -1,
                1 => 1,
                _ => 0,
            };
            canvas.put(
                x,
                y,
                [30 + t / 3, 60 + t / 2, 110 + t / 2].map(|v| v + dither),
            );
        }
    }

    let windows = [
        Rect::percent(w, h, 3, 6, 56, 94),
        Rect::percent(w, h, 48, 4, 97, 58),
        Rect::percent(w, h, 52, 62, 96, 96),
    ];
    for window in windows {
        let title = Rect {
            y1: (window.y0 + TITLE_H).min(window.y1),
            ..window
        };
        canvas.fill(title, [60, 63, 70]);
        for (i, rgb) in [[237, 106, 94], [245, 191, 79], [98, 197, 84]]
            .iter()
            .enumerate()
        {
            let x0 = window.x0 + 8 + i * 18;
            canvas.fill(
                Rect {
                    x0,
                    y0: title.y0 + 6,
                    x1: x0 + 10,
                    y1: title.y0 + 16,
                },
                *rgb,
            );
        }
    }

    let [terminal, document, viewer] = windows.map(|r| Rect {
        y0: (r.y0 + TITLE_H).min(r.y1),
        ..r
    });
    canvas.fill(terminal, [24, 24, 28]);
    text(
        &mut canvas,
        &mut rng,
        &font,
        terminal,
        [24, 24, 28],
        &[
            [204, 204, 204],
            [106, 153, 85],
            [86, 156, 214],
            [206, 145, 120],
        ],
    );
    canvas.fill(document, [250, 250, 250]);
    let toolbar = Rect {
        y1: (document.y0 + 30).min(document.y1),
        ..document
    };
    canvas.fill(toolbar, [236, 236, 240]);
    for i in 0..4 {
        let x0 = document.x0 + 8 + i * 70;
        if x0 + 60 <= document.x1 {
            canvas.fill(
                Rect {
                    x0,
                    y0: toolbar.y0 + 5,
                    x1: x0 + 60,
                    y1: toolbar.y1 - 5,
                },
                [66, 133, 244],
            );
        }
    }
    let page = Rect {
        y0: toolbar.y1,
        ..document
    };
    text(
        &mut canvas,
        &mut rng,
        &font,
        page,
        [250, 250, 250],
        &[[32, 33, 36], [26, 115, 232], [95, 99, 104]],
    );
    photo(&mut canvas, &mut rng, viewer);
    canvas.px
}

fn fixture_path(name: &str) -> PathBuf {
    PathBuf::from(FIXTURE_DIR).join(format!("{name}.bin"))
}

/// Saves a natively compressed frame for builds that cannot compress.
fn export_fixture(name: &str, body: &[u8]) {
    if cfg!(target_arch = "wasm32") {
        return;
    }
    let path = fixture_path(name);
    let written = fs::create_dir_all(FIXTURE_DIR).and_then(|()| fs::write(&path, body));
    if let Err(error) = written {
        eprintln!("raster_zstd: cannot write {}: {error}", path.display());
    }
}

fn bench_raster_zstd(c: &mut Criterion) {
    let mut group = c.benchmark_group("raster_zstd");
    group.sample_size(30);

    for &(name, width, height) in SIZES {
        let rgba = synthetic_frame(width, height);
        group.throughput(Throughput::Bytes(rgba.len() as u64));

        let (body, source) =
            match media::raster_frame_body_with_compression(1, 1, width, height, &rgba, true) {
                Ok(body) => {
                    export_fixture(name, &body);
                    (body, "this build")
                }
                Err(error) if error.kind() == io::ErrorKind::Unsupported => {
                    eprintln!("raster_zstd/encode/{name}: skipped ({error})");
                    let path = fixture_path(name);
                    match fs::read(&path) {
                        Ok(body) => (body, "native fixture"),
                        Err(error) => {
                            eprintln!(
                                "raster_zstd/decode/{name}: skipped, cannot read {} ({error}); \
                                 run the native bench first",
                                path.display()
                            );
                            continue;
                        }
                    }
                }
                Err(error) => panic!("raster encode failed: {error}"),
            };

        let parsed = media::parse_full_raster_frame(&body).expect("valid raster frame");
        assert!(parsed.compressed, "raster frame is not compressed");
        let compressed = parsed.pixels.len();
        let decoded = media::decode_raster_pixels(parsed).expect("decodable raster frame");
        assert!(
            decoded == rgba,
            "decoded {name} frame differs from the source; stale fixture?"
        );
        eprintln!(
            "raster_zstd {name}: {} -> {compressed} bytes ({:.2}x), decode input from {source}",
            rgba.len(),
            rgba.len() as f64 / compressed as f64,
        );

        if source == "this build" {
            group.bench_with_input(BenchmarkId::new("encode", name), &rgba, |b, rgba| {
                b.iter(|| {
                    media::raster_frame_body_with_compression(
                        1,
                        1,
                        width,
                        height,
                        black_box(rgba),
                        true,
                    )
                    .unwrap()
                });
            });
        }
        group.bench_with_input(BenchmarkId::new("decode", name), &body, |b, body| {
            b.iter(|| {
                let parsed = media::parse_full_raster_frame(black_box(body)).unwrap();
                media::decode_raster_pixels(parsed).unwrap()
            });
        });
    }

    group.finish();
}

criterion_group!(benches, bench_raster_zstd);
criterion_main!(benches);
