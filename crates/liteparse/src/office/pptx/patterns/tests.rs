use super::*;

fn coverage(rows: [u8; 8]) -> u32 {
    rows.iter().map(|r| r.count_ones()).sum()
}

#[test]
fn percentage_presets_have_their_density() {
    for pct in [5, 10, 20, 25, 30, 40, 50, 60, 70, 75, 80, 90] {
        let n = coverage(tile(&format!("pct{pct}")));
        assert_eq!(n, (pct * 64 + 50) / 100, "pct{pct}");
    }
    assert_eq!(coverage(tile("pct50")), 32);
}

#[test]
fn every_preset_has_a_tile_with_both_colours() {
    let presets = [
        "horz",
        "vert",
        "ltHorz",
        "ltVert",
        "dkHorz",
        "dkVert",
        "narHorz",
        "narVert",
        "dashHorz",
        "dashVert",
        "cross",
        "lgGrid",
        "smGrid",
        "dotGrid",
        "dnDiag",
        "upDiag",
        "ltDnDiag",
        "ltUpDiag",
        "dkDnDiag",
        "dkUpDiag",
        "wdDnDiag",
        "wdUpDiag",
        "dashDnDiag",
        "dashUpDiag",
        "diagCross",
        "smCheck",
        "lgCheck",
        "smConfetti",
        "lgConfetti",
        "horzBrick",
        "diagBrick",
        "solidDmnd",
        "openDmnd",
        "dotDmnd",
        "plaid",
        "sphere",
        "weave",
        "divot",
        "shingle",
        "wave",
        "trellis",
        "zigZag",
    ];
    for p in presets {
        let n = coverage(tile(p));
        assert!(n > 0 && n < 64, "{p}: {n}");
    }
    assert_eq!(tile("horz")[0], 0xFF);
    assert_eq!(tile("vert")[5], 0x80);
    assert_eq!(coverage(tile("unknownPreset")), 32);
}
