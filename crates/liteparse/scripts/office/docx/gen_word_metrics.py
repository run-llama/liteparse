#!/usr/bin/env python3
"""Generates src/metrics/word_tables.rs: numeric metrics of Microsoft fonts that have
no metric-compatible open clone, so pagination can follow Word while glyphs are
drawn from an open substitute.

Only numbers are written: vertical metrics (OS/2 win and typo, hhea), underline
and strikeout metrics, advance widths per codepoint over common ranges, and
kerning pairs for Latin text. No outlines or font program data.

usage: gen_word_metrics.py <manifest.json> <out.rs>
manifest: [{"family": "Aptos", "bold": false, "italic": false,
            "path": "/local/font.ttf", "index": 0, "source": "where the file came from",
            "extra": "/local/codepoints.txt"}, ...]
"index" picks a face of a .ttc; "extra" lists further codepoints (hex, one per line) to
table, used to keep East Asian tables to the characters a corpus actually uses.
Alternative family names (localized name records) are emitted as aliases.
Font files stay local; they are never committed.
"""
import json, sys
from fontTools.ttLib import TTFont

RANGES = [(0x20, 0x24F), (0x370, 0x3FF), (0x400, 0x4FF), (0x2000, 0x206F), (0x20A0, 0x20CF),
          (0x2100, 0x215F), (0x2190, 0x21FF), (0x2200, 0x22FF), (0x2500, 0x25FF), (0x2600, 0x26FF),
          (0x2700, 0x27BF), (0xFB00, 0xFB06)]
# Kerning is kept for ASCII letters, digits and punctuation plus curly quotes and dashes.
KERN_CHARS = [c for c in range(0x21, 0x7F)] + [0x2018, 0x2019, 0x201C, 0x201D, 0x2013, 0x2014]

def gpos_kern(font, cmap):
    """Pair adjustments (first glyph, second glyph) -> x advance delta, from the
    GPOS 'kern' feature (latn/DFLT), else the legacy 'kern' table."""
    out = {}
    if "GPOS" in font:
        g = font["GPOS"].table
        idx = set()
        for fr in g.FeatureList.FeatureRecord:
            if fr.FeatureTag == "kern":
                idx.update(fr.Feature.LookupListIndex)
        for li in sorted(idx):
            lk = g.LookupList.Lookup[li]
            subs = lk.SubTable
            if lk.LookupType == 9:
                subs = [s.ExtSubTable for s in subs]
            for st in subs:
                if getattr(st, "LookupType", 2) != 2 and lk.LookupType not in (2, 9):
                    continue
                if not hasattr(st, "Format") or not hasattr(st, "Coverage"):
                    continue
                if not (hasattr(st, "PairSet") or hasattr(st, "Class1Record")):
                    continue
                cov = st.Coverage.glyphs
                if st.Format == 1:
                    for i, first in enumerate(cov):
                        for rec in st.PairSet[i].PairValueRecord:
                            v = getattr(rec.Value1, "XAdvance", 0) if rec.Value1 else 0
                            if v: out.setdefault((first, rec.SecondGlyph), v)
                elif st.Format == 2:
                    c1 = st.ClassDef1.classDefs; c2 = st.ClassDef2.classDefs
                    want = {cmap[c] for c in KERN_CHARS if c in cmap}
                    seconds = [gname for gname in want]
                    for first in cov:
                        if first not in want: continue
                        r1 = st.Class1Record[c1.get(first, 0)]
                        for second in seconds:
                            rec = r1.Class2Record[c2.get(second, 0)]
                            v = getattr(rec.Value1, "XAdvance", 0) if rec.Value1 else 0
                            if v: out.setdefault((first, second), v)
    if not out and "kern" in font:
        for t in font["kern"].kernTables:
            for (a, b), v in getattr(t, "kernTable", {}).items():
                out.setdefault((a, b), v)
    return out

def face(entry):
    f = TTFont(entry["path"], fontNumber=entry.get("index", 0), lazy=False)
    cmap = f.getBestCmap() or {}
    hmtx = f["hmtx"].metrics
    os2 = f["OS/2"]; hh = f["hhea"]; post = f["post"]
    ranges = []
    spans = list(RANGES)
    if entry.get("extra"):
        cps = sorted({int(l, 16) for l in open(entry["extra"]) if l.strip()})
        cps = [c for c in cps if not any(lo <= c <= hi for lo, hi in RANGES)]
        for c in cps:
            if spans and spans[-1][1] == c - 1 and spans[-1][0] >= 0x2E80:
                spans[-1] = (spans[-1][0], c)
            else:
                spans.append((c, c))
    for lo, hi in spans:
        run_start, run = None, []
        for cp in range(lo, hi + 1):
            g = cmap.get(cp)
            if g is None:
                if run:
                    ranges.append((run_start, run)); run_start, run = None, []
                continue
            if not run: run_start = cp
            run.append(hmtx[g][0])
        if run: ranges.append((run_start, run))
    ranges.sort()
    kern = []
    pairs = gpos_kern(f, cmap)
    for a in KERN_CHARS:
        ga = cmap.get(a)
        if ga is None: continue
        for b in KERN_CHARS:
            gb = cmap.get(b)
            if gb is None: continue
            v = pairs.get((ga, gb))
            if v: kern.append((a, b, v))
    name = f["name"]
    # Localized family names (ＭＳ ゴシック for MS Gothic). Only name ID 1 in languages
    # other than US English: name ID 16 is the typographic family ("Calibri" for
    # Calibri Light) and would capture the regular family.
    aliases = sorted({r.toUnicode() for r in name.names if r.nameID == 1
                      and not (r.platformID == 3 and r.langID == 0x409) and r.platformID != 1
                      and r.toUnicode().strip() and r.toUnicode().lower() != entry["family"].lower()})
    return dict(entry, version=name.getDebugName(5) or "", fullname=name.getDebugName(4) or "",
                upm=f["head"].unitsPerEm, use_typo=bool(os2.fsSelection & 0x80), win_ascent=os2.usWinAscent, win_descent=os2.usWinDescent,
                typo=(os2.sTypoAscender, os2.sTypoDescender, os2.sTypoLineGap),
                hhea=(hh.ascent, hh.descent, hh.lineGap),
                underline=(post.underlinePosition, post.underlineThickness),
                strike=(os2.yStrikeoutPosition, os2.yStrikeoutSize),
                ranges=ranges, kern=kern, aliases=aliases)

def main():
    manifest = json.load(open(sys.argv[1]))
    faces = [face(e) for e in manifest]
    w = open(sys.argv[2], "w")
    w.write("// @generated by scripts/gen_word_metrics.py. Do not edit.\n")
    w.write("// Numeric metrics only (vertical metrics, advance widths, kerning pairs),\n")
    w.write("// measured from these fonts; no outlines or font program data:\n")
    for fc in faces:
        style = ("Bold " if fc["bold"] else "") + ("Italic" if fc["italic"] else "") or "Regular"
        w.write(f"// - {fc['family']} {style.strip()}: {fc['fullname']}, {fc['version']}; {fc['source']}\n")
    w.write("\nuse super::WordFace;\n\n#[rustfmt::skip]\npub static FACES: &[WordFace] = &[\n")
    for fc in faces:
        w.write("    WordFace {\n")
        w.write(f"        family: {json.dumps(fc['family'])}, bold: {str(fc['bold']).lower()}, italic: {str(fc['italic']).lower()},\n")
        w.write(f"        aliases: &[{', '.join(json.dumps(a, ensure_ascii=False) for a in fc['aliases'])}],\n")
        w.write(f"        use_typo: {str(fc['use_typo']).lower()},\n")
        w.write(f"        upm: {fc['upm']}, win_ascent: {fc['win_ascent']}, win_descent: {fc['win_descent']},\n")
        w.write(f"        hhea: ({fc['hhea'][0]}, {fc['hhea'][1]}, {fc['hhea'][2]}), typo: ({fc['typo'][0]}, {fc['typo'][1]}, {fc['typo'][2]}),\n")
        w.write(f"        underline: ({fc['underline'][0]}, {fc['underline'][1]}), strikeout: ({fc['strike'][0]}, {fc['strike'][1]}),\n")
        w.write("        advances: &[\n")
        for start, run in fc["ranges"]:
            for j in range(0, len(run), 24):
                w.write(f"            (0x{start + j:04X}, &[{','.join(map(str, run[j:j + 24]))}]),\n")
        w.write("        ],\n        kern: &[\n")
        for i in range(0, len(fc["kern"]), 12):
            w.write("            " + " ".join(f"({a},{b},{v})," for a, b, v in fc["kern"][i:i + 12]) + "\n")
        w.write("        ],\n    },\n")
    w.write("];\n")

if __name__ == "__main__":
    main()
