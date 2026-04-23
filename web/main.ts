// Entry point for the browser app.

const fileInput = document.getElementById("file") as HTMLInputElement;
const parseBtn = document.getElementById("parse") as HTMLButtonElement;

fileInput.addEventListener("change", () => {
  parseBtn.disabled = !fileInput.files || fileInput.files.length === 0;
});
