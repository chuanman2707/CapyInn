import { describe, expect, it } from "vitest";

import { buildLatestManifest } from "../../scripts/generate-latest-json.mjs";

describe("buildLatestManifest", () => {
  it("builds a single manifest with all required production targets", () => {
    const manifest = buildLatestManifest({
      version: "0.2.0",
      notes: "Release body",
      pubDate: "2026-04-18T12:00:00Z",
      platforms: {
        "darwin-x86_64": {
          signature: "mac-intel-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn-x64.app.tar.gz",
        },
        "windows-x86_64": {
          signature: "windows-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/capyinn-0.2.0-x64-setup.exe",
        },
        "darwin-aarch64": {
          signature: "mac-arm-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn.app.tar.gz",
        },
      },
    });

    expect(manifest).toEqual({
      version: "0.2.0",
      notes: "Release body",
      pub_date: "2026-04-18T12:00:00Z",
      platforms: {
        "windows-x86_64": {
          signature: "windows-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/capyinn-0.2.0-x64-setup.exe",
        },
        "darwin-aarch64": {
          signature: "mac-arm-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn.app.tar.gz",
        },
        "darwin-x86_64": {
          signature: "mac-intel-sig",
          url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn-x64.app.tar.gz",
        },
      },
    });
    expect(Object.keys(manifest.platforms)).toEqual([
      "windows-x86_64",
      "darwin-aarch64",
      "darwin-x86_64",
    ]);
  });

  it("rejects manifests that use mutable latest URLs", () => {
    expect(() =>
      buildLatestManifest({
        version: "0.2.0",
        notes: "",
        pubDate: "2026-04-18T12:00:00Z",
        platforms: {
          "windows-x86_64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/latest/download/app.exe",
          },
          "darwin-aarch64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn.app.tar.gz",
          },
          "darwin-x86_64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn-x64.app.tar.gz",
          },
        },
      }),
    ).toThrow(/immutable/i);
  });

  it("rejects malformed manifests with missing required platforms", () => {
    expect(() =>
      buildLatestManifest({
        version: "0.2.0",
        notes: "",
        pubDate: "2026-04-18T12:00:00Z",
        platforms: {
          "windows-x86_64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/app.exe",
          },
          "darwin-aarch64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn.app.tar.gz",
          },
        },
      }),
    ).toThrow(/darwin-x86_64/i);
  });

  it("rejects malformed manifests with empty signatures", () => {
    expect(() =>
      buildLatestManifest({
        version: "0.2.0",
        notes: "",
        pubDate: "2026-04-18T12:00:00Z",
        platforms: {
          "windows-x86_64": {
            signature: " ",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/app.exe",
          },
          "darwin-aarch64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn.app.tar.gz",
          },
          "darwin-x86_64": {
            signature: "sig",
            url: "https://github.com/chuanman2707/CapyInn/releases/download/v0.2.0/CapyInn-x64.app.tar.gz",
          },
        },
      }),
    ).toThrow(/signature/i);
  });
});
