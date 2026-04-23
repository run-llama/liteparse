// Entry point for the browser app.

const fileInput = document.getElementById("file") as HTMLInputElement;
const parseBtn = document.getElementById("parse") as HTMLButtonElement;
const statusEl = document.getElementById("status") as HTMLDivElement;
const textOut = document.getElementById("text-output") as HTMLTextAreaElement;
const jsonOut = document.getElementById("json-output") as HTMLTextAreaElement;

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
});

parseBtn.addEventListener("click", async () => {
  const file = fileInput.files?.[0];
  if (!file) return;

  textOut.value = "";
  jsonOut.value = "";

  const bytes = new Uint8Array(await file.arrayBuffer());
  if (!isPdfBytes(bytes)) {
    setStatus("That doesn't look like a PDF. Please choose a .pdf file.", "error");
    return;
  }

  setStatus("PDF validated. Parsing not wired yet.");
});
