#!/usr/bin/env bash
# Downloads the open font set pptx-read renders with when a deck's own fonts are
# not installed: metric-compatible stand-ins for common Office fonts plus popular
# free families. All licences allow commercial use (OFL, Apache-2.0, GUST).
# usage: fetch-fonts.sh <dest-dir>
set -euo pipefail
dest="${1:?usage: fetch-fonts.sh <dest-dir>}"
mkdir -p "$dest"
gf="https://github.com/google/fonts/raw/main"
ctan="https://mirrors.ctan.org/fonts/tex-gyre/opentype"

fetch() { # url file
  # Whole-call retries: mirrors.ctan.org redirects each call to a random mirror, and
  # curl's own --retry does not retry a mirror's TLS failure.
  [ -s "$dest/$2" ] && return
  for _ in 1 2 3 4 5; do curl -fsSL --retry 3 -o "$dest/$2" "$1" && return; sleep 2; done
  echo "failed: $1" >&2; rm -f "$dest/$2"
}

# Google Fonts (OFL unless noted). Variable fonts: pptx-read instances the wght axis
# (and wdth, for condensed requests). Noto Emoji is the monochrome emoji face; colour
# emoji fonts (bitmap or layered glyphs) do not draw. Ubuntu is not shipped: its
# Ubuntu Font Licence is not one of the licences above.
for spec in \
  "ofl/opensans/OpenSans[wdth,wght].ttf" "ofl/opensans/OpenSans-Italic[wdth,wght].ttf" \
  "ofl/roboto/Roboto[wdth,wght].ttf" "ofl/roboto/Roboto-Italic[wdth,wght].ttf" \
  "ofl/montserrat/Montserrat[wght].ttf" "ofl/montserrat/Montserrat-Italic[wght].ttf" \
  "ofl/lato/Lato-Regular.ttf" "ofl/lato/Lato-Bold.ttf" "ofl/lato/Lato-Italic.ttf" "ofl/lato/Lato-BoldItalic.ttf" "ofl/lato/Lato-Light.ttf" \
  "ofl/sourcesans3/SourceSans3[wght].ttf" "ofl/sourcesans3/SourceSans3-Italic[wght].ttf" \
  "ofl/poppins/Poppins-Regular.ttf" "ofl/poppins/Poppins-Bold.ttf" "ofl/poppins/Poppins-Italic.ttf" "ofl/poppins/Poppins-SemiBold.ttf" "ofl/poppins/Poppins-Light.ttf" \
  "ofl/nunito/Nunito[wght].ttf" "ofl/nunito/Nunito-Italic[wght].ttf" \
  "ofl/raleway/Raleway[wght].ttf" "ofl/raleway/Raleway-Italic[wght].ttf" \
  "ofl/librefranklin/LibreFranklin[wght].ttf" "ofl/librefranklin/LibreFranklin-Italic[wght].ttf" \
  "ofl/inconsolata/Inconsolata[wdth,wght].ttf" \
  "ofl/gelasio/Gelasio[wght].ttf" "ofl/gelasio/Gelasio-Italic[wght].ttf" \
  "ofl/notosans/NotoSans[wdth,wght].ttf" "ofl/notosans/NotoSans-Italic[wdth,wght].ttf" \
  "ofl/notoserif/NotoSerif[wdth,wght].ttf" \
  "ofl/notosanssymbols/NotoSansSymbols[wght].ttf" "ofl/notosanssymbols2/NotoSansSymbols2-Regular.ttf" "ofl/notosansmath/NotoSansMath-Regular.ttf" \
  "ofl/notosansarabic/NotoSansArabic[wdth,wght].ttf" "ofl/notosanshebrew/NotoSansHebrew[wdth,wght].ttf" \
  "ofl/notosansthai/NotoSansThai[wdth,wght].ttf" "ofl/notosansdevanagari/NotoSansDevanagari[wdth,wght].ttf" \
  "ofl/inter/Inter[opsz,wght].ttf" "ofl/inter/Inter-Italic[opsz,wght].ttf" \
  "ofl/arimo/Arimo[wght].ttf" "ofl/tinos/Tinos-Regular.ttf" "ofl/tinos/Tinos-Bold.ttf" "ofl/cousine/Cousine-Regular.ttf" \
  "ofl/carlito/Carlito-Regular.ttf" "ofl/carlito/Carlito-Bold.ttf" "ofl/carlito/Carlito-Italic.ttf" "ofl/carlito/Carlito-BoldItalic.ttf" \
  "ofl/cabin/Cabin[wdth,wght].ttf" "ofl/cabin/Cabin-Italic[wdth,wght].ttf" \
  "ofl/comicrelief/ComicRelief-Regular.ttf" "ofl/comicrelief/ComicRelief-Bold.ttf" \
  "ofl/archivoblack/ArchivoBlack-Regular.ttf" "ofl/anton/Anton-Regular.ttf" \
  "ofl/arvo/Arvo-Regular.ttf" "ofl/arvo/Arvo-Bold.ttf" "ofl/arvo/Arvo-Italic.ttf" "ofl/arvo/Arvo-BoldItalic.ttf" \
  "ofl/ebgaramond/EBGaramond[wght].ttf" "ofl/ebgaramond/EBGaramond-Italic[wght].ttf" \
  "ofl/ibmplexsans/IBMPlexSans[wdth,wght].ttf" "ofl/ibmplexsans/IBMPlexSans-Italic[wdth,wght].ttf" \
  "ofl/manrope/Manrope[wght].ttf" "ofl/figtree/Figtree[wght].ttf" "ofl/figtree/Figtree-Italic[wght].ttf" \
  "ofl/barlow/Barlow-Regular.ttf" "ofl/barlow/Barlow-Bold.ttf" "ofl/barlow/Barlow-Italic.ttf" "ofl/barlow/Barlow-BoldItalic.ttf" \
  "ofl/barlow/Barlow-Light.ttf" "ofl/barlow/Barlow-Medium.ttf" "ofl/barlow/Barlow-SemiBold.ttf" \
  "ofl/firasans/FiraSans-Regular.ttf" "ofl/firasans/FiraSans-Bold.ttf" "ofl/firasans/FiraSans-Italic.ttf" "ofl/firasans/FiraSans-BoldItalic.ttf" \
  "ofl/firasans/FiraSans-Light.ttf" "ofl/firasans/FiraSans-Medium.ttf" "ofl/firasans/FiraSans-SemiBold.ttf" \
  "ofl/ptsans/PT_Sans-Web-Regular.ttf" "ofl/ptsans/PT_Sans-Web-Bold.ttf" "ofl/ptsans/PT_Sans-Web-Italic.ttf" "ofl/ptsans/PT_Sans-Web-BoldItalic.ttf" \
  "ofl/zillaslab/ZillaSlab-Regular.ttf" "ofl/zillaslab/ZillaSlab-Bold.ttf" "ofl/zillaslab/ZillaSlab-Italic.ttf" "ofl/zillaslab/ZillaSlab-BoldItalic.ttf" \
  "ofl/notosansbengali/NotoSansBengali[wdth,wght].ttf" "ofl/notosanstamil/NotoSansTamil[wdth,wght].ttf" "ofl/notosanstelugu/NotoSansTelugu[wdth,wght].ttf" "ofl/notosansgujarati/NotoSansGujarati[wdth,wght].ttf" "ofl/notosansgurmukhi/NotoSansGurmukhi[wdth,wght].ttf" "ofl/notosanskannada/NotoSansKannada[wdth,wght].ttf" "ofl/notosansmalayalam/NotoSansMalayalam[wdth,wght].ttf" "ofl/notosanssinhala/NotoSansSinhala[wdth,wght].ttf" "ofl/notosanskhmer/NotoSansKhmer[wdth,wght].ttf" "ofl/notosanslao/NotoSansLao[wdth,wght].ttf" "ofl/notosansmyanmar/NotoSansMyanmar[wdth,wght].ttf" "ofl/notosansethiopic/NotoSansEthiopic[wdth,wght].ttf" "ofl/notosansarmenian/NotoSansArmenian[wdth,wght].ttf" "ofl/notosansgeorgian/NotoSansGeorgian[wdth,wght].ttf" \
  "ofl/sarabun/Sarabun-Regular.ttf" "ofl/sarabun/Sarabun-Bold.ttf" "ofl/sarabun/Sarabun-Italic.ttf" "ofl/sarabun/Sarabun-BoldItalic.ttf" \
  "ofl/oswald/Oswald[wght].ttf" \
  "ofl/shortstack/ShortStack-Regular.ttf" "ofl/dekko/Dekko-Regular.ttf" "apache/chewy/Chewy-Regular.ttf" \
  "ofl/barlowsemicondensed/BarlowSemiCondensed-Regular.ttf" "ofl/barlowsemicondensed/BarlowSemiCondensed-Bold.ttf" \
  "ofl/barlowsemicondensed/BarlowSemiCondensed-Italic.ttf" "ofl/barlowsemicondensed/BarlowSemiCondensed-BoldItalic.ttf" \
  "ofl/notonaskharabic/NotoNaskhArabic[wght].ttf" \
  "ofl/robotocondensed/RobotoCondensed[wght].ttf" "ofl/robotocondensed/RobotoCondensed-Italic[wght].ttf" \
  "ofl/barlowcondensed/BarlowCondensed-Regular.ttf" "ofl/barlowcondensed/BarlowCondensed-Bold.ttf" "ofl/barlowcondensed/BarlowCondensed-Italic.ttf" "ofl/barlowcondensed/BarlowCondensed-BoldItalic.ttf" \
  "ofl/barlowcondensed/BarlowCondensed-Light.ttf" "ofl/barlowcondensed/BarlowCondensed-Medium.ttf" "ofl/barlowcondensed/BarlowCondensed-SemiBold.ttf" \
  "ofl/ptsansnarrow/PT_Sans-Narrow-Web-Regular.ttf" "ofl/ptsansnarrow/PT_Sans-Narrow-Web-Bold.ttf" "ofl/yanonekaffeesatz/YanoneKaffeesatz[wght].ttf" \
  "ofl/worksans/WorkSans[wght].ttf" "ofl/worksans/WorkSans-Italic[wght].ttf" \
  "ofl/librebaskerville/LibreBaskerville[wght].ttf" "ofl/librebaskerville/LibreBaskerville-Italic[wght].ttf" \
  "ofl/publicsans/PublicSans[wght].ttf" "ofl/publicsans/PublicSans-Italic[wght].ttf" \
  "ofl/playfairdisplay/PlayfairDisplay[wght].ttf" "ofl/playfairdisplay/PlayfairDisplay-Italic[wght].ttf" \
  "ofl/titilliumweb/TitilliumWeb-Regular.ttf" "ofl/titilliumweb/TitilliumWeb-Bold.ttf" "ofl/titilliumweb/TitilliumWeb-Italic.ttf" \
  "ofl/titilliumweb/TitilliumWeb-Light.ttf" "ofl/titilliumweb/TitilliumWeb-SemiBold.ttf" \
  "ofl/mulish/Mulish[wght].ttf" "ofl/mulish/Mulish-Italic[wght].ttf" "apache/robotoslab/RobotoSlab[wght].ttf" "ofl/bebasneue/BebasNeue-Regular.ttf" \
  "ofl/kanit/Kanit-Regular.ttf" "ofl/kanit/Kanit-Bold.ttf" "ofl/caveat/Caveat[wght].ttf" "ofl/italianno/Italianno-Regular.ttf" \
  "ofl/notoemoji/NotoEmoji[wght].ttf" \
  ; do
  name="$(basename "$spec")"; url="$gf/$(dirname "$spec")/$(python3 -c 'import sys,urllib.parse;print(urllib.parse.quote(sys.argv[1]))' "$name")"
  fetch "$url" "$name"
done

# TeX Gyre (GUST Font License): URW-derived stand-ins for Century Gothic (Adventor),
# Book Antiqua/Palatino (Pagella), Bookman (Bonum), Century Schoolbook (Schola).
for fam in adventor pagella bonum schola; do
  for style in regular bold italic bolditalic; do
    fetch "$ctan/texgyre$fam-$style.otf" "texgyre$fam-$style.otf"
  done
done
# Selawik (OFL): Microsoft's metric-compatible open stand-in for Segoe UI.
if ! ls "$dest"/selawk*.ttf >/dev/null 2>&1; then
  tmp="$(mktemp -d)"
  curl -fsSL --retry 3 -o "$tmp/selawik.zip" https://github.com/microsoft/Selawik/releases/download/1.01/Selawik_Release.zip \
    && unzip -q -j -o "$tmp/selawik.zip" '*.ttf' -d "$dest" || echo "failed: Selawik" >&2
  rm -rf "$tmp"
fi
# Caladea 1.002 (Apache-2.0), the Cambria stand-in LibreOffice ships. Google Fonts has
# 1.001, whose advance widths and line height do not match Cambria.
if ! ls "$dest"/Caladea-*.ttf >/dev/null 2>&1; then
  tmp="$(mktemp -d)"
  curl -fsSL --retry 3 -o "$tmp/cros.tgz" https://dev-www.libreoffice.org/src/368f114c078f94214a308a74c7e991bc-crosextrafonts-20130214.tar.gz \
    && echo "c48d1c2fd613c9c06c959c34da7b8388059e2408d2bb19845dc3ed35f76e4d09  $tmp/cros.tgz" | { sha256sum -c - 2>/dev/null || shasum -a 256 -c -; } >/dev/null \
    && tar -xzf "$tmp/cros.tgz" -C "$dest" --strip-components=1 || echo "failed: Caladea" >&2
  rm -rf "$tmp"
fi
# Font Awesome 4.7 (font files under OFL 1.1): icon decks address it by private-use
# code points, which no other face maps.
fetch https://github.com/FortAwesome/Font-Awesome/raw/v4.7.0/fonts/fontawesome-webfont.ttf fontawesome-webfont.ttf
echo "fonts in $dest: $(ls "$dest" | wc -l)"
