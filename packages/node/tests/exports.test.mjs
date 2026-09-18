// Entry-point tests for the dual ESM/CJS build.
//
// These deliberately load the package BY NAME rather than by dist path, so they
// exercise the real `exports` map in package.json (Node resolves the package's
// own name via self-reference). Loading `./dist/lib.cjs` directly would still
// pass with a completely broken `exports` field.

import { test } from "node:test";
import assert from "node:assert/strict";
import { createRequire } from "node:module";
import { existsSync, mkdtempSync, readFileSync, rmSync, writeFileSync } from "node:fs";
import { execFileSync } from "node:child_process";
import { tmpdir } from "node:os";
import { dirname, join, resolve } from "node:path";
import { fileURLToPath, pathToFileURL } from "node:url";

const PACKAGE_NAME = "@llamaindex/liteparse";
const packageDir = resolve(dirname(fileURLToPath(import.meta.url)), "..");
const repoRoot = resolve(packageDir, "..", "..");
const samplePdf = join(repoRoot, "integration_tests_data", "sample.pdf");
const require = createRequire(import.meta.url);

/** Collect every file path referenced by a nested exports condition tree. */
function collectPaths(node, found = []) {
  if (typeof node === "string") found.push(node);
  else if (node && typeof node === "object") {
    for (const value of Object.values(node)) collectPaths(value, found);
  }
  return found;
}

test("every path referenced by package.json exists in the build", () => {
  const pkg = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  const referenced = new Set([
    pkg.main,
    pkg.module,
    pkg.types,
    ...Object.values(pkg.bin ?? {}),
    ...collectPaths(pkg.exports),
  ]);

  for (const relativePath of referenced) {
    assert.ok(
      existsSync(join(packageDir, relativePath)),
      `package.json references ${relativePath}, but the build did not emit it`,
    );
  }
});

test("declares a types file for both the import and require conditions", () => {
  const pkg = JSON.parse(readFileSync(join(packageDir, "package.json"), "utf8"));
  // A single top-level `types` resolves to the ESM .d.ts for CJS consumers too,
  // which makes TypeScript reject `require()` with TS1479 even though the call
  // works at runtime. Each condition needs its own declaration file.
  assert.ok(pkg.exports["."].import.types, "missing types for import condition");
  assert.ok(pkg.exports["."].require.types, "missing types for require condition");
  assert.notEqual(
    pkg.exports["."].import.types,
    pkg.exports["."].require.types,
    "import and require must not share one declaration file",
  );
});

test("ESM: import by package name loads the native module and parses", async () => {
  const { LiteParse } = await import(PACKAGE_NAME);
  assert.equal(typeof LiteParse, "function");

  const result = await new LiteParse({ ocrEnabled: false, quiet: true }).parse(
    samplePdf,
  );
  assert.ok(result.pages.length > 0, "expected at least one parsed page");
  assert.ok(result.text.length > 0, "expected non-empty extracted text");
});

test("CJS: require by package name loads the native module and parses", async () => {
  const { LiteParse } = require(PACKAGE_NAME);
  assert.equal(typeof LiteParse, "function");

  const result = await new LiteParse({ ocrEnabled: false, quiet: true }).parse(
    samplePdf,
  );
  assert.ok(result.pages.length > 0, "expected at least one parsed page");
  assert.ok(result.text.length > 0, "expected non-empty extracted text");
});

test("rejects an invalid outputFormat", async () => {
  const { LiteParse } = await import(PACKAGE_NAME);

  assert.throws(
    () => new LiteParse({ outputFormat: "yaml" }),
    /invalid outputFormat: yaml/,
  );
});

// The native .node binary is located relative to the built file's own path.
// Loading from a working directory outside the package, via an entry script
// that cannot resolve the package by name, is the case that catches a loader
// which infers its location from process.cwd() or process.argv[1] instead.
//
// `import()` takes a URL specifier, so the ESM path must go through
// pathToFileURL — on Windows a bare `C:\...` path is read as an unknown
// protocol. `require()` takes a plain filesystem path and must not be a URL.
// POSIX tolerates a bare absolute path in `import()`, so the specifier shape is
// asserted below rather than left to fail on Windows only.
for (const [label, specifier, load] of [
  [
    "ESM",
    pathToFileURL(join(packageDir, "dist", "lib.js")).href,
    (s) => `import(${s})`,
  ],
  [
    "CJS",
    join(packageDir, "dist", "lib.cjs"),
    (s) => `Promise.resolve(require(${s}))`,
  ],
]) {
  test(`${label}: loads by absolute path from an unrelated working directory`, () => {
    if (label === "ESM") {
      assert.ok(
        specifier.startsWith("file://"),
        "dynamic import() needs a file:// URL to work on Windows",
      );
    } else {
      assert.ok(
        !specifier.startsWith("file:"),
        "require() takes a filesystem path, not a URL",
      );
    }

    const script = `
      ${load(JSON.stringify(specifier))}.then(async ({ LiteParse }) => {
        const r = await new LiteParse({ ocrEnabled: false, quiet: true })
          .parse(${JSON.stringify(samplePdf)});
        if (!r.pages.length) throw new Error("no pages parsed");
        console.log("OK");
      }).catch((e) => { console.error(e.message); process.exit(1); });
    `;
    // The temp dir has no package.json, so `.cjs` is CommonJS either way, and
    // the package is unresolvable by name from here — which is the point.
    const scriptDir = mkdtempSync(join(tmpdir(), "liteparse-"));
    const scriptPath = join(scriptDir, "entry.cjs");
    writeFileSync(scriptPath, script);

    try {
      const output = execFileSync(process.execPath, [scriptPath], {
        cwd: scriptDir,
        encoding: "utf8",
      });
      assert.match(output, /OK/);
    } finally {
      rmSync(scriptDir, { recursive: true, force: true });
    }
  });
}

test("ESM and CJS entry points produce identical output", async () => {
  const { LiteParse: EsmLiteParse } = await import(PACKAGE_NAME);
  const { LiteParse: CjsLiteParse } = require(PACKAGE_NAME);

  const config = { ocrEnabled: false, quiet: true };
  const esm = await new EsmLiteParse(config).parse(samplePdf);
  const cjs = await new CjsLiteParse(config).parse(samplePdf);

  assert.equal(cjs.text, esm.text);
  assert.equal(cjs.pages.length, esm.pages.length);
});
