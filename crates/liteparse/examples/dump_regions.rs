//! Print a page's XY-cut region tree: each split's axis, each leaf's box and its first lines.
//! Usage: dump_regions <pdf> [page]   (OCR off; for diagnosing column interleave)
use liteparse::config::{LiteParseConfig, OutputFormat};
use liteparse::parser::LiteParse;
use liteparse::types::{PdfInput, ProjectedLine, Region, RegionKind};

fn walk(r: &Region, path: &mut Vec<u16>, lines: &[ProjectedLine], depth: usize) {
    let b = &r.bbox;
    let pad = "  ".repeat(depth);
    match &r.kind {
        RegionKind::Split { axis, children } => {
            println!(
                "{pad}split {:?} [{:.0},{:.0} {:.0}x{:.0}] {} children",
                axis,
                b.x,
                b.y,
                b.width,
                b.height,
                children.len()
            );
            for (i, c) in children.iter().enumerate() {
                path.push(i as u16);
                walk(c, path, lines, depth + 1);
                path.pop();
            }
        }
        RegionKind::Leaf { item_indices } => {
            let mine: Vec<&ProjectedLine> =
                lines.iter().filter(|l| l.region_path == *path).collect();
            println!(
                "{pad}leaf {:?} [{:.0},{:.0} {:.0}x{:.0}] items={} lines={}",
                path,
                b.x,
                b.y,
                b.width,
                b.height,
                item_indices.len(),
                mine.len()
            );
            for l in mine.iter().take(3) {
                let t: String = l.text.trim().chars().take(90).collect();
                println!("{pad}  | {t}");
            }
        }
    }
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    let path = std::env::args()
        .nth(1)
        .ok_or("usage: dump_regions <pdf> [page]")?;
    let page_num: usize = std::env::args()
        .nth(2)
        .and_then(|s| s.parse().ok())
        .unwrap_or(1);
    let cfg = LiteParseConfig {
        output_format: OutputFormat::Markdown,
        ocr_enabled: false,
        quiet: true,
        ..Default::default()
    };
    let r = LiteParse::new(cfg)
        .parse_input(PdfInput::Path(path))
        .await?;
    let p = &r.pages[page_num - 1];
    walk(&p.regions, &mut Vec::new(), &p.projected_lines, 0);
    Ok(())
}
