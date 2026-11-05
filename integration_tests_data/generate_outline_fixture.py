"""Generate an entirely synthetic, two-page outline fixture with PyMuPDF."""

from pathlib import Path

import pymupdf


document = pymupdf.open()
for title in ("First page", "Second page"):
    page = document.new_page()
    page.insert_text((72, 72), title)

document.set_toc(
    [[1, "Chapter 1", 1], [2, "Section 1.1", 1], [1, "Chapter 2", 2]]
)
document.save(Path(__file__).with_name("outline.pdf"))
document.set_toc([])
document.save(Path(__file__).with_name("no_outline.pdf"))
document.close()
