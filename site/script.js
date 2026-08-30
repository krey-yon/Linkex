/* Tross API playground — vanilla JS, no deps. */
(() => {
  "use strict";

  const $ = (sel) => document.querySelector(sel);

  const profileUrlInput = $("#pg-profile-url");
  const apiKeyInput = $("#pg-api-key");
  const freshSelect = $("#pg-fresh");
  const endpointEl = $("#pg-endpoint");
  const targetSelect = $("#pg-target");
  const clientSelect = $("#pg-client");
  const statusSelect = $("#pg-status");
  const codeEl = $("#pg-code");
  const exampleEl = $("#pg-example");
  const resultsEl = $("#pg-results");
  const resultMeta = $("#pg-resultmeta");
  const resultStatus = $("#pg-result-status");
  const resultTime = $("#pg-result-time");
  const resultCache = $("#pg-result-cache");
  const treeToggle = $("#pg-tree");
  const treeEl = $("#pg-treeview");
  const tryBtn = $("#pg-try");
  const creditEl = $("#pg-credit");
  const balanceRateEl = $("#pg-balance-rate");
  const keyDialog = $("#pg-key-dialog");
  const keyForm = $("#pg-key-form");
  const dialogApiKeyInput = $("#pg-dialog-api-key");
  const keyError = $("#pg-key-error");
  const keySaveBtn = $("#pg-key-save");

  const STORAGE_KEY = "tross_api_key";
  let creditCents = null;
  let cacheHitCostCents = 25;
  let cacheMissCostCents = 50;

  const formatUsd = (cents) => cents === null
    ? "--"
    : new Intl.NumberFormat("en-US", { style: "currency", currency: "USD", minimumFractionDigits: 2 }).format(cents / 100);

  const renderCredit = () => {
    creditEl.textContent = formatUsd(creditCents);
    creditEl.classList.toggle("is-empty", creditCents !== null && creditCents < cacheHitCostCents);
    balanceRateEl.textContent = `${formatUsd(cacheHitCostCents)} hit · ${formatUsd(cacheMissCostCents)} miss`;
    tryBtn.disabled = !apiKeyInput.value.trim() || creditCents === null || creditCents < cacheHitCostCents;
  };

  const openKeyDialog = (message = "") => {
    keyError.textContent = message;
    keyError.hidden = !message;
    dialogApiKeyInput.value = apiKeyInput.value.trim();
    if (!keyDialog.open) keyDialog.showModal();
    requestAnimationFrame(() => dialogApiKeyInput.focus());
  };

  const loadAccount = async (key) => {
    const response = await fetch("/v1/account", { headers: { "X-API-Key": key, Accept: "application/json" } });
    const body = await response.json().catch(() => null);
    if (!response.ok) throw new Error(body?.error?.message || "Could not validate this API key.");
    creditCents = body.balance_cents;
    cacheHitCostCents = body.cache_hit_cost_cents;
    cacheMissCostCents = body.cache_miss_cost_cents;
    renderCredit();
    return body;
  };

  const saveAndLoadKey = async (key) => {
    const normalized = key.trim();
    await loadAccount(normalized);
    apiKeyInput.value = normalized;
    localStorage.setItem(STORAGE_KEY, normalized);
    renderSnippet();
    renderCredit();
  }; 

  // Shapes mirror src/domain/response.rs (envelope) and src/domain/profile.rs (data).
  const EXAMPLES = {
    200: {
      success: true,
      meta: { request_id: "0be46aaa9b28", cached: false, cache_age_seconds: 0, elapsed_ms: 3420, upstream_calls: 2 },
      data: {
        profile_url: "https://www.linkedin.com/in/satyanadella/",
        public_identifier: "satyanadella",
        member_urn: "urn:li:member:19186432",
        profile_id: "ACoAAAEkwwAB9KEc2TrQg0LEQ-vzRyZeCDyc6DQ",
        first_name: "Satya",
        last_name: "Nadella",
        full_name: "Satya Nadella",
        headline: "Chairman and CEO at Microsoft",
        about: "As chairman and CEO of Microsoft, I define my mission and that of my company as empowering every person and every organization on the planet to achieve more.",
        industry: "Computer Software",
        pronouns: null,
        location: { label: "Redmond, Washington, United States", city: "Redmond", country_code: "us", geo_urn: "urn:li:geo:102393975", postal_code: null },
        profile_picture: { url: "https://media.licdn-north.com/shrink_800_800.png", renditions: [{ url: "https://media.licdn-north.com/shrink_400_400.png", width: 400, height: 400, expires_at: "2026-12-01T00:00:00Z" }] },
        background_picture: null,
        network: null,
        contact: null,
        experience: [{ title: "Chairman and CEO", employment_type: null, company: { name: "Microsoft", urn: "urn:li:company:1035", universal_name: "microsoft", linkedin_url: "https://www.linkedin.com/company/microsoft/", logo: null }, location: "Redmond, Washington, United States", description: null, dates: { started_at: { year: 2014, month: 2 }, ended_at: null, duration_months: null }, skills: [] }],
        education: [],
        skills: [{ name: "Cloud Computing", endorsement_count: 99 }],
        certifications: [],
        languages: [{ name: "English", proficiency: "Native or bilingual" }],
        projects: [],
        publications: [],
        honors: [],
        volunteering: [],
        courses: [],
        patents: [],
        test_scores: [],
        organizations: [],
        meta: { fetched_at: "2026-08-30T17:48:26Z", sources: [{ endpoint: "dashProfile", status_code: 200, ok: true, elapsed_ms: 249, attempts: 1 }], warnings: [], sections_populated: ["experience", "skills", "languages"], completeness: 0.62 },
      },
    },
    401: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "API_KEY_MISSING", message: "Missing X-API-Key header.", details: {} },
    },
    403: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "PROFILE_NOT_VISIBLE", message: "No LinkedIn profile model returned data for this member.", details: { attempted: ["dashProfile", "dashProfileMinimal"], identifier: "satyanadella" } },
    },
    404: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "PROFILE_NOT_FOUND", message: "No public profile exists for that identifier.", details: {} },
    },
    422: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "INVALID_PROFILE_URL", message: "The `url` query parameter must be a LinkedIn profile URL like https://www.linkedin.com/in/<handle>.", details: {} },
    },
    429: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "RATE_LIMITED", message: "Too many requests; retry after the indicated window.", details: { retry_after_seconds: 30 } },
    },
    503: {
      success: false,
      request_id: "ca574a989491",
      error: { code: "LINKEDIN_SESSION_EXPIRED", message: "The LinkedIn session cookie is no longer valid; re-seed credentials.", details: { upstream_status: 302 } },
    },
  };

  const DEFAULT_URL = "https://www.linkedin.com/in/satyanadella";

  const buildUrl = () => {
    const params = new URLSearchParams();
    params.set("url", profileUrlInput.value.trim() || DEFAULT_URL);
    if (freshSelect.value) params.set("refresh", freshSelect.value);
    return `/v1/profile?${params.toString()}`;
  };

  // Human-readable variant for display only (the request itself uses buildUrl).
  const prettyUrl = () => {
    const url = profileUrlInput.value.trim() || DEFAULT_URL;
    return `/v1/profile?url=${url}${freshSelect.value ? `&refresh=${freshSelect.value}` : ""}`;
  };

  const renderEndpoint = () => {
    endpointEl.textContent = prettyUrl();
  };

  const headers = () => {
    const key = apiKeyInput.value.trim() || "tross_sk_YOUR_KEY";
    return [["X-API-Key", key], ["Accept", "application/json"]];
  };

  const snippets = {
    curl: () => {
      const lines = [`curl --request GET \\`, `  --url '${location.origin}${buildUrl()}' \\`];
      for (const [k, v] of headers()) lines.push(`  --header '${k}: ${v}' \\`);
      return lines.join("\n").replace(/ \\$/, "");
    },
    httpie: () => {
      const h = headers().map(([k, v]) => `${k}:'${v}'`).join(" ");
      return `http GET '${location.origin}${buildUrl()}' ${h}`;
    },
    wget: () => {
      const h = headers().map(([k, v]) => `--header='${k}: ${v}'`).join(" \\\n  ");
      return `wget -qO- \\\n  '${location.origin}${buildUrl()}' \\\n  ${h}`;
    },
  };

  const renderSnippet = () => {
    const client = clientSelect.value;
    codeEl.textContent = client === "wget" ? snippets.wget() : client === "httpie" ? snippets.httpie() : snippets.curl();
  };

  const renderExample = () => {
    exampleEl.textContent = JSON.stringify(EXAMPLES[statusSelect.value] ?? EXAMPLES[200], null, 2);
  };

  const switchTab = (name) => {
    document.querySelectorAll(".pg-tab").forEach((t) => t.classList.toggle("is-active", t.dataset.tab === name));
    document.querySelectorAll(".pg-tabpane").forEach((p) => p.classList.toggle("is-active", p.dataset.pane === name));
  };

  const copyText = async (text, btn) => {
    try {
      await navigator.clipboard.writeText(text);
      btn.classList.add("is-copied");
      setTimeout(() => btn.classList.remove("is-copied"), 1200);
    } catch {
      /* clipboard unavailable; ignore */
    }
  };

  const setStatus = (status) => {
    resultStatus.textContent = status;
    resultStatus.className = `pg-pill pg-pill--${String(status)[0]}`;
  };

  // ------------------------------------------------------------- json tree

  let lastJson = null;

  const span = (cls, text) => {
    const s = document.createElement("span");
    if (cls) s.className = cls;
    s.textContent = text;
    return s;
  };

  // Collapsible JSON node: "▼ key {3}" for objects/arrays, "key : value" leaves.
  const buildNode = (key, value, depth) => {
    const node = document.createElement("div");
    node.className = "pg-node";
    const head = document.createElement("div");
    head.className = "pg-node-head";
    node.appendChild(head);

    if (value === null || typeof value !== "object") {
      head.classList.add("pg-node-head--leaf");
      if (key !== null) {
        head.appendChild(span("pg-node-key", key));
        head.appendChild(span("pg-node-sep", " : "));
      }
      const s = value === null ? "null" : String(value);
      if (/^https?:\/\//.test(s)) {
        const a = document.createElement("a");
        a.className = "pg-node-link";
        a.href = s;
        a.target = "_blank";
        a.rel = "noopener";
        a.textContent = s;
        head.appendChild(a);
      } else {
        head.appendChild(span("pg-node-val", s));
      }
      return node;
    }

    const isArr = Array.isArray(value);
    const entries = isArr ? value.map((v, i) => [String(i), v]) : Object.entries(value);
    const arrow = span("pg-node-arrow", "▼");
    head.appendChild(arrow);
    if (key !== null) head.appendChild(span("pg-node-key", `${key} `));
    head.appendChild(
      span("pg-node-type", `${key === null ? (isArr ? "array " : "object ") : ""}${isArr ? `[${entries.length}]` : `{${entries.length}}`}`)
    );
    const kids = document.createElement("div");
    kids.className = "pg-node-kids";
    for (const [k, v] of entries) kids.appendChild(buildNode(k, v, depth + 1));
    node.appendChild(kids);

    const setOpen = (open) => {
      node.classList.toggle("is-closed", !open);
      arrow.textContent = open ? "▼" : "▶";
    };
    setOpen(depth < 2);
    head.addEventListener("click", () => setOpen(node.classList.contains("is-closed")));
    return node;
  };

  // Raw JSON by default; the checkbox swaps in the collapsible tree.
  const renderResultsView = () => {
    const tree = treeToggle.checked && lastJson !== null;
    treeEl.hidden = !tree;
    resultsEl.hidden = tree;
    if (tree) treeEl.replaceChildren(buildNode(null, lastJson, 0));
  };

  const runRequest = async () => {
    if (!apiKeyInput.value.trim()) {
      openKeyDialog("Enter your API key before making a request.");
      return;
    }
    if (creditCents === null || creditCents < cacheHitCostCents) {
      switchTab("results");
      resultsEl.textContent = "This API key does not have enough credit for another request.";
      return;
    }
    switchTab("results");
    resultsEl.textContent = "Loading…";
    resultMeta.hidden = true;
    const started = performance.now();
    try {
      const res = await fetch(buildUrl(), { headers: Object.fromEntries(headers()) });
      const ms = Math.round(performance.now() - started);
      const body = await res.text();
      lastJson = null;
      try {
        lastJson = JSON.parse(body);
      } catch {
        /* non-JSON body; tree view stays unavailable */
      }
      const pretty = lastJson ? JSON.stringify(lastJson, null, 2) : body;
      const headerBalance = Number.parseInt(res.headers.get("x-credit-balance-cents"), 10);
      if (Number.isFinite(headerBalance)) {
        creditCents = headerBalance;
      } else if (lastJson?.error?.details && Number.isInteger(lastJson.error.details.balance_cents)) {
        creditCents = lastJson.error.details.balance_cents;
      }
      renderCredit();
      resultMeta.hidden = false;
      setStatus(res.status);
      resultTime.textContent = `${ms} ms`;
      resultCache.textContent = res.headers.get("x-cache") ? `cache: ${res.headers.get("x-cache")}` : "";
      resultsEl.textContent = pretty;
      renderResultsView();
    } catch (err) {
      lastJson = null;
      resultMeta.hidden = true;
      resultsEl.hidden = false;
      resultsEl.textContent = `Request failed: ${err.message}`;
    }
  };

  profileUrlInput.addEventListener("input", () => { renderEndpoint(); renderSnippet(); });
  freshSelect.addEventListener("change", () => { renderEndpoint(); renderSnippet(); });
  apiKeyInput.addEventListener("input", () => {
    creditCents = null;
    renderSnippet();
    renderCredit();
  });
  apiKeyInput.addEventListener("change", async () => {
    const key = apiKeyInput.value.trim();
    if (!key) {
      localStorage.removeItem(STORAGE_KEY);
      renderCredit();
      return;
    }
    try {
      await saveAndLoadKey(key);
    } catch (error) {
      localStorage.removeItem(STORAGE_KEY);
      creditCents = null;
      renderCredit();
      openKeyDialog(error.message);
    }
  });
  clientSelect.addEventListener("change", renderSnippet);
  statusSelect.addEventListener("change", renderExample);
  treeToggle.addEventListener("change", renderResultsView);
  tryBtn.addEventListener("click", runRequest);

  document.querySelectorAll(".pg-tab").forEach((tab) =>
    tab.addEventListener("click", () => switchTab(tab.dataset.tab))
  );

  document.querySelectorAll("[data-copy]").forEach((btn) =>
    btn.addEventListener("click", () => {
      const el = document.querySelector(btn.dataset.copy);
      if (el) copyText(el.textContent, btn);
    })
  );

  keyForm.addEventListener("submit", async (event) => {
    event.preventDefault();
    keySaveBtn.disabled = true;
    keySaveBtn.textContent = "Validating…";
    keyError.hidden = true;
    try {
      await saveAndLoadKey(dialogApiKeyInput.value);
      keyDialog.close();
    } catch (error) {
      keyError.textContent = error.message;
      keyError.hidden = false;
      dialogApiKeyInput.focus();
    } finally {
      keySaveBtn.disabled = false;
      keySaveBtn.textContent = "Use API key";
    }
  });

  const initialize = async () => {
    renderEndpoint();
    renderSnippet();
    renderExample();
    renderCredit();
    const savedKey = localStorage.getItem(STORAGE_KEY);
    if (!savedKey) {
      openKeyDialog();
      return;
    }
    apiKeyInput.value = savedKey;
    renderSnippet();
    try {
      await loadAccount(savedKey);
    } catch (error) {
      localStorage.removeItem(STORAGE_KEY);
      apiKeyInput.value = "";
      creditCents = null;
      renderCredit();
      openKeyDialog(error.message);
    }
  };

  initialize();

  // ------------------------------------------------- architecture diagram

  const ARCH_DEFINITION = `flowchart LR

subgraph group_runtime["Runtime"]
  node_main["Executable<br/>Rust entrypoint<br/>[main.rs]"]
  node_app["App assembly<br/>composition root<br/>[app.rs]"]
  node_state["Shared state<br/>dependencies<br/>[state.rs]"]
  node_errors["Error mapping<br/>stable API errors<br/>[error.rs]"]
end

subgraph group_api["Public API"]
  node_api_routes["Routes<br/>HTTP routing<br/>[mod.rs]"]
  node_middleware["Request middleware<br/>auth and billing<br/>[middleware.rs]"]
  node_profile_api["Profile endpoints<br/>HTTP handlers<br/>[profile.rs]"]
  node_account_api["Account endpoints<br/>HTTP handlers<br/>[account.rs]"]
  node_system_api["System endpoints<br/>HTTP handlers<br/>[system.rs]"]
  node_response_domain["Response envelope<br/>API contract<br/>[response.rs]"]
end

subgraph group_service["Profile Service"]
  node_profile_service["Profile orchestration<br/>use case<br/>[profile.rs]"]
  node_cache[("Profile cache<br/>memory and disk cache<br/>[cache.rs]")]
  node_billing["Billing<br/>credits<br/>[billing.rs]"]
  node_parser["Voyager parser pipeline<br/>normalization pipeline<br/>[mod.rs]"]
  node_profile_domain["Stable profile schema<br/>domain contract<br/>[profile.rs]"]
end

subgraph group_linkedin["LinkedIn Upstream"]
  node_repository["LinkedIn repository<br/>upstream boundary<br/>[repository.rs]"]
  node_voyager_client["Voyager client<br/>authenticated HTTP client<br/>[client.rs]"]
  node_auth_session["Auth and sessions<br/>credential lifecycle<br/>[auth.rs]"]
  node_throttle["Upstream protection<br/>throttle and circuit breaker<br/>[throttle.rs]"]
end

subgraph group_data["Data &amp; Operations"]
  node_config["Runtime config<br/>configuration<br/>[config.rs]"]
  node_redis[("Redis<br/>external balances store<br/>[docker-compose.yml]")]
  node_telemetry["Telemetry<br/>structured logging<br/>[telemetry.rs]"]
end

node_main -->|"starts"| node_app
node_app -->|"builds"| node_state
node_app -->|"serves"| node_api_routes
node_config -->|"configures"| node_app
node_api_routes -->|"applies"| node_middleware
node_api_routes -->|"registers"| node_profile_api
node_api_routes -->|"registers"| node_account_api
node_api_routes -->|"registers"| node_system_api
node_profile_api -->|"retrieves profile"| node_profile_service
node_profile_api -->|"returns"| node_response_domain
node_middleware -->|"handles billable requests"| node_billing
node_account_api -->|"reads balance and pricing"| node_billing
node_profile_service -->|"checks and refreshes"| node_cache
node_profile_service -->|"fetches on miss"| node_repository
node_profile_service -->|"parses payload"| node_parser
node_parser -->|"normalizes into"| node_profile_domain
node_repository -->|"protects requests with"| node_throttle
node_throttle -->|"permits calls to"| node_voyager_client
node_voyager_client -->|"uses session"| node_auth_session
node_billing -->|"stores balances in"| node_redis
node_app -->|"initializes"| node_telemetry
node_api_routes -->|"maps failures through"| node_errors

click node_main "https://github.com/krey-yon/linkex/blob/main/src/main.rs"
click node_app "https://github.com/krey-yon/linkex/blob/main/src/app.rs"
click node_state "https://github.com/krey-yon/linkex/blob/main/src/state.rs"
click node_config "https://github.com/krey-yon/linkex/blob/main/src/config.rs"
click node_api_routes "https://github.com/krey-yon/linkex/blob/main/src/api/mod.rs"
click node_middleware "https://github.com/krey-yon/linkex/blob/main/src/api/middleware.rs"
click node_profile_api "https://github.com/krey-yon/linkex/blob/main/src/api/profile.rs"
click node_account_api "https://github.com/krey-yon/linkex/blob/main/src/api/account.rs"
click node_system_api "https://github.com/krey-yon/linkex/blob/main/src/api/system.rs"
click node_profile_service "https://github.com/krey-yon/linkex/blob/main/src/service/profile.rs"
click node_cache "https://github.com/krey-yon/linkex/blob/main/src/service/cache.rs"
click node_billing "https://github.com/krey-yon/linkex/blob/main/src/billing.rs"
click node_repository "https://github.com/krey-yon/linkex/blob/main/src/linkedin/repository.rs"
click node_voyager_client "https://github.com/krey-yon/linkex/blob/main/src/linkedin/client.rs"
click node_auth_session "https://github.com/krey-yon/linkex/blob/main/src/linkedin/auth.rs"
click node_throttle "https://github.com/krey-yon/linkex/blob/main/src/linkedin/throttle.rs"
click node_parser "https://github.com/krey-yon/linkex/blob/main/src/parser/mod.rs"
click node_profile_domain "https://github.com/krey-yon/linkex/blob/main/src/domain/profile.rs"
click node_response_domain "https://github.com/krey-yon/linkex/blob/main/src/domain/response.rs"
click node_redis "https://github.com/krey-yon/linkex/blob/main/docker-compose.yml"
click node_telemetry "https://github.com/krey-yon/linkex/blob/main/src/telemetry.rs"
click node_errors "https://github.com/krey-yon/linkex/blob/main/src/error.rs"

classDef toneNeutral fill:#f8fafc,stroke:#334155,stroke-width:1.5px,color:#0f172a
classDef toneBlue fill:#dbeafe,stroke:#2563eb,stroke-width:1.5px,color:#172554
classDef toneAmber fill:#fef3c7,stroke:#d97706,stroke-width:1.5px,color:#78350f
classDef toneMint fill:#dcfce7,stroke:#16a34a,stroke-width:1.5px,color:#14532d
classDef toneRose fill:#ffe4e6,stroke:#e11d48,stroke-width:1.5px,color:#881337
classDef toneIndigo fill:#e0e7ff,stroke:#4f46e5,stroke-width:1.5px,color:#312e81
classDef toneTeal fill:#ccfbf1,stroke:#0f766e,stroke-width:1.5px,color:#134e4a
class node_main,node_app,node_state,node_errors toneBlue
class node_api_routes,node_middleware,node_profile_api,node_account_api,node_system_api,node_response_domain toneAmber
class node_profile_service,node_cache,node_billing,node_parser,node_profile_domain toneMint
class node_repository,node_voyager_client,node_auth_session,node_throttle toneRose
class node_config,node_redis,node_telemetry toneIndigo`;

  const archBtn = $("#pg-archbtn");
  const archModal = $("#pg-archmodal");
  const archViewport = $("#pg-archviewport");
  const archInner = $("#pg-archinner");
  let archLoaded = false;
  let archScale = 1;
  let archTx = 0;
  let archTy = 0;

  const archApply = () => {
    archInner.style.transform = `translate(${archTx}px, ${archTy}px) scale(${archScale})`;
  };

  const archReset = () => {
    archScale = 1;
    archTx = 0;
    archTy = 0;
    archApply();
  };

  const archZoom = (factor, cx, cy) => {
    const next = Math.min(4, Math.max(0.3, archScale * factor));
    if (cx === undefined) {
      const rect = archViewport.getBoundingClientRect();
      cx = rect.width / 2;
      cy = rect.height / 2;
    }
    const k = next / archScale;
    archTx = cx - (cx - archTx) * k;
    archTy = cy - (cy - archTy) * k;
    archScale = next;
    archApply();
  };

  const openArch = async () => {
    archModal.hidden = false;
    if (archLoaded) return;
    archInner.textContent = "Loading diagram…";
    try {
      const mermaid = (await import("https://cdn.jsdelivr.net/npm/mermaid@11/dist/mermaid.esm.min.mjs")).default;
      mermaid.initialize({ startOnLoad: false, securityLevel: "loose", theme: "neutral" });
      const { svg } = await mermaid.render("pg-arch-svg", ARCH_DEFINITION);
      archInner.innerHTML = svg; // static, repo-owned diagram definition
      archLoaded = true;
      archReset();
    } catch {
      archInner.textContent = "Could not load the diagram (offline?).";
    }
  };

  const closeArch = () => {
    archModal.hidden = true;
  };

  archBtn.addEventListener("click", openArch);
  archModal.querySelectorAll("[data-arch-close]").forEach((el) => el.addEventListener("click", closeArch));
  document.addEventListener("keydown", (e) => {
    if (e.key === "Escape" && !archModal.hidden) closeArch();
  });
  archModal.querySelectorAll("[data-arch-zoom]").forEach((btn) =>
    btn.addEventListener("click", () => {
      const action = btn.dataset.archZoom;
      if (action === "in") archZoom(1.2);
      else if (action === "out") archZoom(1 / 1.2);
      else archReset();
    })
  );

  archViewport.addEventListener("wheel", (e) => {
    e.preventDefault();
    const rect = archViewport.getBoundingClientRect();
    archZoom(e.deltaY < 0 ? 1.15 : 1 / 1.15, e.clientX - rect.left, e.clientY - rect.top);
  }, { passive: false });

  let archDrag = null;
  archViewport.addEventListener("pointerdown", (e) => {
    archDrag = { x: e.clientX - archTx, y: e.clientY - archTy };
    archViewport.setPointerCapture(e.pointerId);
  });
  archViewport.addEventListener("pointermove", (e) => {
    if (!archDrag) return;
    archTx = e.clientX - archDrag.x;
    archTy = e.clientY - archDrag.y;
    archApply();
  });
  archViewport.addEventListener("pointerup", () => {
    archDrag = null;
  });
})();
