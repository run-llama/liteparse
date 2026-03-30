import os from "os";
import { vi, describe, it, expect, afterEach } from "vitest";
import { EventEmitter } from "events";

const mockFileTypeFromFile = vi.fn();
vi.mock("file-type", () => ({
  fileTypeFromFile: (...args: unknown[]) => mockFileTypeFromFile(...args),
  fileTypeFromBuffer: vi.fn(async (data: Buffer | Uint8Array) => {
    // Replicate enough detection to validate the wrapper
    if (
      data.length >= 4 &&
      data[0] === 0x25 &&
      data[1] === 0x50 &&
      data[2] === 0x44 &&
      data[3] === 0x46
    ) {
      return { ext: "pdf", mime: "application/pdf" };
    }
    if (
      data.length >= 8 &&
      data[0] === 0x89 &&
      data[1] === 0x50 &&
      data[2] === 0x4e &&
      data[3] === 0x47
    ) {
      return { ext: "png", mime: "image/png" };
    }
    if (data.length >= 3 && data[0] === 0xff && data[1] === 0xd8 && data[2] === 0xff) {
      return { ext: "jpg", mime: "image/jpeg" };
    }
    if (
      data.length >= 4 &&
      ((data[0] === 0x49 && data[1] === 0x49 && data[2] === 0x2a && data[3] === 0x00) ||
        (data[0] === 0x4d && data[1] === 0x4d && data[2] === 0x00 && data[3] === 0x2a))
    ) {
      return { ext: "tif", mime: "image/tiff" };
    }
    if (data.length >= 4 && data[0] === 0x50 && data[1] === 0x4b) {
      return { ext: "zip", mime: "application/zip" };
    }
    return undefined;
  }),
}));

const mockProc = {
  stdout: new EventEmitter(),
  stderr: new EventEmitter(),
  kill: vi.fn(),
  on: vi.fn(),
};

vi.mock("child_process", () => ({
  spawn: vi.fn(() => mockProc),
}));

vi.mock("fs", async () => {
  const actual = await vi.importActual<typeof import("fs")>("fs");
  return {
    ...actual,
    promises: {
      access: vi.fn(async (path: string, _mode?: number) => {
        console.log(path);
        const toErrorPath = [
          "/Applications/LibreOffice.app/Contents/MacOS/soffice",
          "/Applications/LibreOffice.app/Contents/MacOS/libreoffice",
          "C:\\Program Files\\Libreoffice\\program\\soffice.exe",
          "./test_fail.pdf",
          "test_fail.pdf",
          "test.docx",
        ];
        if (toErrorPath.includes(path)) {
          throw new Error("unaccessible");
        }
        return;
      }),
      mkdtemp: vi.fn(async () => {
        return "/tmp/test";
      }),
      rm: vi.fn(async () => {}),
      writeFile: vi.fn(async () => {}),
      readFile: vi.fn(async () => {
        return "hello world";
      }),
    },
  };
});

import {
  guessFileExtension,
  guessExtensionFromBuffer,
  findImageMagickCommand,
  findLibreOfficeCommand,
  convertOfficeDocument,
  convertImageToPdf,
  convertToPdf,
  getTmpDir,
} from "./convertToPdf";

describe("test guessFileExtension", () => {
  it("detects PDF via file-type", async () => {
    mockFileTypeFromFile.mockResolvedValue({ ext: "pdf", mime: "application/pdf" });
    expect(await guessFileExtension("/some/file")).toBe(".pdf");
  });

  it("detects PNG via file-type", async () => {
    mockFileTypeFromFile.mockResolvedValue({ ext: "png", mime: "image/png" });
    expect(await guessFileExtension("/some/file")).toBe(".png");
  });

  it("defaults to .pdf when file-type returns undefined", async () => {
    mockFileTypeFromFile.mockResolvedValue(undefined);
    expect(await guessFileExtension("/some/file")).toBe(".pdf");
  });

  it("returns extension directly if present", async () => {
    expect(await guessFileExtension("/some/file.pdf")).toBe(".pdf");
  });
});

describe("test command availability", () => {
  it("libreoffice available", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "/opt/bin/libreoffice");
    const result = await findLibreOfficeCommand();
    expect(result).toBe("libreoffice");
  });

  it("libreoffice not available", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(1);
    });
    mockProc.stderr.emit("data", "command not found");
    // does not throw
    const result = await findLibreOfficeCommand();
    expect(result).toBeNull();
  });

  it("imagemagick available", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "/opt/bin/libreoffice");
    const result = await findImageMagickCommand();
    expect(result).toStrictEqual({ command: "magick", args: [] });
  });

  it("imagemagick not available", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(1);
    });
    mockProc.stderr.emit("data", "command not found");
    // does not throw
    const result = await findImageMagickCommand();
    expect(result).toBeNull();
  });
});

describe("test convertOfficeDocument", () => {
  it("conversion succeeds", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");

    const result = await convertOfficeDocument("test.docx", "./");
    expect(result).toBe("test.pdf");
  });

  it("conversion fails (command not found)", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(1);
    });
    mockProc.stdout.emit("data", "command not found");

    await expect(convertOfficeDocument("test_command.docx", "./")).rejects.toThrow(
      "LibreOffice is not installed. Please install LibreOffice to convert office documents. On macOS: brew install --cask libreoffice, On Ubuntu: apt-get install libreoffice"
    );
  });

  it("conversion fails (output not found)", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");

    await expect(convertOfficeDocument("test_fail.docx", "./")).rejects.toThrow(
      "LibreOffice conversion succeeded but output PDF not found"
    );
  });
});

describe("test convertImageToPdf", () => {
  it("conversion succeeds", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");

    const result = await convertImageToPdf("test.png", "./");
    expect(result).toBe("test.pdf");
  });

  it("conversion fails (command not found)", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(1);
    });
    mockProc.stdout.emit("data", "command not found");

    await expect(convertImageToPdf("test_command.png", "./")).rejects.toThrow(
      "ImageMagick is not installed. Please install ImageMagick to convert images. On macOS: brew install imagemagick, On Ubuntu: apt-get install imagemagick"
    );
  });
});

describe("test convertToPdf", () => {
  it("convert PDF fails because file not found", async () => {
    const result = await convertToPdf("test.docx");
    expect(result).toStrictEqual({
      message: `File not found: test.docx`,
      code: "FILE_NOT_FOUND",
    });
  });

  it("convert an office document (word)", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");
    const result = await convertToPdf("test_1.docx");
    expect(result).toStrictEqual({
      pdfPath: "/tmp/test/test_1.pdf",
      originalExtension: ".docx",
    });
  });

  it("convert an office document (xlsx)", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");
    const result = await convertToPdf("test.xlsx");
    expect(result).toStrictEqual({
      pdfPath: "/tmp/test/test.pdf",
      originalExtension: ".xlsx",
    });
  });

  it("convert an image", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");
    const result = await convertToPdf("test.png");
    expect(result).toStrictEqual({
      pdfPath: "/tmp/test/test.pdf",
      originalExtension: ".png",
    });
  });

  it("convert a text file", async () => {
    mockProc.on.mockImplementation((event, cb) => {
      if (event === "close") cb(0);
    });
    mockProc.stdout.emit("data", "conversion successfull");
    const result = await convertToPdf("test.txt");
    expect(result).toStrictEqual({
      content: "hello world",
    });
  });
});

describe("test convertBufferToPdf", () => {
  it("cleans up the temp directory when an unsupported buffer returns passthrough content", async () => {
    const zipBytes = Buffer.from([0x50, 0x4b, 0x03, 0x04]);

    const { convertBufferToPdf } = await import("./convertToPdf");
    const fsModule = await import("fs");

    const result = await convertBufferToPdf(zipBytes);

    expect(result).toStrictEqual({
      content: "hello world",
    });
    expect(fsModule.promises.rm).toHaveBeenCalledWith("/tmp/test", {
      recursive: true,
      force: true,
    });
  });

  it("does not mask passthrough content when temp cleanup fails", async () => {
    const zipBytes = Buffer.from([0x50, 0x4b, 0x03, 0x04]);

    const { convertBufferToPdf } = await import("./convertToPdf");
    const fsModule = await import("fs");
    vi.mocked(fsModule.promises.rm).mockImplementationOnce(async () => {
      throw new Error("cleanup failed");
    });

    const result = await convertBufferToPdf(zipBytes);

    expect(result).toStrictEqual({
      content: "hello world",
    });
  });
});

describe("test guessExtensionFromBuffer", () => {
  it("detects PDF from magic bytes", async () => {
    const pdfBytes = Buffer.from("%PDF-1.4 some content");
    expect(await guessExtensionFromBuffer(pdfBytes)).toBe(".pdf");
  });

  it("detects PNG from magic bytes", async () => {
    const pngBytes = Buffer.from([0x89, 0x50, 0x4e, 0x47, 0x0d, 0x0a, 0x1a, 0x0a]);
    expect(await guessExtensionFromBuffer(pngBytes)).toBe(".png");
  });

  it("detects JPEG from magic bytes", async () => {
    const jpegBytes = Buffer.from([0xff, 0xd8, 0xff, 0xe0, 0x00, 0x10]);
    expect(await guessExtensionFromBuffer(jpegBytes)).toBe(".jpg");
  });

  it("detects TIFF (little-endian) from magic bytes", async () => {
    const tiffBytes = Buffer.from([0x49, 0x49, 0x2a, 0x00]);
    expect(await guessExtensionFromBuffer(tiffBytes)).toBe(".tif");
  });

  it("detects TIFF (big-endian) from magic bytes", async () => {
    const tiffBytes = Buffer.from([0x4d, 0x4d, 0x00, 0x2a]);
    expect(await guessExtensionFromBuffer(tiffBytes)).toBe(".tif");
  });

  it("detects ZIP-based formats from magic bytes", async () => {
    const zipBytes = Buffer.from([0x50, 0x4b, 0x03, 0x04]);
    expect(await guessExtensionFromBuffer(zipBytes)).toBe(".zip");
  });

  it("defaults to .pdf for unknown bytes", async () => {
    const unknownBytes = Buffer.from([0x00, 0x01, 0x02, 0x03]);
    expect(await guessExtensionFromBuffer(unknownBytes)).toBe(".pdf");
  });

  it("works with Uint8Array input", async () => {
    const pdfBytes = new Uint8Array(Buffer.from("%PDF-1.7"));
    expect(await guessExtensionFromBuffer(pdfBytes)).toBe(".pdf");
  });
});

describe("test getTmpDir", () => {
  const originalEnv = process.env.LITEPARSE_TMPDIR;

  afterEach(() => {
    if (originalEnv === undefined) {
      delete process.env.LITEPARSE_TMPDIR;
    } else {
      process.env.LITEPARSE_TMPDIR = originalEnv;
    }
  });

  it("returns LITEPARSE_TMPDIR when set", () => {
    process.env.LITEPARSE_TMPDIR = "/custom/tmp";
    expect(getTmpDir()).toBe("/custom/tmp");
  });

  it("falls back to os.tmpdir() when LITEPARSE_TMPDIR is not set", () => {
    delete process.env.LITEPARSE_TMPDIR;
    expect(getTmpDir()).toBe(os.tmpdir());
  });
});
