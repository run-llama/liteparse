import { LiteParse } from "./liteparse-browser.js";

const fileInput = document.getElementById("file") as HTMLInputElement;
const parseBtn = document.getElementById("parse") as HTMLButtonElement;
const statusEl = document.getElementById("status") as HTMLDivElement;
const textOut = document.getElementById("text-output") as HTMLTextAreaElement;
const jsonOut = document.getElementById("json-output") as HTMLTextAreaElement;
const ocrToggle = document.getElementById("ocr") as HTMLInputElement;
const shotsToggle = document.getElementById("shots") as HTMLInputElement;
const screenshotsEl = document.getElementById("screenshots") as HTMLElement;
const dropzone = document.getElementById("dropzone") as HTMLLabelElement;
const dropzoneBody = dropzone.querySelector(".dropzone-body") as HTMLDivElement;

let currentScreenshotUrls: string[] = [];
function clearScreenshots() {
  for (const url of currentScreenshotUrls) URL.revokeObjectURL(url);
  currentScreenshotUrls = [];
  screenshotsEl.innerHTML = "";
  screenshotsEl.hidden = true;
}

function renderFilename() {
  const existing = dropzone.querySelector(".filename");
  if (existing) existing.remove();
  const file = fileInput.files?.[0];
  if (!file) return;
  const pill = document.createElement("div");
  pill.className = "filename";
  pill.textContent = file.name;
  pill.title = file.name;
  dropzoneBody.appendChild(pill);
}

dropzone.addEventListener("keydown", (e: KeyboardEvent) => {
  if (e.key === "Enter" || e.key === " ") {
    e.preventDefault();
    fileInput.click();
  }
});

for (const evt of ["dragenter", "dragover"] as const) {
  dropzone.addEventListener(evt, (e) => {
    e.preventDefault();
    dropzone.classList.add("dragover");
  });
}
for (const evt of ["dragleave", "dragend", "drop"] as const) {
  dropzone.addEventListener(evt, () => {
    dropzone.classList.remove("dragover");
  });
}
dropzone.addEventListener("drop", (e: DragEvent) => {
  e.preventDefault();
  const file = e.dataTransfer?.files?.[0];
  if (!file) return;
  const dt = new DataTransfer();
  dt.items.add(file);
  fileInput.files = dt.files;
  fileInput.dispatchEvent(new Event("change", { bubbles: true }));
});

function setStatus(msg: string, kind: "info" | "error" = "info") {
  statusEl.textContent = msg;
  statusEl.classList.toggle("error", kind === "error");
}

function isPdfBytes(bytes: Uint8Array): boolean {
  return (
    bytes.length >= 4 &&
    bytes[0] === 0x25 && // %
    bytes[1] === 0x50 && // P
    bytes[2] === 0x44 && // D
    bytes[3] === 0x46 // F
  );
}

fileInput.addEventListener("change", () => {
  parseBtn.disabled = !fileInput.files || fileInput.files.length === 0;
  setStatus("");
  renderFilename();
});

for (const btn of Array.from(
  document.querySelectorAll<HTMLButtonElement>("button.copy")
)) {
  btn.addEventListener("click", async () => {
    const targetId = btn.dataset.target;
    if (!targetId) return;
    const target = document.getElementById(targetId) as HTMLTextAreaElement | null;
    if (!target) return;
    const text = target.value;
    try {
      await navigator.clipboard.writeText(text);
    } catch {
      // Fallback for browsers without clipboard API — select the text so the
      // user can copy manually.
      target.focus();
      target.select();
    }
    const originalLabel = btn.textContent ?? "Copy";
    btn.textContent = "Copied!";
    window.setTimeout(() => {
      btn.textContent = originalLabel;
    }, 1500);
  });
}

parseBtn.addEventListener("click", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  textOut.value = "";
  jsonOut.value = "";
  clearScreenshots();

  const bytes = new Uint8Array(await file.arrayBuffer());
  if (!isPdfBytes(bytes)) {
    setStatus("That doesn't look like a PDF. Please choose a .pdf file.", "error");
    return;
  }

  parseBtn.disabled = true;
  setStatus("Parsing…");

  try {
    const parser = new LiteParse({
      ocrEnabled: ocrToggle.checked,
      outputFormat: "json",
      preciseBoundingBox: false,
    });
    const result = await parser.parse(bytes, true);
    textOut.value = result.text;
    jsonOut.value = JSON.stringify(result.json ?? result, null, 2);
    setStatus(`Parsed ${result.pages.length} page${result.pages.length === 1 ? "" : "s"}.`);

    if (shotsToggle.checked) {
      setStatus("Rendering screenshots…");
      // parse() detached the bytes via the pdf.js worker transfer; pass
      // a fresh copy to screenshot() so it can load its own doc.
      const screenshotBytes = new Uint8Array(await file.arrayBuffer());
      const shots = await parser.screenshot(screenshotBytes, undefined, true);
      screenshotsEl.hidden = shots.length === 0;
      for (const shot of shots) {
        const blob = new Blob([shot.imageBuffer as unknown as BlobPart], {
          type: "image/png",
        });
        const url = URL.createObjectURL(blob);
        currentScreenshotUrls.push(url);
        const img = document.createElement("img");
        img.src = url;
        img.alt = `Page ${shot.pageNum}`;
        img.loading = "lazy";
        screenshotsEl.appendChild(img);
      }
      setStatus(
        `Parsed ${result.pages.length} page${result.pages.length === 1 ? "" : "s"}; ` +
          `rendered ${shots.length} screenshot${shots.length === 1 ? "" : "s"}.`
      );
    }
  } catch (err) {
    const msg = err instanceof Error ? err.message : String(err);
    setStatus(`Parse failed: ${msg}`, "error");
  } finally {
    parseBtn.disabled = false;
  }
});
