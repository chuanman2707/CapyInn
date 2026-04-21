import { run } from "./shared.mjs";

const iterations = Number(process.argv[2] ?? "3");
const cwd = process.cwd();

for (let index = 0; index < iterations; index += 1) {
  console.log(`verification iteration ${index + 1}/${iterations}`);
  await run("full", "npm", ["run", "verify:full"], { cwd });
}
