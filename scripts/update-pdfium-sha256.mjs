#!/usr/bin/env node
// Rewrites PDFIUM_SHA256_TAG and the PDFIUM_SHA256 table in
// crates/pdfium-sys/build.rs for the release PDFIUM_RELEASE_TAG names, so a
// pdfium bump keeps its pinned hashes in step. Renovate runs it after bumping
// the tag (postUpgradeTasks, run-llama/platform .github/renovate-org-global.json5).
// After changing the tag by hand, run it from the repo root:
//
//   node scripts/update-pdfium-sha256.mjs
//
// Hashes come from GitHub's sha256 digest of each release asset. If the API is
// unavailable (rate limit, outage), each asset is downloaded and hashed instead.
// Exits non-zero, leaving build.rs untouched, if any asset is missing.

import { readFileSync, writeFileSync } from "node:fs";
import { createHash } from "node:crypto";

const BUILD_RS = new URL("../crates/pdfium-sys/build.rs", import.meta.url);
const RELEASES = "run-llama/pdfium-binaries";
const ATTEMPTS = 4;

function fail(message) {
  console.error(`update-pdfium-sha256: ${message}`);
  process.exit(1);
}

async function withRetries(what, fn) {
  let delayMs = 2000;
  for (let attempt = 1; ; attempt++) {
    try {
      return await fn();
    } catch (e) {
      if (attempt === ATTEMPTS)
        throw new Error(`${what}: ${e.message} (after ${ATTEMPTS} attempts)`);
      console.error(`update-pdfium-sha256: ${what}: ${e.message}; retrying in ${delayMs / 1000}s`);
      await new Promise((resolve) => setTimeout(resolve, delayMs));
      delayMs *= 2;
    }
  }
}

async function get(url, accept) {
  const headers = { Accept: accept, "User-Agent": "update-pdfium-sha256" };
  const token = process.env.GITHUB_TOKEN || process.env.GH_TOKEN;
  if (token && url.startsWith("https://api.github.com/")) headers.Authorization = `Bearer ${token}`;
  const response = await fetch(url, { headers, redirect: "follow" });
  if (!response.ok) throw new Error(`GET ${url}: HTTP ${response.status}`);
  return response;
}

const source = readFileSync(BUILD_RS, "utf8");
const tag = /const PDFIUM_RELEASE_TAG: &str = "([^"]+)";/.exec(source)?.[1];
if (!tag) fail("PDFIUM_RELEASE_TAG not found in crates/pdfium-sys/build.rs");
const rowPattern = /^( *)\("([^"]+)", "[0-9a-f]{64}"\),$/gm;
const stems = [...source.matchAll(rowPattern)].map((m) => m[2]);
if (stems.length === 0) fail("no PDFIUM_SHA256 rows found in crates/pdfium-sys/build.rs");

const encodedTag = encodeURIComponent(tag);
let digests = new Map();
try {
  const release = await withRetries("release lookup", async () =>
    (
      await get(
        `https://api.github.com/repos/${RELEASES}/releases/tags/${encodedTag}`,
        "application/vnd.github+json"
      )
    ).json()
  );
  for (const asset of release.assets ?? []) {
    if (typeof asset.digest === "string" && asset.digest.startsWith("sha256:")) {
      digests.set(asset.name, asset.digest.slice("sha256:".length));
    }
  }
} catch (e) {
  console.error(`update-pdfium-sha256: ${e.message}; hashing the downloads instead`);
}

const hashes = new Map();
for (const stem of stems) {
  const asset = `${stem}.tgz`;
  let sha256 = digests.get(asset);
  if (!sha256) {
    const url = `https://github.com/${RELEASES}/releases/download/${encodedTag}/${asset}`;
    try {
      const bytes = await withRetries(`download ${asset}`, async () =>
        Buffer.from(await (await get(url, "application/octet-stream")).arrayBuffer())
      );
      sha256 = createHash("sha256").update(bytes).digest("hex");
    } catch (e) {
      fail(`${e.message}; is ${asset} part of the ${tag} release?`);
    }
  }
  if (!/^[0-9a-f]{64}$/.test(sha256)) fail(`bad sha256 for ${asset}: ${sha256}`);
  hashes.set(stem, sha256);
}

let updated = source.replace(
  rowPattern,
  (_, indent, stem) => `${indent}("${stem}", "${hashes.get(stem)}"),`
);
const tagLine = /const PDFIUM_SHA256_TAG: &str = "[^"]+";/;
if (!tagLine.test(updated)) fail("PDFIUM_SHA256_TAG not found in crates/pdfium-sys/build.rs");
updated = updated.replace(tagLine, `const PDFIUM_SHA256_TAG: &str = "${tag}";`);
writeFileSync(BUILD_RS, updated);
console.log(
  `update-pdfium-sha256: ${stems.length} hashes for ${tag} written to crates/pdfium-sys/build.rs`
);
