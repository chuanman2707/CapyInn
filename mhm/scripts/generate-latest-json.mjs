import fs from "node:fs";
import path from "node:path";
import { fileURLToPath } from "node:url";

const REQUIRED_PLATFORM_KEYS = [
  "linux-x86_64",
  "windows-x86_64",
  "darwin-aarch64",
  "darwin-x86_64",
];

function assertNonEmptyString(value, fieldName) {
  if (typeof value !== "string" || value.trim() === "") {
    throw new Error(`expected non-empty string for ${fieldName}`);
  }
}

function assertPlainObject(value, fieldName) {
  if (!value || typeof value !== "object" || Array.isArray(value)) {
    throw new Error(`expected object for ${fieldName}`);
  }
}

function validateImmutableAssetUrl(value, platformKey) {
  let parsedUrl;

  try {
    parsedUrl = new URL(value.url);
  } catch {
    throw new Error(`invalid url for ${platformKey}`);
  }

  if (parsedUrl.protocol !== "https:") {
    throw new Error(`https asset URL required for ${platformKey}`);
  }

  if (parsedUrl.pathname.includes("/releases/latest/")) {
    throw new Error(`immutable asset URL required for ${platformKey}`);
  }

  if (!parsedUrl.pathname.includes("/releases/download/")) {
    throw new Error(`release download URL required for ${platformKey}`);
  }
}

export function buildLatestManifest(input) {
  assertPlainObject(input, "input");

  const { version, notes, pubDate, platforms } = input;

  assertNonEmptyString(version, "version");
  if (typeof notes !== "string") {
    throw new Error("expected string for notes");
  }
  assertNonEmptyString(pubDate, "pubDate");
  if (Number.isNaN(Date.parse(pubDate))) {
    throw new Error("invalid pubDate");
  }
  assertPlainObject(platforms, "platforms");

  const platformKeys = Object.keys(platforms);

  for (const requiredKey of REQUIRED_PLATFORM_KEYS) {
    if (!platforms[requiredKey]) {
      throw new Error(`missing required platform key: ${requiredKey}`);
    }
  }

  for (const key of platformKeys) {
    if (!REQUIRED_PLATFORM_KEYS.includes(key)) {
      throw new Error(`unexpected platform key: ${key}`);
    }
  }

  const orderedPlatforms = {};
  const seenUrls = new Map();

  for (const key of REQUIRED_PLATFORM_KEYS) {
    const platform = platforms[key];
    assertPlainObject(platform, `platforms.${key}`);
    assertNonEmptyString(platform.signature, `platforms.${key}.signature`);
    assertNonEmptyString(platform.url, `platforms.${key}.url`);
    validateImmutableAssetUrl(platform, key);
    const normalizedUrl = platform.url.trim();
    const existingPlatformForUrl = seenUrls.get(normalizedUrl);
    if (existingPlatformForUrl) {
      throw new Error(
        `duplicate asset URL for ${key}: already used by ${existingPlatformForUrl}`,
      );
    }
    seenUrls.set(normalizedUrl, key);
    orderedPlatforms[key] = {
      signature: platform.signature.trim(),
      url: normalizedUrl,
    };
  }

  return {
    version: version.trim(),
    notes,
    pub_date: pubDate,
    platforms: orderedPlatforms,
  };
}

function main(argv) {
  const [inputPath, outputPath] = argv;

  if (!inputPath || !outputPath) {
    throw new Error("usage: node generate-latest-json.mjs <input.json> <output.json>");
  }

  const payload = JSON.parse(fs.readFileSync(inputPath, "utf8"));
  const manifest = buildLatestManifest(payload);

  fs.mkdirSync(path.dirname(outputPath), { recursive: true });
  fs.writeFileSync(outputPath, `${JSON.stringify(manifest, null, 2)}\n`);
}

const isEntrypoint = process.argv[1] === fileURLToPath(import.meta.url);

if (isEntrypoint) {
  main(process.argv.slice(2));
}
