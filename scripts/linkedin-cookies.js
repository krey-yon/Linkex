#!/usr/bin/env node
// Usage: node scripts/linkedin-cookies.js [path/to/cookies.json|cookies.txt|-]
// Accepts a browser-exported JSON cookie array OR pasted raw cookie text
// ("bcookie=\"...\";li_at=...;..."). Writes LINKEDIN_COOKIE_HEADER to .env
// in the same directory (cwd when reading stdin via "-").

const fs = require("fs");
const path = require("path");

const arg = process.argv[2] || path.join(process.cwd(), "config.json");
const raw = (arg === "-" ? fs.readFileSync(0, "utf8") : fs.readFileSync(arg, "utf8")).trim();
const dir = arg === "-" ? process.cwd() : path.dirname(arg);

let header;
try {
  const cookies = JSON.parse(raw);
  if (!Array.isArray(cookies)) throw new Error();
  const now = Date.now() / 1000;
  header = cookies
    .filter((c) => c.session || !c.expirationDate || c.expirationDate > now)
    .map((c) => `${c.name}=${c.value}`)
    .join("; ");
} catch {
  header = raw
    .split(/;\s*/)
    .map((pair) => {
      const eq = pair.indexOf("=");
      if (eq === -1) return null;
      const name = pair.slice(0, eq).trim();
      const value = pair.slice(eq + 1).trim().replace(/^"(.*)"$/, "$1");
      return name && `${name}=${value}`;
    })
    .filter(Boolean)
    .join("; ");
}

if (!header) throw new Error("No valid cookies found");
if (!/(?:^|; )li_at=/.test(header)) console.warn("Warning: no li_at (auth) cookie found");

const envPath = path.join(dir, ".env");
const line = `LINKEDIN_COOKIE_HEADER='${header}'`;
let env = fs.existsSync(envPath) ? fs.readFileSync(envPath, "utf8") : "";

if (/^LINKEDIN_COOKIE_HEADER=.*$/m.test(env)) {
  env = env.replace(/^LINKEDIN_COOKIE_HEADER=.*$/m, line);
} else {
  env = env ? env.replace(/\n*$/, "\n") + line + "\n" : line + "\n";
}
fs.writeFileSync(envPath, env);

console.log(`Wrote ${header.split("; ").length} cookies -> ${envPath}`);
