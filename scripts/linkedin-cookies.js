#!/usr/bin/env node
// Usage: node scripts/linkedin-cookies.js [path/to/cookies.json|cookies.txt|-]
// Accepts a browser-exported JSON cookie array OR pasted raw cookie text
// ("bcookie=\"...\";li_at=...;..."). Writes the split cookie env vars
// (LINKEDIN_LI_AT, LINKEDIN_JSESSIONID, LINKEDIN_BCOOKIE, LINKEDIN_B_SCOOKIE,
// LINKEDIN_LIAP, LINKEDIN_LIDC) to .env in the same directory (cwd when
// reading stdin via "-"). Values are quote-free: Coolify's env UI escapes
// double quotes, which LinkedIn 400s; it accepts bare values.

const fs = require("fs");
const path = require("path");

const arg = process.argv[2] || path.join(process.cwd(), "config.json");
const raw = (arg === "-" ? fs.readFileSync(0, "utf8") : fs.readFileSync(arg, "utf8")).trim();
const dir = arg === "-" ? process.cwd() : path.dirname(arg);

// name -> bare value (quotes stripped; %25 etc. left verbatim)
let cookies = {};
try {
  const arr = JSON.parse(raw);
  if (!Array.isArray(arr)) throw new Error();
  const now = Date.now() / 1000;
  for (const c of arr.filter((c) => c.session || !c.expirationDate || c.expirationDate > now)) {
    cookies[c.name] = c.value;
  }
} catch {
  for (const pair of raw.split(/;\s*/)) {
    const eq = pair.indexOf("=");
    if (eq === -1) continue;
    const name = pair.slice(0, eq).trim();
    const value = pair.slice(eq + 1).trim().replace(/^"(.*)"$/, "$1");
    if (name && !(name in cookies)) cookies[name] = value;
  }
}

const strip = (v) => (v || "").replace(/\\"/g, '"').replace(/"/g, "");
const get = (name) => strip(cookies[name]);

const li_at = get("li_at");
if (!li_at) throw new Error("No valid li_at (auth) cookie found");

// Split env vars: one per cookie, quote-free, Coolify-safe.
const SPLIT_VARS = [
  ["LINKEDIN_LI_AT", "li_at"],
  ["LINKEDIN_JSESSIONID", "JSESSIONID"],
  ["LINKEDIN_BCOOKIE", "bcookie"],
  ["LINKEDIN_B_SCOOKIE", "bscookie"],
  ["LINKEDIN_LIAP", "liap"],
  ["LINKEDIN_LIDC", "lidc"],
];

const lines = SPLIT_VARS.map(([envName, cookieName]) => {
  const v = get(cookieName);
  return v ? `${envName}=${v}` : null;
}).filter(Boolean);

if (lines.length === 0) throw new Error("No splittable cookies found");

const envPath = path.join(dir, ".env");
let env = fs.existsSync(envPath) ? fs.readFileSync(envPath, "utf8") : "";

for (const line of lines) {
  const key = line.slice(0, line.indexOf("="));
  if (new RegExp("^" + key + "=.*$", "m").test(env)) {
    env = env.replace(new RegExp("^" + key + "=.*$", "m"), line);
  } else {
    env = env ? env.replace(/\n*$/, "\n") + line + "\n" : line + "\n";
  }
}

// The combined header is deprecated: a pasted value gets its quotes escaped
// by Coolify. Drop the old key when rewriting the file.
env = env.replace(/^LINKEDIN_COOKIE_HEADER=.*$/m, "");

fs.writeFileSync(envPath, env);

console.log(`Wrote ${lines.length} cookie vars -> ${envPath}`);
console.log(`li_at: ${li_at.slice(0, 20)}... (len ${li_at.length})`);
