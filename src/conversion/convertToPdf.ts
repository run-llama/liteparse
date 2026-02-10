import { promises as fs } from 'fs';
import { spawn } from 'child_process';
import path from 'path';
import os from 'os';

export interface ConversionResult {
  pdfPath: string;
  originalExtension: string;
}

export interface ConversionError {
  message: string;
  code: string;
}

// File extension categories
export const officeExtensions = [
  '.doc',
  '.docx',
  '.docm',
  '.dot',
  '.dotm',
  '.dotx',
  '.odt',
  '.ott',
  '.ppt',
  '.pptx',
  '.pptm',
  '.pot',
  '.potm',
  '.potx',
  '.odp',
  '.otp',
  '.rtf',
  '.pages',
  '.key',
];

export const spreadsheetExtensions = [
  '.xls',
  '.xlsx',
  '.xlsm',
  '.xlsb',
  '.ods',
  '.ots',
  '.csv',
  '.tsv',
  '.numbers',
];

export const imageExtensions = [
  '.jpg',
  '.jpeg',
  '.png',
  '.gif',
  '.bmp',
  '.tiff',
  '.tif',
  '.webp',
  '.svg',
];

export const htmlExtensions = ['.htm', '.html', '.xhtml'];

/**
 * Guess file extension from file content
 */
export async function guessFileExtension(filePath: string): Promise<string> {
  const ext = path.extname(filePath).toLowerCase();
  if (ext) {
    return ext;
  }

  // Read first few bytes to detect file type
  const buffer = Buffer.alloc(16);
  const fd = await fs.open(filePath, 'r');
  await fd.read(buffer, 0, 16, 0);
  await fd.close();

  // PDF: %PDF
  if (buffer.toString('utf-8', 0, 4) === '%PDF') {
    return '.pdf';
  }

  // PNG: 89 50 4E 47
  if (
    buffer[0] === 0x89 &&
    buffer[1] === 0x50 &&
    buffer[2] === 0x4e &&
    buffer[3] === 0x47
  ) {
    return '.png';
  }

  // JPEG: FF D8 FF
  if (buffer[0] === 0xff && buffer[1] === 0xd8 && buffer[2] === 0xff) {
    return '.jpg';
  }

  // ZIP-based formats (docx, xlsx, etc): PK
  if (buffer[0] === 0x50 && buffer[1] === 0x4b) {
    // Could be docx, xlsx, pptx, odt, etc.
    return '.docx'; // Default to docx for now
  }

  return ext || '.pdf';
}

/**
 * Execute command with timeout
 */
async function executeCommand(
  command: string,
  args: string[],
  timeoutMs = 60000
): Promise<string> {
  return new Promise((resolve, reject) => {
    const proc = spawn(command, args, {
      stdio: ['ignore', 'pipe', 'pipe'],
    });

    let stdout = '';
    let stderr = '';

    proc.stdout?.on('data', (data) => {
      stdout += data.toString();
    });

    proc.stderr?.on('data', (data) => {
      stderr += data.toString();
    });

    const timeout = setTimeout(() => {
      proc.kill();
      reject(new Error(`Command timeout after ${timeoutMs}ms`));
    }, timeoutMs);

    proc.on('close', (code) => {
      clearTimeout(timeout);
      if (code === 0) {
        resolve(stdout);
      } else {
        reject(new Error(`Command failed with code ${code}: ${stderr}`));
      }
    });

    proc.on('error', (err) => {
      clearTimeout(timeout);
      reject(err);
    });
  });
}

/**
 * Check if a command is available
 */
async function isCommandAvailable(command: string): Promise<boolean> {
  try {
    await executeCommand('which', [command], 5000);
    return true;
  } catch {
    return false;
  }
}

/**
 * Convert office documents using LibreOffice
 */
async function convertOfficeDocument(
  filePath: string,
  outputDir: string
): Promise<string> {
  const hasLibreOffice = await isCommandAvailable('libreoffice');
  if (!hasLibreOffice) {
    throw new Error(
      'LibreOffice is not installed. Please install LibreOffice to convert office documents. On macOS: brew install --cask libreoffice, On Ubuntu: apt-get install libreoffice'
    );
  }

  await executeCommand(
    'libreoffice',
    [
      '--headless',
      '--invisible',
      '--convert-to',
      'pdf',
      '--outdir',
      outputDir,
      filePath,
    ],
    120000 // 2 minutes timeout
  );

  // LibreOffice creates output with same name but .pdf extension
  const baseName = path.basename(filePath, path.extname(filePath));
  const pdfPath = path.join(outputDir, `${baseName}.pdf`);

  // Verify the PDF was created
  try {
    await fs.access(pdfPath);
    return pdfPath;
  } catch {
    throw new Error('LibreOffice conversion succeeded but output PDF not found');
  }
}

/**
 * Convert images to PDF using ImageMagick
 */
async function convertImageToPdf(
  filePath: string,
  outputDir: string
): Promise<string> {
  const hasConvert = await isCommandAvailable('convert');
  if (!hasConvert) {
    throw new Error(
      'ImageMagick is not installed. Please install ImageMagick to convert images. On macOS: brew install imagemagick, On Ubuntu: apt-get install imagemagick'
    );
  }

  const baseName = path.basename(filePath, path.extname(filePath));
  const pdfPath = path.join(outputDir, `${baseName}.pdf`);

  await executeCommand(
    'convert',
    [
      filePath,
      '-density',
      '150',
      '-units',
      'PixelsPerInch',
      pdfPath,
    ],
    60000
  );

  return pdfPath;
}

/**
 * Main conversion function
 */
export async function convertToPdf(
  filePath: string
): Promise<ConversionResult | ConversionError> {
  try {
    // Check if file exists
    try {
      await fs.access(filePath);
    } catch {
      return {
        message: `File not found: ${filePath}`,
        code: 'FILE_NOT_FOUND',
      };
    }

    // Get file extension
    const extension = await guessFileExtension(filePath);

    // If already PDF, return as-is
    if (extension === '.pdf') {
      return {
        pdfPath: filePath,
        originalExtension: extension,
      };
    }

    // Create temp directory for output
    const tmpDir = await fs.mkdtemp(path.join(os.tmpdir(), 'liteparse-'));

    // Convert based on file type
    let pdfPath: string;

    if (officeExtensions.includes(extension)) {
      console.error(`Converting office document: ${path.basename(filePath)}`);
      pdfPath = await convertOfficeDocument(filePath, tmpDir);
    } else if (spreadsheetExtensions.includes(extension)) {
      console.error(`Converting spreadsheet: ${path.basename(filePath)}`);
      pdfPath = await convertOfficeDocument(filePath, tmpDir);
    } else if (imageExtensions.includes(extension)) {
      console.error(`Converting image: ${path.basename(filePath)}`);
      pdfPath = await convertImageToPdf(filePath, tmpDir);
    } else if (htmlExtensions.includes(extension)) {
      return {
        message: `HTML conversion not yet supported. Please convert to PDF manually.`,
        code: 'UNSUPPORTED_FORMAT',
      };
    } else {
      return {
        message: `Unsupported file format: ${extension}`,
        code: 'UNSUPPORTED_FORMAT',
      };
    }

    return {
      pdfPath,
      originalExtension: extension,
    };
  } catch (error) {
    return {
      message: error instanceof Error ? error.message : String(error),
      code: 'CONVERSION_ERROR',
    };
  }
}

/**
 * Clean up temporary conversion files
 */
export async function cleanupConversionFiles(pdfPath: string): Promise<void> {
  try {
    // Only delete if in temp directory
    if (pdfPath.includes(os.tmpdir())) {
      const dir = path.dirname(pdfPath);
      await fs.rm(dir, { recursive: true, force: true });
    }
  } catch {
    // Ignore cleanup errors
  }
}
