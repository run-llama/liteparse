"""Regenerate the high-contrast OCR fixture.

Run: uv run --with pillow scripts/generate_macos_ocr_fixture.py

The checked-in PNG makes recognition tests independent of installed fonts.
Arial is supplied by macOS; no document or image from a third party is used.
"""

from pathlib import Path

from PIL import Image, ImageDraw, ImageFont

image = Image.new("RGB", (1200, 720), "white")
draw = ImageDraw.Draw(image)
font = ImageFont.truetype("/System/Library/Fonts/Supplemental/Arial.ttf", 64)
for y, text in [
    (72, "Native Vision OCR"),
    (290, "Invoice 12345"),
    (500, "Café résumé!"),
    (620, "Total: $1,234.56"),
]:
    draw.text((80, y), text, font=font, fill="black", anchor="lt")
image.save(
    Path(__file__).resolve().parents[1] / "integration_tests_data/macos_ocr.png",
    dpi=(150, 150),
)
