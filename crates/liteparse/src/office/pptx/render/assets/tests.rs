use super::*;
use crate::office::pptx::{
    config::Limits,
    error::DocError,
    fonts::FontSystem,
    package::Package,
    render::{ImageCache, RenderTarget, decoded_bytes_estimate, render},
    scene::{ImageItem, ImageRef, Item, Rect, Scene},
};
use std::{cell::Cell, collections::HashMap, io::Write, sync::Arc};
use tiny_skia::{Pixmap, Transform};

struct Source {
    bytes: Vec<u8>,
    reads: Cell<usize>,
    heads: Cell<usize>,
}

impl RenderAssets for Source {
    fn read(&self, part: &str) -> DocResult<Vec<u8>> {
        self.reads.set(self.reads.get() + 1);
        if part != "source-image" {
            return Err(DocError::Broken("missing source image".into()));
        }
        Ok(self.bytes.clone())
    }

    fn read_head(&self, part: &str, max: u64) -> DocResult<Vec<u8>> {
        self.heads.set(self.heads.get() + 1);
        if part != "source-image" {
            return Err(DocError::Broken("missing source image".into()));
        }
        Ok(self.bytes[..self.bytes.len().min(max as usize)].to_vec())
    }

    fn part_size(&self, part: &str) -> Option<u64> {
        (part == "source-image").then_some(self.bytes.len() as u64)
    }
}

fn source() -> Source {
    let mut pixels = Pixmap::new(2, 2).unwrap();
    pixels.fill(tiny_skia::Color::from_rgba8(240, 20, 30, 255));
    Source {
        bytes: pixels.encode_png().unwrap(),
        reads: Cell::new(0),
        heads: Cell::new(0),
    }
}

fn package(source: &Source) -> Document {
    let mut archive = zip::ZipWriter::new(std::io::Cursor::new(Vec::new()));
    archive
        .start_file("source-image", zip::write::SimpleFileOptions::default())
        .unwrap();
    archive.write_all(&source.bytes).unwrap();
    let bytes = archive.finish().unwrap().into_inner();
    Document::from_package(
        Arc::new(Package::open(bytes, &Limits::default()).unwrap()),
        127000,
        127000,
    )
}

#[test]
fn source_owned_image_renders_identically_to_package_image() {
    let source = source();
    let document = package(&source);
    let fonts = FontSystem::new(&[], false, &HashMap::new());
    let scene = Scene {
        width: 10.0,
        height: 10.0,
        items: vec![Item::Image(ImageItem {
            image: ImageRef {
                part: "source-image".into(),
                effects: vec![],
            },
            transform: Transform::from_scale(10.0, 10.0),
            crop: (0.0, 0.0, 0.0, 0.0),
            alpha: 1.0,
            clip: None,
            shape: None,
            alt_text: None,
        })],
        ..Default::default()
    };
    let target = RenderTarget {
        region: Rect {
            x: 0.0,
            y: 0.0,
            w: 10.0,
            h: 10.0,
        },
        width: 10,
        height: 10,
    };
    let images = ImageCache::new(1000, &fonts);
    let native = render(&scene, &target, &source, &fonts, &images).unwrap();
    let packaged = render(
        &scene,
        &target,
        &document,
        &fonts,
        &ImageCache::new(1000, &fonts),
    )
    .unwrap();
    assert_eq!(native.data(), packaged.data());
    assert_eq!(&native.data()[220..224], &[240, 20, 30, 255]);
    assert_eq!(images.native_size(&source, "source-image"), Some((2, 2)));
    assert_eq!(source.reads.get(), 1);
    assert!(images.store().failures().is_empty());
}

#[test]
fn source_read_errors_remain_visible_and_header_estimation_stays_lazy() {
    let source = source();
    let fonts = FontSystem::new(&[], false, &HashMap::new());
    let images = ImageCache::new(1000, &fonts);
    assert_eq!(decoded_bytes_estimate(&source, "source-image"), 16);
    assert_eq!(source.heads.get(), 1);
    assert_eq!(source.reads.get(), 0);
    assert_eq!(source.read_head("source-image", 4).unwrap().len(), 4);
    assert!(images.get_part(&source, "missing").is_none());
    let failures = images.store().failures();
    assert_eq!(failures.len(), 1);
    assert_eq!(failures[0].0, "missing");
    assert!(failures[0].1.contains("missing source image"));
    assert!(images.get_part(&source, "missing").is_none());
    assert_eq!(source.reads.get(), 1);
}

#[test]
fn oversized_unknown_image_estimate_saturates() {
    struct Large;
    impl RenderAssets for Large {
        fn read(&self, _: &str) -> DocResult<Vec<u8>> {
            panic!("estimation must use bounded header reads")
        }
        fn read_head(&self, _: &str, _: u64) -> DocResult<Vec<u8>> {
            Ok(vec![])
        }
        fn part_size(&self, _: &str) -> Option<u64> {
            Some(u64::MAX)
        }
    }
    assert_eq!(decoded_bytes_estimate(&Large, "large"), u64::MAX);
}
