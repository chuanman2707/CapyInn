import { readdir, readFile } from "node:fs/promises";
import path from "node:path";

const root = path.join(process.cwd(), "src-tauri", "src");

const moneyNames = [
  "amount",
  "balance_due",
  "base_price",
  "daily_rate",
  "deposit",
  "deposit_amount",
  "extra_person_fee",
  "final_total",
  "folio_revenue",
  "hourly_rate",
  "overnight_rate",
  "paid_amount",
  "price_per_night",
  "room_revenue",
  "subtotal",
  "total",
  "total_expenses",
  "total_price",
  "total_revenue",
  "unit_price",
];

const moneyNamePattern = moneyNames.join("|");
const decimalMoneyLiteralPattern = "-?[0-9][0-9_]*\\.0(?:_f64)?";
const sqlMoneyColumnsPattern = [
  "amount",
  "balance_due",
  "base_price",
  "daily_rate",
  "deposit_amount",
  "extra_person_fee",
  "folio_revenue",
  "hourly_rate",
  "overnight_rate",
  "paid_amount",
  "room_revenue",
  "subtotal",
  "total_expenses",
  "total_price",
  "total_revenue",
  "unit_price",
].join("|");

const checks = [
  {
    label: "money value typed as f64",
    pattern: new RegExp(
      `\\b(?:${moneyNamePattern})\\s*:\\s*(?:Option\\s*<\\s*)?f64\\b`,
    ),
  },
  {
    label: "money value cast to f64",
    pattern: new RegExp(`\\b(?:${moneyNamePattern})\\s+as\\s+f64\\b`),
  },
  {
    label: "money column read as f64",
    pattern: new RegExp(
      `get::<\\s*(?:Option\\s*<\\s*)?f64\\s*>?\\s*,[^>]+>\\(\\s*"(?:${moneyNamePattern})"`,
    ),
  },
  {
    label: "money seed literal written as f64",
    pattern: new RegExp(
      `\\b(?:let|const|static)\\s+(?:${moneyNamePattern})\\s*=\\s*${decimalMoneyLiteralPattern}\\b`,
    ),
  },
  {
    label: "money column schema typed as REAL",
    pattern: new RegExp(`\\b(?:${sqlMoneyColumnsPattern})\\s+REAL\\b`),
  },
  {
    label: "SQL money assignment uses decimal literal",
    pattern: new RegExp(
      `\\b(?:${sqlMoneyColumnsPattern})\\s*=\\s*${decimalMoneyLiteralPattern}\\b`,
    ),
  },
  {
    label: "JSON money fixture uses decimal literal",
    pattern: new RegExp(
      `["'](?:${sqlMoneyColumnsPattern})["']\\s*:\\s*${decimalMoneyLiteralPattern}\\b`,
    ),
  },
];
const allowedLegacyMoneySchema = [
  {
    file: "src-tauri/src/db.rs",
    pattern: /CREATE TABLE (?:transactions|folio_lines) \(/,
  },
];
async function rustFiles(dir) {
  const entries = await readdir(dir, { withFileTypes: true });
  const files = await Promise.all(
    entries.map(async (entry) => {
      const fullPath = path.join(dir, entry.name);
      if (entry.isDirectory()) {
        return rustFiles(fullPath);
      }
      if (entry.isFile() && entry.name.endsWith(".rs")) {
        return [fullPath];
      }
      return [];
    }),
  );
  return files.flat();
}

const findings = [];
const allowedReportCastFiles = new Set([
  "src-tauri/src/queries/booking/revenue_queries.rs",
]);

for (const file of await rustFiles(root)) {
  const relative = path.relative(process.cwd(), file);
  const source = await readFile(file, "utf8");
  const lines = source.split(/\r?\n/);

  lines.forEach((line, index) => {
    for (const check of checks) {
      if (check.pattern.test(line)) {
        if (
          check.label === "money column schema typed as REAL" &&
          allowedLegacyMoneySchema.some(
            (allow) => allow.file === relative && allow.pattern.test(source),
          )
        ) {
          continue;
        }
        if (
          check.label === "money value cast to f64" &&
          allowedReportCastFiles.has(relative)
        ) {
          continue;
        }
        if (
          check.label === "JSON money fixture uses decimal literal" &&
          relative === "src-tauri/src/db.rs"
        ) {
          continue;
        }
        findings.push({
          file: relative,
          line: index + 1,
          check: check.label,
          text: line.trim(),
        });
      }
    }
  });
}

if (findings.length > 0) {
  console.error("PMS money write contracts and test fixtures must use integer MoneyVnd, not f64.\n");
  for (const finding of findings) {
    console.error(`${finding.file}:${finding.line} ${finding.check}`);
    console.error(`  ${finding.text}`);
  }
  process.exit(1);
}

console.log("No Rust PMS money f64 contracts found.");
